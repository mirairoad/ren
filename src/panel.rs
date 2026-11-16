//! The bottom panel (Ctrl+`): Terminal, Debug and Containers tabs.

use super::*;
use crate::containers::Health;
use crate::render::{primary, put_clip, BAR, BRIGHT, DIM, ERR, STR, TYPE};
use crate::screen::{Screen, Style};
use crate::term;
use crossterm::style::Color;
use debug_ui::DbgHit;

const TABS: [(PanelTab, &str); 3] = [
    (PanelTab::Terminal, "TERMINAL"),
    (PanelTab::Debug, "DEBUG"),
    (PanelTab::Containers, "CONTAINERS"),
];

fn vt_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::AnsiValue(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb { r, g, b },
    }
}

fn health_style(h: Health) -> (char, Color) {
    match h {
        Health::Running => ('●', STR),
        Health::Busy => ('◐', TYPE),
        Health::Stopped => ('○', DIM),
        Health::Bad => ('✖', ERR),
    }
}

impl Editor {
    pub(crate) fn panel_open(&mut self, tab: PanelTab) {
        self.dbg.visible = true;
        self.panel_tab = tab;
        self.focus = Focus::Panel;
        self.mode = Mode::Edit;
    }

    fn cycle_panel_tab(&mut self, d: isize) {
        let i = TABS.iter().position(|t| t.0 == self.panel_tab).unwrap_or(0) as isize;
        let n = TABS.len() as isize;
        self.panel_tab = TABS[((i + d).rem_euclid(n)) as usize].0;
    }

    /// Draws the tab strip at the left of the panel header. Returns the next x.
    pub(crate) fn draw_panel_tabs(&self, s: &mut Screen, y: u16, hits: &mut Vec<(u16, u16, u16, DbgHit)>) -> u16 {
        let mut x = 1;
        for (tab, name) in TABS {
            let active = tab == self.panel_tab;
            let st = if active {
                Style::fg(primary()).on(BAR).bold()
            } else {
                Style::fg(DIM).on(BAR)
            };
            let nx = s.put(x, y, &format!(" {} ", name), st);
            hits.push((y, x, nx, DbgHit::Tab(tab)));
            x = nx;
        }
        s.put(x, y, " │", Style::fg(DIM).on(BAR))
    }

    pub(crate) fn draw_panel(&self, s: &mut Screen, l: &Layout) {
        self.dbg_hits.borrow_mut().clear();
        self.dbg_cursor.set(None);
        if l.dbg_h == 0 {
            return;
        }
        // The panel draws on its own canvas, x from 0, and is copied in right of
        // the side panel, so the file tree keeps its full height.
        let mut p = Screen::new(l.dbg_w, s.h);
        match self.panel_tab {
            PanelTab::Debug => self.draw_debug(&mut p, l),
            PanelTab::Terminal => self.draw_terminal(&mut p, l),
            PanelTab::Containers => self.draw_containers(&mut p, l),
        }
        s.blit(&p, l.dbg_x, l.dbg_y, l.dbg_h);
        if let Some((x, y)) = self.dbg_cursor.get() {
            self.dbg_cursor.set(Some((x + l.dbg_x, y)));
        }
    }

    fn panel_header(&self, s: &mut Screen, l: &Layout, hits: &mut Vec<(u16, u16, u16, DbgHit)>) -> u16 {
        let y0 = l.dbg_y;
        s.fill(0, y0, s.w, Style::new().on(BAR));
        hits.push((y0, 0, s.w, DbgHit::Resize));
        let x = self.draw_panel_tabs(s, y0, hits);
        let cx = s.w.saturating_sub(3);
        s.put(cx, y0, " ✕", Style::fg(BRIGHT).on(BAR));
        hits.push((y0, cx, s.w, DbgHit::Close));
        x
    }

    fn draw_terminal(&self, s: &mut Screen, l: &Layout) {
        let mut hits = self.dbg_hits.borrow_mut();
        let y0 = l.dbg_y;
        let mut x = self.panel_header(s, l, &mut hits);
        for (i, t) in self.terms.iter().enumerate() {
            let label = format!(" {} {}{} ", i + 1, t.label(), if t.alive() { "" } else { " (exited)" });
            let st = if i == self.term_sel {
                Style::fg(BRIGHT).on(BAR).bold()
            } else {
                Style::fg(DIM).on(BAR)
            };
            let nx = s.put(x, y0, &label, st);
            hits.push((y0, x, nx, DbgHit::TermTab(i)));
            // The kill button's room is always kept, so tabs don't shift on hover.
            let hovered = self.term_hover == Some(i);
            let kx = s.put(nx, y0, if hovered { "×" } else { " " }, Style::fg(ERR).on(BAR).bold());
            hits.push((y0, nx, kx + 1, DbgHit::TermKill(i)));
            x = s.put(kx, y0, " ", st);
        }
        let nx = s.put(x, y0, " + ", Style::fg(BRIGHT).on(BAR));
        hits.push((y0, x, nx, DbgHit::TermNew));
        let top = y0 + 1;
        let rows = l.dbg_h.saturating_sub(1);
        hits.push((top, 0, s.w, DbgHit::TermArea));
        for r in 1..rows {
            hits.push((top + r, 0, s.w, DbgHit::TermArea));
        }
        let Some(t) = self.terms.get(self.term_sel) else {
            let msg = self.term_err.clone().unwrap_or_else(|| "Starting shell…".into());
            put_clip(s, 2, top, &msg, Style::fg(DIM), s.w as usize - 2);
            return;
        };
        let sc = t.screen();
        let (srows, scols) = sc.size();
        for r in 0..rows.min(srows) {
            let mut c = 0u16;
            while c < scols.min(s.w) {
                let Some(cell) = sc.cell(r, c) else { break };
                if cell.is_wide_continuation() {
                    c += 1;
                    continue;
                }
                let mut fg = vt_color(cell.fgcolor());
                let mut bg = vt_color(cell.bgcolor());
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                    if fg == Color::Reset {
                        fg = Color::Black;
                    }
                    if bg == Color::Reset {
                        bg = BRIGHT;
                    }
                }
                let mut st = Style::fg(fg).on(bg);
                if cell.bold() {
                    st = st.bold();
                }
                if cell.italic() {
                    st = st.italic();
                }
                if cell.underline() {
                    st = st.underline(fg);
                }
                let ch = cell.contents().chars().next().unwrap_or(' ');
                s.put_char(c, top + r, ch, st);
                c += if cell.is_wide() { 2 } else { 1 };
            }
        }
        if !t.alive() {
            let msg = format!(" process exited ({}) · + for a new terminal ", t.exit.unwrap_or(0));
            put_clip(
                s,
                1,
                top + rows.saturating_sub(1),
                &msg,
                Style::fg(DIM).on(BAR),
                s.w as usize - 2,
            );
        }
        if self.focus == Focus::Panel && t.alive() && !sc.hide_cursor() && sc.scrollback() == 0 {
            let (cr, cc) = sc.cursor_position();
            if cr < rows {
                self.dbg_cursor.set(Some((cc.min(s.w - 1), top + cr)));
            }
        }
    }

    fn draw_containers(&self, s: &mut Screen, l: &Layout) {
        let mut hits = self.dbg_hits.borrow_mut();
        let y0 = l.dbg_y;
        let mut x = self.panel_header(s, l, &mut hits);
        let c = &self.ctr;
        let running = c.selected().is_some_and(|c| c.state == "running");
        let buttons: [(&str, DbgHit, bool); 5] = [
            ("▶ start", DbgHit::CtrAct("start"), c.selected().is_some() && !running),
            ("■ stop", DbgHit::CtrAct("stop"), running),
            ("⟲ restart", DbgHit::CtrAct("restart"), c.selected().is_some()),
            (">_ shell", DbgHit::CtrShell, running),
            ("↻", DbgHit::CtrRefresh, true),
        ];
        // Icons only when the labels don't fit before the panel's ✕.
        let full: u16 = buttons.iter().map(|b| crate::render::text_w(b.0) as u16 + 1).sum();
        let compact = x + full + 4 > s.w;
        for (lab, hit, on) in buttons {
            let lab = if compact {
                lab.split(' ').next().unwrap_or(lab)
            } else {
                lab
            };
            let st = if on {
                Style::fg(BRIGHT).on(BAR)
            } else {
                Style::fg(DIM).on(BAR)
            };
            let nx = s.put(x + 1, y0, lab, st);
            hits.push((y0, x, nx + 1, hit));
            x = nx + 1;
        }
        if let Some((m, err)) = &c.note {
            let st = Style::fg(if *err { ERR } else { DIM }).on(BAR);
            put_clip(s, x + 2, y0, m, st, (s.w.saturating_sub(x + 6)) as usize);
        }
        let top = y0 + 1;
        let rows = l.dbg_h.saturating_sub(1) as usize;
        let lw = (s.w * 45 / 100).max(20);
        for r in 0..rows as u16 {
            s.put_char(lw, top + r, '│', Style::fg(primary()));
        }
        if let Some(e) = &c.error {
            put_clip(s, 1, top, &format!("✖ {}", e), Style::fg(ERR), lw as usize - 2);
            let hint = if self.settings.containers_socket.is_empty() {
                "Set containers.socket in :settings (e.g. unix:///var/run/docker.sock)".to_string()
            } else {
                format!("socket: {}", self.settings.containers_socket)
            };
            put_clip(s, 1, top + 1, &hint, Style::fg(DIM), lw as usize - 2);
        } else if !c.loaded {
            put_clip(s, 1, top, "Loading containers…", Style::fg(DIM), lw as usize - 2);
        } else if c.list.is_empty() {
            put_clip(s, 1, top, "No containers", Style::fg(DIM), lw as usize - 2);
        }
        let focused = self.focus == Focus::Panel;
        for r in 0..rows {
            let i = c.scroll + r;
            let Some(ct) = c.list.get(i) else { break };
            let y = top + r as u16;
            let sel = i == c.sel;
            let bg = if sel && focused {
                crate::render::SEL
            } else {
                Color::Reset
            };
            if sel {
                s.fill(0, y, lw, Style::new().on(bg));
            }
            let (ic, col) = health_style(ct.health());
            s.put_char(1, y, ic, Style::fg(col).on(bg));
            let nx = put_clip(
                s,
                3,
                y,
                &ct.name,
                Style::fg(BRIGHT).on(bg).bold(),
                (lw as usize).saturating_sub(4),
            );
            let detail = if ct.ports.is_empty() {
                format!("  {} · {}", ct.image, ct.status)
            } else {
                format!("  {} · {} · {}", ct.image, ct.status, ct.ports)
            };
            put_clip(
                s,
                nx,
                y,
                &detail,
                Style::fg(DIM).on(bg),
                lw.saturating_sub(nx + 1) as usize,
            );
            hits.push((y, 0, lw, DbgHit::Ctr(i)));
        }
        let lx = lw + 2;
        let room = s.w.saturating_sub(lx + 1) as usize;
        if c.log_id.is_none() {
            put_clip(
                s,
                lx,
                top,
                "Select a container to follow its logs",
                Style::fg(DIM),
                room,
            );
        }
        let n = c.logs.len();
        let end = n.saturating_sub(c.log_back);
        let start = end.saturating_sub(rows);
        for (r, line) in c.logs[start..end].iter().enumerate() {
            put_clip(s, lx, top + r as u16, line, Style::new(), room);
        }
        for r in 0..rows as u16 {
            hits.push((top + r, lw + 1, s.w, DbgHit::LogArea));
        }
    }

    /// Keys while the panel has focus. Returns false to let global shortcuts run.
    pub(crate) fn panel_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = is_ctrl(&k);
        match k.code {
            KeyCode::PageUp | KeyCode::PageDown if ctrl => {
                self.cycle_panel_tab(if k.code == KeyCode::PageDown { 1 } else { -1 });
                return true;
            }
            KeyCode::Char('`') if ctrl => return false,
            // Ctrl+` without kitty keys (it arrives as Ctrl+Space): the panel's
            // key, not the shell's.
            KeyCode::Char(' ') | KeyCode::Null if ctrl && !kitty() => return false,
            _ => {}
        }
        match self.panel_tab {
            PanelTab::Terminal => {
                if ctrl && k.code == KeyCode::Up {
                    self.focus = Focus::Editor;
                    return true;
                }
                let shift = k.modifiers.contains(KeyModifiers::SHIFT);
                if shift && matches!(k.code, KeyCode::PageUp | KeyCode::PageDown) {
                    let rows = self.layout().dbg_h.saturating_sub(2) as isize;
                    if let Some(t) = self.terms.get_mut(self.term_sel) {
                        t.scroll(if k.code == KeyCode::PageUp { rows } else { -rows });
                    }
                    return true;
                }
                match self.terms.get_mut(self.term_sel) {
                    Some(t) if t.alive() => t.key(&k),
                    _ => {
                        if k.code == KeyCode::Enter {
                            self.term_new(None);
                        }
                    }
                }
                true
            }
            PanelTab::Debug | PanelTab::Containers => {
                let global = ctrl
                    && matches!(
                        k.code,
                        KeyCode::Char('w') | KeyCode::Char('b') | KeyCode::Char('k') | KeyCode::Char('s')
                    );
                if global {
                    return false;
                }
                if self.panel_tab == PanelTab::Debug {
                    self.dbg_key(k);
                } else {
                    self.ctr_key(k);
                }
                true
            }
        }
    }

    fn ctr_key(&mut self, k: KeyEvent) {
        let n = self.ctr.list.len();
        match k.code {
            KeyCode::Down => self.ctr.sel = (self.ctr.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Up => self.ctr.sel = self.ctr.sel.saturating_sub(1),
            KeyCode::Home => self.ctr.sel = 0,
            KeyCode::End => self.ctr.sel = n.saturating_sub(1),
            KeyCode::Enter => return self.ctr_shell(),
            KeyCode::PageUp => self.ctr.log_back = (self.ctr.log_back + 10).min(self.ctr.logs.len()),
            KeyCode::PageDown => self.ctr.log_back = self.ctr.log_back.saturating_sub(10),
            KeyCode::Char(':') => return self.start_cmd(),
            KeyCode::Esc => {
                if !kitty() {
                    self.start_cmd();
                } else {
                    self.focus = Focus::Editor;
                }
                return;
            }
            _ => return,
        }
        self.ctr_follow();
    }

    fn ctr_follow(&mut self) {
        let rows = self.layout().dbg_h.saturating_sub(1).max(1) as usize;
        let c = &mut self.ctr;
        if c.sel < c.scroll {
            c.scroll = c.sel;
        } else if c.sel >= c.scroll + rows {
            c.scroll = c.sel + 1 - rows;
        }
        if c.selected().is_some() {
            c.follow_logs();
        }
    }

    fn ctr_shell(&mut self) {
        let Some((cmd, env, name)) = self.ctr.shell_cmd() else {
            return self.error("Select a running container first");
        };
        self.term_spawn(cmd, env, &format!("{} (shell)", name));
    }

    /// Opens a new terminal session (`None` = the login shell).
    pub(crate) fn term_new(&mut self, cmd: Option<Vec<String>>) {
        let cmd = cmd.unwrap_or_else(term::default_shell);
        let title = Path::new(&cmd[0])
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cmd[0].clone());
        self.term_spawn(cmd, Vec::new(), &title);
    }

    fn term_spawn(&mut self, cmd: Vec<String>, env: Vec<(String, String)>, title: &str) {
        let l = self.layout();
        let rows = if l.dbg_h > 1 { l.dbg_h - 1 } else { 12 };
        let cols = l.dbg_w.max(20);
        let root = self.tree.root.clone();
        match term::Session::spawn(&cmd, &root, &env, rows, cols, title) {
            Ok(s) => {
                self.terms.push(s);
                self.term_sel = self.terms.len() - 1;
                self.term_err = None;
                self.panel_open(PanelTab::Terminal);
            }
            Err(e) => {
                self.term_err = Some(format!("Could not start {}: {}", cmd[0], e));
                self.panel_open(PanelTab::Terminal);
            }
        }
    }

    pub(crate) fn term_close(&mut self) {
        self.term_kill(self.term_sel);
    }

    /// Kills terminal `i` and drops its tab. Closing the last one hides the panel
    /// (instead of starting a fresh shell straight away).
    pub(crate) fn term_kill(&mut self, i: usize) {
        if i >= self.terms.len() {
            return;
        }
        self.terms.remove(i);
        self.term_hover = None;
        if i < self.term_sel {
            self.term_sel -= 1;
        }
        self.term_sel = self.term_sel.min(self.terms.len().saturating_sub(1));
        if self.terms.is_empty() {
            self.dbg.visible = false;
            if self.focus == Focus::Panel {
                self.focus = Focus::Editor;
            }
        }
    }

    /// Polls terminals and containers, keeps the terminal sized to the panel.
    pub fn panel_tick(&mut self) -> bool {
        let mut changed = false;
        for t in self.terms.iter_mut() {
            changed |= t.poll();
        }
        self.parked_tick();
        let l = self.layout();
        let visible = l.dbg_h > 0;
        if visible && self.panel_tab == PanelTab::Terminal {
            if self.terms.is_empty() && self.term_err.is_none() {
                self.term_new(None);
                changed = true;
            }
            let (rows, cols) = (l.dbg_h.saturating_sub(1), l.dbg_w);
            for t in self.terms.iter_mut() {
                t.resize(rows, cols);
            }
        }
        if self.ctr.socket != self.settings.containers_socket {
            self.ctr.socket = self.settings.containers_socket.clone();
            self.ctr.loaded = false;
            self.ctr.refresh();
        }
        let show = visible && self.panel_tab == PanelTab::Containers;
        changed |= self.ctr.tick(show);
        if show && self.ctr.log_id.is_none() && !self.ctr.list.is_empty() {
            self.ctr.follow_logs();
            changed = true;
        }
        changed
    }

    /// Panel clicks that aren't debug-specific. Returns true when handled.
    pub(crate) fn panel_click(&mut self, hit: DbgHit) -> bool {
        match hit {
            DbgHit::Tab(t) => {
                self.panel_tab = t;
                self.focus = Focus::Panel;
            }
            DbgHit::TermTab(i) => {
                self.term_sel = i;
                self.focus = Focus::Panel;
            }
            DbgHit::TermKill(i) => self.term_kill(i),
            DbgHit::TermNew => self.term_new(None),
            DbgHit::TermArea => self.focus = Focus::Panel,
            DbgHit::Ctr(i) => {
                self.focus = Focus::Panel;
                self.ctr.sel = i;
                self.ctr_follow();
            }
            DbgHit::CtrAct(v) => self.ctr.action(v),
            DbgHit::CtrShell => self.ctr_shell(),
            DbgHit::CtrRefresh => self.ctr.refresh(),
            DbgHit::LogArea => self.focus = Focus::Panel,
            _ => return false,
        }
        true
    }

    /// Mouse wheel over the panel for the Terminal and Containers tabs.
    pub(crate) fn panel_scroll(&mut self, x: u16, d: isize) -> bool {
        match self.panel_tab {
            PanelTab::Terminal => {
                if let Some(t) = self.terms.get_mut(self.term_sel) {
                    t.scroll(-d);
                }
                true
            }
            PanelTab::Containers => {
                let lw = (self.layout().dbg_w * 45 / 100).max(20);
                if x > lw {
                    let n = self.ctr.logs.len();
                    self.ctr.log_back = (self.ctr.log_back as isize - d).clamp(0, n as isize) as usize;
                } else {
                    let max = self.ctr.list.len().saturating_sub(1) as isize;
                    self.ctr.scroll = (self.ctr.scroll as isize + d).clamp(0, max.max(0)) as usize;
                }
                true
            }
            PanelTab::Debug => false,
        }
    }

    pub(crate) fn panel_paste(&mut self, s: &str) -> bool {
        if self.panel_tab != PanelTab::Terminal {
            return false;
        }
        if let Some(t) = self.terms.get_mut(self.term_sel) {
            t.paste(s);
        }
        true
    }

    /// `:terminal [new|close]`
    pub(crate) fn terminal_command(&mut self, args: &[String]) {
        match args.first().map(String::as_str) {
            Some("new") => {
                let cmd = if args.len() > 1 { Some(args[1..].to_vec()) } else { None };
                self.term_new(cmd);
            }
            Some("close") => self.term_close(),
            _ => self.panel_open(PanelTab::Terminal),
        }
    }

    /// `:containers [start|stop|restart|shell|refresh]`
    pub(crate) fn containers_command(&mut self, args: &[String]) {
        self.panel_open(PanelTab::Containers);
        match args.first().map(String::as_str) {
            Some("start") => self.ctr.action("start"),
            Some("stop") => self.ctr.action("stop"),
            Some("restart") => self.ctr.action("restart"),
            Some("shell") => self.ctr_shell(),
            Some("refresh") | None => self.ctr.refresh(),
            Some(o) => self.error(format!("Unknown :containers option: {}", o)),
        }
    }
}
