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
    /// `'…'` (multi-line, `''` inside); `"…"` and `` `…` `` are names, not strings.
    Sql,
    /// `"…"`, `$"…{x}…"`, `@"…"`, `"""…"""`, `'c'`.
    CSharp,
    /// `"…"` with backslashes as they are (Visual Studio solutions).
    Sln,
    /// Markup: handled by `highlight_xml`.
    Xml,
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
    /// XML: inside a tag (`<a b="c"` before its `>`).
    Tag,
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
    /// Keywords and types match in any case (lists in lower case): SQL, Dockerfile.
    pub ci: bool,
    /// Keywords only as the first word of a line (Dockerfile instructions).
    pub lead: bool,
    /// `$name` and `${…}` outside strings are variables.
    pub vars: bool,
    /// `#word` at the start of a line is a directive (`#region`, `#if`).
    pub directives: bool,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
};

static JSON: LangDef = LangDef {
    name: "JSON",
    // Jupyter notebooks are JSON.
    exts: &["json", "jsonc", "ipynb"],
    keywords: &["true", "false", "null"],
    types: &[],
    line_comment: &[],
    block: None,
    strs: Strs::Json,
    macros: false,
    caps_types: false,
    plain: false,
    ci: false,
    lead: false,
    vars: false,
    directives: false,
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
    ci: false,
    lead: false,
    vars: false,
    directives: false,
};

static SQL: LangDef = LangDef {
    name: "SQL",
    exts: &["sql", "pgsql", "ddl"],
    keywords: &[
        "add",
        "all",
        "alter",
        "always",
        "analyze",
        "and",
        "any",
        "as",
        "asc",
        "begin",
        "between",
        "by",
        "cascade",
        "case",
        "check",
        "collate",
        "column",
        "commit",
        "conflict",
        "constraint",
        "create",
        "cross",
        "current_date",
        "current_time",
        "current_timestamp",
        "database",
        "declare",
        "default",
        "delete",
        "desc",
        "distinct",
        "do",
        "drop",
        "else",
        "end",
        "except",
        "exec",
        "execute",
        "exists",
        "explain",
        "extension",
        "false",
        "fetch",
        "first",
        "for",
        "foreign",
        "from",
        "full",
        "function",
        "generated",
        "go",
        "grant",
        "group",
        "having",
        "identity",
        "if",
        "ilike",
        "in",
        "index",
        "inner",
        "insert",
        "intersect",
        "into",
        "is",
        "join",
        "key",
        "language",
        "last",
        "lateral",
        "left",
        "like",
        "limit",
        "materialized",
        "merge",
        "natural",
        "next",
        "not",
        "nothing",
        "null",
        "nulls",
        "of",
        "offset",
        "on",
        "only",
        "or",
        "order",
        "outer",
        "over",
        "partition",
        "primary",
        "procedure",
        "recursive",
        "references",
        "rename",
        "replace",
        "restrict",
        "return",
        "returning",
        "returns",
        "revoke",
        "right",
        "rollback",
        "row",
        "rows",
        "schema",
        "select",
        "sequence",
        "set",
        "table",
        "temp",
        "temporary",
        "then",
        "to",
        "top",
        "transaction",
        "trigger",
        "true",
        "truncate",
        "type",
        "union",
        "unique",
        "update",
        "using",
        "vacuum",
        "values",
        "view",
        "when",
        "where",
        "window",
        "with",
        "without",
    ],
    types: &[
        "bigint",
        "bigserial",
        "binary",
        "bit",
        "blob",
        "bool",
        "boolean",
        "bytea",
        "char",
        "character",
        "cidr",
        "clob",
        "date",
        "datetime",
        "datetime2",
        "datetimeoffset",
        "decimal",
        "double",
        "enum",
        "float",
        "inet",
        "int",
        "int2",
        "int4",
        "int8",
        "integer",
        "interval",
        "json",
        "jsonb",
        "money",
        "nchar",
        "ntext",
        "numeric",
        "nvarchar",
        "precision",
        "real",
        "serial",
        "smallint",
        "smallserial",
        "text",
        "time",
        "timestamp",
        "timestamptz",
        "timetz",
        "tinyint",
        "uniqueidentifier",
        "uuid",
        "varbinary",
        "varchar",
        "xml",
    ],
    line_comment: &["--"],
    block: Some(("/*", "*/")),
    strs: Strs::Sql,
    macros: false,
    // SQL is often written in capitals: they say nothing about types.
    caps_types: false,
    plain: false,
    ci: true,
    lead: false,
    vars: false,
    directives: false,
};

/// Found by file name (`Dockerfile`, `Dockerfile.flyway`, `app.dockerfile`,
/// `Containerfile`), see `detect`.
static DOCKERFILE: LangDef = LangDef {
    name: "Dockerfile",
    exts: &["dockerfile", "containerfile"],
    keywords: &[
        "add",
        "arg",
        "as",
        "cmd",
        "copy",
        "entrypoint",
        "env",
        "expose",
        "from",
        "healthcheck",
        "label",
        "maintainer",
        "onbuild",
        "run",
        "shell",
        "stopsignal",
        "user",
        "volume",
        "workdir",
    ],
    types: &[],
    line_comment: &["#"],
    block: None,
    strs: Strs::Shell,
    macros: false,
    caps_types: false,
    plain: false,
    ci: true,
    lead: true,
    vars: true,
    directives: false,
};

static CSHARP: LangDef = LangDef {
    name: "C#",
    exts: &["cs", "csx"],
    keywords: &[
        "abstract",
        "add",
        "alias",
        "and",
        "as",
        "async",
        "await",
        "base",
        "break",
        "case",
        "catch",
        "checked",
        "class",
        "const",
        "continue",
        "default",
        "delegate",
        "do",
        "else",
        "enum",
        "event",
        "explicit",
        "extern",
        "false",
        "file",
        "finally",
        "fixed",
        "for",
        "foreach",
        "get",
        "global",
        "goto",
        "if",
        "implicit",
        "in",
        "init",
        "interface",
        "internal",
        "is",
        "lock",
        "nameof",
        "namespace",
        "new",
        "not",
        "null",
        "operator",
        "or",
        "out",
        "override",
        "params",
        "partial",
        "private",
        "protected",
        "public",
        "readonly",
        "record",
        "ref",
        "remove",
        "required",
        "return",
        "scoped",
        "sealed",
        "set",
        "sizeof",
        "stackalloc",
        "static",
        "struct",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "unchecked",
        "unsafe",
        "using",
        "value",
        "var",
        "virtual",
        "volatile",
        "when",
        "where",
        "while",
        "with",
        "yield",
        "from",
        "select",
        "orderby",
        "group",
        "into",
        "join",
        "let",
        "on",
        "equals",
        "by",
        "ascending",
        "descending",
    ],
    types: &[
        "bool", "byte", "char", "decimal", "double", "dynamic", "float", "int", "long", "nint", "nuint", "object",
        "sbyte", "short", "string", "uint", "ulong", "ushort", "void",
    ],
    line_comment: &["//"],
    block: Some(("/*", "*/")),
    strs: Strs::CSharp,
    macros: false,
    caps_types: true,
    plain: false,
    ci: false,
    lead: false,
    vars: false,
    directives: true,
};

/// XML and the .NET files written in it (projects, MSBuild, XAML, resources).
static XML: LangDef = LangDef {
    name: "XML",
    exts: &[
        "xml", "csproj", "fsproj", "vbproj", "props", "targets", "slnx", "xaml", "axaml", "resx", "nuspec", "config",
        "xsd", "xsl", "xslt", "svg", "plist",
    ],
    keywords: &[],
    types: &[],
    line_comment: &[],
    block: Some(("<!--", "-->")),
    strs: Strs::Xml,
    macros: false,
    caps_types: false,
    plain: false,
    ci: false,
    lead: false,
    vars: false,
    directives: false,
};

/// Visual Studio solutions (`.sln`).
static SLN: LangDef = LangDef {
    name: "Solution",
    exts: &["sln"],
    keywords: &[
        "Project",
        "EndProject",
        "ProjectSection",
        "EndProjectSection",
        "Global",
        "EndGlobal",
        "GlobalSection",
        "EndGlobalSection",
        "preSolution",
        "postSolution",
        "preProject",
        "postProject",
    ],
    types: &[],
    line_comment: &["#"],
    block: None,
    strs: Strs::Sln,
    macros: false,
    caps_types: false,
    plain: false,
    ci: false,
    lead: false,
    vars: false,
    directives: false,
};

static LANGS: &[&LangDef] = &[
    &RUST,
    &C,
    &PYTHON,
    &JS,
    &GO,
    &SHELL,
    &TOML,
    &JSON,
    &LUA,
    &SQL,
    &DOCKERFILE,
    &CSHARP,
    &XML,
    &SLN,
];

impl LangDef {
    /// Whether a line's colours can depend on the lines above it.
    pub fn multiline(&self) -> bool {
        self.block.is_some()
            || matches!(
                self.strs,
                Strs::Rust | Strs::Python | Strs::Js | Strs::Go | Strs::Toml | Strs::Sql | Strs::CSharp
            )
    }
}

pub fn plain() -> &'static LangDef {
    &PLAIN
}

/// `Dockerfile`, `Dockerfile.flyway`, `Dockerfile.dev`, `app.dockerfile`,
/// `Containerfile` (not their `.dockerignore` files).
pub fn is_dockerfile(name: &str) -> bool {
    let n = name.to_lowercase();
    if n.ends_with(".dockerignore") {
        return false;
    }
    ["dockerfile", "containerfile"]
        .iter()
        .any(|d| n == *d || n.starts_with(&format!("{}.", d)) || n.ends_with(&format!(".{}", d)))
}

pub fn detect(path: &Path) -> &'static LangDef {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if is_dockerfile(&name) {
        return &DOCKERFILE;
    }
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
        Strs::Sql => (at(0)? == '\'').then(|| (1, s('\'', Esc::No, 0, true))),
        Strs::Sln => (at(0)? == '"').then(|| (1, s('"', Esc::No, 0, false))),
        Strs::CSharp => {
            // Prefixes `$` (interpolated) and `@` (verbatim), in either order.
            let (mut k, mut interp, mut verbatim) = (0, false, false);
            while k < 3 {
                match at(k) {
                    Some('$') => interp = true,
                    Some('@') => verbatim = true,
                    _ => break,
                }
                k += 1;
            }
            let fmt = if interp { FMT_EXPR } else { 0 };
            match at(k)? {
                '"' if at(k + 1) == Some('"') && at(k + 2) == Some('"') => Some((
                    k + 3,
                    StrState {
                        triple: true,
                        ..s('"', Esc::No, fmt, true)
                    },
                )),
                '"' if verbatim => Some((k + 1, s('"', Esc::No, fmt, true))),
                '"' => Some((k + 1, s('"', Esc::Yes, fmt, false))),
                '\'' if k == 0 => Some((1, s('\'', Esc::Yes, 0, false))),
                _ => None,
            }
        }
        Strs::Xml => None,
    }
}

/// Dockerfile: an instruction is the line's first word (after `ONBUILD`),
/// and `AS` names a `FROM` stage.
fn at_lead(line: &[char], start: usize, word: &str) -> bool {
    let before: String = line[..start].iter().collect();
    let before = before.trim();
    before.is_empty()
        || before.eq_ignore_ascii_case("onbuild")
        || (word == "as"
            && before
                .split_whitespace()
                .next()
                .is_some_and(|f| f.eq_ignore_ascii_case("from")))
}

/// `$name`, `${name:-x}`: end (exclusive) of the variable at `line[i] == '$'`.
fn shell_var(line: &[char], i: usize) -> Option<usize> {
    match line.get(i + 1)? {
        '{' => balanced(line, i + 1, '{', '}', '\0'),
        c if c.is_alphabetic() || *c == '_' => {
            let mut e = i + 1;
            while e < line.len() && (line[e].is_alphanumeric() || line[e] == '_') {
                e += 1;
            }
            Some(e)
        }
        _ => None,
    }
}

/// SQL parameters and variables: `@name`, `@@version`, `$1`, `:name` (not a
/// `::type` cast). End (exclusive), at `i`.
fn sql_param(line: &[char], i: usize) -> Option<usize> {
    let c = line[i];
    let next = *line.get(i + 1)?;
    let word_end = |from: usize| {
        let mut e = from;
        while e < line.len() && (line[e].is_alphanumeric() || line[e] == '_') {
            e += 1;
        }
        e
    };
    match c {
        '@' if next == '@' && line.get(i + 2).is_some_and(|c| c.is_alphabetic()) => Some(word_end(i + 2)),
        '@' if next.is_alphabetic() || next == '_' => Some(word_end(i + 1)),
        '$' if next.is_ascii_digit() => Some(word_end(i + 1)),
        ':' if (next.is_alphabetic() || next == '_') && (i == 0 || line[i - 1] != ':') => Some(word_end(i + 1)),
        _ => None,
    }
}

fn xml_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

/// `&amp;` `&#10;` `&#x1F;`: end (exclusive) of the entity at `line[i] == '&'`.
fn xml_entity(line: &[char], i: usize) -> Option<usize> {
    for (j, &c) in line.iter().enumerate().skip(i + 1).take(12) {
        if c == ';' {
            return (j > i + 1).then_some(j + 1);
        }
        if !(c.is_alphanumeric() || c == '#') {
            return None;
        }
    }
    None
}

/// MSBuild `$(Property)`, `@(Items)`, `%(Metadata)`: end (exclusive).
fn msbuild_ref(line: &[char], i: usize) -> Option<usize> {
    if matches!(line[i], '$' | '@' | '%') && line.get(i + 1) == Some(&'(') {
        balanced(line, i + 1, '(', ')', '\0')
    } else {
        None
    }
}

/// XML: tag names as keywords, attribute names as types, values as strings
/// (entities and MSBuild `$(…)` inside them marked), comments, and text with
/// its entities. `st` carries an open comment, tag or value to the next line.
fn highlight_xml(line: &[char], st: &mut State, out: &mut [Tok]) {
    let n = line.len();
    let mut i = 0;
    while i < n {
        match *st {
            State::Block => {
                let start = i;
                while i < n && !starts_with(line, i, "-->") {
                    i += 1;
                }
                if i < n {
                    i += 3;
                    *st = State::Code;
                }
                out[start..i.min(n)].fill(Tok::Comment);
            }
            State::Str(s) => {
                // An attribute value; the tag goes on after it.
                while i < n {
                    let c = line[i];
                    if c == s.quote {
                        out[i] = Tok::Quote;
                        i += 1;
                        *st = State::Tag;
                        break;
                    }
                    let special = if c == '&' {
                        xml_entity(line, i).map(|e| (e, Tok::Escape))
                    } else {
                        msbuild_ref(line, i).map(|e| (e, Tok::Interp))
                    };
                    match special {
                        Some((e, tok)) => {
                            out[i..e].fill(tok);
                            i = e;
                        }
                        None => {
                            out[i] = Tok::Str;
                            i += 1;
                        }
                    }
                }
            }
            State::Tag => {
                let c = line[i];
                if c == '>' || ((c == '/' || c == '?') && line.get(i + 1) == Some(&'>')) {
                    i += if c == '>' { 1 } else { 2 };
                    *st = State::Code;
                } else if c == '"' || c == '\'' {
                    out[i] = Tok::Quote;
                    i += 1;
                    *st = State::Str(StrState {
                        quote: c,
                        triple: false,
                        hashes: 0,
                        esc: Esc::No,
                        fmt: 0,
                        multi: true,
                    });
                } else if xml_name_char(c) {
                    let start = i;
                    while i < n && xml_name_char(line[i]) {
                        i += 1;
                    }
                    out[start..i].fill(Tok::Type);
                } else {
                    i += 1;
                }
            }
            State::Code => {
                let c = line[i];
                if starts_with(line, i, "<!--") {
                    out[i..(i + 4).min(n)].fill(Tok::Comment);
                    i += 4;
                    *st = State::Block;
                } else if c == '<'
                    && line
                        .get(i + 1)
                        .is_some_and(|&d| d == '/' || d == '?' || d == '!' || xml_name_char(d))
                {
                    // `<name`, `</name`, `<?xml`, `<!DOCTYPE`.
                    i += 1;
                    let decl = matches!(line.get(i), Some('?' | '!'));
                    if matches!(line.get(i), Some('/' | '?' | '!')) {
                        i += 1;
                    }
                    let start = i;
                    while i < n && xml_name_char(line[i]) {
                        i += 1;
                    }
                    out[start..i].fill(if decl { Tok::Macro } else { Tok::Keyword });
                    *st = State::Tag;
                } else if c == '&' {
                    match xml_entity(line, i) {
                        Some(e) => {
                            out[i..e].fill(Tok::Escape);
                            i = e;
                        }
                        None => i += 1,
                    }
                } else if let Some(e) = msbuild_ref(line, i) {
                    out[i..e].fill(Tok::Macro);
                    i = e;
                } else {
                    i += 1;
                }
            }
        }
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
    if lang.strs == Strs::Xml {
        highlight_xml(line, st, &mut out);
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
            State::Code | State::Tag => {}
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
        if lang.directives && c == '#' && line[..i].iter().all(|c| c.is_whitespace()) {
            // `#region`, `#if DEBUG`, `#nullable enable`.
            let start = i;
            i += 1;
            while i < n && line[i].is_alphanumeric() {
                i += 1;
            }
            out[start..i].fill(Tok::Macro);
            continue;
        }
        if lang.vars && c == '$' {
            if let Some(e) = shell_var(line, i) {
                out[i..e].fill(Tok::Macro);
                i = e;
                continue;
            }
        }
        if lang.strs == Strs::Sql {
            if c == '"' || c == '`' {
                // A quoted name: no keywords inside.
                let e = line[i + 1..].iter().position(|&q| q == c).map_or(n, |p| i + p + 2);
                i = e;
                continue;
            }
            if let Some(e) = sql_param(line, i) {
                out[i..e].fill(Tok::Macro);
                i = e;
                continue;
            }
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
            let mut word: String = line[start..i].iter().collect();
            if lang.ci {
                word.make_ascii_lowercase();
            }
            let w = word.as_str();
            let next_nb = line[i..].iter().find(|c| !c.is_whitespace()).copied();
            let keyword = lang.keywords.contains(&w) && (!lang.lead || at_lead(line, start, w));
            let tok = if keyword {
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

    /// One letter per character: k keyword, t type, m macro/variable, f function,
    /// c comment, n number, then the string letters of `shape`.
    fn kinds(t: &[Tok]) -> String {
        t.iter()
            .map(|t| match t {
                Tok::Keyword => 'k',
                Tok::Type => 't',
                Tok::Macro => 'm',
                Tok::Func => 'f',
                Tok::Comment => 'c',
                Tok::Number => 'n',
                Tok::Quote => 'q',
                Tok::Str => 's',
                Tok::Escape => 'e',
                Tok::Interp => 'i',
                Tok::Normal => '.',
            })
            .collect()
    }

    #[test]
    fn finds_new_languages_by_name() {
        for (f, lang) in [
            ("Dockerfile", "Dockerfile"),
            ("Dockerfile.flyway", "Dockerfile"),
            ("dockerfile.dev", "Dockerfile"),
            ("api.Dockerfile", "Dockerfile"),
            ("Containerfile", "Dockerfile"),
            ("Dockerfile.dockerignore", "Metin"),
            ("schema.sql", "SQL"),
            ("Program.cs", "C#"),
            ("App.csproj", "XML"),
            ("Directory.Build.props", "XML"),
            ("App.sln", "Solution"),
            ("App.slnx", "XML"),
            ("analysis.ipynb", "JSON"),
        ] {
            assert_eq!(detect(Path::new(f)).name, lang, "{}", f);
        }
    }

    #[test]
    fn sql_line() {
        assert_eq!(
            kinds(&hl(
                &SQL,
                "select id, \"Order\" from t where n = 'it''s' and x::int = @p -- c"
            )),
            "kkkkkk.............kkkk...kkkkk.....qssqqsq.kkk....ttt...mm.cccc"
        );
        // Keywords in any case; capitals aren't types.
        assert_eq!(kinds(&hl(&SQL, "SELECT Name FROM Users")), "kkkkkk......kkkk......");
        let mut st = State::default();
        let a: Vec<char> = "insert into t values ('line one".chars().collect();
        highlight_line(&SQL, &a, &mut st);
        assert!(matches!(st, State::Str(_)), "SQL strings go on over lines");
    }

    #[test]
    fn dockerfile_line() {
        assert_eq!(
            kinds(&hl(&DOCKERFILE, "FROM node:20 AS build")),
            "kkkk......nn.kk......"
        );
        // Instructions only lead a line: `from` in a RUN line is a word.
        assert_eq!(
            kinds(&hl(&DOCKERFILE, "RUN echo from $HOME ${A:-b} \"$X\"")),
            "kkk...........mmmmm.mmmmmmm.qiiq"
        );
        assert_eq!(kinds(&hl(&DOCKERFILE, "onbuild run make")), "kkkkkkk.kkk.....");
    }

    #[test]
    fn csharp_strings() {
        assert_eq!(
            shape(&hl(&CSHARP, r#"var s = $"Hi {user.Name}, {{x}}\n";"#)),
            "........qqsssiiiiiiiiiiisseeseeeeq."
        );
        assert_eq!(shape(&hl(&CSHARP, r#"@"C:\dir\" + 'x'"#)), "qqsssssssq...qsq");
        let mut st = State::default();
        let a: Vec<char> = r#"var j = """{"a": 1}"#.chars().collect();
        highlight_line(&CSHARP, &a, &mut st);
        assert!(matches!(st, State::Str(_)), "raw strings go on over lines");
        let b: Vec<char> = r#"""";"#.chars().collect();
        highlight_line(&CSHARP, &b, &mut st);
        assert_eq!(st, State::Code);
        assert_eq!(kinds(&hl(&CSHARP, "  #region Setup")), "..mmmmmmm.ttttt");
    }

    #[test]
    fn xml_tags() {
        assert_eq!(
            kinds(&hl(&XML, r#"<Item Include="$(Out)\a&amp;b.dll" />"#)),
            ".kkkk.ttttttt.qiiiiiisseeeeesssssq..."
        );
        assert_eq!(kinds(&hl(&XML, r#"<?xml version="1.0"?>"#)), "..mmm.ttttttt.qsssq..");
        assert_eq!(kinds(&hl(&XML, "<A>$(B) &lt; it's</A>")), ".k.mmmm.eeee.......k.");
        // Comments and tags go on over lines.
        let mut st = State::default();
        highlight_line(&XML, &"<!-- a".chars().collect::<Vec<_>>(), &mut st);
        assert_eq!(st, State::Block);
        let t = highlight_line(&XML, &"b --> <Tag".chars().collect::<Vec<_>>(), &mut st);
        assert_eq!(kinds(&t), "ccccc..kkk");
        assert_eq!(st, State::Tag);
        let t = highlight_line(&XML, &"  Name='x'>text".chars().collect::<Vec<_>>(), &mut st);
        assert_eq!(kinds(&t), "..tttt.qsq.....");
        assert_eq!(st, State::Code);
    }

    #[test]
    fn sln_line() {
        assert_eq!(
            kinds(&hl(&SLN, r#"Project("{FAE0}") = "App", "src\App.csproj""#)),
            "kkkkkkk.qssssssq....qsssq..qssssssssssssssq"
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
