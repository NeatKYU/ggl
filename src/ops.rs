//! 저장소를 바꾸는 작업: 체크아웃, 풀, 푸시, 머지, 체리픽.
//! 어떤 커밋에서 무엇을 할 수 있는지는 여기서 정하고, 화면(더블클릭·오른쪽 클릭 메뉴)은 그대로 보여주기만 한다.

use std::path::Path;

use crate::git::{self, InProgress, RefKind, RefLabel, RowKind, Snapshot};

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// 로컬 브랜치로 전환
    Checkout(String),
    /// 리모트 브랜치(origin/x)를 따라가는 로컬 브랜치 x를 만들어 전환
    CheckoutRemote(String),
    /// 리모트의 변경을 받아 현재 브랜치에 합침
    Pull,
    /// 로컬 브랜치를 리모트에 올림. `refspec`은 `main` 또는 `로컬:리모트` 이름.
    /// `set_upstream`이면 리모트에 새 브랜치를 만들고 따라가게 설정한다. 강제 푸시는 하지 않는다.
    Push { remote: String, refspec: String, set_upstream: bool },
    /// 브랜치나 커밋을 현재 브랜치에 머지
    Merge(String),
    /// 커밋 하나를 현재 브랜치에 복사
    CherryPick(String),
    /// 충돌로 멈춰 있는 머지·체리픽·리베이스를 취소
    Abort(InProgress),
}

impl Op {
    fn args(&self) -> Vec<&str> {
        match self {
            Op::Checkout(branch) => vec!["switch", branch],
            Op::CheckoutRemote(remote) => vec!["switch", "--track", remote],
            Op::Pull => vec!["pull"],
            Op::Push { remote, refspec, set_upstream } => {
                let mut args = vec!["push"];
                if *set_upstream {
                    args.push("--set-upstream");
                }
                args.extend([remote.as_str(), refspec.as_str()]);
                args
            }
            Op::Merge(what) => vec!["merge", "--no-edit", what],
            Op::CherryPick(hash) => vec!["cherry-pick", hash],
            Op::Abort(InProgress::Merge) => vec!["merge", "--abort"],
            Op::Abort(InProgress::CherryPick) => vec!["cherry-pick", "--abort"],
            Op::Abort(InProgress::Rebase) => vec!["rebase", "--abort"],
        }
    }

    /// 확인 창에 보여줄 명령 (해시는 짧게)
    pub fn command(&self) -> String {
        let args: Vec<&str> = self.args().into_iter().map(short).collect();
        format!("git {}", args.join(" "))
    }

    /// 체크아웃은 바로 실행하고, 커밋 기록이나 작업 내용을 바꾸는 작업은 먼저 물어본다.
    pub fn needs_confirm(&self) -> bool {
        !matches!(self, Op::Checkout(_) | Op::CheckoutRemote(_))
    }

    /// 실행하는 동안 보여줄 말
    pub fn progress(&self) -> &'static str {
        match self {
            Op::Checkout(_) | Op::CheckoutRemote(_) => "체크아웃하는 중…",
            Op::Pull => "풀 받는 중…",
            Op::Push { .. } => "푸시하는 중…",
            Op::Merge(_) => "머지하는 중…",
            Op::CherryPick(_) => "체리픽하는 중…",
            Op::Abort(_) => "취소하는 중…",
        }
    }

    /// 실패했을 때의 제목
    pub fn failed(&self) -> &'static str {
        match self {
            Op::Checkout(_) | Op::CheckoutRemote(_) => "체크아웃하지 못했어요",
            Op::Pull => "풀 받지 못했어요",
            Op::Push { .. } => "푸시하지 못했어요",
            Op::Merge(_) => "머지하지 못했어요",
            Op::CherryPick(_) => "체리픽하지 못했어요",
            Op::Abort(_) => "취소하지 못했어요",
        }
    }
}

/// 메뉴 한 줄: 보여줄 말과 실행할 작업
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub text: String,
    pub op: Op,
}

fn item(text: String, op: Op) -> Item {
    Item { text, op }
}

fn short(s: &str) -> &str {
    let is_hash = s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hash { &s[..7] } else { s }
}

fn labels<'a>(snap: &'a Snapshot, hash: &str) -> &'a [RefLabel] {
    snap.refs.get(hash).map_or(&[], Vec::as_slice)
}

/// 라벨 하나를 체크아웃하는 작업. 이미 그 브랜치에 있거나 브랜치가 아니면 None.
pub fn checkout(snap: &Snapshot, label: &RefLabel) -> Option<Item> {
    match label.kind {
        RefKind::Branch if !label.is_head => {
            Some(item(format!("체크아웃: {}", label.name), Op::Checkout(label.name.clone())))
        }
        RefKind::Remote => {
            let (_, branch) = label.name.split_once('/')?;
            if snap.head_branch.as_deref() == Some(branch) {
                return None;
            }
            // 같은 이름의 로컬 브랜치가 (다른 커밋에) 이미 있으면 그쪽으로 간다.
            Some(if snap.branches.iter().any(|b| b == branch) {
                item(format!("체크아웃: {branch}"), Op::Checkout(branch.to_string()))
            } else {
                let text = format!("체크아웃: {} (로컬 브랜치 {branch} 만들기)", label.name);
                item(text, Op::CheckoutRemote(label.name.clone()))
            })
        }
        _ => None,
    }
}

/// 이 행에서 체크아웃할 수 있는 브랜치 (로컬 먼저, 그다음 리모트)
pub fn checkouts(snap: &Snapshot, row: usize) -> Vec<Item> {
    let Some(commit) = snap.commits.get(row) else { return Vec::new() };
    if commit.kind != RowKind::Commit {
        return Vec::new();
    }
    labels(snap, &commit.hash).iter().filter_map(|l| checkout(snap, l)).collect()
}

/// 로컬 브랜치를 푸시하는 메뉴 줄.
/// 따라가는 리모트 브랜치가 있으면 그쪽으로, 없으면 리모트마다 올리면서 따라가게 설정하는 줄을 만든다.
pub fn pushes(snap: &Snapshot, branch: &str) -> Vec<Item> {
    let Some(upstream) = snap.upstreams.get(branch) else {
        return snap
            .remotes
            .iter()
            .map(|remote| {
                let op = Op::Push { remote: remote.clone(), refspec: branch.to_string(), set_upstream: true };
                // 리모트에 같은 이름의 브랜치가 이미 있으면 그쪽으로 올리면서 따라가게만 설정한다.
                let target = format!("{remote}/{branch}");
                let text = if snap.remote_refs.contains(&target) {
                    format!("푸시: {branch} → {target} (따라가게 설정)")
                } else {
                    format!("푸시: {branch} → {remote} (새 브랜치)")
                };
                item(text, op)
            })
            .collect();
    };
    // origin/main → (origin, main). 리모트 이름에 /가 들어갈 수 있어서 아는 리모트 이름으로 자른다.
    // 로컬 브랜치를 따라가는 경우(리모트가 아님)에는 푸시할 곳이 없다.
    let split = snap
        .remotes
        .iter()
        .filter_map(|r| Some((r, upstream.strip_prefix(r.as_str())?.strip_prefix('/')?)))
        .max_by_key(|(r, _)| r.len());
    let Some((remote, target)) = split else { return Vec::new() };
    let refspec = if target == branch { branch.to_string() } else { format!("{branch}:{target}") };
    let op = Op::Push { remote: remote.clone(), refspec, set_upstream: false };
    vec![item(format!("푸시: {branch} → {upstream}"), op)]
}

/// 멈춰 있는 작업을 취소하는 메뉴 줄
pub fn abort(state: InProgress) -> Item {
    let text = match state {
        InProgress::Merge => "머지 취소",
        InProgress::CherryPick => "체리픽 취소",
        InProgress::Rebase => "리베이스 취소",
    };
    item(text.to_string(), Op::Abort(state))
}

/// 멈춰 있는 작업을 알리는 말
pub fn stalled(state: InProgress) -> &'static str {
    match state {
        InProgress::Merge => "머지가 끝나지 않았어요. 충돌을 해결해서 커밋하거나 취소할 수 있어요.",
        InProgress::CherryPick => "체리픽이 끝나지 않았어요. 충돌을 해결해서 커밋하거나 취소할 수 있어요.",
        InProgress::Rebase => "리베이스가 끝나지 않았어요. 충돌을 해결해서 이어가거나 취소할 수 있어요.",
    }
}

/// 커밋 행의 오른쪽 클릭 메뉴. 묶음 사이에는 구분선을 긋는다.
pub fn menu(snap: &Snapshot, row: usize) -> Vec<Vec<Item>> {
    let Some(commit) = snap.commits.get(row) else { return Vec::new() };
    // 멈춘 작업이 있으면 다른 작업은 어차피 실패하니 취소만 보여준다.
    if let Some(state) = snap.in_progress {
        return vec![vec![abort(state)]];
    }
    let target = snap.head_branch.as_deref().unwrap_or("HEAD");
    let mut groups = vec![checkouts(snap, row)];

    let is_head = snap.head.as_ref() == Some(&commit.hash);
    if commit.kind == RowKind::Commit && !is_head {
        let mut apply: Vec<Item> = labels(snap, &commit.hash)
            .iter()
            .filter(|l| matches!(l.kind, RefKind::Branch | RefKind::Remote))
            .map(|l| item(format!("머지: {} → {target}", l.name), Op::Merge(l.name.clone())))
            .collect();
        if apply.is_empty() {
            let text = format!("머지: {} → {target}", short(&commit.hash));
            apply.push(item(text, Op::Merge(commit.hash.clone())));
        }
        // 머지 커밋은 어느 부모 기준인지 정해야 해서 체리픽 대상에서 뺀다.
        if commit.parents.len() <= 1 {
            let text = format!("체리픽: {} → {target}", short(&commit.hash));
            apply.push(item(text, Op::CherryPick(commit.hash.clone())));
        }
        groups.push(apply);
    }
    // 리모트와 주고받기: 현재 브랜치의 풀·푸시, 그리고 이 행에 있는 다른 로컬 브랜치의 푸시
    let mut sync = Vec::new();
    if let Some(upstream) = &snap.upstream {
        sync.push(item(format!("풀: {upstream} → {target}"), Op::Pull));
    }
    if let Some(branch) = &snap.head_branch {
        sync.extend(pushes(snap, branch));
    }
    for label in labels(snap, &commit.hash) {
        if label.kind == RefKind::Branch && !label.is_head {
            sync.extend(pushes(snap, &label.name));
        }
    }
    groups.push(sync);
    groups.retain(|g| !g.is_empty());
    groups
}

pub fn run(repo: &Path, op: &Op) -> Result<(), String> {
    if let Op::CherryPick(hash) = op {
        // 이미 들어 있는 커밋을 체리픽하면 git이 "빈 커밋" 상태로 멈춰버린다.
        if git::is_ancestor(repo, hash, "HEAD") {
            return Err("이미 현재 브랜치에 들어 있는 커밋이에요".into());
        }
    }
    let timeout = matches!(op, Op::Pull | Op::Push { .. }).then_some(git::NETWORK_TIMEOUT);
    git::run(repo, &op.args(), timeout)
        .map_err(|e| if e.is_empty() { "git이 이유를 알려주지 않았어요".into() } else { e })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Commit, LoadRequest};
    use std::path::PathBuf;
    use std::process::Command;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.repeat(40),
            parents: parents.iter().map(|p| p.repeat(40)).collect(),
            author: String::new(),
            date: String::new(),
            time: 0,
            subject: String::new(),
            kind: RowKind::Commit,
        }
    }

    fn label(name: &str, kind: RefKind, is_head: bool) -> RefLabel {
        RefLabel { name: name.to_string(), kind, remotes: Vec::new(), is_head }
    }

    /// c(feature, origin/topic, origin/dev) → b(main=HEAD, other, v1) → a
    fn snapshot() -> Snapshot {
        let mut snap = Snapshot {
            commits: vec![commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])],
            head: Some("b".repeat(40)),
            head_branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            branches: ["main", "dev", "feature", "other", "remotes/origin/topic", "remotes/origin/dev"]
                .map(String::from)
                .to_vec(),
            ..Default::default()
        };
        snap.refs.insert(
            "c".repeat(40),
            vec![
                label("feature", RefKind::Branch, false),
                label("origin/topic", RefKind::Remote, false),
                label("origin/dev", RefKind::Remote, false),
            ],
        );
        snap.refs.insert(
            "b".repeat(40),
            vec![
                label("main", RefKind::Branch, true),
                label("other", RefKind::Branch, false),
                label("v1", RefKind::Tag, false),
            ],
        );
        snap
    }

    fn ops(items: &[Item]) -> Vec<Op> {
        items.iter().map(|i| i.op.clone()).collect()
    }

    #[test]
    fn checkout_targets() {
        let snap = snapshot();
        // 로컬 브랜치, 로컬이 없는 리모트(새로 만듦), 로컬이 다른 곳에 있는 리모트(그 로컬로)
        assert_eq!(
            ops(&checkouts(&snap, 0)),
            vec![
                Op::Checkout("feature".into()),
                Op::CheckoutRemote("origin/topic".into()),
                Op::Checkout("dev".into())
            ]
        );
        // 지금 있는 브랜치와 태그는 대상이 아니다
        assert_eq!(ops(&checkouts(&snap, 1)), vec![Op::Checkout("other".into())]);
        assert!(checkouts(&snap, 2).is_empty());
        // 지금 있는 브랜치의 리모트 라벨도 대상이 아니다
        assert_eq!(checkout(&snap, &label("origin/main", RefKind::Remote, false)), None);
    }

    #[test]
    fn menu_per_row() {
        let snap = snapshot();
        let texts = |row| -> Vec<Vec<String>> {
            menu(&snap, row).into_iter().map(|g| g.into_iter().map(|i| i.text).collect()).collect()
        };
        assert_eq!(
            texts(0),
            vec![
                vec![
                    "체크아웃: feature".to_string(),
                    "체크아웃: origin/topic (로컬 브랜치 topic 만들기)".into(),
                    "체크아웃: dev".into()
                ],
                vec![
                    "머지: feature → main".into(),
                    "머지: origin/topic → main".into(),
                    "머지: origin/dev → main".into(),
                    "체리픽: ccccccc → main".into()
                ],
                vec!["풀: origin/main → main".into()],
            ]
        );
        // HEAD 커밋에는 머지·체리픽이 없다
        assert_eq!(texts(1), vec![vec!["체크아웃: other".to_string()], vec!["풀: origin/main → main".into()]]);
        // 브랜치가 없는 커밋은 해시로 머지한다
        assert_eq!(
            texts(2),
            vec![
                vec!["머지: aaaaaaa → main".to_string(), "체리픽: aaaaaaa → main".into()],
                vec!["풀: origin/main → main".into()]
            ]
        );
    }

    #[test]
    fn menu_special_states() {
        // 머지 커밋은 체리픽하지 않는다. 따라가는 리모트 브랜치가 없으면 풀이 없다.
        let mut snap = snapshot();
        snap.commits[0].parents.push("a".repeat(40));
        snap.upstream = None;
        let all: Vec<Op> = menu(&snap, 0).into_iter().flatten().map(|i| i.op).collect();
        assert!(all.iter().all(|op| !matches!(op, Op::CherryPick(_) | Op::Pull)));
        assert!(all.contains(&Op::Merge("feature".into())));

        // 멈춘 작업이 있으면 취소만
        snap.in_progress = Some(InProgress::Merge);
        assert_eq!(ops(&menu(&snap, 0).concat()), vec![Op::Abort(InProgress::Merge)]);
        assert_eq!(Op::Abort(InProgress::Merge).command(), "git merge --abort");
        assert_eq!(Op::CherryPick("c".repeat(40)).command(), "git cherry-pick ccccccc");
    }

    #[test]
    fn push_targets() {
        let mut snap = snapshot();
        snap.remotes = vec!["origin".into(), "fork".into()];
        snap.upstreams.insert("main".into(), "origin/main".into());
        snap.upstreams.insert("other".into(), "fork/renamed".into());
        let push = |remote: &str, refspec: &str, set_upstream| Op::Push {
            remote: remote.into(),
            refspec: refspec.into(),
            set_upstream,
        };

        // 따라가는 브랜치가 있으면 그쪽으로, 이름이 다르면 "로컬:리모트"로
        assert_eq!(ops(&pushes(&snap, "main")), vec![push("origin", "main", false)]);
        assert_eq!(ops(&pushes(&snap, "other")), vec![push("fork", "other:renamed", false)]);
        // 따라가는 게 없으면 리모트마다 새 브랜치로
        let texts: Vec<String> = pushes(&snap, "feature").into_iter().map(|i| i.text).collect();
        assert_eq!(texts, vec!["푸시: feature → origin (새 브랜치)", "푸시: feature → fork (새 브랜치)"]);
        assert_eq!(push("origin", "feature", true).command(), "git push --set-upstream origin feature");
        // 리모트에 같은 이름의 브랜치가 이미 있으면 "따라가게 설정"
        snap.remote_refs.push("fork/feature".into());
        let texts: Vec<String> = pushes(&snap, "feature").into_iter().map(|i| i.text).collect();
        assert_eq!(texts, vec!["푸시: feature → origin (새 브랜치)", "푸시: feature → fork/feature (따라가게 설정)"]);
        // 로컬 브랜치를 따라가면 올릴 리모트가 없다
        snap.upstreams.insert("dev".into(), "main".into());
        assert!(pushes(&snap, "dev").is_empty());
        // 리모트 이름에 /가 있어도 아는 리모트 이름으로 자른다
        snap.remotes.push("team/a".into());
        snap.upstreams.insert("x".into(), "team/a/x".into());
        assert_eq!(ops(&pushes(&snap, "x")), vec![push("team/a", "x", false)]);

        // 메뉴: 풀 → 현재 브랜치 푸시 → 이 행의 다른 로컬 브랜치 푸시
        let sync: Vec<String> = menu(&snap, 1).pop().unwrap().into_iter().map(|i| i.text).collect();
        assert_eq!(sync, vec!["풀: origin/main → main", "푸시: main → origin/main", "푸시: other → fork/renamed"]);
        // 리모트가 없으면 푸시 줄이 없다
        snap.remotes.clear();
        snap.upstreams.clear();
        assert!(menu(&snap, 1).concat().iter().all(|i| !matches!(i.op, Op::Push { .. })));
    }

    /// 실제 git 저장소를 임시 폴더에 만들어 작업을 돌려본다.
    struct Repo(PathBuf);

    impl Repo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ggl-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let repo = Self(std::fs::canonicalize(&dir).unwrap());
            repo.git(&["init", "-q", "-b", "main"]);
            for (key, value) in [
                ("user.name", "t"),
                ("user.email", "t@t"),
                ("commit.gpgsign", "false"),
                ("pull.rebase", "false"),
                ("core.hooksPath", "/dev/null"),
            ] {
                repo.git(&["config", key, value]);
            }
            repo
        }

        fn git(&self, args: &[&str]) -> String {
            let out = Command::new("git").arg("-C").arg(&self.0).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        /// 파일을 쓰고 커밋한 뒤 해시를 돌려준다.
        fn commit(&self, file: &str, content: &str) -> String {
            std::fs::write(self.0.join(file), content).unwrap();
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", &format!("{file}: {content}")]);
            self.git(&["rev-parse", "HEAD"])
        }

        fn run(&self, op: Op) -> Result<(), String> {
            run(&self.0, &op)
        }

        fn branch(&self) -> String {
            self.git(&["symbolic-ref", "--short", "HEAD"])
        }

        fn load(&self) -> Snapshot {
            let req = LoadRequest { repo: self.0.clone(), max: 50, branch: None, show_remotes: true };
            git::load(&req).unwrap()
        }

        fn state(&self) -> Option<InProgress> {
            self.load().in_progress
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn checkout_merge_and_cherry_pick() {
        let repo = Repo::new("basic");
        let base = repo.commit("a.txt", "1");
        repo.git(&["switch", "-q", "-c", "feature"]);
        repo.commit("b.txt", "1");
        let tip = repo.commit("c.txt", "1");

        repo.run(Op::Checkout("main".into())).unwrap();
        assert_eq!(repo.branch(), "main");
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), base);

        // 체리픽: 커밋 하나만 복사된다
        repo.run(Op::CherryPick(tip.clone())).unwrap();
        assert!(repo.0.join("c.txt").exists() && !repo.0.join("b.txt").exists());
        // 이미 들어 있는 커밋은 멈추지 않고 바로 거절한다
        let err = repo.run(Op::CherryPick(base)).unwrap_err();
        assert!(err.contains("이미"), "{err}");
        assert_eq!(repo.state(), None);

        repo.run(Op::Merge("feature".into())).unwrap();
        assert!(repo.0.join("b.txt").exists());
        assert!(git::is_ancestor(&repo.0, &tip, "HEAD"));
        assert_eq!(repo.state(), None);

        // 없는 브랜치는 git의 말을 그대로 돌려준다
        assert!(!repo.run(Op::Checkout("nope".into())).unwrap_err().is_empty());
    }

    #[test]
    fn conflict_stalls_and_abort_restores() {
        let repo = Repo::new("conflict");
        repo.commit("a.txt", "base");
        repo.git(&["switch", "-q", "-c", "feature"]);
        let theirs = repo.commit("a.txt", "theirs");
        repo.git(&["switch", "-q", "main"]);
        let ours = repo.commit("a.txt", "ours");

        let err = repo.run(Op::Merge("feature".into())).unwrap_err();
        assert!(err.contains("a.txt"), "{err}");
        assert_eq!(repo.state(), Some(InProgress::Merge));
        repo.run(Op::Abort(InProgress::Merge)).unwrap();
        assert_eq!(repo.state(), None);

        assert!(repo.run(Op::CherryPick(theirs)).is_err());
        assert_eq!(repo.state(), Some(InProgress::CherryPick));
        repo.run(Op::Abort(InProgress::CherryPick)).unwrap();
        assert_eq!(repo.state(), None);

        // 취소하면 원래대로 돌아온다
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), ours);
        assert_eq!(repo.git(&["status", "--porcelain"]), "");
        assert_eq!(std::fs::read_to_string(repo.0.join("a.txt")).unwrap(), "ours");
    }

    #[test]
    fn checkout_keeps_local_changes_safe() {
        let repo = Repo::new("dirty");
        repo.commit("a.txt", "base");
        repo.git(&["switch", "-q", "-c", "feature"]);
        repo.commit("a.txt", "feature");
        repo.git(&["switch", "-q", "main"]);
        std::fs::write(repo.0.join("a.txt"), "not committed").unwrap();

        // 커밋 안 한 변경을 덮어쓰게 되면 git이 거절하고, 파일은 그대로다
        assert!(repo.run(Op::Checkout("feature".into())).is_err());
        assert_eq!(repo.branch(), "main");
        assert_eq!(std::fs::read_to_string(repo.0.join("a.txt")).unwrap(), "not committed");
    }

    #[test]
    fn push_follows_upstream_and_never_forces() {
        let origin = Repo::new("push-origin");
        origin.commit("a.txt", "1");
        // 체크아웃된 브랜치로도 받을 수 있게 (테스트용 설정)
        origin.git(&["config", "receive.denyCurrentBranch", "updateInstead"]);

        let repo = Repo::new("push-clone");
        repo.git(&["remote", "add", "origin", origin.0.to_str().unwrap()]);
        repo.git(&["fetch", "-q", "origin"]);
        repo.git(&["reset", "-q", "--hard", "origin/main"]);
        repo.git(&["branch", "-q", "--set-upstream-to", "origin/main"]);

        // 따라가는 브랜치로 올린다
        let mine = repo.commit("b.txt", "1");
        let snap = repo.load();
        assert_eq!(snap.ahead, Some(1));
        repo.run(pushes(&snap, "main").remove(0).op).unwrap();
        assert_eq!(origin.git(&["rev-parse", "main"]), mine);
        assert_eq!(repo.load().ahead, Some(0));

        // 새 브랜치는 리모트에 만들고, 앞으로 그쪽을 따라간다
        repo.git(&["switch", "-q", "-c", "feature"]);
        let feature = repo.commit("c.txt", "1");
        let snap = repo.load();
        assert_eq!(snap.upstream, None);
        let item = pushes(&snap, "feature").remove(0);
        assert!(matches!(item.op, Op::Push { set_upstream: true, .. }));
        repo.run(item.op).unwrap();
        assert_eq!(origin.git(&["rev-parse", "feature"]), feature);
        assert_eq!(repo.load().upstream.as_deref(), Some("origin/feature"));

        // 리모트가 앞서 있으면 거절되고, 리모트 커밋은 덮어쓰지 않는다
        let theirs = origin.commit("a.txt", "2");
        repo.git(&["switch", "-q", "main"]);
        repo.commit("d.txt", "1");
        let err = repo.run(pushes(&repo.load(), "main").remove(0).op).unwrap_err();
        assert!(err.contains("rejected"), "{err}");
        assert_eq!(origin.git(&["rev-parse", "main"]), theirs);
    }

    #[test]
    fn remote_checkout_and_pull() {
        let origin = Repo::new("origin");
        origin.commit("a.txt", "1");
        origin.git(&["switch", "-q", "-c", "topic"]);
        origin.commit("t.txt", "1");
        origin.git(&["switch", "-q", "main"]);

        let repo = Repo::new("clone");
        repo.git(&["remote", "add", "origin", origin.0.to_str().unwrap()]);
        repo.git(&["fetch", "-q", "origin"]);
        repo.git(&["reset", "-q", "--hard", "origin/main"]);
        repo.git(&["branch", "-q", "--set-upstream-to", "origin/main"]);

        // 리모트 브랜치 → 따라가는 로컬 브랜치가 생긴다
        repo.run(Op::CheckoutRemote("origin/topic".into())).unwrap();
        assert_eq!(repo.branch(), "topic");
        assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "topic@{upstream}"]), "origin/topic");

        repo.run(Op::Checkout("main".into())).unwrap();
        assert_eq!(repo.load().upstream.as_deref(), Some("origin/main"));
        let newer = origin.commit("a.txt", "2");
        repo.run(Op::Pull).unwrap();
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), newer);
    }
}
