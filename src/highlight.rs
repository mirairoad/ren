use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Normal,
    Keyword,
    Type,
    Str,
    Comment,
    Number,
    Func,
    Macro,
    /// `\n`, `\u{1F600}`, `{{`, `%%` inside a string.
    Escape,
    /// A placeholder inside a string: `{name}`, `{:?}`, `%5.2f`, `${expr}`, `$var`.
    Interp,
    /// The quotes (and prefix: `r#`, `f`, `b`) around a string.
    Quote,
}

/// How a language writes string literals.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strs {
    None,
    Rust,
    C,
    Python,
    Js,
    Go,
    Shell,
    Toml,
    Json,
    Lua,
}

// What a string treats as a placeholder.
/// `{name:spec}` and `{{ }}` (Rust `format!`, Python `.format`).
const FMT_BRACES: u8 = 1;
/// `{expression}` and `{{ }}` (Python f-strings).
const FMT_EXPR: u8 = 2;
/// `${expression}` (JS template literals).
const FMT_DOLLAR: u8 = 4;
/// `%s %5.2f %v` and `%%` (C, Go, Python, Lua).
const FMT_PRINTF: u8 = 8;
/// `$name ${name} $(cmd)` (shell double quotes).
const FMT_SHELL: u8 = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Esc {
    /// Backslash escapes are coloured.
    Yes,
    /// Python raw strings: `\"` still doesn't end the string, nothing is coloured.
    Soft,
    /// Rust raw strings, Go backticks, shell single quotes.
    No,
}

/// An open string literal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StrState {
    quote: char,
    triple: bool,
    /// Rust raw strings: `r##"…"##`.
    hashes: u8,
    esc: Esc,
    fmt: u8,
    /// Can continue on the next line.
    multi: bool,
}

/// Highlighter state carried from one line to the next.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    #[default]
    Code,
    Block,
    Str(StrState),
}

pub struct LangDef {
    pub name: &'static str,
    pub exts: &'static [&'static str],
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    pub line_comment: &'static [&'static str],
    pub block: Option<(&'static str, &'static str)>,
    pub strs: Strs,
    pub macros: bool,
    pub caps_types: bool,
    pub plain: bool,
}

static PLAIN: LangDef = LangDef {
    name: "Metin",
    exts: &[],
    keywords: &[],
    types: &[],
    line_comment: &[],
    block: None,
    strs: Strs::None,
    macros: false,
    caps_types: false,
    plain: true,
};

static RUST: LangDef = LangDef {
    name: "Rust",
    exts: &["rs"],
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self",
        "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield",
    ],
    types: &[
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool",
        "char", "str", "String", "Vec", "Option", "Result", "Box", "Some", "None", "Ok", "Err",
    ],
    line_comment: &["//"],
    block: Some(("/*", "*/")),
    strs: Strs::Rust,
    macros: true,
    caps_types: true,
    plain: false,
};

static C: LangDef = LangDef {
    name: "C/C++",
    exts: &["c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx"],
    keywords: &[
        "auto",
        "break",
        "case",
        "const",
        "continue",
        "default",
        "do",
        "else",
        "enum",
        "extern",
        "for",
        "goto",
        "if",
        "inline",
        "register",
        "return",
        "sizeof",
        "static",
        "struct",
        "switch",
        "typedef",
        "union",
        "volatile",
        "while",
        "class",
        "namespace",
        "template",
        "typename",
        "public",
        "private",
        "protected",
        "virtual",
        "override",
        "new",
        "delete",
        "this",
        "using",
        "nullptr",
        "true",
        "false",
        "include",
        "define",
        "ifdef",
        "ifndef",
        "endif",
        "pragma",
        "constexpr",
        "noexcept",
        "operator",
        "friend",
        "try",
        "catch",
        "throw",
    ],
    types: &[
        "int", "char", "float", "double", "long", "short", "signed", "unsigned", "void", "bool", "size_t", "uint8_t",
        "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t", "std",
    ],
    line_comment: &["//"],
    block: Some(("/*", "*/")),
    strs: Strs::C,
    macros: false,
    caps_types: true,
    plain: false,
};

static PYTHON: LangDef = LangDef {
    name: "Python",
    exts: &["py", "pyw", "pyi"],
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
        "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not",
        "or", "pass", "raise", "return", "True", "try", "while", "with", "yield", "self", "match", "case",
    ],
    types: &[
        "int", "str", "float", "bool", "list", "dict", "set", "tuple", "bytes", "object",
    ],
    line_comment: &["#"],
    block: None,
    strs: Strs::Python,
    macros: false,
    caps_types: true,
    plain: false,
};

static JS: LangDef = LangDef {
    name: "JS/TS",
    exts: &["js", "mjs", "cjs", "jsx", "ts", "tsx", "mts"],
    keywords: &[
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "null",
        "return",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "with",
        "yield",
        "async",
        "await",
        "of",
        "interface",
        "type",
        "enum",
        "implements",
        "private",
        "public",
        "protected",
        "readonly",
        "static",
        "as",
        "from",
    ],
    types: &["string", "number", "boolean", "any", "unknown", "never", "object"],
    line_comment: &["//"],
    block: Some(("/*", "*/")),
    strs: Strs::Js,
    macros: false,
    caps_types: true,
    plain: false,
};

static GO: LangDef = LangDef {
    name: "Go",
    exts: &["go"],
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
        "nil",
        "true",
        "false",
    ],
    types: &[
        "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "float32", "float64",
        "string", "bool", "byte", "rune", "error", "any",
    ],
    line_comment: &["//"],
    block: Some(("/*", "*/")),
    strs: Strs::Go,
    macros: false,
    caps_types: false,
    plain: false,
};

static SHELL: LangDef = LangDef {
    name: "Shell",
    exts: &["sh", "bash", "zsh", "fish"],
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "function", "in", "return",
        "local", "export", "echo", "exit", "source",
    ],
    types: &[],
    line_comment: &["#"],
    block: None,
    strs: Strs::Shell,
    macros: false,
    caps_types: false,
    plain: false,
};

static TOML: LangDef = LangDef {
    name: "TOML",
    exts: &["toml", "ini", "cfg", "conf", "yaml", "yml"],
    keywords: &["true", "false"],
    types: &[],
    line_comment: &["#", ";"],
    block: None,
    strs: Strs::Toml,
    macros: false,
    caps_types: false,
    plain: false,
};

static JSON: LangDef = LangDef {
    name: "JSON",
    exts: &["json", "jsonc"],
    keywords: &["true", "false", "null"],
    types: &[],
    line_comment: &[],
    block: None,
    strs: Strs::Json,
    macros: false,
    caps_types: false,
    plain: false,
};

static LUA: LangDef = LangDef {
    name: "Lua",
    exts: &["lua"],
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not",
        "or", "repeat", "return", "then", "true", "until", "while",
    ],
    types: &[],
    line_comment: &["--"],
    block: Some(("--[[", "]]")),
    strs: Strs::Lua,
    macros: false,
    caps_types: false,
    plain: false,
};

static LANGS: &[&LangDef] = &[&RUST, &C, &PYTHON, &JS, &GO, &SHELL, &TOML, &JSON, &LUA];

impl LangDef {
    /// Whether a line's colours can depend on the lines above it.
    pub fn multiline(&self) -> bool {
        self.block.is_some() || matches!(self.strs, Strs::Rust | Strs::Python | Strs::Js | Strs::Go | Strs::Toml)
    }
}

pub fn plain() -> &'static LangDef {
    &PLAIN
}

pub fn detect(path: &Path) -> &'static LangDef {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    LANGS
        .iter()
        .copied()
        .find(|l| l.exts.contains(&ext.as_str()))
        .unwrap_or(&PLAIN)
}

fn starts_with(line: &[char], i: usize, pat: &str) -> bool {
    let mut n = 0;
    for (j, p) in (i..).zip(pat.chars()) {
        if line.get(j) != Some(&p) {
            return false;
        }
        n += 1;
    }
    n == pat.chars().count()
}

fn is_rust_char_lit(line: &[char], i: usize) -> bool {
    line.get(i + 1) == Some(&'\\') || line.get(i + 2) == Some(&'\'')
}

/// A string literal starting at `i` (prefix included): how many characters open
/// it, and how its contents are read.
fn string_open(strs: Strs, line: &[char], i: usize) -> Option<(usize, StrState)> {
    let at = |k: usize| line.get(i + k).copied();
    let s = |quote, esc, fmt, multi| StrState {
        quote,
        triple: false,
        hashes: 0,
        esc,
        fmt,
        multi,
    };
    let quote = |q: char| matches!(q, '"' | '\'');
    match strs {
        Strs::None => None,
        Strs::Rust => {
            // "…"  b"…"  c"…"  r#"…"#  br"…"  'x'  b'x'
            let k = usize::from(matches!(at(0), Some('b' | 'c')));
            if at(k) == Some('r') {
                let mut h = 0;
                while at(k + 1 + h) == Some('#') {
                    h += 1;
                }
                if at(k + 1 + h) != Some('"') {
                    return None;
                }
                let fmt = if k == 0 { FMT_BRACES } else { 0 };
                let st = StrState {
                    hashes: h.min(255) as u8,
                    ..s('"', Esc::No, fmt, true)
                };
                return Some((k + 2 + h, st));
            }
            match at(k)? {
                '"' => Some((k + 1, s('"', Esc::Yes, if k == 0 { FMT_BRACES } else { 0 }, true))),
                '\'' if k == 0 || at(0) == Some('b') => Some((k + 1, s('\'', Esc::Yes, 0, false))),
                _ => None,
            }
        }
        Strs::Python => {
            // Prefixes r u b f t (any case, up to two), single or triple quotes.
            let (mut k, mut raw, mut f, mut b) = (0, false, false, false);
            while k < 2 {
                match at(k).map(|c| c.to_ascii_lowercase()) {
                    Some('r') => raw = true,
                    Some('f' | 't') => f = true,
                    Some('b') => b = true,
                    Some('u') => {}
                    _ => break,
                }
                k += 1;
            }
            let q = at(k).filter(|&q| quote(q))?;
            let triple = at(k + 1) == Some(q) && at(k + 2) == Some(q);
            let fmt = if f {
                FMT_EXPR
            } else if b {
                FMT_PRINTF
            } else {
                FMT_BRACES | FMT_PRINTF
            };
            let esc = if raw { Esc::Soft } else { Esc::Yes };
            let st = StrState {
                triple,
                ..s(q, esc, fmt, triple)
            };
            Some((k + if triple { 3 } else { 1 }, st))
        }
        Strs::Js => match at(0)? {
            '`' => Some((1, s('`', Esc::Yes, FMT_DOLLAR, true))),
            q if quote(q) => Some((1, s(q, Esc::Yes, 0, false))),
            _ => None,
        },
        Strs::Go => match at(0)? {
            '`' => Some((1, s('`', Esc::No, FMT_PRINTF, true))),
            '"' => Some((1, s('"', Esc::Yes, FMT_PRINTF, false))),
            '\'' => Some((1, s('\'', Esc::Yes, 0, false))),
            _ => None,
        },
        Strs::C | Strs::Lua => match at(0)? {
            '"' => Some((1, s('"', Esc::Yes, FMT_PRINTF, false))),
            '\'' if strs == Strs::Lua => Some((1, s('\'', Esc::Yes, FMT_PRINTF, false))),
            '\'' => Some((1, s('\'', Esc::Yes, 0, false))),
            _ => None,
        },
        Strs::Shell => match at(0)? {
            '"' => Some((1, s('"', Esc::Yes, FMT_SHELL, false))),
            '\'' => Some((1, s('\'', Esc::No, 0, false))),
            _ => None,
        },
        Strs::Toml => {
            let q = at(0).filter(|&q| quote(q))?;
            let esc = if q == '"' { Esc::Yes } else { Esc::No };
            if at(1) == Some(q) && at(2) == Some(q) {
                return Some((
                    3,
                    StrState {
                        triple: true,
                        ..s(q, esc, 0, true)
                    },
                ));
            }
            Some((1, s(q, esc, 0, false)))
        }
        Strs::Json => (at(0)? == '"').then(|| (1, s('"', Esc::Yes, 0, false))),
    }
}

fn closes(line: &[char], i: usize, s: StrState) -> bool {
    let q = if s.triple { 3 } else { 1 };
    (0..q).all(|k| line.get(i + k) == Some(&s.quote))
        && (0..s.hashes as usize).all(|k| line.get(i + q + k) == Some(&'#'))
}

/// Length of the escape sequence at `line[i] == '\\'`.
fn esc_len(line: &[char], i: usize) -> usize {
    let n = line.len();
    let run = |from: usize, max: usize, ok: fn(&char) -> bool| {
        let mut j = from;
        while j < n && j - from < max && ok(&line[j]) {
            j += 1;
        }
        j - i
    };
    let hex = |c: &char| c.is_ascii_hexdigit();
    match line.get(i + 1) {
        None => 1,
        Some('x') => run(i + 2, 2, hex),
        Some('u') if line.get(i + 2) == Some(&'{') => run(i + 2, 10, |c| *c != '}') + 1,
        Some('u') => run(i + 2, 4, hex),
        Some('U') => run(i + 2, 8, hex),
        Some('N') if line.get(i + 2) == Some(&'{') => run(i + 2, 40, |c| *c != '}') + 1,
        Some('0'..='7') => run(i + 1, 3, |c| ('0'..='7').contains(c)),
        Some(_) => 2,
    }
    .min(n - i)
}

/// End (exclusive) of the bracketed run opening at `i`, on this line, stopping at `stop`.
fn balanced(line: &[char], i: usize, open: char, close: char, stop: char) -> Option<usize> {
    let mut depth = 0;
    for (j, &c) in line.iter().enumerate().skip(i) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(j + 1);
            }
        } else if c == stop {
            return None;
        }
    }
    None
}

/// `{}` `{name}` `{0:>8}` `{x:?}` `{a.b!r:^10}`: end (exclusive) of the placeholder at `i`.
fn brace_spec(line: &[char], i: usize) -> Option<usize> {
    let n = line.len();
    let mut j = i + 1;
    while j < n && (line[j].is_alphanumeric() || matches!(line[j], '_' | '.' | '[' | ']')) {
        j += 1;
    }
    if line.get(j) == Some(&'!') {
        j += 2;
    }
    if line.get(j) == Some(&':') {
        while j < n && !matches!(line[j], '}' | '{' | '"' | '\'') {
            j += 1;
        }
    }
    (line.get(j) == Some(&'}') && j - i < 64).then_some(j + 1)
}

/// `%s` `%-5.2f` `%+v` `%(name)s` `%lld`: end (exclusive) of the conversion at `i`.
fn printf_spec(line: &[char], i: usize) -> Option<usize> {
    let n = line.len();
    let mut j = i + 1;
    if line.get(j) == Some(&'(') {
        while j < n && line[j] != ')' {
            j += 1;
        }
        j += 1;
    }
    let skip = |j: &mut usize, ok: &dyn Fn(char) -> bool| {
        while *j < n && ok(line[*j]) {
            *j += 1;
        }
    };
    skip(&mut j, &|c| "-+#0".contains(c));
    skip(&mut j, &|c| c.is_ascii_digit() || c == '*');
    if line.get(j) == Some(&'.') {
        j += 1;
        skip(&mut j, &|c| c.is_ascii_digit() || c == '*');
    }
    skip(&mut j, &|c| "hlLzj".contains(c));
    let c = *line.get(j)?;
    "diouxXeEfFgGaAcspnrvTtbqUOw".contains(c).then_some(j + 1)
}

/// A placeholder or `{{`-style escape at `i` inside a string.
fn special(line: &[char], i: usize, s: StrState) -> Option<(usize, Tok)> {
    let c = line[i];
    let next = line.get(i + 1).copied();
    let fmt = s.fmt;
    if fmt & (FMT_BRACES | FMT_EXPR) != 0 {
        if (c == '{' && next == Some('{')) || (c == '}' && next == Some('}')) {
            return Some((i + 2, Tok::Escape));
        }
        if c == '{' {
            let end = if fmt & FMT_EXPR != 0 {
                balanced(line, i, '{', '}', s.quote)
            } else {
                brace_spec(line, i)
            };
            if let Some(e) = end {
                return Some((e, Tok::Interp));
            }
        }
    }
    if fmt & FMT_DOLLAR != 0 && c == '$' && next == Some('{') {
        if let Some(e) = balanced(line, i + 1, '{', '}', s.quote) {
            return Some((e, Tok::Interp));
        }
    }
    if fmt & FMT_PRINTF != 0 && c == '%' {
        if next == Some('%') {
            return Some((i + 2, Tok::Escape));
        }
        if let Some(e) = printf_spec(line, i) {
            return Some((e, Tok::Interp));
        }
    }
    if fmt & FMT_SHELL != 0 && c == '$' {
        let end = match next {
            Some('{') => balanced(line, i + 1, '{', '}', s.quote),
            Some('(') => balanced(line, i + 1, '(', ')', s.quote),
            Some(ch) if ch.is_alphabetic() || ch == '_' => {
                let mut e = i + 1;
                while e < line.len() && (line[e].is_alphanumeric() || line[e] == '_') {
                    e += 1;
                }
                Some(e)
            }
            Some(ch) if ch.is_ascii_digit() || "@#?$!*-".contains(ch) => Some(i + 2),
            _ => None,
        };
        if let Some(e) = end {
            return Some((e, Tok::Interp));
        }
    }
    None
}

/// Colours string contents from `i` to the closing quote or the end of the line.
fn scan_string(line: &[char], mut i: usize, s: StrState, out: &mut [Tok], st: &mut State) -> usize {
    let n = line.len();
    while i < n {
        let c = line[i];
        if c == s.quote && closes(line, i, s) {
            let e = (i + if s.triple { 3 } else { 1 } + s.hashes as usize).min(n);
            out[i..e].fill(Tok::Quote);
            *st = State::Code;
            return e;
        }
        if c == '\\' && s.esc != Esc::No {
            let (e, tok) = match s.esc {
                Esc::Yes => (i + esc_len(line, i), Tok::Escape),
                _ => ((i + 2).min(n), Tok::Str),
            };
            out[i..e].fill(tok);
            i = e;
            continue;
        }
        if s.fmt != 0 {
            if let Some((e, tok)) = special(line, i, s) {
                out[i..e].fill(tok);
                i = e;
                continue;
            }
        }
        out[i] = Tok::Str;
        i += 1;
    }
    n
}

pub fn highlight_line(lang: &LangDef, line: &[char], st: &mut State) -> Vec<Tok> {
    let n = line.len();
    let mut out = vec![Tok::Normal; n];
    if lang.plain {
        return out;
    }
    let mut i = 0;
    while i < n {
        match *st {
            State::Block => {
                let end = lang.block.map(|b| b.1).unwrap_or("*/");
                let start = i;
                while i < n {
                    if starts_with(line, i, end) {
                        i += end.chars().count();
                        *st = State::Code;
                        break;
                    }
                    i += 1;
                }
                out[start..i.min(n)].fill(Tok::Comment);
                continue;
            }
            State::Str(s) => {
                i = scan_string(line, i, s, &mut out, st);
                continue;
            }
            State::Code => {}
        }
        let c = line[i];
        if let Some((s, _)) = lang.block {
            if starts_with(line, i, s) {
                *st = State::Block;
                let l = s.chars().count();
                out[i..(i + l).min(n)].fill(Tok::Comment);
                i += l;
                continue;
            }
        }
        if lang.line_comment.iter().any(|lc| starts_with(line, i, lc)) {
            out[i..].fill(Tok::Comment);
            break;
        }
        if c == '\'' && lang.strs == Strs::Rust && !is_rust_char_lit(line, i) {
            // A lifetime or a label: 'a, 'static.
            let start = i;
            i += 1;
            while i < n && (line[i].is_alphanumeric() || line[i] == '_') {
                i += 1;
            }
            out[start..i].fill(Tok::Macro);
            continue;
        }
        if let Some((open, s)) = string_open(lang.strs, line, i) {
            let e = (i + open).min(n);
            out[i..e].fill(Tok::Quote);
            i = e;
            *st = State::Str(s);
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < n {
                let ch = line[i];
                if ch == '.' {
                    if i + 1 >= n || !line[i + 1].is_ascii_digit() {
                        break;
                    }
                } else if !(ch.is_ascii_alphanumeric() || ch == '_') {
                    break;
                }
                i += 1;
            }
            out[start..i].fill(Tok::Number);
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < n && (line[i].is_alphanumeric() || line[i] == '_') {
                i += 1;
            }
            let word: String = line[start..i].iter().collect();
            let w = word.as_str();
            let next_nb = line[i..].iter().find(|c| !c.is_whitespace()).copied();
            let tok = if lang.keywords.contains(&w) {
                Tok::Keyword
            } else if lang.types.contains(&w) {
                Tok::Type
            } else if lang.macros && line.get(i) == Some(&'!') && line.get(i + 1) != Some(&'=') {
                i += 1;
                Tok::Macro
            } else if next_nb == Some('(') {
                Tok::Func
            } else if lang.caps_types && c.is_uppercase() {
                Tok::Type
            } else {
                Tok::Normal
            };
            out[start..i].fill(tok);
            continue;
        }
        i += 1;
    }
    if let State::Str(s) = *st {
        if !s.multi {
            *st = State::Code;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hl(lang: &LangDef, s: &str) -> Vec<Tok> {
        let line: Vec<char> = s.chars().collect();
        highlight_line(lang, &line, &mut State::default())
    }

    /// One letter per character: q quote, s string, e escape, i placeholder, . anything else.
    fn shape(t: &[Tok]) -> String {
        t.iter()
            .map(|t| match t {
                Tok::Quote => 'q',
                Tok::Str => 's',
                Tok::Escape => 'e',
                Tok::Interp => 'i',
                _ => '.',
            })
            .collect()
    }

    #[test]
    fn rust_line() {
        let t = hl(&RUST, "let x: &'a str = \"hi\"; // c");
        assert_eq!(t[0], Tok::Keyword);
        assert_eq!(t[8], Tok::Macro);
        assert_eq!(t[17], Tok::Quote);
        assert_eq!(t[18], Tok::Str);
        assert_eq!(*t.last().unwrap(), Tok::Comment);
    }

    #[test]
    fn block() {
        let mut st = State::default();
        let l1: Vec<char> = "a /* b".chars().collect();
        let l2: Vec<char> = "c */ d".chars().collect();
        highlight_line(&RUST, &l1, &mut st);
        assert_eq!(st, State::Block);
        let t = highlight_line(&RUST, &l2, &mut st);
        assert_eq!(st, State::Code);
        assert_eq!(t[0], Tok::Comment);
        assert_eq!(t[5], Tok::Normal);
    }

    #[test]
    fn rust_format_string() {
        let t = hl(
            &RUST,
            r#"println!("Hello, {name}!,\nI am {age} {} {:?} {0:>8} {{x}}");"#,
        );
        assert_eq!(&shape(&t)[9..], "qsssssssiiiiiisseesssssiiiiisiisiiiisiiiiiiseeseeq..");
        // Not placeholders: spaces, quotes, unclosed.
        assert_eq!(shape(&hl(&RUST, r#""{ x } {\"a\": 1} {""#)), "qssssssseeseessssssq");
    }

    #[test]
    fn rust_raw_and_multiline() {
        let mut st = State::default();
        let a: Vec<char> = r##"let s = r#"a "\n" {x}"##.chars().collect();
        let t = highlight_line(&RUST, &a, &mut st);
        assert_eq!(&shape(&t)[8..], "qqqsssssssiii");
        assert!(matches!(st, State::Str(_)));
        let b: Vec<char> = r##"b"# + 1"##.chars().collect();
        let t = highlight_line(&RUST, &b, &mut st);
        assert_eq!(shape(&t), "sqq....");
        assert_eq!(t[6], Tok::Number);
        assert_eq!(st, State::Code);
        assert_eq!(shape(&hl(&RUST, r"let c = '\n';")), "........qeeq.");
    }

    #[test]
    fn python_strings() {
        assert_eq!(
            shape(&hl(&PYTHON, r#"f"hi {user.name!r} {{ok}}\t""#)),
            "qqsssiiiiiiiiiiiiiseesseeeeq"
        );
        assert_eq!(
            shape(&hl(&PYTHON, r#"r"\d+\"" "%s %(n)d 100%""#)),
            "qqsssssq.qiisiiiiisssssq"
        );
        let mut st = State::default();
        let a: Vec<char> = r#"x = """doc "quoted" "#.chars().collect();
        highlight_line(&PYTHON, &a, &mut st);
        assert!(matches!(st, State::Str(_)));
        let b: Vec<char> = r#"still doc""" # done"#.chars().collect();
        let t = highlight_line(&PYTHON, &b, &mut st);
        assert_eq!(st, State::Code);
        assert_eq!(t[9], Tok::Quote);
        assert_eq!(*t.last().unwrap(), Tok::Comment);
    }

    #[test]
    fn js_go_shell() {
        assert_eq!(shape(&hl(&JS, r"`a ${b + {c}.d} $e`")), "qssiiiiiiiiiiiisssq");
        assert_eq!(
            shape(&hl(&GO, r#"fmt.Printf("%-5d|%+v %%\n", x)"#)),
            "...........qiiiisiiiseeeeq...."
        );
        assert_eq!(
            shape(&hl(&SHELL, r#"echo "$HOME ${x:-1} $(pwd) \$" '$x'"#)),
            ".....qiiiiisiiiiiiisiiiiiiseeq.qssq"
        );
    }

    #[test]
    fn unclosed_single_line_string_ends_with_the_line() {
        let mut st = State::default();
        let a: Vec<char> = "x = 'oops".chars().collect();
        highlight_line(&PYTHON, &a, &mut st);
        assert_eq!(st, State::Code);
    }
}
