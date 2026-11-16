//! `:git`: the git panel (replaces the file tree), the side-by-side diff in the
//! preview area, line staging, branches, stash, push/pull, and the commit form
//! built from the templates in settings.

use super::*;
use crate::git::{self, Diff, RowKind, Status};
use crate::highlight::{LangDef, Tok};
use crate::render::{primary, put_clip, text_w, BAR, BRIGHT, DIM, ERR, POP, SEL, STR, TYPE};
use crate::screen::{Screen, Style};
use crate::settings::Template;
use crossterm::style::Color;
use ide::MenuAction;
use std::sync::mpsc::{channel, Receiver};

const RED_BG: Color = Color::Rgb {
    r: 0x45,
    g: 0x24,
    b: 0x28,
};
const GREEN_BG: Color = Color::Rgb {
    r: 0x22,
    g: 0x3d,
    b: 0x2b,
};
const EMPTY_BG: Color = Color::Rgb {
    r: 0x24,
    g: 0x27,
    b: 0x2e,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GRow {
    Head(bool),
    Item(bool, usize),
}

pub struct DiffView {
    pub path: String,
    pub staged: bool,
    pub untracked: bool,
    pub diff: Diff,
    pub cur: usize,
    pub anchor: Option<usize>,
    pub scroll: usize,
    /// Syntax colours per row: (left, right). Each side is highlighted as one
    /// file, so strings and comments spanning lines come out right.
    pub toks: Vec<(Vec<Tok>, Vec<Tok>)>,
}

fn diff_toks(lang: &LangDef, rows: &[git::Row]) -> Vec<(Vec<Tok>, Vec<Tok>)> {
    let mut st = [highlight::State::default(); 2];
    let mut side = |k: usize, line: &Option<(usize, String)>| match line {
        Some((_, t)) => highlight::highlight_line(lang, &t.chars().collect::<Vec<_>>(), &mut st[k]),
        None => Vec::new(),
    };
    rows.iter().map(|r| (side(0, &r.left), side(1, &r.right))).collect()
}

pub struct GitView {
    pub st: Status,
    pub sel: usize,
    pub scroll: usize,
    pub diff: Option<DiffView>,
    pub err: Option<String>,
    pub busy: Option<String>,
    rx: Option<Receiver<(String, Result<String, String>)>>,
}

/// A line of the commit form: text, style, and the cursor column when it is there.
type FormLine = (String, Style, Option<(usize, usize)>);

pub struct CommitForm {
    pub tpl: Option<usize>,
    pub values: Vec<String>,
    pub field: usize,
    pub cx: usize,
    pub error: Option<String>,
}

/// The template in use, or a single free "Message" field when none are set.
fn plain_template() -> Template {
    Template {
        format: vec!["{message}".into()],
        fields: vec![crate::settings::Field {
            key: "message".into(),
            label: "Message".into(),
            lines: 6,
            required: true,
            ..Default::default()
        }],
    }
}

impl Editor {
    fn git_root(&self) -> PathBuf {
        self.tree.root.clone()
    }

    pub(crate) fn git_active(&self) -> bool {
        self.git.is_some() && self.tree.visible
    }

    fn git_rows(&self) -> Vec<GRow> {
        let Some(g) = &self.git else { return Vec::new() };
        let mut v = vec![GRow::Head(true)];
        v.extend((0..g.st.staged.len()).map(|i| GRow::Item(true, i)));
        v.push(GRow::Head(false));
        v.extend((0..g.st.changes.len()).map(|i| GRow::Item(false, i)));
        v
    }

    fn git_entry(&self, r: GRow) -> Option<&git::Entry> {
        let g = self.git.as_ref()?;
        match r {
            GRow::Item(true, i) => g.st.staged.get(i),
            GRow::Item(false, i) => g.st.changes.get(i),
            _ => None,
        }
    }

    /// `:git` with no arguments: shows or hides the git panel.
    pub(crate) fn git_toggle(&mut self) {
        if self.git_active() {
            self.git = None;
            if self.focus == Focus::Diff {
                self.focus = Focus::Tree;
            }
            return;
        }
        let root = self.git_root();
        if !git::is_repo(&root) {
            return self.error("Not a git repository (:git init to create one)");
        }
        self.git = Some(GitView {
            st: Status::default(),
            sel: 1,
            scroll: 0,
            diff: None,
            err: None,
            busy: None,
            rx: None,
        });
        self.tree.visible = true;
        self.focus = Focus::Tree;
        self.mode = Mode::Edit;
        self.git_refresh();
        if let Some(i) = self.git_rows().iter().position(|r| matches!(r, GRow::Item(..))) {
            self.git_move(i);
        }
    }

    /// Re-reads `git status`, keeping the selection on the same file (same
    /// section if it's still there, else the other one).
    pub(crate) fn git_refresh(&mut self) {
        let root = self.git_root();
        let r = git::status(&root);
        let prev = self
            .git
            .as_ref()
            .and_then(|g| self.git_rows().get(g.sel).copied())
            .and_then(|row| {
                self.git_entry(row)
                    .map(|e| (matches!(row, GRow::Item(true, _)), e.path.clone()))
            });
        let Some(g) = self.git.as_mut() else { return };
        match r {
            Ok(st) => {
                g.st = st;
                g.err = None;
            }
            Err(e) => g.err = Some(e),
        }
        let rows = self.git_rows();
        let find = |staged: bool, path: &str| {
            rows.iter().position(|r| {
                matches!(r, GRow::Item(s, _) if *s == staged) && self.git_entry(*r).is_some_and(|e| e.path == path)
            })
        };
        let n = rows.len();
        let pick = prev.and_then(|(staged, p)| find(staged, &p).or_else(|| find(!staged, &p)));
        let first_item = rows.iter().position(|r| matches!(r, GRow::Item(..)));
        if let Some(g) = self.git.as_mut() {
            g.sel = pick
                .or(if matches!(rows.get(g.sel), Some(GRow::Item(..))) {
                    None
                } else {
                    first_item
                })
                .unwrap_or(g.sel);
            g.sel = g.sel.min(n.saturating_sub(1));
        }
        self.git_load_diff();
        if self.focus == Focus::Diff && self.git.as_ref().is_none_or(|g| g.diff.is_none()) {
            self.focus = Focus::Tree;
        }
    }

    /// Loads the diff of the selected file into the preview area.
    fn git_load_diff(&mut self) {
        let rows = self.git_rows();
        let Some(g) = self.git.as_ref() else { return };
        let Some(&row) = rows.get(g.sel) else { return };
        let Some(e) = self.git_entry(row).cloned() else {
            if let Some(g) = self.git.as_mut() {
                g.diff = None;
            }
            return;
        };
        let staged = matches!(row, GRow::Item(true, _));
        let root = self.git_root();
        let keep = g
            .diff
            .as_ref()
            .filter(|d| d.path == e.path && d.staged == staged)
            .map(|d| (d.cur, d.scroll));
        let untracked = e.code == '?';
        let diff = if untracked {
            let text = fs::read_to_string(root.join(&e.path)).unwrap_or_else(|_| "(binary or unreadable file)".into());
            let mut d = Diff::default();
            for (i, l) in text.lines().enumerate() {
                d.rows.push(git::Row {
                    kind: RowKind::Add,
                    left: None,
                    right: Some((i + 1, l.to_string())),
                    del: None,
                    add: None,
                });
            }
            d
        } else {
            let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-U100000"];
            if staged {
                args.push("--cached");
            }
            args.push("--");
            args.push(&e.path);
            match git::run(&root, &args) {
                Ok(t) => git::parse_diff(&t),
                Err(err) => {
                    let mut d = Diff::default();
                    d.header.push(err);
                    d
                }
            }
        };
        let (cur, scroll) = keep.unwrap_or((0, 0));
        let lang = highlight::detect(Path::new(&e.path));
        if let Some(g) = self.git.as_mut() {
            let cur = cur.min(diff.rows.len().saturating_sub(1));
            g.diff = Some(DiffView {
                path: e.path.clone(),
                staged,
                untracked,
                cur,
                anchor: None,
                scroll,
                toks: diff_toks(lang, &diff.rows),
                diff,
            });
        }
    }

    fn git_follow(&mut self) {
        let rows = self.layout().side_h.saturating_sub(3) as usize;
        if let Some(g) = self.git.as_mut() {
            if g.sel < g.scroll {
                g.scroll = g.sel;
            } else if g.sel >= g.scroll + rows.max(1) {
                g.scroll = g.sel + 1 - rows.max(1);
            }
        }
    }

    fn git_move(&mut self, to: usize) {
        let n = self.git_rows().len();
        if let Some(g) = self.git.as_mut() {
            g.sel = to.min(n.saturating_sub(1));
        }
        self.git_follow();
        self.git_load_diff();
    }

    /// Space: ticks or unticks (stage / unstage) the file or the whole section.
    fn git_toggle_row(&mut self, row: GRow) {
        let root = self.git_root();
        let Some(g) = self.git.as_ref() else { return };
        let (staged, paths): (bool, Vec<String>) = match row {
            GRow::Head(s) => (
                s,
                if s { &g.st.staged } else { &g.st.changes }
                    .iter()
                    .map(|e| e.path.clone())
                    .collect(),
            ),
            GRow::Item(s, _) => match self.git_entry(row) {
                Some(e) => {
                    let mut v = vec![e.path.clone()];
                    if let Some(o) = &e.orig {
                        v.push(o.clone());
                    }
                    (s, v)
                }
                None => return,
            },
        };
        if paths.is_empty() {
            return;
        }
        let r = if staged {
            unstage(&root, &paths)
        } else {
            stage(&root, &paths)
        };
        if let Err(e) = r {
            self.error(e);
        }
        self.git_refresh();
    }

    pub(crate) fn git_key(&mut self, k: KeyEvent) {
        let rows = self.git_rows();
        let Some(sel) = self.git.as_ref().map(|g| g.sel) else {
            return;
        };
        let page = self.layout().side_h.saturating_sub(4).max(1) as usize;
        match k.code {
            KeyCode::Down => self.git_move(sel + 1),
            KeyCode::Up => self.git_move(sel.saturating_sub(1)),
            KeyCode::PageDown => self.git_move(sel + page),
            KeyCode::PageUp => self.git_move(sel.saturating_sub(page)),
            KeyCode::Home => self.git_move(0),
            KeyCode::End => self.git_move(rows.len()),
            KeyCode::Char(' ') => {
                if let Some(&r) = rows.get(sel) {
                    self.git_toggle_row(r);
                }
            }
            KeyCode::Enter => self.git_open_selected(None),
            KeyCode::Right | KeyCode::Tab => {
                if self.git.as_ref().is_some_and(|g| g.diff.is_some()) {
                    self.focus = Focus::Diff;
                }
            }
            KeyCode::Esc => self.git_toggle(),
            KeyCode::Char(':') => self.start_cmd(),
            _ => {}
        }
    }

    /// Enter: opens the real file as a normal tab (optionally at a line).
    fn git_open_selected(&mut self, line: Option<usize>) {
        let rows = self.git_rows();
        let Some(sel) = self.git.as_ref().map(|g| g.sel) else {
            return;
        };
        let Some(e) = rows.get(sel).and_then(|r| self.git_entry(*r)).cloned() else {
            return;
        };
        let p = self.git_root().join(&e.path);
        if !p.exists() {
            return self.info("The file was deleted");
        }
        self.open_path(&p);
        self.focus = Focus::Editor;
        if let Some(l) = line {
            let b = self.buf_mut();
            b.cy = l.saturating_sub(1).min(b.last());
            b.cx = b.first_nb(b.cy);
            b.sync_want();
        }
    }

    pub(crate) fn diff_key(&mut self, k: KeyEvent) {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let page = self.layout().text_h.saturating_sub(3).max(1) as usize;
        let Some(d) = self.git.as_mut().and_then(|g| g.diff.as_mut()) else {
            self.focus = Focus::Tree;
            return;
        };
        let n = d.diff.rows.len();
        let mv = |d: &mut DiffView, to: usize| {
            if shift {
                if d.anchor.is_none() {
                    d.anchor = Some(d.cur);
                }
            } else {
                d.anchor = None;
            }
            d.cur = to.min(n.saturating_sub(1));
        };
        match k.code {
            KeyCode::Down => mv(d, d.cur + 1),
            KeyCode::Up => mv(d, d.cur.saturating_sub(1)),
            KeyCode::PageDown => mv(d, d.cur + page),
            KeyCode::PageUp => mv(d, d.cur.saturating_sub(page)),
            KeyCode::Home => mv(d, 0),
            KeyCode::End => mv(d, n),
            KeyCode::Char(' ') => return self.diff_stage_lines(),
            KeyCode::Enter => {
                let line = d
                    .diff
                    .rows
                    .get(d.cur)
                    .and_then(|r| r.right.as_ref().or(r.left.as_ref()))
                    .map(|x| x.0);
                return self.git_open_selected(line);
            }
            KeyCode::Esc | KeyCode::Left | KeyCode::Tab | KeyCode::BackTab => {
                d.anchor = None;
                self.focus = Focus::Tree;
                return;
            }
            KeyCode::Char(':') => return self.start_cmd(),
            _ => {}
        }
        self.diff_follow();
    }

    fn diff_follow(&mut self) {
        let h = self.layout().text_h.saturating_sub(1).max(1) as usize;
        if let Some(d) = self.git.as_mut().and_then(|g| g.diff.as_mut()) {
            if d.cur < d.scroll {
                d.scroll = d.cur;
            } else if d.cur >= d.scroll + h {
                d.scroll = d.cur + 1 - h;
            }
        }
    }

    /// Space in the diff: stages (or unstages) the selected lines, or the block
    /// of changes under the cursor when nothing is selected.
    fn diff_stage_lines(&mut self) {
        let root = self.git_root();
        let Some(d) = self.git.as_ref().and_then(|g| g.diff.as_ref()) else {
            return;
        };
        if d.untracked {
            let r = stage(&root, std::slice::from_ref(&d.path));
            if let Err(e) = r {
                self.error(e);
            }
            return self.git_refresh();
        }
        let rows = &d.diff.rows;
        let (mut a, mut b) = match d.anchor {
            Some(an) => (an.min(d.cur), an.max(d.cur)),
            None => (d.cur, d.cur),
        };
        if d.anchor.is_none() {
            if rows.get(d.cur).is_none_or(|r| r.kind == RowKind::Same) {
                return self.info("No change on this line");
            }
            while a > 0 && rows[a - 1].kind != RowKind::Same {
                a -= 1;
            }
            while b + 1 < rows.len() && rows[b + 1].kind != RowKind::Same {
                b += 1;
            }
        }
        let seqs: Vec<usize> = rows[a..=b.min(rows.len().saturating_sub(1))]
            .iter()
            .flat_map(|r| [r.del, r.add])
            .flatten()
            .collect();
        let Some(patch) = git::partial_patch(&d.diff, &seqs, d.staged) else {
            return self.info("No changes in the selection");
        };
        let mut args = vec!["apply", "--cached", "--recount"];
        if d.staged {
            args.push("--reverse");
        }
        args.push("-");
        let staged = d.staged;
        match git::run_input(&root, &args, &patch) {
            Ok(_) => self.info(if staged {
                "Unstaged the selected lines"
            } else {
                "Staged the selected lines"
            }),
            Err(e) => self.error(format!("git apply: {}", e)),
        }
        if let Some(d) = self.git.as_mut().and_then(|g| g.diff.as_mut()) {
            d.anchor = None;
        }
        self.git_refresh();
    }

    /// Clicks in the git panel (row relative to the panel top).
    pub(crate) fn git_click(&mut self, x: u16, row: usize, double: bool) {
        self.focus = Focus::Tree;
        match row {
            0 => self.branch_menu(),
            1 => {
                let hit = self.git_button_at(x);
                match hit {
                    Some("commit") => self.commit_open(),
                    Some("push") => self.git_background("push"),
                    Some("pull") => self.git_background("pull"),
                    Some("stash") => self.stash_menu(),
                    _ => {}
                }
            }
            _ => {
                let Some(g) = self.git.as_ref() else { return };
                let i = g.scroll + row - 2;
                let rows = self.git_rows();
                let Some(&r) = rows.get(i) else { return };
                let on_box = (x as usize) < if matches!(r, GRow::Head(_)) { 4 } else { 6 };
                self.git_move(i);
                if on_box {
                    self.git_toggle_row(r);
                } else if double {
                    self.git_open_selected(None);
                }
            }
        }
    }

    fn git_buttons() -> [(&'static str, &'static str); 4] {
        [
            ("✓ Commit", "commit"),
            ("↑ Push", "push"),
            ("↓ Pull", "pull"),
            ("⊟ Stash", "stash"),
        ]
    }

    fn git_button_at(&self, x: u16) -> Option<&'static str> {
        let mut bx = 1u16;
        for (label, id) in Self::git_buttons() {
            let w = text_w(label) as u16;
            if x >= bx && x < bx + w {
                return Some(id);
            }
            bx += w + 2;
        }
        None
    }

    pub(crate) fn diff_click(&mut self, y: u16, shift: bool) {
        let l = self.layout();
        self.focus = Focus::Diff;
        if let Some(d) = self.git.as_mut().and_then(|g| g.diff.as_mut()) {
            if y <= l.top {
                return;
            }
            let row = d.scroll + (y - l.top - 1) as usize;
            if row >= d.diff.rows.len() {
                return;
            }
            if shift {
                if d.anchor.is_none() {
                    d.anchor = Some(d.cur);
                }
            } else {
                d.anchor = None;
            }
            d.cur = row;
        }
    }

    pub(crate) fn diff_wheel(&mut self, delta: isize) {
        if let Some(d) = self.git.as_mut().and_then(|g| g.diff.as_mut()) {
            let max = d.diff.rows.len().saturating_sub(1) as isize;
            d.scroll = (d.scroll as isize + delta).clamp(0, max.max(0)) as usize;
        }
    }

    pub(crate) fn draw_git(&self, s: &mut Screen, l: &Layout) {
        let Some(g) = &self.git else { return };
        let w = l.tree_w;
        let focused = self.focus == Focus::Tree;
        let mut head = format!(" ⎇ {}", if g.st.branch.is_empty() { "?" } else { &g.st.branch });
        if g.st.ahead > 0 {
            head += &format!(" ↑{}", g.st.ahead);
        }
        if g.st.behind > 0 {
            head += &format!(" ↓{}", g.st.behind);
        }
        if g.st.upstream.is_none() && !g.st.branch.is_empty() {
            head += " (no upstream)";
        }
        head += " ▾";
        put_clip(
            s,
            0,
            l.top,
            &head,
            Style::fg(if focused { primary() } else { DIM }).bold(),
            w as usize,
        );
        let mut bx = 1u16;
        for (label, _) in Self::git_buttons() {
            put_clip(
                s,
                bx,
                l.top + 1,
                label,
                Style::fg(BRIGHT).on(BAR),
                (w.saturating_sub(bx)) as usize,
            );
            bx += text_w(label) as u16 + 2;
        }
        let rows = self.git_rows();
        let list_h = l.side_h.saturating_sub(2) as usize;
        let mut status_line = None;
        if let Some(b) = &g.busy {
            status_line = Some((b.clone(), DIM));
        } else if let Some(e) = &g.err {
            status_line = Some((e.clone(), ERR));
        }
        let list_h = if status_line.is_some() {
            list_h.saturating_sub(1)
        } else {
            list_h
        };
        for r in 0..list_h {
            let i = g.scroll + r;
            let Some(&row) = rows.get(i) else { break };
            let y = l.top + 2 + r as u16;
            let selected = i == g.sel;
            let mut st = Style::new();
            if selected && (focused || self.focus == Focus::Diff) {
                let bg = if focused { crate::render::SEL } else { BAR };
                st = Style::fg(BRIGHT).on(bg);
                s.fill(0, y, w, Style::new().on(bg));
            }
            match row {
                GRow::Head(staged) => {
                    let n = if staged { g.st.staged.len() } else { g.st.changes.len() };
                    let mark = if staged && n > 0 { "[x]" } else { "[ ]" };
                    let label = format!(" {} {} ({})", mark, if staged { "STAGED" } else { "CHANGES" }, n);
                    put_clip(s, 0, y, &label, st.bold(), w as usize);
                }
                GRow::Item(staged, _) => {
                    let Some(e) = self.git_entry(row) else { continue };
                    let mark = if staged { "[x]" } else { "[ ]" };
                    let code_col = match e.code {
                        'A' | '?' => STR,
                        'D' => ERR,
                        'U' => ERR,
                        _ => TYPE,
                    };
                    let x = put_clip(s, 0, y, &format!("  {} ", mark), st, w as usize);
                    let mut cst = st;
                    cst.fg = code_col;
                    let x = put_clip(s, x, y, &format!("{} ", e.code), cst, w.saturating_sub(x) as usize);
                    let name = match &e.orig {
                        Some(o) => format!("{} ← {}", e.path, o),
                        None => e.path.clone(),
                    };
                    put_clip(s, x, y, &name, st, w.saturating_sub(x) as usize);
                }
            }
        }
        if let Some((m, c)) = status_line {
            put_clip(s, 1, l.top + l.side_h - 1, &m, Style::fg(c), w as usize - 1);
        }
        for r in 0..l.side_h {
            s.put_char(l.tree_w, l.top + r, '│', Style::fg(primary()));
        }
    }

    /// Side-by-side diff in the editor area: old left, new right.
    pub(crate) fn draw_diff(&self, s: &mut Screen, l: &Layout) {
        let Some(d) = self.git.as_ref().and_then(|g| g.diff.as_ref()) else {
            return;
        };
        let focused = self.focus == Focus::Diff;
        let title = format!(
            " {}  {}  ·  Space: {} lines/block  ·  Enter: open file  ·  Esc: back",
            d.path,
            if d.untracked {
                "untracked"
            } else if d.staged {
                "staged"
            } else {
                "unstaged"
            },
            if d.staged { "unstage" } else { "stage" }
        );
        s.fill(l.ed_x, l.top, l.ed_w, Style::new().on(BAR));
        put_clip(
            s,
            l.ed_x,
            l.top,
            &title,
            Style::fg(if focused { BRIGHT } else { DIM }).on(BAR),
            l.ed_w as usize,
        );
        if d.diff.binary || (d.diff.rows.is_empty() && !d.diff.header.is_empty()) {
            let msg = if d.diff.binary {
                "Binary file".to_string()
            } else {
                d.diff.header.join(" ")
            };
            put_clip(s, l.ed_x + 2, l.top + 2, &msg, Style::fg(DIM), l.ed_w as usize - 2);
            return;
        }
        let half = l.ed_w.saturating_sub(1) / 2;
        let lx = l.ed_x;
        let rx = l.ed_x + half + 1;
        let rw = l.ed_r.saturating_sub(rx);
        let (sa, sb) = match d.anchor {
            Some(a) => (a.min(d.cur), a.max(d.cur)),
            None => (d.cur, d.cur),
        };
        for r in 0..l.text_h.saturating_sub(1) {
            let y = l.top + 1 + r;
            let i = d.scroll + r as usize;
            s.put_char(lx + half, y, '│', Style::fg(BAR));
            let Some(row) = d.diff.rows.get(i) else { continue };
            let in_sel = focused && i >= sa && i <= sb;
            if row.left.is_none() && row.right.is_none() {
                put_clip(s, lx + 1, y, "⋯", Style::fg(DIM), 2);
                put_clip(s, rx + 1, y, "⋯", Style::fg(DIM), 2);
                continue;
            }
            let (lbg, rbg) = match row.kind {
                RowKind::Same => (Color::Reset, Color::Reset),
                RowKind::Del => (RED_BG, EMPTY_BG),
                RowKind::Add => (EMPTY_BG, GREEN_BG),
                RowKind::Mod => (RED_BG, GREEN_BG),
            };
            let (lbg, rbg) = if in_sel { (SEL, SEL) } else { (lbg, rbg) };
            let (lt, rt) = d.toks.get(i).map(|t| (&t.0[..], &t.1[..])).unwrap_or_default();
            self.diff_side(s, lx, y, half, row.left.as_ref(), lbg, lt);
            self.diff_side(s, rx, y, rw, row.right.as_ref(), rbg, rt);
            if focused && i == d.cur {
                s.put_char(lx, y, '▶', Style::fg(primary()).on(lbg).bold());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn diff_side(
        &self,
        s: &mut Screen,
        x: u16,
        y: u16,
        w: u16,
        line: Option<&(usize, String)>,
        bg: Color,
        toks: &[Tok],
    ) {
        s.fill(x, y, w, Style::new().on(bg));
        let Some((n, text)) = line else { return };
        let num = format!("{:>5} ", n);
        put_clip(s, x, y, &num, Style::fg(DIM).on(bg), w as usize);
        let chars: Vec<char> = text.chars().collect();
        let mut cx = x + 6;
        let end = x + w;
        for (k, c) in chars.iter().enumerate() {
            let c = if *c == '\t' { ' ' } else { *c };
            let cw = crate::buffer::char_width(c, 0) as u16;
            if cx + cw > end {
                break;
            }
            let mut st = crate::render::tok_style(toks.get(k).copied().unwrap_or(Tok::Normal));
            st.bg = bg;
            cx += s.put_char(cx, y, c, st);
        }
    }

    // ---- branches, stash, push/pull ---------------------------------------

    pub(crate) fn branch_menu(&mut self) {
        let root = self.git_root();
        let (local, remote) = match git::branches(&root) {
            Ok(b) => b,
            Err(e) => return self.error(e),
        };
        let cur = self.git.as_ref().map(|g| g.st.branch.clone()).unwrap_or_else(|| {
            git::run(&root, &["branch", "--show-current"])
                .unwrap_or_default()
                .trim()
                .to_string()
        });
        let mut items: Vec<(String, MenuAction)> =
            vec![("+ New branch…".into(), MenuAction::Run("git-new-branch".into()))];
        for b in &local {
            let mark = if *b == cur { "● " } else { "  " };
            items.push((format!("{}{}", mark, b), MenuAction::Run(format!("git checkout {}", b))));
        }
        for b in remote {
            let short = b.split_once('/').map(|x| x.1).unwrap_or(&b);
            if local.iter().any(|l| l == short) {
                continue;
            }
            items.push((
                format!("  {}  (remote)", b),
                MenuAction::Run(format!("git checkout {}", b)),
            ));
        }
        let sel = items.iter().position(|i| i.0.starts_with("● ")).unwrap_or(0);
        self.menu = Some(Menu::filterable("Checkout branch".into(), items, sel));
    }

    fn stash_menu(&mut self) {
        let root = self.git_root();
        let list = git::run(&root, &["stash", "list"]).unwrap_or_default();
        let mut items: Vec<(String, MenuAction)> =
            vec![("⊟ Stash all changes".into(), MenuAction::Run("git stash".into()))];
        for l in list.lines() {
            let r = l.split(':').next().unwrap_or("").to_string();
            items.push((format!("  {}", l), MenuAction::Run(format!("git stash pick {}", r))));
        }
        self.menu = Some(Menu {
            title: "Stash".into(),
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

    fn stash_actions(&mut self, r: &str) {
        let items = vec![
            (
                "Pop (apply and remove)".to_string(),
                MenuAction::Run(format!("git stash pop {}", r)),
            ),
            (
                "Apply (keep it)".to_string(),
                MenuAction::Run(format!("git stash apply {}", r)),
            ),
            ("Drop".to_string(), MenuAction::Run(format!("git stash drop {}", r))),
        ];
        self.menu = Some(Menu {
            title: r.to_string(),
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

    /// Push/pull run in the background; the panel shows progress.
    fn git_background(&mut self, what: &'static str) {
        let root = self.git_root();
        let branch = git::run(&root, &["branch", "--show-current"])
            .unwrap_or_default()
            .trim()
            .to_string();
        let has_upstream = git::run(&root, &["rev-parse", "--abbrev-ref", "@{u}"]).is_ok();
        let args: Vec<String> = match what {
            "push" if !has_upstream && !branch.is_empty() => {
                let remote = git::run(&root, &["remote"]).unwrap_or_default();
                let remote = remote.lines().next().unwrap_or("origin").to_string();
                vec!["push".into(), "-u".into(), remote, branch]
            }
            "push" => vec!["push".into()],
            _ => vec!["pull".into()],
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            let r = git::run(&root, &a);
            let _ = tx.send((what.to_string(), r));
        });
        if self.git.is_none() {
            self.git_toggle();
        }
        if let Some(g) = self.git.as_mut() {
            g.busy = Some(format!("{}ing…", what));
            g.rx = Some(rx);
        }
        self.info(format!("git {}…", what));
    }

    /// Picks up finished push/pull results.
    pub fn git_tick(&mut self) -> bool {
        let Some(rx) = self.git.as_ref().and_then(|g| g.rx.as_ref()) else {
            return false;
        };
        let Ok((what, r)) = rx.try_recv() else { return false };
        if let Some(g) = self.git.as_mut() {
            g.busy = None;
            g.rx = None;
        }
        match r {
            Ok(out) => {
                let last = out.lines().last().unwrap_or("done").to_string();
                self.info(format!("git {}: {}", what, last));
            }
            Err(e) => self.error(format!("git {}: {}", what, e.lines().last().unwrap_or(&e))),
        }
        if what == "pull" {
            self.reload_from_disk();
        }
        self.git_refresh();
        true
    }

    /// After a checkout or pull: clean tabs reload from disk, vanished files close.
    pub(super) fn reload_from_disk(&mut self) {
        for i in 0..self.bufs.len() {
            let b = &self.bufs[i];
            let Some(p) = b.path.clone() else { continue };
            if b.dirty || !p.exists() {
                continue;
            }
            if let Ok(mut nb) = Buffer::open(&p) {
                if nb.lines != self.bufs[i].lines {
                    let b = &self.bufs[i];
                    nb.cy = b.cy.min(nb.last());
                    nb.cx = b.cx;
                    nb.scroll_y = b.scroll_y;
                    nb.clamp(true);
                    self.lsp_close_buf(i);
                    self.bufs[i] = nb;
                }
            }
        }
        let root = self.git_root();
        self.close_gone(&root);
        self.tree.refresh();
    }

    // ---- :git command line ------------------------------------------------

    pub(crate) fn git_command(&mut self, args: &[String]) {
        let root = self.git_root();
        let sub = args.first().map(String::as_str);
        let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
        match sub {
            None => self.git_toggle(),
            Some("init") => {
                if git::is_repo(&root) {
                    return self.info("Already a git repository");
                }
                if let Err(e) = git::run(&root, &["init"]) {
                    return self.error(format!("git init: {}", e));
                }
                let lines = git::default_ignores(&root);
                let n = crate::fileops::gitignore_add(&root, &lines).unwrap_or(0);
                self.tree.refresh();
                self.git = None;
                self.git_toggle();
                self.info(format!("Initialized a git repository · .gitignore: {} entries", n));
            }
            Some("add") | Some("reset") | Some("restore") => {
                if rest.is_empty() {
                    return self.error(format!("Usage: :git {} <path> …", sub.unwrap_or("")));
                }
                let paths: Vec<String> = rest
                    .iter()
                    .map(|p| {
                        let abs = self.resolve_path(p);
                        abs.strip_prefix(&root)
                            .map(|r| r.display().to_string())
                            .unwrap_or_else(|_| abs.display().to_string())
                    })
                    .collect();
                let r = if sub == Some("add") {
                    stage(&root, &paths)
                } else {
                    unstage(&root, &paths)
                };
                match r {
                    Ok(()) => self.info(format!(
                        "{} {}",
                        if sub == Some("add") { "staged" } else { "unstaged" },
                        paths.join(", ")
                    )),
                    Err(e) => self.error(e),
                }
                self.git_refresh();
            }
            Some("commit") => {
                if rest.is_empty() {
                    return self.commit_open();
                }
                let msg = rest.join(" ");
                self.git_commit(&msg);
            }
            Some("push") => self.git_background("push"),
            Some("pull") => self.git_background("pull"),
            Some("stash") => match rest.first().copied() {
                None | Some("push") => {
                    match git::run(&root, &["stash", "push"]) {
                        Ok(o) => self.info(o.lines().next().unwrap_or("Stashed").to_string()),
                        Err(e) => self.error(format!("git stash: {}", e)),
                    }
                    self.reload_from_disk();
                    self.git_refresh();
                }
                Some("list") => self.stash_menu(),
                Some("pick") => {
                    let r = rest.get(1).copied().unwrap_or("stash@{0}").to_string();
                    self.stash_actions(&r);
                }
                Some(op @ ("pop" | "apply" | "drop")) => {
                    let mut a = vec!["stash", op];
                    if let Some(r) = rest.get(1) {
                        a.push(r);
                    }
                    match git::run(&root, &a) {
                        Ok(_) => self.info(format!("git stash {}: done", op)),
                        Err(e) => self.error(format!("git stash {}: {}", op, e.lines().last().unwrap_or(&e))),
                    }
                    self.reload_from_disk();
                    self.git_refresh();
                }
                Some(o) => self.error(format!("Unknown :git stash option: {}", o)),
            },
            Some("branch") => match rest.first() {
                None => self.branch_menu(),
                Some(name) => match git::run(&root, &["branch", name]) {
                    Ok(_) => self.info(format!("Created branch {}", name)),
                    Err(e) => self.error(format!("git branch: {}", e)),
                },
            },
            Some("checkout") | Some("switch") => {
                let (create, name) = match rest.as_slice() {
                    ["-b", n, ..] | ["-c", n, ..] => (true, Some(n.to_string())),
                    [n, ..] => (false, Some(n.to_string())),
                    [] => (false, None),
                };
                let Some(name) = name else { return self.branch_menu() };
                self.git_checkout(&name, create);
            }
            Some(o) => self.error(format!(
                "Unknown :git command: {} (init, add, reset, commit, push, pull, stash, branch, checkout)",
                o
            )),
        }
    }

    fn git_checkout(&mut self, name: &str, create: bool) {
        if let Some(b) = self.bufs.iter().find(|b| b.dirty && b.path.is_some()) {
            let n = b.name.clone();
            return self.error(format!("Save or revert \"{}\" before switching branch", n));
        }
        let root = self.git_root();
        let r = if create {
            git::run(&root, &["switch", "-c", name])
        } else if let Some((remote, short)) = name.split_once('/').filter(|_| {
            git::run(
                &root,
                &["show-ref", "--verify", "--quiet", &format!("refs/remotes/{}", name)],
            )
            .is_ok()
        }) {
            let _ = remote;
            git::run(&root, &["switch", short]).or_else(|_| git::run(&root, &["switch", "-c", short, "--track", name]))
        } else {
            git::run(&root, &["switch", name])
        };
        match r {
            Ok(_) => {
                self.reload_from_disk();
                self.info(format!(
                    "On branch {}",
                    git::run(&root, &["branch", "--show-current"])
                        .unwrap_or_default()
                        .trim()
                ));
            }
            Err(e) => self.error(format!("git switch: {}", e.lines().last().unwrap_or(&e))),
        }
        self.git_refresh();
    }

    fn git_commit(&mut self, msg: &str) -> bool {
        let root = self.git_root();
        if msg.trim().is_empty() {
            self.error("Empty commit message");
            return false;
        }
        let staged = git::status(&root).map(|s| !s.staged.is_empty()).unwrap_or(false);
        if !staged {
            self.error("Nothing staged (tick files in :git or :git add <path>)");
            return false;
        }
        match git::run_input(&root, &["commit", "-F", "-"], msg) {
            Ok(_) => {
                self.info(format!("Committed: {}", msg.lines().next().unwrap_or("")));
                self.git_refresh();
                true
            }
            Err(e) => {
                self.error(format!("git commit: {}", e.lines().last().unwrap_or(&e)));
                false
            }
        }
    }

    // ---- commit form ------------------------------------------------------

    fn template(&self, tpl: Option<usize>) -> Template {
        match tpl.and_then(|i| self.settings.templates.get(i)) {
            Some((_, t)) => t.clone(),
            None => plain_template(),
        }
    }

    pub(crate) fn commit_open(&mut self) {
        let tpl = if self.settings.templates.is_empty() {
            None
        } else {
            Some(self.last_template.min(self.settings.templates.len() - 1))
        };
        let t = self.template(tpl);
        let branch = self.git.as_ref().map(|g| g.st.branch.clone()).unwrap_or_default();
        let values = t
            .fields
            .iter()
            .map(|f| {
                if f.key == "ticket" {
                    git::ticket_from_branch(&branch).unwrap_or_default()
                } else if !f.choices.is_empty() && !f.free {
                    f.choices[0].0.clone()
                } else {
                    String::new()
                }
            })
            .collect::<Vec<_>>();
        let first = values.iter().position(|v| v.is_empty()).unwrap_or(0);
        self.commit = Some(CommitForm {
            tpl,
            cx: 0,
            values,
            field: first,
            error: None,
        });
    }

    fn commit_submit(&mut self) {
        let Some(f) = self.commit.as_ref() else { return };
        let t = self.template(f.tpl);
        for (i, fl) in t.fields.iter().enumerate() {
            if fl.required && f.values.get(i).is_none_or(|v| v.trim().is_empty()) {
                let label = fl.label.clone();
                if let Some(f) = self.commit.as_mut() {
                    f.error = Some(format!("{} is required", label));
                    f.field = i;
                    f.cx = f.values[i].chars().count();
                }
                return;
            }
        }
        let msg = git::build_message(&t, &f.values);
        let tpl = f.tpl;
        if self.git_commit(&msg) {
            self.commit = None;
            if let Some(t) = tpl {
                self.last_template = t;
            }
        } else if let Some(f) = self.commit.as_mut() {
            f.error = self.msg.as_ref().map(|m| m.0.clone());
        }
    }

    pub(crate) fn commit_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(f) = self.commit.as_ref() else { return };
        let t = self.template(f.tpl);
        let nf = t.fields.len();
        let fi = f.field.min(nf.saturating_sub(1));
        let field = t.fields.get(fi).cloned().unwrap_or_default();
        let choices_only = !field.choices.is_empty() && !field.free;
        match k.code {
            KeyCode::Esc => {
                self.commit = None;
                return;
            }
            KeyCode::Char('s') if ctrl => return self.commit_submit(),
            KeyCode::Enter if ctrl => return self.commit_submit(),
            KeyCode::Char('t') if ctrl => {
                let n = self.settings.templates.len();
                if n > 1 {
                    let next = f.tpl.map(|i| (i + 1) % n);
                    self.last_template = next.unwrap_or(0);
                    self.commit_open();
                }
                return;
            }
            _ => {}
        }
        let Some(f) = self.commit.as_mut() else { return };
        f.error = None;
        let val: Vec<char> = f.values[fi].chars().collect();
        let set = |f: &mut CommitForm, v: Vec<char>, cx: usize| {
            f.values[fi] = v.into_iter().collect();
            f.cx = cx;
        };
        match k.code {
            KeyCode::Tab | KeyCode::Down => {
                f.field = (fi + 1) % nf.max(1);
                f.cx = f.values[f.field].chars().count();
            }
            KeyCode::BackTab | KeyCode::Up => {
                f.field = (fi + nf.max(1) - 1) % nf.max(1);
                f.cx = f.values[f.field].chars().count();
            }
            KeyCode::Left | KeyCode::Right if !field.choices.is_empty() => {
                let cur = field.choices.iter().position(|c| c.0 == f.values[fi]);
                let n = field.choices.len();
                let next = match (cur, k.code == KeyCode::Right) {
                    (Some(i), true) => (i + 1) % n,
                    (Some(i), false) => (i + n - 1) % n,
                    (None, true) => 0,
                    (None, false) => n - 1,
                };
                let v: Vec<char> = field.choices[next].0.chars().collect();
                let l = v.len();
                set(f, v, l);
            }
            KeyCode::Left => f.cx = f.cx.saturating_sub(1),
            KeyCode::Right => f.cx = (f.cx + 1).min(val.len()),
            KeyCode::Home => f.cx = 0,
            KeyCode::End => f.cx = val.len(),
            _ if choices_only => {}
            KeyCode::Enter => {
                if field.lines > 0 {
                    let mut v = val;
                    let cx = f.cx.min(v.len());
                    v.insert(cx, '\n');
                    set(f, v, cx + 1);
                } else {
                    f.field = (fi + 1) % nf.max(1);
                    f.cx = f.values[f.field].chars().count();
                }
            }
            KeyCode::Backspace => {
                let mut v = val;
                let cx = f.cx.min(v.len());
                if cx > 0 {
                    v.remove(cx - 1);
                    set(f, v, cx - 1);
                }
            }
            KeyCode::Delete => {
                let mut v = val;
                let cx = f.cx.min(v.len());
                if cx < v.len() {
                    v.remove(cx);
                    set(f, v, cx);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                let mut v = val;
                let cx = f.cx.min(v.len());
                v.insert(cx, c);
                set(f, v, cx + 1);
            }
            _ => {}
        }
    }

    pub(crate) fn commit_paste(&mut self, s: &str) {
        let Some(f) = self.commit.as_ref() else { return };
        let t = self.template(f.tpl);
        let fi = f.field;
        let multi = t.fields.get(fi).is_some_and(|x| x.lines > 0);
        let text: String = if multi { s.to_string() } else { s.replace('\n', " ") };
        if let Some(f) = self.commit.as_mut() {
            let mut v: Vec<char> = f.values[fi].chars().collect();
            let cx = f.cx.min(v.len());
            let ins: Vec<char> = text.chars().collect();
            let n = ins.len();
            v.splice(cx..cx, ins);
            f.values[fi] = v.into_iter().collect();
            f.cx = cx + n;
        }
    }

    pub(crate) fn draw_commit(&self, s: &mut Screen, l: &Layout) {
        self.commit_cursor.set(None);
        let Some(f) = &self.commit else { return };
        let t = self.template(f.tpl);
        let w = (l.ed_w.saturating_sub(4)).clamp(30, 90);
        let x0 = l.ed_x + l.ed_w.saturating_sub(w) / 2;
        let inner = w as usize - 4;
        let mut lines: Vec<FormLine> = Vec::new();
        let tname = match f.tpl {
            Some(i) => self.settings.templates.get(i).map(|t| t.0.clone()).unwrap_or_default(),
            None => "message".into(),
        };
        let hint = if self.settings.templates.len() > 1 {
            "  (Ctrl+T: next template)"
        } else {
            ""
        };
        lines.push((format!("Template: {}{}", tname, hint), Style::fg(DIM).on(POP), None));
        lines.push((String::new(), Style::new().on(POP), None));
        for (i, fl) in t.fields.iter().enumerate() {
            let active = i == f.field;
            let lab = format!("{}{}", fl.label, if fl.required { " *" } else { "" });
            lines.push((
                lab,
                Style::fg(if active { primary() } else { BRIGHT }).on(POP).bold(),
                None,
            ));
            let v = &f.values[i];
            if !fl.choices.is_empty() {
                let label = fl
                    .choices
                    .iter()
                    .find(|c| &c.0 == v)
                    .map(|c| c.1.clone())
                    .unwrap_or_else(|| v.clone());
                let shown = format!(
                    "◂ {} ▸{}",
                    if label.is_empty() { "—".into() } else { label },
                    if fl.free { "  (or type)" } else { "" }
                );
                let cur = if active && fl.free {
                    Some((i, 2 + v.chars().take(f.cx).count()))
                } else {
                    None
                };
                lines.push((shown, Style::fg(BRIGHT).on(if active { SEL } else { BAR }), cur));
            } else {
                let h = fl.lines.max(1);
                let vl: Vec<&str> = if v.is_empty() {
                    vec![""]
                } else {
                    v.split('\n').collect()
                };
                // cursor line/col within the value
                let before: String = v.chars().take(f.cx).collect();
                let cl = before.matches('\n').count();
                let cc = before.rsplit('\n').next().unwrap_or("").chars().count();
                for k in 0..h.max(vl.len().min(12)) {
                    let text = vl.get(k).copied().unwrap_or("");
                    let shown = if v.is_empty() && k == 0 && !fl.hint.is_empty() && !active {
                        fl.hint.clone()
                    } else {
                        text.to_string()
                    };
                    let st = if v.is_empty() && !fl.hint.is_empty() && !active {
                        Style::fg(DIM).on(BAR)
                    } else {
                        Style::fg(BRIGHT).on(if active { SEL } else { BAR })
                    };
                    let cur = if active && k == cl { Some((i, cc)) } else { None };
                    lines.push((shown, st, cur));
                }
                if active && !fl.hint.is_empty() {
                    lines.push((fl.hint.clone(), Style::fg(DIM).on(POP), None));
                }
            }
            lines.push((String::new(), Style::new().on(POP), None));
        }
        let msg = git::build_message(&t, &f.values);
        lines.push(("Message preview".into(), Style::fg(DIM).on(POP).bold(), None));
        for ml in msg.lines().take(6) {
            lines.push((ml.to_string(), Style::fg(TYPE).on(POP), None));
        }
        lines.push((String::new(), Style::new().on(POP), None));
        if let Some(e) = &f.error {
            lines.push((e.clone(), Style::fg(ERR).on(POP), None));
        }
        lines.push((
            "Ctrl+S commit · Tab/↑↓ field · ◂▸ choose · Esc cancel".into(),
            Style::fg(DIM).on(POP),
            None,
        ));
        let max_h = l.text_h.saturating_sub(1) as usize;
        let h = (lines.len() + 2).min(max_h) as u16;
        let y0 = l.top + (l.text_h.saturating_sub(h)) / 2;
        crate::render::draw_box(s, x0, y0, w, h, "Commit");
        for (r, (text, st, cur)) in lines.iter().take(h as usize - 2).enumerate() {
            let y = y0 + 1 + r as u16;
            if st.bg == SEL || st.bg == BAR {
                s.fill(x0 + 2, y, inner as u16, Style::new().on(st.bg));
            }
            put_clip(s, x0 + 2, y, text, *st, inner);
            if let Some((_, col)) = cur {
                let cx = (x0 + 2 + *col as u16).min(x0 + w - 3);
                self.commit_cursor.set(Some((cx, y)));
            }
        }
    }

    /// Clean tabs whose file vanished (e.g. after a checkout) are closed.
    fn close_gone(&mut self, root: &Path) {
        let mut i = 0;
        while i < self.bufs.len() {
            let gone = self.bufs[i]
                .path
                .as_ref()
                .is_some_and(|p| p.starts_with(root) && !p.exists())
                && !self.bufs[i].dirty;
            if gone {
                self.lsp_close_buf(i);
                self.bufs.remove(i);
                if self.cur >= i && self.cur > 0 {
                    self.cur -= 1;
                }
            } else {
                i += 1;
            }
        }
        if self.bufs.is_empty() {
            self.bufs.push(Buffer::scratch(NONAME));
            self.cur = 0;
        }
    }
}

/// `git add` (works for deletions too).
pub fn stage(root: &Path, paths: &[String]) -> Result<(), String> {
    let mut a: Vec<&str> = vec!["add", "-A", "--"];
    a.extend(paths.iter().map(String::as_str));
    git::run(root, &a).map(|_| ())
}

/// `git restore --staged`, or `git rm --cached` before the first commit.
pub fn unstage(root: &Path, paths: &[String]) -> Result<(), String> {
    let has_head = git::run(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
    let mut a: Vec<&str> = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "-r", "-q", "--"]
    };
    a.extend(paths.iter().map(String::as_str));
    git::run(root, &a).map(|_| ())
}
