//! 파일 트리: git이 알려준 파일 목록을 폴더 구조로 묶는다.
//! 폴더가 먼저, 그다음 파일이 이름순으로 온다.
//! 하위 폴더 하나만 든 폴더는 한 줄로 합쳐 보여준다 (`src/main/java`처럼, VS Code와 같음).

use std::collections::{HashMap, HashSet};

use crate::git::TreeFile;

pub struct Node {
    /// 저장소 기준 경로 (루트는 빈 문자열)
    pub path: String,
    /// 경로에서 이름이 시작하는 위치
    name_at: usize,
    pub dir: bool,
    /// 파일의 커밋 안 된 변경
    pub status: Option<char>,
    /// 폴더 안에 바뀐 파일 수
    pub changed: usize,
    children: Vec<usize>,
}

impl Node {
    pub fn name(&self) -> &str {
        &self.path[self.name_at..]
    }

    /// 파일이 든 폴더 (맨 위면 빈 문자열)
    pub fn folder(&self) -> &str {
        self.path[..self.name_at].trim_end_matches('/')
    }
}

/// 화면에 그릴 한 줄
pub struct Row {
    pub node: usize,
    pub depth: usize,
    /// 보여줄 이름이 경로에서 시작하는 위치 (합친 폴더는 맨 위 폴더 이름부터)
    label_at: usize,
}

pub struct FileTree {
    /// 0번이 루트
    pub nodes: Vec<Node>,
    pub files: usize,
    pub changed: usize,
}

impl FileTree {
    pub fn build(files: Vec<TreeFile>) -> Self {
        let root = Node { path: String::new(), name_at: 0, dir: true, status: None, changed: 0, children: Vec::new() };
        let mut tree = FileTree { nodes: vec![root], files: files.len(), changed: 0 };
        let mut dirs: HashMap<String, usize> = HashMap::new();
        for f in files {
            let mut parent = 0;
            let mut ancestors = vec![0];
            for (i, _) in f.path.match_indices('/') {
                let dir = &f.path[..i];
                parent = match dirs.get(dir) {
                    Some(&id) => id,
                    None => {
                        let id = tree.push(parent, dir.to_string(), true, None);
                        dirs.insert(dir.to_string(), id);
                        id
                    }
                };
                ancestors.push(parent);
            }
            if f.status.is_some() {
                tree.changed += 1;
                for a in ancestors {
                    tree.nodes[a].changed += 1;
                }
            }
            tree.push(parent, f.path, false, f.status);
        }
        let keys: Vec<(bool, String)> = tree.nodes.iter().map(|n| (!n.dir, n.name().to_lowercase())).collect();
        for n in &mut tree.nodes {
            n.children.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
        }
        tree
    }

    fn push(&mut self, parent: usize, path: String, dir: bool, status: Option<char>) -> usize {
        let id = self.nodes.len();
        let name_at = path.rfind('/').map_or(0, |i| i + 1);
        self.nodes.push(Node { path, name_at, dir, status, changed: 0, children: Vec::new() });
        self.nodes[parent].children.push(id);
        id
    }

    /// 보여줄 이름 (합친 폴더는 `src/main/java`)
    pub fn label(&self, row: &Row) -> &str {
        &self.nodes[row.node].path[row.label_at..]
    }

    /// 펼친 폴더(`open`) 안쪽까지 화면에 보일 줄들
    pub fn rows(&self, open: &HashSet<String>) -> Vec<Row> {
        let mut rows = Vec::new();
        self.push_rows(0, 0, open, &mut rows);
        rows
    }

    fn push_rows(&self, dir: usize, depth: usize, open: &HashSet<String>, rows: &mut Vec<Row>) {
        for &child in &self.nodes[dir].children {
            let label_at = self.nodes[child].name_at;
            let mut node = child;
            while let [only] = self.nodes[node].children[..] {
                if !self.nodes[only].dir {
                    break;
                }
                node = only;
            }
            rows.push(Row { node, depth, label_at });
            if self.nodes[node].dir && open.contains(&self.nodes[node].path) {
                self.push_rows(node, depth + 1, open, rows);
            }
        }
    }

    /// 경로에 `query`(소문자)가 들어간 파일. 파일 이름에 들어간 것이 먼저 온다.
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        let mut by_name = Vec::new();
        let mut by_path = Vec::new();
        for (i, n) in self.nodes.iter().enumerate().filter(|(_, n)| !n.dir) {
            if n.name().to_lowercase().contains(query) {
                by_name.push(i);
            } else if n.path.to_lowercase().contains(query) {
                by_path.push(i);
            }
        }
        by_name.extend(by_path);
        by_name.truncate(limit);
        by_name
    }

    /// 빠른 열기(⌃P): `query`의 글자가 경로에 차례대로 들어 있는 파일을 잘 맞는 순서로 (VS Code의 ⌘P처럼).
    /// 파일 이름에서 맞은 것, 글자가 이어서 맞은 것, 단어 첫 글자에서 맞은 것이 앞에 온다.
    /// 검색어가 비어 있으면 커밋 안 한 변경이 있는 파일이 먼저 온다.
    pub fn fuzzy(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
        let files = self.nodes.iter().enumerate().filter(|(_, n)| !n.dir);
        let mut hits: Vec<(i32, Hit)> = if q.is_empty() {
            files.map(|(i, n)| (if n.status.is_some() { 1 } else { 0 }, Hit { node: i, marks: Vec::new() })).collect()
        } else {
            files
                .filter_map(|(i, n)| {
                    // 이름에서 다 맞으면 경로에서 맞은 것보다 앞에 온다. `/`를 넣으면 경로로 찾는다.
                    let in_name = if q.contains(&'/') { None } else { subsequence(&n.path, n.name_at, &q) };
                    let (score, marks) = match in_name {
                        Some((s, m)) => (s + 1000, m),
                        None => subsequence(&n.path, 0, &q)?,
                    };
                    Some((score, Hit { node: i, marks }))
                })
                .collect()
        };
        // 점수가 같으면 짧은 경로, 그다음 경로 이름순
        let paths = |h: &Hit| &self.nodes[h.node].path;
        hits.sort_by(|(sa, a), (sb, b)| {
            sb.cmp(sa).then(paths(a).len().cmp(&paths(b).len())).then_with(|| paths(a).cmp(paths(b)))
        });
        hits.truncate(limit);
        hits.into_iter().map(|(_, h)| h).collect()
    }
}

/// 빠른 열기에서 찾은 파일
pub struct Hit {
    pub node: usize,
    /// 경로에서 맞은 글자의 위치 (바이트)
    pub marks: Vec<usize>,
}

/// `text[from..]`에 `query`(소문자) 글자가 차례대로 들어 있으면 점수와 맞은 위치.
/// 글자마다 앞에서부터 맞추되, 단어 첫 글자(`/`·`_`·`-`·`.` 뒤, 대문자)에서 맞는 곳이 가까이 있으면 그쪽을 고른다.
fn subsequence(text: &str, from: usize, query: &[char]) -> Option<(i32, Vec<usize>)> {
    let chars: Vec<(usize, char)> = text[from..].char_indices().map(|(i, c)| (i + from, c)).collect();
    let starts_word = |k: usize| {
        k == 0 || {
            let (prev, cur) = (chars[k - 1].1, chars[k].1);
            matches!(prev, '/' | '_' | '-' | '.' | ' ') || (prev.is_lowercase() && cur.is_uppercase())
        }
    };
    let same = |k: usize, q: char| chars[k].1.to_lowercase().eq(std::iter::once(q));
    let (mut score, mut marks, mut k) = (0, Vec::with_capacity(query.len()), 0);
    let mut last: Option<usize> = None;
    for &q in query {
        let first = (k..chars.len()).find(|&j| same(j, q))?;
        // 바로 이어지지 않으면, 남은 곳에서 단어 첫 글자로 맞는 곳을 찾아본다 (`ap` → `app_state`의 a·p보다 `AppState`)
        let j = if last.is_some_and(|l| l + 1 == first) || starts_word(first) {
            first
        } else {
            (first..chars.len()).find(|&j| same(j, q) && starts_word(j)).unwrap_or(first)
        };
        score += 1;
        if last.is_some_and(|l| l + 1 == j) {
            score += 5;
        }
        if starts_word(j) {
            score += 3;
        }
        marks.push(chars[j].0);
        last = Some(j);
        k = j + 1;
    }
    Some((score, marks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[(&str, Option<char>)]) -> FileTree {
        FileTree::build(paths.iter().map(|&(p, s)| TreeFile { path: p.into(), status: s }).collect())
    }

    fn labels(t: &FileTree, open: &[&str]) -> Vec<String> {
        let open: HashSet<String> = open.iter().map(|s| s.to_string()).collect();
        t.rows(&open).iter().map(|r| format!("{}{}", "  ".repeat(r.depth), t.label(r))).collect()
    }

    #[test]
    fn folders_first_and_compact_chains() {
        let t = tree(&[
            ("README.md", None),
            ("src/main/java/App.java", Some('M')),
            ("src/main/java/Util.java", None),
            ("a.txt", None),
            ("docs/x.md", None),
            ("docs/img/y.png", None),
        ]);
        assert_eq!(labels(&t, &[]), ["docs", "src/main/java", "a.txt", "README.md"]);
        assert_eq!(
            labels(&t, &["docs", "src/main/java"]),
            ["docs", "  img", "  x.md", "src/main/java", "  App.java", "  Util.java", "a.txt", "README.md"]
        );
        // 합친 줄을 펼치는 열쇠는 맨 아래 폴더다.
        assert_eq!(labels(&t, &["src"]), ["docs", "src/main/java", "a.txt", "README.md"]);
    }

    #[test]
    fn counts_changes_up_the_folders() {
        let t = tree(&[("src/a.rs", Some('M')), ("src/b/c.rs", Some('U')), ("x.rs", None)]);
        let changed = |path: &str| t.nodes.iter().find(|n| n.path == path).map(|n| n.changed);
        assert_eq!((t.files, t.changed), (3, 2));
        assert_eq!(changed("src"), Some(2));
        assert_eq!(changed("src/b"), Some(1));
    }

    #[test]
    fn search_puts_name_hits_first() {
        let t = tree(&[("app/main.rs", None), ("main/lib.rs", None), ("x/Main.kt", None)]);
        let found: Vec<&str> = t.search("main", 10).into_iter().map(|i| t.nodes[i].path.as_str()).collect();
        assert_eq!(found, ["app/main.rs", "x/Main.kt", "main/lib.rs"]);
        assert_eq!(t.nodes[t.search("lib", 1)[0]].folder(), "main");
    }

    #[test]
    fn fuzzy_ranks_name_and_word_starts_first() {
        let t = tree(&[
            ("src/view/toolbar.rs", None),
            ("src/app.rs", None),
            ("docs/application.md", None),
            ("src/view/app_state.rs", None),
            ("tests/AppState.kt", None),
            ("README.md", Some('M')),
        ]);
        let found = |q: &str| -> Vec<String> { t.fuzzy(q, 10).iter().map(|h| t.nodes[h.node].path.clone()).collect() };
        // 이어서 맞은 이름이 먼저, 흩어져 맞은 이름은 뒤에
        assert_eq!(found("app")[0], "src/app.rs");
        // 단어 첫 글자: as → app_state, AppState
        assert_eq!(&found("as")[..2], ["tests/AppState.kt", "src/view/app_state.rs"]);
        // 이름에 없으면 경로에서 찾는다
        assert_eq!(found("viewtool"), ["src/view/toolbar.rs"]);
        assert_eq!(found("src/app"), ["src/app.rs", "src/view/app_state.rs"]);
        assert!(found("xyz").is_empty());
        // 빈 검색어면 바뀐 파일이 먼저
        assert_eq!(found("")[0], "README.md");
        // 맞은 글자 위치(바이트)
        let hit = &t.fuzzy("tb", 1)[0];
        assert_eq!((t.nodes[hit.node].path.as_str(), hit.marks.as_slice()), ("src/view/toolbar.rs", &[9, 13][..]));
    }
}
