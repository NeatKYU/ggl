//! 앱 상태. git 작업은 백그라운드 스레드에서 돌리고, 화면은 결과만 그린다.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Frame, Key, Margin, Modifiers};
use serde::{Deserialize, Serialize};

use crate::edit::Editor;
use crate::git::{self, Details, Diff, FileChange, Grep, LoadRequest, RefLabel, Snapshot};
use crate::graph::{self, Layout, OFFSCREEN};
use crate::ops::{self, Item, Op};
use crate::remote::{self, Request};
use crate::style::Palette;
use crate::tree::FileTree;
use crate::view;
use crate::watcher::RepoWatcher;

const INITIAL_LOAD: usize = 300;
const LOAD_MORE: usize = 100;
const MAX_RECENT: usize = 10;

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 최근 연 저장소 (맨 앞이 마지막으로 연 것)
    pub recent: Vec<PathBuf>,
    pub show_remotes: bool,
    /// 커밋 상세(인라인) 높이와, 왼쪽(요약) 칸이 차지하는 비율
    pub details_height: f32,
    pub details_split: f32,
    /// 왼쪽 파일 트리를 보여줄지
    pub show_files: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { recent: Vec::new(), show_remotes: true, details_height: 260.0, details_split: 0.5, show_files: false }
    }
}

/// 오른쪽에 열린 파일 diff
pub struct DiffView {
    pub hash: String,
    pub index: usize,
    pub file: FileChange,
    pub result: Option<Result<Diff, String>>,
}

/// 파일 트리에서 연 파일 (아래쪽 패널)
pub struct FileView {
    pub path: String,
    pub status: Option<char>,
    pub result: Option<Result<Diff, String>>,
    /// 내용이 오면 이 줄로 스크롤한다 (한 번만)
    pub goto: Option<u32>,
    /// 검색 결과에서 연 줄 (계속 칠해 둔다)
    pub mark: Option<u32>,
}

/// 왼쪽 패널에 보이는 것
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Files,
    Search,
}

/// 파일 내용 검색 (⌘⇧F)
#[derive(Default)]
pub struct Find {
    pub query: String,
    /// 대소문자 구분
    pub case: bool,
    pub focus: bool,
    /// 지금 보여주는(또는 기다리는) 결과의 검색어와 대소문자 구분
    pub ran: Option<(String, bool)>,
    pub result: Option<Result<Grep, String>>,
    pub running: bool,
    /// 입력이 멈추면 검색할 시각
    due: Option<f64>,
    /// 이름이 맞는 파일 (트리 노드 번호)
    pub names: Vec<usize>,
}

/// 편집 중인 파일을 떠나는 동작. 저장 안 한 변경이 있으면 먼저 묻는다.
pub enum Leave {
    StopEditing,
    CloseFile,
    OpenFile(String, Option<char>, Option<u32>),
    OpenDiff(usize),
    OpenRepo(PathBuf),
    Quit,
}

/// 불러온 저장소 데이터와 그래프 배치
pub struct Loaded {
    pub snap: Snapshot,
    pub layout: Layout,
    /// 해시 → 행 번호
    pub rows: HashMap<String, usize>,
}

impl Loaded {
    fn new(snap: Snapshot) -> Self {
        let rows: HashMap<String, usize> =
            snap.commits.iter().enumerate().map(|(i, c)| (c.hash.clone(), i)).collect();
        let parents: Vec<Vec<usize>> = snap
            .commits
            .iter()
            .map(|c| c.parents.iter().map(|p| rows.get(p).copied().unwrap_or(OFFSCREEN)).collect())
            .collect();
        let uncommitted = snap.commits.first().is_some_and(|c| c.hash == git::UNCOMMITTED);
        let layout = graph::layout(&parents, uncommitted);
        Self { snap, layout, rows }
    }
}

enum Job {
    Load(u64, LoadRequest),
    Details(PathBuf, String),
    Diff(PathBuf, String, Vec<String>, FileChange),
    Tree(PathBuf),
    File(PathBuf, String, Option<char>),
}

/// 리모트 새로고침(fetch) 상태
#[derive(Clone, PartialEq)]
pub enum FetchState {
    Idle,
    Running,
    Failed(String),
}

/// 표 위에 띄우는 알림 (작업 실패 등). 닫기를 누르거나 다음 작업을 시작하면 사라진다.
pub struct Notice {
    pub title: String,
    pub body: String,
    pub error: bool,
}

/// 커밋 행을 누른 기록. 더블클릭의 두 번째 클릭이 왔을 때 어느 커밋이었는지 알려준다.
pub struct RowClick {
    pub hash: String,
    /// 브랜치·태그 라벨 위를 눌렀으면 그 라벨
    pub label: Option<RefLabel>,
}

enum Reply {
    Fetched(PathBuf, Result<(), String>),
    Done(PathBuf, Op, Result<(), String>),
    Loaded(u64, Result<Loaded, String>),
    Details(PathBuf, String, Result<Details, String>),
    Diff(PathBuf, String, String, Result<Diff, String>),
    Tree(PathBuf, Result<FileTree, String>),
    File(PathBuf, String, Result<Diff, String>),
    Grep(PathBuf, (String, bool), Result<Grep, String>),
}

/// 스크롤 요청: 가운데로 보낼지, 화면 밖일 때만 살짝 움직일지
#[derive(Clone, Copy)]
pub enum ScrollTo {
    Center(usize),
    Reveal(usize),
}

pub struct App {
    pub settings: Settings,
    pub repo: Option<PathBuf>,
    pub data: Option<Loaded>,
    pub error: Option<String>,
    pub loading: bool,
    pub refreshed_at: String,
    pub fetch_state: FetchState,
    /// 마지막으로 리모트를 가져온 시각
    pub fetched_at: String,
    max_commits: usize,
    pub branch: Option<String>,

    /// 지금 실행 중인 작업 (체크아웃, 머지 …). 한 번에 하나만 돌린다.
    pub running: Option<Op>,
    /// 실행 전에 확인 창으로 물어보는 중인 작업
    pub confirm: Option<Item>,
    pub notice: Option<Notice>,
    pub row_click: Option<RowClick>,

    pub selected: Option<String>,
    pub details: Option<(String, Result<Details, String>)>,
    pub diff: Option<DiffView>,

    /// 왼쪽 파일 트리
    pub tree: Option<Result<FileTree, String>>,
    /// 펼친 폴더 경로
    pub tree_open: HashSet<String>,
    pub tree_filter: String,
    /// 트리 검색어에 맞는 파일 (트리 노드 번호)
    pub tree_matches: Vec<usize>,
    pub file_view: Option<FileView>,
    pub side: Side,
    pub find: Find,
    /// 아래쪽에서 편집 중인 파일 (`file_view`와 같은 파일)
    pub editor: Option<Editor>,
    /// 저장 안 한 편집이 있어서 확인 창으로 물어보는 중인 이동
    pub leaving: Option<Leave>,

    pub search: String,
    pub matches: Vec<usize>,
    pub match_pos: usize,
    pub focus_search: bool,

    pub scroll_to: Option<ScrollTo>,
    /// 지난 프레임의 스크롤 위치 (Reveal 계산용)
    pub scroll_offset: f32,

    generation: u64,
    jobs: Sender<Job>,
    replies: Receiver<Reply>,
    /// 작업 스레드 밖(리모트 새로고침)에서 결과를 보낼 때 쓴다
    reply_tx: Sender<Reply>,
    watcher: Option<RepoWatcher>,
    dirty: Arc<AtomicBool>,
    /// 같은 저장소를 다시 열려는 다른 ggl의 요청을 받는다
    remote: Option<remote::Listener>,
    remote_tx: Sender<Request>,
    remote_rx: Receiver<Request>,
    ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, arg_repo: Option<PathBuf>, show_files: bool) -> Self {
        crate::style::install(&cc.egui_ctx);
        let mut settings: Settings =
            cc.storage.and_then(|s| eframe::get_value(s, "settings")).unwrap_or_default();
        settings.show_files |= show_files;
        let (jobs, replies, reply_tx) = spawn_worker(cc.egui_ctx.clone());
        let (remote_tx, remote_rx) = mpsc::channel();
        let mut app = Self {
            settings,
            repo: None,
            data: None,
            error: None,
            loading: false,
            refreshed_at: String::new(),
            fetch_state: FetchState::Idle,
            fetched_at: String::new(),
            max_commits: INITIAL_LOAD,
            branch: None,
            running: None,
            confirm: None,
            notice: None,
            row_click: None,
            selected: None,
            details: None,
            diff: None,
            tree: None,
            tree_open: HashSet::new(),
            tree_filter: String::new(),
            tree_matches: Vec::new(),
            file_view: None,
            side: Side::Files,
            find: Find::default(),
            editor: None,
            leaving: None,
            search: String::new(),
            matches: Vec::new(),
            match_pos: 0,
            focus_search: false,
            scroll_to: None,
            scroll_offset: 0.0,
            generation: 0,
            jobs,
            replies,
            reply_tx,
            watcher: None,
            dirty: Arc::new(AtomicBool::new(false)),
            remote: None,
            remote_tx,
            remote_rx,
            ctx: cc.egui_ctx.clone(),
        };
        let start = arg_repo.or_else(|| app.settings.recent.first().cloned());
        if let Some(repo) = start {
            app.open_repo(&repo);
        }
        app
    }

    pub fn open_repo(&mut self, dir: &Path) {
        self.leave(Leave::OpenRepo(dir.to_path_buf()));
    }

    fn load_repo(&mut self, dir: &Path) {
        let repo = match git::toplevel(dir) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.settings.recent.retain(|p| p != &repo);
        self.settings.recent.insert(0, repo.clone());
        self.settings.recent.truncate(MAX_RECENT);

        self.generation += 1;
        self.data = None;
        self.error = None;
        self.branch = None;
        self.fetch_state = FetchState::Idle;
        self.fetched_at.clear();
        self.max_commits = INITIAL_LOAD;
        self.confirm = None;
        self.notice = None;
        self.row_click = None;
        self.close_details();
        self.search.clear();
        self.matches.clear();
        self.scroll_to = None;
        self.scroll_offset = 0.0;
        self.tree = None;
        self.tree_open.clear();
        self.tree_filter.clear();
        self.tree_matches.clear();
        self.file_view = None;
        self.find = Find::default();

        let title = format!("{} — ggl", repo_name(&repo));
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let dirty = self.dirty.clone();
        let ctx = self.ctx.clone();
        self.watcher = RepoWatcher::start(&repo, move || {
            dirty.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        })
        .ok();
        // 이전 저장소의 소켓을 먼저 닫아야 같은 저장소를 다시 열 때도 받을 수 있다.
        self.remote = None;
        let (tx, ctx) = (self.remote_tx.clone(), self.ctx.clone());
        self.remote = remote::Listener::start(&repo, move |req| {
            let _ = tx.send(req);
            ctx.request_repaint();
        });
        self.repo = Some(repo);
        self.refresh();
    }

    pub fn pick_folder(&mut self) {
        let dialog = rfd::FileDialog::new().set_title("Git 저장소 폴더 선택");
        let dialog = match &self.repo {
            Some(r) => dialog.set_directory(r.parent().unwrap_or(r)),
            None => dialog,
        };
        if let Some(dir) = dialog.pick_folder() {
            self.open_repo(&dir);
        }
    }

    pub fn forget_repo(&mut self, repo: &Path) {
        self.settings.recent.retain(|p| p != repo);
    }

    /// 커밋 목록과 파일 트리(열려 있으면)를 다시 읽는다. 내용 검색 결과도 새로 찾는다.
    pub fn refresh(&mut self) {
        self.reload();
        self.refresh_files();
        if self.side == Side::Search && self.find.ran.is_some() {
            self.run_find();
        }
    }

    /// 파일 트리는 보이거나 연 파일이 있을 때만 읽는다. 연 파일은 트리를 받은 뒤 새 상태로 다시 읽는다.
    fn refresh_files(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        if self.settings.show_files || self.file_view.is_some() {
            let _ = self.jobs.send(Job::Tree(repo));
        }
    }

    /// 파일 트리를 켜고 끈다 (⌘B).
    pub fn toggle_files(&mut self) {
        self.settings.show_files = !self.settings.show_files;
        if self.settings.show_files {
            self.refresh_files();
        }
    }

    /// 파일 트리나 검색 결과에서 누른 파일을 아래쪽에 연다. `line`이 있으면 그 줄로 스크롤한다.
    pub fn open_file(&mut self, path: &str, status: Option<char>, line: Option<u32>) {
        if let Some(view) = self.file_view.as_mut().filter(|v| v.path == path) {
            if line.is_some() {
                (view.goto, view.mark) = (line, line);
            }
            return;
        }
        self.leave(Leave::OpenFile(path.to_string(), status, line));
    }

    fn show_file(&mut self, path: &str, status: Option<char>, line: Option<u32>) {
        let Some(repo) = self.repo.clone() else { return };
        self.diff = None;
        let _ = self.jobs.send(Job::File(repo, path.to_string(), status));
        self.file_view = Some(FileView { path: path.to_string(), status, result: None, goto: line, mark: line });
    }

    /// 트리에서 그 파일의 커밋 안 된 변경 상태
    pub fn file_status(&self, path: &str) -> Option<char> {
        match &self.tree {
            Some(Ok(t)) => t.nodes.iter().find(|n| !n.dir && n.path == path).and_then(|n| n.status),
            _ => None,
        }
    }

    pub fn edit_dirty(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::dirty)
    }

    /// 아래쪽에 연 파일을 편집하기 시작한다.
    pub fn start_edit(&mut self) {
        let (Some(repo), Some(view)) = (&self.repo, &self.file_view) else { return };
        match Editor::open(repo, &view.path) {
            Ok(editor) => self.editor = Some(editor),
            Err(body) => self.notice = Some(Notice { title: "이 파일은 편집할 수 없어요".into(), body, error: true }),
        }
    }

    /// 편집한 내용을 저장한다 (⌘S). 그사이 다른 곳에서 파일이 바뀌었으면 `force`가 아닌 한 묻는다.
    pub fn save_edit(&mut self, force: bool) -> bool {
        let (Some(repo), Some(editor)) = (self.repo.clone(), self.editor.as_mut()) else { return false };
        let saved = match editor.save(&repo, force) {
            Ok(saved) => {
                (editor.conflict, editor.error) = (!saved, None);
                saved
            }
            Err(e) => {
                editor.error = Some(e);
                false
            }
        };
        if saved {
            // 파일 감시보다 먼저 트리의 변경 표시와 검색 결과를 바꾼다.
            self.refresh();
        }
        saved
    }

    /// 편집 중인 파일을 떠난다. 저장 안 한 변경이 있으면 확인 창을 띄우고 기다린다.
    pub fn leave(&mut self, to: Leave) {
        if self.edit_dirty() {
            self.leaving = Some(to);
        } else {
            self.go(to);
        }
    }

    /// 실제로 떠난다. 편집 중이던 내용은 버린다.
    pub fn go(&mut self, to: Leave) {
        self.leaving = None;
        self.editor = None;
        match to {
            Leave::StopEditing => {}
            Leave::CloseFile => self.file_view = None,
            Leave::OpenFile(path, status, line) => self.show_file(&path, status, line),
            Leave::OpenDiff(index) => self.show_diff(index),
            Leave::OpenRepo(dir) => self.load_repo(&dir),
            Leave::Quit => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// ⌘⇧F: 왼쪽 패널을 내용 검색으로 바꾸고 입력칸에 커서를 둔다.
    pub fn open_find(&mut self) {
        if !self.settings.show_files {
            self.toggle_files();
        }
        self.side = Side::Search;
        self.find.focus = true;
    }

    /// 검색어를 고치면 입력이 잠깐 멈출 때까지 기다렸다가 찾는다 (타자마다 git을 돌리지 않게).
    pub fn find_changed(&mut self) {
        let now = self.ctx.input(|i| i.time);
        self.find.due = Some(now + 0.25);
        self.ctx.request_repaint_after(Duration::from_millis(260));
        self.update_find_names();
    }

    /// 내용 검색을 따로 스레드에서 돌린다. 새 결과가 올 때까지 이전 결과를 보여준다.
    pub fn run_find(&mut self) {
        self.find.due = None;
        let Some(repo) = self.repo.clone() else { return };
        if self.find.query.trim().is_empty() {
            (self.find.ran, self.find.result, self.find.running) = (None, None, false);
            return;
        }
        let key = (self.find.query.clone(), self.find.case);
        self.find.ran = Some(key.clone());
        self.find.running = true;
        let (tx, ctx) = (self.reply_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = git::grep(&repo, &key.0, key.1);
            let _ = tx.send(Reply::Grep(repo, key, result));
            ctx.request_repaint();
        });
    }

    fn update_find_names(&mut self) {
        let q = self.find.query.trim().to_lowercase();
        self.find.names = match &self.tree {
            Some(Ok(tree)) if !q.is_empty() => tree.search(&q, 8),
            _ => Vec::new(),
        };
    }

    pub fn update_tree_matches(&mut self) {
        let q = self.tree_filter.trim().to_lowercase();
        self.tree_matches = match &self.tree {
            Some(Ok(tree)) if !q.is_empty() => tree.search(&q, 500),
            _ => Vec::new(),
        };
    }

    pub fn reload(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        let req = LoadRequest {
            repo,
            max: self.max_commits,
            branch: self.branch.clone(),
            show_remotes: self.settings.show_remotes,
        };
        self.loading = true;
        let _ = self.jobs.send(Job::Load(self.generation, req));
    }

    /// 리모트 새로고침. 네트워크라 오래 걸릴 수 있어서 따로 스레드를 띄운다
    /// (커밋 목록·상세·diff 작업이 기다리지 않게).
    pub fn fetch(&mut self) {
        let Some(repo) = self.repo.clone() else { return };
        if self.busy() {
            return;
        }
        self.fetch_state = FetchState::Running;
        let (tx, ctx) = (self.reply_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = git::fetch(&repo);
            let _ = tx.send(Reply::Fetched(repo, result));
            ctx.request_repaint();
        });
    }

    /// 저장소를 바꾸는 작업(리모트 새로고침 포함)이 돌고 있는지. 겹쳐서 돌리지 않는다.
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.fetch_state == FetchState::Running
    }

    /// 메뉴나 더블클릭에서 고른 작업. 커밋 기록을 바꾸는 작업은 먼저 확인 창으로 물어본다.
    pub fn request(&mut self, item: Item) {
        if self.busy() {
            return;
        }
        if item.op.needs_confirm() {
            self.confirm = Some(item);
        } else {
            self.run_op(item.op);
        }
    }

    /// 작업을 따로 스레드에서 실행한다 (풀은 네트워크라 오래 걸릴 수 있다).
    pub fn run_op(&mut self, op: Op) {
        let Some(repo) = self.repo.clone() else { return };
        if self.busy() {
            return;
        }
        self.notice = None;
        self.running = Some(op.clone());
        let (tx, ctx) = (self.reply_tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let result = ops::run(&repo, &op);
            let _ = tx.send(Reply::Done(repo, op, result));
            ctx.request_repaint();
        });
    }

    /// 커밋 행을 더블클릭하면 그 커밋의 브랜치로 체크아웃한다.
    /// 라벨 위였으면 그 브랜치로, 아니면 체크아웃할 브랜치가 하나로 정해질 때만.
    pub fn double_click(&mut self, click: RowClick) {
        let Some(data) = &self.data else { return };
        let mut items = match &click.label {
            Some(label) => ops::checkout(&data.snap, label).into_iter().collect(),
            None => data.rows.get(&click.hash).map_or_else(Vec::new, |&row| ops::checkouts(&data.snap, row)),
        };
        if items.len() > 1 {
            self.notice = Some(Notice {
                title: "이 커밋에는 브랜치가 여러 개 있어요".into(),
                body: "체크아웃할 브랜치의 라벨을 더블클릭하거나, 오른쪽 클릭 메뉴에서 골라주세요.".into(),
                error: false,
            });
        } else if let Some(item) = items.pop() {
            self.request(item);
        }
    }

    /// 필터가 바뀌면 이전 결과는 버리고 처음부터 다시 불러온다.
    pub fn set_branch(&mut self, branch: Option<String>) {
        if self.branch != branch {
            self.branch = branch;
            self.generation += 1;
            self.max_commits = INITIAL_LOAD;
            self.scroll_to = Some(ScrollTo::Center(0));
            self.reload();
        }
    }

    pub fn toggle_remotes(&mut self) {
        if !self.settings.show_remotes && self.branch.as_ref().is_some_and(|b| b.starts_with("remotes/")) {
            self.branch = None;
        }
        self.generation += 1;
        self.reload();
    }

    pub fn load_more(&mut self) {
        if !self.loading && self.data.as_ref().is_some_and(|d| d.snap.more) {
            self.max_commits += LOAD_MORE;
            self.reload();
        }
    }

    pub fn select(&mut self, hash: String) {
        if self.selected.as_ref() == Some(&hash) {
            return;
        }
        if let Some(repo) = &self.repo {
            let _ = self.jobs.send(Job::Details(repo.clone(), hash.clone()));
        }
        self.selected = Some(hash);
        self.details = None;
        self.diff = None;
    }

    pub fn close_details(&mut self) {
        self.selected = None;
        self.details = None;
        self.diff = None;
    }

    /// 상세의 파일 목록에서 `index`번째 파일의 diff를 연다.
    pub fn open_diff(&mut self, index: usize) {
        self.leave(Leave::OpenDiff(index));
    }

    fn show_diff(&mut self, index: usize) {
        let Some((hash, Ok(d))) = &self.details else { return };
        let Some(file) = d.files.get(index).cloned() else { return };
        let (hash, parents) = (hash.clone(), d.parents.clone());
        self.request_diff(&hash, parents, file.clone());
        let opening = self.diff.is_none();
        self.file_view = None;
        self.diff = Some(DiffView { hash, index, file, result: None });
        // 아래에 diff가 열리면 표가 짧아지니, 선택한 커밋과 상세가 가려지지 않게 한다.
        if opening {
            self.scroll_to = self.selected_row().map(ScrollTo::Reveal);
        }
    }

    fn request_diff(&self, hash: &str, parents: Vec<String>, file: FileChange) {
        if let Some(repo) = &self.repo {
            let _ = self.jobs.send(Job::Diff(repo.clone(), hash.to_string(), parents, file));
        }
    }

    pub fn file_count(&self) -> usize {
        match &self.details {
            Some((_, Ok(d))) => d.files.len(),
            _ => 0,
        }
    }

    pub fn selected_row(&self) -> Option<usize> {
        let data = self.data.as_ref()?;
        data.rows.get(self.selected.as_ref()?).copied()
    }

    pub fn select_row(&mut self, row: usize) {
        let Some(hash) = self.data.as_ref().and_then(|d| d.snap.commits.get(row)).map(|c| c.hash.clone())
        else {
            return;
        };
        self.select(hash);
        self.scroll_to = Some(ScrollTo::Reveal(row));
    }

    pub fn update_matches(&mut self) {
        self.matches.clear();
        self.match_pos = 0;
        let q = self.search.trim().to_lowercase();
        let Some(data) = &self.data else { return };
        if q.is_empty() {
            return;
        }
        for (i, c) in data.snap.commits.iter().enumerate() {
            let refs = data.snap.refs.get(&c.hash);
            let hit = c.subject.to_lowercase().contains(&q)
                || c.author.to_lowercase().contains(&q)
                || c.hash.starts_with(&q)
                || c.date.contains(&q)
                || refs.is_some_and(|r| r.iter().any(|l| l.name.to_lowercase().contains(&q)));
            if hit {
                self.matches.push(i);
            }
        }
        if let Some(&first) = self.matches.first() {
            self.scroll_to = Some(ScrollTo::Center(first));
        }
    }

    pub fn step_match(&mut self, forward: bool) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.match_pos = if forward { (self.match_pos + 1) % n } else { (self.match_pos + n - 1) % n };
        self.scroll_to = Some(ScrollTo::Center(self.matches[self.match_pos]));
    }

    fn receive(&mut self) {
        while let Ok(reply) = self.replies.try_recv() {
            match reply {
                Reply::Fetched(repo, result) => {
                    if self.repo.as_ref() != Some(&repo) {
                        continue;
                    }
                    match result {
                        Ok(()) => {
                            self.fetch_state = FetchState::Idle;
                            self.fetched_at = git::clock_now();
                            self.reload();
                        }
                        Err(e) => self.fetch_state = FetchState::Failed(e),
                    }
                }
                Reply::Done(repo, op, result) => {
                    self.running = None;
                    if self.repo.as_ref() != Some(&repo) {
                        continue;
                    }
                    if let Err(body) = result {
                        self.notice = Some(Notice { title: op.failed().into(), body, error: true });
                    }
                    // 실패해도 충돌 상태 등이 남을 수 있으니 항상 다시 읽는다.
                    self.refresh();
                }
                Reply::Loaded(generation, result) if generation == self.generation => {
                    self.loading = false;
                    self.refreshed_at = git::clock_now();
                    match result {
                        Ok(data) => {
                            self.error = None;
                            self.data = Some(data);
                            let q = self.search.clone();
                            if !q.trim().is_empty() {
                                // 새로고침 때문에 검색 위치가 튀지 않게 스크롤 요청은 버린다.
                                let keep = self.scroll_to;
                                self.update_matches();
                                self.scroll_to = keep;
                            }
                            self.refresh_selected();
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Reply::Loaded(..) => {}
                Reply::Details(repo, hash, result) => {
                    if self.repo.as_ref() == Some(&repo) && self.selected.as_ref() == Some(&hash) {
                        self.details = Some((hash, result));
                        self.refresh_diff();
                    }
                }
                Reply::Diff(repo, hash, path, result) => {
                    let current = self.diff.as_mut().filter(|d| d.hash == hash && d.file.path == path);
                    if let (true, Some(view)) = (self.repo.as_ref() == Some(&repo), current) {
                        view.result = Some(result);
                    }
                }
                Reply::Tree(repo, result) => {
                    if self.repo.as_ref() != Some(&repo) {
                        continue;
                    }
                    // 연 파일의 상태(수정됨, 새 파일 …)가 바뀌었을 수 있으니 새 상태로 다시 읽는다.
                    // 새 내용이 올 때까지는 이전 내용을 그대로 보여준다.
                    if let (Ok(tree), Some(view)) = (&result, &mut self.file_view) {
                        view.status = tree.nodes.iter().find(|n| !n.dir && n.path == view.path).and_then(|n| n.status);
                        let _ = self.jobs.send(Job::File(repo, view.path.clone(), view.status));
                    }
                    self.tree = Some(result);
                    self.update_tree_matches();
                    self.update_find_names();
                }
                Reply::File(repo, path, result) => {
                    let current = self.file_view.as_mut().filter(|v| v.path == path);
                    if let (true, Some(view)) = (self.repo.as_ref() == Some(&repo), current) {
                        view.result = Some(result);
                    }
                }
                Reply::Grep(repo, key, result) => {
                    if self.repo.as_ref() == Some(&repo) && self.find.ran.as_ref() == Some(&key) {
                        self.find.result = Some(result);
                        self.find.running = false;
                    }
                }
            }
        }
        if self.dirty.swap(false, Ordering::Relaxed) {
            self.refresh();
        }
        if self.find.due.is_some_and(|due| self.ctx.input(|i| i.time) >= due) {
            self.run_find();
        }
        // 같은 저장소를 열려던 다른 ggl(herdr 단축키 등)이 넘긴 요청: 이 창을 앞으로 가져온다.
        while let Ok(req) = self.remote_rx.try_recv() {
            if req == Request::Files && !self.settings.show_files {
                self.toggle_files();
            }
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    /// 커밋 안 된 변경의 상세가 새로 오면, 열려 있던 diff도 같은 파일로 다시 읽는다.
    fn refresh_diff(&mut self) {
        let Some(view) = &self.diff else { return };
        if view.hash != git::UNCOMMITTED {
            return;
        }
        let path = view.file.path.clone();
        let found = match &self.details {
            Some((_, Ok(d))) => d.files.iter().position(|f| f.path == path),
            _ => None,
        };
        match found {
            Some(i) => {
                let result = self.diff.as_mut().and_then(|v| v.result.take());
                self.show_diff(i);
                // 새 결과가 올 때까지 이전 내용을 계속 보여줘서 깜빡이지 않게 한다.
                if let Some(v) = self.diff.as_mut() {
                    v.result = result;
                }
            }
            None => self.diff = None,
        }
    }

    /// 새로고침 뒤: 선택한 커밋이 사라졌으면 상세를 닫고, 커밋 안 된 변경이면 다시 읽는다.
    fn refresh_selected(&mut self) {
        let Some(hash) = self.selected.clone() else { return };
        let exists = self.data.as_ref().is_some_and(|d| d.rows.contains_key(&hash));
        if !exists {
            self.close_details();
        } else if hash == git::UNCOMMITTED {
            if let Some(repo) = &self.repo {
                let _ = self.jobs.send(Job::Details(repo.clone(), hash));
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // 확인 창이 떠 있으면 키는 그 창이 받는다 (Enter 실행, Esc 취소).
        if self.confirm.is_some() || self.leaving.is_some() || self.editor.as_ref().is_some_and(|e| e.conflict) {
            return;
        }
        let typing = ctx.memory(|m| m.focused().is_some());
        let (fetch, refresh, head, find_files, find, open, files, save, esc, up, down) = ctx.input_mut(|i| {
            (
                // ⌘R 패턴은 Shift가 눌려 있어도 맞으므로 ⌘⇧R을 먼저 확인한다. ⌘⇧F도 마찬가지.
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::R),
                i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::COMMAND, Key::H),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::B),
                i.consume_key(Modifiers::COMMAND, Key::S),
                !typing && i.consume_key(Modifiers::NONE, Key::Escape),
                !typing && i.consume_key(Modifiers::NONE, Key::ArrowUp),
                !typing && i.consume_key(Modifiers::NONE, Key::ArrowDown),
            )
        });
        if fetch {
            self.fetch();
        } else if refresh {
            self.refresh();
        }
        if head {
            let row = self.data.as_ref().and_then(|d| d.rows.get(d.snap.head.as_ref()?).copied());
            if let Some(row) = row {
                self.scroll_to = Some(ScrollTo::Center(row));
            }
        }
        if find_files {
            self.open_find();
        } else if find {
            self.focus_search = true;
        }
        if save && self.edit_dirty() {
            self.save_edit(false);
        }
        if open {
            self.pick_folder();
        }
        if files {
            self.toggle_files();
        }
        if esc {
            if self.diff.is_some() {
                self.diff = None;
            } else if self.editor.is_some() {
                self.leave(Leave::StopEditing);
            } else if self.file_view.is_some() {
                self.leave(Leave::CloseFile);
            } else {
                self.close_details();
            }
        }
        if up || down {
            let count = self.data.as_ref().map_or(0, |d| d.snap.commits.len());
            let row = match self.selected_row() {
                Some(r) if up => r.saturating_sub(1),
                Some(r) => (r + 1).min(count.saturating_sub(1)),
                None => 0,
            };
            if count > 0 {
                self.select_row(row);
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 창을 닫을 때(⌘Q 포함) 저장 안 한 편집이 있으면 먼저 묻는다.
        if ui.ctx().input(|i| i.viewport().close_requested()) && self.edit_dirty() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.leaving = Some(Leave::Quit);
        }
        self.handle_keys(&ui.ctx().clone());
        let pal = Palette::of(ui);

        egui::Panel::top("toolbar")
            .frame(Frame::new().fill(pal.bg).inner_margin(Margin::symmetric(10, 7)))
            .show(ui, |ui| view::toolbar::show(self, ui));

        if view::ops::has_banner(self) {
            egui::Panel::top("banner")
                .frame(Frame::new().fill(pal.inline_bg).inner_margin(Margin::symmetric(10, 6)))
                .show(ui, |ui| view::ops::banner(self, ui));
        }

        // 파일 트리는 왼쪽에 위아래 전체 높이로 (오른쪽 경계를 끌어서 폭 조절)
        if self.settings.show_files {
            let w = ui.available_width();
            egui::Panel::left("files")
                .resizable(true)
                .default_size(280.0)
                .size_range(180.0..=(w * 0.6).max(180.0))
                .frame(Frame::new().fill(pal.bg))
                .show(ui, |ui| view::files::side(self, ui));
        }

        // diff와 트리에서 연 파일은 아래쪽에 남은 폭 전체로 (위 경계를 끌어서 높이 조절)
        if self.diff.is_some() || self.file_view.is_some() {
            let h = ui.available_height();
            egui::Panel::bottom("diff-bottom")
                .resizable(true)
                .default_size((h * 0.45).max(200.0))
                .size_range(150.0..=(h - 160.0).max(150.0))
                .frame(Frame::new().fill(pal.bg))
                .show(ui, |ui| {
                    if self.diff.is_some() {
                        view::diff::show(self, ui);
                    } else {
                        view::files::file(self, ui);
                    }
                });
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.bg))
            .show(ui, |ui| view::table::show(self, ui));

        view::ops::confirm(self, ui.ctx());
        view::edit::dialogs(self, ui.ctx());

        #[cfg(feature = "screenshot")]
        crate::devshot::tick(self, ui);
    }

    /// 글자 캐시(폰트 아틀라스) 폭을 2048로 제한한다.
    /// 기본값(GPU 최대치, 8192)이면 최악의 경우 가득 찰 때까지 수백 MB로 커질 수 있고,
    /// 2048이면 약 16MB를 넘기 전에 비우고 다시 채운다. (커밋 1,900개를 훑어도 실제로는 8MB 정도)
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.max_texture_side = Some(raw.max_texture_side.unwrap_or(2048).min(2048));
        #[cfg(feature = "screenshot")]
        crate::devshot::inject(raw);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "settings", &self.settings);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

pub fn repo_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn spawn_worker(ctx: egui::Context) -> (Sender<Job>, Receiver<Reply>, Sender<Reply>) {
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let (reply_tx, reply_rx) = mpsc::channel();
    let extra_tx = reply_tx.clone();
    thread::spawn(move || {
        while let Ok(first) = job_rx.recv() {
            // 밀린 작업이 있으면 종류별로 가장 최근 것만 처리한다.
            let mut load = None;
            let mut details = None;
            let mut diff = None;
            let mut tree = None;
            let mut file = None;
            for job in std::iter::once(first).chain(job_rx.try_iter()) {
                match job {
                    Job::Load(..) => load = Some(job),
                    Job::Details(..) => details = Some(job),
                    Job::Diff(..) => diff = Some(job),
                    Job::Tree(..) => tree = Some(job),
                    Job::File(..) => file = Some(job),
                }
            }
            if let Some(Job::Details(repo, hash)) = details {
                let result = git::details(&repo, &hash);
                if reply_tx.send(Reply::Details(repo, hash, result)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            if let Some(Job::Diff(repo, hash, parents, file)) = diff {
                let result = git::file_diff(&repo, &hash, &parents, &file);
                if reply_tx.send(Reply::Diff(repo, hash, file.path, result)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            if let Some(Job::File(repo, path, status)) = file {
                let result = git::worktree_file(&repo, &path, status);
                if reply_tx.send(Reply::File(repo, path, result)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            if let Some(Job::Tree(repo)) = tree {
                let result = git::tree_files(&repo).map(FileTree::build);
                if reply_tx.send(Reply::Tree(repo, result)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            if let Some(Job::Load(generation, req)) = load {
                let result = git::load(&req).map(Loaded::new);
                if reply_tx.send(Reply::Loaded(generation, result)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
    });
    (job_tx, reply_rx, extra_tx)
}
