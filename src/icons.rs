//! Nerd Font file-type icons for the tree and tabs (`"icons": true` in settings).
//! Ghostty ships the Nerd Font symbols, so no extra font is needed there.

use crossterm::style::Color;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

const GREY: Color = rgb(0x8a, 0x93, 0xa3);

pub const FOLDER: (char, Color) = ('\u{f07b}', rgb(0x7f, 0xa8, 0xd8));
pub const FOLDER_OPEN: (char, Color) = ('\u{f07c}', rgb(0x7f, 0xa8, 0xd8));
/// Codicons (Nerd Fonts' `nf-cod-*`): error, warning, info, search.
pub const ERROR: char = '\u{ea87}';
pub const WARNING: char = '\u{ea6c}';
pub const INFO: char = '\u{ea74}';
pub const SEARCH: char = '\u{ea6d}';

/// Icon and colour for a file name.
pub fn file(name: &str) -> (char, Color) {
    let lower = name.to_lowercase();
    if crate::highlight::is_dockerfile(&lower) {
        return ('\u{e7b0}', rgb(0x45, 0x8e, 0xe6));
    }
    match lower.as_str() {
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" | ".dockerignore" => {
            return ('\u{e7b0}', rgb(0x45, 0x8e, 0xe6))
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" => return ('\u{e702}', rgb(0xf1, 0x50, 0x2f)),
        "makefile" | "justfile" => return ('\u{e615}', rgb(0x6d, 0x80, 0x86)),
        "license" | "license.md" | "licence" => return ('\u{e60a}', rgb(0xd0, 0xbf, 0x41)),
        "cargo.lock" | "uv.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "deno.lock"
        | "poetry.lock" => return ('\u{f023}', GREY),
        _ => {}
    }
    if lower == ".envrc" || crate::highlight::is_dotenv(&lower) {
        return ('\u{f084}', rgb(0xfa, 0xf7, 0x43));
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => ('\u{e7a8}', rgb(0xde, 0xa5, 0x84)),
        "py" | "pyi" => ('\u{e73c}', rgb(0xff, 0xbc, 0x03)),
        "js" | "mjs" | "cjs" => ('\u{e74e}', rgb(0xcb, 0xcb, 0x41)),
        "jsx" | "tsx" => ('\u{e7ba}', rgb(0x20, 0xc2, 0xe3)),
        "ts" | "mts" | "cts" => ('\u{e628}', rgb(0x51, 0x9a, 0xba)),
        "json" | "jsonc" => ('\u{e60b}', rgb(0xcb, 0xcb, 0x41)),
        "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" => ('\u{e615}', rgb(0x6d, 0x80, 0x86)),
        "md" | "markdown" => ('\u{e73e}', rgb(0xdd, 0xdd, 0xdd)),
        "go" => ('\u{e627}', rgb(0x00, 0xad, 0xd8)),
        "c" | "h" => ('\u{e61e}', rgb(0x59, 0x9e, 0xff)),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => ('\u{e61d}', rgb(0xf3, 0x4b, 0x7d)),
        "sh" | "bash" | "zsh" | "fish" => ('\u{e795}', rgb(0x89, 0xe0, 0x51)),
        "lua" => ('\u{e620}', rgb(0x51, 0xa0, 0xcf)),
        "html" | "htm" => ('\u{e736}', rgb(0xe4, 0x4d, 0x26)),
        "css" | "scss" | "sass" | "less" => ('\u{e749}', rgb(0x42, 0xa5, 0xf5)),
        "sql" | "pgsql" | "ddl" => ('\u{e706}', rgb(0xda, 0xd8, 0xd8)),
        "cs" | "csx" => ('\u{e648}', rgb(0x9b, 0x4f, 0x96)),
        "csproj" | "fsproj" | "vbproj" | "sln" | "slnx" | "props" | "targets" => ('\u{e70c}', rgb(0x85, 0x4c, 0xc7)),
        "xml" | "xaml" | "axaml" | "resx" | "nuspec" | "config" | "xsd" | "xsl" | "xslt" | "plist" => {
            ('\u{e619}', rgb(0xe3, 0x79, 0x33))
        }
        "ipynb" => ('\u{f082e}', rgb(0xf3, 0x76, 0x26)),
        "java" => ('\u{e738}', rgb(0xcc, 0x3e, 0x44)),
        "rb" => ('\u{e739}', rgb(0x70, 0x15, 0x16)),
        "svg" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" => ('\u{f1c5}', rgb(0xa0, 0x74, 0xc4)),
        "lock" => ('\u{f023}', GREY),
        "txt" | "log" => ('\u{f0f6}', GREY),
        _ => ('\u{f016}', GREY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_name_then_extension() {
        assert_eq!(file("main.rs").0, '\u{e7a8}');
        assert_eq!(file("Dockerfile").0, '\u{e7b0}');
        assert_eq!(file("Dockerfile.flyway").0, '\u{e7b0}');
        assert_eq!(file("api.dockerfile").0, '\u{e7b0}');
        assert_eq!(file("Program.cs").0, '\u{e648}');
        assert_eq!(file("App.sln").0, '\u{e70c}');
        assert_eq!(file(".env.local").0, '\u{f084}');
        assert_eq!(file(".env.template").0, '\u{f084}');
        assert_eq!(file("prod.env").0, '\u{f084}');
        assert_eq!(file("Cargo.lock").0, '\u{f023}');
        assert_eq!(file("noext").0, '\u{f016}');
    }
}
