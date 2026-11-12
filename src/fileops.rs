//! File commands (`:touch`, `:mkdir`, `:cp`, `:mv`, `:duplicate`, `:rm`) and the
//! `.bin` folder that makes them undoable.
//!
//! Nothing is ever hard-deleted: removed or overwritten files are moved into
//! `<workspace>/.bin/<slot>/<name>`. Each command is recorded as an `Op` made of
//! `Step`s (all of them renames), so undo and redo just replay the renames
//! backwards or forwards. The history lives in `.bin/index.json` and survives
//! restarts.

use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const BIN: &str = ".bin";
const MAX_OPS: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// `path` was created. While the op is undone it waits in bin `slot`.
    Made { path: PathBuf, slot: Option<String> },
    /// `path` was moved into bin `slot`.
    Trashed { path: PathBuf, slot: String },
    /// `from` was renamed to `to`.
    Moved { from: PathBuf, to: PathBuf },
}

#[derive(Clone, Debug)]
pub struct Op {
    pub label: String,
    pub time: u64,
    pub steps: Vec<Step>,
}

/// Paths in `Step`s are stored relative to the workspace root.
pub struct Bin {
    pub root: PathBuf,
    pub undo: Vec<Op>,
    pub redo: Vec<Op>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

/// Rename, falling back to copy + delete across file systems.
fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p)?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_path(from, to)?;
            if from.is_dir() {
                fs::remove_dir_all(from)
            } else {
                fs::remove_file(from)
            }
        }
    }
}

/// Copies a file, or a folder recursively.
pub fn copy_path(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            copy_path(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(from, to).map(|_| ())
    }
}

/// The topmost missing ancestor folder of `p`, if any (what `create_dir_all` would create first).
fn top_missing_parent(p: &Path) -> Option<PathBuf> {
    let mut top = None;
    let mut cur = p.parent();
    while let Some(d) = cur {
        if d.exists() || d.as_os_str().is_empty() {
            break;
        }
        top = Some(d.to_path_buf());
        cur = d.parent();
    }
    top
}

/// `name.ext` → `name_copy.ext`, then `name_copy2.ext`, … (first that doesn't exist).
pub fn duplicate_name(p: &Path) -> PathBuf {
    let dir = p.parent().unwrap_or(Path::new(""));
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = if p.is_dir() {
        (name.clone(), String::new())
    } else {
        match name.rfind('.') {
            Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
            _ => (name.clone(), String::new()),
        }
    };
    let mut n = 1;
    loop {
        let suffix = if n == 1 {
            "_copy".to_string()
        } else {
            format!("_copy{}", n)
        };
        let c = dir.join(format!("{}{}{}", stem, suffix, ext));
        if !c.exists() {
            return c;
        }
        n += 1;
    }
}

fn step_json(s: &Step) -> Value {
    let p = |p: &Path| p.to_string_lossy().into_owned();
    match s {
        Step::Made { path, slot } => json!({ "made": p(path), "slot": slot }),
        Step::Trashed { path, slot } => json!({ "trashed": p(path), "slot": slot }),
        Step::Moved { from, to } => json!({ "from": p(from), "to": p(to) }),
    }
}

fn step_from(v: &Value) -> Option<Step> {
    if let Some(m) = v["made"].as_str() {
        return Some(Step::Made {
            path: PathBuf::from(m),
            slot: v["slot"].as_str().map(String::from),
        });
    }
    if let Some(t) = v["trashed"].as_str() {
        return Some(Step::Trashed {
            path: PathBuf::from(t),
            slot: v["slot"].as_str()?.to_string(),
        });
    }
    Some(Step::Moved {
        from: PathBuf::from(v["from"].as_str()?),
        to: PathBuf::from(v["to"].as_str()?),
    })
}

fn op_json(o: &Op) -> Value {
    json!({ "label": o.label, "time": o.time, "steps": o.steps.iter().map(step_json).collect::<Vec<_>>() })
}

fn op_from(v: &Value) -> Option<Op> {
    Some(Op {
        label: v["label"].as_str().unwrap_or("").to_string(),
        time: v["time"].as_u64().unwrap_or(0),
        steps: v["steps"].as_array()?.iter().filter_map(step_from).collect(),
    })
}

impl Bin {
    /// Loads `<root>/.bin/index.json` (or starts empty).
    pub fn open(root: &Path) -> Bin {
        let mut b = Bin {
            root: root.to_path_buf(),
            undo: Vec::new(),
            redo: Vec::new(),
        };
        if let Ok(text) = fs::read_to_string(b.dir().join("index.json")) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                let ops = |k: &str| -> Vec<Op> {
                    v[k].as_array()
                        .map(|a| a.iter().filter_map(op_from).collect())
                        .unwrap_or_default()
                };
                b.undo = ops("undo");
                b.redo = ops("redo");
            }
        }
        b
    }

    pub fn dir(&self) -> PathBuf {
        self.root.join(BIN)
    }

    fn abs(&self, rel: &Path) -> PathBuf {
        self.root.join(rel)
    }

    fn rel(&self, p: &Path) -> PathBuf {
        p.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| p.to_path_buf())
    }

    pub fn save(&self) -> io::Result<()> {
        fs::create_dir_all(self.dir())?;
        let v = json!({
            "undo": self.undo.iter().map(op_json).collect::<Vec<_>>(),
            "redo": self.redo.iter().map(op_json).collect::<Vec<_>>(),
        });
        fs::write(
            self.dir().join("index.json"),
            serde_json::to_string_pretty(&v).unwrap_or_default(),
        )
    }

    /// The single entry stored in a slot folder.
    fn slot_entry(&self, slot: &str) -> io::Result<PathBuf> {
        let d = self.dir().join(slot);
        fs::read_dir(&d)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .next()
            .ok_or_else(|| err(format!("bin slot {} is empty", slot)))
    }

    fn new_slot(&self) -> String {
        let t = now();
        let mut n = 0;
        loop {
            let s = format!("{}-{}", t, n);
            if !self.dir().join(&s).exists() {
                return s;
            }
            n += 1;
        }
    }

    /// Moves `rel` into a fresh (or the given) slot.
    fn trash(&self, rel: &Path, slot: Option<&str>) -> io::Result<String> {
        let path = self.abs(rel);
        if !path.exists() && fs::symlink_metadata(&path).is_err() {
            return Err(err(format!("{} does not exist", rel.display())));
        }
        let slot = slot.map(String::from).unwrap_or_else(|| self.new_slot());
        let name = path
            .file_name()
            .ok_or_else(|| err("cannot remove the workspace root"))?;
        move_path(&path, &self.dir().join(&slot).join(name))?;
        Ok(slot)
    }

    fn restore(&self, slot: &str, rel: &Path) -> io::Result<()> {
        let path = self.abs(rel);
        if path.exists() {
            return Err(err(format!("{} already exists", rel.display())));
        }
        let src = self.slot_entry(slot)?;
        move_path(&src, &path)?;
        let _ = fs::remove_dir(self.dir().join(slot));
        Ok(())
    }

    fn record(&mut self, label: String, steps: Vec<Step>) -> io::Result<()> {
        self.undo.push(Op {
            label,
            time: now(),
            steps,
        });
        if self.undo.len() > MAX_OPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.save()
    }

    /// Records the creation of `path`: the topmost new parent folder if parents
    /// had to be created, otherwise the path itself.
    fn made(&self, path: &Path, top: Option<PathBuf>) -> Step {
        Step::Made {
            path: self.rel(&top.unwrap_or_else(|| path.to_path_buf())),
            slot: None,
        }
    }

    /// `touch`: creates an empty file and missing parents. An existing file only
    /// gets its modified time updated, as Unix `touch` does. Returns false then.
    pub fn touch(&mut self, path: &Path) -> io::Result<bool> {
        if path.exists() {
            fs::File::options()
                .append(true)
                .open(path)?
                .set_modified(SystemTime::now())?;
            return Ok(false);
        }
        let top = top_missing_parent(path);
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::File::create(path)?;
        let step = self.made(path, top);
        self.record(format!("touch {}", self.rel(path).display()), vec![step])?;
        Ok(true)
    }

    /// `mkdir -p`. Returns false when the folder already existed.
    pub fn mkdir(&mut self, path: &Path) -> io::Result<bool> {
        if path.is_dir() {
            return Ok(false);
        }
        if path.exists() {
            return Err(err(format!("{} is a file", self.rel(path).display())));
        }
        let top = top_missing_parent(path);
        fs::create_dir_all(path)?;
        let step = self.made(path, top);
        self.record(format!("mkdir {}", self.rel(path).display()), vec![step])?;
        Ok(true)
    }

    /// Where `cp`/`mv` put `src` when `dst` is an existing folder.
    pub fn target(src: &Path, dst: &Path) -> PathBuf {
        if dst.is_dir() {
            if let Some(n) = src.file_name() {
                return dst.join(n);
            }
        }
        dst.to_path_buf()
    }

    fn check_src(&self, src: &Path) -> io::Result<()> {
        if !src.exists() {
            return Err(err(format!("{} does not exist", self.rel(src).display())));
        }
        if src == self.root || src.starts_with(self.dir()) {
            return Err(err("cannot copy or move the workspace root or the bin"));
        }
        Ok(())
    }

    /// `cp`. Anything it would overwrite goes to the bin first. Returns the destination.
    pub fn copy(&mut self, src: &Path, dst: &Path) -> io::Result<PathBuf> {
        self.check_src(src)?;
        let to = Self::target(src, dst);
        if to.starts_with(src) && src.is_dir() {
            return Err(err("cannot copy a folder into itself"));
        }
        let mut steps = Vec::new();
        if to.exists() {
            let rel = self.rel(&to);
            let slot = self.trash(&rel, None)?;
            steps.push(Step::Trashed { path: rel, slot });
        }
        let top = top_missing_parent(&to);
        copy_path(src, &to)?;
        steps.push(self.made(&to, top));
        self.record(
            format!("cp {} {}", self.rel(src).display(), self.rel(&to).display()),
            steps,
        )?;
        Ok(to)
    }

    /// `duplicate`: copy next to the original as `name_copy.ext`.
    pub fn duplicate(&mut self, src: &Path) -> io::Result<PathBuf> {
        self.check_src(src)?;
        let to = duplicate_name(src);
        copy_path(src, &to)?;
        let step = self.made(&to, None);
        self.record(format!("duplicate {}", self.rel(src).display()), vec![step])?;
        Ok(to)
    }

    /// `mv`. Anything it would overwrite goes to the bin first. Returns the destination.
    pub fn mv(&mut self, src: &Path, dst: &Path) -> io::Result<PathBuf> {
        self.check_src(src)?;
        let to = Self::target(src, dst);
        if to == src {
            return Ok(to);
        }
        if to.starts_with(src) && src.is_dir() {
            return Err(err("cannot move a folder into itself"));
        }
        let mut steps = Vec::new();
        if to.exists() {
            let rel = self.rel(&to);
            let slot = self.trash(&rel, None)?;
            steps.push(Step::Trashed { path: rel, slot });
        }
        if let Some(top) = top_missing_parent(&to) {
            fs::create_dir_all(to.parent().unwrap_or(&top))?;
            steps.push(Step::Made {
                path: self.rel(&top),
                slot: None,
            });
        }
        move_path(src, &to)?;
        steps.push(Step::Moved {
            from: self.rel(src),
            to: self.rel(&to),
        });
        self.record(
            format!("mv {} {}", self.rel(src).display(), self.rel(&to).display()),
            steps,
        )?;
        Ok(to)
    }

    /// `rm`: moves the file or folder into the bin.
    pub fn rm(&mut self, path: &Path) -> io::Result<()> {
        if path == self.root || path.starts_with(self.dir()) {
            return Err(err("cannot remove the workspace root or the bin"));
        }
        let rel = self.rel(path);
        let slot = self.trash(&rel, None)?;
        self.record(format!("rm {}", rel.display()), vec![Step::Trashed { path: rel, slot }])
    }

    fn apply(&self, step: &mut Step, forward: bool) -> io::Result<()> {
        match step {
            Step::Made { path, slot } => {
                if forward {
                    if let Some(s) = slot.take() {
                        self.restore(&s, path)?;
                    }
                } else {
                    *slot = Some(self.trash(path, None)?);
                }
            }
            Step::Trashed { path, slot } => {
                if forward {
                    self.trash(path, Some(slot))?;
                } else {
                    self.restore(slot, path)?;
                }
            }
            Step::Moved { from, to } => {
                let (a, b) = if forward { (from, to) } else { (to, from) };
                let (a, b) = (self.abs(a), self.abs(b));
                if b.exists() {
                    return Err(err(format!("{} already exists", self.rel(&b).display())));
                }
                move_path(&a, &b)?;
            }
        }
        Ok(())
    }

    /// Undoes the last file command and returns it.
    pub fn undo(&mut self) -> io::Result<Option<Op>> {
        let Some(mut op) = self.undo.pop() else { return Ok(None) };
        let n = op.steps.len();
        for i in (0..n).rev() {
            if let Err(e) = self.apply(&mut op.steps[i], false) {
                self.undo.push(op);
                self.save()?;
                return Err(e);
            }
        }
        self.redo.push(op.clone());
        self.save()?;
        Ok(Some(op))
    }

    /// Redoes the last undone file command and returns it.
    pub fn redo(&mut self) -> io::Result<Option<Op>> {
        let Some(mut op) = self.redo.pop() else { return Ok(None) };
        for i in 0..op.steps.len() {
            if let Err(e) = self.apply(&mut op.steps[i], true) {
                self.redo.push(op);
                self.save()?;
                return Err(e);
            }
        }
        self.undo.push(op.clone());
        self.save()?;
        Ok(Some(op))
    }

    /// Number of slots (removed or overwritten items) in the bin.
    pub fn items(&self) -> usize {
        fs::read_dir(self.dir())
            .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count())
            .unwrap_or(0)
    }

    /// `:bin empty`: deletes everything in the bin and forgets the history.
    pub fn empty(&mut self) -> io::Result<usize> {
        let n = self.items();
        if self.dir().exists() {
            fs::remove_dir_all(self.dir())?;
        }
        self.undo.clear();
        self.redo.clear();
        Ok(n)
    }
}

/// True when `<root>/.gitignore` already ignores the bin.
pub fn gitignored(root: &Path) -> bool {
    fs::read_to_string(root.join(".gitignore"))
        .map(|t| {
            t.lines()
                .map(str::trim)
                .any(|l| l == ".bin" || l == ".bin/" || l == "/.bin" || l == "/.bin/")
        })
        .unwrap_or(false)
}

/// Appends lines to `.gitignore` that it doesn't already contain. Returns how many were added.
pub fn gitignore_add(root: &Path, lines: &[&str]) -> io::Result<usize> {
    let p = root.join(".gitignore");
    let mut text = fs::read_to_string(&p).unwrap_or_default();
    let have: Vec<String> = text
        .lines()
        .map(|l| l.trim().trim_end_matches('/').to_string())
        .collect();
    let mut added = 0;
    for l in lines {
        let key = l.trim().trim_end_matches('/');
        if l.is_empty() || l.starts_with('#') {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(l);
            text.push('\n');
            continue;
        }
        if have.iter().any(|h| h == key) {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(l);
        text.push('\n');
        added += 1;
    }
    if added > 0 {
        fs::write(&p, text)?;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ren-fileops-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn touch_rm_undo_redo() {
        let root = tmp("a");
        let mut bin = Bin::open(&root);
        let f = root.join("src/deep/file.rs");
        assert!(bin.touch(&f).unwrap());
        assert!(f.exists());
        assert!(!bin.touch(&f).unwrap());
        bin.undo().unwrap();
        assert!(!root.join("src").exists());
        bin.redo().unwrap();
        assert!(f.exists());
        fs::write(&f, "hello").unwrap();
        bin.rm(&f).unwrap();
        assert!(!f.exists());
        assert_eq!(bin.items(), 1);
        // history survives a reload
        let mut bin = Bin::open(&root);
        bin.undo().unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "hello");
        bin.redo().unwrap();
        assert!(!f.exists());
        assert_eq!(bin.empty().unwrap(), 1);
        assert!(!bin.dir().exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cp_mv_duplicate() {
        let root = tmp("b");
        let mut bin = Bin::open(&root);
        let a = root.join("a.txt");
        fs::write(&a, "A").unwrap();
        let d = bin.duplicate(&a).unwrap();
        assert_eq!(d, root.join("a_copy.txt"));
        assert_eq!(bin.duplicate(&a).unwrap(), root.join("a_copy2.txt"));
        fs::create_dir_all(root.join("dir")).unwrap();
        let to = bin.copy(&a, &root.join("dir")).unwrap();
        assert_eq!(to, root.join("dir/a.txt"));
        // overwrite goes to the bin and comes back on undo
        fs::write(root.join("b.txt"), "B").unwrap();
        bin.copy(&a, &root.join("b.txt")).unwrap();
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "A");
        bin.undo().unwrap();
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "B");
        let m = bin.mv(&a, &root.join("x/y/moved.txt")).unwrap();
        assert!(m.exists() && !a.exists());
        bin.undo().unwrap();
        assert!(a.exists() && !root.join("x").exists());
        assert!(bin.mv(&root.join("dir"), &root.join("dir/inner")).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_names_and_gitignore() {
        let root = tmp("c");
        assert_eq!(duplicate_name(&root.join("Makefile")), root.join("Makefile_copy"));
        assert_eq!(duplicate_name(&root.join(".env")), root.join(".env_copy"));
        assert!(!gitignored(&root));
        assert_eq!(gitignore_add(&root, &[".bin/", "target/"]).unwrap(), 2);
        assert!(gitignored(&root));
        assert_eq!(gitignore_add(&root, &[".bin", "target"]).unwrap(), 0);
        let _ = fs::remove_dir_all(&root);
    }
}
