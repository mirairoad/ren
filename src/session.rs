//! Session state outside the project, in `~/.ren/session/<workspace>/`.
//!
//! - `:reload` writes the session (tabs, cursors, tree, panel, and the text of
//!   unsaved tabs), restarts ren and restores it, so nothing is lost.
//! - While editing, unsaved tabs are copied there every few seconds. A clean
//!   quit deletes them; after a crash the next start offers `:recover`.

use super::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io;

const AUTOSAVE_EVERY: Duration = Duration::from_secs(3);

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// `~/.ren/session/<folder name>-<hash of the full path>`.
pub fn session_dir(root: &Path) -> Option<PathBuf> {
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
    fn session_json(&self, unsaved: &[(usize, String)]) -> Value {
        let tabs: Vec<Value> = self
            .bufs
            .iter()
            .enumerate()
            .filter(|(i, b)| b.path.is_some() || unsaved.iter().any(|u| u.0 == *i))
            .map(|(i, b)| {
                json!({
                    "path": b.path.as_ref().map(|p| p.display().to_string()),
                    "name": b.name,
                    "cx": b.cx,
                    "cy": b.cy,
                    "scroll_y": b.scroll_y,
                    "preview": b.preview,
                    "unsaved": unsaved.iter().find(|u| u.0 == i).map(|u| u.1.clone()),
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
        json!({
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
        })
    }

    /// Writes `session.json` and copies of all unsaved tabs. Returns how many were unsaved.
    pub(crate) fn write_session(&mut self) -> io::Result<usize> {
        let dir = session_dir(&self.tree.root).ok_or_else(|| io::Error::other("no home directory"))?;
        let ud = dir.join("unsaved");
        let _ = fs::remove_dir_all(&ud);
        let mut unsaved = Vec::new();
        for (i, b) in self.bufs.iter().enumerate() {
            let untitled_text = b.path.is_none() && b.name == NONAME && !self.is_pristine(i);
            if !(b.dirty && (b.path.is_some() || untitled_text)) {
                continue;
            }
            fs::create_dir_all(&ud)?;
            let f = format!("{}.txt", i);
            fs::write(ud.join(&f), b.text())?;
            unsaved.push((i, f));
        }
        fs::create_dir_all(&dir)?;
        let v = self.session_json(&unsaved);
        fs::write(
            dir.join("session.json"),
            serde_json::to_string_pretty(&v).unwrap_or_default(),
        )?;
        Ok(unsaved.len())
    }

    fn read_session(&self) -> Option<(PathBuf, Value)> {
        let dir = session_dir(&self.tree.root)?;
        let text = fs::read_to_string(dir.join("session.json")).ok()?;
        Some((dir, serde_json::from_str(&text).ok()?))
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

    /// After `:reload`: reopens tabs, unsaved text, tree and panel state.
    pub(crate) fn restore_session(&mut self) {
        let Some((dir, v)) = self.read_session() else { return };
        let mut n_unsaved = 0;
        for t in v["tabs"].as_array().cloned().unwrap_or_default() {
            if self.restore_tab(&dir, &t, false) && t["unsaved"].is_string() {
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
        let _ = fs::remove_dir_all(dir.join("unsaved"));
        self.info(if n_unsaved > 0 {
            format!("Reloaded · {} unsaved tabs kept", n_unsaved)
        } else {
            "Reloaded".to_string()
        });
    }

    /// At startup: points at `:recover` when a crashed session left unsaved text.
    pub(crate) fn check_recovery(&mut self) {
        let Some(dir) = session_dir(&self.tree.root) else {
            return;
        };
        let n = fs::read_dir(dir.join("unsaved")).map(|rd| rd.count()).unwrap_or(0);
        if n > 0 {
            self.error(format!(
                "{} unsaved tabs from a session that didn't close cleanly: :recover to restore, :recover discard to drop",
                n
            ));
        }
    }

    /// `:recover`: reopens the unsaved tabs a crashed session left behind.
    pub(crate) fn recover(&mut self, arg: &str) {
        let Some((dir, v)) = self.read_session() else {
            return self.info("Nothing to recover");
        };
        if arg == "discard" {
            let _ = fs::remove_dir_all(dir.join("unsaved"));
            return self.info("Discarded the unsaved copies");
        }
        let mut n = 0;
        for t in v["tabs"].as_array().cloned().unwrap_or_default() {
            if self.restore_tab(&dir, &t, true) {
                n += 1;
            }
        }
        let _ = fs::remove_dir_all(dir.join("unsaved"));
        self.autosave_sig = 0;
        self.info(format!("Recovered {} unsaved tabs (not written to disk yet)", n));
    }

    /// Copies unsaved tabs to the session folder every few seconds, for crash recovery.
    pub fn autosave_tick(&mut self) {
        if self.autosave_at.elapsed() < AUTOSAVE_EVERY {
            return;
        }
        self.autosave_at = Instant::now();
        let sig = self.bufs.iter().filter(|b| b.dirty).fold(1u64, |h, b| {
            h.wrapping_mul(31).wrapping_add(b.rev).wrapping_add(b.cy as u64)
        });
        let any = self.bufs.iter().any(|b| b.dirty);
        if sig == self.autosave_sig || (!any && self.autosave_sig == 0) {
            return;
        }
        self.autosave_sig = if any { sig } else { 0 };
        if any {
            let _ = self.write_session();
        } else if let Some(dir) = session_dir(&self.tree.root) {
            let _ = fs::remove_dir_all(dir.join("unsaved"));
        }
    }

    /// On a clean quit: the unsaved copies are no longer needed.
    pub fn clean_exit(&mut self) {
        if self.reload {
            return;
        }
        if let Some(dir) = session_dir(&self.tree.root) {
            let _ = fs::remove_dir_all(dir.join("unsaved"));
        }
    }

    /// `:reload`: saves the session and asks main to restart ren.
    pub(crate) fn reload_app(&mut self) {
        match self.write_session() {
            Ok(_) => {
                self.reload = true;
                self.quit = true;
            }
            Err(e) => self.error(format!("Could not save the session: {}", e)),
        }
    }
}
