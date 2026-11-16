//! Sessions, in `~/.ren/sessions/<id>/`: every open project with its tabs,
//! cursors, tree and panel, plus copies of unsaved text. One per running ren.
//!
//! - Written a few seconds after anything changes, so a crash, a killed process
//!   or a closed terminal loses at most those seconds. A hang-up (the terminal
//!   closing) or SIGTERM saves at once.
//! - A clean quit keeps the session (marked clean, without unsaved text, which
//!   quitting discarded).
//! - The welcome screen lists recent sessions, newest first, one per set of
//!   projects (besides ones with unsaved text, and windows that went down
//!   together, like several closed by one terminal crash); picking one reopens
//!   all its projects with their tabs. A session that didn't close cleanly says
//!   so, with its unsaved tabs.
//! - `:reload` saves the session, restarts ren and restores it.
//! - Each running ren holds a lock on its session folder, so another ren never
//!   lists (or takes over) a session that is still open.
//! - Checkpoints: besides its latest state (`session.json`, `unsaved/`), a
//!   session keeps the two before it in `1/` and `2/`. A new one starts when
//!   what's open (projects, tabs other than previews) changes after staying
//!   the same for a minute, so a layout that lasted a while survives the next
//!   change, a closed project or a crash. A clean quit that threw away unsaved
//!   text starts one too, so that text stays one step back. `:session` and
//!   `ren --resume` list them under their session; restoring one keeps the
//!   newer states as checkpoints.

use super::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io;

const AUTOSAVE_EVERY: Duration = Duration::from_secs(3);
/// Sessions kept on disk; older ones are deleted.
const KEEP: usize = 12;
/// Sessions the welcome screen lists.
pub const SHOWN: usize = 5;
/// States a session keeps: the latest and the checkpoints before it.
const CHECKPOINTS: usize = 3;
/// Interrupted sessions saved this close to the newest one for the same
/// projects went down with it (several windows, one crash): they're all kept.
const TOGETHER_SECS: u64 = 60;
/// How long what's open must stay the same before a change starts a new
/// checkpoint (quicker changes just update the latest one).
const STABLE_SECS: u64 = 60;

/// A saved session, as the welcome screen and `:session` list it.
pub struct SessionInfo {
    pub id: String,
    pub roots: Vec<PathBuf>,
    pub tabs: usize,
    /// Unix seconds of the last save.
    pub saved: u64,
    pub clean: bool,
    pub unsaved: usize,
    /// Where it's kept: 0 for `session.json`, k for the checkpoint in `k/`.
    pub step: usize,
    /// The session's earlier checkpoints, newest first.
    pub earlier: Vec<SessionInfo>,
}

impl SessionInfo {
    /// `ren + alpha`: the projects' folder names, the one that was on screen first.
    pub fn title(&self) -> String {
        let names: Vec<String> = self
            .roots
            .iter()
            .map(|r| {
                r.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| r.display().to_string())
            })
            .collect();
        names.join(" + ")
    }

    /// `3 tabs · 2 h ago`, and the warning part (`interrupted · 2 unsaved`).
    pub fn detail(&self) -> (String, String) {
        let tabs = match self.tabs {
            0 => "no tabs".to_string(),
            1 => "1 tab".to_string(),
            n => format!("{} tabs", n),
        };
        let base = format!("{} · {}", tabs, ago(now_secs().saturating_sub(self.saved)));
        let mut warn = Vec::new();
        if !self.clean {
            warn.push("interrupted".to_string());
        }
        if self.unsaved > 0 {
            warn.push(format!("{} unsaved", self.unsaved));
        }
        (base, warn.join(" · "))
    }
}

pub fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A new session's folder name: when it started and the process id.
pub fn new_id() -> String {
    format!("{}-{}", now_secs(), std::process::id())
}

pub fn sessions_dir() -> Option<PathBuf> {
    Some(crate::settings::dir()?.join("sessions"))
}

fn lock_file(dir: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("lock"))
}

/// True when another ren holds the session's lock (it's still open there).
fn is_live(dir: &Path) -> bool {
    match lock_file(dir) {
        Ok(f) => matches!(f.try_lock(), Err(fs::TryLockError::WouldBlock)),
        Err(_) => false,
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Where a session keeps state `step`: itself for the latest, `k/` for the
/// checkpoint k steps back.
fn step_dir(dir: &Path, step: usize) -> PathBuf {
    if step == 0 {
        dir.to_path_buf()
    } else {
        dir.join(step.to_string())
    }
}

/// What's open, ignoring order, cursors and preview tabs: when this changes
/// after a while, a new checkpoint starts.
fn shape_of(v: &Value) -> u64 {
    let mut parts: Vec<String> = v["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let mut tabs: Vec<&str> = p["tabs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| !t["preview"].as_bool().unwrap_or(false))
                .map(|t| t["path"].as_str().or(t["name"].as_str()).unwrap_or(""))
                .collect();
            tabs.sort_unstable();
            format!("{}\n{}", p["root"].as_str().unwrap_or(""), tabs.join("\n"))
        })
        .collect();
    parts.sort();
    fnv(&parts.join("\0")).max(1)
}

/// Pushes the latest state back one step (the oldest goes). It's copied, not
/// moved, so a crash before the new one is written still finds it.
fn push_checkpoint(dir: &Path) -> io::Result<()> {
    let _ = fs::remove_dir_all(step_dir(dir, CHECKPOINTS - 1));
    for k in (1..CHECKPOINTS - 1).rev() {
        let from = step_dir(dir, k);
        if from.exists() {
            fs::rename(&from, step_dir(dir, k + 1))?;
        }
    }
    let to = step_dir(dir, 1);
    fs::create_dir_all(&to)?;
    let ud = dir.join("unsaved");
    if let Ok(rd) = fs::read_dir(&ud) {
        fs::create_dir_all(to.join("unsaved"))?;
        for e in rd.flatten() {
            fs::copy(e.path(), to.join("unsaved").join(e.file_name()))?;
        }
    }
    fs::copy(dir.join("session.json"), to.join("session.json"))?;
    Ok(())
}

/// A session's states, latest first: the one to list, with the others as its
/// earlier checkpoints. None when nothing in it can be read.
fn read_session(id: &str, dir: &Path) -> Option<SessionInfo> {
    let mut states: Vec<SessionInfo> = (0..CHECKPOINTS)
        .filter_map(|k| {
            let v = read_json(&step_dir(dir, k).join("session.json"))?;
            let mut s = info_of(id, &v);
            s.step = k;
            Some(s)
        })
        .collect();
    if states.is_empty() {
        return None;
    }
    let mut s = states.remove(0);
    for e in &mut states {
        // Only the latest state says how the session ended.
        e.clean = true;
    }
    s.earlier = states;
    Some(s)
}

fn info_of(id: &str, v: &Value) -> SessionInfo {
    let projects = v["projects"].as_array().cloned().unwrap_or_default();
    let here = v["project"].as_u64().unwrap_or(0) as usize;
    let mut roots: Vec<PathBuf> = projects
        .iter()
        .filter_map(|p| p["root"].as_str().map(PathBuf::from))
        .collect();
    if here < roots.len() {
        let r = roots.remove(here);
        roots.insert(0, r);
    }
    let tabs = projects.iter().map(|p| p["tabs"].as_array().map_or(0, Vec::len)).sum();
    let unsaved = projects
        .iter()
        .flat_map(|p| p["tabs"].as_array().cloned().unwrap_or_default())
        .filter(|t| t["unsaved"].is_string())
        .count();
    SessionInfo {
        id: id.to_string(),
        roots,
        tabs,
        saved: v["saved"].as_u64().unwrap_or(0),
        clean: v["clean"].as_bool().unwrap_or(false),
        unsaved,
        step: 0,
        earlier: Vec::new(),
    }
}

/// Saved sessions other than `own`, newest first: one per set of projects (an
/// older one stays when it holds unsaved text, or went down together with the
/// newest), and no session still open in another ren. Deletes the duplicates
/// and everything past `KEEP`.
pub fn list_sessions(own: &str) -> Vec<SessionInfo> {
    let Some(base) = sessions_dir() else { return Vec::new() };
    let Ok(rd) = fs::read_dir(&base) else { return Vec::new() };
    let mut all: Vec<SessionInfo> = Vec::new();
    for e in rd.flatten() {
        let id = e.file_name().to_string_lossy().into_owned();
        let dir = e.path();
        if id == own || !dir.is_dir() || is_live(&dir) {
            continue;
        }
        match read_session(&id, &dir) {
            Some(s) => all.push(s),
            // Unreadable or half-made: nothing to restore.
            None => {
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }
    all.sort_by_key(|s| std::cmp::Reverse(s.saved));
    // The newest save of each set of projects.
    let mut newest: HashMap<Vec<String>, u64> = HashMap::new();
    let mut kept = Vec::new();
    for s in all {
        let mut key: Vec<String> = s.roots.iter().map(|r| path_key(r)).collect();
        key.sort();
        let wanted = match newest.get(&key) {
            None => {
                newest.insert(key, s.saved);
                true
            }
            Some(&t) => s.unsaved > 0 || (!s.clean && t.saturating_sub(s.saved) <= TOGETHER_SECS),
        };
        if wanted && kept.len() < KEEP {
            kept.push(s);
        } else {
            let _ = fs::remove_dir_all(base.join(&s.id));
        }
    }
    kept
}

/// What one project contributes to the session: its state, and the tabs whose
/// text gets copied (file name, tab, revision).
type ProjectState = (Value, Vec<(String, usize, u64)>);

/// `~/.ren/session/<folder name>-<hash of the full path>`: where ren before the
/// session list kept one session per folder. Only read, for a `:reload` from
/// such a ren into this one.
fn legacy_dir(root: &Path) -> Option<PathBuf> {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".into());
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(
        crate::settings::dir()?
            .join("session")
            .join(format!("{}-{:016x}", safe, fnv(&root.display().to_string()))),
    )
}

impl Editor {
    /// The project on screen: its tabs, tree, panel. `j` names the copies of
    /// unsaved text (`<j>-<tab>.txt`); `clean` leaves them out.
    fn project_state(&self, j: usize, clean: bool) -> ProjectState {
        let mut unsaved = Vec::new();
        if !clean {
            for (i, b) in self.bufs.iter().enumerate() {
                let untitled_text = b.path.is_none() && b.name == NONAME && !self.is_pristine(i);
                if b.dirty && (b.path.is_some() || untitled_text) {
                    unsaved.push((format!("{}-{}.txt", j, i), i, b.rev));
                }
            }
        }
        let tabs: Vec<Value> = self
            .bufs
            .iter()
            .enumerate()
            .filter(|(i, b)| b.path.is_some() || unsaved.iter().any(|u| u.1 == *i))
            .map(|(i, b)| {
                json!({
                    "path": b.path.as_ref().map(|p| p.display().to_string()),
                    "name": b.name,
                    "cx": b.cx,
                    "cy": b.cy,
                    "scroll_y": b.scroll_y,
                    "preview": b.preview,
                    "unsaved": unsaved.iter().find(|u| u.1 == i).map(|u| u.0.clone()),
                    "current": i == self.cur,
                })
            })
            .collect();
        let expanded: Vec<String> = self
            .tree
            .nodes
            .iter()
            .filter(|n| n.expanded)
            .map(|n| n.path.display().to_string())
            .collect();
        let v = json!({
            "root": self.tree.root.display().to_string(),
            "tabs": tabs,
            "focus": match self.focus { Focus::Tree => "tree", Focus::Panel => "panel", _ => "editor" },
            "tree": {
                "visible": self.tree.visible,
                "width": self.tree.width,
                "expanded": expanded,
                "sel": self.tree.nodes.get(self.tree.sel).map(|n| n.path.display().to_string()),
            },
            "panel": { "visible": self.dbg.visible, "height": self.dbg.height, "tab": self.panel_tab_name() },
        });
        (v, unsaved)
    }

    /// Runs `f` for every project, the parked ones brought on screen for it.
    fn each_project<R>(&mut self, mut f: impl FnMut(&mut Self, usize) -> R) -> Vec<R> {
        let mut out = Vec::new();
        for j in 0..self.project_count() {
            if j == self.proj {
                out.push(f(self, j));
            } else if let Some(r) = self.with_parked(j, |ed| f(ed, j)) {
                out.push(r);
            }
        }
        out
    }

    /// Writes the session when something changed since the last write (or
    /// always, with `force`). Nothing worth restoring (no file tab, no typed
    /// text, one project) removes it instead. Returns how many tabs were unsaved.
    pub(crate) fn save_session(&mut self, clean: bool, force: bool) -> io::Result<usize> {
        let base = sessions_dir().ok_or_else(|| io::Error::other("no home directory"))?;
        let dir = base.join(&self.session_id);
        let states = self.each_project(|ed, j| ed.project_state(j, clean));
        let now = now_secs();
        let had_live = dir.join("session.json").exists();
        let settled = had_live && now.saturating_sub(self.ckpt_since) >= STABLE_SECS;
        // Closing everything still saves when there's a state worth keeping
        // one step back.
        let worth = states.len() > 1
            || states
                .iter()
                .any(|(v, _)| v["tabs"].as_array().is_some_and(|t| !t.is_empty()))
            || settled
            || step_dir(&dir, 1).exists();
        if !worth {
            self.session_lock = None;
            if dir.exists() {
                fs::remove_dir_all(&dir)?;
            }
            self.autosave_sig = 0;
            self.ckpt_shape = 0;
            return Ok(0);
        }
        let projects: Vec<Value> = states.iter().map(|(v, _)| v.clone()).collect();
        let mut v = json!({
            "version": 1,
            "clean": clean,
            "project": self.proj,
            "projects": projects,
        });
        let mut sig = fnv(&v.to_string());
        for (name, _, rev) in states.iter().flat_map(|s| s.1.iter()) {
            sig = sig.wrapping_mul(31) ^ fnv(name) ^ rev;
        }
        let n_unsaved = states.iter().map(|s| s.1.len()).sum();
        if sig == self.autosave_sig && !force && dir.exists() {
            return Ok(n_unsaved);
        }
        fs::create_dir_all(&dir)?;
        // (Again if the folder was deleted under us.)
        if self.session_lock.is_none() || !dir.join("lock").exists() {
            let f = lock_file(&dir)?;
            let _ = f.try_lock();
            self.session_lock = Some(f);
        }
        // A new checkpoint when what's open changed after staying the same a
        // while, or when quitting throws unsaved text away.
        let shape = shape_of(&v);
        let changed = shape != self.ckpt_shape;
        if had_live && self.ckpt_shape != 0 && ((changed && settled) || (clean && self.ckpt_unsaved > 0)) {
            push_checkpoint(&dir)?;
        }
        if changed || !had_live {
            self.ckpt_since = now;
        }
        let ud = dir.join("unsaved");
        let _ = fs::remove_dir_all(&ud);
        if n_unsaved > 0 {
            fs::create_dir_all(&ud)?;
            let lists: Vec<Vec<(String, usize, u64)>> = states.into_iter().map(|s| s.1).collect();
            let written = self.each_project(|ed, j| -> io::Result<()> {
                for (name, i, _) in &lists[j] {
                    fs::write(ud.join(name), ed.bufs[*i].text())?;
                }
                Ok(())
            });
            written.into_iter().collect::<io::Result<()>>()?;
        }
        v["saved"] = json!(now);
        v["since"] = json!(self.ckpt_since);
        // Written aside and renamed, so a crash mid-write leaves the last one.
        let tmp = dir.join("session.json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&v).unwrap_or_default())?;
        fs::rename(&tmp, dir.join("session.json"))?;
        self.autosave_sig = sig;
        self.ckpt_shape = shape;
        self.ckpt_unsaved = n_unsaved;
        Ok(n_unsaved)
    }

    /// Saves the session every few seconds when something changed.
    pub fn autosave_tick(&mut self) {
        if self.autosave_at.elapsed() < AUTOSAVE_EVERY {
            return;
        }
        self.autosave_at = Instant::now();
        let _ = self.save_session(false, false);
    }

    /// Opens one saved tab. `only_unsaved` is used by `:recover`.
    fn restore_tab(&mut self, dir: &Path, t: &Value, only_unsaved: bool) -> bool {
        let unsaved = t["unsaved"]
            .as_str()
            .and_then(|f| fs::read_to_string(dir.join("unsaved").join(f)).ok());
        if only_unsaved && unsaved.is_none() {
            return false;
        }
        let mut b = match t["path"].as_str() {
            Some(p) => {
                let p = PathBuf::from(p);
                if !p.exists() && unsaved.is_none() {
                    return false;
                }
                if let Some(i) = self.find_buf(&p) {
                    if let Some(text) = &unsaved {
                        let b = &mut self.bufs[i];
                        b.checkpoint();
                        b.set_text(text.strip_suffix('\n').unwrap_or(text));
                        b.touch();
                    }
                    return true;
                }
                match Buffer::open(&p) {
                    Ok(b) => b,
                    Err(_) => return false,
                }
            }
            None => Buffer::scratch(NONAME),
        };
        if let Some(text) = unsaved {
            b.set_text(text.strip_suffix('\n').unwrap_or(&text));
            b.touch();
        }
        b.preview = t["preview"].as_bool().unwrap_or(false);
        let num = |k: &str| t[k].as_u64().unwrap_or(0) as usize;
        b.cy = num("cy").min(b.last());
        b.cx = num("cx");
        b.clamp(true);
        b.scroll_y = num("scroll_y").min(b.last());
        b.sync_want();
        if self.is_pristine(self.cur) && self.bufs.len() == 1 {
            self.bufs[0] = b;
        } else {
            self.bufs.push(b);
        }
        if t["current"].as_bool().unwrap_or(false) {
            self.cur = self.bufs.len() - 1;
        }
        true
    }

    /// Puts a saved project back on screen: tabs (unsaved text from `dir`),
    /// tree, panel, focus. Returns how many tabs had unsaved text.
    fn restore_project(&mut self, dir: &Path, v: &Value) -> usize {
        let mut n_unsaved = 0;
        for t in v["tabs"].as_array().cloned().unwrap_or_default() {
            if self.restore_tab(dir, &t, false) && t["unsaved"].is_string() {
                n_unsaved += 1;
            }
        }
        let tr = &v["tree"];
        self.tree.visible = tr["visible"].as_bool().unwrap_or(self.tree.visible);
        if let Some(w) = tr["width"].as_u64() {
            self.tree.width = w as u16;
        }
        let expanded: HashSet<PathBuf> = tr["expanded"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str().map(PathBuf::from))
            .collect();
        self.tree.set_expanded(&expanded);
        if let Some(p) = tr["sel"].as_str() {
            self.tree.reveal(Path::new(p));
        }
        let pn = &v["panel"];
        self.dbg.visible = pn["visible"].as_bool().unwrap_or(false);
        if let Some(h) = pn["height"].as_u64() {
            self.dbg.height = h as u16;
        }
        if let Some(t) = pn["tab"].as_str() {
            self.set_panel_tab_name(t);
        }
        self.focus = match v["focus"].as_str() {
            Some("tree") if self.tree.visible => Focus::Tree,
            Some("panel") if self.dbg.visible => Focus::Panel,
            _ => Focus::Editor,
        };
        n_unsaved
    }

    /// Nothing open yet: one project, the welcome screen, no terminal.
    fn untouched(&self) -> bool {
        self.project_count() == 1 && self.is_splash() && self.terms.is_empty()
    }

    /// Opens saved projects: `(unsaved folder, project state)` each, `here`
    /// the one that was on screen. When nothing is open yet they replace the
    /// empty start; otherwise they join the open projects (an open folder keeps
    /// its tabs). Returns (projects opened, unsaved tabs).
    fn restore_projects(&mut self, saved: Vec<(PathBuf, Value)>, here: usize) -> (usize, usize) {
        let replace = self.untouched();
        let start = self.proj;
        let start_key = path_key(&self.tree.root);
        let mut target = None;
        let (mut opened, mut n_unsaved) = (0, 0);
        let mut keep_start = !replace;
        for (k, (dir, v)) in saved.iter().enumerate() {
            let Some(root) = v["root"].as_str().map(PathBuf::from).filter(|r| r.is_dir()) else {
                continue;
            };
            let key = path_key(&root);
            let open = (0..self.project_count()).find(|&j| path_key(self.project_root(j)) == key);
            let j = match open {
                Some(j) if replace && j == start => {
                    // The empty start is this folder: it takes the saved tabs.
                    keep_start = true;
                    n_unsaved += self.restore_project(dir, v);
                    opened += 1;
                    j
                }
                Some(j) => j,
                None => {
                    let j = self.add_parked(root);
                    n_unsaved += self.with_parked(j, |ed| ed.restore_project(dir, v)).unwrap_or(0);
                    opened += 1;
                    j
                }
            };
            if k == here || target.is_none() {
                target = Some(j);
            }
        }
        if let Some(t) = target {
            self.switch_project(t);
        }
        if !keep_start && path_key(self.tree.root.as_path()) != start_key {
            if let Some(j) = (0..self.project_count()).find(|&j| path_key(self.project_root(j)) == start_key) {
                self.drop_parked(j);
            }
        }
        if replace {
            // The start may have been one of them; the saved order wins.
            let roots: Vec<PathBuf> = saved
                .iter()
                .filter_map(|(_, v)| v["root"].as_str().map(PathBuf::from))
                .collect();
            self.order_projects(&roots);
        }
        (opened, n_unsaved)
    }

    /// Restores a saved session (`step` 0, its latest state, or one of its
    /// checkpoints) and takes it over: from now on this ren saves into it.
    pub(crate) fn restore_session(&mut self, id: &str, step: usize) -> Result<String, String> {
        let base = sessions_dir().ok_or("No home directory")?;
        let dir = base.join(id);
        if id != self.session_id && is_live(&dir) {
            return Err("That session is open in another ren".into());
        }
        let from = step_dir(&dir, step);
        let v = read_json(&from.join("session.json")).ok_or("The session is gone")?;
        let saved: Vec<(PathBuf, Value)> = v["projects"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (from.clone(), p))
            .collect();
        let here = v["project"].as_u64().unwrap_or(0) as usize;
        let (opened, n_unsaved) = self.restore_projects(saved, here);
        let info = info_of(id, &v);
        let title = info.title();
        // Take it over: the restored text is saved into it right away, and
        // this ren's own (if it wrote one) goes.
        let own = std::mem::replace(&mut self.session_id, id.to_string());
        self.session_lock = None;
        if own != id {
            let _ = fs::remove_dir_all(base.join(&own));
        }
        if step > 0 {
            // Going back: the newer states stay as checkpoints.
            let _ = push_checkpoint(&dir);
            self.ckpt_shape = 0;
            self.ckpt_since = now_secs();
        } else {
            // Carrying on from where it was: if this ren had other projects
            // open, the saved state becomes a checkpoint.
            self.ckpt_shape = shape_of(&v);
            self.ckpt_since = v["since"].as_u64().or(v["saved"].as_u64()).unwrap_or(0);
            self.ckpt_unsaved = info.unsaved;
        }
        let _ = self.save_session(false, true);
        self.sessions.retain(|s| s.id != id);
        if opened == 0 {
            return Err(format!("Nothing to restore from {}: its folders are gone", title));
        }
        let when = if step > 0 {
            format!(" as of {}", ago(now_secs().saturating_sub(info.saved)))
        } else {
            String::new()
        };
        Ok(if n_unsaved > 0 {
            format!(
                "Restored {}{} · {} unsaved tabs (not written to disk yet)",
                title, when, n_unsaved
            )
        } else {
            format!("Restored {}{}", title, when)
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Started by `:reload`: the session it saved comes back.
    pub fn reloaded(&mut self, id: &str) {
        match self.restore_session(id, 0) {
            Ok(m) => self.info(m.replacen("Restored", "Reloaded", 1)),
            Err(e) => self.error(e),
        }
    }

    /// `:reload` from a ren that kept one session per folder: reopens what it
    /// saved, then moves it into this ren's session.
    pub(crate) fn restore_legacy(&mut self) {
        let Some(own_dir) = legacy_dir(&self.tree.root) else {
            return;
        };
        let list = read_json(&own_dir.join("projects.json"));
        let mut roots: Vec<PathBuf> = list
            .as_ref()
            .and_then(|v| v["projects"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r.as_str().map(PathBuf::from))
            .collect();
        let mut here = list.as_ref().and_then(|v| v["project"].as_u64()).unwrap_or(0) as usize;
        if roots.is_empty() {
            roots.push(self.tree.root.clone());
            here = 0;
        }
        let mut saved = Vec::new();
        for r in &roots {
            if let Some(d) = legacy_dir(r) {
                if let Some(v) = read_json(&d.join("session.json")) {
                    saved.push((d, v));
                }
            }
        }
        let (_, n_unsaved) = self.restore_projects(saved, here);
        let _ = self.save_session(false, true);
        for r in &roots {
            if let Some(d) = legacy_dir(r) {
                let _ = fs::remove_dir_all(d);
            }
        }
        self.info(if n_unsaved > 0 {
            format!("Reloaded · {} unsaved tabs kept", n_unsaved)
        } else {
            "Reloaded".to_string()
        });
    }

    /// At startup: points at the newest session that didn't close cleanly and
    /// left unsaved text.
    pub(crate) fn check_recovery(&mut self) {
        let Some(i) = self.sessions.iter().position(|s| !s.clean && s.unsaved > 0) else {
            return;
        };
        let s = &self.sessions[i];
        let where_ = if i < SHOWN { " (also on the welcome screen)" } else { "" };
        let msg = format!(
            "{} didn't close cleanly ({} unsaved tabs): :session {} restores it{}",
            s.title(),
            s.unsaved,
            i + 1,
            where_
        );
        self.error(msg);
    }

    /// The n-th listed session (1-based) comes back: its latest state, or with
    /// `back` > 0 that many checkpoints before it.
    pub(crate) fn open_session(&mut self, n: usize, back: usize) {
        let Some(s) = self.sessions.get(n.wrapping_sub(1)) else {
            return self.error(format!("No session {} ({} saved)", n, self.sessions.len()));
        };
        let step = if back == 0 {
            s.step
        } else {
            match s.earlier.get(back - 1) {
                Some(e) => e.step,
                None => {
                    let msg = match s.earlier.len() {
                        0 => format!("Session {} has no earlier checkpoints", n),
                        k => format!("Session {} has {} earlier checkpoints ({}.1 to {}.{})", n, k, n, n, k),
                    };
                    return self.error(msg);
                }
            }
        };
        let id = s.id.clone();
        match self.restore_session(&id, step) {
            Ok(m) => self.info(m),
            Err(e) => self.error(e),
        }
    }

    /// `ren --resume`: the session list, or with `n` (`2`, `2.1`) that
    /// session straight away.
    pub fn resume(&mut self, pick: Option<&str>) {
        match pick {
            None => self.session_menu(),
            Some(p) => match parse_pick(p) {
                Some((n, back)) => self.open_session(n, back),
                None => self.error(format!("ren --resume {}: expected a number like 2 or 2.1", p)),
            },
        }
    }

    /// `:session` lists saved sessions; `:session <n>` restores one
    /// (`<n>.<k>`: its checkpoint k steps back); `:session forget <n>`
    /// deletes it.
    pub(crate) fn session_command(&mut self, rest: &[String]) {
        match rest.first().map(String::as_str) {
            None | Some("list") => self.session_menu(),
            Some("forget" | "rm") => {
                let Some(n) = rest.get(1).and_then(|s| s.parse::<usize>().ok()) else {
                    return self.error("Usage: :session forget <n>");
                };
                let Some(s) = (n >= 1).then(|| n - 1).filter(|&i| i < self.sessions.len()) else {
                    return self.error(format!("No session {}", n));
                };
                let s = self.sessions.remove(s);
                if let Some(base) = sessions_dir() {
                    let _ = fs::remove_dir_all(base.join(&s.id));
                }
                self.info(format!("Forgot {}", s.title()));
            }
            Some(a) => match parse_pick(a) {
                Some((n, back)) => self.open_session(n, back),
                None => self.error("Usage: :session [<n> | <n>.<k> | forget <n>]"),
            },
        }
    }

    fn session_menu(&mut self) {
        if self.sessions.is_empty() {
            return self.info("No saved sessions");
        }
        // Each session, then its earlier checkpoints under it (`↳`).
        let num_w = self.sessions.len().to_string().len();
        let w = self
            .sessions
            .iter()
            .flat_map(|s| std::iter::once(s).chain(s.earlier.iter()))
            .map(|s| s.title().chars().count())
            .max()
            .unwrap_or(0)
            .min(40);
        let row = |lead: String, s: &SessionInfo, run: String| {
            let title = s.title();
            let pad = w.saturating_sub(title.chars().count());
            let (base, warn) = s.detail();
            let warn = if warn.is_empty() {
                warn
            } else {
                format!(" · ⚠ {}", warn)
            };
            (
                format!("{}  {}{}   {}{}", lead, title, " ".repeat(pad), base, warn),
                ide::MenuAction::Run(run),
            )
        };
        let mut items: Vec<(String, ide::MenuAction)> = Vec::new();
        for (i, s) in self.sessions.iter().enumerate() {
            let n = i + 1;
            items.push(row(format!("{:>num_w$}", n), s, format!("session {}", n)));
            for (k, e) in s.earlier.iter().enumerate() {
                let lead = format!("{:>num_w$}", "↳");
                items.push(row(lead, e, format!("session {}.{}", n, k + 1)));
            }
        }
        self.enter_edit();
        self.menu = Some(Menu {
            title: "Sessions · Enter restores · ↳ earlier checkpoint".into(),
            items,
            sel: 0,
            scroll: 0,
            anchor: None,
            dir: None,
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
    }

    /// `:recover`: restores the newest session that didn't close cleanly;
    /// `:recover discard` drops its unsaved copies.
    pub(crate) fn recover(&mut self, arg: &str) {
        let Some(i) = self.sessions.iter().position(|s| !s.clean && s.unsaved > 0) else {
            return self.info("Nothing to recover");
        };
        if arg == "discard" {
            let s = &mut self.sessions[i];
            if let Some(base) = sessions_dir() {
                let _ = fs::remove_dir_all(base.join(&s.id).join("unsaved"));
            }
            s.unsaved = 0;
            return self.info("Discarded the unsaved copies");
        }
        self.open_session(i + 1, 0);
    }

    /// On a clean quit: the session stays, marked clean and without unsaved
    /// copies (quitting discarded them).
    pub fn clean_exit(&mut self) {
        if self.reload {
            return;
        }
        let _ = self.save_session(true, true);
    }

    /// The terminal went away (hang-up, SIGTERM, a write error): everything,
    /// unsaved text included, is saved for next time.
    pub fn hangup_exit(&mut self) {
        let _ = self.save_session(false, true);
    }

    /// `:reload`: saves the session and asks main to restart ren.
    pub(crate) fn reload_app(&mut self) {
        match self.save_session(false, true) {
            Ok(_) => {
                self.reload = true;
                self.quit = true;
            }
            Err(e) => self.error(format!("Could not save the session: {}", e)),
        }
    }
}

/// `2` → (2, 0); `2.1` → (2, 1): a session's number, and how many
/// checkpoints back.
fn parse_pick(s: &str) -> Option<(usize, usize)> {
    let (n, back) = match s.split_once('.') {
        Some((n, k)) => (n.parse().ok()?, k.parse().ok()?),
        None => (s.parse().ok()?, 0),
    };
    (n >= 1).then_some((n, back))
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_titles_and_age() {
        let v = json!({
            "project": 1,
            "saved": now_secs() - 7200,
            "clean": false,
            "projects": [
                { "root": "/w/alpha", "tabs": [{ "path": "/w/alpha/a" }] },
                { "root": "/w/ren", "tabs": [{ "path": "/w/ren/x" }, { "name": "[untitled]", "unsaved": "1-1.txt" }] },
            ],
        });
        let s = info_of("id", &v);
        assert_eq!(s.title(), "ren + alpha");
        assert_eq!(s.tabs, 3);
        assert_eq!(s.unsaved, 1);
        assert_eq!(
            s.detail(),
            ("3 tabs · 2 h ago".to_string(), "interrupted · 1 unsaved".to_string())
        );
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(150), "2 min ago");
        assert_eq!(ago(3 * 86400), "3 d ago");
    }

    #[test]
    fn picks_and_shapes() {
        assert_eq!(parse_pick("2"), Some((2, 0)));
        assert_eq!(parse_pick("2.1"), Some((2, 1)));
        assert_eq!(parse_pick("0"), None);
        assert_eq!(parse_pick("x.1"), None);
        let a = json!({ "projects": [
            { "root": "/w/a", "tabs": [{ "path": "/w/a/1" }, { "path": "/w/a/2" }] },
            { "root": "/w/b", "tabs": [] },
        ]});
        // Order, cursors and preview tabs don't count.
        let b = json!({ "projects": [
            { "root": "/w/b", "tabs": [{ "path": "/w/b/p", "preview": true }] },
            { "root": "/w/a", "tabs": [{ "path": "/w/a/2", "cy": 9 }, { "path": "/w/a/1" }] },
        ]});
        assert_eq!(shape_of(&a), shape_of(&b));
        let c = json!({ "projects": [{ "root": "/w/a", "tabs": [{ "path": "/w/a/1" }] }] });
        assert_ne!(shape_of(&a), shape_of(&c));
    }

    #[test]
    fn checkpoints_rotate() {
        let dir = std::env::temp_dir().join(format!("ren-ckpt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |n: usize, unsaved: bool| {
            let tabs: Vec<Value> = (0..n).map(|i| json!({ "path": format!("/w/a/{}", i) })).collect();
            let v = json!({ "saved": n, "projects": [{ "root": "/w/a", "tabs": tabs }] });
            fs::write(dir.join("session.json"), v.to_string()).unwrap();
            let _ = fs::remove_dir_all(dir.join("unsaved"));
            if unsaved {
                fs::create_dir_all(dir.join("unsaved")).unwrap();
                fs::write(dir.join("unsaved").join("0-0.txt"), format!("text {}", n)).unwrap();
            }
        };
        fs::create_dir_all(&dir).unwrap();
        write(1, true);
        for n in 2..=4 {
            push_checkpoint(&dir).unwrap();
            write(n, false);
        }
        let s = read_session("id", &dir).unwrap();
        assert_eq!(s.tabs, 4);
        let back: Vec<(usize, usize)> = s.earlier.iter().map(|e| (e.step, e.tabs)).collect();
        assert_eq!(back, vec![(1, 3), (2, 2)]);
        // The first state (and its unsaved copy) was the oldest, so it went.
        assert!(!dir.join("3").exists());
        // A lost latest state still lists the session, from its checkpoints.
        fs::remove_file(dir.join("session.json")).unwrap();
        let s = read_session("id", &dir).unwrap();
        assert_eq!((s.step, s.tabs, s.earlier.len()), (1, 3, 1));
        let _ = fs::remove_dir_all(&dir);
    }
}
