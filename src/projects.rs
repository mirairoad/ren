//! Several projects open at once. The first spot of the tab bar is a selector
//! that shows the project on screen; its list switches to another open project
//! or opens a folder as a new one.
//!
//! Each project keeps its own tabs, tree, language servers, project scan,
//! debugger, terminals and git panel. Switching parks the project on screen as
//! it is (its servers keep running, its terminals keep reading output) and
//! brings the other one back. Containers are shared: they belong to the Docker
//! socket, not to a folder.

use super::*;
use crate::fileops::Bin;
use ide::MenuAction;

/// Title of the project list, so the tab bar can show the selector as open.
pub const PROJECTS_TITLE: &str = "Projects";
/// The longest project name the selector shows before it cuts it with `…`.
const NAME_MAX: usize = 28;

/// What a project keeps while another one is on screen. The fields mirror the
/// editor's; `swap_project` exchanges them.
pub struct Project {
    bufs: Vec<Buffer>,
    cur: usize,
    tree: Tree,
    focus: Focus,
    sel: Option<Pos>,
    jumps: Vec<(PathBuf, Pos)>,
    lsp: Lsp,
    lsp_idle_gen: u64,
    scan: scan::Scan,
    find: Option<FindBar>,
    dbg: debug_ui::Debugger,
    panel_tab: PanelTab,
    bin: Option<Bin>,
    gitignore_asked: bool,
    terms: Vec<crate::term::Session>,
    term_sel: usize,
    term_err: Option<String>,
    git: Option<git_ui::GitView>,
    commit: Option<git_ui::CommitForm>,
}

impl Project {
    /// A folder just opened: an empty tab, the tree shown and focused.
    fn new(root: PathBuf, tree_w: u16, panel_h: u16) -> Self {
        let mut tree = Tree::new(root);
        tree.visible = true;
        tree.width = tree_w;
        let mut dbg = debug_ui::Debugger::new();
        dbg.height = panel_h;
        Project {
            bufs: vec![Buffer::scratch(NONAME)],
            cur: 0,
            tree,
            focus: Focus::Tree,
            sel: None,
            jumps: Vec::new(),
            lsp: Lsp::new(),
            lsp_idle_gen: 0,
            scan: Default::default(),
            find: None,
            dbg,
            panel_tab: PanelTab::Debug,
            bin: None,
            gitignore_asked: false,
            terms: Vec::new(),
            term_sel: 0,
            term_err: None,
            git: None,
            commit: None,
        }
    }
}

/// The first tab with changes that would be lost: a file, or typed-in text.
fn first_unsaved(bufs: &[Buffer]) -> Option<usize> {
    bufs.iter()
        .position(|b| b.dirty && (b.path.is_some() || b.name == NONAME))
}

fn folder_name(root: &Path) -> String {
    root.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

/// `~/x` for paths in the home folder.
fn tilde(p: &Path) -> String {
    match crate::settings::home().and_then(|h| p.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(r) if r.as_os_str().is_empty() => "~".into(),
        Some(r) => format!("~/{}", r.display()),
        None => p.display().to_string(),
    }
}

/// The project's name as the selector shows it: the folder's name, cut to
/// `NAME_MAX` characters.
pub fn selector_label(root: &Path) -> String {
    let name = folder_name(root);
    if name.chars().count() <= NAME_MAX {
        return name;
    }
    let mut s: String = name.chars().take(NAME_MAX - 1).collect();
    s.push('…');
    s
}

impl Editor {
    /// Exchanges everything that belongs to a project between the editor and
    /// `p`. Doing it twice puts everything back.
    fn swap_project(&mut self, p: &mut Project) {
        use std::mem::swap;
        swap(&mut self.bufs, &mut p.bufs);
        swap(&mut self.cur, &mut p.cur);
        swap(&mut self.tree, &mut p.tree);
        swap(&mut self.focus, &mut p.focus);
        swap(&mut self.sel, &mut p.sel);
        swap(&mut self.jumps, &mut p.jumps);
        swap(&mut self.lsp, &mut p.lsp);
        swap(&mut self.lsp_idle_gen, &mut p.lsp_idle_gen);
        swap(&mut self.scan, &mut p.scan);
        swap(&mut self.find, &mut p.find);
        swap(&mut self.dbg, &mut p.dbg);
        swap(&mut self.panel_tab, &mut p.panel_tab);
        swap(&mut self.bin, &mut p.bin);
        swap(&mut self.gitignore_asked, &mut p.gitignore_asked);
        swap(&mut self.terms, &mut p.terms);
        swap(&mut self.term_sel, &mut p.term_sel);
        swap(&mut self.term_err, &mut p.term_err);
        swap(&mut self.git, &mut p.git);
        swap(&mut self.commit, &mut p.commit);
    }

    /// Runs `f` with parked project `j` on screen, then parks it again, with
    /// none of the side effects of switching (used to save and restore the
    /// sessions of all projects on `:reload`).
    pub(crate) fn with_parked<R>(&mut self, j: usize, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let mut p = self.projects.get_mut(j)?.take()?;
        self.swap_project(&mut p);
        let r = f(self);
        self.swap_project(&mut p);
        self.projects[j] = Some(p);
        Some(r)
    }

    /// The folder of project `i` (the one on screen too).
    pub(crate) fn project_root(&self, i: usize) -> &Path {
        match self.projects.get(i) {
            Some(Some(p)) => &p.tree.root,
            _ => &self.tree.root,
        }
    }

    pub fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// Folders of all open projects, in selector order.
    pub(crate) fn project_roots(&self) -> Vec<PathBuf> {
        (0..self.projects.len())
            .map(|i| self.project_root(i).to_path_buf())
            .collect()
    }

    fn project_unsaved(&self, i: usize) -> bool {
        match self.projects.get(i) {
            Some(Some(p)) => first_unsaved(&p.bufs).is_some(),
            _ => first_unsaved(&self.bufs).is_some(),
        }
    }

    /// Before quitting: a parked project with unsaved changes comes on screen
    /// with its first unsaved tab, and quitting stops. True = nothing to lose.
    pub(crate) fn parked_saved(&mut self) -> bool {
        let Some(j) = (0..self.projects.len()).find(|&j| j != self.proj && self.project_unsaved(j)) else {
            return true;
        };
        self.switch_project(j);
        if let Some(i) = first_unsaved(&self.bufs) {
            self.cur = i;
            self.focus = Focus::Editor;
            let name = self.bufs[i].name.clone();
            let project = self.tree.root_name();
            self.error(format!("\"{}\" in {} is not saved (:qa! to discard)", name, project));
        }
        false
    }

    /// On quit: language servers of every project shut down.
    pub fn stop_servers(&mut self) {
        self.lsp.stop_all();
        for p in self.projects.iter_mut().flatten() {
            p.lsp.stop_all();
        }
    }

    /// Parked terminals keep reading their output (so a long build doesn't
    /// pile it up) and notice when their shell exits.
    pub(crate) fn parked_tick(&mut self) {
        for p in self.projects.iter_mut().flatten() {
            for t in p.terms.iter_mut() {
                t.poll();
            }
        }
    }

    /// Closes popups and lists that belong to the project on screen.
    fn park_ui(&mut self) {
        self.enter_edit();
        self.menu = None;
        self.hover = None;
        self.peek = None;
        self.rest = None;
        self.cmd_comp = None;
        self.cmd_hint = None;
        self.tab_hover = None;
        self.term_hover = None;
        self.ws_sym_id = None;
        self.ca_pending = 0;
        self.ca_menu_open = false;
        self.last_edit = None;
        self.drag = Drag::None;
        self.hscroll_free = false;
        // Keyed by tab index and revision, which the next project reuses.
        *self.find_cache.borrow_mut() = None;
        // The session is saved as it was before the switch.
        let _ = self.save_session(false, false);
    }

    /// After a project comes on screen: its folder becomes the working
    /// directory, its settings load, and files changed meanwhile reload.
    fn enter_project(&mut self, note: String) {
        let _ = env::set_current_dir(&self.tree.root);
        let errs = self.load_settings();
        self.reload_from_disk();
        if errs.is_empty() {
            self.info(note);
        } else {
            self.error(errs.join("; "));
        }
    }

    pub(crate) fn switch_project(&mut self, j: usize) {
        if j == self.proj {
            return self.info(format!("Project: {}", self.tree.root_name()));
        }
        let Some(mut p) = self.projects.get_mut(j).and_then(Option::take) else {
            return;
        };
        self.park_ui();
        self.swap_project(&mut p);
        self.projects[self.proj] = Some(p);
        self.proj = j;
        let note = format!("Project: {}", self.tree.root_name());
        self.enter_project(note);
    }

    /// Opens a folder as a new project (or switches to it when it's open).
    pub(crate) fn open_project(&mut self, dir: &Path) {
        let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        if !dir.is_dir() {
            return self.error(format!("Not a folder: {}", dir.display()));
        }
        let key = path_key(&dir);
        if let Some(j) = (0..self.projects.len()).find(|&j| path_key(self.project_root(j)) == key) {
            return self.switch_project(j);
        }
        let p = Project::new(dir.clone(), self.tree.width, self.dbg.height);
        self.projects.push(Some(Box::new(p)));
        self.switch_project(self.projects.len() - 1);
        self.push_recent(&dir);
        if !matches!(self.msg, Some((_, true))) {
            self.info(format!("Opened project: {}", self.tree.root_name()));
        }
    }

    /// `:project close`: the project on screen closes (its servers stop, its
    /// terminals and debug session end) and the next one comes on screen.
    fn close_project(&mut self) {
        if self.projects.len() < 2 {
            return self.error("This is the only project open");
        }
        if let Some(i) = first_unsaved(&self.bufs) {
            self.cur = i;
            let name = self.bufs[i].name.clone();
            return self.error(format!("\"{}\" is not saved: save it, or close it with :q!", name));
        }
        let name = self.tree.root_name();
        let j = if self.proj + 1 < self.projects.len() {
            self.proj + 1
        } else {
            self.proj - 1
        };
        let Some(mut p) = self.projects[j].take() else { return };
        self.park_ui();
        self.swap_project(&mut p);
        // `p` now holds the closing project.
        self.projects.remove(self.proj);
        self.proj = if j > self.proj { j - 1 } else { j };
        p.lsp.stop_all();
        drop(p);
        self.enter_project(format!("Closed {}", name));
    }

    /// The selector's list: open projects (the one on screen ticked), then
    /// "Open folder…".
    pub(crate) fn project_menu(&mut self) {
        self.enter_edit();
        let names: Vec<String> = self.project_roots().iter().map(|r| selector_label(r)).collect();
        let w = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
        let mut items: Vec<(String, MenuAction)> = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let mark = if i == self.proj { '✓' } else { ' ' };
            let dirty = if self.project_unsaved(i) { '●' } else { ' ' };
            let pad = w - name.chars().count();
            items.push((
                format!(
                    "{}  {}{} {}  {}",
                    mark,
                    name,
                    " ".repeat(pad),
                    dirty,
                    tilde(self.project_root(i))
                ),
                MenuAction::Run(format!("project {}", i + 1)),
            ));
        }
        items.push(("+  Open folder…".into(), MenuAction::Run("project open".into())));
        self.menu = Some(Menu {
            title: PROJECTS_TITLE.into(),
            items,
            sel: self.proj,
            scroll: 0,
            anchor: Some((self.proj_span.0, TAB_H)),
            dir: None,
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
        self.menu_follow();
    }

    /// `:project` lists the projects; `:project <n>` switches to the n-th;
    /// `:project <folder>` opens one (`:project open` browses for it);
    /// `:project close` closes the one on screen.
    pub(crate) fn project_command(&mut self, rest: &[String]) {
        match rest.first().map(String::as_str) {
            None | Some("list") => self.project_menu(),
            Some("close") => self.close_project(),
            Some("open") if rest.len() == 1 => {
                // Start in the folder around this project, on this project.
                let root = self.tree.root.clone();
                let dir = root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
                self.folder_picker(dir, Some(root), true);
            }
            Some(a) => {
                let arg = if a == "open" { rest[1].as_str() } else { a };
                if let Ok(n) = arg.parse::<usize>() {
                    if n == 0 || n > self.projects.len() {
                        return self.error(format!("No project {} (1–{} are open)", n, self.projects.len()));
                    }
                    return self.switch_project(n - 1);
                }
                // A real absolute path wins over `/x` = workspace root.
                let p = Path::new(arg);
                let dir = if p.is_absolute() && p.is_dir() {
                    p.to_path_buf()
                } else {
                    self.resolve_path(arg)
                };
                self.open_project(&dir);
            }
        }
    }
}

impl Editor {
    /// A saved project comes back parked (`session.rs` fills it in).
    pub(crate) fn add_parked(&mut self, root: PathBuf) -> usize {
        let p = Project::new(root, self.tree.width, self.dbg.height);
        self.projects.push(Some(Box::new(p)));
        self.projects.len() - 1
    }

    /// Puts the projects in the order of `roots` (others after them), the one
    /// on screen staying on screen.
    pub(crate) fn order_projects(&mut self, roots: &[PathBuf]) {
        let keys: Vec<String> = roots.iter().map(|r| path_key(r)).collect();
        let mut perm: Vec<usize> = (0..self.projects.len()).collect();
        perm.sort_by_key(|&i| {
            let k = path_key(self.project_root(i));
            keys.iter().position(|x| *x == k).unwrap_or(usize::MAX)
        });
        let mut old: Vec<Option<Option<Box<Project>>>> = self.projects.drain(..).map(Some).collect();
        self.projects = perm.iter().map(|&i| old[i].take().flatten()).collect();
        self.proj = perm.iter().position(|&i| i == self.proj).unwrap_or(0);
    }

    /// Closes parked project `j` (the empty start a restored session replaces).
    pub(crate) fn drop_parked(&mut self, j: usize) {
        if j == self.proj || j >= self.projects.len() {
            return;
        }
        if let Some(mut p) = self.projects.remove(j) {
            p.lsp.stop_all();
        }
        if j < self.proj {
            self.proj -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_names_are_cut() {
        assert_eq!(selector_label(Path::new("/a/ren")), "ren");
        let long = format!("/a/{}", "x".repeat(40));
        let label = selector_label(Path::new(&long));
        assert_eq!(label.chars().count(), NAME_MAX);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn unsaved_needs_a_file_or_typed_text() {
        let mut out = Buffer::scratch("[!ls]");
        out.dirty = true;
        assert_eq!(first_unsaved(&[out]), None);
        let mut typed = Buffer::scratch(NONAME);
        typed.dirty = true;
        assert_eq!(first_unsaved(&[Buffer::scratch(NONAME), typed]), Some(1));
    }
}
