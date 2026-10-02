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
}
