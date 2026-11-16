use super::*;
use crate::buffer::{leading_ws, Hint};
use crate::lsp::{
    candidates, diag_message, expand_snippet, from_lsp_col, language_id, path_key, path_to_uri, to_lsp_col,
    uri_to_path, Diag, Enc, LspEvent, Pending,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const POPUP_ROWS: usize = 10;
pub const MENU_ROWS: usize = 14;

pub struct CItem {
    pub label: String,
    /// Shown dim right after the label (LSP `labelDetails.detail`), e.g.
    /// rust-analyzer's `(use crate::utils::deep::deep_fn)` for an auto-import.
    pub suffix: String,
    pub kind: u64,
    pub detail: String,
    filter: String,
    sort: String,
    text: String,
    snippet: bool,
    start: Option<usize>,
    end: Option<usize>,
    extra: Vec<Value>,
    raw: Value,
}

pub struct Popup {
    pub items: Vec<CItem>,
    pub shown: Vec<usize>,
    pub sel: usize,
    pub scroll: usize,
    pub y: usize,
    pub start: usize,
    pub lsp: bool,
    pub incomplete: bool,
}

#[derive(Clone)]
pub enum MenuAction {
    Code(&'static str, Value),
    Jump(PathBuf, usize, usize, Enc),
    Cmd(&'static str),
    /// Runs a `:` command line (empty = just close the menu).
    Run(String),
    Dir(PathBuf),
    OpenDir(PathBuf),
    /// Opens the folder as a project of its own (the project selector).
    OpenProject(PathBuf),
}

pub struct Menu {
    pub title: String,
    pub items: Vec<(String, MenuAction)>,
    pub sel: usize,
    pub scroll: usize,
    pub anchor: Option<(u16, u16)>,
    pub dir: Option<PathBuf>,
    pub filter: Option<String>,
    pub base: Vec<(String, MenuAction)>,
    pub ws_symbols: bool,
}

impl Menu {
    pub fn filterable(title: String, items: Vec<(String, MenuAction)>, sel: usize) -> Self {
        Menu {
            title,
            base: items.clone(),
            items,
            sel,
            scroll: 0,
            anchor: None,
            dir: None,
            filter: Some(String::new()),
            ws_symbols: false,
        }
    }

    fn apply_filter(&mut self) {
        let f = self.filter.clone().unwrap_or_default().to_lowercase();
        self.items = self
            .base
            .iter()
            .filter(|(t, _)| f.is_empty() || t.to_lowercase().contains(&f))
            .cloned()
            .collect();
        self.sel = 0;
        self.scroll = 0;
    }
}

fn fuzzy(pat: &str, s: &str) -> Option<u8> {
    if pat.is_empty() || s.starts_with(pat) {
        return Some(0);
    }
    if s.contains(pat) {
        return Some(1);
    }
    let mut it = s.chars();
    for p in pat.chars() {
        it.find(|c| *c == p)?;
    }
    Some(2)
}

fn markup_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(markup_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(o) => o.get("value").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// `Diag::server` of ren's own "not in the crate" warning.
pub(super) const UNLINKED: &str = "ren";

/// A Rust file's module name and the existing files that could declare it with
/// `mod name;`. None when it isn't inside a Cargo crate.
fn module_parents(path: &Path) -> Option<(String, Vec<PathBuf>)> {
    let crate_dir = path.ancestors().skip(1).find(|d| d.join("Cargo.toml").exists())?;
    let src = crate_dir.join("src");
    let (name, dir) = if path.file_name()? == "mod.rs" {
        let d = path.parent()?;
        (d.file_name()?.to_string_lossy().into_owned(), d.parent()?)
    } else {
        (path.file_stem()?.to_string_lossy().into_owned(), path.parent()?)
    };
    let cands = if dir == src {
        vec![src.join("lib.rs"), src.join("main.rs")]
    } else {
        vec![dir.with_extension("rs"), dir.join("mod.rs")]
    };
    Some((name, cands.into_iter().filter(|p| p.exists() && p != path).collect()))
}

/// Files Cargo builds as crates by themselves: never "not in the crate".
fn is_crate_root(path: &Path) -> bool {
    let Some(crate_dir) = path.ancestors().skip(1).find(|d| d.join("Cargo.toml").exists()) else {
        return false;
    };
    let Ok(rel) = path.strip_prefix(crate_dir) else {
        return false;
    };
    let parts: Vec<String> = rel.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    matches!(
        parts.as_slice(),
        ["build.rs"]
            | ["src", "main.rs"]
            | ["src", "lib.rs"]
            | ["src", "bin", ..]
            | ["examples" | "tests" | "benches", ..]
    )
}

fn is_rust_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_alphabetic() || c == '_') && cs.all(|c| c.is_alphanumeric() || c == '_')
}

/// Where a new `mod x;` goes: after the last top-level `mod` declaration, or
/// else at the top, after inner attributes and `//!` docs. True = add a blank
/// line after it (nothing to group with).
fn mod_insert_line(lines: &[Vec<char>]) -> (usize, bool) {
    let is_decl = |l: &Vec<char>| {
        let t: String = l.iter().collect();
        let t = t.trim_end();
        let rest = t.strip_prefix("pub(crate) ").or(t.strip_prefix("pub ")).unwrap_or(t);
        rest.starts_with("mod ") && rest.ends_with(';')
    };
    if let Some(last) = lines.iter().rposition(is_decl) {
        return (last + 1, false);
    }
    let top = lines
        .iter()
        .take_while(|l| {
            let t: String = l.iter().collect();
            t.starts_with("//!") || t.starts_with("#![")
        })
        .count();
    (top, true)
}

/// How long accepting a completion waits for its auto-import edits.
const RESOLVE_TIMEOUT: Duration = Duration::from_millis(500);

/// How long typing must pause before inlay hints are asked for again.
const HINT_DELAY: Duration = Duration::from_millis(250);

/// An inlay hint's label (a string or label parts) on one line, kept short.
fn hint_label(v: &Value) -> Option<String> {
    let raw: String = match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
        _ => return None,
    };
    // One line; runs of whitespace become one space, edge spaces included
    // (rust-analyzer sends " -> i32").
    let mut one = String::new();
    for c in raw.chars() {
        if c.is_whitespace() {
            if !one.ends_with(' ') {
                one.push(' ');
            }
        } else if !c.is_control() {
            one.push(c);
        }
    }
    if one.trim().is_empty() {
        return None;
    }
    const MAX: usize = 48;
    Some(if one.chars().count() > MAX {
        format!("{}…", one.chars().take(MAX - 1).collect::<String>())
    } else {
        one
    })
}

/// The inlay hint kinds in the `:hints` menu: settings key, label, example.
const HINT_ROWS: [(&str, &str, &str); 5] = [
    ("types", "Types", "let x: i32"),
    ("parameters", "Parameter names", "add(a: 1)"),
    ("returnTypes", "Return types", "|x| -> i32 { … }"),
    ("chaining", "Chain types", ".iter()  Iter<i32>"),
    ("closingBraces", "Closing braces", "}  fn main"),
];

/// A `:hints` argument to its settings key (short names allowed).
fn hint_key(k: &str) -> Option<&'static str> {
    Some(match k.to_lowercase().as_str() {
        "enabled" | "all" => "enabled",
        "types" | "type" => "types",
        "parameters" | "params" | "param" | "names" => "parameters",
        "returntypes" | "returns" | "return" => "returnTypes",
        "chaining" | "chain" | "chains" => "chaining",
        "closingbraces" | "braces" | "closing" => "closingBraces",
        _ => return None,
    })
}

/// Sorts a hint into one of the settings' kinds. LSP only tells types (1) from
/// parameter names (2), so return types, chain types and closing-brace labels
/// are told apart by their label and where they sit.
fn hint_kind(lsp_kind: Option<u64>, label: &str, line: &[char], x: usize) -> &'static str {
    let label = label.trim_start();
    let before = x.checked_sub(1).and_then(|i| line.get(i)).copied();
    let at_end = line.get(x..).is_none_or(|rest| rest.iter().all(|c| c.is_whitespace()));
    match lsp_kind {
        Some(2) => "parameters",
        _ if label.starts_with("->") => "returnTypes",
        // TypeScript: `function add(a, b): number`.
        Some(1) if before == Some(')') && label.starts_with(':') => "returnTypes",
        Some(1) if at_end && !label.starts_with(':') => "chaining",
        Some(1) => "types",
        _ if before == Some('}') => "closingBraces",
        _ => "types",
    }
}

/// A hover answer as plain lines: code fences dropped, blank runs squeezed.
pub(super) fn hover_lines(r: &Value) -> Vec<String> {
    let text = markup_text(&r["contents"]);
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        let l = l.trim_end();
        if l.starts_with("```") {
            continue;
        }
        // A markdown rule (`---`) separates sections: a blank line does that here.
        let rule = l.len() >= 3 && (l.chars().all(|c| c == '-') || l.chars().all(|c| c == '*'));
        let l = if rule { "" } else { l };
        if l.is_empty() && lines.last().is_none_or(|x| x.is_empty()) {
            continue;
        }
        lines.push(l.replace("&nbsp;", " "));
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    pub(crate) fn lsp_lang(&self, i: usize) -> Option<&'static str> {
        let b = self.bufs.get(i)?;
        if b.path.is_some() && !candidates(b.lang.name).is_empty() {
            Some(b.lang.name)
        } else {
            None
        }
    }

    /// Settings keys that configure a language's servers.
    fn lang_keys(lang: &str) -> Vec<String> {
        let mut v = vec![lang.to_lowercase()];
        match lang {
            "JS/TS" => v.extend(["typescript".into(), "javascript".into()]),
            "C/C++" => v.extend(["c".into(), "cpp".into()]),
            "Shell" => v.extend(["bash".into(), "sh".into()]),
            _ => {}
        }
        v
    }

    pub(super) fn lsp_config(&self, lang: &str) -> Option<Vec<Vec<String>>> {
        Self::lang_keys(lang)
            .iter()
            .find_map(|k| self.settings.lsp_servers.get(k))
            .cloned()
    }

    fn fmt_pref(&self, lang: &str) -> Option<String> {
        Self::lang_keys(lang)
            .iter()
            .find_map(|k| self.settings.lsp_format.get(k))
            .cloned()
    }

    /// The first ready server of the current file's language.
    fn cur_ready(&self) -> Option<&'static str> {
        let lang = self.lsp_lang(self.cur)?;
        self.lsp.ids(lang).into_iter().find(|id| self.lsp.ready(id))
    }

    /// The first ready server of the current file's language that has `cap`.
    fn cur_cap(&self, cap: &str) -> Option<&'static str> {
        let lang = self.lsp_lang(self.cur)?;
        self.lsp.pick(lang, cap, None)
    }

    pub(crate) fn lsp_sync(&mut self, i: usize) {
        let Some(lang) = self.lsp_lang(i) else { return };
        let b = &mut self.bufs[i];
        if !b.lsp_open || b.rev == b.lsp_rev {
            return;
        }
        b.lsp_version += 1;
        b.lsp_rev = b.rev;
        let params = json!({
            "textDocument": { "uri": path_to_uri(b.path.as_ref().unwrap()), "version": b.lsp_version },
            "contentChanges": [{ "text": b.text() }]
        });
        self.lsp.notify_lang(lang, "textDocument/didChange", params);
    }

    fn lsp_open_buf(&mut self, i: usize) {
        let Some(lang) = self.lsp_lang(i) else { return };
        if self.bufs[i].lsp_open {
            return;
        }
        if self.lsp.alive_lang(lang) {
            // A file from another project: the running server takes its folder too.
            let path = self.bufs[i].path.clone().unwrap();
            if let Some(root) = self.lsp.add_root(lang, &path, &self.tree.root) {
                self.info(format!("Language server: added project {}", root.display()));
            }
        } else {
            if self.lsp.failed.contains(lang) {
                return;
            }
            let path = self.bufs[i].path.clone().unwrap();
            let conf = self.lsp_config(lang);
            match self.lsp.start(lang, &path, &self.tree.root, conf.as_ref()) {
                Ok((names, missing)) => {
                    if missing.is_empty() {
                        self.info(format!("starting {}…", names.join(" + ")));
                    } else {
                        self.error(format!(
                            "starting {}… (not found: {})",
                            names.join(" + "),
                            missing.join(", ")
                        ));
                    }
                }
                Err(e) => {
                    if !e.is_empty() {
                        self.error(e);
                    }
                    return;
                }
            }
        }
        let b = &mut self.bufs[i];
        b.lsp_open = true;
        b.lsp_rev = b.rev;
        b.lsp_version = 1;
        let path = b.path.clone().unwrap();
        let params = json!({
            "textDocument": {
                "uri": path_to_uri(&path),
                "languageId": language_id(lang, &path),
                "version": 1,
                "text": b.text(),
            }
        });
        self.lsp.notify_lang(lang, "textDocument/didOpen", params);
    }

    pub(crate) fn lsp_close_buf(&mut self, i: usize) {
        let Some(lang) = self.lsp_lang(i) else { return };
        if !self.bufs[i].lsp_open {
            return;
        }
        let path = self.bufs[i].path.clone().unwrap();
        self.lsp.notify_lang(
            lang,
            "textDocument/didClose",
            json!({ "textDocument": { "uri": path_to_uri(&path) } }),
        );
        // The servers' diagnostics stay: rust-analyzer's `cargo check` results
        // still hold for a closed file (the tree shows them), and servers that
        // only check open files clear theirs on didClose. ren's own crate
        // warning goes, since it's only rechecked for open files.
        if let Some(ds) = self.lsp.diags.get_mut(&path_key(&path)) {
            ds.retain(|d| d.server != UNLINKED);
        }
        self.bufs[i].lsp_open = false;
    }

    pub(crate) fn lsp_did_save(&mut self) {
        let Some(lang) = self.lsp_lang(self.cur) else { return };
        if !self.buf().lsp_open {
            return;
        }
        self.lsp_sync(self.cur);
        let uri = path_to_uri(self.buf().path.as_ref().unwrap());
        self.lsp
            .notify_lang(lang, "textDocument/didSave", json!({ "textDocument": { "uri": uri } }));
    }

    pub fn lsp_tick(&mut self) -> bool {
        let before = self.lsp_status();
        let had_msg = self.msg.is_some();
        for i in 0..self.bufs.len() {
            if self.bufs[i].path.is_some() && !self.bufs[i].lsp_open {
                self.lsp_open_buf(i);
            }
            self.lsp_sync(i);
        }
        let mut changed = self.inlay_tick();
        self.pull_tick();
        changed |= self.peek_tick();
        changed |= self.scan_tick();
        self.crate_tick();
        let events = self.lsp.poll();
        changed |= !events.is_empty();
        for ev in events {
            self.on_lsp_event(ev);
        }
        if self.lsp_status() != before || self.msg.is_some() != had_msg {
            changed = true;
        }
        changed
    }

    /// Asks for the open file's inlay hints once typing pauses; drops them all
    /// when they're switched off. True when something on screen changed.
    fn inlay_tick(&mut self) -> bool {
        if !self.settings.inlay.enabled {
            let mut changed = false;
            for b in self.bufs.iter_mut().filter(|b| !b.hints.is_empty()) {
                b.hints.clear();
                b.hint_rev = None;
                changed = true;
            }
            return changed;
        }
        let i = self.cur;
        let Some(lang) = self.lsp_lang(i) else { return false };
        let b = &self.bufs[i];
        if !b.lsp_open || b.hint_rev == Some(b.rev) || b.edited_at.elapsed() < HINT_DELAY {
            return false;
        }
        if b.hint_req.is_some_and(|(id, req)| self.lsp.is_pending(id, req)) {
            return false;
        }
        let Some(id) = self.lsp.pick(lang, "inlayHintProvider", None) else {
            return false;
        };
        let enc = self.lsp.enc(id);
        let last = b.last();
        let path = b.path.clone().unwrap();
        let p = json!({
            "textDocument": { "uri": path_to_uri(&path) },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": last, "character": to_lsp_col(&b.lines[last], b.lines[last].len(), enc) }
            }
        });
        let rev = b.rev;
        let req = self
            .lsp
            .request(id, "textDocument/inlayHint", p, Pending::InlayHints(path, rev));
        let b = &mut self.bufs[i];
        b.hint_rev = Some(rev);
        b.hint_req = req.map(|r| (id, r));
        false
    }

    /// Asks for the open file's diagnostics once typing pauses, from a server
    /// that only gives them when asked (csharp-ls). Servers that send them by
    /// themselves aren't asked.
    fn pull_tick(&mut self) {
        let i = self.cur;
        let Some(lang) = self.lsp_lang(i) else { return };
        let b = &self.bufs[i];
        if !b.lsp_open || b.pull_rev == Some(b.rev) || b.edited_at.elapsed() < HINT_DELAY {
            return;
        }
        if b.pull_req.is_some_and(|(id, req)| self.lsp.is_pending(id, req)) {
            return;
        }
        let Some(id) = self.lsp.pick_pull(lang) else { return };
        let path = b.path.clone().unwrap();
        let rev = b.rev;
        let p = json!({ "textDocument": { "uri": path_to_uri(&path) } });
        let req = self
            .lsp
            .request(id, "textDocument/diagnostic", p, Pending::PullDiags(path, rev));
        let b = &mut self.bufs[i];
        b.pull_rev = Some(rev);
        b.pull_req = req.map(|r| (id, r));
    }

    /// A pull-diagnostics answer: a full report replaces the file's
    /// diagnostics from that server, as `publishDiagnostics` would.
    fn on_pull_diags(&mut self, id: &'static str, path: &Path, rev: u64, result: Value) {
        let current = self
            .bufs
            .iter()
            .any(|b| b.path.as_deref() == Some(path) && b.rev == rev);
        if !current || result["kind"].as_str() != Some("full") {
            return;
        }
        let params = json!({ "uri": path_to_uri(path), "diagnostics": result["items"].clone() });
        self.on_diagnostics(id, &params);
    }

    /// Identifies what decides whether a Rust file is in a crate: every open
    /// file of the language and its edits, and the server's finished work.
    fn crate_key(&self, lang: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.lsp_idle_gen.hash(&mut h);
        for b in self.bufs.iter().filter(|b| b.lang.name == lang) {
            (&b.path, b.rev).hash(&mut h);
        }
        h.finish()
    }

    /// Rust: asks rust-analyzer whether the open file belongs to a crate. A file
    /// no `mod` reaches gets no completion, hints or errors, and the server
    /// doesn't say so, so ren warns (`on_parent_module`). Asked once the server
    /// is idle and typing has paused, again whenever a Rust file changes.
    fn crate_tick(&mut self) {
        let i = self.cur;
        let Some(lang) = self.lsp_lang(i).filter(|l| *l == "Rust") else {
            return;
        };
        if self.bufs[i].path.as_deref().is_some_and(is_crate_root) {
            return;
        }
        let Some(id) = self.lsp.pick_experimental(lang, "parentModule") else {
            return;
        };
        if !self.lsp.idle(id) {
            return;
        }
        let key = self.crate_key(lang);
        let b = &self.bufs[i];
        if !b.lsp_open || b.crate_key == Some(key) || b.edited_at.elapsed() < Duration::from_millis(400) {
            return;
        }
        if b.crate_req.is_some_and(|(sid, r)| self.lsp.is_pending(sid, r)) {
            return;
        }
        let path = b.path.clone().unwrap();
        let p = json!({
            "textDocument": { "uri": path_to_uri(&path) },
            "position": { "line": 0, "character": 0 }
        });
        let req = self
            .lsp
            .request(id, "experimental/parentModule", p, Pending::ParentModule(path));
        let b = &mut self.bufs[i];
        b.crate_key = Some(key);
        b.crate_req = req.map(|r| (id, r));
    }

    /// rust-analyzer answers null when the file is in no crate (a crate root
    /// gets its Cargo.toml, a module its `mod` line).
    fn on_parent_module(&mut self, path: &Path, r: Value) {
        let unlinked = r.is_null() || r.as_array().is_some_and(|a| a.is_empty());
        let key = path_key(path);
        let Some(i) = self
            .bufs
            .iter()
            .position(|b| b.path.as_deref().is_some_and(|p| path_key(p) == key))
        else {
            return;
        };
        let was = self.bufs[i].unlinked;
        self.bufs[i].unlinked = unlinked;
        // Linked after all (e.g. `mod` just added): drop the warning from the bar too.
        if was && !unlinked && self.msg.as_ref().is_some_and(|m| m.0 == self.unlinked_msg(path)) {
            self.msg = None;
        }
        let msg = unlinked.then(|| self.unlinked_msg(path));
        let ds = self.lsp.diags.entry(key).or_default();
        ds.retain(|d| d.server != UNLINKED);
        if let Some(msg) = msg {
            ds.push(Diag {
                server: UNLINKED,
                l0: 0,
                c0: 0,
                l1: 0,
                c1: 0,
                sev: 2,
                msg: msg.clone(),
                raw: Value::Null,
                enc: Enc::Utf32,
            });
            if !was && i == self.cur {
                self.error(msg);
            }
        }
    }

    fn unlinked_msg(&self, path: &Path) -> String {
        match module_parents(path) {
            Some((name, parents)) if !parents.is_empty() => format!(
                "Not in the crate: add `mod {};` to {} (Ctrl+. or :mod). rust-analyzer ignores this file until then",
                name,
                self.rel(&parents[0])
            ),
            Some((name, _)) => format!(
                "Not in the crate: declare it with `mod {};` in its parent module. rust-analyzer ignores this file until then",
                name
            ),
            None => "Not in any Cargo crate: rust-analyzer ignores this file".into(),
        }
    }

    /// Fixes ren offers itself in the code-action menu.
    pub(super) fn local_actions(&self) -> Vec<(String, MenuAction)> {
        let b = self.buf();
        let Some(path) = b.path.as_deref().filter(|_| b.unlinked) else {
            return Vec::new();
        };
        let Some((name, parents)) = module_parents(path) else {
            return Vec::new();
        };
        parents
            .iter()
            .map(|p| {
                let arg = match p.strip_prefix(&self.tree.root) {
                    Ok(r) => format!(" \"/{}\"", r.display()),
                    Err(_) => String::new(),
                };
                (
                    format!("⚡ Insert `mod {};` in {}", name, self.rel(p)),
                    MenuAction::Run(format!("mod{}", arg)),
                )
            })
            .collect()
    }

    /// `:mod [parent]`: declares the open Rust file in its parent module with
    /// `mod name;` so rust-analyzer takes it in. The parent is edited in a tab of
    /// its own, not saved.
    pub(crate) fn link_module(&mut self, parent: Option<&str>) {
        let Some(path) = self.buf().path.clone().filter(|_| self.buf().lang.name == "Rust") else {
            return self.error("Not a Rust file");
        };
        let Some((name, parents)) = module_parents(&path) else {
            return self.error("Not inside a Cargo crate (no Cargo.toml above it)");
        };
        if !is_rust_ident(&name) {
            return self.error(format!(
                "\"{}\" can't be a module name: rename the file (letters, digits and _)",
                name
            ));
        }
        let target = match parent {
            Some(p) => self.resolve_path(p),
            None => match parents.first() {
                Some(p) => p.clone(),
                None => return self.error("No parent module file to declare it in (src/x.rs or src/x/mod.rs)"),
            },
        };
        let Some(i) = self.buf_index_for(&target) else {
            return self.error(format!("Can't open {}", self.rel(&target)));
        };
        let decl = format!("mod {};", name);
        let b = &mut self.bufs[i];
        let declared = b.lines.iter().any(|l| {
            let t: String = l.iter().collect();
            let t = t.trim();
            t == decl || (t.starts_with("pub") && t.ends_with(&format!(" {}", decl)))
        });
        if declared {
            let rel = self.rel(&target);
            return self.info(format!("{} already has `{}`", rel, decl));
        }
        let (at, gap) = mod_insert_line(&b.lines);
        b.checkpoint();
        if at < b.lines.len() {
            let mut text: Vec<char> = decl.chars().collect();
            text.push('\n');
            if gap && !b.lines[at].iter().all(|c| c.is_whitespace()) {
                text.push('\n');
            }
            b.insert_text((0, at), &text);
        } else {
            let y = b.last();
            let x = b.line_len(y);
            let mut text = vec!['\n'];
            text.extend(decl.chars());
            b.insert_text((x, y), &text);
        }
        let rel = self.rel(&target);
        self.info(format!("Added `{}` to {} (not saved yet)", decl, rel));
    }

    fn on_inlay_hints(&mut self, lang: &'static str, path: &Path, rev: u64, r: Value) {
        let enc = self.lsp.enc(lang);
        let key = path_key(path);
        let Some(b) = self
            .bufs
            .iter_mut()
            .find(|b| b.path.as_deref().is_some_and(|p| path_key(p) == key))
        else {
            return;
        };
        // An answer for older text: the next pause asks again.
        let shown = self.settings.inlay;
        if b.rev != rev || !shown.enabled {
            return;
        }
        let mut hints: Vec<Hint> = r
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| {
                let y = h["position"]["line"].as_u64()? as usize;
                let line = b.lines.get(y)?;
                let x = from_lsp_col(line, h["position"]["character"].as_u64()? as usize, enc);
                let label = hint_label(&h["label"])?;
                let kind = hint_kind(h["kind"].as_u64(), &label, line, x);
                if shown.get(kind) != Some(true) {
                    return None;
                }
                let pad = |k: &str| if h[k].as_bool() == Some(true) { " " } else { "" };
                Some(Hint {
                    x,
                    y,
                    text: format!("{}{}{}", pad("paddingLeft"), label, pad("paddingRight")),
                    before: h["kind"].as_u64() == Some(2),
                    kind,
                })
            })
            .collect();
        hints.sort_by_key(|h| (h.y, h.x));
        b.hints = hints;
    }

    pub fn lsp_status(&self) -> Option<String> {
        let lang = self.lsp_lang(self.cur)?;
        let ids = self.lsp.ids(lang);
        if ids.is_empty() {
            return None;
        }
        for id in &ids {
            let s = &self.lsp.servers[id];
            if let Some((t, m)) = s.progress.values().next() {
                let txt = format!("{}: {} {}", s.name, t, m);
                return Some(txt.trim().chars().take(48).collect());
            }
        }
        let names: Vec<String> = ids
            .iter()
            .map(|id| {
                let s = &self.lsp.servers[id];
                if s.ready {
                    s.name.clone()
                } else {
                    format!("{}…", s.name)
                }
            })
            .collect();
        Some(names.join(" + "))
    }

    fn on_lsp_event(&mut self, ev: LspEvent) {
        match ev {
            LspEvent::Ready(name) => self.info(format!("{} ready", name)),
            LspEvent::Exited { lang, id, name } => {
                let others = self.lsp.alive_lang(lang);
                for b in self.bufs.iter_mut() {
                    if b.lang.name == lang && !others {
                        b.lsp_open = false;
                    }
                }
                for ds in self.lsp.diags.values_mut() {
                    ds.retain(|d| d.server != id);
                }
                self.error(format!("{} exited (restart with :lsp restart)", name));
            }
            LspEvent::Notification { lang, method, params } => match method.as_str() {
                "textDocument/publishDiagnostics" => self.on_diagnostics(lang, &params),
                // The server's hints changed (e.g. it finished indexing): ask again.
                // Also sent by ren itself when a server goes idle.
                "workspace/inlayHint/refresh" => {
                    self.lsp_idle_gen += 1;
                    for b in self.bufs.iter_mut() {
                        b.hint_rev = None;
                        // Diagnostics that depend on other files may have changed too.
                        b.pull_rev = None;
                    }
                }
                "workspace/diagnostic/refresh" => {
                    for b in self.bufs.iter_mut() {
                        b.pull_rev = None;
                    }
                }
                "window/showMessage" => {
                    let m = params["message"]
                        .as_str()
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if params["type"].as_u64().unwrap_or(4) <= 1 {
                        self.error(m);
                    } else {
                        self.info(m);
                    }
                }
                _ => {}
            },
            LspEvent::ApplyEdit { lang, id, params } => {
                self.apply_workspace_edit(lang, &params["edit"]);
                self.lsp.reply(lang, id, json!({ "applied": true }));
            }
            LspEvent::Response {
                lang,
                id,
                kind,
                result,
                error,
            } => {
                if let Some(e) = error {
                    if !matches!(
                        kind,
                        Pending::Completion
                            | Pending::Signature
                            | Pending::Hover
                            | Pending::InlayHints(..)
                            | Pending::ParentModule(..)
                            | Pending::PeekHover(..)
                            | Pending::PeekActions(..)
                            | Pending::PullDiags(..)
                    ) {
                        self.error(format!("LSP: {}", e));
                    }
                    if let Pending::PeekHover(seq) | Pending::PeekActions(seq) = kind {
                        self.peek_answered(seq);
                    }
                    if matches!(kind, Pending::Completion) && Some(id) == self.comp_id {
                        self.comp_id = None;
                    }
                    return;
                }
                match kind {
                    Pending::Completion => {
                        if Some(id) == self.comp_id {
                            self.comp_id = None;
                            self.on_completion(lang, result);
                        }
                    }
                    Pending::CompResolve(buf) => self.on_comp_resolve(lang, buf, result),
                    Pending::Hover => self.on_hover(result),
                    Pending::DocSymbols => self.on_doc_symbols(lang, result),
                    Pending::WsSymbols => self.on_ws_symbols(lang, id, result),
                    Pending::Definition => self.on_definition(lang, result),
                    Pending::References => self.on_references(lang, result),
                    Pending::CodeAction => self.on_code_actions(lang, result),
                    Pending::Resolve => self.run_action(lang, result, false),
                    Pending::Signature => self.on_signature(result),
                    Pending::Format(path) => self.on_format(lang, &path, result),
                    Pending::Rename => {
                        if result.is_null() {
                            self.error("Rename failed");
                        } else {
                            let n = self.apply_workspace_edit(lang, &result);
                            self.info(format!("Renamed in {} files (:wa to save)", n));
                        }
                    }
                    Pending::InlayHints(path, rev) => self.on_inlay_hints(lang, &path, rev, result),
                    Pending::ParentModule(path) => self.on_parent_module(&path, result),
                    Pending::PeekHover(seq) => self.on_peek_hover(seq, result),
                    Pending::PeekActions(seq) => self.on_peek_actions(lang, seq, result),
                    Pending::PullDiags(path, rev) => self.on_pull_diags(lang, &path, rev, result),
                    Pending::Execute | Pending::Init => {}
                }
            }
        }
    }

    fn on_diagnostics(&mut self, lang: &'static str, params: &Value) {
        let Some(path) = params["uri"].as_str().and_then(uri_to_path) else {
            return;
        };
        let enc = self.lsp.enc(lang);
        let server = self.lsp.servers.get(lang).map(|s| s.name.clone()).unwrap_or_default();
        let id = lang;
        let num = |v: &Value| v.as_u64().unwrap_or(0) as usize;
        let ds: Vec<Diag> = params["diagnostics"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|d| {
                        let r = &d["range"];
                        let msg = diag_message(d, &server);
                        Diag {
                            server: id,
                            l0: num(&r["start"]["line"]),
                            c0: num(&r["start"]["character"]),
                            l1: num(&r["end"]["line"]),
                            c1: num(&r["end"]["character"]),
                            sev: d["severity"].as_u64().unwrap_or(1) as u8,
                            msg,
                            raw: d.clone(),
                            enc,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let all = self.lsp.diags.entry(path_key(&path)).or_default();
        all.retain(|d| d.server != id);
        all.extend(ds);
    }

    pub fn buf_diags(&self, i: usize) -> Option<&Vec<Diag>> {
        let p = self.bufs.get(i)?.path.as_ref()?;
        self.lsp.diags.get(&path_key(p)).filter(|d| !d.is_empty())
    }

    /// Files and folders with problems, by `path_key`: 1 when there's an error,
    /// 2 when only warnings. Folders take the worst of what's inside them, so a
    /// collapsed folder still shows that something in it is broken.
    /// Includes what the project scan found in files no server watches.
    pub(crate) fn problem_marks(&self) -> HashMap<String, u8> {
        problem_marks(&self.lsp.diags, self.scan_found())
    }

    pub fn diag_at_cursor(&self) -> Option<(u8, String)> {
        let b = self.buf();
        let ds = self.buf_diags(self.cur)?;
        ds.iter()
            .filter(|d| d.l0 <= b.cy && b.cy <= d.l1)
            .min_by_key(|d| d.sev)
            .map(|d| (d.sev, d.msg.lines().next().unwrap_or("").to_string()))
    }

    pub fn diag_counts(&self) -> (usize, usize) {
        self.buf_diags(self.cur).map_or((0, 0), |ds| {
            (
                ds.iter().filter(|d| d.sev == 1).count(),
                ds.iter().filter(|d| d.sev == 2).count(),
            )
        })
    }

    pub(crate) fn goto_diag(&mut self, fwd: bool) {
        let b = self.buf();
        let Some(ds) = self.buf_diags(self.cur) else {
            return self.info("No diagnostics");
        };
        let mut pos: Vec<(usize, usize, String)> = ds
            .iter()
            .map(|d| {
                let l = d.l0.min(b.last());
                (
                    l,
                    from_lsp_col(&b.lines[l], d.c0, d.enc),
                    d.msg.lines().next().unwrap_or("").to_string(),
                )
            })
            .collect();
        pos.sort_by_key(|a| (a.0, a.1));
        let cur = (b.cy, b.cx);
        let pick = if fwd {
            pos.iter().find(|p| (p.0, p.1) > cur).or(pos.first())
        } else {
            pos.iter().rev().find(|p| (p.0, p.1) < cur).or(pos.last())
        };
        if let Some((y, x, m)) = pick.cloned() {
            let b = self.buf_mut();
            b.cy = y;
            b.cx = x;
            b.sync_want();
            self.info(m);
        }
    }

    pub(crate) fn diag_menu(&mut self) {
        let b = self.buf();
        let Some(ds) = self.buf_diags(self.cur) else {
            return self.info("No diagnostics");
        };
        let path = b.path.clone().unwrap();
        let mut items: Vec<(u8, usize, String, MenuAction)> = ds
            .iter()
            .map(|d| {
                let sev = match d.sev {
                    1 => "error",
                    2 => "warn",
                    3 => "info",
                    _ => "hint",
                };
                let label = format!("{:>4}  {:<5} {}", d.l0 + 1, sev, d.msg.lines().next().unwrap_or(""));
                (d.sev, d.l0, label, MenuAction::Jump(path.clone(), d.l0, d.c0, d.enc))
            })
            .collect();
        items.sort_by_key(|i| (i.0, i.1));
        let n = items.len();
        self.menu = Some(Menu {
            title: format!("Diagnostics ({})", n),
            items: items.into_iter().map(|i| (i.2, i.3)).collect(),
            sel: 0,
            scroll: 0,
            anchor: None,
            dir: None,
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
    }

    fn tdpp(&self, lang: &str) -> Value {
        let b = self.buf();
        let enc = self.lsp.enc(lang);
        json!({
            "textDocument": { "uri": path_to_uri(b.path.as_ref().unwrap()) },
            "position": { "line": b.cy, "character": to_lsp_col(&b.lines[b.cy], b.cx, enc) }
        })
    }

    /// The server that answers a request needing `cap`: the first ready one that
    /// supports it (for formatting, the `lsp.format` server wins).
    fn ready_lang(&mut self, cap: &str) -> Option<&'static str> {
        let Some(lang) = self.lsp_lang(self.cur) else {
            self.error("No language server for this file");
            return None;
        };
        if !self.lsp.any_ready(lang) {
            if self.lsp.alive_lang(lang) {
                self.info("Language server is still starting…");
            } else {
                self.error("Language server is not running (:lsp)");
            }
            return None;
        }
        let prefer = if cap == "documentFormattingProvider" {
            self.fmt_pref(lang)
        } else {
            None
        };
        let Some(id) = self.lsp.pick(lang, cap, prefer.as_deref()) else {
            self.error("Server does not support this");
            return None;
        };
        self.lsp_sync(self.cur);
        Some(id)
    }

    pub(crate) fn lsp_hover(&mut self) {
        let Some(lang) = self.ready_lang("hoverProvider") else {
            return;
        };
        let p = self.tdpp(lang);
        self.lsp.request(lang, "textDocument/hover", p, Pending::Hover);
    }

    pub(crate) fn lsp_definition(&mut self) {
        let Some(lang) = self.ready_lang("definitionProvider") else {
            return;
        };
        let p = self.tdpp(lang);
        self.lsp
            .request(lang, "textDocument/definition", p, Pending::Definition);
    }

    pub(crate) fn lsp_references(&mut self) {
        let Some(lang) = self.ready_lang("referencesProvider") else {
            return;
        };
        let mut p = self.tdpp(lang);
        p["context"] = json!({ "includeDeclaration": true });
        self.lsp
            .request(lang, "textDocument/references", p, Pending::References);
    }

    pub(crate) fn lsp_format(&mut self) {
        let Some(lang) = self.ready_lang("documentFormattingProvider") else {
            return;
        };
        let b = self.buf();
        let path = b.path.clone().unwrap();
        let p = json!({
            "textDocument": { "uri": path_to_uri(&path) },
            "options": { "tabSize": TAB_W, "insertSpaces": !b.use_tabs }
        });
        self.lsp
            .request(lang, "textDocument/formatting", p, Pending::Format(path));
    }

    pub(crate) fn format_before_save(&mut self, i: usize) -> FormatOutcome {
        if !self.settings.format_on_save {
            return FormatOutcome::Skipped;
        }
        let Some(lang_name) = self.lsp_lang(i) else {
            return FormatOutcome::Skipped;
        };
        let prefer = self.fmt_pref(lang_name);
        let Some(lang) = self
            .lsp
            .pick(lang_name, "documentFormattingProvider", prefer.as_deref())
        else {
            return FormatOutcome::Skipped;
        };
        self.lsp_sync(i);
        let b = &self.bufs[i];
        let Some(path) = b.path.clone() else {
            return FormatOutcome::Skipped;
        };
        let rev = b.rev;
        let p = json!({
            "textDocument": { "uri": path_to_uri(&path) },
            "options": { "tabSize": TAB_W, "insertSpaces": !b.use_tabs }
        });
        let Some(id) = self
            .lsp
            .request(lang, "textDocument/formatting", p, Pending::Format(path))
        else {
            return FormatOutcome::Skipped;
        };
        let deadline = Instant::now() + FORMAT_TIMEOUT;
        while self.lsp.is_pending(lang, id) {
            if Instant::now() >= deadline {
                self.lsp.cancel(lang, id);
                return FormatOutcome::TimedOut;
            }
            for ev in self.lsp.poll() {
                self.on_lsp_event(ev);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if self.bufs.get(i).is_some_and(|b| b.rev != rev) {
            FormatOutcome::Formatted
        } else {
            FormatOutcome::Unchanged
        }
    }

    pub(crate) fn lsp_rename(&mut self, new_name: &str) {
        let Some(lang) = self.ready_lang("renameProvider") else {
            return;
        };
        let mut p = self.tdpp(lang);
        p["newName"] = json!(new_name);
        self.lsp.request(lang, "textDocument/rename", p, Pending::Rename);
    }

    pub(crate) fn word_under_cursor(&self) -> String {
        let b = self.buf();
        let line = &b.lines[b.cy];
        if b.cx >= line.len() || !is_ident(line[b.cx]) {
            return String::new();
        }
        let mut s = b.cx;
        while s > 0 && is_ident(line[s - 1]) {
            s -= 1;
        }
        let mut e = b.cx;
        while e < line.len() && is_ident(line[e]) {
            e += 1;
        }
        line[s..e].iter().collect()
    }

    pub(crate) fn start_rename(&mut self) {
        let w = self.word_under_cursor();
        self.prefill_cmd(&format!("rename {}", w));
    }

    pub(crate) fn lsp_code_actions(&mut self) {
        // ren's own fixes come first; the servers' answers are added below them.
        let local = self.local_actions();
        self.ca_menu_open = !local.is_empty();
        if !local.is_empty() {
            self.menu = Some(Menu {
                title: "Code actions".into(),
                items: local,
                sel: 0,
                scroll: 0,
                anchor: self.menu_anchor.take(),
                dir: None,
                filter: None,
                base: Vec::new(),
                ws_symbols: false,
            });
        }
        let Some(first) = self.ready_lang("codeActionProvider") else {
            return;
        };
        let lang_name = self.lsp.lang_of(first).unwrap_or("");
        let targets: Vec<&'static str> = self
            .lsp
            .ids(lang_name)
            .into_iter()
            .filter(|id| self.lsp.ready(id) && self.lsp.cap(id, "codeActionProvider"))
            .collect();
        self.ca_pending = targets.len();
        for lang in targets {
            self.code_actions_from(lang);
        }
    }

    fn code_actions_from(&mut self, lang: &'static str) {
        let enc = self.lsp.enc(lang);
        let (a, e) = self
            .sel_range()
            .unwrap_or(((self.buf().cx, self.buf().cy), (self.buf().cx, self.buf().cy)));
        let b = self.buf();
        let range = json!({
            "start": { "line": a.1, "character": to_lsp_col(&b.lines[a.1], a.0, enc) },
            "end": { "line": e.1, "character": to_lsp_col(&b.lines[e.1], e.0, enc) },
        });
        let diags: Vec<Value> = self
            .buf_diags(self.cur)
            .map(|ds| {
                ds.iter()
                    .filter(|d| d.server == lang && d.l0 <= e.1 && a.1 <= d.l1)
                    .map(|d| d.raw.clone())
                    .collect()
            })
            .unwrap_or_default();
        let p = json!({
            "textDocument": { "uri": path_to_uri(b.path.as_ref().unwrap()) },
            "range": range,
            "context": { "diagnostics": diags, "triggerKind": 1 }
        });
        self.lsp
            .request(lang, "textDocument/codeAction", p, Pending::CodeAction);
    }

    fn on_hover(&mut self, r: Value) {
        let mut lines = hover_lines(&r);
        if lines.is_empty() {
            self.info("No information");
        } else {
            lines.truncate(40);
            self.hover = Some(lines);
        }
    }

    fn on_definition(&mut self, lang: &'static str, r: Value) {
        let loc = if r.is_array() { r[0].clone() } else { r };
        if loc.is_null() {
            return self.info("Definition not found");
        }
        let (uri, range) = if loc["targetUri"].is_string() {
            (&loc["targetUri"], &loc["targetSelectionRange"])
        } else {
            (&loc["uri"], &loc["range"])
        };
        let Some(path) = uri.as_str().and_then(uri_to_path) else {
            return;
        };
        let line = range["start"]["line"].as_u64().unwrap_or(0) as usize;
        let col = range["start"]["character"].as_u64().unwrap_or(0) as usize;
        let enc = self.lsp.enc(lang);
        self.jump_to(&path, line, col, enc);
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.tree.root).unwrap_or(p).display().to_string()
    }

    fn on_references(&mut self, lang: &'static str, r: Value) {
        let arr = r.as_array().cloned().unwrap_or_default();
        if arr.is_empty() {
            return self.info("No references found");
        }
        let enc = self.lsp.enc(lang);
        let mut cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
        let mut items = Vec::new();
        for loc in arr.iter().take(300) {
            let Some(path) = loc["uri"].as_str().and_then(uri_to_path) else {
                continue;
            };
            let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
            let col = loc["range"]["start"]["character"].as_u64().unwrap_or(0) as usize;
            let preview = match self
                .bufs
                .iter()
                .find(|b| b.path.as_ref().map(|p| path_key(p)) == Some(path_key(&path)))
            {
                Some(b) => b
                    .lines
                    .get(line)
                    .map(|l| l.iter().collect::<String>())
                    .unwrap_or_default(),
                None => cache
                    .entry(path.clone())
                    .or_insert_with(|| {
                        fs::read_to_string(&path)
                            .unwrap_or_default()
                            .lines()
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .get(line)
                    .cloned()
                    .unwrap_or_default(),
            };
            let label = format!("{}:{}  {}", self.rel(&path), line + 1, preview.trim());
            items.push((label, MenuAction::Jump(path, line, col, enc)));
        }
        if items.len() == 1 {
            if let Some((_, MenuAction::Jump(p, l, c, e))) = items.pop() {
                self.jump_to(&p, l, c, e);
            }
            return;
        }
        let n = items.len();
        self.menu = Some(Menu {
            title: format!("References ({})", n),
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

    /// Code actions from every server are merged into one menu.
    fn on_code_actions(&mut self, lang: &'static str, r: Value) {
        self.ca_pending = self.ca_pending.saturating_sub(1);
        let mut arr = r.as_array().cloned().unwrap_or_default();
        if arr.is_empty() {
            if self.ca_pending == 0 && !self.ca_menu_open {
                self.info("No code actions available");
            }
            return;
        }
        arr.sort_by_key(|a| {
            let quick = a["kind"].as_str().is_some_and(|k| k.starts_with("quickfix"));
            let pref = a["isPreferred"].as_bool().unwrap_or(false);
            (!pref, !quick)
        });
        let items = arr
            .into_iter()
            .map(|a| {
                let mut t = a["title"].as_str().unwrap_or("?").to_string();
                if a["kind"].as_str().is_some_and(|k| k.starts_with("quickfix")) {
                    t = format!("⚡ {}", t);
                }
                (t, MenuAction::Code(lang, a))
            })
            .collect::<Vec<_>>();
        if self.ca_menu_open {
            if let Some(m) = self.menu.as_mut().filter(|m| m.title == "Code actions") {
                m.items.extend(items);
                return;
            }
        }
        self.ca_menu_open = true;
        let anchor = self.menu_anchor.take();
        self.menu = Some(Menu {
            title: "Code actions".into(),
            items,
            sel: 0,
            scroll: 0,
            anchor,
            dir: None,
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
    }

    pub(crate) fn run_action(&mut self, lang: &'static str, a: Value, allow_resolve: bool) {
        let mut did = false;
        if a["edit"].is_object() {
            self.apply_workspace_edit(lang, &a["edit"]);
            did = true;
        }
        let cmd = if a["command"].is_string() {
            Some(a.clone())
        } else if a["command"].is_object() {
            Some(a["command"].clone())
        } else {
            None
        };
        if let Some(c) = cmd {
            let p = json!({ "command": c["command"], "arguments": c["arguments"] });
            self.lsp.request(lang, "workspace/executeCommand", p, Pending::Execute);
            did = true;
        }
        if !did && allow_resolve && !a["data"].is_null() {
            self.lsp.request(lang, "codeAction/resolve", a, Pending::Resolve);
            return;
        }
        if !did {
            self.error("Could not apply action");
        }
    }

    fn on_signature(&mut self, r: Value) {
        if self.mode != Mode::Edit || self.focus != Focus::Editor {
            return;
        }
        let Some(sigs) = r["signatures"].as_array().filter(|s| !s.is_empty()) else {
            self.sig = None;
            return;
        };
        let idx = (r["activeSignature"].as_u64().unwrap_or(0) as usize).min(sigs.len() - 1);
        let s = &sigs[idx];
        let label = s["label"].as_str().unwrap_or("").to_string();
        let ap = s["activeParameter"]
            .as_u64()
            .or(r["activeParameter"].as_u64())
            .unwrap_or(0) as usize;
        let lc: Vec<char> = label.chars().collect();
        let hl = match &s["parameters"][ap]["label"] {
            Value::String(p) => {
                let pc: Vec<char> = p.chars().collect();
                (0..lc.len().saturating_sub(pc.len()) + 1)
                    .find(|&i| lc[i..].starts_with(&pc))
                    .map(|i| (i, i + pc.len()))
            }
            Value::Array(a) if a.len() == 2 => {
                let s0 = a[0].as_u64().unwrap_or(0) as usize;
                let s1 = a[1].as_u64().unwrap_or(0) as usize;
                Some((from_lsp_col(&lc, s0, Enc::Utf16), from_lsp_col(&lc, s1, Enc::Utf16)))
            }
            _ => None,
        };
        self.sig = Some((label, hl));
    }

    fn on_format(&mut self, lang: &'static str, path: &Path, r: Value) {
        let Some(edits) = r.as_array() else {
            return self.error("No changes or formatting failed (syntax error?)");
        };
        let key = path_key(path);
        let Some(i) = self
            .bufs
            .iter()
            .position(|b| b.path.as_ref().map(|p| path_key(p)) == Some(key.clone()))
        else {
            return;
        };
        if edits.is_empty() {
            return self.info("Already formatted");
        }
        let enc = self.lsp.enc(lang);
        self.bufs[i].checkpoint();
        self.apply_text_edits(i, edits, enc, None);
        self.bufs[i].clamp(true);
        self.info("Formatted");
    }

    fn buf_index_for(&mut self, path: &Path) -> Option<usize> {
        let key = path_key(path);
        if let Some(i) = self
            .bufs
            .iter()
            .position(|b| b.path.as_ref().map(|p| path_key(p)) == Some(key.clone()))
        {
            return Some(i);
        }
        let b = Buffer::open(path).ok()?;
        self.bufs.push(b);
        Some(self.bufs.len() - 1)
    }

    pub(crate) fn apply_workspace_edit(&mut self, lang: &'static str, edit: &Value) -> usize {
        let enc = self.lsp.enc(lang);
        let mut per: Vec<(PathBuf, Vec<Value>)> = Vec::new();
        if let Some(ch) = edit["changes"].as_object() {
            for (uri, edits) in ch {
                if let (Some(p), Some(e)) = (uri_to_path(uri), edits.as_array()) {
                    per.push((p, e.clone()));
                }
            }
        }
        if let Some(dc) = edit["documentChanges"].as_array() {
            for d in dc {
                if let (Some(p), Some(e)) = (
                    d["textDocument"]["uri"].as_str().and_then(uri_to_path),
                    d["edits"].as_array(),
                ) {
                    per.push((p, e.clone()));
                }
            }
        }
        let mut n = 0;
        for (path, edits) in per {
            if let Some(i) = self.buf_index_for(&path) {
                self.bufs[i].checkpoint();
                self.apply_text_edits(i, &edits, enc, None);
                n += 1;
            }
        }
        n
    }

    fn apply_text_edits(&mut self, i: usize, edits: &[Value], enc: Enc, track: Option<usize>) -> isize {
        let b = &mut self.bufs[i];
        let conv = |b: &Buffer, p: &Value| -> Pos {
            let l = p["line"].as_u64().unwrap_or(0) as usize;
            let c = p["character"].as_u64().unwrap_or(0) as usize;
            if l > b.last() {
                (b.line_len(b.last()), b.last())
            } else {
                (from_lsp_col(&b.lines[l], c, enc), l)
            }
        };
        let mut es: Vec<(Pos, Pos, Vec<char>)> = edits
            .iter()
            .filter_map(|e| {
                let s = conv(b, &e["range"]["start"]);
                let en = conv(b, &e["range"]["end"]);
                let t: Vec<char> = e["newText"].as_str()?.replace("\r\n", "\n").chars().collect();
                Some((s, en, t))
            })
            .collect();
        es.sort_by_key(|e| std::cmp::Reverse((e.0 .1, e.0 .0)));
        let mut delta = 0isize;
        for (s, e, t) in es {
            b.delete_text(s, e);
            b.insert_text(s, &t);
            if let Some(ty) = track {
                if e.1 < ty {
                    delta += t.iter().filter(|c| **c == '\n').count() as isize - (e.1 - s.1) as isize;
                }
            }
        }
        b.touch();
        delta
    }

    pub(crate) fn word_start(&self) -> usize {
        let b = self.buf();
        let line = &b.lines[b.cy];
        let mut s = b.cx.min(line.len());
        while s > 0 && is_ident(line[s - 1]) {
            s -= 1;
        }
        s
    }

    pub(crate) fn trigger_completion(&mut self, manual: bool, trig: Option<char>) {
        let lang = self.cur_cap("completionProvider");
        let Some(lang) = lang else {
            return self.word_completion(manual);
        };
        self.lsp_sync(self.cur);
        if let Some(old) = self.comp_id.take() {
            self.lsp.cancel(lang, old);
        }
        let mut p = self.tdpp(lang);
        p["context"] = match trig {
            Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
            None => json!({ "triggerKind": if manual { 1 } else { 3 } }),
        };
        if trig.is_none() && !manual {
            p["context"] = json!({ "triggerKind": 1 });
        }
        self.comp_id = self
            .lsp
            .request(lang, "textDocument/completion", p, Pending::Completion);
    }

    fn word_completion(&mut self, manual: bool) {
        let start = self.word_start();
        let b = self.buf();
        let prefix: String = b.lines[b.cy][start..b.cx.min(b.line_len(b.cy))].iter().collect();
        if prefix.chars().count() < if manual { 1 } else { 3 } {
            if !manual {
                self.popup = None;
            }
            return;
        }
        let mut words: HashSet<String> = HashSet::new();
        for buf in &self.bufs {
            for line in buf.lines.iter().take(20_000) {
                let mut cur = String::new();
                for &c in line.iter().chain(std::iter::once(&' ')) {
                    if is_ident(c) {
                        cur.push(c);
                    } else {
                        if cur.chars().count() >= 3 && !cur.chars().next().unwrap().is_ascii_digit() {
                            words.insert(std::mem::take(&mut cur));
                        }
                        cur.clear();
                    }
                }
            }
        }
        words.remove(&prefix);
        if words.is_empty() {
            self.popup = None;
            return;
        }
        let items = words
            .into_iter()
            .map(|w| CItem {
                suffix: String::new(),
                filter: w.to_lowercase(),
                sort: w.clone(),
                text: w.clone(),
                label: w,
                kind: 1,
                detail: String::new(),
                snippet: false,
                start: None,
                end: None,
                extra: Vec::new(),
                raw: Value::Null,
            })
            .collect();
        let y = self.buf().cy;
        self.popup = Some(Popup {
            items,
            shown: Vec::new(),
            sel: 0,
            scroll: 0,
            y,
            start,
            lsp: false,
            incomplete: false,
        });
        self.refilter();
    }

    fn on_completion(&mut self, lang: &'static str, r: Value) {
        if self.mode != Mode::Edit || self.focus != Focus::Editor {
            return;
        }
        let enc = self.lsp.enc(lang);
        let (list, incomplete) = if r.is_array() {
            (r.as_array().cloned().unwrap_or_default(), false)
        } else {
            (
                r["items"].as_array().cloned().unwrap_or_default(),
                r["isIncomplete"].as_bool().unwrap_or(false),
            )
        };
        let b = self.buf();
        let y = b.cy;
        let line = &b.lines[y];
        let items: Vec<CItem> = list
            .iter()
            .map(|it| {
                let label = it["label"].as_str().unwrap_or("").to_string();
                let te = &it["textEdit"];
                let range = if te["range"].is_object() {
                    &te["range"]
                } else {
                    &te["replace"]
                };
                let (text, start, end) = match te["newText"].as_str() {
                    Some(t) if range["start"]["line"].as_u64() == Some(y as u64) => {
                        let s = from_lsp_col(line, range["start"]["character"].as_u64().unwrap_or(0) as usize, enc);
                        let e = from_lsp_col(line, range["end"]["character"].as_u64().unwrap_or(0) as usize, enc);
                        (t.to_string(), Some(s), Some(e))
                    }
                    Some(t) => (t.to_string(), None, None),
                    None => (it["insertText"].as_str().unwrap_or(&label).to_string(), None, None),
                };
                let mut detail = it["detail"]
                    .as_str()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if detail.is_empty() {
                    if let Some(d) = it["labelDetails"]["description"].as_str() {
                        detail = d.to_string();
                    }
                }
                CItem {
                    suffix: it["labelDetails"]["detail"].as_str().unwrap_or("").trim().to_string(),
                    filter: it["filterText"].as_str().unwrap_or(&label).to_lowercase(),
                    sort: it["sortText"].as_str().unwrap_or(&label).to_string(),
                    kind: it["kind"].as_u64().unwrap_or(0),
                    detail,
                    text,
                    snippet: it["insertTextFormat"].as_u64() == Some(2),
                    start,
                    end,
                    extra: it["additionalTextEdits"].as_array().cloned().unwrap_or_default(),
                    raw: it.clone(),
                    label,
                }
            })
            .collect();
        if items.is_empty() {
            self.popup = None;
            return;
        }
        let start = self.word_start();
        self.popup = Some(Popup {
            items,
            shown: Vec::new(),
            sel: 0,
            scroll: 0,
            y,
            start,
            lsp: true,
            incomplete,
        });
        self.refilter();
    }

    pub(crate) fn refilter(&mut self) {
        let b = &self.bufs[self.cur];
        let (cx, cy) = (b.cx.min(b.line_len(b.cy)), b.cy);
        let line = &b.lines[cy];
        let Some(p) = self.popup.as_mut() else { return };
        if cy != p.y || cx < p.start {
            self.popup = None;
            return;
        }
        let prefix: String = line[p.start..cx].iter().collect::<String>().to_lowercase();
        let mut scored: Vec<(u8, usize)> = p
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| fuzzy(&prefix, &it.filter).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| p.items[a.1].sort.cmp(&p.items[b.1].sort)));
        p.shown = scored.into_iter().map(|x| x.1).collect();
        p.sel = 0;
        p.scroll = 0;
        let only_exact = p.shown.len() == 1 && p.items[p.shown[0]].filter == prefix;
        if p.shown.is_empty() || (!p.lsp && only_exact) {
            self.popup = None;
        }
    }

    pub(crate) fn popup_move(&mut self, d: isize) {
        let Some(p) = self.popup.as_mut() else { return };
        let n = p.shown.len() as isize;
        if n == 0 {
            return;
        }
        p.sel = (p.sel as isize + d).rem_euclid(n) as usize;
        if p.sel < p.scroll {
            p.scroll = p.sel;
        } else if p.sel >= p.scroll + POPUP_ROWS {
            p.scroll = p.sel + 1 - POPUP_ROWS;
        }
    }

    pub(crate) fn popup_fully_typed(&self) -> bool {
        let Some(p) = &self.popup else { return false };
        let Some(&i) = p.shown.get(p.sel) else { return false };
        let b = self.buf();
        let typed: String = b.lines[b.cy][p.start..b.cx.min(b.line_len(b.cy))].iter().collect();
        p.items[i].text == typed && p.items[i].extra.is_empty()
    }

    pub(crate) fn accept_completion(&mut self) {
        let Some(p) = self.popup.take() else { return };
        let Some(&idx) = p.shown.get(p.sel) else { return };
        let it = &p.items[idx];
        let enc = self
            .cur_cap("completionProvider")
            .map(|l| self.lsp.enc(l))
            .unwrap_or(Enc::Utf16);
        let unit = self.indent_unit();
        // Edits that come with the item (auto-import `use` lines). Servers like
        // rust-analyzer only work them out on resolve, against the text as it is
        // now: ask before changing anything, or the answer comes back empty.
        let mut extra = it.extra.clone();
        if extra.is_empty() && !it.raw.is_null() {
            if let Some(r) = self.resolve_completion(it.raw.clone()) {
                extra = r["additionalTextEdits"].as_array().cloned().unwrap_or_default();
            }
        }
        let b = &mut self.bufs[self.cur];
        let y = b.cy;
        let len = b.line_len(y);
        let cx = b.cx.min(len);
        let start = it.start.filter(|s| *s <= cx).unwrap_or(p.start).min(cx);
        let end = it.end.filter(|e| *e > cx && *e <= len).unwrap_or(cx);
        let (raw, off) = if it.snippet {
            expand_snippet(&it.text)
        } else {
            (it.text.clone(), None)
        };
        let base = leading_ws(&b.lines[y]);
        let mut chars: Vec<char> = Vec::new();
        let mut new_off = None;
        let mut line_start = false;
        for (k, c) in raw.chars().enumerate() {
            if Some(k) == off {
                new_off = Some(chars.len());
            }
            if c == '\n' {
                chars.push('\n');
                chars.extend(&base);
                line_start = true;
                continue;
            }
            if line_start && c == '\t' {
                chars.extend(&unit);
                continue;
            }
            line_start = false;
            chars.push(c);
        }
        if off.is_some_and(|o| o >= raw.chars().count()) {
            new_off = Some(chars.len());
        }
        b.delete_text((start, y), (end, y));
        let endp = b.insert_text((start, y), &chars);
        let (mut nx, mut ny) = match new_off {
            Some(o) => {
                let before = &chars[..o];
                match before.iter().rposition(|c| *c == '\n') {
                    Some(nl) => (o - nl - 1, y + before.iter().filter(|c| **c == '\n').count()),
                    None => (start + o, y),
                }
            }
            None => endp,
        };
        if !extra.is_empty() {
            let d = self.apply_text_edits(self.cur, &extra, enc, Some(y));
            ny = (ny as isize + d).max(0) as usize;
            if d != 0 {
                nx = nx.min(self.bufs[self.cur].line_len(ny.min(self.bufs[self.cur].last())));
            }
        }
        let b = self.buf_mut();
        b.cy = ny.min(b.last());
        b.cx = nx;
        b.sync_want();
        if chars.last() == Some(&')') && new_off.is_some() {
            self.trigger_signature();
        }
    }

    /// Resolves a completion item and waits for the answer (up to
    /// `RESOLVE_TIMEOUT`) while the buffer is still as the server saw it. A late
    /// answer is applied by `on_comp_resolve` when it arrives.
    fn resolve_completion(&mut self, item: Value) -> Option<Value> {
        let lang = self.cur_cap("completionProvider")?;
        let can = self.lsp.servers.get(lang).is_some_and(|s| {
            s.caps["completionProvider"]["resolveProvider"]
                .as_bool()
                .unwrap_or(false)
        });
        if !can {
            return None;
        }
        self.lsp_sync(self.cur);
        let id = self
            .lsp
            .request(lang, "completionItem/resolve", item, Pending::CompResolve(self.cur))?;
        let deadline = Instant::now() + RESOLVE_TIMEOUT;
        loop {
            for ev in self.lsp.poll() {
                match ev {
                    LspEvent::Response {
                        id: rid,
                        result,
                        error: None,
                        ..
                    } if rid == id => return Some(result),
                    ev => self.on_lsp_event(ev),
                }
            }
            if !self.lsp.is_pending(lang, id) || Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn on_comp_resolve(&mut self, lang: &'static str, buf: usize, r: Value) {
        let Some(edits) = r["additionalTextEdits"].as_array().filter(|e| !e.is_empty()).cloned() else {
            return;
        };
        if buf >= self.bufs.len() {
            return;
        }
        let enc = self.lsp.enc(lang);
        let y = self.bufs[buf].cy;
        let d = self.apply_text_edits(buf, &edits, enc, Some(y));
        let b = &mut self.bufs[buf];
        b.cy = ((b.cy as isize + d).max(0) as usize).min(b.last());
        b.clamp(true);
    }

    pub(crate) fn after_insert_char(&mut self, c: char) {
        if self.burst {
            self.popup = None;
            return;
        }
        let comp_trig = self
            .cur_cap("completionProvider")
            .map(|l| self.lsp.trigger_chars(l, "completionProvider"))
            .unwrap_or_default();
        if is_ident(c) {
            match &self.popup {
                Some(p) if !p.incomplete => self.refilter(),
                Some(_) => {
                    self.refilter();
                    self.trigger_completion(false, None);
                }
                None => self.trigger_completion(false, None),
            }
        } else if comp_trig.contains(&c) {
            self.popup = None;
            self.trigger_completion(false, Some(c));
        } else {
            self.popup = None;
        }
        let mut sig_trig = self
            .cur_cap("signatureHelpProvider")
            .map(|l| self.lsp.trigger_chars(l, "signatureHelpProvider"))
            .unwrap_or_default();
        if sig_trig.is_empty() {
            sig_trig = vec!['(', ','];
        }
        if sig_trig.contains(&c) {
            self.trigger_signature();
        } else if c == ')' {
            self.sig = None;
        }
    }

    pub(crate) fn trigger_signature(&mut self) {
        let Some(lang) = self.cur_cap("signatureHelpProvider") else {
            return;
        };
        self.lsp_sync(self.cur);
        let p = self.tdpp(lang);
        self.lsp
            .request(lang, "textDocument/signatureHelp", p, Pending::Signature);
    }

    pub(crate) fn menu_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(m) = self.menu.as_mut() else { return };
        let n = m.items.len();
        if m.filter.is_some() {
            match k.code {
                KeyCode::Char(c) if !ctrl => {
                    m.filter.as_mut().unwrap().push(c);
                    m.apply_filter();
                    if m.ws_symbols {
                        let q = m.filter.clone().unwrap_or_default();
                        self.ws_symbols_query(&q);
                    }
                    return;
                }
                KeyCode::Backspace => {
                    m.filter.as_mut().unwrap().pop();
                    m.apply_filter();
                    if m.ws_symbols {
                        let q = m.filter.clone().unwrap_or_default();
                        self.ws_symbols_query(&q);
                    }
                    return;
                }
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::Enter
                | KeyCode::Esc
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Tab
                | KeyCode::BackTab => {}
                _ => return,
            }
        }
        if let Some(d) = m.dir.clone() {
            match k.code {
                KeyCode::Backspace | KeyCode::Left => {
                    let project = matches!(m.items.first(), Some((_, MenuAction::OpenProject(_))));
                    if let Some(p) = d.parent() {
                        self.folder_picker(p.to_path_buf(), Some(d), project);
                    }
                    return;
                }
                KeyCode::Right => {
                    let s = m.sel;
                    return self.menu_choose(s);
                }
                KeyCode::Char(c) if c.is_alphabetic() && !ctrl => {
                    let c = c.to_lowercase().next().unwrap_or(c);
                    let name = |t: &str| t.trim_start_matches(|ch: char| !ch.is_alphanumeric()).to_lowercase();
                    let start = m.sel + 1;
                    if let Some(i) = (0..n)
                        .map(|k| (start + k) % n)
                        .find(|&i| name(&m.items[i].0).starts_with(c))
                    {
                        m.sel = i;
                    }
                    return self.menu_follow();
                }
                _ => {}
            }
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.menu = None,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => m.sel = (m.sel + 1) % n.max(1),
            KeyCode::Char('n') if ctrl => m.sel = (m.sel + 1) % n.max(1),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => m.sel = (m.sel + n.max(1) - 1) % n.max(1),
            KeyCode::Char('p') if ctrl => m.sel = (m.sel + n.max(1) - 1) % n.max(1),
            KeyCode::PageDown => m.sel = (m.sel + MENU_ROWS).min(n.saturating_sub(1)),
            KeyCode::PageUp => m.sel = m.sel.saturating_sub(MENU_ROWS),
            KeyCode::Home | KeyCode::Char('g') => m.sel = 0,
            KeyCode::End | KeyCode::Char('G') => m.sel = n.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('l') => {
                let s = m.sel;
                return self.menu_choose(s);
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let i = c.to_digit(10).unwrap() as usize - 1;
                if i < n {
                    return self.menu_choose(i);
                }
            }
            _ => {}
        }
        self.menu_follow();
    }

    pub(crate) fn menu_follow(&mut self) {
        if let Some(m) = self.menu.as_mut() {
            if m.sel < m.scroll {
                m.scroll = m.sel;
            } else if m.sel >= m.scroll + MENU_ROWS {
                m.scroll = m.sel + 1 - MENU_ROWS;
            }
        }
    }

    /// `:hints` opens the toggle menu; `:hints on|off`; `:hints <kind>` toggles one kind.
    pub(crate) fn hints_command(&mut self, args: &[String]) {
        let mut a = args.iter().map(|s| s.as_str());
        match a.next() {
            None => self.hints_menu(0),
            // From the menu: toggle, then reopen it on the same row.
            Some("menu") => {
                let key = a.next().and_then(hint_key).unwrap_or("enabled");
                self.toggle_hint(key);
                self.info(self.hints_summary());
                let row = HINT_ROWS.iter().position(|r| r.0 == key).map_or(0, |i| i + 1);
                self.hints_menu(row);
            }
            Some("on") => {
                self.set_hint("enabled", true);
                self.info(self.hints_summary());
            }
            Some("off") => {
                self.set_hint("enabled", false);
                self.info(self.hints_summary());
            }
            Some(k) => match hint_key(k) {
                Some(key) => {
                    self.toggle_hint(key);
                    self.info(self.hints_summary());
                }
                None => self.error("Usage: :hints [on|off|types|params|returns|chaining|braces]"),
            },
        }
    }

    fn toggle_hint(&mut self, key: &str) {
        let on = self.settings.inlay.get(key).unwrap_or(false);
        self.set_hint(key, !on);
    }

    /// Switches a kind for this session. Off takes effect at once; on asks the
    /// server again.
    fn set_hint(&mut self, key: &str, v: bool) {
        if !self.settings.inlay.set(key, v) {
            return;
        }
        for b in self.bufs.iter_mut() {
            if !v {
                b.hints.retain(|h| h.kind != key);
            }
            b.hint_rev = None;
        }
    }

    fn hints_summary(&self) -> String {
        let i = self.settings.inlay;
        let keep = "(\"inlayHints\" in :settings keeps it)";
        if !i.enabled {
            return format!("Inlay hints off {}", keep);
        }
        let on: Vec<&str> = HINT_ROWS
            .iter()
            .filter(|r| i.get(r.0) == Some(true))
            .map(|r| r.1)
            .collect();
        let on = if on.is_empty() {
            "none".to_string()
        } else {
            on.join(", ")
        };
        format!("Inlay hints: {} {}", on, keep)
    }

    fn hints_menu(&mut self, sel: usize) {
        let i = self.settings.inlay;
        let mark = |on: bool| if on { "[x]" } else { "[ ]" };
        let mut items = vec![(
            format!("{} Show inlay hints", mark(i.enabled)),
            MenuAction::Run("hints menu enabled".into()),
        )];
        for (key, label, eg) in HINT_ROWS {
            items.push((
                format!("{} {:<16} {}", mark(i.get(key) == Some(true)), label, eg),
                MenuAction::Run(format!("hints menu {}", key)),
            ));
        }
        self.menu = Some(Menu {
            title: "Inlay hints · Enter toggles".into(),
            items,
            sel,
            scroll: 0,
            anchor: None,
            dir: None,
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
    }

    pub(crate) fn menu_choose(&mut self, idx: usize) {
        let Some(m) = self.menu.take() else { return };
        let project = matches!(m.items.first(), Some((_, MenuAction::OpenProject(_))));
        let Some((_, act)) = m.items.into_iter().nth(idx) else {
            return;
        };
        match act {
            MenuAction::Code(lang, v) => self.run_action(lang, v, true),
            MenuAction::Jump(p, l, c, e) => self.jump_to(&p, l, c, e),
            MenuAction::Cmd(c) => self.context_cmd(c),
            MenuAction::Run(s) => self.run_command(&s),
            MenuAction::Dir(p) => self.folder_picker(p, None, project),
            MenuAction::OpenProject(p) => self.open_project(&p),
            MenuAction::OpenDir(p) => {
                self.open_path(&p);
                let name = self.tree.root_name();
                self.info(format!("Opened folder: {}", name));
            }
        }
    }

    /// Browses folders. `project`: the chosen folder opens as a new project
    /// instead of replacing this one's tree.
    pub(crate) fn folder_picker(&mut self, dir: PathBuf, from: Option<PathBuf>, project: bool) {
        self.enter_edit();
        let open = if project {
            ("✓  Open as a project".into(), MenuAction::OpenProject(dir.clone()))
        } else {
            ("✓  Open this folder".into(), MenuAction::OpenDir(dir.clone()))
        };
        let mut items: Vec<(String, MenuAction)> = vec![open];
        match dir.parent() {
            Some(p) => items.push(("↑  ..".into(), MenuAction::Dir(p.to_path_buf()))),
            None if cfg!(windows) => {
                for l in 'C'..='Z' {
                    let d = PathBuf::from(format!("{}:\\", l));
                    if d != dir && d.exists() {
                        items.push((format!("▸  {}:\\", l), MenuAction::Dir(d)));
                    }
                }
            }
            None => {}
        }
        let mut subs: Vec<(String, PathBuf)> = fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                    .filter(|(n, _)| !n.starts_with('.') && !n.starts_with('$'))
                    .collect()
            })
            .unwrap_or_default();
        subs.sort_by_key(|(n, _)| n.to_lowercase());
        let mut sel = 0;
        for (n, p) in subs {
            if from.as_ref() == Some(&p) {
                sel = items.len();
            }
            items.push((format!("▸  {}", n), MenuAction::Dir(p)));
        }
        let full = dir.display().to_string();
        let chars: Vec<char> = full.chars().collect();
        let title = if chars.len() > 50 {
            format!("…{}", chars[chars.len() - 49..].iter().collect::<String>())
        } else {
            full
        };
        self.menu = Some(Menu {
            title,
            items,
            sel,
            scroll: 0,
            anchor: None,
            dir: Some(dir),
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
        self.menu_follow();
    }

    pub(crate) fn open_context_menu(&mut self, x: u16, y: u16) {
        let selected = self.sel_range().is_some();
        let mut items: Vec<(String, MenuAction)> = Vec::new();
        if self.cur_ready().is_some() {
            items.push(("Go to definition    F12".into(), MenuAction::Cmd("def")));
            items.push(("Find references     ⇧F12".into(), MenuAction::Cmd("refs")));
            items.push(("Code actions        Ctrl+.".into(), MenuAction::Cmd("actions")));
            items.push(("Show info           F1".into(), MenuAction::Cmd("hover")));
            items.push(("Rename              F2".into(), MenuAction::Cmd("rename")));
            items.push(("Format document".into(), MenuAction::Cmd("format")));
            items.push(("Symbols             Ctrl+⇧O".into(), MenuAction::Cmd("symbols")));
        }
        if selected {
            items.push(("Copy                Ctrl+C".into(), MenuAction::Cmd("copy")));
            items.push(("Cut                 Ctrl+X".into(), MenuAction::Cmd("cut")));
        } else {
            items.push(("Copy line           Ctrl+C".into(), MenuAction::Cmd("copy")));
        }
        items.push(("Paste               Ctrl+V".into(), MenuAction::Cmd("paste")));
        items.push(("Select all          Ctrl+A".into(), MenuAction::Cmd("selectall")));
        items.push(("Toggle breakpoint   F9".into(), MenuAction::Cmd("breakpoint")));
        items.push(("Find and replace    Ctrl+H".into(), MenuAction::Cmd("find")));
        self.ctx_pos = Some((x, y));
        self.menu = Some(Menu {
            title: String::new(),
            items,
            sel: 0,
            scroll: 0,
            anchor: Some((x, y)),
            dir: None,
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
    }

    fn context_cmd(&mut self, c: &str) {
        match c {
            "def" => self.lsp_definition(),
            "refs" => self.lsp_references(),
            "actions" => {
                self.menu_anchor = self.ctx_pos;
                self.lsp_code_actions();
            }
            "hover" => self.lsp_hover(),
            "rename" => self.start_rename(),
            "format" => self.lsp_format(),
            "symbols" => self.lsp_document_symbols(),
            "find" => self.open_find(true),
            "breakpoint" => {
                let y = self.buf().cy;
                self.toggle_breakpoint(y);
            }
            "copy" => self.copy(),
            "cut" => self.cut(),
            "paste" => self.paste(),
            "selectall" => self.select_all(),
            _ => {}
        }
    }

    pub(crate) fn push_jump(&mut self) {
        let b = self.buf();
        if let Some(p) = &b.path {
            self.jumps.push((p.clone(), (b.cx, b.cy)));
            if self.jumps.len() > 100 {
                self.jumps.remove(0);
            }
        }
    }

    pub(crate) fn jump_to(&mut self, path: &Path, line: usize, col: usize, enc: Enc) {
        self.push_jump();
        self.open_path(path);
        self.focus = Focus::Editor;
        let b = self.buf_mut();
        b.cy = line.min(b.last());
        b.cx = from_lsp_col(&b.lines[b.cy], col, enc);
        b.sync_want();
    }

    pub(crate) fn jump_back(&mut self) {
        let Some((p, (x, y))) = self.jumps.pop() else {
            return self.info("Jump list is empty");
        };
        self.open_path(&p);
        let b = self.buf_mut();
        b.cy = y.min(b.last());
        b.cx = x;
        b.clamp(true);
        b.sync_want();
    }

    pub(crate) fn lsp_command(&mut self, arg: &str) {
        let lang = self.buf().lang.name;
        match arg {
            "restart" | "start" => {
                self.lsp.stop_lang(lang);
                self.lsp.failed.remove(lang);
                for i in 0..self.bufs.len() {
                    if self.bufs[i].lang.name == lang {
                        self.bufs[i].lsp_open = false;
                    }
                }
                self.info("Restarting language server…");
            }
            "stop" => {
                self.lsp.stop_lang(lang);
                self.lsp.failed.insert(lang);
                for b in self.bufs.iter_mut() {
                    if b.lang.name == lang {
                        b.lsp_open = false;
                    }
                }
                self.info("Language server stopped");
            }
            _ => {
                if self.lsp.servers.is_empty() {
                    let c: Vec<&str> = candidates(lang).iter().map(|c| c[0]).collect();
                    if c.is_empty() {
                        self.info(format!("No language server configured for {}", lang));
                    } else {
                        self.info(format!("No server running. Looking for ({}): {}", lang, c.join(", ")));
                    }
                    return;
                }
                let s = self
                    .lsp
                    .servers
                    .values()
                    .map(|s| {
                        format!(
                            "{}: {} [{}] {}",
                            s.lang,
                            s.name,
                            if s.ready { "ready" } else { "starting" },
                            s.root.display()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("  |  ");
                self.info(s);
            }
        }
    }

    pub(crate) fn lsp_document_symbols(&mut self) {
        self.enter_edit();
        let Some(lang) = self.ready_lang("documentSymbolProvider") else {
            return;
        };
        let uri = path_to_uri(self.buf().path.as_ref().unwrap());
        self.lsp.request(
            lang,
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
            Pending::DocSymbols,
        );
    }

    pub(crate) fn workspace_symbols(&mut self) {
        self.enter_edit();
        let Some(_) = self.ready_lang("workspaceSymbolProvider") else {
            return;
        };
        self.menu = Some(Menu {
            title: "Workspace symbols".to_string(),
            items: Vec::new(),
            sel: 0,
            scroll: 0,
            anchor: None,
            dir: None,
            filter: Some(String::new()),
            base: Vec::new(),
            ws_symbols: true,
        });
    }

    fn ws_symbols_query(&mut self, q: &str) {
        let Some(lang) = self.cur_cap("workspaceSymbolProvider") else {
            return;
        };
        if let Some(old) = self.ws_sym_id.take() {
            self.lsp.cancel(lang, old);
        }
        if q.is_empty() {
            return;
        }
        self.ws_sym_id = self
            .lsp
            .request(lang, "workspace/symbol", json!({ "query": q }), Pending::WsSymbols);
    }

    fn symbol_label(kind: u64) -> &'static str {
        match kind {
            1 => "file",
            2 => "mod",
            3 => "ns",
            4 => "pkg",
            5 => "class",
            6 => "meth",
            7 => "prop",
            8 => "field",
            9 => "new",
            10 => "enum",
            11 => "trait",
            12 => "fn",
            13 => "var",
            14 => "const",
            15 => "str",
            16 => "num",
            17 => "bool",
            18 => "arr",
            19 => "obj",
            20 => "key",
            21 => "null",
            22 => "member",
            23 => "struct",
            24 => "event",
            25 => "op",
            26 => "type",
            _ => "",
        }
    }

    fn on_doc_symbols(&mut self, lang: &'static str, r: Value) {
        let enc = self.lsp.enc(lang);
        let Some(path) = self.buf().path.clone() else { return };
        let mut flat: Vec<(usize, u64, String, String, usize, usize)> = Vec::new();
        fn walk(v: &Value, depth: usize, out: &mut Vec<(usize, u64, String, String, usize, usize)>) {
            for s in v.as_array().into_iter().flatten() {
                let range = if s["selectionRange"].is_object() {
                    &s["selectionRange"]
                } else {
                    &s["location"]["range"]
                };
                out.push((
                    depth,
                    s["kind"].as_u64().unwrap_or(0),
                    s["name"].as_str().unwrap_or("").to_string(),
                    s["detail"]
                        .as_str()
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string(),
                    range["start"]["line"].as_u64().unwrap_or(0) as usize,
                    range["start"]["character"].as_u64().unwrap_or(0) as usize,
                ));
                walk(&s["children"], depth + 1, out);
            }
        }
        walk(&r, 0, &mut flat);
        if flat.is_empty() {
            return self.info("No symbols found");
        }
        if flat.iter().all(|f| f.0 == 0) {
            flat.sort_by_key(|f| (f.4, f.5));
        }
        let cy = self.buf().cy;
        let sel = flat.iter().rposition(|f| f.4 <= cy).unwrap_or(0);
        let items: Vec<(String, MenuAction)> = flat
            .into_iter()
            .map(|(depth, kind, name, detail, line, col)| {
                let mut label = format!("{}{:<7}{}", "  ".repeat(depth), Self::symbol_label(kind), name);
                if !detail.is_empty() {
                    label = format!("{}  {}", label, detail);
                }
                (label, MenuAction::Jump(path.clone(), line, col, enc))
            })
            .collect();
        let n = items.len();
        let mut m = Menu::filterable(format!("Symbols ({})", n), items, sel);
        m.sel = sel;
        self.menu = Some(m);
        self.menu_follow();
    }

    fn on_ws_symbols(&mut self, lang: &'static str, id: i64, r: Value) {
        if self.ws_sym_id != Some(id) {
            return;
        }
        self.ws_sym_id = None;
        let enc = self.lsp.enc(lang);
        let items: Vec<(String, MenuAction)> = r
            .as_array()
            .into_iter()
            .flatten()
            .take(300)
            .filter_map(|s| {
                let loc = &s["location"];
                let path = loc["uri"].as_str().and_then(uri_to_path)?;
                let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                let col = loc["range"]["start"]["character"].as_u64().unwrap_or(0) as usize;
                let kind = Self::symbol_label(s["kind"].as_u64().unwrap_or(0));
                let name = s["name"].as_str().unwrap_or("");
                let container = s["containerName"].as_str().filter(|c| !c.is_empty());
                let mut label = format!("{:<7}{}", kind, name);
                if let Some(c) = container {
                    label = format!("{}  ({})", label, c);
                }
                label = format!("{}  {}:{}", label, self.rel(&path), line + 1);
                Some((label, MenuAction::Jump(path, line, col, enc)))
            })
            .collect();
        if let Some(m) = self.menu.as_mut().filter(|m| m.ws_symbols) {
            m.base = items.clone();
            m.items = items;
            m.sel = 0;
            m.scroll = 0;
        }
    }
}

/// See `Editor::problem_marks`.
fn problem_marks(diags: &HashMap<String, Vec<Diag>>, scanned: Vec<(&String, u8)>) -> HashMap<String, u8> {
    let mut marks: HashMap<String, u8> = HashMap::new();
    let served = diags.iter().filter_map(|(key, ds)| {
        let sev = ds.iter().map(|d| d.sev).filter(|s| (1..=2).contains(s)).min()?;
        Some((key, sev))
    });
    for (key, sev) in served.chain(scanned) {
        for p in Path::new(key).ancestors() {
            let m = marks.entry(path_key(p)).or_insert(sev);
            *m = (*m).min(sev);
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_marks_take_the_worst() {
        let d = |sev: u8| Diag {
            server: "Rust",
            l0: 0,
            c0: 0,
            l1: 0,
            c1: 1,
            sev,
            msg: String::new(),
            raw: Value::Null,
            enc: Enc::Utf16,
        };
        let mut diags = HashMap::new();
        diags.insert("/p/src/a.rs".to_string(), vec![d(2), d(1)]); // both: error
        diags.insert("/p/src/b.rs".to_string(), vec![d(2), d(3)]); // warning
        diags.insert("/p/lib/c.rs".to_string(), vec![d(3), d(4)]); // info/hint only
        diags.insert("/p/lib/d.rs".to_string(), vec![]);
        diags.insert("/p/doc/e.rs".to_string(), vec![d(2)]);
        let m = problem_marks(&diags, Vec::new());
        assert_eq!(m.get("/p/src/a.rs"), Some(&1));
        assert_eq!(m.get("/p/src/b.rs"), Some(&2));
        assert_eq!(m.get("/p/lib/c.rs"), None);
        assert_eq!(m.get("/p/lib"), None);
        // Folders take the worst inside them.
        assert_eq!(m.get("/p/src"), Some(&1));
        assert_eq!(m.get("/p/doc"), Some(&2));
        assert_eq!(m.get("/p"), Some(&1));
        // A checker's findings count too, and the worst still wins.
        let (x, y) = ("/p/lib/x.py".to_string(), "/p/src/b.rs".to_string());
        let m = problem_marks(&diags, vec![(&x, 2), (&y, 1)]);
        assert_eq!(m.get("/p/lib"), Some(&2));
        assert_eq!(m.get("/p/src/b.rs"), Some(&1));
    }

    fn kind(lsp: Option<u64>, label: &str, line: &str, x: usize) -> &'static str {
        hint_kind(lsp, label, &line.chars().collect::<Vec<_>>(), x)
    }

    #[test]
    fn hint_kinds() {
        assert_eq!(kind(Some(1), ": i32", "let x = 1;", 5), "types");
        assert_eq!(kind(Some(2), "a:", "add(1)", 4), "parameters");
        assert_eq!(kind(Some(1), " -> i32", "|x| { x }", 3), "returnTypes");
        assert_eq!(kind(Some(1), ": number", "function f(a) {", 13), "returnTypes");
        assert_eq!(kind(Some(1), "Iter<'_, i32>", "    .iter()", 11), "chaining");
        assert_eq!(kind(None, "fn main", "}", 1), "closingBraces");
    }

    #[test]
    fn module_files() {
        let root = std::env::temp_dir().join(format!("ren-mod-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/utils")).unwrap();
        fs::create_dir_all(root.join("src/bin")).unwrap();
        for f in [
            "Cargo.toml",
            "src/main.rs",
            "src/utils/mod.rs",
            "src/utils/deep.rs",
            "src/orphan.rs",
        ] {
            fs::write(root.join(f), "").unwrap();
        }
        let src = root.join("src");
        // A file next to main.rs is declared there; one in a folder, in its mod.rs.
        assert_eq!(
            module_parents(&src.join("orphan.rs")),
            Some(("orphan".into(), vec![src.join("main.rs")]))
        );
        assert_eq!(
            module_parents(&src.join("utils/mod.rs")),
            Some(("utils".into(), vec![src.join("main.rs")]))
        );
        assert_eq!(
            module_parents(&src.join("utils/deep.rs")),
            Some(("deep".into(), vec![src.join("utils/mod.rs")]))
        );
        assert!(is_crate_root(&src.join("main.rs")) && is_crate_root(&src.join("bin/tool.rs")));
        assert!(!is_crate_root(&src.join("orphan.rs")));
        assert!(module_parents(&std::env::temp_dir().join("loose.rs")).is_none_or(|(_, p)| p.is_empty()));
        let _ = fs::remove_dir_all(&root);
        assert!(is_rust_ident("my_mod2") && !is_rust_ident("my-mod") && !is_rust_ident("2x"));
    }

    #[test]
    fn mod_line_position() {
        let lines = |s: &str| s.lines().map(|l| l.chars().collect::<Vec<_>>()).collect::<Vec<_>>();
        // After the last `mod x;`; inline `mod tests {` doesn't count.
        assert_eq!(
            mod_insert_line(&lines("mod a;\npub mod b;\n\nfn main() {}\nmod tests {")),
            (2, false)
        );
        // No mods yet: at the top, below inner docs and attributes.
        assert_eq!(
            mod_insert_line(&lines("//! Crate docs\n#![allow(x)]\nuse std::fs;")),
            (2, true)
        );
    }

    #[test]
    fn hint_labels() {
        assert_eq!(hint_label(&json!(" -> i32")).as_deref(), Some(" -> i32"));
        assert_eq!(
            hint_label(&json!([{ "value": ": " }, { "value": "Vec" }, { "value": "<i32>" }])).as_deref(),
            Some(": Vec<i32>")
        );
        assert_eq!(hint_label(&json!("a\n   b")).as_deref(), Some("a b"));
        assert_eq!(hint_label(&json!("  ")), None);
        assert_eq!(hint_label(&json!("x".repeat(60))).map(|s| s.chars().count()), Some(48));
    }
}
