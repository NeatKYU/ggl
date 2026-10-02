//! 같은 저장소를 연 ggl 창이 이미 있으면 새 창을 띄우지 않고 그 창에 넘긴다.
//! 창마다 지금 연 저장소의 유닉스 소켓을 연다: `~/Library/Caches/ggl/<경로 해시>.sock`
//! (herdr 단축키처럼 같은 저장소를 여러 번 열어도 창이 쌓이지 않게)

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 다른 ggl이 보내는 요청
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// 창을 앞으로
    Show,
    /// 파일 트리를 펼치고 창을 앞으로
    Files,
}

impl Request {
    fn as_str(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Files => "files",
        }
    }

    fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "show" => Some(Self::Show),
            "files" => Some(Self::Files),
            _ => None,
        }
    }
}

fn socket_path(repo: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    // 버전이 다른 ggl끼리도 같은 이름이 나와야 해서 표준 해시 대신 FNV-1a를 쓴다.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in repo.as_os_str().as_encoded_bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    Some(PathBuf::from(home).join(format!("Library/Caches/ggl/{h:016x}.sock")))
}

/// `repo`를 연 ggl이 떠 있으면 요청을 넘기고 true
pub fn hand_off(repo: &Path, req: Request) -> bool {
    let Some(path) = socket_path(repo) else { return false };
    let Ok(mut stream) = UnixStream::connect(&path) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, "{}", req.as_str()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// 요청을 받는 쪽. 버리면 소켓도 닫힌다.
pub struct Listener {
    path: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Listener {
    /// 같은 저장소를 이미 다른 창이 받고 있으면 None (그 창에 맡긴다).
    pub fn start(repo: &Path, on_request: impl Fn(Request) + Send + 'static) -> Option<Self> {
        let path = socket_path(repo)?;
        if UnixStream::connect(&path).is_ok() {
            return None;
        }
        std::fs::create_dir_all(path.parent()?).ok()?;
        // 꺼진 ggl이 남긴 소켓 파일
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).ok()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                if let Some(req) = Request::parse(&line) {
                    on_request(req);
                    let _ = writeln!(stream, "ok");
                }
            }
        });
        Some(Self { path, stop })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // accept에서 기다리는 스레드를 깨워서 끝낸다.
        let _ = UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn hands_off_to_the_window_with_the_same_repo() {
        let repo = std::env::temp_dir().join(format!("ggl-remote-test-{}", std::process::id()));
        assert!(!hand_off(&repo, Request::Show), "아직 받는 창이 없다");

        let (tx, rx) = mpsc::channel();
        let listener = Listener::start(&repo, move |req| tx.send(req).unwrap()).unwrap();
        assert!(Listener::start(&repo, |_| {}).is_none(), "같은 저장소는 한 창만 받는다");
        assert!(hand_off(&repo, Request::Files));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Request::Files));

        drop(listener);
        assert!(!hand_off(&repo, Request::Show), "창이 닫히면 넘기지 않는다");
        let again = Listener::start(&repo, |_| {});
        assert!(again.is_some(), "닫힌 뒤에는 다시 받을 수 있다");
    }
}
