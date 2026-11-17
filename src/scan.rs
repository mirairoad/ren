//! Project scan: problems in files that aren't open, so the tree and the tabs
//! can colour them. A second after a folder opens, ren starts the language
//! servers that check the whole project by themselves (rust-analyzer runs
//! `cargo check`, gopls checks every package, pyright/basedpyright switch to
//! workspace mode) and runs the command-line checker of the servers that only
//! look at open files (ruff, tsc), or of languages whose servers don't check
//! the whole project (`dotnet build` for C#, when the folder has a solution
//! or project file).
//!
//! The checkers run again after a save. They run in the background, one at a
//! time, at low priority, and a rerun waits at least as long as the last run
//! took, so a slow checker gets at most half the time.

use super::*;
use crate::lsp::{find_exe, find_marker, path_key, resolve, root_markers};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;

/// How long after a folder opens the scan starts, so opening stays instant.
const START_DELAY: Duration = Duration::from_secs(1);
/// A rerun after a save waits this long for more saves.
const SAVE_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone)]
struct Checker {
    name: &'static str,
    /// Saving a file of this language reruns it.
    lang: &'static str,
    exe: PathBuf,
    root: PathBuf,
}

struct Run {
    name: &'static str,
    rx: Receiver<Option<HashMap<String, Counts>>>,
    started: Instant,
}

#[derive(Default)]
pub struct Scan {
    /// The folder scanned; another one starts over.
    root: Option<PathBuf>,
    /// When the scan starts.
    start_at: Option<Instant>,
    checkers: Vec<Checker>,
    /// Checkers waiting to run, each with the time it may start.
    queue: Vec<(&'static str, Instant)>,
    running: Option<Run>,
    /// Each checker's last run: when it ended and how long it took.
    took: HashMap<&'static str, (Instant, Duration)>,
    /// What each checker found: `path_key` → (errors, warnings).
    pub found: HashMap<&'static str, HashMap<String, Counts>>,
}

impl Editor {
    /// Starts the scan when a folder has been open for a second, runs queued
    /// checkers and picks up their results. True = redraw.
    pub(crate) fn scan_tick(&mut self) -> bool {
        self.lsp.scan = self.settings.scan;
        if !self.settings.scan {
            let had = !self.scan.found.is_empty() || self.scan.running.is_some();
            if self.scan.root.is_some() {
                self.scan = Scan::default();
            }
            return had;
        }
        if self.scan.root.as_ref() != Some(&self.tree.root) {
            let had = !self.scan.found.is_empty() || self.scan.running.is_some();
            self.scan = Scan {
                root: Some(self.tree.root.clone()),
                start_at: Some(Instant::now() + START_DELAY),
                ..Default::default()
            };
            return had;
        }
        if self.scan.start_at.is_some_and(|t| Instant::now() >= t) {
            self.scan.start_at = None;
            self.scan_start(false);
        }
        let mut changed = false;
        if let Some(run) = &self.scan.running {
            let got = match run.rx.try_recv() {
                Ok(found) => Some(found),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(None),
            };
            if let Some(found) = got {
                let (name, took) = (run.name, run.started.elapsed());
                self.scan.running = None;
                self.scan.took.insert(name, (Instant::now(), took));
                // A checker that couldn't run keeps what it found last time.
                if let Some(f) = found {
                    self.scan.found.insert(name, f);
                }
                changed = true;
            }
        }
        if self.scan.running.is_none() {
            let now = Instant::now();
            let s = &self.scan;
            let ready = s
                .queue
                .iter()
                .position(|&(name, at)| now >= at && s.took.get(name).is_none_or(|&(end, took)| now >= end + took));
            if let Some(i) = ready {
                let (name, _) = self.scan.queue.remove(i);
                if let Some(c) = self.scan.checkers.iter().find(|c| c.name == name).cloned() {
                    self.scan.running = Some(spawn(c));
                    changed = true;
                }
            }
        }
        changed
    }

    /// Starts the servers that check the whole project and queues the
    /// checkers. `manual` (`:scan`) also reruns what's already up and says so.
    fn scan_start(&mut self, manual: bool) {
        let root = self.tree.root.clone();
        let marker = |lang: &str| root_markers(lang).iter().find_map(|m| find_marker(&root, m));
        let mut servers: Vec<(&'static str, PathBuf)> = Vec::new();
        let mut checkers: Vec<Checker> = Vec::new();
        for lang in ["Rust", "Go"] {
            if let Some(m) = marker(lang) {
                servers.push((lang, m));
            }
        }
        if let Some(m) = marker("Python") {
            let conf = self.lsp_config("Python");
            let (specs, _) = resolve("Python", &[root.as_path()], conf.as_ref());
            let name = |s: &[String]| {
                Path::new(&s[0])
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            // pyright/basedpyright check the whole project themselves.
            if specs.iter().any(|(_, s)| name(s).contains("pyright")) {
                servers.push(("Python", m));
            }
            if let Some((exe, _)) = specs.iter().find(|(_, s)| name(s) == "ruff") {
                checkers.push(Checker {
                    name: "ruff",
                    lang: "Python",
                    exe: exe.clone(),
                    root: root.clone(),
                });
            }
        }
        if root.join("tsconfig.json").is_file() {
            if let Some(exe) = find_exe("tsc", &[root.as_path()]) {
                checkers.push(Checker {
                    name: "tsc",
                    lang: "JS/TS",
                    exe,
                    root: root.clone(),
                });
            }
        }
        if let Some(target) = dotnet_target(&root) {
            if let Some(exe) = find_exe("dotnet", &[root.as_path()]) {
                checkers.push(Checker {
                    name: "dotnet build",
                    lang: "C#",
                    exe,
                    root: target,
                });
            }
        }
        let mut tools: Vec<String> = Vec::new();
        for (lang, m) in servers {
            if self.lsp.alive_lang(lang) {
                if manual && lang == "Rust" {
                    // rust-analyzer checks again without a save.
                    for id in self.lsp.ids(lang) {
                        self.lsp
                            .notify(id, "rust-analyzer/runFlycheck", json!({ "textDocument": Value::Null }));
                    }
                }
                tools.extend(self.lsp.ids(lang).iter().map(|id| self.lsp.servers[id].name.clone()));
                continue;
            }
            if self.lsp.failed.contains(lang) {
                continue;
            }
            let conf = self.lsp_config(lang);
            match self.lsp.start(lang, &m, &root, conf.as_ref()) {
                Ok((names, _)) => tools.extend(names),
                // Not installed: opening a file of that language says so.
                Err(_) => {
                    self.lsp.failed.remove(lang);
                }
            }
        }
        let now = Instant::now();
        if manual {
            self.scan.took.clear();
        }
        self.scan.queue = checkers.iter().map(|c| (c.name, now)).collect();
        tools.extend(checkers.iter().map(|c| c.name.to_string()));
        self.scan.checkers = checkers;
        if manual && tools.is_empty() {
            self.info("Nothing checks this project as a whole (see :help, Project scan)");
        } else if !tools.is_empty() {
            self.info(format!("Checking the project: {}…", tools.join(", ")));
        }
    }

    /// After a save: the checkers for that language (or all of them, for a
    /// config file) run again once saves pause.
    pub(crate) fn scan_saved(&mut self) {
        if !self.settings.scan || self.scan.checkers.is_empty() {
            return;
        }
        let b = self.buf();
        let name = b.name.to_lowercase();
        let config = matches!(
            name.as_str(),
            "pyproject.toml"
                | "ruff.toml"
                | ".ruff.toml"
                | "package.json"
                | "setup.cfg"
                | "directory.build.props"
                | "directory.packages.props"
                | "global.json"
        ) || (name.starts_with("tsconfig") && name.ends_with(".json"))
            || [".csproj", ".sln", ".slnx"].iter().any(|e| name.ends_with(e));
        let lang = b.lang.name;
        let at = Instant::now() + SAVE_DELAY;
        for c in &self.scan.checkers {
            if c.lang != lang && !config {
                continue;
            }
            match self.scan.queue.iter_mut().find(|q| q.0 == c.name) {
                Some(q) => q.1 = at,
                None => self.scan.queue.push((c.name, at)),
            }
        }
    }

    /// `:scan`: check the project again now.
    pub(crate) fn scan_command(&mut self) {
        if !self.settings.scan {
            return self.error("The project scan is off (\"scan\": false in settings)");
        }
        self.scan.root = Some(self.tree.root.clone());
        self.scan.start_at = None;
        self.scan_start(true);
    }

    /// The checker running now, for the status bar.
    pub(crate) fn scan_status(&self) -> Option<String> {
        self.scan.running.as_ref().map(|r| format!("{}…", r.name))
    }

    /// What the checkers found, for files no live server watches: an open file
    /// with a server running shows the server's view, which follows unsaved
    /// edits.
    pub(crate) fn scan_found(&self) -> Vec<(&String, Counts)> {
        let live: Vec<String> = self
            .bufs
            .iter()
            .filter(|b| b.lsp_open)
            .filter_map(|b| b.path.as_deref().map(path_key))
            .collect();
        self.scan
            .found
            .values()
            .flat_map(|m| m.iter())
            .filter(|(k, _)| !live.contains(k))
            .map(|(k, &s)| (k, s))
            .collect()
    }
}

fn spawn(c: Checker) -> Run {
    let (tx, rx) = channel();
    let name = c.name;
    thread::spawn(move || {
        let _ = tx.send(check(&c));
    });
    Run {
        name,
        rx,
        started: Instant::now(),
    }
}

/// Runs a checker over the project: None when it couldn't run.
fn check(c: &Checker) -> Option<HashMap<String, Counts>> {
    match c.name {
        "ruff" => {
            let args = ["check", "--output-format", "json", "--exit-zero", "--no-cache", "."];
            parse_ruff(&run(c, args.iter().map(OsString::from).collect())?)
        }
        "tsc" => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            c.root.hash(&mut h);
            // Incremental: reruns only recheck what changed.
            let info = env::temp_dir().join(format!("ren-tsc-{:016x}.tsbuildinfo", h.finish()));
            let base = ["--noEmit", "--pretty", "false", "-p", "tsconfig.json"];
            let mut args: Vec<OsString> = base.iter().map(OsString::from).collect();
            args.extend([
                "--incremental".into(),
                "--tsBuildInfoFile".into(),
                info.into_os_string(),
            ]);
            let out = run(c, args)?;
            let found = parse_tsc(&out, &c.root);
            // Some project setups refuse the incremental flags: try without.
            if found.is_empty() && out.contains("error TS") {
                let out = run(c, base.iter().map(OsString::from).collect())?;
                return Some(parse_tsc(&out, &c.root));
            }
            Some(found)
        }
        "dotnet build" => {
            // `root` is the solution or project file; the build runs next to it.
            let args = [
                "build",
                "-nologo",
                "-v:q",
                "-clp:NoSummary",
                "-nodeReuse:false",
                "-p:GenerateFullPaths=true",
            ];
            let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
            args.insert(1, c.root.clone().into_os_string());
            let dir = c.root.parent().unwrap_or(&c.root).to_path_buf();
            let out = run(
                &Checker {
                    root: dir.clone(),
                    ..c.clone()
                },
                args,
            )?;
            // `File.cs(3,7): error CS0103: …`, the shape tsc prints.
            Some(parse_tsc(&out, &dir))
        }
        _ => None,
    }
}

/// What `dotnet build` builds for a folder: its one solution (`.sln`, `.slnx`)
/// or else its one project (`.csproj`). None with none, or several to choose
/// from (`dotnet build` refuses to guess too).
fn dotnet_target(root: &Path) -> Option<PathBuf> {
    let with = |exts: &[&str]| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
            })
            .collect();
        v.sort();
        v
    };
    for exts in [&["sln", "slnx"][..], &["csproj"]] {
        let mut found = with(exts);
        match found.len() {
            0 => continue,
            1 => return found.pop(),
            _ => return None,
        }
    }
    None
}

/// Runs the checker's program in the project folder at low priority, and
/// returns what it printed.
fn run(c: &Checker, args: Vec<OsString>) -> Option<String> {
    let nice = Path::new("/usr/bin/nice");
    let mut cmd = if cfg!(unix) && nice.exists() {
        let mut k = Command::new(nice);
        k.args(["-n", "10"]).arg(&c.exe);
        k
    } else {
        Command::new(&c.exe)
    };
    let out = cmd
        .args(args)
        .current_dir(&c.root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A file's problems: (errors, warnings).
pub type Counts = (u32, u32);

fn count(m: &mut HashMap<String, Counts>, key: String, sev: u8) {
    let e = m.entry(key).or_default();
    if sev == 1 {
        e.0 += 1;
    } else {
        e.1 += 1;
    }
}

/// `ruff check --output-format json`, with ruff's server severities: syntax
/// errors, undefined names and unreadable files are errors, the rest warnings.
fn parse_ruff(out: &str) -> Option<HashMap<String, Counts>> {
    let v: Value = serde_json::from_str(out).ok()?;
    let mut m = HashMap::new();
    for d in v.as_array()? {
        let Some(f) = d["filename"].as_str() else { continue };
        let sev = match d["code"].as_str() {
            None | Some("invalid-syntax" | "F821" | "E902" | "E999") => 1,
            _ => 2,
        };
        count(&mut m, path_key(Path::new(f)), sev);
    }
    Some(m)
}

/// `tsc --pretty false`: `src/a.ts(3,7): error TS2322: …`, paths relative to
/// the project. Indented lines continue the message above.
fn parse_tsc(out: &str, root: &Path) -> HashMap<String, Counts> {
    let mut m = HashMap::new();
    for line in out.lines().filter(|l| !l.starts_with(char::is_whitespace)) {
        let Some((loc, rest)) = line.split_once("): ") else {
            continue;
        };
        let Some((file, _)) = loc.rsplit_once('(') else {
            continue;
        };
        let sev = if rest.starts_with("error") {
            1
        } else if rest.starts_with("warning") {
            2
        } else {
            continue;
        };
        count(&mut m, path_key(&absolute(&root.join(file))), sev);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruff_output() {
        let out = r#"[
            {"code": "F401", "filename": "/p/a.py", "message": "unused"},
            {"code": "F821", "filename": "/p/a.py", "message": "undefined"},
            {"code": "I001", "filename": "/p/b.py", "message": "unsorted"},
            {"code": null, "filename": "/p/c.py", "message": "syntax"},
            {"code": "invalid-syntax", "filename": "/p/d.py", "message": "syntax"}
        ]"#;
        let m = parse_ruff(out).unwrap();
        assert_eq!(m.get("/p/a.py"), Some(&(1, 1)));
        assert_eq!(m.get("/p/b.py"), Some(&(0, 1)));
        assert_eq!(m.get("/p/c.py"), Some(&(1, 0)));
        assert_eq!(m.get("/p/d.py"), Some(&(1, 0)));
        assert!(parse_ruff("ruff failed").is_none());
    }

    #[test]
    fn dotnet_output_and_target() {
        let out = "/p/Program.cs(2,19): error CS0103: The name 'y' does not exist [/p/App.csproj]\n\
                   /p/Other.cs(1,30): warning CS0168: The variable 'unused' is declared but never used [/p/App.csproj]\n\
                   /p/App.csproj : error NU1101: Unable to find package Nope.\n";
        let m = parse_tsc(out, Path::new("/p"));
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("/p/Program.cs"), Some(&(1, 0)));
        assert_eq!(m.get("/p/Other.cs"), Some(&(0, 1)));

        let dir = env::temp_dir().join(format!("ren-dotnet-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(dotnet_target(&dir), None);
        fs::write(dir.join("App.csproj"), "").unwrap();
        assert_eq!(dotnet_target(&dir), Some(dir.join("App.csproj")));
        // A solution wins over the projects next to it.
        fs::write(dir.join("App.sln"), "").unwrap();
        fs::write(dir.join("Tests.csproj"), "").unwrap();
        assert_eq!(dotnet_target(&dir), Some(dir.join("App.sln")));
        fs::write(dir.join("Other.sln"), "").unwrap();
        assert_eq!(dotnet_target(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tsc_output() {
        let out = "src/a.ts(3,7): error TS2322: Type 'string' is not assignable to type 'number'.\n  \
                   The expected type comes from here.\n\
                   src/lib/../b.ts(1,1): warning TS6133: 'x' is declared but never used.\n\
                   error TS5083: Cannot read file 'tsconfig.base.json'.\n";
        let m = parse_tsc(out, Path::new("/p"));
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("/p/src/a.ts"), Some(&(1, 0)));
        assert_eq!(m.get("/p/src/b.ts"), Some(&(0, 1)));
    }
}
