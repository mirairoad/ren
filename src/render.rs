use crate::buffer::{char_width, disp_col};
use crate::editor::{Editor, Focus, Layout, Mode, MENU_ROWS, POPUP_ROWS, TAB_H};
use crate::highlight::{self, Tok};
use crate::icons;
use crate::lsp::from_lsp_col;
use crate::screen::{Screen, Style};
use crossterm::style::Color;
use std::sync::atomic::{AtomicU32, Ordering};
use unicode_width::UnicodeWidthStr;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

pub(crate) const DIM: Color = rgb(0x5c, 0x63, 0x70);
pub(crate) const BRIGHT: Color = rgb(0xe6, 0xe6, 0xe6);
const KW: Color = rgb(0xc6, 0x78, 0xdd);
pub(crate) const STR: Color = rgb(0x98, 0xc3, 0x79);
const NUM: Color = rgb(0xd1, 0x9a, 0x66);
const FUNC: Color = rgb(0x61, 0xaf, 0xef);
pub(crate) const TYPE: Color = rgb(0xe5, 0xc0, 0x7b);
pub(crate) const MACRO: Color = rgb(0x56, 0xb6, 0xc2);
const ESC: Color = rgb(0x56, 0xb6, 0xc2);
pub(crate) const ERR: Color = rgb(0xe0, 0x6c, 0x75);
pub(crate) const BAR: Color = rgb(0x21, 0x25, 0x2b);
pub(crate) const SEL: Color = rgb(0x3e, 0x44, 0x51);
const FIND: Color = rgb(0x5a, 0x4a, 0x1e);
const FIND_CUR: Color = rgb(0x8a, 0x6a, 0x1e);
const EXEC: Color = rgb(0x2e, 0x3b, 0x26);
const BP: Color = rgb(0xe5, 0x48, 0x4d);
const INK: Color = rgb(0x1b, 0x1e, 0x23);
/// Inlay hints: dim italic text on a faint box, so they don't read as code or comments.
const HINT: Color = rgb(0x8b, 0x92, 0xa0);
const HINT_BG: Color = rgb(0x2f, 0x34, 0x3e);
/// Where the `:` command text starts in the bottom bar: after " COMMAND " and a space.
const CMD_X: u16 = 11;
static PRIMARY: AtomicU32 = AtomicU32::new(crate::settings::DEFAULT_PRIMARY);

pub(crate) fn set_primary(v: u32) {
    PRIMARY.store(v, Ordering::Relaxed);
}

fn split(v: u32) -> (u8, u8, u8) {
    ((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub(crate) fn primary() -> Color {
    let (r, g, b) = split(PRIMARY.load(Ordering::Relaxed));
    rgb(r, g, b)
}

fn primary_tint() -> Color {
    let (r, g, b) = split(PRIMARY.load(Ordering::Relaxed));
    let (br, bg, bb) = (0x28u16, 0x2cu16, 0x34u16);
    let mix = |c: u8, base: u16| ((c as u16 * 30 + base * 70) / 100) as u8;
    rgb(mix(r, br), mix(g, bg), mix(b, bb))
}
pub(crate) const POP: Color = rgb(0x28, 0x2c, 0x34);
const BORDER: Color = rgb(0x4b, 0x52, 0x63);

/// Draws virtual text (an inlay hint) at screen column `v` of a text row,
/// clipped to the visible columns. Returns its width.
fn put_virtual(s: &mut Screen, l: &Layout, scroll_x: usize, sy: u16, v: usize, text: &str, st: Style) -> usize {
    let right = scroll_x + l.text_w as usize;
    let mut x = v;
    for ch in text.chars() {
        let w = char_width(ch, 0);
        if x >= right {
            break;
        }
        if x >= scroll_x && x + w <= right {
            s.put_char(l.text_x + (x - scroll_x) as u16, sy, ch, st);
        } else if x + w > scroll_x {
            let a = x.max(scroll_x);
            s.fill(
                l.text_x + (a - scroll_x) as u16,
                sy,
                ((x + w).min(right) - a) as u16,
                st,
            );
        }
        x += w;
    }
    text.width()
}

fn sev_color(sev: u8) -> Color {
    match sev {
        1 => ERR,
        2 => TYPE,
        3 => FUNC,
        _ => DIM,
    }
}

fn kind_info(k: u64) -> (&'static str, Color) {
    match k {
        1 => ("txt", DIM),
        2 | 3 => ("fn", FUNC),
        4 => ("new", FUNC),
        5 => ("fld", MACRO),
        6 => ("var", BRIGHT),
        7 => ("cls", TYPE),
        8 => ("ifc", TYPE),
        9 => ("mod", KW),
        10 => ("prp", MACRO),
        11 => ("unit", NUM),
        12 => ("val", NUM),
        13 => ("enum", TYPE),
        14 => ("kw", KW),
        15 => ("snip", STR),
        16 => ("clr", NUM),
        17 => ("file", DIM),
        18 => ("ref", DIM),
        19 => ("dir", DIM),
        20 => ("mem", TYPE),
        21 => ("cst", NUM),
        22 => ("st", TYPE),
        23 => ("evt", KW),
        24 => ("op", KW),
        25 => ("T", TYPE),
        _ => ("", DIM),
    }
}

pub(crate) fn text_w(t: &str) -> usize {
    t.chars().map(|c| char_width(c, 0)).sum()
}

pub(crate) fn put_clip(s: &mut Screen, x: u16, y: u16, t: &str, st: Style, max: usize) -> u16 {
    let mut cx = x;
    let end = x as usize + max;
    for c in t.chars() {
        let c = if c == '\t' { ' ' } else { c };
        let w = char_width(c, 0);
        if cx as usize + w > end {
            break;
        }
        cx += s.put_char(cx, y, c, st);
    }
    cx
}

pub(crate) fn draw_box(s: &mut Screen, x: u16, y: u16, w: u16, h: u16, title: &str) {
    let b = Style::fg(BORDER).on(POP);
    for r in 0..h {
        s.fill(x, y + r, w, Style::new().on(POP));
    }
    s.put_char(x, y, '╭', b);
    s.put_char(x + w - 1, y, '╮', b);
    s.put_char(x, y + h - 1, '╰', b);
    s.put_char(x + w - 1, y + h - 1, '╯', b);
    for i in 1..w - 1 {
        s.put_char(x + i, y, '─', b);
        s.put_char(x + i, y + h - 1, '─', b);
    }
    for r in 1..h - 1 {
        s.put_char(x, y + r, '│', b);
        s.put_char(x + w - 1, y + r, '│', b);
    }
    if !title.is_empty() && w > 6 {
        put_clip(
            s,
            x + 2,
            y,
            &format!(" {} ", title),
            Style::fg(BRIGHT).on(POP).bold(),
            w as usize - 4,
        );
    }
}

pub(crate) fn tok_style(t: Tok) -> Style {
    match t {
        Tok::Normal => Style::new(),
        Tok::Keyword => Style::fg(KW),
        Tok::Type => Style::fg(TYPE),
        Tok::Str => Style::fg(STR),
        Tok::Comment => Style::fg(DIM).italic(),
        Tok::Number => Style::fg(NUM),
        Tok::Func => Style::fg(FUNC),
        Tok::Macro => Style::fg(MACRO),
        Tok::Escape => Style::fg(ESC),
        // Quotes (and prefixes like r#, f, b) are string-coloured; placeholders read as code.
        Tok::Quote => Style::fg(STR),
        Tok::Interp => Style::new(),
    }
}

impl Editor {
    pub fn draw(&mut self, s: &mut Screen) {
        s.clear();
        let l = self.layout();
        self.draw_tabs(s);
        if self.git_active() {
            self.draw_git(s, &l);
        } else if self.tree.visible {
            self.draw_tree(s, &l);
        }
        if self.diff_shown() {
            self.draw_diff(s, &l);
        } else if self.is_splash() {
            self.draw_splash(s, &l);
        } else {
            self.draw_text(s, &l);
        }
        self.draw_panel(s, &l);
        self.draw_find(s, &l);
        self.draw_status(s);
        self.draw_cmd_comp(s);
        self.draw_sig(s, &l);
        self.draw_popup(s);
        self.draw_hover(s, &l);
        self.draw_commit(s, &l);
        self.draw_menu(s, &l);
    }

    pub fn cursor_pos(&self) -> Option<(u16, u16)> {
        match self.mode {
            Mode::Command => {
                let w: usize = self.cmd[..self.cmd_x].iter().map(|c| char_width(*c, 0)).sum();
                Some((
                    (CMD_X as usize + w).min(self.w as usize - 1) as u16,
                    self.h.saturating_sub(1),
                ))
            }
            _ if self.menu.is_some() => None,
            _ if self.commit.is_some() => self.commit_cursor.get(),
            _ if self.focus == Focus::Diff => None,
            _ if self.focus == Focus::Find => self.find_cursor.get(),
            _ if self.focus == Focus::Panel => self.dbg_cursor.get(),
            _ if self.focus == Focus::Tree => None,
            _ if self.is_splash() => None,
            _ => {
                let l = self.layout();
                let b = self.buf();
                let dc = b.vcol(b.cy, b.cx);
                if b.cy < b.scroll_y || dc < b.scroll_x {
                    return None;
                }
                let row = b.cy - b.scroll_y;
                let col = dc - b.scroll_x;
                if row >= l.text_h as usize || col >= l.text_w as usize {
                    return None;
                }
                Some((l.text_x + col as u16, l.top + row as u16))
            }
        }
    }

    pub fn bar_cursor(&self) -> bool {
        true
    }

    /// Two rows: a padding row on top, the labels below it. The open tab takes
    /// the editor's background, with a primary line just above its label.
    fn draw_tabs(&mut self, s: &mut Screen) {
        for y in 0..TAB_H {
            s.fill_row(y, Style::new().on(BAR));
        }
        let y = TAB_H - 1;
        self.tab_spans.clear();
        let mut x = 0u16;
        for (i, b) in self.bufs.iter().enumerate() {
            let active = i == self.cur;
            let mut st = if active {
                Style::fg(BRIGHT).bold()
            } else {
                Style::fg(DIM).on(BAR)
            };
            if b.preview {
                st = st.italic();
            }
            let x0 = x;
            let mut nx = s.put(x0, y, " ", st);
            // The icon slot turns into a close × while the mouse is over the tab.
            let close_x = nx;
            if self.tab_hover == Some(i) {
                let mut cst = st;
                cst.fg = ERR;
                cst.bold = true;
                nx += s.put_char(nx, y, '×', cst);
            } else if self.settings.icons {
                let (ic, col) = icons::file(&b.name);
                let mut ist = st;
                ist.fg = col;
                nx += s.put_char(nx, y, ic, ist);
            } else {
                nx = s.put(nx, y, " ", st);
            }
            let label = format!(" {}{} ", b.name, if b.dirty { " ●" } else { "" });
            let nx = s.put(nx, y, &label, st);
            // The open tab's line sits at the bottom of the padding row, right on
            // top of its label.
            if active {
                for c in x0..nx.min(s.w) {
                    s.put_char(c, y - 1, '▁', Style::fg(primary()).on(BAR));
                }
            }
            self.tab_spans.push((x0, nx, i, close_x));
            x = nx;
            if x >= s.w {
                break;
            }
        }
        let brand = " ren ";
        let bw = brand.len() as u16;
        if x + bw + 2 <= s.w {
            s.put(s.w - bw, y, brand, Style::fg(DIM).on(BAR));
        }
    }

    fn draw_tree(&self, s: &mut Screen, l: &Layout) {
        let t = &self.tree;
        let focused = self.focus == Focus::Tree;
        let w = l.tree_w;
        let header = format!(" {}", t.root_name().to_uppercase());
        s.put(
            0,
            l.top,
            &header,
            Style::fg(if focused { primary() } else { DIM }).bold(),
        );
        let open_path = self.buf().path.as_deref();
        let rows = l.side_h.saturating_sub(1) as usize;
        for r in 0..rows {
            let i = t.scroll + r;
            let y = l.top + 1 + r as u16;
            let Some(n) = t.nodes.get(i) else { break };
            let selected = i == t.sel;
            let mut st = if n.is_dir {
                Style::fg(BRIGHT)
            } else if Some(n.path.as_path()) == open_path {
                Style::fg(primary())
            } else {
                Style::new()
            };
            if selected && focused {
                let tint = primary_tint();
                st = Style::fg(BRIGHT).on(tint).bold();
                s.fill(0, y, w, Style::new().on(tint));
            }
            let arrow = if n.is_dir {
                if n.expanded {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };
            let lead = format!(" {}{}", "  ".repeat(n.depth), arrow);
            let mut x = put_clip(s, 0, y, &lead, st, w as usize);
            if self.settings.icons && x + 2 <= w {
                let (ic, col) = if n.is_dir {
                    if n.expanded {
                        icons::FOLDER_OPEN
                    } else {
                        icons::FOLDER
                    }
                } else {
                    icons::file(&n.name)
                };
                let mut ist = st;
                ist.fg = col;
                x += s.put_char(x, y, ic, ist);
                x += s.put_char(x, y, ' ', st);
            }
            put_clip(s, x, y, &n.name, st, w.saturating_sub(x) as usize);
        }
        for r in 0..l.side_h {
            s.put_char(l.tree_w, l.top + r, '│', Style::fg(primary()));
        }
    }

    fn draw_splash(&self, s: &mut Screen, l: &Layout) {
        const LOGO: [&str; 3] = ["█▀▄ █▀▀ █▀█  ", "█▀▄ █▀▀ █ █  ", "▀ ▀ ▀▀▀ ▀ ▀ ▄"];
        let keys: [(&str, &str); 6] = [
            ("Ctrl+K", "open folder"),
            ("Ctrl+B", "toggle file tree"),
            ("Ctrl+Esc", "command line  (Esc without kitty keys)"),
            (":touch f", "create a file"),
            (":help", "all shortcuts"),
            (":qa", "quit"),
        ];
        let mut hits = self.splash_hits.borrow_mut();
        hits.clear();
        let area_x = l.ed_x;
        let area_w = l.ed_w;
        let recent_n = self.recent.len().min(5);
        let mut need = 3 + 2 + 2 + keys.len();
        let show_recent = recent_n > 0 && (l.text_h as usize) >= need + 3 + recent_n;
        if show_recent {
            need += 2 + recent_n;
        }
        let show_keys = (l.text_h as usize) >= 7 + 3;
        if !show_keys {
            need = 5;
        }
        let mut y = l.top + l.text_h.saturating_sub(need as u16) / 2;
        let center = |w: usize| area_x + area_w.saturating_sub(w as u16) / 2;

        let logo_w = text_w(LOGO[0]);
        let lx = center(logo_w);
        for (ri, row) in LOGO.iter().enumerate() {
            let mut x = lx;
            let last = row.chars().count() - 1;
            for (ci, c) in row.chars().enumerate() {
                let st = if ri == LOGO.len() - 1 && ci == last {
                    Style::fg(BRIGHT)
                } else {
                    Style::fg(primary())
                };
                x += s.put_char(x, y, c, st);
            }
            y += 1;
        }
        y += 1;
        let tag = format!("refined terminal IDE  ·  v{}", env!("CARGO_PKG_VERSION"));
        s.put(center(text_w(&tag)), y, &tag, Style::fg(DIM));
        y += 2;
        if !show_keys {
            return;
        }
        let block_w = 10 + keys.iter().map(|k| text_w(k.1)).max().unwrap_or(0);
        let kx = center(block_w.max(36));
        for (k, d) in keys {
            s.put(kx, y, k, Style::fg(primary()));
            s.put(kx + 10, y, d, Style::fg(DIM));
            if k == "Ctrl+K" {
                hits.push((y, crate::editor::SPLASH_OPEN));
            } else if k == ":help" {
                hits.push((y, crate::editor::SPLASH_HELP));
            }
            y += 1;
        }
        if show_recent {
            y += 1;
            s.put(kx, y, "Recent folders", Style::fg(BRIGHT).bold());
            y += 1;
            let room = (area_w as usize).saturating_sub((kx - area_x) as usize + 2);
            for (i, p) in self.recent.iter().take(recent_n).enumerate() {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string());
                s.put(kx, y, &(i + 1).to_string(), Style::fg(primary()));
                let nx = put_clip(s, kx + 3, y, &name, Style::fg(BRIGHT), 22.min(room));
                let px = (kx + 3 + 22).max(nx + 2);
                let full = p.display().to_string();
                let left = room.saturating_sub((px - kx) as usize);
                let chars: Vec<char> = full.chars().collect();
                let shown = if chars.len() > left && left > 1 {
                    format!("…{}", chars[chars.len() - (left - 1)..].iter().collect::<String>())
                } else {
                    full
                };
                put_clip(s, px, y, &shown, Style::fg(DIM), left);
                hits.push((y, i));
                y += 1;
            }
        }
    }

    fn draw_text(&self, s: &mut Screen, l: &Layout) {
        let b = self.buf();
        let range = self.sel_range();
        let visual = range.is_some();
        let (sa, se) = range.unwrap_or_default();
        let in_sel = |x: usize, y: usize| -> bool { visual && (y, x) >= (sa.1, sa.0) && (y, x) < (se.1, se.0) };
        let pat = if self.hl_search { self.search.as_ref() } else { None };

        let mut hl = highlight::State::default();
        if b.lang.multiline() {
            for y in 0..b.scroll_y.min(b.lines.len()) {
                highlight::highlight_line(b.lang, &b.lines[y], &mut hl);
            }
        }
        let right = b.scroll_x + l.text_w as usize;
        let diags = self.buf_diags(self.cur);
        let cur_path = b.path.as_deref();
        let exec_line = match (&self.dbg.at, cur_path) {
            (Some((p, line)), Some(cp)) if crate::lsp::path_key(p) == crate::lsp::path_key(cp) => Some(*line),
            _ => None,
        };
        for row in 0..l.text_h {
            let sy = l.top + row;
            let ly = b.scroll_y + row as usize;
            if ly >= b.lines.len() {
                s.put(l.ed_x + l.gutter.saturating_sub(2), sy, "~", Style::fg(BAR));
                continue;
            }
            let line = &b.lines[ly];
            let mut ulm = vec![0u8; line.len() + 1];
            let mut sign = 0u8;
            if let Some(ds) = diags {
                for d in ds.iter().filter(|d| d.l0 <= ly && ly <= d.l1) {
                    let x0 = if d.l0 == ly { from_lsp_col(line, d.c0, d.enc) } else { 0 };
                    let mut x1 = if d.l1 == ly {
                        from_lsp_col(line, d.c1, d.enc)
                    } else {
                        line.len()
                    };
                    if x1 <= x0 {
                        x1 = x0 + 1;
                    }
                    let end = x1.min(line.len() + 1);
                    for u in ulm.iter_mut().take(end).skip(x0) {
                        if *u == 0 || d.sev < *u {
                            *u = d.sev;
                        }
                    }
                    if sign == 0 || d.sev < sign {
                        sign = d.sev;
                    }
                }
            }
            if sign != 0 && sign < 4 {
                s.put_char(l.ed_x, sy, '●', Style::fg(sev_color(sign)));
            }
            if let Some(v) = cur_path.and_then(|p| self.dbg.has_bp(p, ly)) {
                s.put_char(
                    l.ed_x,
                    sy,
                    if v { '●' } else { '○' },
                    Style::fg(if v { BP } else { DIM }),
                );
            }
            let exec = exec_line == Some(ly);
            if exec {
                s.put_char(l.ed_x, sy, '▶', Style::fg(TYPE).bold());
                s.fill(l.text_x, sy, l.text_w, Style::new().on(EXEC));
            }
            if self.numbers {
                let num = ly + 1;
                let txt = format!("{:>w$} ", num, w = (l.gutter - 3) as usize);
                let st = if ly == b.cy { Style::fg(BRIGHT) } else { Style::fg(DIM) };
                s.put(l.ed_x + 1, sy, &txt, st);
            }
            let toks = highlight::highlight_line(b.lang, line, &mut hl);
            let mut mask = vec![0u8; line.len()];
            if let Some(p) = pat {
                for (a, e) in p.find_line(line) {
                    let cur = ly == b.cy && a == b.cx && self.find.is_some();
                    for v in mask.iter_mut().skip(a).take(e - a) {
                        *v = if cur { 2 } else { 1 };
                    }
                }
            }
            // Inlay hints push the text right: `col` counts the line's own
            // columns (tab stops), `off` the hint cells drawn so far.
            let hints = b.hints_on(ly);
            let hint_st = if exec {
                Style::fg(HINT).on(EXEC).italic()
            } else {
                Style::fg(HINT).on(HINT_BG).italic()
            };
            let (mut col, mut off, mut hi) = (0usize, 0usize, 0usize);
            for (i, &c) in line.iter().enumerate() {
                while hi < hints.len() && hints[hi].x <= i {
                    off += put_virtual(s, l, b.scroll_x, sy, col + off, &hints[hi].text, hint_st);
                    hi += 1;
                }
                let w = char_width(c, col);
                let start = col + off;
                col += w;
                let end = col + off;
                if end <= b.scroll_x {
                    continue;
                }
                if start >= right {
                    break;
                }
                let mut st = tok_style(toks[i]);
                if in_sel(i, ly) {
                    st = st.on(SEL);
                } else if mask[i] == 2 {
                    st = st.on(FIND_CUR);
                } else if mask[i] == 1 {
                    st = st.on(FIND);
                } else if exec {
                    st = st.on(EXEC);
                }
                if ulm[i] != 0 {
                    st = st.underline(sev_color(ulm[i]));
                }
                let vis_start = start.max(b.scroll_x);
                let vis_end = end.min(right);
                let sx = l.text_x + (vis_start - b.scroll_x) as u16;
                if c == '\t' || start < b.scroll_x || end > right {
                    s.fill(sx, sy, (vis_end - vis_start) as u16, st);
                } else {
                    s.put_char(sx, sy, c, st);
                }
            }
            let nl_selected = visual && ly >= sa.1 && ly < se.1;
            let eol = col + off;
            if eol >= b.scroll_x && eol < right {
                let ex = l.text_x + (eol - b.scroll_x) as u16;
                if nl_selected {
                    s.put_char(ex, sy, ' ', Style::new().on(SEL));
                } else if ulm[line.len()] != 0 {
                    s.put_char(ex, sy, ' ', Style::new().underline(sev_color(ulm[line.len()])));
                }
            }
            for h in &hints[hi..] {
                off += put_virtual(s, l, b.scroll_x, sy, col + off, &h.text, hint_st);
            }
        }
    }

    /// The `:` line's Tab completions: a list right above the bar, under the
    /// word being completed, with where the matches came from on top.
    fn draw_cmd_comp(&self, s: &mut Screen) {
        let Some(c) = self.cmd_comp.as_ref().filter(|_| self.mode == Mode::Command) else {
            return;
        };
        let bar_y = s.h.saturating_sub(1);
        // Leave the tab bar alone.
        let rows = c.items.len().min(12).min(bar_y.saturating_sub(TAB_H + 1) as usize);
        let scroll = c.sel.map_or(0, |i| (i + 1).saturating_sub(rows));
        let title = match c.sel {
            _ if c.items.is_empty() => c.title.clone(),
            Some(i) if c.items.len() > rows => format!("{}  {}/{}", c.title, i + 1, c.items.len()),
            _ if c.items.len() > rows => format!("{}  {} matches", c.title, c.items.len()),
            _ => c.title.clone(),
        };
        let item_w = c.items.iter().map(|i| text_w(&i.0)).max().unwrap_or(0);
        let w = ((item_w + 2).max(text_w(&title) + 2) as u16).min(s.w);
        let word_x = CMD_X + text_w(&c.head) as u16;
        let x0 = word_x.saturating_sub(1).min(s.w.saturating_sub(w));
        let y0 = bar_y.saturating_sub(rows as u16 + 1);
        s.fill(x0, y0, w, Style::new().on(BAR));
        let tst = if c.items.is_empty() { ERR } else { DIM };
        put_clip(s, x0 + 1, y0, &title, Style::fg(tst).on(BAR).italic(), w as usize - 1);
        for r in 0..rows {
            let i = scroll + r;
            let (shown, _) = &c.items[i];
            let y = y0 + 1 + r as u16;
            let bg = if c.sel == Some(i) { SEL } else { POP };
            let fg = if shown.ends_with('/') { FUNC } else { BRIGHT };
            s.fill(x0, y, w, Style::new().on(bg));
            put_clip(s, x0 + 1, y, shown, Style::fg(fg).on(bg), w as usize - 1);
        }
    }

    /// The one bar at the bottom: the mode badge; then the `:` command line while
    /// typing a command, otherwise a message, or the file and the diagnostic under
    /// the cursor; position and language on the right.
    fn draw_status(&self, s: &mut Screen) {
        let y = s.h.saturating_sub(1);
        let bar = Style::fg(DIM).on(BAR);
        s.fill_row(y, bar);
        let (label, color) = if self.mode == Mode::Command {
            ("COMMAND", TYPE)
        } else {
            match self.focus {
                Focus::Panel => ("PANEL", ERR),
                Focus::Find => ("FIND", TYPE),
                Focus::Tree if self.git_active() => ("GIT", primary()),
                Focus::Tree => ("TREE", primary()),
                Focus::Diff => ("DIFF", primary()),
                Focus::Editor => ("EDIT", FUNC),
            }
        };
        let x = s.put(0, y, &format!(" {} ", label), Style::fg(INK).on(color).bold());
        let b = self.buf();
        let pct = if b.lines.len() <= 1 {
            100
        } else {
            b.cy * 100 / (b.lines.len() - 1)
        };
        let col = disp_col(&b.lines[b.cy], b.cx) + 1;
        let selinfo = match self.sel_range() {
            Some((a, e)) if a.1 != e.1 => format!("{} lines  ", e.1 - a.1 + 1),
            Some((a, e)) => format!("{} sel  ", e.0 - a.0),
            None => String::new(),
        };
        let right = format!(
            "{}{}{}  {}  {}:{}  {:>3}% ",
            selinfo,
            self.lsp_status().map(|s| s + "  ").unwrap_or_default(),
            if b.lang.name == "Metin" { "Text" } else { b.lang.name },
            if b.crlf { "CRLF" } else { "LF" },
            b.cy + 1,
            col,
            pct
        );
        let (e, w) = self.diag_counts();
        let mut counts: Vec<(String, Color)> = Vec::new();
        if e > 0 {
            counts.push((format!("✖ {} ", e), ERR));
        }
        if w > 0 {
            counts.push((format!("▲ {} ", w), TYPE));
        }
        let rw = right.width() as u16;
        let cw: u16 = counts.iter().map(|p| p.0.width() as u16).sum();

        if self.mode == Mode::Command {
            let text: String = std::iter::once(':').chain(self.cmd.iter().copied()).collect();
            let end = put_clip(
                s,
                CMD_X - 1,
                y,
                &text,
                Style::fg(BRIGHT).on(BAR),
                s.w.saturating_sub(CMD_X) as usize,
            );
            // An empty line can carry a hint of what to type (closing an unsaved tab).
            if let Some(h) = self.cmd_hint.as_deref().filter(|_| self.cmd.is_empty()) {
                put_clip(
                    s,
                    end + 1,
                    y,
                    h,
                    Style::fg(DIM).on(BAR).italic(),
                    s.w.saturating_sub(end + 2) as usize,
                );
                return;
            }
            // The right side stays while the command is short enough.
            if end + 2 + rw <= s.w {
                s.put(s.w - rw, y, &right, bar);
            }
            return;
        }

        // Right side first; the middle is clipped before it.
        let mut mid_end = s.w;
        if x + rw + 10 < s.w {
            s.put(s.w - rw, y, &right, bar);
            mid_end = s.w - rw;
            if x + cw + rw + 30 < s.w {
                let mut cx = s.w - rw - cw;
                for (t, c) in counts {
                    cx = s.put(cx, y, &t, Style::fg(c).on(BAR));
                }
                mid_end -= cw;
            }
        }
        let room = |x: u16| mid_end.saturating_sub(x + 1) as usize;
        if let Some((m, err)) = &self.msg {
            let st = Style::fg(if *err { ERR } else { BRIGHT }).on(BAR);
            put_clip(s, x + 1, y, m, st, room(x + 1));
            return;
        }
        let mut x = put_clip(s, x + 1, y, &self.rel_path(b), Style::fg(BRIGHT).on(BAR), room(x + 1));
        if b.dirty {
            x = put_clip(s, x, y, " [+]", Style::fg(TYPE).on(BAR), room(x));
        }
        if self.focus == Focus::Editor && self.popup.is_none() {
            if let Some((sev, m)) = self.diag_at_cursor() {
                let first = m.lines().next().unwrap_or("");
                put_clip(
                    s,
                    x + 2,
                    y,
                    &format!("● {}", first),
                    Style::fg(sev_color(sev)).on(BAR),
                    room(x + 2),
                );
            }
        }
    }
}

impl Editor {
    fn draw_popup(&self, s: &mut Screen) {
        self.popup_rect.set(None);
        let Some(p) = &self.popup else { return };
        let Some((cxs, cys)) = self.cursor_pos() else { return };
        let b = self.buf();
        let line = &b.lines[b.cy];
        let typed = disp_col(line, b.cx).saturating_sub(disp_col(line, p.start)) as u16;
        let rows = p.shown.len().saturating_sub(p.scroll).min(POPUP_ROWS);
        if rows == 0 {
            return;
        }
        let vis = &p.shown[p.scroll..p.scroll + rows];
        let labw = vis
            .iter()
            .map(|&i| text_w(&p.items[i].label))
            .max()
            .unwrap_or(0)
            .min(42);
        let detw = vis
            .iter()
            .map(|&i| text_w(&p.items[i].detail))
            .max()
            .unwrap_or(0)
            .min(32);
        let mut w = (6 + labw + if detw > 0 { 2 + detw } else { 0 } + 1) as u16;
        w = w.min(s.w);
        let mut x0 = cxs.saturating_sub(typed).saturating_sub(6);
        x0 = x0.min(s.w.saturating_sub(w));
        let below = s.h.saturating_sub(1).saturating_sub(cys + 1);
        let y0 = if rows as u16 <= below {
            cys + 1
        } else {
            cys.saturating_sub(rows as u16)
        };
        for (r, &i) in vis.iter().enumerate() {
            let it = &p.items[i];
            let y = y0 + r as u16;
            let bg = if p.scroll + r == p.sel { SEL } else { POP };
            s.fill(x0, y, w, Style::new().on(bg));
            let (k, kc) = kind_info(it.kind);
            put_clip(s, x0 + 1, y, k, Style::fg(kc).on(bg), 4);
            let lx = put_clip(s, x0 + 6, y, &it.label, Style::fg(BRIGHT).on(bg), labw);
            if detw > 0 {
                let dx = (x0 + 6 + labw as u16 + 2).max(lx + 1);
                let room = (x0 + w).saturating_sub(dx + 1) as usize;
                put_clip(s, dx, y, &it.detail, Style::fg(DIM).on(bg), room);
            }
        }
        if p.shown.len() > rows {
            let n = p.shown.len();
            let thumb = (p.scroll * rows / n).min(rows - 1);
            s.put_char(x0 + w - 1, y0 + thumb as u16, '▐', Style::fg(BORDER).on(POP));
        }
        self.popup_rect.set(Some((x0, y0, w, rows as u16)));
    }

    fn draw_sig(&self, s: &mut Screen, l: &Layout) {
        let Some((label, hl)) = &self.sig else { return };
        let Some((cxs, cys)) = self.cursor_pos() else { return };
        let w = (text_w(label) + 2).min(s.w as usize) as u16;
        let y = if cys > l.top { cys - 1 } else { cys + 1 };
        let x0 = cxs.saturating_sub(4).min(s.w.saturating_sub(w));
        s.fill(x0, y, w, Style::new().on(POP));
        let mut x = x0 + 1;
        for (i, c) in label.chars().enumerate() {
            let active = hl.is_some_and(|(a, b)| i >= a && i < b);
            let st = if active {
                Style::fg(TYPE).on(POP).bold()
            } else {
                Style::fg(DIM).on(POP)
            };
            if x + 1 >= x0 + w {
                break;
            }
            x += s.put_char(x, y, c, st);
        }
    }

    fn draw_hover(&self, s: &mut Screen, l: &Layout) {
        let Some(lines) = &self.hover else { return };
        let (cxs, cys) = self.cursor_pos().unwrap_or((l.text_x, l.top));
        let maxw = (s.w as usize).saturating_sub(4).clamp(10, 90);
        let inner = maxw - 4;
        let mut wrapped: Vec<String> = Vec::new();
        for ln in lines {
            let mut cur = String::new();
            let mut cw = 0;
            for c in ln.chars() {
                let w = char_width(c, 0);
                if cw + w > inner {
                    wrapped.push(std::mem::take(&mut cur));
                    cw = 0;
                }
                cur.push(c);
                cw += w;
            }
            wrapped.push(cur);
        }
        let rows = wrapped.len().min((s.h as usize).saturating_sub(6)).max(1);
        let w = (wrapped.iter().map(|t| text_w(t)).max().unwrap_or(0) + 4)
            .max(12)
            .min(maxw) as u16;
        let h = rows as u16 + 2;
        let y0 = if cys >= l.top + h {
            cys - h
        } else {
            (cys + 1).min(s.h.saturating_sub(2 + h))
        };
        let x0 = cxs.min(s.w.saturating_sub(w));
        draw_box(s, x0, y0, w, h, "");
        for (r, t) in wrapped.iter().take(rows).enumerate() {
            let st = if r == 0 {
                Style::fg(FUNC).on(POP)
            } else {
                Style::fg(BRIGHT).on(POP)
            };
            put_clip(s, x0 + 2, y0 + 1 + r as u16, t, st, w as usize - 4);
        }
    }

    fn draw_menu(&self, s: &mut Screen, l: &Layout) {
        self.menu_rect.set(None);
        let Some(m) = &self.menu else { return };
        let rows = m.items.len().clamp(1, MENU_ROWS);
        let itemw = m.items.iter().map(|i| text_w(&i.0)).max().unwrap_or(0);
        let min_w = if m.filter.is_some() { 44 } else { 20 };
        let w = (itemw + 4).max(text_w(&m.title) + 8).max(min_w).min(s.w as usize - 2) as u16;
        let h = rows as u16 + 2;
        let (x0, y0) = match m.anchor {
            Some((ax, ay)) => {
                let y = if ay + h < s.h.saturating_sub(1) {
                    ay
                } else {
                    ay.saturating_sub(h + 1)
                };
                (ax.min(s.w.saturating_sub(w)), y)
            }
            None => {
                let area = l.ed_w;
                (l.ed_x + area.saturating_sub(w) / 2, l.top + 1)
            }
        };
        let title = match &m.filter {
            Some(f) => format!("{}: {}▏", m.title, f),
            None => m.title.clone(),
        };
        draw_box(s, x0, y0, w, h, &title);
        for r in 0..rows {
            let i = m.scroll + r;
            let Some((t, _)) = m.items.get(i) else { break };
            let y = y0 + 1 + r as u16;
            let sel = i == m.sel;
            let st = if sel {
                Style::fg(BRIGHT).on(SEL)
            } else {
                Style::new().on(POP)
            };
            s.fill(x0 + 1, y, w - 2, st);
            put_clip(s, x0 + 2, y, t, st, w as usize - 4);
        }
        if m.items.is_empty() {
            let hint = if m.ws_symbols && m.filter.as_deref().unwrap_or("").is_empty() {
                "Type to search…"
            } else {
                "No results"
            };
            put_clip(s, x0 + 2, y0 + 1, hint, Style::fg(DIM).on(POP), w as usize - 4);
        }
        if m.items.len() > rows {
            let c = format!(" {}/{} ", m.sel + 1, m.items.len());
            put_clip(
                s,
                x0 + w - 2 - text_w(&c) as u16,
                y0 + h - 1,
                &c,
                Style::fg(DIM).on(POP),
                12,
            );
        }
        self.menu_rect.set(Some((x0, y0, w, h - 1)));
    }
}
