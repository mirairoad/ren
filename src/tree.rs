use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const IGNORED: &[&str] = &[".git", ".bin", "target", "node_modules", "__pycache__", ".DS_Store"];
/// Left out of the explorer search when the project isn't a git repository
/// (in one, `.gitignore` decides): besides `IGNORED`, folders of installed
/// or generated files.
const NOT_SEARCHED: &[&str] = &[
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".next",
];
/// The most files the search lists, and the most rows it shows.
const SEARCH_FILES: usize = 100_000;
const SEARCH_ROWS: usize = 5_000;

pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

pub struct Tree {
    pub root: PathBuf,
    pub nodes: Vec<Node>,
    pub sel: usize,
    pub scroll: usize,
    pub visible: bool,
    pub width: u16,
    /// The explorer's search box: what's typed. While it isn't empty, `nodes`
    /// are the matches with the folders they're in.
    pub query: String,
    /// The search box has the keys (typing goes into it).
    pub typing: bool,
    /// The tree as it was before the search (nodes, selection, scroll).
    stash: Option<(Vec<Node>, usize, usize)>,
    /// The project's files (relative paths), listed once per search.
    files: Option<Vec<PathBuf>>,
}

fn read_dir(p: &Path, depth: usize) -> Vec<Node> {
    let Ok(rd) = fs::read_dir(p) else { return Vec::new() };
    let mut v: Vec<Node> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if IGNORED.contains(&name.as_str()) {
                return None;
            }
            let path = e.path();
            let is_dir = path.is_dir();
            Some(Node {
                path,
                name,
                depth,
                is_dir,
                expanded: false,
            })
        })
        .collect();
    v.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

fn build(p: &Path, depth: usize, expanded: &HashSet<PathBuf>, out: &mut Vec<Node>) {
    for mut n in read_dir(p, depth) {
        if n.is_dir && expanded.contains(&n.path) {
            n.expanded = true;
            let path = n.path.clone();
            out.push(n);
            build(&path, depth + 1, expanded, out);
        } else {
            out.push(n);
        }
    }
}

impl Tree {
    pub fn new(root: PathBuf) -> Self {
        let nodes = read_dir(&root, 0);
        Tree {
            root,
            nodes,
            sel: 0,
            scroll: 0,
            visible: true,
            width: 28,
            query: String::new(),
            typing: false,
            stash: None,
            files: None,
        }
    }

    /// Sets the search box's text: the tree shows the files whose name has it
    /// (or whose path has it, with a `/` in it), case aside, with the folders
    /// they're in opened; empty puts the tree back as it was.
    pub fn set_query(&mut self, q: &str) {
        self.query = q.to_string();
        if q.is_empty() {
            if let Some((nodes, sel, scroll)) = self.stash.take() {
                self.nodes = nodes;
                self.sel = sel;
                self.scroll = scroll;
                self.refresh();
            }
            self.files = None;
            return;
        }
        if self.stash.is_none() {
            self.stash = Some((std::mem::take(&mut self.nodes), self.sel, self.scroll));
        }
        if self.files.is_none() {
            self.files = Some(list_files(&self.root));
        }
        self.nodes = search(&self.root, self.files.as_deref().unwrap_or(&[]), q);
        self.sel = self.nodes.iter().position(|n| !n.is_dir).unwrap_or(0);
        self.scroll = 0;
    }

    pub fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// How many files the search found.
    pub fn found(&self) -> usize {
        self.nodes.iter().filter(|n| !n.is_dir).count()
    }

    pub fn root_name(&self) -> String {
        self.root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.query.clear();
        self.typing = false;
        self.stash = None;
        self.files = None;
        self.nodes = read_dir(&root, 0);
        self.root = root;
        self.sel = 0;
        self.scroll = 0;
    }

    pub fn toggle(&mut self, i: usize) {
        if i >= self.nodes.len() || !self.nodes[i].is_dir {
            return;
        }
        if self.nodes[i].expanded {
            let d = self.nodes[i].depth;
            let mut j = i + 1;
            while j < self.nodes.len() && self.nodes[j].depth > d {
                j += 1;
            }
            self.nodes.drain(i + 1..j);
            self.nodes[i].expanded = false;
            if self.sel > i && self.sel < j {
                self.sel = i;
            }
        } else {
            let kids = read_dir(&self.nodes[i].path, self.nodes[i].depth + 1);
            self.nodes.splice(i + 1..i + 1, kids);
            self.nodes[i].expanded = true;
        }
    }

    pub fn parent_of(&self, i: usize) -> Option<usize> {
        let d = self.nodes.get(i)?.depth;
        if d == 0 {
            return None;
        }
        (0..i).rev().find(|&j| self.nodes[j].depth == d - 1)
    }

    /// Rebuilds the tree with exactly these folders expanded.
    pub fn set_expanded(&mut self, expanded: &HashSet<PathBuf>) {
        let mut out = Vec::new();
        build(&self.root, 0, expanded, &mut out);
        self.nodes = out;
        self.sel = self.sel.min(self.nodes.len().saturating_sub(1));
    }

    /// Expands the folders down to `p` and selects it.
    pub fn reveal(&mut self, p: &Path) {
        let Ok(rel) = p.strip_prefix(&self.root) else { return };
        let comps: Vec<_> = rel.components().collect();
        let mut cur = self.root.clone();
        for (k, c) in comps.iter().enumerate() {
            cur.push(c);
            let Some(i) = self.nodes.iter().position(|n| n.path == cur) else {
                break;
            };
            if k + 1 < comps.len() && self.nodes[i].is_dir && !self.nodes[i].expanded {
                self.toggle(i);
            }
            self.sel = i;
        }
    }

    pub fn refresh(&mut self) {
        if self.searching() {
            // Files may have come or gone: list them again.
            let (q, sel) = (self.query.clone(), self.nodes.get(self.sel).map(|n| n.path.clone()));
            self.files = None;
            self.set_query(&q);
            if let Some(i) = sel.and_then(|p| self.nodes.iter().position(|n| n.path == p)) {
                self.sel = i;
            }
            return;
        }
        let expanded: HashSet<PathBuf> = self
            .nodes
            .iter()
            .filter(|n| n.expanded)
            .map(|n| n.path.clone())
            .collect();
        let sel_path = self.nodes.get(self.sel).map(|n| n.path.clone());
        let mut out = Vec::new();
        build(&self.root, 0, &expanded, &mut out);
        self.nodes = out;
        if let Some(p) = sel_path {
            self.sel = self.nodes.iter().position(|n| n.path == p).unwrap_or(0);
        }
        self.sel = self.sel.min(self.nodes.len().saturating_sub(1));
    }
}

/// The project's files, relative to `root`: `git ls-files` in a repository
/// (tracked and untracked, `.gitignore` respected), else a walk that leaves
/// out `IGNORED` and `NOT_SEARCHED` folders.
fn list_files(root: &Path) -> Vec<PathBuf> {
    let git = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    if let Some(o) = git.ok().filter(|o| o.status.success()) {
        let text = String::from_utf8_lossy(&o.stdout);
        let mut v: Vec<PathBuf> = text
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| {
                !p.components()
                    .any(|c| IGNORED.contains(&c.as_os_str().to_string_lossy().as_ref()))
            })
            .take(SEARCH_FILES)
            .collect();
        v.sort();
        v.dedup();
        return v;
    }
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(rd) = fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if IGNORED.contains(&name.as_str()) || NOT_SEARCHED.contains(&name.as_str()) {
                continue;
            }
            let p = rel.join(&name);
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(p);
            } else {
                out.push(p);
                if out.len() >= SEARCH_FILES {
                    return out;
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Folder {
    /// Lowercase name → (name, folder).
    dirs: BTreeMap<String, (String, Folder)>,
    files: Vec<String>,
}

/// The search's rows: matching files with the folders they're in (opened),
/// folders first, then files, by name, as the tree sorts them.
fn search(root: &Path, files: &[PathBuf], q: &str) -> Vec<Node> {
    let q = q.to_lowercase();
    let by_path = q.contains('/');
    let mut top = Folder::default();
    let mut n = 0;
    for f in files {
        let name = f
            .file_name()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let hit = if by_path {
            f.to_string_lossy().to_lowercase().contains(&q)
        } else {
            name.contains(&q)
        };
        if !hit {
            continue;
        }
        let comps: Vec<String> = f
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let Some((file, dirs)) = comps.split_last() else {
            continue;
        };
        let mut at = &mut top;
        for d in dirs {
            at = &mut at
                .dirs
                .entry(d.to_lowercase())
                .or_insert_with(|| (d.clone(), Folder::default()))
                .1;
        }
        at.files.push(file.clone());
        n += 1;
        if n >= SEARCH_ROWS {
            break;
        }
    }
    let mut out = Vec::new();
    flatten(&top, root, 0, &mut out);
    out
}

fn flatten(f: &Folder, at: &Path, depth: usize, out: &mut Vec<Node>) {
    for (name, sub) in f.dirs.values() {
        let path = at.join(name);
        out.push(Node {
            path: path.clone(),
            name: name.clone(),
            depth,
            is_dir: true,
            expanded: true,
        });
        flatten(sub, &path, depth + 1, out);
    }
    let mut files = f.files.clone();
    files.sort_by_key(|a| a.to_lowercase());
    for name in files {
        out.push(Node {
            path: at.join(&name),
            name,
            depth,
            is_dir: false,
            expanded: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keeps_the_tree_shape() {
        let files: Vec<PathBuf> = [
            "src/api/views.py",
            "src/api/models.py",
            "src/lib/view_utils.py",
            "README.md",
            "views.txt",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let rows: Vec<(usize, String, bool)> = search(Path::new("/p"), &files, "VIEW")
            .into_iter()
            .map(|n| (n.depth, n.name, n.is_dir))
            .collect();
        assert_eq!(
            rows,
            vec![
                (0, "src".into(), true),
                (1, "api".into(), true),
                (2, "views.py".into(), false),
                (1, "lib".into(), true),
                (2, "view_utils.py".into(), false),
                (0, "views.txt".into(), false),
            ]
        );
        // With a `/`, the path counts.
        let rows: Vec<String> = search(Path::new("/p"), &files, "api/m")
            .into_iter()
            .map(|n| n.name)
            .collect();
        assert_eq!(rows, vec!["src", "api", "models.py"]);
    }

    #[test]
    fn query_and_back() {
        let dir = std::env::temp_dir().join(format!("ren-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/b/target.rs"), "").unwrap();
        fs::write(dir.join("top.rs"), "").unwrap();
        let mut t = Tree::new(dir.clone());
        let before: Vec<String> = t.nodes.iter().map(|n| n.name.clone()).collect();
        t.set_query("targ");
        assert_eq!(t.found(), 1);
        assert_eq!(t.nodes[t.sel].name, "target.rs");
        t.set_query("");
        let after: Vec<String> = t.nodes.iter().map(|n| n.name.clone()).collect();
        assert_eq!(before, after);
        let _ = fs::remove_dir_all(&dir);
    }
}
