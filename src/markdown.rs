//! Markdown rendered for reading (Ctrl+Shift+V or `:preview` on a `.md` file):
//! headings, paragraphs wrapped to the window, lists with bullets, quotes,
//! tables drawn as a grid, code blocks in their language's colours, links,
//! **bold**, *italic*, `code`, and the HTML a README uses (`<p align="center">`,
//! `<img alt>`, `<h1>`, `<code>`). The file itself isn't touched: the same key
//! goes back to it.

use crate::highlight::{self, md_list_marker, State};
use crate::render::{primary, tok_style, BRIGHT, DIM, ERR, FUNC, KW, MACRO, NUM, POP, STR, TYPE};
use crate::screen::Style;
use crossterm::style::Color;
use regex::Regex;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthChar;

pub type Cell = (char, Style);

/// One row of the rendered view.
pub struct Line {
    pub cells: Vec<Cell>,
    /// Background for the row from column `indent` on (code blocks).
    pub fill: Option<Style>,
    pub indent: usize,
    /// Centred, as HTML `align="center"` asks.
    pub center: bool,
    /// The source line it comes from, to keep the place when switching views.
    pub src: usize,
}

/// The widest text column; wider windows keep a margin on the right.
pub const MAX_WIDTH: usize = 100;

fn cw(c: char) -> usize {
    c.width().unwrap_or(0)
}

pub fn width_of(cells: &[Cell]) -> usize {
    cells.iter().map(|c| cw(c.0)).sum()
}

fn text_cells(s: &str, st: Style) -> Vec<Cell> {
    s.chars().map(|c| (c, st)).collect()
}

fn link_style(base: Style) -> Style {
    Style {
        fg: FUNC,
        ul: Some(FUNC),
        ..base
    }
}

fn code_style() -> Style {
    Style::fg(STR).on(POP)
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

/// Inline HTML as Markdown: `<code>` → `` ` ``, `<b>` → `**`, `<em>` → `*`,
/// `<br>` → a line break, `<img alt="x">` → `![x]()`; other tags go.
fn html_to_md(s: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static ALT: OnceLock<Regex> = OnceLock::new();
    if !s.contains('<') {
        return s.to_string();
    }
    let tag = re(&TAG, r"(?i)<(/?)([a-z][a-z0-9]*)\b[^>]*?>");
    let alt = re(&ALT, r#"(?i)\balt\s*=\s*["']([^"']*)["']"#);
    tag.replace_all(s, |c: &regex::Captures| {
        // `<https://…>` is an autolink, not a tag.
        if c[0].contains("://") || c[0].starts_with("<mailto:") {
            return c[0].to_string();
        }
        let close = !c[1].is_empty();
        match c[2].to_ascii_lowercase().as_str() {
            "code" | "kbd" | "tt" => "`".to_string(),
            "b" | "strong" => "**".to_string(),
            "em" | "i" => "*".to_string(),
            "br" => "\n".to_string(),
            "img" if !close => {
                let a = alt.captures(&c[0]).map(|m| m[1].to_string()).unwrap_or_default();
                format!("![{}]()", a)
            }
            _ => String::new(),
        }
    })
    .into_owned()
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" | "#39" => '\'',
        "nbsp" => ' ',
        "copy" => '©',
        "reg" => '®',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        n if n.starts_with("#x") => char::from_u32(u32::from_str_radix(&n[2..], 16).ok()?)?,
        n if n.starts_with('#') => char::from_u32(n[1..].parse().ok()?)?,
        _ => return None,
    })
}

/// Inline Markdown as styled characters: code spans, emphasis, links (their
/// text; the target isn't shown), images (`[image: alt]`), autolinks, escapes,
/// entities. A `\n` cell is a hard line break.
pub fn inline(s: &str, base: Style) -> Vec<Cell> {
    let s = html_to_md(s);
    let mut out = Vec::new();
    inline_into(&s.chars().collect::<Vec<_>>(), base, &mut out);
    out
}

fn inline_into(t: &[char], base: Style, out: &mut Vec<Cell>) {
    let n = t.len();
    let word = |k: usize| t.get(k).is_some_and(|c| c.is_alphanumeric());
    let mut i = 0;
    while i < n {
        let c = t[i];
        match c {
            '\\' if t.get(i + 1).is_some_and(|c| c.is_ascii_punctuation()) => {
                out.push((t[i + 1], base));
                i += 2;
            }
            '`' => {
                let run = t[i..].iter().take_while(|c| **c == '`').count();
                let close = (i + run..n).find(|&k| {
                    t[k] == '`' && t[k..].iter().take_while(|c| **c == '`').count() == run && t[k - 1] != '`'
                });
                match close {
                    Some(k) => {
                        let inner: String = t[i + run..k].iter().collect();
                        let inner = if inner.len() > 2 && inner.starts_with(' ') && inner.ends_with(' ') {
                            &inner[1..inner.len() - 1]
                        } else {
                            &inner
                        };
                        out.extend(text_cells(inner, code_style()));
                        i = k + run;
                    }
                    None => {
                        out.extend(text_cells(&"`".repeat(run), base));
                        i += run;
                    }
                }
            }
            '!' | '[' => {
                let open = if c == '!' { i + 1 } else { i };
                let close = (t.get(open) == Some(&'[')).then(|| bracket_end(t, open)).flatten();
                let Some(close) = close else {
                    out.push((c, base));
                    i += 1;
                    continue;
                };
                let end = match t.get(close + 1) {
                    Some('(') => paren_end(t, close + 1),
                    Some('[') => t[close + 1..].iter().position(|c| *c == ']').map(|p| close + 2 + p),
                    _ => None,
                };
                let text = &t[open + 1..close];
                if c == '!' {
                    let alt: String = text.iter().collect();
                    let label = if alt.trim().is_empty() {
                        "[image]".to_string()
                    } else {
                        format!("[image: {}]", alt.trim())
                    };
                    out.extend(text_cells(&label, Style::fg(DIM).italic()));
                    i = end.unwrap_or(close + 1);
                } else if let Some(end) = end {
                    inline_into(text, link_style(base), out);
                    i = end;
                } else if is_task(text) {
                    out.push((c, base));
                    i += 1;
                } else {
                    // `[text]` alone: a reference link, or just brackets.
                    out.push(('[', base));
                    inline_into(text, base, out);
                    out.push((']', base));
                    i = close + 1;
                }
            }
            '<' => {
                let end = t[i..].iter().position(|c| *c == '>').map(|p| i + p);
                let inner: Option<String> = end.map(|e| t[i + 1..e].iter().collect());
                match (end, inner) {
                    (Some(e), Some(u)) if u.contains("://") || u.starts_with("mailto:") => {
                        out.extend(text_cells(u.trim_start_matches("mailto:"), link_style(base)));
                        i = e + 1;
                    }
                    _ => {
                        out.push((c, base));
                        i += 1;
                    }
                }
            }
            '&' => {
                let end = t[i..].iter().take(10).position(|c| *c == ';').map(|p| i + p);
                let ch = end.and_then(|e| entity(&t[i + 1..e].iter().collect::<String>()));
                match (end, ch) {
                    (Some(e), Some(ch)) => {
                        out.push((ch, base));
                        i = e + 1;
                    }
                    _ => {
                        out.push((c, base));
                        i += 1;
                    }
                }
            }
            '*' | '_' | '~' => {
                let run = t[i..].iter().take_while(|k| **k == c).count();
                let len = if c == '~' { 2 } else { run.min(3) };
                let opens = (c != '~' || run >= 2)
                    && t.get(i + len).is_some_and(|k| !k.is_whitespace())
                    && !(c == '_' && word(i.wrapping_sub(1)));
                let marker: Vec<char> = std::iter::repeat_n(c, len).collect();
                let close = opens
                    .then(|| {
                        (i + len + 1..n).find(|&k| {
                            t[k..].starts_with(&marker)
                                && !t[k - 1].is_whitespace()
                                && !(c == '_' && word(k + len))
                                && (len > 1 || t.get(k + 1) != Some(&c))
                        })
                    })
                    .flatten();
                match close {
                    Some(k) => {
                        let st = match (c, len) {
                            ('~', _) => Style { fg: DIM, ..base },
                            (_, 3) => base.bold().italic(),
                            (_, 2) => base.bold(),
                            _ => base.italic(),
                        };
                        inline_into(&t[i + len..k], st, out);
                        i = k + len;
                    }
                    None => {
                        out.extend(std::iter::repeat_n((c, base), run));
                        i += run;
                    }
                }
            }
            'h' if t[i..].starts_with(&['h', 't', 't', 'p', 's', ':', '/', '/'])
                || t[i..].starts_with(&['h', 't', 't', 'p', ':', '/', '/']) =>
            {
                let end = (i..n)
                    .find(|&k| t[k].is_whitespace() || matches!(t[k], ')' | '>' | '"' | '\''))
                    .unwrap_or(n);
                let url: String = t[i..end].iter().collect();
                // A sentence's full stop isn't part of the address.
                let url = url.trim_end_matches(['.', ',', ';', ':']);
                out.extend(text_cells(url, link_style(base)));
                i += url.chars().count();
            }
            _ => {
                out.push((c, base));
                i += 1;
            }
        }
    }
}

fn is_task(text: &[char]) -> bool {
    matches!(text, [' '] | ['x'] | ['X'])
}

/// The `]` closing the `[` at `open` (nested brackets allowed).
fn bracket_end(t: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut k = open;
    while k < t.len() {
        match t[k] {
            '\\' => k += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// End (exclusive) of `(…)` opening at `open`.
fn paren_end(t: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (k, &c) in t.iter().enumerate().skip(open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Wraps styled text to `width` columns: words move to the next line whole
/// (one longer than a line is cut), `first` starts the first line and `rest`
/// the others (bullets, quote bars, hanging indents).
pub fn wrap(cells: &[Cell], width: usize, first: &[Cell], rest: &[Cell]) -> Vec<Vec<Cell>> {
    let width = width.max(width_of(rest) + 4);
    let mut lines = Vec::new();
    let mut cur: Vec<Cell> = first.to_vec();
    let mut cur_w = width_of(first);
    let mut content = false;
    let mut pending_space: Option<Cell> = None;
    let mut word: Vec<Cell> = Vec::new();
    let flush_word = |word: &mut Vec<Cell>,
                      cur: &mut Vec<Cell>,
                      cur_w: &mut usize,
                      content: &mut bool,
                      space: &mut Option<Cell>,
                      lines: &mut Vec<Vec<Cell>>| {
        if word.is_empty() {
            return;
        }
        let ww = width_of(word);
        let sw = usize::from(space.is_some() && *content);
        if *content && *cur_w + sw + ww > width {
            lines.push(std::mem::replace(cur, rest.to_vec()));
            *cur_w = width_of(rest);
            *content = false;
        } else if let (Some(sp), true) = (*space, *content) {
            cur.push(sp);
            *cur_w += 1;
        }
        *space = None;
        for &c in word.iter() {
            let w = cw(c.0);
            if *cur_w + w > width && *content {
                lines.push(std::mem::replace(cur, rest.to_vec()));
                *cur_w = width_of(rest);
            }
            cur.push(c);
            *cur_w += w;
            *content = true;
        }
        word.clear();
    };
    for &c in cells {
        match c.0 {
            '\n' => {
                flush_word(
                    &mut word,
                    &mut cur,
                    &mut cur_w,
                    &mut content,
                    &mut pending_space,
                    &mut lines,
                );
                lines.push(std::mem::replace(&mut cur, rest.to_vec()));
                cur_w = width_of(rest);
                content = false;
                pending_space = None;
            }
            ' ' | '\t' => {
                flush_word(
                    &mut word,
                    &mut cur,
                    &mut cur_w,
                    &mut content,
                    &mut pending_space,
                    &mut lines,
                );
                if pending_space.is_none() {
                    pending_space = Some((' ', c.1));
                }
            }
            _ => word.push(c),
        }
    }
    flush_word(
        &mut word,
        &mut cur,
        &mut cur_w,
        &mut content,
        &mut pending_space,
        &mut lines,
    );
    if content || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn atx(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    if line.len() - t.len() > 3 {
        return None;
    }
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &t[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    // A closing run of #s is decoration.
    let text = rest.trim().trim_end_matches('#').trim_end();
    Some((hashes, text))
}

fn is_rule(line: &str) -> bool {
    let t: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && matches!(t[0], '-' | '*' | '_') && t.iter().all(|&c| c == t[0])
}

/// `|---|:--:|--:|`: the row under a table's header, with each column's alignment.
fn table_aligns(line: &str) -> Option<Vec<char>> {
    let t = line.trim();
    if !t.contains('-') || !t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) {
        return None;
    }
    let cells = split_row(t);
    if cells.is_empty() || cells.iter().any(|c| !c.contains('-')) {
        return None;
    }
    Some(
        cells
            .iter()
            .map(|c| match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => 'c',
                (false, true) => 'r',
                _ => 'l',
            })
            .collect(),
    )
}

/// A table row's cells: split on `|` (not `\|`), the outer pipes dropped.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") {
        &t[..t.len() - 1]
    } else {
        t
    };
    let mut cells = vec![String::new()];
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cells.last_mut().unwrap().push('|');
                chars.next();
            }
            '|' => cells.push(String::new()),
            _ => cells.last_mut().unwrap().push(c),
        }
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

fn fence(line: &str) -> Option<(char, usize, usize, String)> {
    let indent = line.len() - line.trim_start().len();
    let t = line.trim_start();
    let ch = t.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = t.chars().take_while(|c| *c == ch).count();
    if len < 3 {
        return None;
    }
    let info = t[len..].trim().to_string();
    (ch == '~' || !info.contains('`')).then_some((ch, len, indent, info))
}

/// What a new line starts, when it isn't more of a paragraph.
fn starts_block(line: &str) -> bool {
    let t = line.trim_start();
    atx(line).is_some()
        || fence(line).is_some()
        || t.starts_with('>')
        || is_rule(line)
        || md_list_marker(&line.chars().collect::<Vec<_>>()).is_some()
        || html_block(t)
}

fn html_block(t: &str) -> bool {
    static BLOCK: OnceLock<Regex> = OnceLock::new();
    re(
        &BLOCK,
        r"(?i)^</?(p|div|h[1-6]|img|br|hr|table|tr|td|th|thead|tbody|details|summary|picture|source|center|ul|ol|li|section|figure|a)\b",
    )
    .is_match(t)
}

struct Renderer {
    out: Vec<Line>,
    width: usize,
    para: Vec<String>,
    para_src: usize,
    /// Hanging indent of the paragraph being collected (inside a list item).
    para_indent: usize,
    /// Indents of the open lists, outermost first.
    lists: Vec<usize>,
    /// Inside HTML `align="center"`.
    center: bool,
}

impl Renderer {
    fn push(&mut self, cells: Vec<Cell>, src: usize) {
        self.out.push(Line {
            cells,
            fill: None,
            indent: 0,
            center: self.center,
            src,
        });
    }

    /// A blank row between blocks (none at the top, none twice).
    fn gap(&mut self, src: usize) {
        if self.out.last().is_some_and(|l| !l.cells.is_empty() || l.fill.is_some()) {
            self.out.push(Line {
                cells: Vec::new(),
                fill: None,
                indent: 0,
                center: false,
                src,
            });
        }
    }

    fn flush(&mut self) {
        if self.para.is_empty() {
            return;
        }
        let mut text = String::new();
        for (k, l) in self.para.iter().enumerate() {
            // Two spaces or a backslash at the end of a line: a line break.
            let hard = l.ends_with("  ") || l.ends_with('\\');
            text.push_str(l.trim_end().trim_end_matches('\\').trim());
            if k + 1 < self.para.len() {
                text.push(if hard { '\n' } else { ' ' });
            }
        }
        self.para.clear();
        let src = self.para_src;
        let pad = vec![(' ', Style::new()); self.para_indent];
        let cells = inline(&text, Style::new());
        if self.para_indent == 0 {
            self.gap(src);
        }
        for l in wrap(&cells, self.width, &pad, &pad) {
            self.push(l, src);
        }
    }

    fn heading(&mut self, level: usize, text: &str, src: usize) {
        self.gap(src);
        let st = match level {
            1 => Style::fg(primary()).bold(),
            2 => Style::fg(KW).bold(),
            3 => Style::fg(FUNC).bold(),
            _ => Style::fg(BRIGHT).bold(),
        };
        let cells = inline(text, st);
        let lines = wrap(&cells, self.width, &[], &[]);
        let w = lines.iter().map(|l| width_of(l)).max().unwrap_or(0);
        for l in lines {
            self.push(l, src);
        }
        if level <= 2 {
            let (ch, color) = if level == 1 { ('━', primary()) } else { ('─', DIM) };
            let len = if self.center { w } else { self.width };
            self.push(vec![(ch, Style::fg(color)); len], src);
        }
    }

    fn rule(&mut self, src: usize) {
        self.gap(src);
        self.push(vec![('─', Style::fg(DIM)); self.width], src);
    }

    /// A code block, `indent` columns in (inside a list item).
    fn code(&mut self, lines: &[String], info: &str, indent: usize, src: usize) {
        self.gap(src);
        let bg = Style::new().on(POP);
        let lang = highlight::fence_lang(info).map(|i| highlight::langs()[i]);
        let inner = self.width.saturating_sub(2 + indent).max(8);
        let label = info.split_whitespace().next().unwrap_or("");
        let pad: Vec<Cell> = vec![(' ', Style::new()); indent];
        let mut top = pad.clone();
        top.extend(vec![(' ', bg); inner + 1 - label.chars().count().min(inner)]);
        top.extend(text_cells(label, Style::fg(DIM).on(POP)));
        self.out.push(Line {
            cells: top,
            fill: Some(bg),
            indent,
            center: false,
            src,
        });
        let mut st = State::default();
        for (k, l) in lines.iter().enumerate() {
            let l = l.replace('\t', "    ");
            let chars: Vec<char> = l.chars().collect();
            let toks = match lang {
                Some(lang) => highlight::highlight_line(lang, &chars, &mut st),
                None => vec![highlight::Tok::Normal; chars.len()],
            };
            let mut cells: Vec<Cell> = pad.clone();
            cells.push((' ', bg));
            let mut w = 0;
            for (c, t) in chars.iter().zip(toks) {
                if w + cw(*c) > inner {
                    cells.pop();
                    cells.push(('…', Style::fg(DIM).on(POP)));
                    break;
                }
                let s = if lang.is_some() { tok_style(t) } else { Style::new() };
                cells.push((*c, s.on(POP)));
                w += cw(*c);
            }
            self.out.push(Line {
                cells,
                fill: Some(bg),
                indent,
                center: false,
                src: src + 1 + k,
            });
        }
        self.out.push(Line {
            cells: Vec::new(),
            fill: Some(bg),
            indent,
            center: false,
            src: src + lines.len() + 1,
        });
    }

    fn quote(&mut self, body: &[(usize, String)], src: usize) {
        self.gap(src);
        // GitHub's alerts: `> [!NOTE]`.
        let alert = |s: &str| -> Option<(&'static str, Color)> {
            let t = s.trim().to_ascii_uppercase();
            Some(match t.as_str() {
                "[!NOTE]" => ("Note", FUNC),
                "[!TIP]" => ("Tip", STR),
                "[!IMPORTANT]" => ("Important", KW),
                "[!WARNING]" => ("Warning", TYPE),
                "[!CAUTION]" => ("Caution", ERR),
                _ => return None,
            })
        };
        let mut bar_color = DIM;
        let mut paras: Vec<(usize, String)> = Vec::new();
        for (k, (depth, text)) in body.iter().enumerate() {
            if k == 0 {
                if let Some((label, c)) = alert(text) {
                    bar_color = c;
                    let mut cells = bar(*depth, bar_color);
                    cells.extend(text_cells(label, Style::fg(c).bold()));
                    self.push(cells, src);
                    continue;
                }
            }
            match paras.last_mut() {
                Some((d, p)) if *d == *depth && !text.is_empty() && !p.is_empty() => {
                    p.push(' ');
                    p.push_str(text);
                }
                _ => paras.push((*depth, text.clone())),
            }
        }
        for (depth, p) in paras {
            let prefix = bar(depth, bar_color);
            if p.is_empty() {
                self.push(prefix, src);
                continue;
            }
            let cells = inline(&p, Style::new().italic());
            for l in wrap(&cells, self.width, &prefix, &prefix) {
                self.push(l, src);
            }
        }
    }

    fn table(&mut self, rows: &[Vec<String>], aligns: &[char], src: usize) {
        self.gap(src);
        let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(aligns.len());
        if ncols == 0 {
            return;
        }
        let rendered: Vec<Vec<Vec<Cell>>> = rows
            .iter()
            .enumerate()
            .map(|(ri, r)| {
                (0..ncols)
                    .map(|c| {
                        let st = if ri == 0 { Style::new().bold() } else { Style::new() };
                        inline(r.get(c).map(String::as_str).unwrap_or(""), st)
                    })
                    .collect()
            })
            .collect();
        let mut widths: Vec<usize> = (0..ncols)
            .map(|c| rendered.iter().map(|r| width_of(&r[c])).max().unwrap_or(0).max(1))
            .collect();
        let sep_w = 3 * (ncols - 1);
        // Too wide: the widest column gives way first; its cells wrap.
        while widths.iter().sum::<usize>() + sep_w > self.width {
            let (i, w) = widths.iter().copied().enumerate().max_by_key(|(_, w)| *w).unwrap();
            if w <= 6 {
                break;
            }
            widths[i] -= 1;
        }
        let wrapped: Vec<Vec<Vec<Vec<Cell>>>> = rendered
            .iter()
            .map(|r| {
                r.iter()
                    .enumerate()
                    .map(|(c, cell)| wrap(cell, widths[c], &[], &[]))
                    .collect()
            })
            .collect();
        let tall = wrapped.iter().skip(1).any(|r| r.iter().any(|c| c.len() > 1));
        let line_sep = |cross: char| -> Vec<Cell> {
            let mut v = Vec::new();
            for (c, w) in widths.iter().enumerate() {
                if c > 0 {
                    v.extend([('─', Style::fg(DIM)), (cross, Style::fg(DIM)), ('─', Style::fg(DIM))]);
                }
                v.extend(std::iter::repeat_n(('─', Style::fg(DIM)), *w));
            }
            v
        };
        for (ri, row) in wrapped.iter().enumerate() {
            if ri == 1 || (ri > 1 && tall) {
                self.push(line_sep('┼'), src);
            }
            let h = row.iter().map(|c| c.len()).max().unwrap_or(1);
            for k in 0..h {
                let mut v: Vec<Cell> = Vec::new();
                for (c, cell) in row.iter().enumerate() {
                    if c > 0 {
                        v.extend([(' ', Style::new()), ('│', Style::fg(DIM)), (' ', Style::new())]);
                    }
                    let part: &[Cell] = cell.get(k).map(Vec::as_slice).unwrap_or(&[]);
                    let pad = widths[c].saturating_sub(width_of(part));
                    let (l, r) = match aligns.get(c).copied().unwrap_or('l') {
                        'r' => (pad, 0),
                        'c' => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    v.extend(std::iter::repeat_n((' ', Style::new()), l));
                    v.extend_from_slice(part);
                    v.extend(std::iter::repeat_n((' ', Style::new()), r));
                }
                while v.last().is_some_and(|c| c.0 == ' ') {
                    v.pop();
                }
                self.push(v, src + ri + usize::from(ri > 0));
            }
        }
    }

    fn list_item(&mut self, indent: usize, marker: &str, ordered: bool, text: &str, src: usize) {
        if self.lists.is_empty() {
            self.gap(src);
        }
        while self.lists.last().is_some_and(|&top| top > indent) {
            self.lists.pop();
        }
        if self.lists.last() != Some(&indent) {
            self.lists.push(indent);
        }
        let depth = self.lists.len() - 1;
        let pad = 2 * depth;
        let (mark, body) = if ordered {
            (format!("{} ", marker), text.to_string())
        } else {
            let bullet = ['•', '◦', '▪'][depth % 3];
            match text.get(..3) {
                Some("[ ]") => ("☐ ".to_string(), text[3..].trim_start().to_string()),
                Some("[x]" | "[X]") => ("☑ ".to_string(), text[3..].trim_start().to_string()),
                _ => (format!("{} ", bullet), text.to_string()),
            }
        };
        let color = if ordered { NUM } else { MACRO };
        let mut first: Vec<Cell> = vec![(' ', Style::new()); pad];
        first.extend(text_cells(&mark, Style::fg(color)));
        let hang = pad + mark.chars().map(cw).sum::<usize>();
        let rest = vec![(' ', Style::new()); hang];
        let cells = inline(&body, Style::new());
        for l in wrap(&cells, self.width, &first, &rest) {
            self.push(l, src);
        }
        self.para_indent = hang;
    }
}

fn bar(depth: usize, color: Color) -> Vec<Cell> {
    let mut v = Vec::new();
    for _ in 0..depth.max(1) {
        v.push(('▎', Style::fg(color)));
        v.push((' ', Style::new()));
    }
    v
}

/// Renders a Markdown file for a window `width` columns wide.
pub fn render(lines: &[Vec<char>], width: usize) -> Vec<Line> {
    let src: Vec<String> = lines.iter().map(|l| l.iter().collect()).collect();
    let mut r = Renderer {
        out: Vec::new(),
        width: width.clamp(20, MAX_WIDTH),
        para: Vec::new(),
        para_src: 0,
        para_indent: 0,
        lists: Vec::new(),
        center: false,
    };
    let mut i = 0;
    while i < src.len() {
        let line = &src[i];
        let t = line.trim();
        if t.is_empty() {
            r.flush();
            i += 1;
            continue;
        }
        // HTML comments, also over several lines.
        if t.starts_with("<!--") {
            r.flush();
            while i < src.len() && !src[i].contains("-->") {
                i += 1;
            }
            i += 1;
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let in_list = !r.lists.is_empty();
        // A list ends at a line that isn't indented under it.
        if in_list && indent == 0 && md_list_marker(&line.chars().collect::<Vec<_>>()).is_none() {
            r.flush();
            r.lists.clear();
            r.para_indent = 0;
        }
        if let Some((ch, len, f_indent, info)) = fence(line) {
            r.flush();
            let mut body = Vec::new();
            let start = i;
            i += 1;
            while i < src.len() {
                if let Some((c2, l2, _, info2)) = fence(&src[i]) {
                    if c2 == ch && l2 >= len && info2.is_empty() {
                        i += 1;
                        break;
                    }
                }
                let l = &src[i];
                let cut = l.len() - l.trim_start().len();
                body.push(l[cut.min(f_indent)..].to_string());
                i += 1;
            }
            // Indented under a list item: the block sits under its text.
            let indent = if in_list && f_indent >= 2 { r.para_indent } else { 0 };
            r.code(&body, &info, indent, start);
            continue;
        }
        if let Some((level, text)) = atx(line) {
            r.flush();
            r.lists.clear();
            r.para_indent = 0;
            r.heading(level, text, i);
            i += 1;
            continue;
        }
        // Setext: a paragraph underlined with === or ---.
        if !r.para.is_empty()
            && indent < 4
            && !t.is_empty()
            && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-'))
        {
            let text = r.para.join(" ");
            let src_line = r.para_src;
            r.para.clear();
            r.heading(if t.starts_with('=') { 1 } else { 2 }, text.trim(), src_line);
            i += 1;
            continue;
        }
        if is_rule(line) {
            r.flush();
            r.rule(i);
            i += 1;
            continue;
        }
        if t.contains('|') {
            if let Some(aligns) = src.get(i + 1).and_then(|n| table_aligns(n)) {
                r.flush();
                let start = i;
                let mut rows = vec![split_row(line)];
                i += 2;
                while i < src.len() && src[i].contains('|') && !src[i].trim().is_empty() {
                    rows.push(split_row(&src[i]));
                    i += 1;
                }
                r.table(&rows, &aligns, start);
                continue;
            }
        }
        if t.starts_with('>') {
            r.flush();
            let start = i;
            let mut body = Vec::new();
            while i < src.len() && src[i].trim_start().starts_with('>') {
                let mut s = src[i].trim_start();
                let mut depth = 0;
                while let Some(rest) = s.strip_prefix('>') {
                    depth += 1;
                    s = rest.strip_prefix(' ').unwrap_or(rest).trim_start_matches(' ');
                    if !s.starts_with('>') {
                        break;
                    }
                }
                body.push((depth, s.trim_end().to_string()));
                i += 1;
            }
            r.quote(&body, start);
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        if let Some((m, text_at, ordered)) = md_list_marker(&chars) {
            r.flush();
            let marker: String = chars[m..text_at].iter().collect::<String>().trim().to_string();
            let mut text: String = chars[text_at..].iter().collect();
            let start = i;
            i += 1;
            // Continuation lines: more of the item's first paragraph.
            while i < src.len() {
                let next = &src[i];
                if next.trim().is_empty()
                    || starts_block(next)
                    || (next.contains('|') && src.get(i + 1).and_then(|n| table_aligns(n)).is_some())
                {
                    break;
                }
                text.push(' ');
                text.push_str(next.trim());
                i += 1;
            }
            r.list_item(m, &marker, ordered, text.trim(), start);
            continue;
        }
        if html_block(t) {
            r.flush();
            let lower = t.to_ascii_lowercase();
            if lower.contains("align=\"center\"") || lower.contains("align='center'") || lower.starts_with("<center") {
                r.center = true;
            }
            static HEAD: OnceLock<Regex> = OnceLock::new();
            let head = re(&HEAD, r"(?is)<h([1-6])\b[^>]*>(.*?)</h[1-6]>");
            if let Some(c) = head.captures(t) {
                let level: usize = c[1].parse().unwrap_or(1);
                let text = html_to_md(&c[2]);
                r.heading(level, &text, i);
            } else {
                let text = html_to_md(t);
                if !text.trim().is_empty() {
                    r.para.push(text);
                    r.para_src = i;
                    r.flush();
                }
            }
            if lower.contains("</p>") || lower.contains("</div>") || lower.contains("</center>") {
                r.center = false;
            }
            i += 1;
            continue;
        }
        // An indented paragraph under a list item belongs to it.
        if in_list && indent >= 2 && r.para.is_empty() {
            r.para_src = i;
        } else if r.para.is_empty() {
            r.para_indent = 0;
            r.para_src = i;
        }
        r.para.push(line.clone());
        i += 1;
    }
    r.flush();
    r.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.cells.iter().map(|c| c.0).collect::<String>())
            .collect()
    }

    fn doc(s: &str, width: usize) -> Vec<String> {
        let lines: Vec<Vec<char>> = s.lines().map(|l| l.chars().collect()).collect();
        plain(&render(&lines, width))
    }

    #[test]
    fn inline_markup_goes() {
        let cells = inline(
            "a **b** *c* `d` [e](http://x) ![f](y.png) \\* &amp; <https://g.io>",
            Style::new(),
        );
        let s: String = cells.iter().map(|c| c.0).collect();
        assert_eq!(s, "a b c d e [image: f] * & https://g.io");
        let st = |ch: char| cells.iter().find(|c| c.0 == ch).unwrap().1;
        assert!(st('b').bold);
        assert!(st('c').italic);
        assert_eq!(st('d').bg, POP);
        assert_eq!(st('e').fg, FUNC);
        // snake_case isn't emphasis.
        let s: String = inline("a_b_c", Style::new()).iter().map(|c| c.0).collect();
        assert_eq!(s, "a_b_c");
    }

    #[test]
    fn paragraphs_wrap_and_join() {
        let out = doc("one two three\nfour five six seven", 20);
        assert_eq!(out, vec!["one two three four", "five six seven"]);
    }

    #[test]
    fn blocks() {
        let md = "# Title\n\nSome *text*.\n\n- a\n- b\n  - c\n1. one\n\n> quoted\n\n```rust\nlet x = 1;\n```\n\n---";
        let out = doc(md, 30);
        assert_eq!(out[0], "Title");
        assert!(out[1].starts_with("━━━"));
        assert!(out.contains(&"Some text.".to_string()));
        assert!(out.contains(&"• a".to_string()));
        assert!(out.contains(&"  ◦ c".to_string()));
        assert!(out.contains(&"1. one".to_string()));
        assert!(out.contains(&"▎ quoted".to_string()));
        assert!(out.iter().any(|l| l == " let x = 1;"));
        assert!(out.last().unwrap().starts_with("───"));
    }

    #[test]
    fn tables_line_up() {
        let out = doc("| Key | Action |\n|---|--:|\n| `a` | b |\n| ccc | d |", 40);
        assert_eq!(
            out,
            vec!["Key │ Action", "────┼───────", "a   │      b", "ccc │      d"]
        );
    }

    #[test]
    fn readme_html() {
        let md = "<p align=\"center\">\n  <img src=\"x.png\" alt=\"ren\">\n</p>\n<h1 align=\"center\">ren</h1>\n<p align=\"center\">A <code>:</code> IDE</p>";
        let lines: Vec<Vec<char>> = md.lines().map(|l| l.chars().collect()).collect();
        let out = render(&lines, 40);
        let text = plain(&out);
        assert!(text.contains(&"[image: ren]".to_string()));
        assert!(text.contains(&"ren".to_string()));
        assert!(text.contains(&"A : IDE".to_string()));
        assert!(out.iter().filter(|l| !l.cells.is_empty()).all(|l| l.center));
    }
}
