//! `~/.ren/settings.json` (global) and `<project>/.ren/settings.json` (per
//! project). The project file merges over the global one, VS Code style:
//! objects merge key by key, arrays and plain values replace.

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DEFAULT_PRIMARY: u32 = 0xDA56D1;

pub const DEFAULT_FILE: &str = r##"{
  "theme": {
    "primary": "#DA56D1"
  },
  "formatOnSave": true,
  "icons": true,
  "scan": true,
  "inlayHints": {
    "enabled": true,
    "types": true,
    "parameters": true,
    "returnTypes": true,
    "chaining": true,
    "closingBraces": true
  },
  "lsp": {
    "servers": {},
    "format": {}
  },
  "containers": {
    "socket": ""
  },
  "git": {
    "templates": {}
  }
}
"##;

pub const PROJECT_FILE: &str = r##"{
  // Project settings: merged over ~/.ren/settings.json.
}
"##;

/// One field of a commit template.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub hint: String,
    pub prefix: String,
    pub suffix: String,
    /// (value that goes into the message, label shown in the list)
    pub choices: Vec<(String, String)>,
    /// With `choices`: also allow typing a value that isn't in the list.
    pub free: bool,
    /// Height of a multi-line box; 0 = one line.
    pub lines: usize,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Template {
    pub format: Vec<String>,
    pub fields: Vec<Field>,
}

/// Which inlay hints to draw. `"inlayHints": false` (or `true`) is short for `enabled`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlayHints {
    pub enabled: bool,
    /// `let x: i32`, Go `range` variables, Python `x: int`.
    pub types: bool,
    /// Parameter names at call sites: `add(a: 1)`.
    pub parameters: bool,
    /// `|x| -> i32 { … }`, TypeScript `f(): number`, Python `-> int`.
    pub return_types: bool,
    /// The type at the end of each line of a method chain (Rust).
    pub chaining: bool,
    /// What a long block closes: `} fn main`.
    pub closing_braces: bool,
}

impl Default for InlayHints {
    fn default() -> Self {
        InlayHints {
            enabled: true,
            types: true,
            parameters: true,
            return_types: true,
            chaining: true,
            closing_braces: true,
        }
    }
}

/// Settings keys of the inlay hint kinds, in menu order.
pub const HINT_KEYS: [&str; 5] = ["types", "parameters", "returnTypes", "chaining", "closingBraces"];

impl InlayHints {
    pub fn get(&self, key: &str) -> Option<bool> {
        Some(match key {
            "enabled" => self.enabled,
            "types" => self.types,
            "parameters" => self.parameters,
            "returnTypes" => self.return_types,
            "chaining" => self.chaining,
            "closingBraces" => self.closing_braces,
            _ => return None,
        })
    }

    pub fn set(&mut self, key: &str, v: bool) -> bool {
        let f = match key {
            "enabled" => &mut self.enabled,
            "types" => &mut self.types,
            "parameters" => &mut self.parameters,
            "returnTypes" => &mut self.return_types,
            "chaining" => &mut self.chaining,
            "closingBraces" => &mut self.closing_braces,
            _ => return false,
        };
        *f = v;
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub primary: u32,
    pub format_on_save: bool,
    pub icons: bool,
    /// Check the whole project when it opens and after saves (scan.rs).
    pub scan: bool,
    /// Types and parameter names from the language server, drawn in the code.
    pub inlay: InlayHints,
    /// Language (lowercase, e.g. "python") → server command lines.
    pub lsp_servers: BTreeMap<String, Vec<Vec<String>>>,
    /// Language → name of the server that formats (e.g. "ruff").
    pub lsp_format: BTreeMap<String, String>,
    /// Docker-compatible socket, e.g. "unix:///var/run/docker.sock". Empty = docker's default.
    pub containers_socket: String,
    pub templates: Vec<(String, Template)>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            primary: DEFAULT_PRIMARY,
            format_on_save: true,
            icons: true,
            scan: true,
            inlay: InlayHints::default(),
            lsp_servers: BTreeMap::new(),
            lsp_format: BTreeMap::new(),
            containers_socket: String::new(),
            templates: Vec::new(),
        }
    }
}

pub fn home() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        env::var_os("USERPROFILE")
    } else {
        env::var_os("HOME")
    }?;
    Some(PathBuf::from(home))
}

pub fn dir() -> Option<PathBuf> {
    Some(home()?.join(".ren"))
}

pub fn path() -> Option<PathBuf> {
    Some(dir()?.join("settings.json"))
}

pub fn project_path(root: &Path) -> PathBuf {
    root.join(".ren").join("settings.json")
}

pub fn ensure() -> io::Result<Option<PathBuf>> {
    let p = path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    if p.exists() {
        return Ok(None);
    }
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(&p, DEFAULT_FILE)?;
    Ok(Some(p))
}

pub fn ensure_project(root: &Path) -> io::Result<PathBuf> {
    let p = project_path(root);
    if !p.exists() {
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&p, PROJECT_FILE)?;
    }
    Ok(p)
}

/// Removes `//` and `/* */` comments and trailing commas, so settings files may
/// be written like VS Code's JSONC.
pub fn strip_jsonc(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    while i < c.len() {
        if in_str {
            out.push(c[i]);
            if c[i] == '\\' && i + 1 < c.len() {
                out.push(c[i + 1]);
                i += 2;
                continue;
            }
            if c[i] == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c[i] == '"' {
            in_str = true;
            out.push('"');
            i += 1;
        } else if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    let re = regex::Regex::new(r",(\s*[}\]])").unwrap();
    re.replace_all(&out, "$1").into_owned()
}

/// Deep merge: objects merge key by key, everything else in `over` replaces.
pub fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(bv) if bv.is_object() && v.is_object() => merge(bv, v),
                    _ => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

fn read_json(p: &Path, errs: &mut Vec<String>) -> Option<Value> {
    let text = fs::read_to_string(p).ok()?;
    match serde_json::from_str(&strip_jsonc(&text)) {
        Ok(v) => Some(v),
        Err(e) => {
            errs.push(format!("{}: {}", p.display(), e));
            None
        }
    }
}

/// Global settings with the project's `.ren/settings.json` merged on top.
pub fn load(root: Option<&Path>) -> (Settings, Vec<String>) {
    let mut errs = Vec::new();
    let mut v = path()
        .and_then(|p| read_json(&p, &mut errs))
        .unwrap_or(Value::Object(Map::new()));
    if let Some(r) = root {
        if let Some(pv) = read_json(&project_path(r), &mut errs) {
            merge(&mut v, pv);
        }
    }
    let (s, more) = from_value(&v);
    errs.extend(more);
    (s, errs)
}

#[cfg(test)]
pub fn parse(text: &str) -> (Settings, Vec<String>) {
    match serde_json::from_str::<Value>(&strip_jsonc(text)) {
        Ok(v) => from_value(&v),
        Err(e) => (Settings::default(), vec![format!("settings.json: {}", e)]),
    }
}

fn s_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn template_from(name: &str, v: &Value, errs: &mut Vec<String>) -> Option<Template> {
    let format: Vec<String> = match &v["format"] {
        Value::String(s) => s.split('\n').map(String::from).collect(),
        Value::Array(a) => a.iter().map(s_of).collect(),
        _ => {
            errs.push(format!("git.templates.{}.format must be a list of lines", name));
            return None;
        }
    };
    let mut fields = Vec::new();
    for f in v["fields"].as_array().into_iter().flatten() {
        let key = s_of(&f["key"]);
        if key.is_empty() {
            errs.push(format!("git.templates.{}: every field needs a \"key\"", name));
            continue;
        }
        let choices = f["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| match c {
                Value::String(s) => (s.clone(), s.clone()),
                o => {
                    let val = s_of(&o["value"]);
                    let lab = o["label"].as_str().map(String::from).unwrap_or_else(|| val.clone());
                    (val, lab)
                }
            })
            .collect();
        fields.push(Field {
            label: f["label"].as_str().map(String::from).unwrap_or_else(|| key.clone()),
            key,
            hint: s_of(&f["hint"]),
            prefix: s_of(&f["prefix"]),
            suffix: s_of(&f["suffix"]),
            choices,
            free: f["free"].as_bool().unwrap_or(false),
            lines: f["lines"].as_u64().unwrap_or(0) as usize,
            required: f["required"].as_bool().unwrap_or(false),
        });
    }
    Some(Template { format, fields })
}

pub fn from_value(v: &Value) -> (Settings, Vec<String>) {
    let mut s = Settings::default();
    let mut errs = Vec::new();
    if !v.is_object() {
        errs.push("settings.json must be an object".into());
        return (s, errs);
    }
    match &v["theme"]["primary"] {
        Value::Null => {}
        Value::String(c) => match parse_color(c) {
            Some(rgb) => s.primary = rgb,
            None => errs.push(format!("theme.primary: \"{}\" is not a #RRGGBB color", c)),
        },
        _ => errs.push("theme.primary must be a string like \"#DA56D1\"".into()),
    }
    match &v["formatOnSave"] {
        Value::Null => {}
        Value::Bool(b) => s.format_on_save = *b,
        _ => errs.push("formatOnSave must be true or false".into()),
    }
    match &v["icons"] {
        Value::Null => {}
        Value::Bool(b) => s.icons = *b,
        _ => errs.push("icons must be true or false".into()),
    }
    match &v["scan"] {
        Value::Null => {}
        Value::Bool(b) => s.scan = *b,
        _ => errs.push("scan must be true or false".into()),
    }
    match &v["inlayHints"] {
        Value::Null => {}
        Value::Bool(b) => s.inlay.enabled = *b,
        Value::Object(m) => {
            for (k, val) in m {
                match val.as_bool() {
                    Some(b) if s.inlay.set(k, b) => {}
                    Some(_) => errs.push(format!("inlayHints.{}: unknown (enabled, {})", k, HINT_KEYS.join(", "))),
                    None => errs.push(format!("inlayHints.{} must be true or false", k)),
                }
            }
        }
        _ => errs.push("inlayHints must be true, false or an object".into()),
    }
    if let Some(m) = v["lsp"]["servers"].as_object() {
        for (lang, list) in m {
            let cmds: Vec<Vec<String>> = match list {
                Value::String(c) => vec![c.split_whitespace().map(String::from).collect()],
                Value::Array(a) => a
                    .iter()
                    .filter_map(|c| c.as_str())
                    .map(|c| c.split_whitespace().map(String::from).collect::<Vec<_>>())
                    .filter(|c| !c.is_empty())
                    .collect(),
                _ => {
                    errs.push(format!("lsp.servers.{} must be a list of commands", lang));
                    continue;
                }
            };
            s.lsp_servers.insert(lang.to_lowercase(), cmds);
        }
    }
    if let Some(m) = v["lsp"]["format"].as_object() {
        for (lang, name) in m {
            if let Some(n) = name.as_str() {
                s.lsp_format.insert(lang.to_lowercase(), n.to_string());
            }
        }
    }
    s.containers_socket = s_of(&v["containers"]["socket"]);
    if let Some(m) = v["git"]["templates"].as_object() {
        for (name, t) in m {
            if let Some(t) = template_from(name, t, &mut errs) {
                s.templates.push((name.clone(), t));
            }
        }
        if let Some(i) = s.templates.iter().position(|t| t.0 == "default") {
            let d = s.templates.remove(i);
            s.templates.insert(0, d);
        }
    }
    (s, errs)
}

pub fn parse_color(c: &str) -> Option<u32> {
    let h = c.trim().trim_start_matches('#');
    let full = match h.len() {
        6 => h.to_string(),
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        _ => return None,
    };
    u32::from_str_radix(&full, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_file_matches_defaults() {
        let (s, errs) = parse(DEFAULT_FILE);
        assert!(errs.is_empty(), "{:?}", errs);
        assert_eq!(s, Settings::default());
        let (_, errs) = parse(PROJECT_FILE);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn inlay_hints() {
        let (s, errs) = parse(r#"{ "inlayHints": false }"#);
        assert!(errs.is_empty());
        assert!(!s.inlay.enabled && s.inlay.types);
        let (s, errs) = parse(r#"{ "inlayHints": { "types": false, "closingBraces": false } }"#);
        assert!(errs.is_empty());
        assert!(s.inlay.enabled && !s.inlay.types && !s.inlay.closing_braces && s.inlay.parameters);
        let (s, errs) = parse(r#"{ "inlayHints": { "typo": true, "chaining": 1 } }"#);
        assert_eq!(errs.len(), 2);
        assert_eq!(s.inlay, InlayHints::default());
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#DA56D1"), Some(0xDA56D1));
        assert_eq!(parse_color("da56d1"), Some(0xDA56D1));
        assert_eq!(parse_color("#fff"), Some(0xFFFFFF));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#GGGGGG"), None);
    }

    #[test]
    fn partial_and_invalid() {
        let (s, errs) = parse(r#"{ "formatOnSave": false }"#);
        assert!(errs.is_empty());
        assert_eq!(s.primary, DEFAULT_PRIMARY);
        assert!(!s.format_on_save);

        let (s, errs) = parse(r#"{ "theme": { "primary": "pink" }, "formatOnSave": "yes" }"#);
        assert_eq!(errs.len(), 2);
        assert_eq!(s, Settings::default());

        let (s, errs) = parse("{ not json");
        assert_eq!(errs.len(), 1);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn project_merges_over_global() {
        let mut g = json!({
            "formatOnSave": true,
            "lsp": { "servers": { "python": ["pyright-langserver --stdio"] } },
            "git": { "templates": { "default": { "format": ["{summary}"], "fields": [] } } }
        });
        merge(
            &mut g,
            json!({
                "formatOnSave": false,
                "lsp": { "servers": { "python": ["basedpyright-langserver --stdio", "ruff server"] } },
                "git": { "templates": { "ticket": { "format": "[{t}] {s}", "fields": [{ "key": "t" }, { "key": "s" }] } } }
            }),
        );
        let (s, errs) = from_value(&g);
        assert!(errs.is_empty(), "{:?}", errs);
        assert!(!s.format_on_save);
        assert_eq!(s.lsp_servers["python"].len(), 2);
        assert_eq!(s.lsp_servers["python"][1], vec!["ruff", "server"]);
        let names: Vec<&str> = s.templates.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(names, vec!["default", "ticket"]);
        assert_eq!(s.templates[1].1.fields[0].label, "t");
    }

    #[test]
    fn template_fields() {
        let (s, errs) = parse(
            r#"{ "git": { "templates": { "default": {
                "format": ["{ticket} {type} {summary}", "", "{body}"],
                "fields": [
                  { "key": "ticket", "label": "Ticket", "prefix": "[", "suffix": "]" },
                  { "key": "type", "choices": [{ "value": "🩹", "label": "🩹 Bug fix" }, "x"] },
                  { "key": "summary", "required": true },
                  { "key": "body", "lines": 5 }
                ] } } } }"#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
        let t = &s.templates[0].1;
        assert_eq!(t.format.len(), 3);
        assert_eq!(
            t.fields[1].choices,
            vec![("🩹".into(), "🩹 Bug fix".into()), ("x".into(), "x".into())]
        );
        assert!(t.fields[2].required);
        assert_eq!(t.fields[3].lines, 5);
    }

    #[test]
    fn jsonc() {
        let v: Value =
            serde_json::from_str(&strip_jsonc("{\n // c\n \"a\": \"x//y\", /* b */ \"b\": [1,2,],\n}")).unwrap();
        assert_eq!(v["a"], "x//y");
        assert_eq!(v["b"].as_array().unwrap().len(), 2);
    }
}
