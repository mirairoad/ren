//! The box that opens when the mouse rests on code for a second: the problems
//! found there (what, which tool, its rule code and docs link), the language
//! server's hover (type, docs) and the quick fixes, which apply on click.

use super::ide::{hover_lines, MenuAction, UNLINKED};
use super::*;
use crate::lsp::{from_lsp_col, path_to_uri, to_lsp_col, Diag, Pending};
use serde_json::{json, Value};

/// How long the mouse must rest on code before the box opens.
const PEEK_DELAY: Duration = Duration::from_millis(1000);
/// At most this many quick fixes are listed (Ctrl+. shows them all).
const MAX_FIXES: usize = 8;

/// One problem as the box shows it.
pub struct PeekDiag {
    pub sev: u8,
    /// Who found it and its rule: "rust-analyzer E0308", "Ruff F401".
    pub source: String,
    pub lines: Vec<String>,
    /// The rule's documentation, when the server sends one.
    pub link: Option<String>,
}

pub struct Peek {
    /// Each box gets a number; answers meant for an older one are dropped.
    pub seq: u64,
    pub buf: usize,
    pub rev: u64,
    /// The screen cell the mouse rested on.
    pub cell: (u16, u16),
    /// Text range that keeps the box open while the mouse stays on it: the
    /// problems' span, or the word.
    pub span: (Pos, Pos),
    pub diags: Vec<PeekDiag>,
    /// The server's hover text.
    pub info: Vec<String>,
    pub actions: Vec<(String, MenuAction)>,
    /// Answers still to come.
    pub waiting: usize,
    /// The quick fix under the mouse.
    pub hot: Option<usize>,
}

impl Editor {
    /// The text spot (char, line) under a screen cell, only over actual text.
    fn text_spot(&self, x: u16, y: u16) -> Option<Pos> {
        let l = self.layout();
        if self.is_splash() || self.diff_shown() || x < l.text_x || y < l.top || y >= l.top + l.text_h {
            return None;
        }
        let b = self.buf();
        let ly = b.scroll_y + (y - l.top) as usize;
        if ly >= b.lines.len() {
            return None;
        }
        let vc = b.scroll_x + (x - l.text_x) as usize;
        if vc >= b.vwidth(ly) {
            return None;
        }
        let cx = b.idx_at_vcol(ly, vc);
        (cx < b.lines[ly].len()).then_some((cx, ly))
    }

    /// Mouse events for the box: resting starts the timer, moving off closes
    /// it, a click on a quick fix applies it. True = handled.
    pub(crate) fn peek_mouse(&mut self, m: &MouseEvent) -> bool {
        let (x, y) = (m.column, m.row);
        let in_box = self.peek.is_some() && Self::in_rect(self.peek_rect.get(), x, y).is_some();
        let row_action = || self.peek_rows.borrow().iter().find(|r| r.0 == y).map(|r| r.1);
        match m.kind {
            MouseEventKind::Moved => {
                if in_box {
                    let hot = row_action();
                    if let Some(p) = self.peek.as_mut() {
                        p.hot = hot;
                    }
                    return true;
                }
                let spot = self.text_spot(x, y);
                if let Some(p) = &self.peek {
                    let (a, e) = p.span;
                    let on_span = spot.is_some_and(|(sx, sy)| (sy, sx) >= (a.1, a.0) && (sy, sx) < (e.1, e.0));
                    if on_span {
                        return false;
                    }
                    self.peek = None;
                }
                self.rest = match spot {
                    Some(s) if self.rest.is_some_and(|r| r.0 == s) => self.rest,
                    Some(s) => Some((s, (x, y), Instant::now())),
                    None => None,
                };
                false
            }
            MouseEventKind::Down(MouseButton::Left) if in_box => {
                if let Some(i) = row_action() {
                    if let Some((_, act)) = self.peek.take().and_then(|p| p.actions.into_iter().nth(i)) {
                        match act {
                            MenuAction::Code(lang, v) => self.run_action(lang, v, true),
                            MenuAction::Run(c) => self.run_command(&c),
                            _ => {}
                        }
                    }
                }
                true
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if in_box => true,
            MouseEventKind::Up(_) if in_box => true,
            _ => {
                self.peek = None;
                self.rest = None;
                false
            }
        }
    }

    /// Opens the box once the mouse has rested for `PEEK_DELAY`, and closes it
    /// when the text changes or another tab shows. True = redraw.
    pub(crate) fn peek_tick(&mut self) -> bool {
        if let Some(p) = &self.peek {
            if p.buf != self.cur || self.bufs.get(p.buf).is_none_or(|b| b.rev != p.rev) {
                self.peek = None;
                return true;
            }
            return false;
        }
        let Some((at, cell, since)) = self.rest else {
            return false;
        };
        if since.elapsed() < PEEK_DELAY {
            return false;
        }
        self.rest = None;
        let busy = self.mode != Mode::Edit
            || self.menu.is_some()
            || self.popup.is_some()
            || self.hover.is_some()
            || self.commit.is_some();
        if busy || self.text_spot(cell.0, cell.1) != Some(at) {
            return false;
        }
        self.open_peek(at, cell)
    }

    fn open_peek(&mut self, at: Pos, cell: (u16, u16)) -> bool {
        self.peek_seq += 1;
        let seq = self.peek_seq;
        let i = self.cur;
        let (x, y) = at;
        let b = &self.bufs[i];
        let key = |p: Pos| (p.1, p.0);
        // The problems covering the spot.
        let mut diags: Vec<(&Diag, Pos, Pos)> = Vec::new();
        if let Some(ds) = self.buf_diags(i) {
            for d in ds {
                let (l0, l1) = (d.l0.min(b.last()), d.l1.min(b.last()));
                let a = (from_lsp_col(&b.lines[l0], d.c0, d.enc), l0);
                let mut e = (from_lsp_col(&b.lines[l1], d.c1, d.enc), l1);
                if key(e) <= key(a) {
                    e = (a.0 + 1, a.1);
                }
                if key(at) >= key(a) && key(at) < key(e) {
                    diags.push((d, a, e));
                }
            }
        }
        diags.sort_by_key(|d| d.0.sev);
        let span = match (
            diags.iter().map(|d| key(d.1)).min(),
            diags.iter().map(|d| key(d.2)).max(),
        ) {
            (Some(a), Some(e)) => ((a.1, a.0), (e.1, e.0)),
            _ => {
                let line = &b.lines[y];
                let word = |c: &char| c.is_alphanumeric() || *c == '_';
                if word(&line[x]) {
                    let s = line[..x].iter().rposition(|c| !word(c)).map_or(0, |p| p + 1);
                    let e = line[x..].iter().position(|c| !word(c)).map_or(line.len(), |p| x + p);
                    ((s, y), (e, y))
                } else {
                    ((x, y), (x + 1, y))
                }
            }
        };
        let shown: Vec<PeekDiag> = diags.iter().map(|d| peek_diag(d.0)).collect();
        let unlinked = diags.iter().any(|d| d.0.server == UNLINKED);
        let raw: Vec<(&'static str, Value)> = diags.iter().map(|d| (d.0.server, d.0.raw.clone())).collect();
        let rev = b.rev;
        // Ask the servers: the hover at the spot, and fixes for the problems.
        let mut waiting = 0;
        if let Some(lang) = self.lsp_lang(i) {
            self.lsp_sync(i);
            let path = self.bufs[i].path.clone().unwrap();
            let uri = path_to_uri(&path);
            let pos = |id: &'static str, p: Pos, ed: &Editor| {
                let line = &ed.bufs[i].lines[p.1];
                json!({ "line": p.1, "character": to_lsp_col(line, p.0, ed.lsp.enc(id)) })
            };
            if let Some(id) = self.lsp.pick(lang, "hoverProvider", None) {
                let p = json!({ "textDocument": { "uri": uri }, "position": pos(id, at, self) });
                if self
                    .lsp
                    .request(id, "textDocument/hover", p, Pending::PeekHover(seq))
                    .is_some()
                {
                    waiting += 1;
                }
            }
            if !raw.is_empty() {
                let ids: Vec<&'static str> = self
                    .lsp
                    .ids(lang)
                    .into_iter()
                    .filter(|id| self.lsp.ready(id) && self.lsp.cap(id, "codeActionProvider"))
                    .collect();
                for id in ids {
                    let ds: Vec<Value> = raw.iter().filter(|r| r.0 == id).map(|r| r.1.clone()).collect();
                    let p = json!({
                        "textDocument": { "uri": uri },
                        "range": { "start": pos(id, span.0, self), "end": pos(id, span.1, self) },
                        "context": { "diagnostics": ds, "only": ["quickfix"], "triggerKind": 1 }
                    });
                    if self
                        .lsp
                        .request(id, "textDocument/codeAction", p, Pending::PeekActions(seq))
                        .is_some()
                    {
                        waiting += 1;
                    }
                }
            }
        }
        let actions = if unlinked { self.local_actions() } else { Vec::new() };
        if shown.is_empty() && waiting == 0 {
            return false;
        }
        self.peek = Some(Peek {
            seq,
            buf: i,
            rev,
            cell,
            span,
            diags: shown,
            info: Vec::new(),
            actions,
            waiting,
            hot: None,
        });
        true
    }

    /// The box `seq`, if it is still the open one.
    fn peek_for(&mut self, seq: u64) -> Option<&mut Peek> {
        self.peek.as_mut().filter(|p| p.seq == seq)
    }

    /// An answer (or an error) came for box `seq`; with nothing at all to show
    /// once everything is in, the box closes.
    pub(crate) fn peek_answered(&mut self, seq: u64) {
        let Some(p) = self.peek_for(seq) else { return };
        p.waiting = p.waiting.saturating_sub(1);
        if p.waiting == 0 && p.diags.is_empty() && p.info.is_empty() && p.actions.is_empty() {
            self.peek = None;
        }
    }

    pub(crate) fn on_peek_hover(&mut self, seq: u64, r: Value) {
        let lines = hover_lines(&r);
        if let Some(p) = self.peek_for(seq) {
            p.info = lines.into_iter().take(14).collect();
        }
        self.peek_answered(seq);
    }

    pub(crate) fn on_peek_actions(&mut self, lang: &'static str, seq: u64, r: Value) {
        if let Some(p) = self.peek_for(seq) {
            let mut arr = r.as_array().cloned().unwrap_or_default();
            // Quick fixes only: refactorings aren't about the problem.
            arr.retain(|a| {
                a["kind"]
                    .as_str()
                    .is_none_or(|k| k.is_empty() || k.starts_with("quickfix"))
            });
            arr.sort_by_key(|a| !a["isPreferred"].as_bool().unwrap_or(false));
            for a in arr {
                if p.actions.len() >= MAX_FIXES {
                    break;
                }
                let title = a["title"].as_str().unwrap_or("?").to_string();
                if !p.actions.iter().any(|(t, _)| *t == title) {
                    p.actions.push((title, MenuAction::Code(lang, a)));
                }
            }
        }
        self.peek_answered(seq);
    }
}

/// A diagnostic for the box: its own message (without the tag the bar adds),
/// who reported it with the rule code, and the rule's docs link.
fn peek_diag(d: &Diag) -> PeekDiag {
    let raw = &d.raw;
    let server = d.server.split('#').nth(1).unwrap_or(d.server);
    let (source, msg) = if raw.is_object() {
        let src = raw["source"].as_str().filter(|s| !s.is_empty()).unwrap_or(server);
        let code = match &raw["code"] {
            Value::String(c) => c.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        let source = if code.is_empty() {
            src.to_string()
        } else {
            format!("{} {}", src, code)
        };
        (source, raw["message"].as_str().unwrap_or(&d.msg).to_string())
    } else {
        (server.to_string(), d.msg.clone())
    };
    let mut lines: Vec<String> = msg.lines().map(|l| l.trim_end().to_string()).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.truncate(20);
    PeekDiag {
        sev: d.sev,
        source,
        lines,
        link: raw["codeDescription"]["href"].as_str().map(String::from),
    }
}
