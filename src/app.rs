//! 앱 상태. git 작업은 백그라운드 스레드에서 돌리고, 화면은 결과만 그린다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use eframe::egui::{self, Frame, Key, Margin, Modifiers};
use serde::{Deserialize, Serialize};

use crate::git::{self, Details, Diff, FileChange, LoadRequest, RefLabel, Snapshot};
use crate::graph::{self, Layout, OFFSCREEN};
use crate::ops::{self, Item, Op};
use crate::style::Palette;
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
}

impl Default for Settings {
    fn default() -> Self {
        Self { recent: Vec::new(), show_remotes: true, details_height: 260.0, details_split: 0.5 }
    }
}

/// 오른쪽에 열린 파일 diff
pub struct DiffView {
    pub hash: String,
    pub index: usize,
    pub file: FileChange,
    pub result: Option<Result<Diff, String>>,
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
    ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, arg_repo: Option<PathBuf>) -> Self {
        crate::style::install(&cc.egui_ctx);
        let settings: Settings =
            cc.storage.and_then(|s| eframe::get_value(s, "settings")).unwrap_or_default();
        let (jobs, replies, reply_tx) = spawn_worker(cc.egui_ctx.clone());
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
            ctx: cc.egui_ctx.clone(),
        };
        let start = arg_repo.or_else(|| app.settings.recent.first().cloned());
        if let Some(repo) = start {
            app.open_repo(&repo);
        }
        app
    }

    pub fn open_repo(&mut self, dir: &Path) {
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

        let title = format!("{} — ggl", repo_name(&repo));
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let dirty = self.dirty.clone();
        let ctx = self.ctx.clone();
        self.watcher = RepoWatcher::start(&repo, move || {
            dirty.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        })
        .ok();
        self.repo = Some(repo);
        self.reload();
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
        let Some((hash, Ok(d))) = &self.details else { return };
        let Some(file) = d.files.get(index).cloned() else { return };
        let (hash, parents) = (hash.clone(), d.parents.clone());
        self.request_diff(&hash, parents, file.clone());
        let opening = self.diff.is_none();
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
                    self.reload();
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
            }
        }
        if self.dirty.swap(false, Ordering::Relaxed) {
            self.reload();
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
                self.open_diff(i);
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
        if self.confirm.is_some() {
            return;
        }
        let typing = ctx.memory(|m| m.focused().is_some());
        let (fetch, refresh, head, find, open, esc, up, down) = ctx.input_mut(|i| {
            (
                // ⌘R 패턴은 Shift가 눌려 있어도 맞으므로 ⌘⇧R을 먼저 확인한다.
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::R),
                i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::COMMAND, Key::H),
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::O),
                !typing && i.consume_key(Modifiers::NONE, Key::Escape),
                !typing && i.consume_key(Modifiers::NONE, Key::ArrowUp),
                !typing && i.consume_key(Modifiers::NONE, Key::ArrowDown),
            )
        });
        if fetch {
            self.fetch();
        } else if refresh {
            self.reload();
        }
        if head {
            let row = self.data.as_ref().and_then(|d| d.rows.get(d.snap.head.as_ref()?).copied());
            if let Some(row) = row {
                self.scroll_to = Some(ScrollTo::Center(row));
            }
        }
        if find {
            self.focus_search = true;
        }
        if open {
            self.pick_folder();
        }
        if esc {
            if self.diff.is_some() {
                self.diff = None;
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

        // diff는 아래쪽에 가로 전체 폭으로 (위 경계를 끌어서 높이 조절)
        if self.diff.is_some() {
            let h = ui.available_height();
            egui::Panel::bottom("diff-bottom")
                .resizable(true)
                .default_size((h * 0.45).max(200.0))
                .size_range(150.0..=(h - 160.0).max(150.0))
                .frame(Frame::new().fill(pal.bg))
                .show(ui, |ui| view::diff::show(self, ui));
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.bg))
            .show(ui, |ui| view::table::show(self, ui));

        view::ops::confirm(self, ui.ctx());

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
            for job in std::iter::once(first).chain(job_rx.try_iter()) {
                match job {
                    Job::Load(..) => load = Some(job),
                    Job::Details(..) => details = Some(job),
                    Job::Diff(..) => diff = Some(job),
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
