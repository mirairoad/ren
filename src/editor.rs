use crate::buffer::{cls, disp_col, idx_at_col, leading_ws, Buffer, Pos, TAB_W};
use crate::highlight;
use crate::lsp::{path_key, Lsp};
use crate::tree::Tree;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::cell::{Cell, RefCell};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[path = "debug_run.rs"]
pub(crate) mod debug_run;
#[path = "debug_ui.rs"]
mod debug_ui;
#[path = "files.rs"]
mod files;
#[path = "find.rs"]
mod find;
pub use find::{FindBar, Matcher};
#[path = "git_ui.rs"]
mod git_ui;
#[path = "ide.rs"]
mod ide;
#[path = "panel.rs"]
mod panel;
#[path = "peek.rs"]
mod peek;
#[path = "projects.rs"]
mod projects;
pub use projects::{selector_label, PROJECTS_TITLE};
#[path = "scan.rs"]
mod scan;
#[path = "session.rs"]
pub mod session;
#[path = "workspace.rs"]
mod workspace;
pub use ide::{Menu, Popup, MENU_ROWS, POPUP_ROWS};
pub use session::SHOWN;
pub use workspace::is_workspace_file;

pub const NONAME: &str = "[untitled]";
const HELP_NAME: &str = "[help]";
/// Rows taken by the tab bar: a padding row, then the labels.
pub const TAB_H: u16 = 2;

const FORMAT_TIMEOUT: Duration = Duration::from_millis(1500);

/// True when the terminal speaks the kitty keyboard protocol, so Ctrl+Esc and
/// Ctrl+` arrive as their own keys. Without it Ctrl+Esc is plain Esc, so plain
/// Esc opens the command line once there is nothing left to close; and Ctrl+`
/// sends the same byte as Ctrl+Space (NUL), which then toggles the panel
/// (completion stays on Ctrl+N).
pub static KITTY: AtomicBool = AtomicBool::new(false);

pub fn kitty() -> bool {
    KITTY.load(Ordering::Relaxed)
}

/// Set from the SIGHUP / SIGTERM handler: the terminal is gone (or someone
/// asked ren to stop), so main saves the session and leaves.
pub static HANGUP: AtomicBool = AtomicBool::new(false);

pub fn hangup() -> bool {
    HANGUP.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormatOutcome {
    Skipped,
    Unchanged,
    Formatted,
    TimedOut,
}

impl FormatOutcome {
    fn note(self) -> &'static str {
        match self {
            FormatOutcome::Formatted => ", formatted",
            FormatOutcome::TimedOut => ", format timed out",
            _ => "",
        }
    }
}

pub fn help_text() -> &'static str {
    include_str!("help.txt")
}

/// Two modes: typing in a file (Edit) and the `:` line (Command).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Edit,
    Command,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    /// The side panel: the file tree, or the git panel while `:git` is on.
    Tree,
    Find,
    Panel,
    /// The side-by-side diff of the git panel.
    Diff,
}

/// Tabs of the bottom panel (Ctrl+`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelTab {
    Terminal,
    Debug,
    Containers,
}

#[derive(Clone)]
pub enum Reg {
    Chars(Vec<char>),
    Lines(Vec<Vec<char>>),
}

/// Consecutive edits of the same kind share one undo step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Type,
    Delete,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Text,
    Sep,
    DbgSep,
}

/// Tab completion on the `:` line: the matches, and which one is filled in.
pub struct CmdComp {
    /// Command text before the word being completed.
    pub head: String,
    /// Text after the cursor.
    pub rest: String,
    /// (shown in the list, what goes into the line). Folders end with `/`.
    pub items: Vec<(String, String)>,
    pub sel: Option<usize>,
    /// What's listed: "commands", "in /src/", or why nothing is.
    pub title: String,
}

/// Command names Tab completes (the main spelling of each).
const COMMANDS: &[&str] = &[
    "actions",
    "bin",
    "break",
    "breakclear",
    "cd",
    "containers",
    "cp",
    "debug",
    "def",
    "diag",
    "duplicate",
    "find",
    "format",
    "git",
    "help",
    "hints",
    "hover",
    "icons",
    "install",
    "lsp",
    "mkdir",
    "mkfile",
    "mod",
    "mv",
    "open",
    "panel",
    "project",
    "preview",
    "pwd",
    "q",
    "q!",
    "qa",
    "qa!",
    "recover",
    "refs",
    "reload",
    "rename",
    "replace",
    "revert",
    "rm",
    "save",
    "scan",
    "session",
    "settings",
    "symbols",
    "terminal",
    "touch",
    "tree",
    "uninstall",
    "unwatch",
    "w",
    "watch",
    "workspace",
    "wq",
    "wsymbols",
];

/// Fixed words a command takes as its first argument (paths are offered when
/// none match).
fn arg_words(cmd: &str) -> &'static [&'static str] {
    match cmd {
        "hints" => &["on", "off", "types", "params", "returns", "chaining", "braces"],
        "lsp" => &["restart", "stop"],
        "settings" => &["project"],
        "terminal" | "term" => &["new", "close"],
        "containers" => &["start", "stop", "restart", "shell", "refresh"],
        "debug" => &["config", "stop", "restart", "args", "program"],
        "recover" => &["discard"],
        "session" | "sessions" => &["forget", "list"],
        "project" | "proj" => &["close", "list", "open"],
        "workspace" | "ws" => &["edit", "open", "save"],
        "install" | "uninstall" => &["codelldb", "debugpy", "delve"],
        _ => &[],
    }
}

pub struct Layout {
    pub tree_w: u16,
    pub ed_x: u16,
    pub gutter: u16,
    pub text_x: u16,
    pub text_w: u16,
    pub top: u16,
    pub text_h: u16,
    pub ed_w: u16,
    pub ed_r: u16,
    /// Height of the side panel (file tree / git): it runs past the bottom panel.
    pub side_h: u16,
    /// The explorer's list: its first row (the search box is the row above)
    /// and how many rows it has.
    pub tree_top: u16,
    pub tree_h: u16,
    /// The bottom panel sits under the editor only: from `dbg_x`, `dbg_w` wide.
    pub dbg_x: u16,
    pub dbg_w: u16,
    pub dbg_y: u16,
    pub dbg_h: u16,
    /// Debug tab column widths, in panel coordinates (x from 0 at `dbg_x`).
    pub dbg_cols: (u16, u16, u16),
}

pub struct Editor {
    pub bufs: Vec<Buffer>,
    pub cur: usize,
    pub tree: Tree,
    pub focus: Focus,
    pub mode: Mode,
    pub cmd: Vec<char>,
    pub cmd_x: usize,
    pub msg: Option<(String, bool)>,
    pub reg: Reg,
    pub search: Option<Matcher>,
    pub hl_search: bool,
    pub numbers: bool,
    pub w: u16,
    pub h: u16,
    pub quit: bool,
    pub burst: bool,
    /// Selection anchor; the selection runs from here to the cursor.
    pub sel: Option<Pos>,
    /// Tabs on screen: (start x, end x, buffer, x of the icon / close slot).
    pub tab_spans: Vec<(u16, u16, usize, u16)>,
    /// The tab under the mouse: its icon turns into a close ×.
    pub tab_hover: Option<usize>,
    /// Dim text shown in the empty `:` line, e.g. what to type next.
    pub cmd_hint: Option<String>,
    /// Tab completion on the `:` line, shown as a list above the bar.
    pub cmd_comp: Option<CmdComp>,
    /// Counts the times a language server finished its background work.
    pub(crate) lsp_idle_gen: u64,
    /// The mouse resting on text: the spot, its screen cell, since when.
    pub(crate) rest: Option<(Pos, (u16, u16), Instant)>,
    /// The box that opens when the mouse rests on code (`peek.rs`).
    pub peek: Option<peek::Peek>,
    pub peek_rect: Cell<Option<(u16, u16, u16, u16)>>,
    /// Screen rows of the box's quick fixes: (y, fix).
    pub peek_rows: RefCell<Vec<(u16, usize)>>,
    peek_seq: u64,
    /// Problems in files that aren't open (see scan.rs).
    scan: scan::Scan,
    /// Set by horizontal wheel scrolling: the view no longer follows the cursor
    /// until the next key or click.
    hscroll_free: bool,
    last_edit: Option<EditKind>,
    history: Vec<String>,
    hist_i: usize,
    last_click: Option<(Instant, u16, u16)>,
    clicks: u8,
    drag: Drag,
    drag_anchor: Pos,
    pub lsp: Lsp,
    pub popup: Option<Popup>,
    pub popup_rect: Cell<Option<(u16, u16, u16, u16)>>,
    pub menu: Option<Menu>,
    pub menu_rect: Cell<Option<(u16, u16, u16, u16)>>,
    pub hover: Option<Vec<String>>,
    pub sig: Option<(String, Option<(usize, usize)>)>,
    comp_id: Option<i64>,
    menu_anchor: Option<(u16, u16)>,
    ctx_pos: Option<(u16, u16)>,
    jumps: Vec<(PathBuf, Pos)>,
    clip: Option<arboard::Clipboard>,
    clip_last: String,
    pub recent: Vec<PathBuf>,
    pub find: Option<FindBar>,
    pub find_hits: RefCell<Vec<(u16, u16, u16, find::FindHit)>>,
    pub find_cursor: Cell<Option<(u16, u16)>>,
    find_cache: RefCell<find::MatchCache>,
    ws_sym_id: Option<i64>,
    pub dbg: debug_ui::Debugger,
    pub dbg_hits: RefCell<Vec<(u16, u16, u16, debug_ui::DbgHit)>>,
    pub dbg_cursor: Cell<Option<(u16, u16)>>,
    pub splash_hits: RefCell<Vec<(u16, usize)>>,
    pub settings: crate::settings::Settings,
    pub bin: Option<crate::fileops::Bin>,
    pub panel_tab: PanelTab,
    /// Set by `:reload`: main restarts ren after the loop ends.
    pub reload: bool,
    autosave_at: Instant,
    /// What the session file holds, to write it only when something changed.
    autosave_sig: u64,
    /// This ren's session folder in `~/.ren/sessions/` (`session.rs`), and the
    /// lock that tells other rens it's still open.
    session_id: String,
    session_lock: Option<fs::File>,
    /// The session's latest checkpoint: what was open at the last write (a
    /// hash), since when (unix secs), and how many tabs had unsaved text.
    ckpt_shape: u64,
    ckpt_since: u64,
    ckpt_unsaved: usize,
    /// Saved sessions of earlier runs, for the welcome screen and `:session`.
    pub sessions: Vec<session::SessionInfo>,
    /// The workspace file this window was opened from or saved into
    /// (`workspace.rs`); its session is tagged with it.
    pub(crate) workspace: Option<PathBuf>,
    gitignore_asked: bool,
    /// Code-action requests still out, and whether their menu is showing.
    ca_pending: usize,
    ca_menu_open: bool,
    pub terms: Vec<crate::term::Session>,
    pub term_sel: usize,
    /// The terminal tab under the mouse (shows its × button).
    pub term_hover: Option<usize>,
    pub term_err: Option<String>,
    pub ctr: crate::containers::Containers,
    pub git: Option<git_ui::GitView>,
    pub commit: Option<git_ui::CommitForm>,
    pub commit_cursor: Cell<Option<(u16, u16)>>,
    last_template: usize,
    /// Open projects in selector order; the one on screen is `None` here (its
    /// state is in the fields above), the others are parked (`projects.rs`).
    projects: Vec<Option<Box<projects::Project>>>,
    /// The project on screen.
    proj: usize,
    /// The project selector in the tab bar: start and end x.
    pub proj_span: (u16, u16),
    /// The rendered Markdown of the tab on screen, for (tab, path, rev, width).
    md_cache: RefCell<Option<(MdKey, Vec<crate::markdown::Line>)>>,
    /// Columns of the status bar's Preview / Raw switch, while it's shown.
    pub md_chip: Cell<Option<(u16, u16)>>,
    /// The name under the mouse while Ctrl is held, drawn as a link (VS
    /// Code's go-to-definition look): (tab, line, start, end).
    pub link: Option<(usize, usize, usize, usize)>,
}

pub const SPLASH_OPEN: usize = 1000;
pub const SPLASH_HELP: usize = 1001;
/// Welcome-screen sessions: this plus the session's place in the list.
pub const SPLASH_SESSION: usize = 2000;

fn is_ctrl(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL) && !k.modifiers.contains(KeyModifiers::ALT)
}

pub fn absolute(p: &Path) -> PathBuf {
    let base = if p.is_absolute() {
        PathBuf::new()
    } else {
        env::current_dir().unwrap_or_default()
    };
    let mut out = base;
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl Editor {
    pub fn new(arg: Option<String>, w: u16, h: u16) -> Self {
        let mut bufs = Vec::new();
        let mut tree_visible = arg.is_some();
        let mut msg = None;
        let mut opened_dir = false;
        if let Some(a) = arg {
            let p = PathBuf::from(&a);
            if p.is_dir() {
                let _ = env::set_current_dir(&p);
                opened_dir = true;
            } else {
                // Through symlinks to the real path, as the tree's root (the
                // working directory) is: otherwise `/tmp/x/a.rs` and the tree's
                // `/private/tmp/x/a.rs` are two files to ren and its servers.
                let abs = absolute(&p);
                let abs = match (abs.parent().map(fs::canonicalize), abs.file_name()) {
                    (Some(Ok(dir)), Some(name)) => dir.join(name),
                    _ => abs,
                };
                match Buffer::open(&abs) {
                    Ok(b) => {
                        if !abs.exists() {
                            msg = Some((format!("\"{}\" [new file]", a), false));
                        }
                        bufs.push(b);
                    }
                    Err(e) => msg = Some((format!("{}: {}", a, e), true)),
                }
                tree_visible = false;
            }
        }
        if bufs.is_empty() {
            bufs.push(Buffer::scratch(NONAME));
        }
        let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut tree = Tree::new(root.clone());
        tree.visible = tree_visible;
        let recent: Vec<PathBuf> = crate::config::get_list("recent")
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect();
        let mut ed = Editor {
            bufs,
            cur: 0,
            tree,
            focus: Focus::Editor,
            mode: Mode::Edit,
            cmd: Vec::new(),
            cmd_x: 0,
            msg,
            reg: Reg::Chars(Vec::new()),
            search: None,
            hl_search: false,
            numbers: true,
            w,
            h,
            quit: false,
            burst: false,
            sel: None,
            tab_spans: Vec::new(),
            tab_hover: None,
            cmd_hint: None,
            cmd_comp: None,
            lsp_idle_gen: 0,
            rest: None,
            peek: None,
            peek_rect: Cell::new(None),
            peek_rows: RefCell::new(Vec::new()),
            peek_seq: 0,
            scan: Default::default(),
            hscroll_free: false,
            last_edit: None,
            history: Vec::new(),
            hist_i: 0,
            last_click: None,
            clicks: 0,
            drag: Drag::None,
            drag_anchor: (0, 0),
            lsp: Lsp::new(),
            popup: None,
            popup_rect: Cell::new(None),
            menu: None,
            menu_rect: Cell::new(None),
            hover: None,
            sig: None,
            comp_id: None,
            menu_anchor: None,
            ctx_pos: None,
            jumps: Vec::new(),
            clip: arboard::Clipboard::new().ok(),
            clip_last: String::new(),
            recent,
            splash_hits: RefCell::new(Vec::new()),
            find: None,
            find_hits: RefCell::new(Vec::new()),
            find_cursor: Cell::new(None),
            find_cache: RefCell::new(None),
            ws_sym_id: None,
            dbg: debug_ui::Debugger::new(),
            dbg_hits: RefCell::new(Vec::new()),
            dbg_cursor: Cell::new(None),
            settings: crate::settings::Settings::default(),
            bin: None,
            panel_tab: PanelTab::Debug,
            reload: false,
            autosave_at: Instant::now(),
            autosave_sig: 0,
            session_id: session::new_id(),
            session_lock: None,
            ckpt_shape: 0,
            ckpt_since: 0,
            ckpt_unsaved: 0,
            sessions: Vec::new(),
            workspace: None,
            gitignore_asked: false,
            ca_pending: 0,
            ca_menu_open: false,
            terms: Vec::new(),
            term_sel: 0,
            term_hover: None,
            term_err: None,
            ctr: crate::containers::Containers::new(),
            git: None,
            commit: None,
            commit_cursor: Cell::new(None),
            last_template: 0,
            projects: vec![None],
            proj: 0,
            proj_span: (0, 0),
            md_cache: RefCell::new(None),
            md_chip: Cell::new(None),
            link: None,
        };
        ed.sessions = session::list_sessions(&ed.session_id);
        let errs = ed.load_settings();
        if !errs.is_empty() && ed.msg.is_none() {
            ed.error(errs.join("; "));
        }
        if opened_dir {
            ed.push_recent(&root);
            ed.focus = Focus::Tree;
        }
        ed
    }

    pub(crate) fn load_settings(&mut self) -> Vec<String> {
        let _ = crate::settings::ensure();
        let (s, errs) = crate::settings::load(Some(&self.tree.root));
        crate::render::set_primary(s.primary);
        self.settings = s;
        errs
    }

    fn is_settings_file(&self, i: usize) -> bool {
        let Some(a) = &self.bufs[i].path else { return false };
        let global = crate::settings::path().is_some_and(|b| path_key(a) == path_key(&b));
        global || path_key(a) == path_key(&crate::settings::project_path(&self.tree.root))
    }

    /// `:settings` opens ~/.ren/settings.json; `:settings project` the project's .ren/settings.json.
    fn open_settings(&mut self, arg: &str) {
        let p = if arg == "project" {
            match crate::settings::ensure_project(&self.tree.root) {
                Ok(p) => p,
                Err(e) => return self.error(format!("Could not create project settings: {}", e)),
            }
        } else {
            if let Err(e) = crate::settings::ensure() {
                return self.error(format!("Could not create settings: {}", e));
            }
            match crate::settings::path() {
                Some(p) => p,
                None => return self.error("No home directory"),
            }
        };
        self.open_path(&p);
        self.focus = Focus::Editor;
    }

    pub fn panel_tab_name(&self) -> &'static str {
        match self.panel_tab {
            PanelTab::Terminal => "terminal",
            PanelTab::Debug => "debug",
            PanelTab::Containers => "containers",
        }
    }

    pub fn set_panel_tab_name(&mut self, s: &str) {
        self.panel_tab = match s {
            "terminal" => PanelTab::Terminal,
            "containers" => PanelTab::Containers,
            _ => PanelTab::Debug,
        };
    }

    pub(crate) fn push_recent(&mut self, p: &Path) {
        let key = path_key(p);
        self.recent.retain(|r| path_key(r) != key);
        self.recent.insert(0, p.to_path_buf());
        self.recent.truncate(9);
        let joined = self
            .recent
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join("|");
        let _ = crate::config::set("recent", &joined);
    }

    pub fn buf(&self) -> &Buffer {
        &self.bufs[self.cur]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        &mut self.bufs[self.cur]
    }

    fn info(&mut self, s: impl Into<String>) {
        self.msg = Some((s.into(), false));
    }

    fn error(&mut self, s: impl Into<String>) {
        self.msg = Some((s.into(), true));
    }

    pub fn layout(&self) -> Layout {
        let tree_w = if self.tree.visible {
            self.tree.width.min(self.w.saturating_sub(20))
        } else {
            0
        };
        let ed_x = if self.tree.visible { tree_w + 1 } else { 0 };
        let b = self.buf();
        let gutter = if self.numbers {
            (b.lines.len().to_string().len().max(3) + 3) as u16
        } else {
            2
        };
        let text_x = ed_x + gutter;
        let ed_r = self.w;
        let text_w = ed_r.saturating_sub(text_x).max(1);
        // Rows: tabs on top, the bar at the bottom.
        let total_h = self.h.saturating_sub(TAB_H + 1).max(1);
        let dbg_h = if self.dbg.visible && total_h >= 14 {
            self.dbg.height.clamp(6, total_h - 6)
        } else {
            0
        };
        let text_h = total_h - dbg_h;
        let dbg_y = TAB_H + text_h;
        let dbg_x = ed_x;
        let dbg_w = self.w.saturating_sub(dbg_x).max(1);
        let vw = dbg_w * 40 / 100;
        let sw = dbg_w * 25 / 100;
        let dbg_cols = (vw, sw, dbg_w.saturating_sub(vw + sw));
        let ed_w = ed_r.saturating_sub(ed_x);
        Layout {
            tree_w,
            ed_x,
            gutter,
            text_x,
            text_w,
            top: TAB_H,
            text_h,
            ed_w,
            ed_r,
            side_h: total_h,
            tree_top: TAB_H + 1,
            tree_h: total_h.saturating_sub(1),
            dbg_x,
            dbg_w,
            dbg_y,
            dbg_h,
            dbg_cols,
        }
    }

    /// The git diff takes the editor area while the git panel or the diff has focus.
    pub fn diff_shown(&self) -> bool {
        self.git_active()
            && matches!(self.focus, Focus::Tree | Focus::Diff)
            && self.git.as_ref().is_some_and(|g| g.diff.is_some())
    }

    pub fn is_splash(&self) -> bool {
        self.bufs.len() == 1 && self.is_pristine(0)
    }

    fn is_pristine(&self, i: usize) -> bool {
        let b = &self.bufs[i];
        b.path.is_none() && !b.dirty && b.name == NONAME && b.lines.len() == 1 && b.lines[0].is_empty()
    }

    pub fn rel_path(&self, b: &Buffer) -> String {
        match &b.path {
            Some(p) => p.strip_prefix(&self.tree.root).unwrap_or(p).display().to_string(),
            None => b.name.clone(),
        }
    }

    fn cursor(&self) -> Pos {
        (self.buf().cx, self.buf().cy)
    }

    /// The selected range, start first, end exclusive. None when nothing is selected.
    pub fn sel_range(&self) -> Option<(Pos, Pos)> {
        let a = self.sel?;
        let c = self.cursor();
        if a == c {
            return None;
        }
        Some(if (a.1, a.0) <= (c.1, c.0) { (a, c) } else { (c, a) })
    }

    fn selected_text(&self) -> Option<Vec<char>> {
        let (a, e) = self.sel_range()?;
        Some(self.buf().get_text(a, e))
    }

    pub(crate) fn set_reg(&mut self, r: Reg) {
        let text = match &r {
            Reg::Chars(c) => c.iter().collect::<String>(),
            Reg::Lines(ls) => ls.iter().map(|l| l.iter().collect::<String>() + "\n").collect(),
        };
        if let Some(c) = self.clip.as_mut() {
            if c.set_text(text.clone()).is_ok() {
                self.clip_last = text;
            }
        }
        self.reg = r;
    }

    fn sync_from_clip(&mut self) {
        let Some(c) = self.clip.as_mut() else { return };
        let Ok(t) = c.get_text() else { return };
        let t = t.replace("\r\n", "\n");
        if t.is_empty() || t == self.clip_last.replace("\r\n", "\n") {
            return;
        }
        self.clip_last = t.clone();
        self.reg = Reg::Chars(t.chars().collect());
    }

    fn indent_unit(&self) -> Vec<char> {
        if self.buf().use_tabs {
            vec!['\t']
        } else {
            vec![' '; TAB_W]
        }
    }

    pub fn handle(&mut self, ev: Event) {
        match ev {
            Event::Key(k) => {
                if k.kind == KeyEventKind::Release {
                    return;
                }
                self.link = None;
                self.hscroll_free = false;
                self.on_key(k);
                self.after(3);
            }
            Event::Mouse(m) => {
                self.on_mouse(m);
                self.after(0);
            }
            Event::Paste(s) => {
                self.hscroll_free = false;
                self.on_paste(&s);
                self.after(3);
            }
            Event::Resize(w, h) => {
                self.w = w;
                self.h = h;
                self.after(0);
            }
            _ => {}
        }
    }

    fn after(&mut self, so: usize) {
        if self.bufs.is_empty() {
            self.bufs.push(Buffer::scratch(NONAME));
        }
        self.cur = self.cur.min(self.bufs.len() - 1);
        self.buf_mut().clamp(true);
        if let Some(a) = self.sel {
            let b = self.buf();
            let y = a.1.min(b.last());
            self.sel = Some((a.0.min(b.line_len(y)), y));
        }
        self.scroll_to_cursor(so);
        let rows = self.tree_rows();
        let t = &mut self.tree;
        t.sel = t.sel.min(t.nodes.len().saturating_sub(1));
        t.scroll = t.scroll.min(t.nodes.len().saturating_sub(rows.max(1)));
    }

    fn scroll_to_cursor(&mut self, so: usize) {
        let l = self.layout();
        let h = l.text_h as usize;
        let w = l.text_w as usize;
        let so = so.min(h.saturating_sub(1) / 2);
        let free = self.hscroll_free;
        let b = self.buf_mut();
        if b.cy < b.scroll_y + so {
            b.scroll_y = b.cy.saturating_sub(so);
        } else if b.cy + so >= b.scroll_y + h {
            b.scroll_y = b.cy + so + 1 - h;
        }
        if free {
            return;
        }
        let dc = b.vcol(b.cy, b.cx);
        if dc < b.scroll_x {
            b.scroll_x = dc;
        } else if dc >= b.scroll_x + w {
            b.scroll_x = dc + 1 - w;
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.mode != Mode::Command {
            self.msg = None;
        }
        self.peek = None;
        self.rest = None;
        let ctrl = is_ctrl(&k);
        // Ctrl+Esc swaps between Edit and Command from anywhere.
        if k.code == KeyCode::Esc && ctrl {
            self.menu = None;
            self.hover = None;
            if self.mode == Mode::Command {
                self.mode = Mode::Edit;
            } else {
                self.start_cmd();
            }
            return;
        }
        if self.hover.take().is_some() && k.code == KeyCode::Esc {
            return;
        }
        if self.menu.is_some() {
            return self.menu_key(k);
        }
        if self.mode == Mode::Command {
            return self.cmd_key(k);
        }
        if self.commit.is_some() {
            return self.commit_key(k);
        }
        if let KeyCode::F(n) = k.code {
            let shift = k.modifiers.contains(KeyModifiers::SHIFT);
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match (n, shift, ctrl) {
                (5, true, true) => return self.debug_restart(),
                (5, true, false) => return self.debug_stop(),
                (5, false, _) => return self.debug_start(),
                (6, _, _) => return self.debug_step("pause"),
                (9, _, _) => {
                    let y = self.buf().cy;
                    return self.toggle_breakpoint(y);
                }
                (10, _, _) => return self.debug_step("next"),
                (11, true, _) => return self.debug_step("stepOut"),
                (11, false, _) => return self.debug_step("stepIn"),
                _ => {}
            }
        }
        if self.focus == Focus::Panel && self.panel_key(k) {
            return;
        }
        if self.focus == Focus::Find {
            let global = ctrl
                && matches!(
                    k.code,
                    KeyCode::Char('w')
                        | KeyCode::Char('b')
                        | KeyCode::Char('k')
                        | KeyCode::Char('s')
                        | KeyCode::Char('z')
                        | KeyCode::Char('y')
                        | KeyCode::Char('t')
                );
            if !global {
                return self.find_key(k);
            }
        }
        if ctrl && self.ctrl_key(k) {
            return;
        }
        match self.focus {
            Focus::Tree if self.git_active() => self.git_key(k),
            Focus::Tree => self.tree_key(k),
            Focus::Diff if self.git_active() => self.diff_key(k),
            Focus::Diff => self.focus = Focus::Editor,
            _ if self.buf().md_view => self.md_key(k),
            _ => self.edit_key(k),
        }
    }

    /// Global Ctrl shortcuts. Returns true when the key was handled.
    fn ctrl_key(&mut self, k: KeyEvent) -> bool {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        // A rendered Markdown view takes no edits.
        let in_editor = self.focus == Focus::Editor && !self.buf().md_view;
        match k.code {
            KeyCode::Char('s') => {
                self.save(None);
            }
            KeyCode::Char('b') => self.toggle_tree(),
            KeyCode::Char('e') => self.tree_search(),
            KeyCode::Char('k') => {
                let d = self.tree.root.clone();
                self.folder_picker(d, None, false);
            }
            KeyCode::Char('f') => self.open_find(false),
            KeyCode::Char('h') => self.open_find(true),
            KeyCode::Char('z') if self.focus == Focus::Tree && shift => self.file_redo(),
            KeyCode::Char('Z') | KeyCode::Char('y') if self.focus == Focus::Tree => self.file_redo(),
            KeyCode::Char('z') if self.focus == Focus::Tree => self.file_undo(),
            KeyCode::Char('z') if shift => self.redo_any(),
            KeyCode::Char('Z') | KeyCode::Char('y') => self.redo_any(),
            KeyCode::Char('z') => self.undo_any(),
            KeyCode::Char('t') => self.workspace_symbols(),
            KeyCode::Char('r') => self.project_menu(),
            KeyCode::Char('O') => self.lsp_document_symbols(),
            KeyCode::Char('o') if shift => self.lsp_document_symbols(),
            KeyCode::Char('o') => self.jump_back(),
            KeyCode::Char('g') => self.start_cmd(),
            KeyCode::Char('.') => {
                self.enter_edit();
                self.lsp_code_actions();
            }
            KeyCode::Char('`') => self.toggle_panel(),
            // Without kitty keys Ctrl+` arrives as Ctrl+Space (both send NUL),
            // e.g. Windows Terminal under WSL: it's the panel's key then.
            KeyCode::Char(' ') | KeyCode::Null if !kitty() => self.toggle_panel(),
            KeyCode::Char('w') => {
                self.enter_edit();
                self.focus = match self.focus {
                    Focus::Editor if self.tree.visible => Focus::Tree,
                    Focus::Editor | Focus::Tree if self.dbg.visible => Focus::Panel,
                    _ => Focus::Editor,
                };
            }
            KeyCode::PageDown => self.cycle_buf(1),
            KeyCode::PageUp => self.cycle_buf(-1),
            // Ctrl+/ arrives as Ctrl+7 (0x1F) without kitty keys, Ctrl+_ on some terminals.
            KeyCode::Char('/') | KeyCode::Char('7') | KeyCode::Char('_') if in_editor => self.toggle_comment(),
            KeyCode::Char('a') if in_editor => self.select_all(),
            KeyCode::Char('c') if in_editor => self.copy(),
            KeyCode::Char('x') if in_editor => self.cut(),
            // Ctrl+Shift+V on a Markdown file: rendered / raw (as in VS Code).
            KeyCode::Char('V') | KeyCode::Char('v')
                if (shift || k.code == KeyCode::Char('V'))
                    && self.focus == Focus::Editor
                    && self.buf().lang.name == "Markdown" =>
            {
                self.toggle_md_view()
            }
            KeyCode::Char('v') if in_editor => self.paste(),
            KeyCode::Char(' ') | KeyCode::Null if in_editor => self.trigger_completion(true, None),
            KeyCode::Char('n') | KeyCode::Char('p') if in_editor && self.popup.is_none() => {
                self.trigger_completion(true, None)
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn enter_edit(&mut self) {
        self.mode = Mode::Edit;
        self.popup = None;
        self.sig = None;
        self.comp_id = None;
    }

    fn toggle_tree(&mut self) {
        self.enter_edit();
        self.tree.visible = !self.tree.visible;
        self.focus = if self.tree.visible { Focus::Tree } else { Focus::Editor };
        if self.tree.visible {
            self.tree.refresh();
        }
    }

    /// Ctrl+`: shows the bottom panel (focused) or hides it.
    fn toggle_panel(&mut self) {
        if self.dbg.visible && self.focus != Focus::Panel {
            self.focus = Focus::Panel;
            return;
        }
        self.dbg.visible = !self.dbg.visible;
        self.focus = if self.dbg.visible { Focus::Panel } else { Focus::Editor };
    }

    fn tree_rows(&self) -> usize {
        self.layout().tree_h as usize
    }

    fn tree_follow(&mut self) {
        let r = self.tree_rows().max(1);
        let t = &mut self.tree;
        if t.sel < t.scroll {
            t.scroll = t.sel;
        } else if t.sel >= t.scroll + r {
            t.scroll = t.sel + 1 - r;
        }
    }

    /// The explorer's search box has the keys: typing filters the tree,
    /// ↑/↓ move through what it found, Enter opens it, Esc empties the box
    /// (again: back to the list). Other keys go to the list.
    fn tree_search_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let mut q = self.tree.query.clone();
        match k.code {
            KeyCode::Char('u') if ctrl => q.clear(),
            KeyCode::Char(c) if !ctrl => q.push(c),
            KeyCode::Backspace => {
                q.pop();
            }
            KeyCode::Esc if !q.is_empty() => q.clear(),
            KeyCode::Esc | KeyCode::Tab => {
                self.tree.typing = false;
                return true;
            }
            KeyCode::Enter => {
                if let Some(n) = self.tree.nodes.get(self.tree.sel) {
                    let i = self.tree.sel;
                    if n.is_dir {
                        // A folder: out of the search, to that folder.
                        let p = n.path.clone();
                        self.tree.typing = false;
                        self.tree.set_query("");
                        self.tree.reveal(&p);
                        self.tree_follow();
                    } else {
                        self.tree_activate(i);
                    }
                }
                return true;
            }
            _ => return false,
        }
        self.tree.set_query(&q);
        self.tree_follow();
        true
    }

    /// Ctrl+E, a click on the search box: the explorer shown, its search box
    /// taking the keys.
    pub(crate) fn tree_search(&mut self) {
        self.enter_edit();
        if !self.tree.visible || self.git_active() {
            self.git = None;
            self.tree.visible = true;
        }
        self.focus = Focus::Tree;
        self.tree.typing = true;
    }

    fn tree_key(&mut self, k: KeyEvent) {
        if self.tree.typing && self.tree_search_key(k) {
            return;
        }
        let n = self.tree.nodes.len();
        let page = self.tree_rows().max(1);
        match k.code {
            KeyCode::Down => {
                if self.tree.sel + 1 < n {
                    self.tree.sel += 1;
                }
            }
            KeyCode::Up => self.tree.sel = self.tree.sel.saturating_sub(1),
            KeyCode::PageDown => self.tree.sel = (self.tree.sel + page).min(n.saturating_sub(1)),
            KeyCode::PageUp => self.tree.sel = self.tree.sel.saturating_sub(page),
            KeyCode::Home => self.tree.sel = 0,
            KeyCode::End => self.tree.sel = n.saturating_sub(1),
            KeyCode::Char(' ') => self.tree_preview(self.tree.sel),
            KeyCode::Enter => self.tree_activate(self.tree.sel),
            KeyCode::Right => {
                let i = self.tree.sel;
                if i < n && self.tree.nodes[i].is_dir && !self.tree.nodes[i].expanded {
                    self.tree.toggle(i);
                }
            }
            KeyCode::Left => {
                let i = self.tree.sel;
                if i < n {
                    if self.tree.nodes[i].is_dir && self.tree.nodes[i].expanded {
                        self.tree.toggle(i);
                    } else if let Some(p) = self.tree.parent_of(i) {
                        self.tree.sel = p;
                    }
                }
            }
            KeyCode::Delete => {
                if let Some(p) = self.tree_sel_rel() {
                    self.prefill_cmd(&format!("rm {}", quote_arg(&p)));
                }
            }
            KeyCode::F(2) => {
                if let Some(p) = self.tree_sel_rel() {
                    let q = quote_arg(&p);
                    self.prefill_cmd(&format!("mv {} {}", q, q));
                }
            }
            KeyCode::Char(':') => self.start_cmd(),
            KeyCode::Esc if self.tree.searching() => self.tree.set_query(""),
            KeyCode::Esc => {
                if !kitty() {
                    self.start_cmd();
                }
            }
            KeyCode::Tab => self.focus = Focus::Editor,
            // Typing searches, as in VS Code's explorer.
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree.typing = true;
                let q = format!("{}{}", self.tree.query, c);
                self.tree.set_query(&q);
            }
            KeyCode::Backspace if self.tree.searching() => {
                self.tree.typing = true;
                let mut q = self.tree.query.clone();
                q.pop();
                self.tree.set_query(&q);
            }
            _ => {}
        }
        self.tree_follow();
    }

    /// The selected tree entry as a workspace path (`/src/main.rs`).
    fn tree_sel_rel(&self) -> Option<String> {
        let n = self.tree.nodes.get(self.tree.sel)?;
        let rel = n.path.strip_prefix(&self.tree.root).ok()?;
        Some(format!("/{}", rel.display()))
    }

    fn tree_preview(&mut self, i: usize) {
        let Some(n) = self.tree.nodes.get(i) else { return };
        if n.is_dir {
            self.tree.toggle(i);
            return;
        }
        let p = n.path.clone();
        self.open_preview(&p);
    }

    fn tree_activate(&mut self, i: usize) {
        if i >= self.tree.nodes.len() {
            return;
        }
        if self.tree.nodes[i].is_dir {
            self.tree.toggle(i);
        } else {
            let p = self.tree.nodes[i].path.clone();
            self.open_path(&p);
            self.focus = Focus::Editor;
        }
    }

    fn find_buf(&self, p: &Path) -> Option<usize> {
        let key = path_key(p);
        self.bufs
            .iter()
            .position(|b| b.path.as_ref().map(|x| path_key(x)).as_ref() == Some(&key))
    }

    /// Opens a file in the single preview slot. The next preview replaces it;
    /// editing it or opening it with Enter makes it a normal tab.
    pub fn open_preview(&mut self, p: &Path) {
        let p = absolute(p);
        if p.is_dir() {
            return;
        }
        if let Some(i) = self.find_buf(&p) {
            self.cur = i;
            return;
        }
        match Buffer::open(&p) {
            Ok(mut b) => {
                b.preview = true;
                self.sel = None;
                if let Some(i) = self.bufs.iter().position(|b| b.preview && !b.dirty) {
                    self.lsp_close_buf(i);
                    self.bufs[i] = b;
                    self.cur = i;
                } else if self.is_pristine(self.cur) {
                    self.bufs[self.cur] = b;
                } else {
                    self.bufs.push(b);
                    self.cur = self.bufs.len() - 1;
                }
            }
            Err(e) => self.error(format!("{}: {}", p.display(), e)),
        }
    }

    pub fn open_path(&mut self, p: &Path) {
        let p = absolute(p);
        if p.is_dir() {
            let _ = env::set_current_dir(&p);
            self.push_recent(&p);
            self.tree.set_root(p);
            self.tree.visible = true;
            self.focus = Focus::Tree;
            return;
        }
        self.enter_edit();
        self.sel = None;
        if let Some(i) = self.find_buf(&p) {
            self.cur = i;
            self.bufs[i].preview = false;
            return;
        }
        match Buffer::open(&p) {
            Ok(b) => {
                let is_new = !p.exists();
                if self.is_pristine(self.cur) {
                    self.bufs[self.cur] = b;
                } else {
                    self.bufs.push(b);
                    self.cur = self.bufs.len() - 1;
                }
                let name = self.rel_path(self.buf());
                if is_new {
                    self.info(format!("\"{}\" [new file]", name));
                } else {
                    let n = self.buf().lines.len();
                    self.info(format!("\"{}\" {} lines", name, n));
                }
            }
            Err(e) => self.error(format!("{}: {}", p.display(), e)),
        }
    }

    /// Starts an undo step when the kind of edit changes (typing, deleting) or
    /// for any other edit. Editing a preview tab makes it a normal tab.
    fn begin_edit(&mut self, kind: EditKind) {
        if kind == EditKind::Other || self.last_edit != Some(kind) {
            self.buf_mut().checkpoint();
        }
        self.last_edit = if kind == EditKind::Other { None } else { Some(kind) };
        self.buf_mut().preview = false;
    }

    /// Ctrl+/: comments or uncomments the line, or every selected line. Only the
    /// marker at the start of a line (after its indent) counts, so
    /// `// a // b` becomes `a // b`. When every non-blank line is commented the
    /// markers come off (with one space after them); otherwise each line gets
    /// `// ` at the block's smallest indent, so the markers line up. Blank lines
    /// are left alone. One undo step.
    ///
    /// The markers depend on the file type (`comments.rs`): `//`, `#`, `--`,
    /// `;`, `%`… Types without line comments get a block comment around the
    /// lines instead: `/* … */` in CSS, `<!-- … -->` in HTML, XML and Markdown.
    fn toggle_comment(&mut self) {
        let b = self.buf();
        let style = crate::comments::style_for(b.path.as_deref(), b.lang);
        let (y0, y1) = match self.sel_range() {
            // A selection ending at the start of a line doesn't take that line.
            Some((a, e)) if e.0 == 0 && e.1 > a.1 => (a.1, e.1 - 1),
            Some((a, e)) => (a.1, e.1),
            None => (b.cy, b.cy),
        };
        if !style.line.is_empty() {
            self.toggle_line_comments(style.line, y0, y1);
        } else if let Some((open, close)) = style.block {
            self.toggle_block_comment(open, close, y0, y1);
        } else {
            self.error("This file type has no comments");
        }
    }

    fn toggle_line_comments(&mut self, toks: &[&str], y0: usize, y1: usize) {
        let first = toks[0];
        let indent = |l: &[char]| l.iter().take_while(|c| **c == ' ' || **c == '\t').count();
        // Width of the comment marker a line starts with (after its indent), if any.
        let marker = |l: &[char]| {
            let i = indent(l);
            toks.iter()
                .map(|t| t.chars().collect::<Vec<char>>())
                .find(|t| l[i..].starts_with(t))
                .map(|t| t.len())
        };
        let b = self.buf();
        let rows: Vec<usize> = (y0..=y1)
            .filter(|&y| !b.lines[y].iter().all(|c| c.is_whitespace()))
            .collect();
        if rows.is_empty() {
            return;
        }
        let uncomment = rows.iter().all(|&y| marker(&b.lines[y]).is_some());
        let col = rows.iter().map(|&y| indent(&b.lines[y])).min().unwrap_or(0);
        self.begin_edit(EditKind::Other);
        let mut cur = self.cursor();
        let mut anchor = self.sel;
        let b = self.buf_mut();
        // Keeps a position on line `y` on the same text after `n` chars change at `at`.
        let shift = |p: &mut Pos, y: usize, at: usize, n: isize| {
            if p.1 == y && p.0 >= at {
                p.0 = (p.0 as isize + n).max(at as isize) as usize;
            }
        };
        for &y in &rows {
            if uncomment {
                let i = indent(&b.lines[y]);
                let mut n = marker(&b.lines[y]).unwrap_or(0);
                if b.lines[y].get(i + n) == Some(&' ') {
                    n += 1;
                }
                b.delete_text((i, y), (i + n, y));
                shift(&mut cur, y, i, -(n as isize));
                if let Some(a) = anchor.as_mut() {
                    shift(a, y, i, -(n as isize));
                }
            } else {
                let mut ins: Vec<char> = first.chars().collect();
                ins.push(' ');
                b.insert_text((col, y), &ins);
                shift(&mut cur, y, col, ins.len() as isize);
                if let Some(a) = anchor.as_mut() {
                    shift(a, y, col, ins.len() as isize);
                }
            }
        }
        b.cx = cur.0.min(b.line_len(cur.1));
        b.sync_want();
        self.sel = anchor;
    }

    /// One block comment around lines `y0..=y1` (from the first non-blank
    /// line's indent to the last one's end), or off again when they already
    /// start with `open` and end with `close`.
    fn toggle_block_comment(&mut self, open: &str, close: &str, y0: usize, y1: usize) {
        let (oc, cc): (Vec<char>, Vec<char>) = (open.chars().collect(), close.chars().collect());
        let b = self.buf();
        let rows: Vec<usize> = (y0..=y1)
            .filter(|&y| !b.lines[y].iter().all(|c| c.is_whitespace()))
            .collect();
        let (Some(&first), Some(&last)) = (rows.first(), rows.last()) else {
            return;
        };
        let fi = b.lines[first].iter().take_while(|c| c.is_whitespace()).count();
        let end = |l: &[char]| l.len() - l.iter().rev().take_while(|c| c.is_whitespace()).count();
        let le = end(&b.lines[last]);
        let room = first != last || le >= fi + oc.len() + cc.len();
        let wrapped = room && b.lines[first][fi..].starts_with(&oc) && b.lines[last][..le].ends_with(&cc);
        self.begin_edit(EditKind::Other);
        let mut cur = self.cursor();
        let mut anchor = self.sel;
        let b = self.buf_mut();
        // The close goes first: it is the later edit, so `fi` stays valid on one line.
        let delta = if wrapped {
            let mut cs = le - cc.len();
            if cs > fi + oc.len() && b.lines[last][cs - 1] == ' ' {
                cs -= 1;
            }
            b.delete_text((cs, last), (le, last));
            let mut n = oc.len();
            if b.lines[first].get(fi + n) == Some(&' ') {
                n += 1;
            }
            b.delete_text((fi, first), (fi + n, first));
            -(n as isize)
        } else {
            let mut tail = vec![' '];
            tail.extend(&cc);
            b.insert_text((le, last), &tail);
            let mut head = oc.clone();
            head.push(' ');
            b.insert_text((fi, first), &head);
            head.len() as isize
        };
        for p in std::iter::once(&mut cur).chain(anchor.as_mut()) {
            if p.1 == first && p.0 >= fi {
                p.0 = (p.0 as isize + delta).max(fi as isize) as usize;
            }
            p.0 = p.0.min(b.line_len(p.1));
        }
        b.cx = cur.0;
        b.sync_want();
        self.sel = anchor;
    }

    /// Prepares a cursor move: Shift extends the selection, otherwise it is dropped.
    fn start_move(&mut self, extend: bool) {
        self.last_edit = None;
        if extend {
            if self.sel.is_none() {
                self.sel = Some(self.cursor());
            }
        } else {
            self.sel = None;
        }
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, e)) = self.sel_range() else {
            self.sel = None;
            return false;
        };
        let b = self.buf_mut();
        b.delete_text(a, e);
        b.cx = a.0;
        b.cy = a.1;
        b.sync_want();
        self.sel = None;
        true
    }

    fn escape(&mut self) {
        if self.popup.take().is_some() || self.sig.take().is_some() {
            return;
        }
        if self.sel.take().is_some() {
            return;
        }
        if self.find.is_some() {
            return self.close_find();
        }
        if !kitty() {
            self.start_cmd();
        }
    }

    fn edit_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if self.popup.is_some() {
            match k.code {
                KeyCode::Down => return self.popup_move(1),
                KeyCode::Up => return self.popup_move(-1),
                KeyCode::Char('n') if ctrl => return self.popup_move(1),
                KeyCode::Char('p') if ctrl => return self.popup_move(-1),
                KeyCode::PageDown => return self.popup_move(POPUP_ROWS as isize),
                KeyCode::PageUp => return self.popup_move(-(POPUP_ROWS as isize)),
                KeyCode::Tab => return self.accept_completion(),
                KeyCode::Enter => {
                    if self.popup_fully_typed() {
                        self.popup = None;
                    } else {
                        return self.accept_completion();
                    }
                }
                _ => {}
            }
        }
        if self.is_splash() && !ctrl && !alt {
            if let KeyCode::Char(c) = k.code {
                if let Some(d) = c.to_digit(10).filter(|d| *d >= 1) {
                    // Sessions come first on the welcome screen, then folders.
                    let d = d as usize;
                    let shown = self.sessions.len().min(SHOWN);
                    if d <= shown {
                        return self.open_session(d, 0);
                    }
                    if let Some(p) = self.recent.get(d - shown - 1).cloned() {
                        return self.open_path(&p);
                    }
                }
            }
        }
        match k.code {
            KeyCode::Esc => self.escape(),
            KeyCode::Char('b') | KeyCode::Char('f') if alt => {
                self.start_move(shift);
                let b = self.buf_mut();
                let p = if k.code == KeyCode::Char('b') {
                    b.word_left((b.cx, b.cy))
                } else {
                    b.word_right((b.cx, b.cy))
                };
                b.cx = p.0;
                b.cy = p.1;
                b.sync_want();
            }
            KeyCode::Char(c) if !ctrl && !alt => self.type_char(c),
            KeyCode::Enter => {
                self.sig = None;
                self.begin_edit(EditKind::Other);
                self.delete_selection();
                self.insert_newline();
            }
            KeyCode::Backspace => {
                if (ctrl || alt) && self.sel_range().is_none() {
                    let b = self.buf();
                    let a = b.word_left((b.cx, b.cy));
                    self.sel = Some(a);
                }
                if self.sel_range().is_some() {
                    self.begin_edit(EditKind::Other);
                    self.delete_selection();
                } else {
                    self.sel = None;
                    self.begin_edit(EditKind::Delete);
                    self.backspace();
                }
                if self.popup.is_some() {
                    self.refilter();
                }
            }
            KeyCode::Delete => {
                if ctrl && self.sel_range().is_none() {
                    let b = self.buf();
                    let a = b.word_right((b.cx, b.cy));
                    self.sel = Some(a);
                }
                if self.sel_range().is_some() {
                    self.begin_edit(EditKind::Other);
                    self.delete_selection();
                } else {
                    self.sel = None;
                    self.begin_edit(EditKind::Delete);
                    self.delete_forward();
                }
            }
            KeyCode::Tab => {
                let multi = self.sel_range().is_some_and(|(a, e)| a.1 != e.1);
                if multi {
                    self.indent_lines(true);
                } else {
                    self.begin_edit(EditKind::Other);
                    self.delete_selection();
                    self.insert_tab();
                }
            }
            KeyCode::BackTab => self.indent_lines(false),
            KeyCode::Left | KeyCode::Right if !shift && !ctrl && !alt && self.sel_range().is_some() => {
                let (a, e) = self.sel_range().unwrap_or_default();
                let p = if k.code == KeyCode::Left { a } else { e };
                self.start_move(false);
                let b = self.buf_mut();
                b.cx = p.0;
                b.cy = p.1;
                b.sync_want();
            }
            KeyCode::Left | KeyCode::Right => {
                self.popup = None;
                self.sig = None;
                self.start_move(shift);
                let word = ctrl || alt;
                let b = self.buf_mut();
                let p = match (k.code == KeyCode::Left, word) {
                    (true, true) => b.word_left((b.cx, b.cy)),
                    (false, true) => b.word_right((b.cx, b.cy)),
                    (true, false) if b.cx > 0 => (b.cx - 1, b.cy),
                    (true, false) if b.cy > 0 => (b.line_len(b.cy - 1), b.cy - 1),
                    (false, false) if b.cx < b.line_len(b.cy) => (b.cx + 1, b.cy),
                    (false, false) if b.cy < b.last() => (0, b.cy + 1),
                    _ => (b.cx, b.cy),
                };
                b.cx = p.0;
                b.cy = p.1;
                b.sync_want();
            }
            KeyCode::Up | KeyCode::Down => {
                self.popup = None;
                self.sig = None;
                self.start_move(shift);
                let b = self.buf_mut();
                if k.code == KeyCode::Up {
                    if b.cy == 0 {
                        b.cx = 0;
                        b.sync_want();
                        return;
                    }
                    b.cy -= 1;
                } else {
                    if b.cy >= b.last() {
                        b.cx = b.line_len(b.cy);
                        b.sync_want();
                        return;
                    }
                    b.cy += 1;
                }
                b.cx = idx_at_col(&b.lines[b.cy], b.want_col);
            }
            KeyCode::Home => {
                self.start_move(shift);
                let b = self.buf_mut();
                if ctrl {
                    b.cy = 0;
                    b.cx = 0;
                } else {
                    let nb = b.first_nb(b.cy);
                    b.cx = if b.cx == nb { 0 } else { nb };
                }
                b.sync_want();
            }
            KeyCode::End => {
                self.start_move(shift);
                let b = self.buf_mut();
                if ctrl {
                    b.cy = b.last();
                }
                b.cx = b.line_len(b.cy);
                b.sync_want();
            }
            KeyCode::PageDown | KeyCode::PageUp => {
                self.start_move(shift);
                self.full_page(k.code == KeyCode::PageDown);
            }
            KeyCode::F(1) => {
                if !self.debug_hover() {
                    self.lsp_hover();
                }
            }
            KeyCode::F(2) => self.start_rename(),
            KeyCode::F(12) if shift => self.lsp_references(),
            KeyCode::F(12) => self.lsp_definition(),
            KeyCode::F(8) => self.goto_diag(!shift),
            _ => {}
        }
    }

    fn type_char(&mut self, c: char) {
        if self.sel_range().is_some() {
            self.begin_edit(EditKind::Other);
            self.delete_selection();
            self.last_edit = Some(EditKind::Type);
        } else {
            self.sel = None;
            self.begin_edit(EditKind::Type);
        }
        self.insert_char(c);
        self.buf_mut().sync_want();
        self.after_insert_char(c);
    }

    /// Tab / Shift+Tab on the selected lines (or the cursor line).
    fn indent_lines(&mut self, indent: bool) {
        let unit = self.indent_unit();
        let (y0, y1) = match self.sel_range() {
            Some((a, e)) => (a.1, if e.0 == 0 && e.1 > a.1 { e.1 - 1 } else { e.1 }),
            None => (self.buf().cy, self.buf().cy),
        };
        self.begin_edit(EditKind::Other);
        let b = self.buf_mut();
        let mut shift_cur = 0isize;
        for y in y0..=y1 {
            let line = &mut b.lines[y];
            let delta: isize = if indent {
                if line.is_empty() {
                    0
                } else {
                    line.splice(0..0, unit.iter().copied());
                    unit.len() as isize
                }
            } else if line.first() == Some(&'\t') {
                line.remove(0);
                -1
            } else {
                let n = line.iter().take(TAB_W).take_while(|c| **c == ' ').count();
                line.drain(..n);
                -(n as isize)
            };
            if y == b.cy {
                shift_cur = delta;
            }
        }
        b.touch();
        b.cx = (b.cx as isize + shift_cur).max(0) as usize;
        b.clamp(true);
        if let Some(a) = self.sel {
            if (a.1 >= y0 && a.1 <= y1) && a.0 > 0 {
                let len = self.buf().line_len(a.1);
                self.sel = Some((
                    (a.0 as isize + if indent { unit.len() as isize } else { -1 }).clamp(0, len as isize) as usize,
                    a.1,
                ));
            }
        }
    }

    fn select_all(&mut self) {
        self.last_edit = None;
        self.sel = Some((0, 0));
        let b = self.buf_mut();
        b.cy = b.last();
        b.cx = b.line_len(b.cy);
        b.sync_want();
    }

    /// Ctrl+C: the selection, or the whole line when nothing is selected.
    pub(crate) fn copy(&mut self) {
        match self.selected_text() {
            Some(t) => {
                self.set_reg(Reg::Chars(t));
                self.info("Copied");
            }
            None => {
                let line = self.buf().lines[self.buf().cy].clone();
                self.set_reg(Reg::Lines(vec![line]));
                self.info("Line copied");
            }
        }
    }

    /// Ctrl+X: cuts the selection, or the whole line when nothing is selected.
    pub(crate) fn cut(&mut self) {
        self.begin_edit(EditKind::Other);
        match self.selected_text() {
            Some(t) => {
                self.set_reg(Reg::Chars(t));
                self.delete_selection();
            }
            None => {
                let b = self.buf_mut();
                let y = b.cy;
                let ls = b.delete_lines(y, y);
                b.cy = y.min(b.last());
                b.cx = b.first_nb(b.cy);
                self.set_reg(Reg::Lines(ls));
            }
        }
    }

    /// Ctrl+V: replaces the selection. A copied whole line goes above the cursor line.
    pub(crate) fn paste(&mut self) {
        self.sync_from_clip();
        let reg = self.reg.clone();
        self.begin_edit(EditKind::Other);
        match reg {
            Reg::Lines(ls) if self.sel_range().is_none() => {
                if ls.is_empty() {
                    return;
                }
                let b = self.buf_mut();
                let at = b.cy;
                b.insert_lines(at, &ls);
                b.cy = at + ls.len();
            }
            Reg::Lines(ls) => {
                self.delete_selection();
                let text: Vec<char> = ls
                    .iter()
                    .flat_map(|l| l.iter().copied().chain(std::iter::once('\n')))
                    .collect();
                self.insert_at_cursor(&text);
            }
            Reg::Chars(cs) => {
                self.delete_selection();
                self.insert_at_cursor(&cs);
            }
        }
        self.buf_mut().sync_want();
    }

    fn insert_at_cursor(&mut self, text: &[char]) {
        let b = self.buf_mut();
        let p = b.insert_text((b.cx, b.cy), text);
        b.cx = p.0;
        b.cy = p.1;
    }

    pub(crate) fn undo_any(&mut self) {
        self.popup = None;
        self.sig = None;
        self.sel = None;
        self.last_edit = None;
        if !self.buf_mut().undo() {
            self.info("Already at oldest change");
        }
        self.buf_mut().clamp(true);
    }

    pub(crate) fn redo_any(&mut self) {
        self.popup = None;
        self.sel = None;
        self.last_edit = None;
        if !self.buf_mut().redo() {
            self.info("Already at newest change");
        }
        self.buf_mut().clamp(true);
    }

    fn full_page(&mut self, down: bool) {
        let h = (self.layout().text_h as usize).saturating_sub(2).max(1);
        let b = self.buf_mut();
        let last = b.last();
        if down {
            b.cy = (b.cy + h).min(last);
            b.scroll_y = (b.scroll_y + h).min(last);
        } else {
            b.cy = b.cy.saturating_sub(h);
            b.scroll_y = b.scroll_y.saturating_sub(h);
        }
        b.cx = idx_at_col(&b.lines[b.cy], b.want_col);
    }

    fn insert_char(&mut self, c: char) {
        let unit_len = self.indent_unit().len();
        let burst = self.burst;
        let b = self.buf_mut();
        let (x, y) = (b.cx.min(b.line_len(b.cy)), b.cy);
        b.insert_text((x, y), &[c]);
        b.cx = x + 1;
        if !burst && matches!(c, '}' | ']' | ')') {
            let line = &b.lines[y];
            if x > 0 && line[..x].iter().all(|c| *c == ' ' || *c == '\t') {
                let rm = if line[0] == '\t' { 1 } else { unit_len.min(x) };
                b.delete_text((0, y), (rm, y));
                b.cx -= rm;
            }
        }
    }

    fn insert_newline(&mut self) {
        let unit = self.indent_unit();
        let py = self.buf().lang.name == "Python";
        let burst = self.burst;
        let b = self.buf_mut();
        let (cx, cy) = (b.cx.min(b.line_len(b.cy)), b.cy);
        if burst {
            let p = b.insert_text((cx, cy), &['\n']);
            b.cx = p.0;
            b.cy = p.1;
            return;
        }
        let line = &b.lines[cy];
        let base = leading_ws(&line[..cx]);
        let before = line[..cx].iter().rev().find(|c| !c.is_whitespace()).copied();
        let after = line[cx..].iter().find(|c| !c.is_whitespace()).copied();
        let opener = matches!(before, Some('{' | '[' | '(')) || (py && before == Some(':'));
        let ws_after = line[cx..].iter().take_while(|c| **c == ' ' || **c == '\t').count();
        if ws_after > 0 {
            b.delete_text((cx, cy), (cx + ws_after, cy));
        }
        let mut ins = vec!['\n'];
        ins.extend(&base);
        if opener {
            ins.extend(&unit);
        }
        let pos = b.insert_text((cx, cy), &ins);
        let closes = matches!(
            (before, after),
            (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')'))
        );
        if closes {
            let mut tail = vec!['\n'];
            tail.extend(&base);
            b.insert_text(pos, &tail);
        }
        b.cx = pos.0;
        b.cy = pos.1;
        b.sync_want();
    }

    fn backspace(&mut self) {
        let b = self.buf_mut();
        if b.cx > 0 {
            let x = b.cx.min(b.line_len(b.cy));
            let line = &b.lines[b.cy];
            let all_sp = line[..x].iter().all(|c| *c == ' ');
            let n = if all_sp && !b.use_tabs {
                let r = x % TAB_W;
                if r == 0 {
                    TAB_W.min(x)
                } else {
                    r
                }
            } else {
                1
            };
            let y = b.cy;
            b.delete_text((x - n, y), (x, y));
            b.cx = x - n;
        } else if b.cy > 0 {
            let (y, py) = (b.cy, b.cy - 1);
            let len = b.lines[py].len();
            b.delete_text((len, py), (0, y));
            b.cy = py;
            b.cx = len;
        }
        b.sync_want();
    }

    fn delete_forward(&mut self) {
        let b = self.buf_mut();
        let len = b.line_len(b.cy);
        let y = b.cy;
        if b.cx < len {
            let x = b.cx;
            b.delete_text((x, y), (x + 1, y));
        } else if y < b.last() {
            b.delete_text((len, y), (0, y + 1));
        }
    }

    fn insert_tab(&mut self) {
        let b = self.buf_mut();
        let y = b.cy;
        let x = b.cx.min(b.line_len(y));
        if b.use_tabs {
            b.insert_text((x, y), &['\t']);
            b.cx = x + 1;
        } else {
            let col = disp_col(&b.lines[y], x);
            let n = TAB_W - col % TAB_W;
            b.insert_text((x, y), &vec![' '; n]);
            b.cx = x + n;
        }
        b.sync_want();
    }

    fn on_paste(&mut self, s: &str) {
        if self.focus == Focus::Find && self.mode != Mode::Command {
            return self.find_paste(s);
        }
        if self.focus == Focus::Panel && self.mode != Mode::Command && self.panel_paste(s) {
            return;
        }
        if self.commit.is_some() && self.mode != Mode::Command {
            return self.commit_paste(s);
        }
        let text: Vec<char> = s.replace("\r\n", "\n").replace('\r', "\n").chars().collect();
        match self.mode {
            Mode::Command => {
                for c in text.into_iter().take_while(|c| *c != '\n') {
                    self.cmd.insert(self.cmd_x, c);
                    self.cmd_x += 1;
                }
            }
            Mode::Edit => {
                if self.focus != Focus::Editor {
                    return;
                }
                self.begin_edit(EditKind::Other);
                self.delete_selection();
                self.insert_at_cursor(&text);
            }
        }
    }

    pub(crate) fn start_cmd(&mut self) {
        self.mode = Mode::Command;
        self.cmd_hint = None;
        self.cmd_comp = None;
        self.popup = None;
        self.sig = None;
        self.cmd.clear();
        self.cmd_x = 0;
        self.hist_i = self.history.len();
        self.msg = None;
    }

    /// Opens the command line with text already typed, ready to edit or confirm.
    pub(crate) fn prefill_cmd(&mut self, s: &str) {
        self.start_cmd();
        self.cmd = s.chars().collect();
        self.cmd_x = self.cmd.len();
    }

    fn cmd_key(&mut self, k: KeyEvent) {
        // An open completion list takes ↑/↓ and Esc; any other key but Tab closes it.
        let listing = self.cmd_comp.as_ref().is_some_and(|c| c.items.len() > 1);
        match k.code {
            KeyCode::Up | KeyCode::Down if listing => return self.complete(k.code == KeyCode::Up),
            KeyCode::Esc if self.cmd_comp.is_some() => {
                self.cmd_comp = None;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {}
            _ => self.cmd_comp = None,
        }
        match k.code {
            KeyCode::Esc => self.mode = Mode::Edit,
            KeyCode::Enter => {
                let s: String = self.cmd.iter().collect();
                self.mode = Mode::Edit;
                if !s.is_empty() && self.history.last() != Some(&s) {
                    self.history.push(s.clone());
                }
                self.run_command(&s);
            }
            KeyCode::Backspace => {
                if self.cmd.is_empty() {
                    self.mode = Mode::Edit;
                } else if self.cmd_x > 0 {
                    self.cmd_x -= 1;
                    self.cmd.remove(self.cmd_x);
                }
            }
            KeyCode::Delete => {
                if self.cmd_x < self.cmd.len() {
                    self.cmd.remove(self.cmd_x);
                }
            }
            KeyCode::Left => self.cmd_x = self.cmd_x.saturating_sub(1),
            KeyCode::Right => self.cmd_x = (self.cmd_x + 1).min(self.cmd.len()),
            KeyCode::Home => self.cmd_x = 0,
            KeyCode::End => self.cmd_x = self.cmd.len(),
            KeyCode::Up => {
                if self.hist_i > 0 {
                    self.hist_i -= 1;
                    self.cmd = self.history[self.hist_i].chars().collect();
                    self.cmd_x = self.cmd.len();
                }
            }
            KeyCode::Down => {
                if self.hist_i + 1 < self.history.len() {
                    self.hist_i += 1;
                    self.cmd = self.history[self.hist_i].chars().collect();
                } else {
                    self.hist_i = self.history.len();
                    self.cmd.clear();
                }
                self.cmd_x = self.cmd.len();
            }
            KeyCode::BackTab => self.complete(true),
            KeyCode::Tab => self.complete(k.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::Char(c) => {
                if is_ctrl(&k) {
                    if c == 'u' {
                        self.cmd.drain(..self.cmd_x);
                        self.cmd_x = 0;
                    }
                } else {
                    self.cmd.insert(self.cmd_x, c);
                    self.cmd_x += 1;
                }
            }
            _ => {}
        }
    }

    /// Tab on the `:` line completes the last path argument. Paths follow the
    /// command path rules: `/x` from the workspace root, `~/x` from home, and
    /// anything else from the current folder (tree selection or active file).
    /// Tab on the `:` line: completes a command name, a command's word, or a
    /// path (relative paths start from `here()`). The longest common part goes
    /// in; with several matches a list opens and Tab / Shift+Tab / ↑↓ step
    /// through it.
    fn complete(&mut self, back: bool) {
        if let Some(c) = self.cmd_comp.as_mut().filter(|c| c.items.len() > 1) {
            let n = c.items.len();
            let i = match (c.sel, back) {
                (None, false) => 0,
                (None, true) => n - 1,
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
            };
            c.sel = Some(i);
            let new = format!("{}{}", c.head, c.items[i].1);
            let rest = c.rest.clone();
            self.cmd_x = new.chars().count();
            self.cmd = format!("{}{}", new, rest).chars().collect();
            return;
        }
        let before: String = self.cmd[..self.cmd_x].iter().collect();
        let rest: String = self.cmd[self.cmd_x..].iter().collect();
        let (head, word) = match before.rfind(' ') {
            Some(sp) => (before[..=sp].to_string(), before[sp + 1..].to_string()),
            None => (String::new(), before.clone()),
        };
        let words: Vec<&str> = head.split_whitespace().collect();
        let (items, title) = if words.is_empty() {
            let items = COMMANDS
                .iter()
                .filter(|c| c.starts_with(word.as_str()))
                .map(|c| (c.to_string(), format!("{} ", c)))
                .collect();
            (items, "commands".to_string())
        } else {
            let fixed: Vec<(String, String)> = if words.len() == 1 {
                arg_words(words[0])
                    .iter()
                    .filter(|w| w.starts_with(word.as_str()))
                    .map(|w| (w.to_string(), w.to_string()))
                    .collect()
            } else {
                Vec::new()
            };
            if fixed.is_empty() {
                self.path_matches(&word)
            } else {
                (fixed, words[0].to_string())
            }
        };
        if items.is_empty() {
            self.cmd_comp = Some(CmdComp {
                head,
                rest,
                items,
                sel: None,
                title: format!("nothing matches \"{}\" {}", word, title),
            });
            return;
        }
        // The common start of every match goes in.
        let first: Vec<char> = items[0].1.chars().collect();
        let mut common = first.len();
        for (_, w) in &items[1..] {
            common = common.min(first.iter().zip(w.chars()).take_while(|(a, b)| **a == *b).count());
        }
        let fill: String = first[..common].iter().collect();
        let fill = if fill.chars().count() >= word.chars().count() {
            fill
        } else {
            word
        };
        let new = format!("{}{}", head, fill);
        self.cmd_x = new.chars().count();
        self.cmd = format!("{}{}", new, rest).chars().collect();
        self.cmd_comp = (items.len() > 1).then_some(CmdComp {
            head,
            rest,
            items,
            sel: None,
            title,
        });
    }

    /// Files and folders that start with `word`'s last part, and where they were
    /// looked for ("in /src/").
    fn path_matches(&self, word: &str) -> (Vec<(String, String)>, String) {
        let quoted = word.starts_with('"');
        let arg = word.trim_start_matches('"');
        let (dir, prefix) = match arg.rfind('/') {
            Some(i) => (&arg[..=i], &arg[i + 1..]),
            None => ("", arg),
        };
        let base = self.resolve_path(if dir.is_empty() { "." } else { dir });
        let title = format!("in {}", self.show_dir(&base));
        let Ok(rd) = fs::read_dir(&base) else {
            return (Vec::new(), title);
        };
        let mut found: Vec<(bool, String)> = rd
            .filter_map(|e| e.ok())
            .map(|e| (!e.path().is_dir(), e.file_name().to_string_lossy().into_owned()))
            .filter(|(_, n)| n.starts_with(prefix) && (prefix.starts_with('.') || !n.starts_with('.')))
            .collect();
        // Folders first, then files, each alphabetical.
        found.sort_by_key(|(file, n)| (*file, n.to_lowercase()));
        let items = found
            .into_iter()
            .map(|(file, n)| {
                let shown = if file { n } else { format!("{}/", n) };
                let path = format!("{}{}", dir, shown);
                let put = if quoted || path.contains(' ') {
                    format!("\"{}", path)
                } else {
                    path
                };
                (shown, put)
            })
            .collect();
        (items, title)
    }

    /// A folder the way commands take it: `/x/` from the workspace root, `~/x/`
    /// from home.
    fn show_dir(&self, p: &Path) -> String {
        if let Ok(r) = p.strip_prefix(&self.tree.root) {
            let r = r.display().to_string();
            return if r.is_empty() { "/".into() } else { format!("/{}/", r) };
        }
        if let Some(r) = crate::settings::home().and_then(|h| p.strip_prefix(h).ok().map(Path::to_path_buf)) {
            return format!("~/{}/", r.display()).replace("//", "/");
        }
        format!("{}/", p.display())
    }

    /// The folder relative paths start from: the selected tree entry's folder
    /// when the tree has focus, otherwise the active file's folder, otherwise
    /// the workspace root.
    pub(crate) fn here(&self) -> PathBuf {
        if self.focus == Focus::Tree {
            if let Some(n) = self.tree.nodes.get(self.tree.sel) {
                return if n.is_dir {
                    n.path.clone()
                } else {
                    n.path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.tree.root.clone())
                };
            }
        }
        if let Some(p) = self.buf().path.as_ref().and_then(|p| p.parent()) {
            return p.to_path_buf();
        }
        self.tree.root.clone()
    }

    /// Resolves a command path: `/x` = workspace root, `~/x` = home, else relative to `here()`.
    pub(crate) fn resolve_path(&self, p: &str) -> PathBuf {
        let joined = if let Some(r) = p.strip_prefix("~/").or(if p == "~" { Some("") } else { None }) {
            crate::settings::home().unwrap_or_default().join(r)
        } else if let Some(r) = p.strip_prefix('/') {
            self.tree.root.join(r)
        } else {
            self.here().join(p)
        };
        absolute(&joined)
    }

    fn run_command(&mut self, s: &str) {
        let s = s.trim();
        if s.is_empty() {
            return;
        }
        if let Ok(n) = s.parse::<usize>() {
            self.focus = Focus::Editor;
            self.sel = None;
            let b = self.buf_mut();
            b.cy = n.saturating_sub(1).min(b.last());
            b.cx = b.first_nb(b.cy);
            b.sync_want();
            return;
        }
        if let Some(rest) = s.strip_prefix('!') {
            return self.run_shell(rest.trim());
        }
        let args = split_args(s);
        let cmd = args.first().cloned().unwrap_or_default();
        let rest: Vec<String> = args[1..].to_vec();
        let arg = s[cmd.len().min(s.len())..].trim();
        let arg_opt = if arg.is_empty() { None } else { Some(arg) };
        if self.file_command(&cmd, &rest) {
            return;
        }
        match cmd.as_str() {
            "q" | "quit" | "close" => self.close_buffer(false),
            "q!" | "quit!" => self.close_buffer(true),
            "qa" | "qall" => self.quit_all(false),
            "qa!" | "qall!" => self.quit_all(true),
            "wq" | "x" | "xit" => {
                if self.save(None) {
                    self.close_buffer(false);
                }
            }
            "w" | "write" | "save" => match rest.first() {
                Some(p) => {
                    let p = self.resolve_path(p);
                    self.save(Some(&p.display().to_string()));
                }
                None => {
                    self.save(None);
                }
            },
            "revert" => self.revert(),
            "open" => match rest.first() {
                Some(p) => {
                    // `/x` is the workspace's x, unless only a real `/x` exists.
                    let mut path = self.resolve_path(p);
                    if p.starts_with('/') && !path.exists() && Path::new(p).exists() {
                        path = absolute(Path::new(p));
                    }
                    self.open_path(&path);
                }
                None => {
                    let d = self.tree.root.clone();
                    self.folder_picker(d, None, false);
                }
            },
            "tree" => self.toggle_tree(),
            "project" | "proj" => self.project_command(&rest),
            "workspace" | "ws" => self.workspace_command(&rest),
            // The answer to what a launch asked (`${input:…}`), see debug_run.rs.
            "input" => self.input_answer(arg),
            "cd" => {
                let target = rest
                    .first()
                    .map(|p| self.resolve_path(p))
                    .unwrap_or_else(|| self.tree.root.clone());
                match env::set_current_dir(&target) {
                    Ok(()) => {
                        let root = env::current_dir().unwrap_or(target);
                        self.info(format!("{}", root.display()));
                        self.push_recent(&root);
                        self.tree.set_root(root);
                    }
                    Err(e) => self.error(format!("cd: {}", e)),
                }
            }
            "preview" | "md" => self.toggle_md_view(),
            "pwd" => {
                let d = env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
                self.info(d);
            }
            "help" => self.open_help(),
            "settings" => self.open_settings(arg),
            "reload" => self.reload_app(),
            "recover" => self.recover(arg),
            "session" | "sessions" => self.session_command(&rest),
            "hints" => self.hints_command(&rest),
            "scan" => self.scan_command(),
            "mod" => self.link_module(rest.first().map(String::as_str)),
            "icons" => {
                self.settings.icons = !self.settings.icons;
                self.info(format!(
                    "Icons {} (set \"icons\" in :settings to keep it)",
                    if self.settings.icons { "on" } else { "off" }
                ));
            }
            "rename" => match arg_opt {
                Some(n) => self.lsp_rename(n),
                None => self.start_rename(),
            },
            "format" => self.lsp_format(),
            "actions" => self.lsp_code_actions(),
            "diag" | "diagnostics" => self.diag_menu(),
            "def" | "definition" => self.lsp_definition(),
            "refs" | "references" => self.lsp_references(),
            "symbols" => self.lsp_document_symbols(),
            "wsymbols" => self.workspace_symbols(),
            "find" => self.open_find(false),
            "replace" => self.open_find(true),
            "hover" => self.lsp_hover(),
            "lsp" => self.lsp_command(arg),
            "panel" => self.toggle_panel(),
            "terminal" | "term" => self.terminal_command(&rest),
            "git" => self.git_command(&rest),
            "git-new-branch" => self.prefill_cmd("git checkout -b "),
            "containers" => self.containers_command(&rest),
            "debug" => self.debug_command(arg),
            "install" => self.install_command(arg),
            "uninstall" => self.uninstall_command(arg),
            "break" => {
                let y = self.buf().cy;
                self.toggle_breakpoint(y);
            }
            "breakclear" => {
                let keys: Vec<String> = self.dbg.bps.keys().cloned().collect();
                for k in keys {
                    if let Some(e) = self.dbg.bps.get_mut(&k) {
                        e.1.clear();
                    }
                }
                let y = self.buf().cy;
                self.toggle_breakpoint(y);
                self.toggle_breakpoint(y);
                self.dbg.bps.clear();
                self.info("All breakpoints cleared");
            }
            "watch" => match arg_opt {
                Some(e) => self.add_watch(e),
                None => self.error("Usage: :watch <expression>"),
            },
            "unwatch" => match arg.parse::<usize>() {
                Ok(n) if n >= 1 && n <= self.dbg.watches.len() => {
                    self.dbg.watches.remove(n - 1);
                }
                _ => self.dbg.watches.clear(),
            },
            _ => self.error(format!("Unknown command: {}", cmd)),
        }
    }

    fn open_help(&mut self) {
        if let Some(i) = self.bufs.iter().position(|b| b.name == HELP_NAME) {
            self.cur = i;
            return;
        }
        let b = Buffer::from_text(HELP_NAME, help_text());
        if self.is_pristine(self.cur) {
            self.bufs[self.cur] = b;
        } else {
            self.bufs.push(b);
            self.cur = self.bufs.len() - 1;
        }
    }

    /// `:revert`: reloads the file from disk, dropping unsaved changes.
    fn revert(&mut self) {
        let Some(p) = self.buf().path.clone() else {
            return self.error("No file name");
        };
        match Buffer::open(&p) {
            Ok(mut nb) => {
                let b = self.buf();
                nb.cy = b.cy;
                nb.cx = b.cx;
                nb.scroll_y = b.scroll_y;
                self.lsp_close_buf(self.cur);
                self.bufs[self.cur] = nb;
                self.sel = None;
                self.info("Reloaded from disk");
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    pub fn save(&mut self, path: Option<&str>) -> bool {
        if let Some(p) = path {
            let abs = absolute(Path::new(p));
            self.lsp_close_buf(self.cur);
            let b = self.buf_mut();
            b.name = abs
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            b.lang = highlight::detect(&abs);
            b.path = Some(abs);
        }
        if self.buf().path.is_none() {
            self.error("No file name — use :save <path>");
            return false;
        }
        let fmt = if path.is_none() {
            self.format_before_save(self.cur)
        } else {
            FormatOutcome::Skipped
        };
        self.buf_mut().preview = false;
        match self.buf_mut().save() {
            Ok(bytes) => {
                let name = self.rel_path(self.buf());
                let n = self.buf().lines.len();
                self.info(format!("\"{}\" {}L {}B written{}", name, n, bytes, fmt.note()));
                self.tree.refresh();
                self.lsp_did_save();
                self.scan_saved();
                if self.is_settings_file(self.cur) {
                    let errs = self.load_settings();
                    if errs.is_empty() {
                        self.info("Settings applied");
                    } else {
                        self.error(errs.join("; "));
                    }
                }
                true
            }
            Err(e) => {
                self.error(format!("Write failed: {}", e));
                false
            }
        }
    }

    /// `:q`, `:q!`, `:wq`, a tab's ×: closes the tab. The last one leaves an
    /// empty tab (the welcome screen); ren stays open, and only `:qa` quits.
    fn close_buffer(&mut self, force: bool) {
        if self.buf().dirty && !force && !self.is_scratch_output() {
            return self.error("Unsaved changes (Ctrl+S to save, :q! to discard)");
        }
        self.sel = None;
        if self.bufs.len() == 1 {
            if self.is_pristine(0) {
                return self.info("Nothing to close (:qa quits ren)");
            }
            self.lsp_close_buf(0);
            self.bufs[0] = Buffer::scratch(NONAME);
            self.mode = Mode::Edit;
            return;
        }
        self.lsp_close_buf(self.cur);
        self.bufs.remove(self.cur);
        if self.cur >= self.bufs.len() {
            self.cur = self.bufs.len() - 1;
        }
        self.mode = Mode::Edit;
    }

    /// The tab under (x, y): (start x, end x, buffer, close slot x).
    fn tab_at(&self, x: u16, y: u16) -> Option<(u16, u16, usize, u16)> {
        if y >= TAB_H {
            return None;
        }
        self.tab_spans.iter().copied().find(|&(a, b, _, _)| x >= a && x < b)
    }

    /// The × on a tab: a saved tab just closes (the last one leaves an empty tab,
    /// ren stays open); with unsaved changes the `:` line opens for `q!` or `wq`.
    fn close_tab(&mut self, i: usize) {
        if i >= self.bufs.len() {
            return;
        }
        self.enter_edit();
        self.sel = None;
        self.cur = i;
        self.focus = Focus::Editor;
        self.tab_hover = None;
        if self.buf().dirty && !self.is_scratch_output() {
            self.start_cmd();
            self.cmd_hint = Some(format!(
                "\"{}\" has unsaved changes: q! to discard, wq to save",
                self.buf().name
            ));
        } else {
            self.close_buffer(false);
        }
    }

    fn is_scratch_output(&self) -> bool {
        let b = self.buf();
        b.path.is_none() && (b.name.starts_with("[!") || b.name == HELP_NAME)
    }

    fn quit_all(&mut self, force: bool) {
        if !force {
            if let Some(i) = self
                .bufs
                .iter()
                .position(|b| b.dirty && (b.path.is_some() || b.name == NONAME))
            {
                self.cur = i;
                let name = self.bufs[i].name.clone();
                return self.error(format!("\"{}\" is not saved (:qa! to discard)", name));
            }
            if !self.parked_saved() {
                return;
            }
        }
        self.quit = true;
    }

    fn cycle_buf(&mut self, d: isize) {
        let n = self.bufs.len() as isize;
        self.cur = ((self.cur as isize + d).rem_euclid(n)) as usize;
        self.sel = None;
        self.mode = Mode::Edit;
    }

    fn run_shell(&mut self, cmd: &str) {
        if cmd.is_empty() {
            return self.error("Usage: :!command");
        }
        let file = self
            .buf()
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let full = cmd.replace('%', &format!("\"{}\"", file));
        let mut c = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", &full]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", &full]);
            c
        };
        c.current_dir(&self.tree.root).stdin(Stdio::null());
        match c.output() {
            Ok(o) => {
                let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                let err = String::from_utf8_lossy(&o.stderr);
                if !err.is_empty() {
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text += &err;
                }
                let code = o.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                let mut short: String = cmd.chars().take(24).collect();
                if cmd.chars().count() > 24 {
                    short.push('…');
                }
                let name = format!("[!{}]", short);
                let b = Buffer::from_text(&name, text.trim_end_matches('\n'));
                if let Some(i) = self.bufs.iter().position(|b| b.name == name) {
                    self.bufs[i] = b;
                    self.cur = i;
                } else if self.is_pristine(self.cur) {
                    self.bufs[self.cur] = b;
                } else {
                    self.bufs.push(b);
                    self.cur = self.bufs.len() - 1;
                }
                let msg = format!("!{} → exit code {}", cmd, code);
                if o.status.success() {
                    self.info(msg);
                } else {
                    self.error(msg);
                }
                self.tree.refresh();
            }
            Err(e) => self.error(format!("Failed to run: {}", e)),
        }
    }

    /// The name under the mouse, when there's a language server to take
    /// Ctrl+click to its definition: (tab, line, start, end).
    fn link_at(&self, x: u16, y: u16, l: &Layout) -> Option<(usize, usize, usize, usize)> {
        if self.is_splash() || self.buf().md_view || self.lsp_lang(self.cur).is_none() {
            return None;
        }
        let (cx, cy) = self.pos_from_mouse(x, y, l);
        let line = self.buf().lines.get(cy)?;
        // On the character itself, not past the end of the line.
        let (back, _) = self.pos_from_mouse(x.saturating_sub(1), y, l);
        if cx >= line.len() || (cx > 0 && back == cx && x > l.text_x) {
            return None;
        }
        let word = |c: char| c.is_alphanumeric() || c == '_';
        if !word(line[cx]) {
            return None;
        }
        let mut s = cx;
        while s > 0 && word(line[s - 1]) {
            s -= 1;
        }
        let mut e = cx;
        while e < line.len() && word(line[e]) {
            e += 1;
        }
        Some((self.cur, cy, s, e))
    }

    fn pos_from_mouse(&self, x: u16, y: u16, l: &Layout) -> Pos {
        let b = self.buf();
        let ly = if y < l.top {
            b.scroll_y.saturating_sub(1)
        } else {
            b.scroll_y + (y - l.top) as usize
        };
        let ly = ly.min(b.last());
        let vcol = b.scroll_x + x.saturating_sub(l.text_x) as usize;
        (b.idx_at_vcol(ly, vcol), ly)
    }

    fn in_rect(r: Option<(u16, u16, u16, u16)>, x: u16, y: u16) -> Option<(u16, u16)> {
        let (rx, ry, rw, rh) = r?;
        (x >= rx && x < rx + rw && y >= ry && y < ry + rh).then(|| (x - rx, y - ry))
    }

    fn mouse_overlays(&mut self, m: &MouseEvent) -> bool {
        let (x, y) = (m.column, m.row);
        let is_down = matches!(m.kind, MouseEventKind::Down(_));
        let wheel = match m.kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => 0,
        };
        if self.menu.is_some() {
            let hit = Self::in_rect(self.menu_rect.get(), x, y);
            if wheel != 0 {
                if let Some(mm) = self.menu.as_mut() {
                    let n = mm.items.len();
                    mm.sel = (mm.sel as isize + wheel).clamp(0, n as isize - 1) as usize;
                }
                self.menu_follow();
                return true;
            }
            if is_down {
                match hit {
                    Some((_, ry)) if ry >= 1 => {
                        let i = self.menu.as_ref().map_or(0, |mm| mm.scroll) + ry as usize - 1;
                        if i < self.menu.as_ref().map_or(0, |mm| mm.items.len()) {
                            self.menu_choose(i);
                        }
                    }
                    Some(_) => {}
                    None => self.menu = None,
                }
                return true;
            }
            return matches!(
                m.kind,
                MouseEventKind::Moved | MouseEventKind::Up(_) | MouseEventKind::Drag(_)
            );
        }
        if self.popup.is_some() {
            let hit = Self::in_rect(self.popup_rect.get(), x, y);
            if let Some((_, ry)) = hit {
                if wheel != 0 {
                    self.popup_move(wheel);
                    return true;
                }
                if is_down {
                    let i = self.popup.as_ref().map_or(0, |p| p.scroll) + ry as usize;
                    if let Some(p) = self.popup.as_mut() {
                        if i < p.shown.len() {
                            p.sel = i;
                        }
                    }
                    self.accept_completion();
                    return true;
                }
            } else if is_down {
                self.popup = None;
            }
        }
        false
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if matches!(
            m.kind,
            MouseEventKind::Down(_) | MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        ) {
            self.hover = None;
        }
        self.tab_hover = self.tab_at(m.column, m.row).map(|t| t.2);
        if self.md_mouse(&m) {
            return;
        }
        if self.peek_mouse(&m) {
            return;
        }
        if self.mouse_overlays(&m) {
            return;
        }
        let l = self.layout();
        let (x, y) = (m.column, m.row);
        let in_text_rows = y >= l.top && y < l.top + l.text_h;
        let in_side_rows = y >= l.top && y < l.top + l.side_h;
        let over_tree = self.tree.visible && x < l.tree_w && in_side_rows;
        let on_sep = self.tree.visible && x == l.tree_w && in_side_rows;
        let over_dbg = l.dbg_h > 0 && y >= l.dbg_y && y < l.dbg_y + l.dbg_h && x >= l.dbg_x;
        let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
        self.link = if ctrl && in_text_rows && x >= l.text_x && !over_dbg && m.kind == MouseEventKind::Moved {
            self.link_at(x, y, &l)
        } else {
            None
        };
        // Panel hit boxes are in panel coordinates.
        let px = x.saturating_sub(l.dbg_x);
        let hover = if over_dbg { self.panel_hit(px, y) } else { None };
        // Hovering a terminal tab shows its × (kill) button.
        self.term_hover = match hover {
            Some(debug_ui::DbgHit::TermTab(i) | debug_ui::DbgHit::TermKill(i)) => Some(i),
            _ => None,
        };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if over_dbg => {
                self.mode = Mode::Edit;
                let now = Instant::now();
                let double = matches!(self.last_click, Some((t, lx, ly)) if now.duration_since(t).as_millis() < 400 && lx == x && ly == y);
                self.last_click = Some((now, x, y));
                if !self.dbg_click(px, y, double) && y == l.dbg_y {
                    self.drag = Drag::DbgSep;
                }
                return;
            }
            MouseEventKind::Down(MouseButton::Middle) if over_dbg => {
                if let Some(debug_ui::DbgHit::TermTab(i) | debug_ui::DbgHit::TermKill(i)) = hover {
                    self.term_kill(i);
                }
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.drag == Drag::DbgSep => {
                let total = self.h.saturating_sub(TAB_H + 1);
                self.dbg.height = (total + TAB_H)
                    .saturating_sub(y)
                    .clamp(6, total.saturating_sub(6).max(6));
                return;
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if over_dbg => {
                let d: isize = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
                if !self.panel_scroll(px, d) {
                    self.dbg_scroll(px, d);
                }
                return;
            }
            MouseEventKind::Down(MouseButton::Left)
                if in_text_rows && x >= l.ed_x && x < l.text_x && !self.is_splash() =>
            {
                let ly = self.buf().scroll_y + (y - l.top) as usize;
                let on_find = self.find.is_some() && self.find_click(x, y);
                if !on_find && ly < self.buf().lines.len() {
                    self.toggle_breakpoint(ly);
                }
                return;
            }
            _ => {}
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let now = Instant::now();
                let same = matches!(self.last_click, Some((t, lx, ly)) if now.duration_since(t).as_millis() < 400 && lx == x && ly == y);
                self.clicks = if same { self.clicks % 3 + 1 } else { 1 };
                self.last_click = Some((now, x, y));
                self.mode = Mode::Edit;
                self.last_edit = None;
                self.hscroll_free = false;
                if y < l.top {
                    if x >= self.proj_span.0 && x < self.proj_span.1 {
                        return self.project_menu();
                    }
                    if let Some((_, _, i, close_x)) = self.tab_at(x, y) {
                        if x == close_x {
                            return self.close_tab(i);
                        }
                        self.enter_edit();
                        self.sel = None;
                        self.cur = i;
                        self.focus = Focus::Editor;
                    }
                    return;
                }
                if !in_text_rows && !over_tree && !on_sep {
                    return;
                }
                if on_sep {
                    self.drag = Drag::Sep;
                    return;
                }
                if over_tree && self.git_active() {
                    self.enter_edit();
                    self.git_click(x, (y - l.top) as usize, self.clicks >= 2);
                    return;
                }
                if x >= l.ed_x && self.diff_shown() {
                    self.diff_click(y, m.modifiers.contains(KeyModifiers::SHIFT));
                    return;
                }
                if over_tree && y < l.tree_top {
                    return self.tree_search();
                }
                if over_tree {
                    self.enter_edit();
                    self.focus = Focus::Tree;
                    self.tree.typing = false;
                    let i = self.tree.scroll + (y - l.tree_top) as usize;
                    if i < self.tree.nodes.len() {
                        self.tree.sel = i;
                        if self.clicks >= 2 || self.tree.nodes[i].is_dir {
                            self.tree_activate(i);
                        } else {
                            self.tree_preview(i);
                        }
                    }
                    return;
                }
                if self.find.is_some() && self.find_click(x, y) {
                    return;
                }
                if x >= l.ed_x && x < l.ed_r && self.is_splash() {
                    let hit = self.splash_hits.borrow().iter().find(|(hy, _)| *hy == y).map(|h| h.1);
                    match hit {
                        Some(SPLASH_OPEN) => {
                            let d = self.tree.root.clone();
                            self.folder_picker(d, None, false);
                        }
                        Some(SPLASH_HELP) => self.open_help(),
                        Some(i) if i >= SPLASH_SESSION => self.open_session(i - SPLASH_SESSION + 1, 0),
                        Some(i) => {
                            if let Some(p) = self.recent.get(i).cloned() {
                                self.open_path(&p);
                            }
                        }
                        None => {}
                    }
                    return;
                }
                if x >= l.ed_x && x < l.ed_r {
                    self.focus = Focus::Editor;
                    self.popup = None;
                    self.sig = None;
                    let (cx, cy) = self.pos_from_mouse(x, y, &l);
                    let shift = m.modifiers.contains(KeyModifiers::SHIFT);
                    if shift && self.clicks == 1 {
                        if self.sel.is_none() {
                            self.sel = Some(self.cursor());
                        }
                    } else {
                        self.sel = None;
                    }
                    self.drag = Drag::Text;
                    self.drag_anchor = self.sel.unwrap_or((cx, cy));
                    let word = {
                        let line = &self.buf().lines[cy];
                        if cx < line.len() {
                            let k = cls(line[cx]);
                            let mut s = cx;
                            while s > 0 && cls(line[s - 1]) == k {
                                s -= 1;
                            }
                            let mut e = cx;
                            while e < line.len() && cls(line[e]) == k {
                                e += 1;
                            }
                            Some((s, e))
                        } else {
                            None
                        }
                    };
                    let last = self.buf().last();
                    let b = self.buf_mut();
                    b.cx = cx;
                    b.cy = cy;
                    match (self.clicks, word) {
                        (2, Some((s, e))) => {
                            self.buf_mut().cx = e;
                            self.sel = Some((s, cy));
                            self.drag_anchor = (s, cy);
                        }
                        (3, _) => {
                            self.sel = Some((0, cy));
                            let b = self.buf_mut();
                            if cy < last {
                                b.cy = cy + 1;
                                b.cx = 0;
                            } else {
                                b.cx = b.line_len(cy);
                            }
                        }
                        _ => {}
                    }
                    self.buf_mut().sync_want();
                    if m.modifiers.contains(KeyModifiers::CONTROL) && self.clicks == 1 {
                        self.drag = Drag::None;
                        self.lsp_definition();
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if in_text_rows && x >= l.ed_x && !over_tree {
                    self.focus = Focus::Editor;
                    self.mode = Mode::Edit;
                    if self.sel_range().is_none() {
                        let (cx, cy) = self.pos_from_mouse(x, y, &l);
                        self.sel = None;
                        let b = self.buf_mut();
                        b.cx = cx;
                        b.cy = cy;
                        b.sync_want();
                    }
                    self.open_context_menu(x, y + 1);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.drag {
                Drag::Sep => {
                    let max = self.w.saturating_sub(20).max(12);
                    self.tree.width = x.clamp(12, max);
                }
                Drag::Text => {
                    let p = self.pos_from_mouse(x, y, &l);
                    if self.sel.is_none() && p == self.cursor() {
                        return;
                    }
                    if self.sel.is_none() {
                        self.sel = Some(self.drag_anchor);
                    }
                    let b = self.buf_mut();
                    b.cx = p.0;
                    b.cy = p.1;
                    b.sync_want();
                }
                Drag::None | Drag::DbgSep => {}
            },
            MouseEventKind::Up(_) => self.drag = Drag::None,
            MouseEventKind::Down(MouseButton::Middle) => {
                if let Some((_, _, i, _)) = self.tab_at(x, y) {
                    self.close_tab(i);
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d: isize = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
                if over_tree && self.git_active() {
                    if let Some(g) = self.git.as_mut() {
                        g.scroll = (g.scroll as isize + d).max(0) as usize;
                    }
                } else if over_tree {
                    let max = self.tree.nodes.len().saturating_sub(1) as isize;
                    self.tree.scroll = (self.tree.scroll as isize + d).clamp(0, max.max(0)) as usize;
                } else if self.diff_shown() {
                    self.diff_wheel(d);
                } else {
                    self.wheel(d, &l);
                }
            }
            MouseEventKind::ScrollRight | MouseEventKind::ScrollLeft => {
                if over_tree || over_dbg || !in_text_rows || self.is_splash() || self.diff_shown() {
                    return;
                }
                let (w, h) = (l.text_w as usize, l.text_h as usize);
                let b = self.buf_mut();
                if m.kind == MouseEventKind::ScrollRight {
                    // Only as far as the widest line on screen needs: its end stays in view.
                    let end = (b.scroll_y + h).min(b.lines.len());
                    let widest = (b.scroll_y.min(end)..end).map(|y| b.vwidth(y)).max().unwrap_or(0);
                    let max = (widest + 1).saturating_sub(w);
                    b.scroll_x = (b.scroll_x + 4).min(max.max(b.scroll_x));
                } else {
                    b.scroll_x = b.scroll_x.saturating_sub(4);
                }
                // The cursor comes along while its line reaches into the view;
                // on a shorter line it stays put, off screen, until the next key.
                let col = b.vcol(b.cy, b.cx);
                if col < b.scroll_x && b.vwidth(b.cy) >= b.scroll_x {
                    b.cx = b.idx_at_vcol(b.cy, b.scroll_x);
                } else if col >= b.scroll_x + w {
                    b.cx = b.idx_at_vcol(b.cy, b.scroll_x + w - 1);
                }
                self.hscroll_free = true;
            }
            _ => {}
        }
    }

    fn wheel(&mut self, d: isize, l: &Layout) {
        if self.buf().md_view {
            return self.md_scroll_by(d);
        }
        let h = l.text_h as usize;
        let b = self.buf_mut();
        let max = b.last() as isize;
        b.scroll_y = (b.scroll_y as isize + d).clamp(0, max) as usize;
        if b.cy < b.scroll_y {
            b.cy = b.scroll_y;
        } else if b.cy >= b.scroll_y + h {
            b.cy = b.scroll_y + h - 1;
        }
        b.cy = b.cy.min(b.last());
        b.cx = idx_at_col(&b.lines[b.cy], b.want_col);
    }
}

/// Splits a command line into words. Quotes are optional and only needed for
/// arguments with spaces: `touch "/src/my file.rs"` = one path argument.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                has = true;
            }
            None if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            None => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Quotes a path for the command line when it contains spaces.
pub fn quote_arg(p: &str) -> String {
    if p.contains(' ') {
        format!("\"{}\"", p)
    } else {
        p.to_string()
    }
}

/// Which rendering `md_cache` holds: tab, file, revision, width.
type MdKey = (usize, Option<PathBuf>, u64, usize);

/// Text width of the rendered Markdown view: the editor less a margin on
/// each side, at most `markdown::MAX_WIDTH`.
pub fn md_width(l: &Layout) -> usize {
    (l.ed_w as usize).saturating_sub(4).min(crate::markdown::MAX_WIDTH)
}

impl Editor {
    /// The tab on screen rendered as Markdown, `width` wide (cached).
    pub(crate) fn md_lines(&self, width: usize) -> std::cell::Ref<'_, Vec<crate::markdown::Line>> {
        let b = self.buf();
        let key: MdKey = (self.cur, b.path.clone(), b.rev, width);
        {
            let mut c = self.md_cache.borrow_mut();
            if c.as_ref().map(|x| &x.0) != Some(&key) {
                *c = Some((key, crate::markdown::render(&b.lines, width)));
            }
        }
        std::cell::Ref::map(self.md_cache.borrow(), |c| &c.as_ref().unwrap().1)
    }

    /// Ctrl+Shift+V, `:preview`, the status bar switch: a Markdown file
    /// rendered, or back to its text, keeping the place.
    pub(crate) fn toggle_md_view(&mut self) {
        if self.buf().lang.name != "Markdown" {
            return self.error("The preview is for Markdown files (.md)");
        }
        let l = self.layout();
        let width = md_width(&l);
        let h = l.text_h as usize;
        self.popup = None;
        self.sel = None;
        self.focus = Focus::Editor;
        if self.buf().md_view {
            let src = self.md_lines(width).get(self.buf().md_scroll).map(|x| x.src);
            let b = self.buf_mut();
            b.md_view = false;
            if let Some(src) = src {
                b.scroll_y = src.min(b.last());
                if b.cy < b.scroll_y || b.cy >= b.scroll_y + h {
                    b.cy = b.scroll_y;
                    b.cx = 0;
                    b.sync_want();
                }
            }
            self.info("Editing the file · Ctrl+Shift+V or :preview shows it rendered");
        } else {
            let top = self.buf().scroll_y;
            // The first rendered row of what was at the top (not a spacer).
            let at = self
                .md_lines(width)
                .iter()
                .position(|x| x.src >= top && (!x.cells.is_empty() || x.fill.is_some()))
                .unwrap_or(0);
            let b = self.buf_mut();
            b.md_view = true;
            b.md_scroll = at;
            self.md_scroll_by(0);
            self.info("Preview · Ctrl+Shift+V, :preview or Esc goes back to the file");
        }
    }

    /// Scrolls the rendered view by `d` rows (0: just keeps it in range).
    fn md_scroll_by(&mut self, d: isize) {
        let l = self.layout();
        let n = self.md_lines(md_width(&l)).len();
        let max = n.saturating_sub(l.text_h as usize) as isize;
        let b = self.buf_mut();
        b.md_scroll = (b.md_scroll as isize + d).clamp(0, max.max(0)) as usize;
    }

    /// Keys in the rendered view: it scrolls; Esc or Enter goes back to the text.
    fn md_key(&mut self, k: KeyEvent) {
        let h = self.layout().text_h as isize;
        match k.code {
            KeyCode::Up => self.md_scroll_by(-1),
            KeyCode::Down => self.md_scroll_by(1),
            KeyCode::PageUp => self.md_scroll_by(-h),
            KeyCode::PageDown | KeyCode::Char(' ') => self.md_scroll_by(h),
            KeyCode::Home => self.md_scroll_by(isize::MIN / 2),
            KeyCode::End => self.md_scroll_by(isize::MAX / 2),
            KeyCode::Esc | KeyCode::Enter => self.toggle_md_view(),
            _ => self.info("The preview is read-only: Esc, Ctrl+Shift+V or :preview to edit the file"),
        }
    }

    /// The mouse over the rendered view: the wheel scrolls it, a click only
    /// focuses it, and resting on it opens nothing. The status bar's
    /// Preview / Raw switch toggles. True when handled.
    fn md_mouse(&mut self, m: &MouseEvent) -> bool {
        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
            if let Some((a, e)) = self.md_chip.get() {
                if m.row == self.h.saturating_sub(1) && m.column >= a && m.column < e {
                    self.toggle_md_view();
                    return true;
                }
            }
        }
        if !self.buf().md_view || self.menu.is_some() {
            return false;
        }
        let l = self.layout();
        let in_text = m.row >= l.top
            && m.row < l.top + l.text_h
            && m.column >= l.ed_x
            && !(l.dbg_h > 0 && m.row >= l.dbg_y && m.column >= l.dbg_x);
        if !in_text {
            return false;
        }
        match m.kind {
            MouseEventKind::ScrollDown => self.md_scroll_by(3),
            MouseEventKind::ScrollUp => self.md_scroll_by(-3),
            MouseEventKind::Down(_) => {
                self.focus = Focus::Editor;
                self.mode = Mode::Edit;
            }
            MouseEventKind::Up(_) => self.drag = Drag::None,
            _ => {}
        }
        self.rest = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        assert_eq!(split_args("touch /src/file.rs"), vec!["touch", "/src/file.rs"]);
        assert_eq!(split_args("touch \"/src/file.rs\""), vec!["touch", "/src/file.rs"]);
        assert_eq!(split_args("cp  \"a b.rs\"   c.rs "), vec!["cp", "a b.rs", "c.rs"]);
        assert_eq!(split_args("rm ''"), vec!["rm", ""]);
        assert_eq!(quote_arg("/a b"), "\"/a b\"");
        assert_eq!(quote_arg("/ab"), "/ab");
    }
}
