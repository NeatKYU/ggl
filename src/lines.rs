//! 저장소 줄 수: 추적 중인 파일과 아직 추가 안 한 새 파일의 줄을 센다 (`.gitignore`에 걸린 파일은 부르는 쪽에서 뺀다).
//! 바이너리 파일과 자동으로 만들어지는 잠금 파일(Cargo.lock 등)은 뺀다.
//! 파일 크기·수정 시각이 그대로면 지난번에 센 값을 다시 쓴다 (파일이 바뀔 때마다 다 읽지 않게).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 세지 않는 잠금 파일. 사람이 쓴 코드가 아니고 길어서 전체 줄 수를 부풀린다.
const LOCK_FILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "Gemfile.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "composer.lock",
    "go.sum",
    "Podfile.lock",
    "pubspec.lock",
    "flake.lock",
];

/// 이보다 큰 파일은 데이터로 보고 세지 않는다.
const MAX_SIZE: u64 = 10 * 1024 * 1024;

pub struct LineCount {
    pub total: u64,
    /// 센 파일 수
    pub files: usize,
    /// 확장자별 줄 수, 많은 순
    pub by_ext: Vec<(String, u64)>,
}

#[derive(Default)]
pub struct Counter {
    repo: PathBuf,
    /// 경로 → (크기, 수정 시각, 줄 수). 세지 않는 파일은 None
    cache: HashMap<String, (u64, SystemTime, Option<u64>)>,
}

impl Counter {
    pub fn count(&mut self, repo: &Path, paths: &[String]) -> LineCount {
        if self.repo != repo {
            self.repo = repo.to_path_buf();
            self.cache.clear();
        }
        let mut cache = HashMap::with_capacity(paths.len());
        let mut by_ext: HashMap<String, u64> = HashMap::new();
        let (mut total, mut files) = (0, 0);
        for path in paths {
            let name = path.rsplit('/').next().unwrap_or(path);
            if LOCK_FILES.contains(&name) {
                continue;
            }
            let full = repo.join(path);
            let Ok(meta) = std::fs::metadata(&full) else { continue };
            if !meta.is_file() {
                continue;
            }
            let (size, mtime) = (meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            let lines = match self.cache.get(path.as_str()) {
                Some(&(s, m, lines)) if (s, m) == (size, mtime) => lines,
                _ if size > MAX_SIZE => None,
                _ => std::fs::read(&full).ok().and_then(|bytes| lines_in(&bytes)),
            };
            cache.insert(path.clone(), (size, mtime, lines));
            let Some(n) = lines else { continue };
            total += n;
            files += 1;
            *by_ext.entry(ext(name)).or_default() += n;
        }
        // 이번 목록에 없는 파일(지워진 파일)은 캐시에서도 빠진다.
        self.cache = cache;
        let mut by_ext: Vec<(String, u64)> = by_ext.into_iter().collect();
        by_ext.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        LineCount { total, files, by_ext }
    }
}

/// 줄 수. 앞부분에 NUL 바이트가 있으면 바이너리로 보고 None. 마지막 줄에 줄바꿈이 없어도 한 줄로 센다.
fn lines_in(bytes: &[u8]) -> Option<u64> {
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return None;
    }
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    Some(newlines + u64::from(bytes.last().is_some_and(|&b| b != b'\n')))
}

/// 확장자 (`.rs`). 없으면 파일 이름 그대로 (`Makefile`)
fn ext(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() && !e.is_empty() => format!(".{}", e.to_lowercase()),
        _ => name.to_string(),
    }
}

/// 세 자리마다 쉼표 (6813 → "6,813")
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_lines_and_skips_binary() {
        assert_eq!(lines_in(b""), Some(0));
        assert_eq!(lines_in(b"a\nb\n"), Some(2));
        assert_eq!(lines_in(b"a\nb"), Some(2));
        assert_eq!(lines_in(b"\x89PNG\0\0"), None);
    }

    #[test]
    fn groups_by_extension() {
        assert_eq!(ext("main.RS"), ".rs");
        assert_eq!(ext("Makefile"), "Makefile");
        assert_eq!(ext(".gitignore"), ".gitignore");
    }

    #[test]
    fn formats_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(6813), "6,813");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn counts_a_repo_and_reuses_the_cache() {
        let dir = std::env::temp_dir().join(format!("ggl-lines-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        std::fs::write(dir.join("README.md"), "# x").unwrap();
        std::fs::write(dir.join("Cargo.lock"), "x\n".repeat(100)).unwrap();
        std::fs::write(dir.join("logo.png"), b"\x89PNG\0\0\0").unwrap();
        let paths: Vec<String> =
            ["src/a.rs", "README.md", "Cargo.lock", "logo.png", "gone.txt"].iter().map(|s| s.to_string()).collect();

        let mut counter = Counter::default();
        let c = counter.count(&dir, &paths);
        assert_eq!((c.total, c.files), (3, 2));
        assert_eq!(c.by_ext, [(".rs".to_string(), 2), (".md".to_string(), 1)]);

        // 바뀐 파일만 다시 읽는다 (크기가 달라지면 다시 센다)
        std::fs::write(dir.join("README.md"), "# x\n\nmore\n").unwrap();
        assert_eq!(counter.count(&dir, &paths).total, 5);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
