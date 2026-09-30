//! 저장소 파일이 바뀌면 잠시(750ms) 기다렸다가 한 번만 새로고침을 알린다.
//! 어떤 파일에 반응할지는 원본 Git Graph의 규칙을 따른다.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE: Duration = Duration::from_millis(750);

/// 자주 바뀌지만 git 상태와는 거의 상관없는 폴더 (빌드 결과물 등)
const IGNORED_DIRS: &[&str] =
    &["node_modules", ".dart_tool", "build", "target", ".gradle", "DerivedData", "Pods", ".next"];

pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
}

impl RepoWatcher {
    /// 감시를 시작한다. 반환값을 버리면 감시도 멈춘다.
    pub fn start(repo: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<()>();
        // macOS는 /private/var ↔ /var 처럼 경로가 다르게 올 수 있어서 실제 경로도 같이 비교한다.
        let roots = [repo.to_path_buf(), std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf())];
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            if event.paths.iter().any(|p| is_relevant(&roots, p)) {
                let _ = tx.send(());
            }
        })?;
        watcher.watch(repo, RecursiveMode::Recursive)?;

        thread::spawn(move || {
            while rx.recv().is_ok() {
                // 변경이 750ms 동안 멈출 때까지 기다린다.
                loop {
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(()) => continue,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_change();
            }
        });
        Ok(Self { _watcher: watcher })
    }
}

fn is_relevant(roots: &[PathBuf], path: &Path) -> bool {
    let Some(rel) = roots.iter().find_map(|r| path.strip_prefix(r).ok()) else {
        return false;
    };
    let rel = rel.to_string_lossy().replace('\\', "/");

    if let Some(inner) = rel.strip_prefix(".git/") {
        if inner.ends_with(".lock") {
            return false;
        }
        return matches!(inner, "config" | "index" | "HEAD" | "packed-refs" | "refs/stash")
            || inner.starts_with("refs/heads/")
            || inner.starts_with("refs/remotes/")
            || inner.starts_with("refs/tags/");
    }
    if rel == ".git" {
        return false;
    }
    // 하위 모듈의 .git 폴더나 빌드 폴더 안의 변경은 무시
    !Path::new(&rel).components().any(|c| match c {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            name == ".git" || IGNORED_DIRS.contains(&name.as_ref())
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_like_git_graph() {
        let root = [PathBuf::from("/r")];
        let yes = ["/r/src/main.rs", "/r/.gitignore", "/r/.git/HEAD", "/r/.git/index", "/r/.git/refs/heads/main"];
        let no = [
            "/r/.git/objects/ab/cdef",
            "/r/.git/index.lock",
            "/r/.git/logs/HEAD",
            "/r/node_modules/x/index.js",
            "/r/app/build/out.bin",
            "/r/sub/.git/HEAD",
        ];
        for p in yes {
            assert!(is_relevant(&root, Path::new(p)), "{p}");
        }
        for p in no {
            assert!(!is_relevant(&root, Path::new(p)), "{p}");
        }
    }
}
