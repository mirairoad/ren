//! Workspace files: `<name>.workspace.json` lists folders that open together,
//! `{ "open": ["/work/api", "~/work/web", "../shared"] }` (comments and
//! trailing commas allowed, and JavaScript's `{ open: ['/a', '/b'] }` too;
//! `~/` is home, a relative path starts from the file's folder).
//!
//! `ren name.workspace.json` (or `:workspace open`) opens every folder as a
//! project and brings back where the workspace was left: the window's session
//! is tagged with the file (`session.rs`), so its tabs, cursors, trees, panel
//! and unsaved text come back the next time the file is opened.
//!
//! The file decides which folders open: one added to it since last time opens
//! fresh, one taken out stays closed. Opening or closing projects in the
//! window only changes the session; `:workspace save` writes the open
//! projects into the file (only its `"open"` list is rewritten, so comments
//! and other keys stay).

use super::*;
use serde_json::Value;
use std::collections::HashSet;

/// The end of a file name that makes the file a workspace.
pub const SUFFIX: &str = ".workspace.json";

pub fn is_workspace_file(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().ends_with(SUFFIX))
}

/// `work` for `work.workspace.json`.
pub fn workspace_name(p: &Path) -> String {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = if n.to_ascii_lowercase().ends_with(SUFFIX) {
        &n[..n.len() - SUFFIX.len()]
    } else {
        n.as_str()
    };
    if stem.is_empty() {
        "workspace".into()
    } else {
        stem.to_string()
    }
}

/// What a workspace file lists.
#[derive(Default)]
pub struct Listed {
    /// The folders that exist, in the file's order, each with its entry as
    /// written (kept when the file is saved again).
    pub folders: Vec<(String, PathBuf)>,
    /// Entries that aren't folders (not cloned here, moved, mistyped).
    pub missing: Vec<String>,
}

impl Listed {
    fn paths(&self) -> Vec<PathBuf> {
        self.folders.iter().map(|f| f.1.clone()).collect()
    }

    fn has(&self, dir: &Path) -> bool {
        let key = path_key(dir);
        self.folders.iter().any(|f| path_key(&f.1) == key)
    }
}

/// A listed folder: `~/x` from home, an absolute path as it is, anything
/// else from the workspace file's folder.
fn resolve_entry(base: &Path, s: &str) -> PathBuf {
    let home = || crate::settings::home().unwrap_or_default();
    let p = if s == "~" {
        home()
    } else if let Some(r) = s.strip_prefix("~/") {
        home().join(r)
    } else if Path::new(s).is_absolute() {
        PathBuf::from(s)
    } else {
        base.join(s)
    };
    let p = absolute(&p);
    fs::canonicalize(&p).unwrap_or(p)
}

/// The file's JSON: comments and trailing commas allowed, and when that
/// doesn't read, JavaScript's bare keys and 'single quotes'.
fn parse_json(text: &str) -> Result<Value, String> {
    let read = |t: &str| serde_json::from_str::<Value>(&crate::settings::strip_jsonc(t));
    read(text)
        .or_else(|e| read(&relax(text)).map_err(|_| e))
        .map_err(|e| format!("not valid JSON ({})", e))
}

/// JavaScript object text as JSON: `'single quotes'` become `"double"` and
/// bare keys (`open:`) get quotes. "Strings" and comments stay as they are,
/// so valid JSON comes out unchanged.
fn relax(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '"' => {
                let start = i;
                i += 1;
                while i < c.len() && c[i] != '"' {
                    i += if c[i] == '\\' { 2 } else { 1 };
                }
                i = (i + 1).min(c.len());
                out.extend(&c[start..i]);
            }
            '\'' => {
                out.push('"');
                i += 1;
                while i < c.len() && c[i] != '\'' {
                    match c[i] {
                        '\\' if i + 1 < c.len() => {
                            if c[i + 1] != '\'' {
                                out.push('\\');
                            }
                            out.push(c[i + 1]);
                            i += 1;
                        }
                        '"' => out.push_str("\\\""),
                        ch => out.push(ch),
                    }
                    i += 1;
                }
                out.push('"');
                i += 1;
            }
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    out.push(c[i]);
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                let start = i;
                i += 2;
                while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i = (i + 2).min(c.len());
                out.extend(&c[start..i]);
            }
            ch if ch.is_alphabetic() || ch == '_' || ch == '$' => {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') {
                    i += 1;
                }
                let word: String = c[start..i].iter().collect();
                let mut j = i;
                while j < c.len() && c[j].is_whitespace() {
                    j += 1;
                }
                if c.get(j) == Some(&':') {
                    out.push('"');
                    out.push_str(&word);
                    out.push('"');
                } else {
                    out.push_str(&word);
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

fn parse(text: &str, base: &Path) -> Result<Listed, String> {
    let v = parse_json(text)?;
    let list = v
        .get("open")
        .and_then(Value::as_array)
        .ok_or("no \"open\" list of folders")?;
    let mut out = Listed::default();
    let mut seen = HashSet::new();
    for e in list {
        let s = e.as_str().ok_or("\"open\" should list folder paths, as strings")?;
        let p = resolve_entry(base, s);
        if !p.is_dir() {
            out.missing.push(s.to_string());
        } else if seen.insert(path_key(&p)) {
            out.folders.push((s.to_string(), p));
        }
    }
    Ok(out)
}

fn base_of(file: &Path) -> PathBuf {
    file.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn read_file(file: &Path) -> Result<Listed, String> {
    let text = fs::read_to_string(file).map_err(|e| e.to_string())?;
    parse(&text, &base_of(file))
}

/// `~/x` for paths in the home folder, else the absolute path: how a new
/// folder is written into the file, and how messages show paths.
fn tilde(p: &Path) -> String {
    match crate::settings::home().and_then(|h| p.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(r) if r.as_os_str().is_empty() => "~".into(),
        Some(r) => format!("~/{}", r.display()),
        None => p.display().to_string(),
    }
}

/// Skips whitespace and comments from `i`.
fn skip_ws(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else {
            return i;
        }
    }
}

/// One past the closing quote of the string that starts at `i`.
fn string_end(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// One past the end of the value that starts at `i`: a string, an object or
/// array with everything in it, or a bare word (number, `true`, …).
fn value_end(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        Some(b'"') => string_end(b, i),
        Some(b'[' | b'{') => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = string_end(b, j);
                        continue;
                    }
                    b'/' if matches!(b.get(j + 1), Some(b'/' | b'*')) => {
                        j = skip_ws(b, j);
                        continue;
                    }
                    b'[' | b'{' => depth += 1,
                    b']' | b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return j + 1;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            b.len()
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']') && !b[j].is_ascii_whitespace() {
                j += 1;
            }
            j
        }
    }
}

/// The whitespace that starts the line `i` is on.
fn line_indent(text: &str, i: usize) -> String {
    let start = text[..i].rfind('\n').map_or(0, |n| n + 1);
    text[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// `["a", "b"]` when `one_line` and it's short, else one entry per line,
/// indented one step past `indent`.
fn format_list(items: &[String], indent: &str, one_line: bool) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|s| serde_json::to_string(s).unwrap_or_default())
        .collect();
    if quoted.is_empty() {
        return "[]".into();
    }
    let flat = format!("[{}]", quoted.join(", "));
    if one_line && flat.len() <= 80 {
        return flat;
    }
    let step = if indent.is_empty() { "  " } else { indent };
    let rows: Vec<String> = quoted.iter().map(|q| format!("{}{}{}", indent, step, q)).collect();
    format!("[\n{}\n{}]", rows.join(",\n"), indent)
}

/// A new workspace file listing `items`.
fn new_file(items: &[String]) -> String {
    format!("{{\n  \"open\": {}\n}}\n", format_list(items, "  ", false))
}

/// The file's text with its `"open"` list set to `items`. Only that list is
/// rewritten (added first when there's none), so comments and other keys
/// stay. None when the text isn't a JSON object.
fn with_open(text: &str, items: &[String]) -> Option<String> {
    let b = text.as_bytes();
    let start = skip_ws(b, 0);
    if b.get(start) != Some(&b'{') {
        return None;
    }
    let mut i = skip_ws(b, start + 1);
    while b.get(i) == Some(&b'"') {
        let key_end = string_end(b, i);
        let key: Option<String> = serde_json::from_str(&text[i..key_end]).ok();
        let colon = skip_ws(b, key_end);
        if b.get(colon) != Some(&b':') {
            return None;
        }
        let v_start = skip_ws(b, colon + 1);
        let v_end = value_end(b, v_start);
        if key.as_deref() == Some("open") {
            let one_line = !text[v_start..v_end].contains('\n');
            let list = format_list(items, &line_indent(text, i), one_line);
            return Some(format!("{}{}{}", &text[..v_start], list, &text[v_end..]));
        }
        i = skip_ws(b, v_end);
        if b.get(i) != Some(&b',') {
            break;
        }
        i = skip_ws(b, i + 1);
    }
    let empty = b.get(skip_ws(b, start + 1)) == Some(&b'}');
    let list = format!(
        "\n  \"open\": {}{}",
        format_list(items, "  ", false),
        if empty { "\n" } else { "," }
    );
    Some(format!("{}{}{}", &text[..=start], list, &text[start + 1..]))
}

impl Editor {
    /// `ren name.workspace.json`, `:workspace open`: every folder the file
    /// lists opens as a project, with the tabs and the rest the workspace's
    /// session saved (when it has one).
    pub fn open_workspace(&mut self, file: &Path) {
        let file = fs::canonicalize(file).unwrap_or_else(|_| absolute(file));
        let name = workspace_name(&file);
        let listed = match read_file(&file) {
            Ok(l) if !l.folders.is_empty() => l,
            Ok(l) if l.missing.is_empty() => return self.broken_workspace(&file, "it lists no folders".into()),
            Ok(l) => {
                let why = format!("none of its folders exist ({})", l.missing.join(", "));
                return self.broken_workspace(&file, why);
            }
            Err(e) => return self.broken_workspace(&file, e),
        };
        let key = path_key(&file);
        if self.workspace.as_ref().is_some_and(|w| path_key(w) == key) {
            return self.open_missing(&name, &listed);
        }
        // What this window had stays saved as a session of its own.
        if !self.untouched() || self.workspace.is_some() {
            self.detach_session();
        }
        let id = self
            .sessions
            .iter()
            .find(|s| s.workspace.as_ref().is_some_and(|w| path_key(w) == key))
            .map(|s| s.id.clone());
        let elsewhere = id.is_none() && session::workspace_live(&file);
        self.workspace = Some(file.clone());
        let (_, tabs, unsaved) = self.open_folders_with(&listed.paths(), id.as_deref());
        // Counted from the file: some of its folders may have been open already.
        let projects = match listed.folders.len() {
            1 => "1 project".to_string(),
            n => format!("{} projects", n),
        };
        let mut msg = match tabs {
            Some(tabs) => {
                let tabs = match tabs {
                    0 => String::new(),
                    1 => ", 1 tab".into(),
                    n => format!(", {} tabs", n),
                };
                format!("Restored workspace {}: {}{}", name, projects, tabs)
            }
            None => format!("Opened workspace {}: {}", name, projects),
        };
        match unsaved {
            0 => {}
            1 => msg += " · 1 unsaved tab (not written to disk yet)",
            n => msg += &format!(" · {} unsaved tabs (not written to disk yet)", n),
        }
        if elsewhere {
            msg += " · it's open in another ren too, so its saved tabs stayed there";
        }
        if listed.missing.is_empty() {
            self.info(msg);
        } else {
            self.error(format!("{} · not found: {}", msg, listed.missing.join(", ")));
        }
    }

    /// `ren new.workspace.json` for a file that isn't there yet: its tab
    /// (unsaved) gets a starting point listing the folder ren started in.
    pub fn start_workspace_file(&mut self) {
        let b = self.buf();
        let Some(path) = b.path.clone() else { return };
        if !is_workspace_file(&path) || path.exists() || b.lines.len() > 1 || !b.lines[0].is_empty() {
            return;
        }
        let here = env::current_dir().map(|d| tilde(&d)).unwrap_or_else(|_| "~".into());
        let entry = serde_json::to_string(&here).unwrap_or_default();
        let text = format!(
            "{{\n  // Folders to open together: absolute, ~/… (home), or relative to this file.\n  \"open\": [\n    {}\n  ]\n}}\n",
            entry
        );
        let b = self.buf_mut();
        b.set_text(&text);
        b.touch();
        b.cy = 3;
        b.cx = b.lines[3].len();
        b.sync_want();
        self.focus = Focus::Editor;
        self.info(format!(
            "New workspace {}: list its folders, save (Ctrl+S), then :workspace open",
            workspace_name(&path)
        ));
    }

    /// A workspace file that can't be used opens as text, to fix it.
    fn broken_workspace(&mut self, file: &Path, why: String) {
        if file.is_file() {
            self.open_path(file);
        }
        self.error(format!("{}: {}", tilde(file), why));
    }

    /// `:workspace open` on the workspace that's open: the folders it lists
    /// that aren't open (added to the file since) open too.
    fn open_missing(&mut self, name: &str, listed: &Listed) {
        let open: HashSet<String> = self.project_roots().iter().map(|r| path_key(r)).collect();
        let mut added = Vec::new();
        for (_, dir) in &listed.folders {
            if !open.contains(&path_key(dir)) {
                self.add_parked(dir.clone());
                added.push(selector_label(dir));
            }
        }
        if added.is_empty() {
            self.info(format!("All of {}'s folders are open", name));
        } else {
            self.info(format!("{}: opened {}", name, added.join(", ")));
        }
    }

    /// `:workspace save [file]`: the open projects, in selector order, become
    /// the file's `"open"` list, and the window is tied to that file. Entries
    /// already in it keep how they were written; ones that aren't folders
    /// here stay at the end.
    fn save_workspace(&mut self, arg: Option<&str>) {
        let file = match arg {
            Some(a) => self.workspace_target(a),
            None => match &self.workspace {
                Some(w) => w.clone(),
                None => {
                    // Asks for a name, starting next to the project's folder.
                    let root = self.tree.root.clone();
                    let dir = root.parent().unwrap_or(&root);
                    let mut start = tilde(dir);
                    if !start.ends_with('/') {
                        start.push('/');
                    }
                    return self.prefill_cmd(&format!("workspace save {}", start));
                }
            },
        };
        let old = fs::read_to_string(&file).ok();
        let listed = match &old {
            Some(t) => match parse_json(t) {
                Ok(v) if v.is_object() => parse(t, &base_of(&file)).unwrap_or_default(),
                Ok(_) => return self.error(format!("{} isn't a JSON object", tilde(&file))),
                Err(e) => return self.error(format!("{} is {}: fix it first (:workspace edit)", tilde(&file), e)),
            },
            None => Listed::default(),
        };
        let mut items: Vec<String> = self
            .project_roots()
            .iter()
            .map(|r| {
                let key = path_key(r);
                listed
                    .folders
                    .iter()
                    .find(|f| path_key(&f.1) == key)
                    .map_or_else(|| tilde(r), |f| f.0.clone())
            })
            .collect();
        let n = items.len();
        items.extend(listed.missing.iter().cloned());
        let text = match &old {
            // Written back as JSON, when it was JavaScript-style.
            Some(t) => match with_open(&relax(t), &items) {
                Some(t) => t,
                None => return self.error(format!("{} isn't a JSON object", tilde(&file))),
            },
            None => new_file(&items),
        };
        let written = file
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::write(&file, text));
        if let Err(e) = written {
            return self.error(format!("Could not write {}: {}", tilde(&file), e));
        }
        let file = fs::canonicalize(&file).unwrap_or(file);
        let tied = self.workspace.as_ref().is_some_and(|w| path_key(w) == path_key(&file));
        self.workspace = Some(file.clone());
        let _ = self.save_session(false, true);
        self.reload_from_disk();
        let projects = if n == 1 {
            "1 project".into()
        } else {
            format!("{} projects", n)
        };
        let mut msg = format!("Saved {} to {}", projects, tilde(&file));
        if !listed.missing.is_empty() {
            msg += &format!(" (kept {}, not found here)", listed.missing.join(", "));
        }
        if !tied {
            msg += &format!(" · ren {} opens them", tilde(&file));
        }
        self.info(msg);
    }

    /// Where `:workspace save <path>` writes: `/x` is the project's x unless
    /// only a real `/x`'s folder exists (as for `:open`), and the name gets
    /// `.workspace.json` when it doesn't end with it.
    fn workspace_target(&self, a: &str) -> PathBuf {
        let mut p = self.resolve_path(a);
        let real = Path::new(a);
        let parent_ok = |p: &Path| p.parent().is_some_and(Path::is_dir);
        if a.starts_with('/') && !parent_ok(&p) && parent_ok(real) {
            p = absolute(real);
        }
        if !is_workspace_file(&p) {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().trim_end_matches(".json").to_string())
                .unwrap_or_default();
            p.set_file_name(format!("{}{}", name, SUFFIX));
        }
        p
    }

    /// `:workspace`: which file this window is tied to, and how the open
    /// projects differ from it.
    fn workspace_status(&mut self) {
        let Some(file) = self.workspace.clone() else {
            return self.info("No workspace: :workspace save <file> keeps the open projects in one");
        };
        let name = workspace_name(&file);
        let listed = match read_file(&file) {
            Ok(l) => l,
            Err(e) => return self.error(format!("Workspace {} ({}): {}", name, tilde(&file), e)),
        };
        let roots = self.project_roots();
        let extra: Vec<String> = roots
            .iter()
            .filter(|r| !listed.has(r))
            .map(|r| selector_label(r))
            .collect();
        let open: HashSet<String> = roots.iter().map(|r| path_key(r)).collect();
        let closed: Vec<String> = listed
            .folders
            .iter()
            .filter(|f| !open.contains(&path_key(&f.1)))
            .map(|f| selector_label(&f.1))
            .collect();
        let mut msg = format!(
            "Workspace {} · {} · {} folders",
            name,
            tilde(&file),
            listed.folders.len()
        );
        if !extra.is_empty() {
            msg += &format!(" · not in it: {}", extra.join(", "));
        }
        if !closed.is_empty() {
            msg += &format!(" · closed here: {}", closed.join(", "));
        }
        if !extra.is_empty() || !closed.is_empty() {
            msg += " (:workspace save writes the open projects)";
        }
        self.info(msg);
    }

    /// `:workspace` shows the workspace; `:workspace open [file]` opens one
    /// (no file: the tab's, or the folders added to this one's file);
    /// `:workspace save [file]` writes the open projects into it;
    /// `:workspace edit` opens the file in a tab.
    pub(crate) fn workspace_command(&mut self, rest: &[String]) {
        match rest.first().map(String::as_str) {
            None => self.workspace_status(),
            Some("open") => {
                let file = match rest.get(1) {
                    Some(a) => {
                        let p = self.resolve_path(a);
                        if a.starts_with('/') && !p.exists() && Path::new(a).exists() {
                            absolute(Path::new(a))
                        } else {
                            p
                        }
                    }
                    None => {
                        let tab = self.buf().path.clone().filter(|p| is_workspace_file(p));
                        match tab.or_else(|| self.workspace.clone()) {
                            Some(p) => p,
                            None => return self.error("Usage: :workspace open <name.workspace.json>"),
                        }
                    }
                };
                if !file.is_file() {
                    return self.error(format!("No such file: {}", tilde(&file)));
                }
                self.open_workspace(&file);
            }
            Some("save") => self.save_workspace(rest.get(1).map(String::as_str)),
            Some("edit") => match self.workspace.clone() {
                Some(w) => {
                    self.open_path(&w);
                    self.focus = Focus::Editor;
                }
                None => self.error("No workspace: :workspace save <file> makes one"),
            },
            Some(a) => self.error(format!(
                "Unknown :workspace {}: try :workspace, open [file], save [file] or edit",
                a
            )),
        }
    }

    /// After a project opens or closes in a workspace window: whether the
    /// file still says otherwise (empty when it agrees, or there's none).
    pub(crate) fn workspace_note(&self, root: &Path, opened: bool) -> String {
        let Some(file) = &self.workspace else {
            return String::new();
        };
        let Ok(listed) = read_file(file) else {
            return String::new();
        };
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match (opened, listed.has(root)) {
            (true, false) => format!(" · not in {} (:workspace save adds it)", name),
            (false, true) => format!(" · still in {} (:workspace save drops it)", name),
            _ => String::new(),
        }
    }

    /// The project list's workspace row: save the open projects into the
    /// file when they differ from it, or into a new one.
    pub(crate) fn workspace_menu_item(&self) -> Option<String> {
        match &self.workspace {
            Some(file) => {
                let listed = read_file(file).ok()?;
                let roots = self.project_roots();
                let same = roots.len() == listed.folders.len() && roots.iter().all(|r| listed.has(r));
                let name = file.file_name()?.to_string_lossy().into_owned();
                (!same).then(|| format!("◇  Save projects to {}", name))
            }
            None => (self.project_count() > 1).then(|| "◇  Save as workspace…".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(is_workspace_file(Path::new("/w/work.workspace.json")));
        assert!(is_workspace_file(Path::new("Team.Workspace.JSON")));
        assert!(!is_workspace_file(Path::new("/w/workspace.json")));
        assert!(!is_workspace_file(Path::new("/w/package.json")));
        assert_eq!(workspace_name(Path::new("/w/work.workspace.json")), "work");
        assert_eq!(workspace_name(Path::new("/w/.workspace.json")), "workspace");
    }

    #[test]
    fn lists_folders() {
        let dir = std::env::temp_dir().join(format!("ren-ws-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for d in ["a", "b", "ws"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        let dir = fs::canonicalize(&dir).unwrap();
        let base = dir.join("ws");
        let text = format!(
            "{{\n  // mine\n  \"open\": [\"{}\", \"../b\", \"../a\", \"../gone\",],\n}}",
            dir.join("a").display()
        );
        let l = parse(&text, &base).unwrap();
        // `../a` is the same folder as the absolute entry: listed once.
        assert_eq!(l.paths(), vec![dir.join("a"), dir.join("b")]);
        assert_eq!(l.folders[1].0, "../b");
        assert_eq!(l.missing, vec!["../gone".to_string()]);
        assert!(l.has(&dir.join("b")));
        assert!(parse("{}", &base).is_err());
        assert!(parse("{\"open\": [1]}", &base).is_err());
        assert!(parse("{\"open\": [", &base).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn javascript_style() {
        let js = "{open:['/a', \"/b\", 'it\\'s', 'say \"hi\"'], // team's\n x_1 : true, /* 'c' */ n: 1e5}";
        let v = parse_json(js).unwrap();
        assert_eq!(v["open"], serde_json::json!(["/a", "/b", "it's", "say \"hi\""]));
        assert_eq!(v["x_1"], serde_json::json!(true));
        // Valid JSON(C) comes through unchanged.
        let ok = "{\n  // team's repos\n  \"open\": [\"/a\", \"it's: x\"], /* k: 'v' */\n  \"n\": -1.5e3\n}";
        assert_eq!(relax(ok), ok);
        // Saving JavaScript-style text writes JSON, keeping the comment.
        let out = with_open(&relax("{ // mine\n open: ['/old'] }"), &["/new".to_string()]).unwrap();
        assert_eq!(out, "{ // mine\n \"open\": [\"/new\"] }");
        assert!(parse_json("{open: ['/a'").is_err());
    }

    #[test]
    fn rewrites_only_the_list() {
        let items = vec!["/w/a".to_string(), "~/b".to_string()];
        let text = "{\n  // folders\n  \"name\": \"x\", /* keep */\n  \"open\": [\n    \"/old\"\n  ],\n  \"other\": [1, {\"open\": 2}]\n}\n";
        let out = with_open(text, &items).unwrap();
        assert_eq!(
            out,
            "{\n  // folders\n  \"name\": \"x\", /* keep */\n  \"open\": [\n    \"/w/a\",\n    \"~/b\"\n  ],\n  \"other\": [1, {\"open\": 2}]\n}\n"
        );
        // A one-line list stays on one line.
        let out = with_open("{ \"open\": [\"/old\"] }", &items).unwrap();
        assert_eq!(out, "{ \"open\": [\"/w/a\", \"~/b\"] }");
        // No list yet: it goes first.
        let out = with_open("{\n  \"x\": 1\n}", &items[..1]).unwrap();
        assert_eq!(out, "{\n  \"open\": [\n    \"/w/a\"\n  ],\n  \"x\": 1\n}");
        let out = with_open("{}", &items[..1]).unwrap();
        assert_eq!(out, "{\n  \"open\": [\n    \"/w/a\"\n  ]\n}");
        assert_eq!(with_open("[1]", &items), None);
        let fresh = new_file(&items);
        assert_eq!(fresh, "{\n  \"open\": [\n    \"/w/a\",\n    \"~/b\"\n  ]\n}\n");
        let l = parse(&fresh, Path::new("/")).unwrap();
        assert_eq!(l.missing.len() + l.folders.len(), 2);
    }
}
