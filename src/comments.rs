//! How each file type writes comments, for Ctrl+/. Covers many more types than
//! the highlighter knows, by file name and extension; anything else falls back
//! to the highlighter's language.

use crate::highlight::LangDef;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommentStyle {
    /// Line comment markers; the first is the one Ctrl+/ adds, all of them
    /// are recognised when removing.
    pub line: &'static [&'static str],
    /// Block comment (open, close): used when the type has no line comment.
    pub block: Option<(&'static str, &'static str)>,
}

const fn style(line: &'static [&'static str], block: Option<(&'static str, &'static str)>) -> CommentStyle {
    CommentStyle { line, block }
}

/// `//` and `/* */`: the C family and its relatives.
const C_LIKE: CommentStyle = style(&["//"], Some(("/*", "*/")));
const SLASHES: CommentStyle = style(&["//"], None);
const HASH: CommentStyle = style(&["#"], None);
const DASHES: CommentStyle = style(&["--"], None);
const SEMI: CommentStyle = style(&[";"], None);
const PERCENT: CommentStyle = style(&["%"], None);
/// Block-only types.
const CSS: CommentStyle = style(&[], Some(("/*", "*/")));
const MARKUP: CommentStyle = style(&[], Some(("<!--", "-->")));

/// The comment style for a file, or the highlighter's when ren doesn't know
/// the type.
pub fn style_for(path: Option<&Path>, lang: &LangDef) -> CommentStyle {
    if let Some(p) = path {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if let Some(s) = by_name(&name) {
            return s;
        }
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if let Some(s) = by_ext(&ext) {
            return s;
        }
    }
    style(lang.line_comment, lang.block)
}

fn by_name(name: &str) -> Option<CommentStyle> {
    let hash = matches!(
        name,
        "dockerfile"
            | "containerfile"
            | "makefile"
            | "gnumakefile"
            | "cmakelists.txt"
            | "gemfile"
            | "rakefile"
            | "justfile"
            | "procfile"
            | "vagrantfile"
            | "brewfile"
            | "pipfile"
            | ".gitignore"
            | ".gitattributes"
            | ".dockerignore"
            | ".npmrc"
            | ".bashrc"
            | ".zshrc"
            | ".profile"
    ) || name.starts_with(".env")
        || crate::highlight::is_dockerfile(name);
    if hash {
        return Some(HASH);
    }
    match name {
        ".editorconfig" => Some(style(&["#", ";"], None)),
        _ => None,
    }
}

fn by_ext(ext: &str) -> Option<CommentStyle> {
    Some(match ext {
        "rs" | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "m" | "mm" | "js" | "mjs" | "cjs" | "jsx"
        | "ts" | "tsx" | "mts" | "cts" | "go" | "java" | "kt" | "kts" | "swift" | "cs" | "scala" | "sc" | "dart"
        | "groovy" | "gradle" | "proto" | "jsonc" | "json5" | "scss" | "less" | "sol" | "glsl" | "hlsl" | "wgsl"
        | "vert" | "frag" | "v" | "d" | "hx" => C_LIKE,
        "php" => style(&["//", "#"], Some(("/*", "*/"))),
        "zig" | "odin" => SLASHES,
        "fs" | "fsi" | "fsx" => style(&["//"], Some(("(*", "*)"))),
        "tf" | "hcl" => style(&["#", "//"], Some(("/*", "*/"))),
        "py" | "pyw" | "pyi" | "sh" | "bash" | "zsh" | "fish" | "rb" | "pl" | "pm" | "r" | "nix" | "ex" | "exs"
        | "yaml" | "yml" | "toml" | "conf" | "cfg" | "properties" | "mk" | "cmake" | "dockerfile" | "cr" | "coffee"
        | "nim" | "graphql" | "gql" | "tcl" | "ps1" | "psm1" | "jl" | "awk" | "sed" | "gitignore" | "env"
        | "service" | "desktop" | "tsv" | "sln" => HASH,
        "ini" | "reg" => style(&[";", "#"], None),
        "lua" => style(&["--"], Some(("--[[", "]]"))),
        "sql" => style(&["--"], Some(("/*", "*/"))),
        "hs" | "lhs" | "elm" | "purs" => style(&["--"], Some(("{-", "-}"))),
        "ada" | "adb" | "ads" | "vhd" | "vhdl" => DASHES,
        "lisp" | "lsp" | "clj" | "cljs" | "cljc" | "edn" | "el" | "scm" | "ss" | "rkt" | "asm" | "s" => SEMI,
        "tex" | "sty" | "cls" | "bib" | "erl" | "hrl" | "mat" => PERCENT,
        "vim" => style(&["\""], None),
        "vb" | "vbs" | "bas" => style(&["'"], None),
        "bat" | "cmd" => style(&["REM", "::"], None),
        "css" => CSS,
        "html" | "htm" | "xhtml" | "xml" | "svg" | "xsl" | "xslt" | "xaml" | "plist" | "md" | "markdown" | "vue"
        | "svelte" | "astro" | "csproj" | "fsproj" | "vbproj" | "props" | "targets" | "jsp" | "erb" | "hbs"
        | "handlebars" | "njk" | "slnx" | "axaml" | "resx" | "nuspec" | "config" | "xsd" => MARKUP,
        // JSON has no comments.
        "json" => style(&[], None),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight;

    fn of(p: &str) -> CommentStyle {
        let p = Path::new(p);
        style_for(Some(p), highlight::detect(p))
    }

    #[test]
    fn styles() {
        assert_eq!(of("src/main.rs").line, &["//"]);
        assert_eq!(of("app.py").line, &["#"]);
        assert_eq!(of("q.sql").line, &["--"]);
        assert_eq!(of("Dockerfile").line, &["#"]);
        assert_eq!(of("Dockerfile.flyway").line, &["#"]);
        assert_eq!(of("db.Containerfile").line, &["#"]);
        assert_eq!(of("App.sln").line, &["#"]);
        assert_eq!(of("Program.cs").line, &["//"]);
        assert_eq!(of("App.csproj").block, Some(("<!--", "-->")));
        assert!(of("nb.ipynb").line.is_empty() && of("nb.ipynb").block.is_none());
        assert_eq!(of(".env.local").line, &["#"]);
        // YAML used to borrow the TOML/INI markers (`#` and `;`).
        assert_eq!(of("ci.yml").line, &["#"]);
        assert_eq!(
            of("site.css"),
            CommentStyle {
                line: &[],
                block: Some(("/*", "*/"))
            }
        );
        assert_eq!(of("index.html").block, Some(("<!--", "-->")));
        assert_eq!(of("README.md").block, Some(("<!--", "-->")));
        assert!(of("package.json").line.is_empty() && of("package.json").block.is_none());
        // Unknown types fall back to the highlighter (plain text: none).
        assert!(of("notes.txt").line.is_empty());
    }
}
