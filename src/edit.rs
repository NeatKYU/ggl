//! 파일 편집: 디스크에서 읽고, 고치고, 저장한다.
//! 편집을 시작한 뒤 다른 곳(편집기, git)에서 파일이 바뀌었으면 덮어쓰기 전에 묻는다.

use std::path::Path;

/// 이보다 큰 파일은 편집하지 않는다 (입력할 때마다 전체를 다시 배치해서 느려진다).
pub const MAX_BYTES: u64 = 512 * 1024;

pub struct Editor {
    pub path: String,
    pub text: String,
    /// 마지막으로 저장한(또는 연) 내용. 이것과 다르면 저장 안 한 변경이 있다.
    saved: String,
    /// 그때 디스크에 있던 내용 (그사이 다른 곳에서 바뀌었는지 비교한다)
    disk: Vec<u8>,
    /// 줄바꿈이 모두 CRLF인 파일. 편집은 LF로 하고 저장할 때 되돌린다.
    crlf: bool,
    /// 저장 실패 사유
    pub error: Option<String>,
    /// 다른 곳에서 바뀐 파일을 덮어쓸지 묻는 중
    pub conflict: bool,
}

impl Editor {
    pub fn open(repo: &Path, path: &str) -> Result<Self, String> {
        let full = repo.join(path);
        let meta = std::fs::metadata(&full).map_err(|e| format!("파일을 읽을 수 없어요: {e}"))?;
        if !meta.is_file() {
            return Err("파일이 아니에요".into());
        }
        if meta.len() > MAX_BYTES {
            return Err(format!("{}KB가 넘는 파일은 편집할 수 없어요", MAX_BYTES / 1024));
        }
        let disk = std::fs::read(&full).map_err(|e| format!("파일을 읽을 수 없어요: {e}"))?;
        if disk.contains(&0) {
            return Err("바이너리 파일은 편집할 수 없어요".into());
        }
        let text = String::from_utf8(disk.clone()).map_err(|_| "UTF-8이 아닌 파일은 편집할 수 없어요".to_string())?;
        // 줄바꿈이 섞여 있으면 바꾸지 않고 그대로 편집한다.
        let crlf = text.contains("\r\n") && !text.replace("\r\n", "").contains('\n');
        let text = if crlf { text.replace("\r\n", "\n") } else { text };
        Ok(Self { path: path.to_string(), saved: text.clone(), text, disk, crlf, error: None, conflict: false })
    }

    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }

    /// 저장한다. 편집하는 동안 디스크의 파일이 바뀌었으면 `force`가 아닌 한 저장하지 않고 false.
    pub fn save(&mut self, repo: &Path, force: bool) -> Result<bool, String> {
        let full = repo.join(&self.path);
        if !force && std::fs::read(&full).ok().as_deref() != Some(self.disk.as_slice()) {
            return Ok(false);
        }
        let out = if self.crlf { self.text.replace('\n', "\r\n") } else { self.text.clone() };
        std::fs::write(&full, &out).map_err(|e| format!("저장하지 못했어요: {e}"))?;
        self.disk = out.into_bytes();
        self.saved = self.text.clone();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ggl-edit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_and_keeps_crlf() {
        let repo = temp_repo("crlf");
        std::fs::write(repo.join("a.txt"), "하나\r\n둘\r\n").unwrap();
        let mut ed = Editor::open(&repo, "a.txt").unwrap();
        assert_eq!(ed.text, "하나\n둘\n");
        assert!(!ed.dirty());
        ed.text.push_str("셋\n");
        assert!(ed.dirty());
        assert_eq!(ed.save(&repo, false), Ok(true));
        assert!(!ed.dirty());
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "하나\r\n둘\r\n셋\r\n");
    }

    #[test]
    fn asks_before_overwriting_outside_changes() {
        let repo = temp_repo("conflict");
        std::fs::write(repo.join("a.txt"), "old\n").unwrap();
        let mut ed = Editor::open(&repo, "a.txt").unwrap();
        ed.text = "mine\n".into();
        std::fs::write(repo.join("a.txt"), "theirs\n").unwrap();
        assert_eq!(ed.save(&repo, false), Ok(false));
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "theirs\n");
        assert_eq!(ed.save(&repo, true), Ok(true));
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "mine\n");
        // 저장한 뒤에는 그 내용이 기준이 된다.
        ed.text = "again\n".into();
        assert_eq!(ed.save(&repo, false), Ok(true));
    }

    #[test]
    fn refuses_files_it_cannot_edit() {
        let repo = temp_repo("refuse");
        std::fs::write(repo.join("bin"), [0u8, 1, 2]).unwrap();
        std::fs::write(repo.join("latin1"), [0xe9u8, b'\n']).unwrap();
        std::fs::write(repo.join("mixed"), "a\r\nb\n").unwrap();
        assert!(Editor::open(&repo, "bin").is_err());
        assert!(Editor::open(&repo, "latin1").is_err());
        assert!(Editor::open(&repo, "missing").is_err());
        assert_eq!(Editor::open(&repo, "mixed").unwrap().text, "a\r\nb\n");
    }
}
