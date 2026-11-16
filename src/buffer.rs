use crate::highlight::{self, LangDef};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const TAB_W: usize = 4;

pub type Pos = (usize, usize);

pub fn char_width(c: char, col: usize) -> usize {
    if c == '\t' {
        TAB_W - col % TAB_W
    } else {
        c.width().unwrap_or(1)
    }
}

pub fn disp_col(line: &[char], x: usize) -> usize {
    let mut col = 0;
    for &c in &line[..x.min(line.len())] {
        col += char_width(c, col);
    }
    col
}

pub fn idx_at_col(line: &[char], target: usize) -> usize {
    let mut col = 0;
    for (i, &c) in line.iter().enumerate() {
        let w = char_width(c, col);
        if col + w > target {
            return i;
        }
        col += w;
    }
    line.len()
}

pub fn cls(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

pub fn leading_ws(line: &[char]) -> Vec<char> {
    line.iter().take_while(|c| **c == ' ' || **c == '\t').copied().collect()
}

/// An inlay hint from the language server (a type, a parameter name): text
/// drawn in the line without being part of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    /// Char index it sits before (`line.len()` = end of line).
    pub x: usize,
    pub y: usize,
    /// Padding included.
    pub text: String,
    /// A parameter-name hint labels the code after it: typing at its spot goes
    /// after the hint. Type hints follow the code before them and get pushed.
    pub before: bool,
    /// Settings key of its kind (`types`, `parameters`, …).
    pub kind: &'static str,
}

struct Snapshot {
    lines: Vec<Vec<char>>,
    cx: usize,
    cy: usize,
}

pub struct Buffer {
    pub lines: Vec<Vec<char>>,
    pub path: Option<PathBuf>,
    pub name: String,
    pub dirty: bool,
    pub crlf: bool,
    pub bom: bool,
    pub use_tabs: bool,
    pub cx: usize,
    pub cy: usize,
    pub want_col: usize,
    pub scroll_y: usize,
    pub scroll_x: usize,
    pub lang: &'static LangDef,
    pub rev: u64,
    pub lsp_rev: u64,
    pub lsp_version: i32,
    pub lsp_open: bool,
    /// Opened with Space in the tree: the next preview replaces it.
    pub preview: bool,
    /// Inlay hints, sorted by position. Edits move them; the server's next
    /// answer replaces them.
    pub hints: Vec<Hint>,
    /// `rev` the hints were last asked for (None: ask again).
    pub hint_rev: Option<u64>,
    /// The request in flight: (server id, request id).
    pub hint_req: Option<(&'static str, i64)>,
    /// `rev` diagnostics were last asked for, from a server that only answers
    /// when asked (None: ask again), and that request.
    pub pull_rev: Option<u64>,
    pub pull_req: Option<(&'static str, i64)>,
    /// Time of the last edit, to wait for a pause in typing.
    pub edited_at: Instant,
    /// Rust: no `mod` reaches this file, so rust-analyzer ignores it.
    pub unlinked: bool,
    /// State (`Editor::crate_key`) the crate check last ran for, and the request in flight.
    pub crate_key: Option<u64>,
    pub crate_req: Option<(&'static str, i64)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl Buffer {
    pub fn scratch(name: &str) -> Self {
        Buffer {
            lines: vec![Vec::new()],
            path: None,
            name: name.to_string(),
            dirty: false,
            crlf: false,
            bom: false,
            use_tabs: false,
            cx: 0,
            cy: 0,
            want_col: 0,
            scroll_y: 0,
            scroll_x: 0,
            lang: highlight::plain(),
            rev: 0,
            lsp_rev: 0,
            lsp_version: 0,
            lsp_open: false,
            preview: false,
            hints: Vec::new(),
            hint_rev: None,
            hint_req: None,
            pull_rev: None,
            pull_req: None,
            edited_at: Instant::now(),
            unlinked: false,
            crate_key: None,
            crate_req: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn from_text(name: &str, text: &str) -> Self {
        let mut b = Self::scratch(name);
        b.set_text(text);
        b
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut b = Self::scratch(&name);
        b.lang = highlight::detect(path);
        let mut tabs = 0;
        let mut spaces = 0;
        if path.exists() {
            let bytes = fs::read(path)?;
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(t) = text.strip_prefix('\u{feff}') {
                text = t.to_string();
                b.bom = true;
            }
            b.crlf = text.contains("\r\n");
            b.set_text(&text);
            tabs = b.lines.iter().filter(|l| l.first() == Some(&'\t')).count();
            spaces = b.lines.iter().filter(|l| l.first() == Some(&' ')).count();
        }
        b.use_tabs = if tabs + spaces == 0 {
            b.lang.name == "Go" || name == "Makefile" || name == "makefile"
        } else {
            tabs > spaces
        };
        b.path = Some(path.to_path_buf());
        Ok(b)
    }

    pub fn set_text(&mut self, text: &str) {
        let mut lines: Vec<Vec<char>> = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).chars().collect())
            .collect();
        if lines.len() > 1 && text.ends_with('\n') {
            lines.pop();
        }
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        self.lines = lines;
        self.hints.clear();
    }

    pub fn save(&mut self) -> io::Result<usize> {
        let path = self.path.clone().ok_or_else(|| io::Error::other("no file name"))?;
        let nl = if self.crlf { "\r\n" } else { "\n" };
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        if !(self.lines.len() == 1 && self.lines[0].is_empty()) {
            for l in &self.lines {
                out.extend(l.iter());
                out.push_str(nl);
            }
        }
        fs::write(&path, out.as_bytes())?;
        self.dirty = false;
        Ok(out.len())
    }

    pub fn touch(&mut self) {
        self.dirty = true;
        self.rev += 1;
        self.edited_at = Instant::now();
    }

    /// The inlay hints of line `y`.
    pub fn hints_on(&self, y: usize) -> &[Hint] {
        let a = self.hints.partition_point(|h| h.y < y);
        let b = self.hints.partition_point(|h| h.y <= y);
        &self.hints[a..b]
    }

    /// Screen column (from the line start) of char `x` on line `y`, counting the
    /// inlay hints before it. A hint at `x` itself comes after the cursor.
    pub fn vcol(&self, y: usize, x: usize) -> usize {
        let hw: usize = self
            .hints_on(y)
            .iter()
            .filter(|h| h.x < x)
            .map(|h| h.text.width())
            .sum();
        disp_col(&self.lines[y], x) + hw
    }

    /// Width of line `y` on screen, inlay hints included.
    pub fn vwidth(&self, y: usize) -> usize {
        let line = &self.lines[y];
        disp_col(line, line.len()) + self.hints_on(y).iter().map(|h| h.text.width()).sum::<usize>()
    }

    /// Char index at screen column `vc` of line `y` (the inverse of `vcol`).
    /// A click on a hint lands on its spot.
    pub fn idx_at_vcol(&self, y: usize, vc: usize) -> usize {
        let line = &self.lines[y];
        let hs = self.hints_on(y);
        let (mut col, mut off, mut hi) = (0, 0, 0);
        for (i, &c) in line.iter().enumerate() {
            while hi < hs.len() && hs[hi].x <= i {
                off += hs[hi].text.width();
                hi += 1;
                if vc < col + off {
                    return i;
                }
            }
            let w = char_width(c, col);
            if vc < col + off + w {
                return i;
            }
            col += w;
        }
        line.len()
    }

    /// Keeps inlay hints on their code across an edit that replaced `[a, b)`
    /// with text ending at `e`. Hints inside the removed text go.
    fn shift_hints(&mut self, a: Pos, b: Pos, e: Pos) {
        let (ka, kb) = ((a.1, a.0), (b.1, b.0));
        self.hints.retain_mut(|h| {
            let p = (h.y, h.x);
            if p < ka || (p == ka && (a != b || h.before)) {
                return true;
            }
            if p < kb {
                return false;
            }
            if h.y == b.1 {
                h.x = e.0 + (h.x - b.0);
                h.y = e.1;
            } else {
                h.y = h.y - b.1 + e.1;
            }
            true
        });
    }

    /// A line's text was replaced wholesale: its hints no longer fit.
    pub fn drop_hints_on(&mut self, y: usize) {
        self.hints.retain(|h| h.y != y);
    }

    pub fn text(&self) -> String {
        let mut s = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            s.extend(l.iter());
        }
        s.push('\n');
        s
    }

    pub fn checkpoint(&mut self) {
        self.undo.push(Snapshot {
            lines: self.lines.clone(),
            cx: self.cx,
            cy: self.cy,
        });
        if self.undo.len() > 1000 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn drop_noop_checkpoint(&mut self) {
        if let Some(s) = self.undo.last() {
            if s.lines == self.lines {
                self.undo.pop();
            }
        }
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(s) => {
                let cur = Snapshot {
                    lines: std::mem::replace(&mut self.lines, s.lines),
                    cx: self.cx,
                    cy: self.cy,
                };
                self.redo.push(cur);
                self.cx = s.cx;
                self.cy = s.cy;
                self.hints.clear();
                self.touch();
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(s) => {
                let cur = Snapshot {
                    lines: std::mem::replace(&mut self.lines, s.lines),
                    cx: self.cx,
                    cy: self.cy,
                };
                self.undo.push(cur);
                self.cx = s.cx;
                self.cy = s.cy;
                self.hints.clear();
                self.touch();
                true
            }
            None => false,
        }
    }

    pub fn line_len(&self, y: usize) -> usize {
        self.lines.get(y).map_or(0, |l| l.len())
    }

    pub fn last(&self) -> usize {
        self.lines.len() - 1
    }

    pub fn first_nb(&self, y: usize) -> usize {
        self.lines[y].iter().position(|c| !c.is_whitespace()).unwrap_or(0)
    }

    pub fn clamp(&mut self, allow_end: bool) {
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        self.cy = self.cy.min(self.lines.len() - 1);
        let len = self.line_len(self.cy);
        let max = if allow_end || len == 0 { len } else { len - 1 };
        self.cx = self.cx.min(max);
    }

    pub fn sync_want(&mut self) {
        self.want_col = disp_col(&self.lines[self.cy], self.cx);
    }

    pub fn insert_text(&mut self, (x, y): Pos, text: &[char]) -> Pos {
        let x = x.min(self.line_len(y));
        let tail: Vec<char> = self.lines[y].split_off(x);
        let (mut cx, mut cy) = (x, y);
        for &c in text {
            if c == '\n' {
                cy += 1;
                self.lines.insert(cy, Vec::new());
                cx = 0;
            } else {
                self.lines[cy].push(c);
                cx += 1;
            }
        }
        self.lines[cy].extend(tail);
        self.touch();
        self.shift_hints((x, y), (x, y), (cx, cy));
        (cx, cy)
    }

    pub fn get_text(&self, a: Pos, b: Pos) -> Vec<char> {
        let mut out = Vec::new();
        let bx = b.0.min(self.line_len(b.1));
        if a.1 == b.1 {
            if a.0 < bx {
                out.extend(&self.lines[a.1][a.0..bx]);
            }
            return out;
        }
        out.extend(&self.lines[a.1][a.0.min(self.line_len(a.1))..]);
        out.push('\n');
        for line in self.lines.get(a.1 + 1..b.1).unwrap_or_default() {
            out.extend(line);
            out.push('\n');
        }
        out.extend(&self.lines[b.1][..bx]);
        out
    }

    pub fn delete_text(&mut self, a: Pos, b: Pos) -> Vec<char> {
        let t = self.get_text(a, b);
        let ax = a.0.min(self.line_len(a.1));
        let bx = b.0.min(self.line_len(b.1));
        if a.1 == b.1 {
            if ax < bx {
                self.lines[a.1].drain(ax..bx);
            }
        } else {
            let tail: Vec<char> = self.lines[b.1][bx..].to_vec();
            self.lines[a.1].truncate(ax);
            self.lines[a.1].extend(tail);
            self.lines.drain(a.1 + 1..=b.1);
        }
        self.touch();
        self.shift_hints((ax, a.1), (bx, b.1), (ax, a.1));
        t
    }

    pub fn delete_lines(&mut self, y0: usize, y1: usize) -> Vec<Vec<char>> {
        let removed: Vec<_> = self.lines.drain(y0..=y1).collect();
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        self.touch();
        self.hints.retain(|h| h.y < y0 || h.y > y1);
        for h in self.hints.iter_mut().filter(|h| h.y > y1) {
            h.y -= y1 + 1 - y0;
        }
        removed
    }

    pub fn insert_lines(&mut self, at: usize, ls: &[Vec<char>]) {
        let at = at.min(self.lines.len());
        self.lines.splice(at..at, ls.iter().cloned());
        self.touch();
        for h in self.hints.iter_mut().filter(|h| h.y >= at) {
            h.y += ls.len();
        }
    }

    /// Ctrl/Alt+Left: start of the previous word (or end of the previous line).
    pub fn word_left(&self, (x, y): Pos) -> Pos {
        if x == 0 {
            return if y > 0 { (self.line_len(y - 1), y - 1) } else { (0, 0) };
        }
        let line = &self.lines[y];
        let mut i = x.min(line.len());
        while i > 0 && cls(line[i - 1]) == 0 {
            i -= 1;
        }
        if i > 0 {
            let k = cls(line[i - 1]);
            while i > 0 && cls(line[i - 1]) == k {
                i -= 1;
            }
        }
        (i, y)
    }

    /// Ctrl/Alt+Right: end of the next word (or start of the next line).
    pub fn word_right(&self, (x, y): Pos) -> Pos {
        let len = self.line_len(y);
        if x >= len {
            return if y < self.last() { (0, y + 1) } else { (len, y) };
        }
        let line = &self.lines[y];
        let mut i = x;
        while i < len && cls(line[i]) == 0 {
            i += 1;
        }
        if i < len {
            let k = cls(line[i]);
            while i < len && cls(line[i]) == k {
                i += 1;
            }
        }
        (i, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> Buffer {
        Buffer::from_text("t", s)
    }

    #[test]
    fn insert_delete() {
        let mut b = buf("hello\nworld");
        let end = b.insert_text((5, 0), &"!\nnew".chars().collect::<Vec<_>>());
        assert_eq!(end, (3, 1));
        assert_eq!(b.lines.len(), 3);
        let t = b.delete_text((5, 0), (3, 1));
        assert_eq!(t.iter().collect::<String>(), "!\nnew");
        assert_eq!(b.lines[0].iter().collect::<String>(), "hello");
    }

    #[test]
    fn words() {
        let b = buf("foo bar.baz\n\nqux");
        assert_eq!(b.word_right((0, 0)), (3, 0));
        assert_eq!(b.word_right((3, 0)), (7, 0));
        assert_eq!(b.word_right((7, 0)), (8, 0));
        assert_eq!(b.word_right((11, 0)), (0, 1));
        assert_eq!(b.word_left((11, 0)), (8, 0));
        assert_eq!(b.word_left((4, 0)), (0, 0));
        assert_eq!(b.word_left((0, 2)), (0, 1));
    }

    fn hint(x: usize, y: usize, text: &str, before: bool) -> Hint {
        Hint {
            x,
            y,
            text: text.into(),
            before,
            kind: "types",
        }
    }

    fn at(b: &Buffer) -> Vec<(usize, usize)> {
        b.hints.iter().map(|h| (h.x, h.y)).collect()
    }

    #[test]
    fn hints_follow_edits() {
        // `let x = f(1);` with a type hint after `x` and a parameter hint before `1`.
        let mut b = buf("let x = f(1);\nnext");
        b.hints = vec![
            hint(5, 0, ": i32", false),
            hint(10, 0, "a: ", true),
            hint(4, 1, " end", false),
        ];
        // Typing at a type hint pushes it; at a parameter hint the text goes after it.
        b.insert_text((5, 0), &['y']);
        b.insert_text((11, 0), &['2']);
        assert_eq!(at(&b), vec![(6, 0), (11, 0), (4, 1)]);
        // A new line above moves the next line's hints down.
        b.insert_text((0, 0), &['\n']);
        assert_eq!(at(&b), vec![(6, 1), (11, 1), (4, 2)]);
        // Deleting over a hint drops it; joining lines carries hints along.
        b.delete_text((4, 1), (8, 1));
        assert_eq!(at(&b), vec![(7, 1), (4, 2)]);
        let len = b.line_len(1);
        b.delete_text((len, 1), (0, 2));
        assert_eq!(at(&b), vec![(7, 1), (len + 4, 1)]);
        b.delete_lines(0, 0);
        assert_eq!(at(&b), vec![(7, 0), (len + 4, 0)]);
        b.insert_lines(0, &[vec!['a']]);
        assert_eq!(at(&b), vec![(7, 1), (len + 4, 1)]);
        // Undo swaps whole snapshots: the hints wait for the server's next answer.
        b.checkpoint();
        b.insert_text((0, 0), &['z']);
        assert!(b.undo());
        assert!(b.hints.is_empty());
    }

    #[test]
    fn hint_columns() {
        let mut b = buf("let x = 1;");
        b.hints = vec![hint(5, 0, ": i32", false)];
        // The cursor at the hint's spot sits before it; the text after moves right.
        assert_eq!(b.vcol(0, 5), 5);
        assert_eq!(b.vcol(0, 6), 11);
        assert_eq!(b.vwidth(0), 15);
        // Screen columns back to chars: a click on the hint lands on its spot.
        assert_eq!(b.idx_at_vcol(0, 4), 4);
        assert_eq!(b.idx_at_vcol(0, 7), 5);
        assert_eq!(b.idx_at_vcol(0, 11), 6);
        assert_eq!(b.idx_at_vcol(0, 99), 10);
    }

    #[test]
    fn cols() {
        let l: Vec<char> = "\tab".chars().collect();
        assert_eq!(disp_col(&l, 1), 4);
        assert_eq!(idx_at_col(&l, 2), 0);
        assert_eq!(idx_at_col(&l, 5), 2);
    }
}
