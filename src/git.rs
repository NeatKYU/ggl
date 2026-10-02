//! git 명령을 실행하고 결과를 파싱한다.
//!
//! 읽는 명령에는 모두 `--no-optional-locks`를 붙여서 화면을 그리는 것만으로는 저장소에 아무것도 쓰지 않게 한다.
//! (`git status`가 `.git/index`를 갱신하면 파일 감시가 다시 새로고침을 부르는 루프가 생긴다.)
//! 저장소를 바꾸는 명령(fetch, checkout, merge …)은 `run`으로만 실행한다.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// git 출력 필드 구분자 (ASCII Unit Separator). 커밋 메시지에 나올 일이 없다.
const SEP: char = '\x1f';

#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Commit,
    Stash { selector: String },
    Uncommitted { count: usize },
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub date: String,
    pub time: i64,
    pub subject: String,
    pub kind: RowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Branch,
    Remote,
    Tag,
    Stash,
}

#[derive(Clone, Debug)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    /// 같은 커밋을 가리키는 같은 이름의 리모트 브랜치 (예: main 옆의 "origin")
    pub remotes: Vec<String>,
    pub is_head: bool,
}

#[derive(Clone, Debug)]
pub struct LoadRequest {
    pub repo: PathBuf,
    pub max: usize,
    /// None이면 모든 브랜치, Some이면 해당 ref 하나만
    pub branch: Option<String>,
    pub show_remotes: bool,
}

/// 충돌 등으로 중간에 멈춰 있는 작업
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InProgress {
    Merge,
    CherryPick,
    Rebase,
}

#[derive(Debug, Default)]
pub struct Snapshot {
    /// 화면에 표시될 순서 그대로 (맨 위 = 0)
    pub commits: Vec<Commit>,
    pub refs: HashMap<String, Vec<RefLabel>>,
    pub head: Option<String>,
    pub head_branch: Option<String>,
    /// 현재 브랜치가 따라가는 리모트 브랜치 (예: origin/main). 없으면 풀을 할 수 없다.
    pub upstream: Option<String>,
    /// 현재 브랜치에서 아직 upstream에 올리지 않은 커밋 수
    pub ahead: Option<usize>,
    /// 로컬 브랜치 → 따라가는 브랜치 (예: main → origin/main). 따라가는 게 없는 브랜치는 빠진다.
    pub upstreams: HashMap<String, String>,
    /// 리모트 이름들 (예: origin)
    pub remotes: Vec<String>,
    /// 리모트에 있는 브랜치 전부 (예: origin/main). 리모트 브랜치를 화면에서 숨겨도 채워진다.
    pub remote_refs: Vec<String>,
    pub in_progress: Option<InProgress>,
    pub more: bool,
    /// 브랜치 필터 목록 (로컬 먼저, 그다음 리모트)
    pub branches: Vec<String>,
    /// 리모트가 하나라도 있으면 true (리모트 새로고침 버튼용)
    pub has_remotes: bool,
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub status: char,
    pub path: String,
    pub old_path: Option<String>,
    /// 바이너리 파일이면 None
    pub added: Option<u32>,
    pub deleted: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Details {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub date: String,
    pub committer: String,
    pub commit_date: String,
    pub body: String,
    pub files: Vec<FileChange>,
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.quotepath=false"])
        .args(["-c", "color.ui=false", "-c", "log.showSignature=false"])
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("git을 실행할 수 없어요: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 네트워크를 쓰는 명령(fetch, pull)이 기다리는 최대 시간
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);

/// 저장소를 바꾸는 명령을 실행한다.
/// 비밀번호나 편집기를 기다리며 멈추지 않게 하고, `timeout`이 지나면 중단한다.
/// 실패하면 git이 남긴 말을 돌려준다 (충돌 안내는 stdout으로 나온다).
pub fn run(repo: &Path, args: &[&str], timeout: Option<Duration>) -> Result<(), String> {
    let mut child = Command::new("git")
        .args(["-c", "color.ui=false", "-C"])
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git을 실행할 수 없어요: {e}"))?;
    // 출력이 길어도 파이프가 막히지 않게 따로 읽는다.
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = timeout.map(|t| Instant::now() + t);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if deadline.is_some_and(|d| Instant::now() > d) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("2분이 지나도 끝나지 않아서 중단했어요 (네트워크나 인증을 확인해주세요)".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    };
    if status.success() {
        return Ok(());
    }
    // git이 띄운 다른 프로세스(ssh 등)가 파이프를 쥐고 있을 수 있어서 오래 기다리지 않는다.
    let text = |rx: Receiver<String>| rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
    let (out, err) = (text(out), text(err));
    let parts: Vec<&str> = [out.trim(), err.trim()].into_iter().filter(|s| !s.is_empty()).collect();
    Err(parts.join("\n"))
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    rx
}

/// 리모트 새로고침 (`git fetch --all --prune`).
/// 리모트 추적 브랜치(origin/…)와 태그만 바뀌고, 내 브랜치와 작업 파일은 그대로다.
/// 비밀번호를 물어보면 기다리지 않고 실패하고, 너무 오래 걸리면 중단한다.
pub fn fetch(repo: &Path) -> Result<(), String> {
    run(repo, &["fetch", "--all", "--prune", "--quiet"], Some(NETWORK_TIMEOUT))
        .map_err(|e| if e.is_empty() { "리모트를 가져오지 못했어요".into() } else { e })
}

/// `ancestor`가 `of`의 조상(또는 같은 커밋)인지
pub fn is_ancestor(repo: &Path, ancestor: &str, of: &str) -> bool {
    git(repo, &["merge-base", "--is-ancestor", ancestor, of]).is_ok()
}

/// 끝나지 않은 머지·체리픽·리베이스가 있는지 (git이 남겨둔 표시 파일로 안다)
fn in_progress(repo: &Path) -> Option<InProgress> {
    let dir = PathBuf::from(git(repo, &["rev-parse", "--absolute-git-dir"]).ok()?.trim());
    [
        ("MERGE_HEAD", InProgress::Merge),
        ("CHERRY_PICK_HEAD", InProgress::CherryPick),
        ("REBASE_HEAD", InProgress::Rebase),
    ]
    .into_iter()
    .find_map(|(file, state)| dir.join(file).exists().then_some(state))
}

/// 폴더 안의 git 저장소 최상위 경로를 찾는다.
pub fn toplevel(dir: &Path) -> Result<PathBuf, String> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .map(|s| PathBuf::from(s.trim()))
        .map_err(|_| format!("git 저장소가 아니에요: {}", dir.display()))
}

pub fn load(req: &LoadRequest) -> Result<Snapshot, String> {
    let repo = req.repo.as_path();
    let mut snap = Snapshot {
        head_branch: git(repo, &["symbolic-ref", "--short", "-q", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        ..Default::default()
    };

    snap.remotes = git(repo, &["remote"]).unwrap_or_default().lines().map(str::to_string).collect();
    snap.has_remotes = !snap.remotes.is_empty();
    snap.in_progress = in_progress(repo);
    if snap.head_branch.is_some() {
        snap.upstream = git(repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    if snap.upstream.is_some() {
        snap.ahead = git(repo, &["rev-list", "--count", "@{upstream}..HEAD"]).ok().and_then(|s| s.trim().parse().ok());
    }
    // 브랜치 이름에는 공백이 들어갈 수 없어서 공백으로 나눠도 안전하다.
    let tracking = git(repo, &["for-each-ref", "--format=%(refname:short) %(upstream:short)", "refs/heads"]);
    snap.upstreams = tracking
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(_, up)| !up.is_empty())
        .map(|(b, up)| (b.to_string(), up.to_string()))
        .collect();

    // 브랜치·태그 위치. 커밋이 하나도 없는 저장소에서는 실패하므로 빈 결과로 둔다.
    let show_ref = git(repo, &["show-ref", "-d", "--head"]).unwrap_or_default();
    let mut heads = Vec::new();
    let mut remotes = Vec::new();
    let mut tags: Vec<(String, String)> = Vec::new();
    for line in show_ref.lines() {
        let Some((hash, name)) = line.split_once(' ') else { continue };
        if name == "HEAD" {
            snap.head = Some(hash.to_string());
        } else if let Some(n) = name.strip_prefix("refs/heads/") {
            heads.push((n.to_string(), hash.to_string()));
        } else if let Some(n) = name.strip_prefix("refs/remotes/") {
            if !n.ends_with("/HEAD") {
                snap.remote_refs.push(n.to_string());
                if req.show_remotes {
                    remotes.push((n.to_string(), hash.to_string()));
                }
            }
        } else if let Some(n) = name.strip_prefix("refs/tags/") {
            // 주석 태그는 "v1^{}" 줄이 실제 커밋을 가리킨다.
            let (n, peeled) = match n.strip_suffix("^{}") {
                Some(n) => (n, true),
                None => (n, false),
            };
            match tags.iter_mut().find(|(t, _)| t == n) {
                Some(t) if peeled => t.1 = hash.to_string(),
                Some(_) => {}
                None => tags.push((n.to_string(), hash.to_string())),
            }
        }
    }

    let Some(head) = snap.head.clone() else {
        // 아직 커밋이 없는 저장소
        let count = uncommitted_count(repo);
        if count > 0 {
            snap.commits.push(uncommitted_row(count, None));
        }
        return Ok(snap);
    };

    let stashes = if req.branch.is_none() { stashes(repo) } else { Vec::new() };

    // 커밋 목록. 한 개 더 받아서 "더 있음" 여부를 판단한다.
    let max = (req.max + 1).to_string();
    let mut args = vec![
        "log".to_string(),
        format!("--max-count={max}"),
        format!("--format=%H{SEP}%P{SEP}%an{SEP}%at{SEP}%s"),
        "--date-order".to_string(),
    ];
    match &req.branch {
        Some(b) => args.push(match b.strip_prefix("remotes/") {
            Some(r) => format!("refs/remotes/{r}"),
            None => format!("refs/heads/{b}"),
        }),
        None => {
            args.push("--branches".into());
            args.push("--tags".into());
            if req.show_remotes {
                args.push("--remotes".into());
            }
            for s in &stashes {
                if let Some(base) = s.parents.first() {
                    args.push(base.clone());
                }
            }
            args.push("HEAD".into());
        }
    }
    args.push("--".into());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = git(repo, &args)?;

    let mut commits: Vec<Commit> = out.lines().filter_map(parse_log_line).collect();
    if commits.len() > req.max {
        commits.truncate(req.max);
        snap.more = true;
    }

    // 스태시는 기준 커밋 바로 위(시간 순서에 맞는 자리)에 끼워 넣는다.
    let mut stash_labels = Vec::new();
    for s in stashes {
        let Some(base) = s.parents.first().cloned() else { continue };
        if commits.iter().any(|c| c.hash == s.hash) {
            continue;
        }
        let Some(mut pos) = commits.iter().position(|c| c.hash == base) else { continue };
        while pos > 0 && commits[pos - 1].time < s.time {
            pos -= 1;
        }
        let RowKind::Stash { selector } = &s.kind else { continue };
        stash_labels.push((s.hash.clone(), selector.clone()));
        commits.insert(pos, Commit { parents: vec![base], ..s });
    }

    // 커밋 안 된 변경은 HEAD가 목록에 있을 때만 맨 위에 보여준다.
    if commits.iter().any(|c| c.hash == head) {
        let count = uncommitted_count(repo);
        if count > 0 {
            commits.insert(0, uncommitted_row(count, Some(head.clone())));
        }
    }
    snap.commits = commits;

    // 라벨 만들기: 로컬 브랜치와 같은 이름·같은 위치의 리모트 브랜치는 하나로 합친다.
    let mut used_remote = vec![false; remotes.len()];
    let mut add = |hash: &str, label: RefLabel| {
        snap.refs.entry(hash.to_string()).or_default().push(label);
    };
    for (name, hash) in &heads {
        let mut merged = Vec::new();
        for (i, (rname, rhash)) in remotes.iter().enumerate() {
            if let Some((remote, branch)) = rname.split_once('/') {
                if branch == name && rhash == hash {
                    merged.push(remote.to_string());
                    used_remote[i] = true;
                }
            }
        }
        let is_head = snap.head_branch.as_deref() == Some(name.as_str());
        add(hash, RefLabel { name: name.clone(), kind: RefKind::Branch, remotes: merged, is_head });
    }
    for (i, (name, hash)) in remotes.iter().enumerate() {
        if !used_remote[i] {
            add(hash, RefLabel { name: name.clone(), kind: RefKind::Remote, remotes: vec![], is_head: false });
        }
    }
    for (name, hash) in &tags {
        add(hash, RefLabel { name: name.clone(), kind: RefKind::Tag, remotes: vec![], is_head: false });
    }
    for (hash, selector) in stash_labels {
        add(&hash, RefLabel { name: selector, kind: RefKind::Stash, remotes: vec![], is_head: false });
    }
    // HEAD 브랜치 라벨이 항상 맨 앞에 오게 한다.
    for labels in snap.refs.values_mut() {
        labels.sort_by_key(|l| (!l.is_head, l.kind as u8));
    }

    // 브랜치 필터 목록
    let mut local: Vec<&String> = heads.iter().map(|(n, _)| n).collect();
    local.sort_by_key(|n| (snap.head_branch.as_ref() != Some(*n), n.to_lowercase()));
    snap.branches = local.into_iter().cloned().collect();
    snap.branches.extend(remotes.iter().map(|(n, _)| format!("remotes/{n}")));
    Ok(snap)
}

fn parse_log_line(line: &str) -> Option<Commit> {
    let f: Vec<&str> = line.splitn(5, SEP).collect();
    if f.len() != 5 {
        return None;
    }
    let time = f[3].parse().unwrap_or(0);
    Some(Commit {
        hash: f[0].to_string(),
        parents: f[1].split_whitespace().map(str::to_string).collect(),
        author: f[2].to_string(),
        date: format_time(time),
        time,
        subject: f[4].to_string(),
        kind: RowKind::Commit,
    })
}

fn stashes(repo: &Path) -> Vec<Commit> {
    let format = format!("--format=%H{SEP}%P{SEP}%gD{SEP}%an{SEP}%at{SEP}%s");
    let Ok(out) = git(repo, &["reflog", &format, "refs/stash", "--"]) else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(6, SEP).collect();
            if f.len() != 6 || f[1].is_empty() {
                return None;
            }
            let time = f[4].parse().unwrap_or(0);
            Some(Commit {
                hash: f[0].to_string(),
                parents: f[1].split_whitespace().map(str::to_string).collect(),
                author: f[3].to_string(),
                date: format_time(time),
                time,
                subject: f[5].to_string(),
                kind: RowKind::Stash { selector: f[2].trim_start_matches("refs/").to_string() },
            })
        })
        .collect()
}

fn uncommitted_count(repo: &Path) -> usize {
    git(repo, &["status", "--porcelain", "--untracked-files=all"])
        .map(|s| s.lines().filter(|l| !l.is_empty()).count())
        .unwrap_or(0)
}

fn uncommitted_row(count: usize, head: Option<String>) -> Commit {
    Commit {
        hash: UNCOMMITTED.to_string(),
        parents: head.into_iter().collect(),
        author: String::new(),
        date: String::new(),
        time: i64::MAX,
        subject: format!("커밋 안 된 변경 ({count})"),
        kind: RowKind::Uncommitted { count },
    }
}

/// 커밋 안 된 변경 행의 가짜 해시
pub const UNCOMMITTED: &str = "*";

pub fn details(repo: &Path, hash: &str) -> Result<Details, String> {
    if hash == UNCOMMITTED {
        return uncommitted_details(repo);
    }
    let format = format!("--format=%H{SEP}%P{SEP}%an{SEP}%ae{SEP}%at{SEP}%cn{SEP}%ct{SEP}%B");
    let out = git(repo, &["show", "-s", &format, hash, "--"])?;
    let f: Vec<&str> = out.splitn(8, SEP).collect();
    if f.len() != 8 {
        return Err("커밋 정보를 읽지 못했어요".into());
    }
    let parents: Vec<String> = f[1].split_whitespace().map(str::to_string).collect();
    // 머지 커밋·스태시는 첫 번째 부모와 비교한다 (원본 Git Graph와 같음).
    let files = match parents.first() {
        Some(p) => diff_files(repo, &["diff", "-M", p.as_str(), hash])?,
        None => diff_files(repo, &["diff-tree", "--root", "-r", "--no-commit-id", "-M", hash])?,
    };
    Ok(Details {
        hash: f[0].to_string(),
        parents,
        author: f[2].to_string(),
        email: f[3].to_string(),
        date: format_time(f[4].parse().unwrap_or(0)),
        committer: f[5].to_string(),
        commit_date: format_time(f[6].parse().unwrap_or(0)),
        body: f[7].trim_end().to_string(),
        files,
    })
}

fn uncommitted_details(repo: &Path) -> Result<Details, String> {
    let has_head = git(repo, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
    let mut files = if has_head { diff_files(repo, &["diff", "-M", "HEAD"])? } else { Vec::new() };
    let untracked = git(repo, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    files.extend(untracked.split('\0').filter(|p| !p.is_empty()).map(|p| FileChange {
        status: 'U',
        path: p.to_string(),
        old_path: None,
        added: None,
        deleted: None,
    }));
    Ok(Details {
        hash: UNCOMMITTED.to_string(),
        body: "커밋 안 된 변경".into(),
        files,
        ..Default::default()
    })
}

/// `--name-status`와 `--numstat` 결과를 합쳐서 파일별 변경 내역을 만든다.
fn diff_files(repo: &Path, base: &[&str]) -> Result<Vec<FileChange>, String> {
    let with = |extra: &str| {
        let mut args = base.to_vec();
        args.extend([extra, "-z"]);
        git(repo, &args)
    };
    let mut files = parse_name_status(&with("--name-status")?);
    let stats = parse_numstat(&with("--numstat")?);
    for f in &mut files {
        if let Some(&(a, d)) = stats.get(&f.path) {
            f.added = a;
            f.deleted = d;
        }
    }
    Ok(files)
}

fn parse_name_status(out: &str) -> Vec<FileChange> {
    let mut tokens = out.split('\0').filter(|t| !t.is_empty());
    let mut files = Vec::new();
    while let Some(status) = tokens.next() {
        let letter = status.chars().next().unwrap_or('M');
        let (old_path, path) = if letter == 'R' || letter == 'C' {
            let old = tokens.next().unwrap_or_default().to_string();
            (Some(old), tokens.next().unwrap_or_default().to_string())
        } else {
            (None, tokens.next().unwrap_or_default().to_string())
        };
        files.push(FileChange { status: letter, path, old_path, added: None, deleted: None });
    }
    files
}

type Stat = (Option<u32>, Option<u32>);

fn parse_numstat(out: &str) -> HashMap<String, Stat> {
    let mut tokens = out.split('\0');
    let mut stats = HashMap::new();
    while let Some(tok) = tokens.next() {
        let mut parts = tok.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        // 이름이 바뀐 파일은 경로 칸이 비어 있고 이전 경로, 새 경로가 따로 온다.
        let path = if path.is_empty() {
            tokens.next();
            tokens.next().unwrap_or_default()
        } else {
            path
        };
        stats.insert(path.to_string(), (a.parse().ok(), d.parse().ok()));
    }
    stats
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// `@@ -1,3 +1,4 @@` 구간 머리
    Hunk,
    Context,
    Added,
    Deleted,
    /// `\ No newline at end of file`
    Meta,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    /// 0이면 해당 쪽에 줄 번호 없음
    pub old: u32,
    pub new: u32,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Diff {
    pub lines: Vec<DiffLine>,
    pub binary: bool,
    /// 너무 길어서 잘랐으면 true
    pub truncated: bool,
    /// 가장 긴 줄의 표시 폭 (한글·한자는 2칸)
    pub max_cols: usize,
    /// 변경이 아니라 파일 내용 그대로면 true (줄 번호 칸을 하나만 쓴다)
    pub plain: bool,
}

const MAX_DIFF_LINES: usize = 50_000;
const MAX_UNTRACKED_BYTES: u64 = 2 * 1024 * 1024;

/// 파일 하나의 변경 내용. 목록과 같은 기준(첫 번째 부모, 커밋 안 된 변경은 HEAD)으로 비교한다.
pub fn file_diff(repo: &Path, hash: &str, parents: &[String], file: &FileChange) -> Result<Diff, String> {
    if file.status == 'U' {
        return untracked_diff(repo, &file.path);
    }
    let mut paths: Vec<&str> = file.old_path.iter().map(String::as_str).collect();
    paths.push(&file.path);

    let mut args = vec!["--no-ext-diff", "-M"];
    let base: Vec<&str> = if hash == UNCOMMITTED {
        vec!["diff", "HEAD"]
    } else if let Some(p) = parents.first() {
        vec!["diff", p.as_str(), hash]
    } else {
        vec!["diff-tree", "-p", "--root", "--no-commit-id", hash]
    };
    args.splice(0..0, base);
    args.push("--");
    args.extend(paths);
    Ok(parse_diff(&git(repo, &args)?))
}

fn parse_diff(out: &str) -> Diff {
    let mut diff = Diff::default();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in out.lines() {
        if diff.lines.len() >= MAX_DIFF_LINES {
            diff.truncated = true;
            break;
        }
        if let Some(rest) = raw.strip_prefix("@@") {
            // "@@ -a,b +c,d @@ 함수 이름"
            let mut nums = rest.split_whitespace();
            old = nums.next().and_then(|s| start_line(s, '-')).unwrap_or(0);
            new = nums.next().and_then(|s| start_line(s, '+')).unwrap_or(0);
            in_hunk = true;
            push_line(&mut diff, LineKind::Hunk, 0, 0, raw);
            continue;
        }
        if !in_hunk {
            if raw.starts_with("Binary files") {
                diff.binary = true;
            }
            continue;
        }
        let (kind, text) = match raw.as_bytes().first() {
            Some(b'+') => (LineKind::Added, &raw[1..]),
            Some(b'-') => (LineKind::Deleted, &raw[1..]),
            Some(b' ') => (LineKind::Context, &raw[1..]),
            Some(b'\\') => (LineKind::Meta, raw),
            // 빈 줄은 (공백이 지워진) 문맥 줄
            None => (LineKind::Context, ""),
            // 다음 파일의 머리가 시작됨
            _ => {
                in_hunk = false;
                continue;
            }
        };
        let (o, n) = match kind {
            LineKind::Added => (0, next(&mut new)),
            LineKind::Deleted => (next(&mut old), 0),
            LineKind::Context => (next(&mut old), next(&mut new)),
            _ => (0, 0),
        };
        push_line(&mut diff, kind, o, n, text);
    }
    diff
}

fn start_line(s: &str, sign: char) -> Option<u32> {
    s.strip_prefix(sign)?.split(',').next()?.parse().ok()
}

fn next(n: &mut u32) -> u32 {
    let cur = *n;
    *n += 1;
    cur
}

fn push_line(diff: &mut Diff, kind: LineKind, old: u32, new: u32, text: &str) {
    let text = text.trim_end_matches('\r').replace('\t', "    ");
    diff.max_cols = diff.max_cols.max(display_width(&text));
    diff.lines.push(DiffLine { kind, old, new, text });
}

/// 추적 안 되는 새 파일은 git diff로 볼 수 없어서 직접 읽어 "전부 추가"로 보여준다.
fn untracked_diff(repo: &Path, path: &str) -> Result<Diff, String> {
    let mut diff = Diff::default();
    let Some(text) = read_text(&repo.join(path), &mut diff)? else { return Ok(diff) };
    let count = text.lines().count();
    push_line(&mut diff, LineKind::Hunk, 0, 0, &format!("@@ -0,0 +1,{count} @@ 새 파일"));
    for (i, line) in text.lines().enumerate().take(MAX_DIFF_LINES) {
        push_line(&mut diff, LineKind::Added, 0, i as u32 + 1, line);
    }
    diff.truncated = count > MAX_DIFF_LINES;
    Ok(diff)
}

/// 작업 폴더의 파일을 글자로 읽는다. 너무 크거나 바이너리면 `diff`에 표시하고 None을 돌려준다.
fn read_text(full: &Path, diff: &mut Diff) -> Result<Option<String>, String> {
    let meta = std::fs::metadata(full).map_err(|e| format!("파일을 읽을 수 없어요: {e}"))?;
    if meta.is_dir() {
        return Err("폴더예요 (하위 모듈일 수 있어요)".into());
    }
    if meta.len() > MAX_UNTRACKED_BYTES {
        diff.truncated = true;
        return Ok(None);
    }
    let bytes = std::fs::read(full).map_err(|e| format!("파일을 읽을 수 없어요: {e}"))?;
    if bytes.iter().take(8000).any(|&b| b == 0) {
        diff.binary = true;
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// 파일 트리에 보여줄 파일 하나
#[derive(Clone, Debug)]
pub struct TreeFile {
    pub path: String,
    /// 커밋 안 된 변경 (M, A, D, R, T, U = 추적 안 됨, ! = 충돌). 그대로면 None
    pub status: Option<char>,
}

/// 파일 트리용 목록: 추적 중인 파일과 무시되지 않은 새 파일. `.gitignore`에 걸린 파일은 빠진다.
pub fn tree_files(repo: &Path) -> Result<Vec<TreeFile>, String> {
    let listed = git(repo, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?;
    let mut status = worktree_status(repo);
    let mut seen = HashSet::new();
    // 충돌 중인 파일은 단계마다 한 번씩 나오므로 한 번만 넣는다.
    let mut files: Vec<TreeFile> = listed
        .split('\0')
        .filter(|p| !p.is_empty() && seen.insert(*p))
        .map(|p| TreeFile { path: p.to_string(), status: status.remove(p) })
        .collect();
    // `git rm`으로 지운 파일은 목록에 없지만, 지워진 것도 변경이라 보여준다.
    files.extend(
        status.into_iter().filter(|(_, s)| *s == 'D').map(|(path, s)| TreeFile { path, status: Some(s) }),
    );
    Ok(files)
}

/// 커밋 안 된 변경이 있는 파일과 그 상태
fn worktree_status(repo: &Path) -> HashMap<String, char> {
    parse_status(&git(repo, &["status", "--porcelain", "-z", "--untracked-files=all"]).unwrap_or_default())
}

fn parse_status(out: &str) -> HashMap<String, char> {
    let mut map = HashMap::new();
    let mut tokens = out.split('\0');
    while let Some(entry) = tokens.next() {
        let b = entry.as_bytes();
        if b.len() < 4 {
            continue;
        }
        let (x, y) = (b[0] as char, b[1] as char);
        // 이름이 바뀐 파일은 다음 칸에 이전 경로가 온다.
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            tokens.next();
        }
        let s = match (x, y) {
            ('?', _) => 'U',
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => '!',
            ('A' | 'R', _) => x,
            (_, ' ') => x,
            _ => y,
        };
        map.insert(entry[3..].to_string(), s);
    }
    map
}

/// 파일 트리에서 연 파일의 내용.
/// 바뀐 파일은 전체 내용에 바뀐 줄을 표시하고(HEAD와 비교), 그대로인 파일은 내용만 보여준다.
pub fn worktree_file(repo: &Path, path: &str, status: Option<char>) -> Result<Diff, String> {
    match status {
        Some('U') => untracked_diff(repo, path),
        Some(s) if git(repo, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() => {
            if s != 'D' {
                // 너무 큰 파일은 git diff를 돌리기 전에 거른다.
                let mut diff = Diff::default();
                if read_text(&repo.join(path), &mut diff)?.is_none() {
                    return Ok(diff);
                }
            }
            let mut d = parse_diff(&git(repo, &["diff", "--no-ext-diff", "-U1000000", "HEAD", "--", path])?);
            // 문맥을 파일 전체로 잡았으니 구간은 하나뿐이다. 구간 머리는 보여줄 필요가 없다.
            d.lines.retain(|l| l.kind != LineKind::Hunk);
            // 권한만 바뀌었거나 되돌린 직후면 diff가 비어 있다.
            if d.lines.is_empty() && !d.binary { plain_file(repo, path) } else { Ok(d) }
        }
        _ => plain_file(repo, path),
    }
}

/// 바뀌지 않은 파일: 내용 그대로, 줄 번호 하나
fn plain_file(repo: &Path, path: &str) -> Result<Diff, String> {
    let mut diff = Diff { plain: true, ..Default::default() };
    let Some(text) = read_text(&repo.join(path), &mut diff)? else { return Ok(diff) };
    let count = text.lines().count();
    for (i, line) in text.lines().enumerate().take(MAX_DIFF_LINES) {
        push_line(&mut diff, LineKind::Context, 0, i as u32 + 1, line);
    }
    diff.truncated = count > MAX_DIFF_LINES;
    Ok(diff)
}

/// 고정폭 글꼴에서의 대략적인 폭 (한글·한자·이모지는 2칸)
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if (c as u32) >= 0x1100 { 2 } else { 1 }).sum()
}

/// 유닉스 시간을 로컬 시간 "2026-09-30 14:22"로 바꾼다.
pub fn format_time(ts: i64) -> String {
    local_tm(ts).map_or_else(String::new, |tm| {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        )
    })
}

/// 지금 시각 "14:22:05"
pub fn clock_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    local_tm(now).map_or_else(String::new, |tm| {
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    })
}

fn local_tm(ts: i64) -> Option<libc::tm> {
    let t = ts as libc::time_t;
    // SAFETY: localtime_r은 스레드 안전하고, tm은 호출이 채워준다.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&t, &mut tm).is_null()).then_some(tm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_line() {
        let line = format!("abc{SEP}p1 p2{SEP}sun{SEP}0{SEP}Merge: a{SEP}b");
        let c = parse_log_line(&line).unwrap();
        assert_eq!(c.parents, vec!["p1", "p2"]);
        assert_eq!(c.subject, format!("Merge: a{SEP}b"));
    }

    #[test]
    fn parses_unified_diff() {
        let out = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n\
                   @@ -10,3 +10,3 @@ fn main\n ctx\n-old\tline\n+새 줄\n\\ No newline at end of file\n";
        let d = parse_diff(out);
        let kinds: Vec<LineKind> = d.lines.iter().map(|l| l.kind).collect();
        use LineKind::*;
        assert_eq!(kinds, vec![Hunk, Context, Deleted, Added, Meta]);
        assert_eq!((d.lines[1].old, d.lines[1].new), (10, 10));
        assert_eq!((d.lines[2].old, d.lines[2].new), (11, 0));
        assert_eq!((d.lines[3].old, d.lines[3].new), (0, 11));
        assert_eq!(d.lines[2].text, "old    line");
        assert!(!d.binary);

        let bin = parse_diff("diff --git a/i b/i\nBinary files a/i and b/i differ\n");
        assert!(bin.binary && bin.lines.is_empty());
    }

    #[test]
    fn parses_renames() {
        let ns = "M\0a.txt\0R100\0old.rs\0new.rs\0";
        let files = parse_name_status(ns);
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, "new.rs");
        assert_eq!(files[1].old_path.as_deref(), Some("old.rs"));

        let num = "3\t1\ta.txt\0" .to_string() + "0\t0\t\0old.rs\0new.rs\0-\t-\timg.png\0";
        let stats = parse_numstat(&num);
        assert_eq!(stats["a.txt"], (Some(3), Some(1)));
        assert_eq!(stats["new.rs"], (Some(0), Some(0)));
        assert_eq!(stats["img.png"], (None, None));
    }

    #[test]
    fn parses_porcelain_status() {
        let out = " M src/a.rs\0A  new.rs\0AM added-then-edited.rs\0R  b.rs\0old-b.rs\0\
                   ?? 새 파일.txt\0UU conflict.rs\0D  gone.rs\0 D missing.rs\0";
        let s = parse_status(out);
        assert_eq!(s["src/a.rs"], 'M');
        assert_eq!(s["new.rs"], 'A');
        assert_eq!(s["added-then-edited.rs"], 'A');
        assert_eq!(s["b.rs"], 'R');
        assert!(!s.contains_key("old-b.rs"));
        assert_eq!(s["새 파일.txt"], 'U');
        assert_eq!(s["conflict.rs"], '!');
        assert_eq!(s["gone.rs"], 'D');
        assert_eq!(s["missing.rs"], 'D');
    }
}
