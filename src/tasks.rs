//! `.vscode/tasks.json`, read as VS Code reads it, for a launch
//! configuration's `preLaunchTask` / `postDebugTask` (`taskConfiguration.ts`,
//! `terminalTaskSystem.ts`, `debugTaskRunner.ts`):
//!
//! - a task is found by its `label` (or `"type: name"` for tasks without one);
//! - the OS block (`windows` / `linux` / `osx`) goes over the task: `command`
//!   and `args` replaced, `options.env` merged key by key, `cwd` replaced;
//!   the file's top-level `options` fill what the task leaves out (a task's
//!   own `env` hides the top-level one entirely);
//! - `type: shell` runs the command line in the user's shell (`-c`), each
//!   part quoted only when it has a space outside quotes; `process` runs it
//!   as is;
//! - `options.cwd` defaults to the folder;
//! - `isBackground` tasks are ready when their problem matcher's
//!   `background.endsPattern` shows up (after `beginsPattern`, unless
//!   `activeOnStart`); `$tsc-watch` and `$tsgo-watch` are known by name;
//! - `dependsOn` runs first (`dependsOrder`: `parallel` by default, or
//!   `sequence`).

use crate::launch::{platform_key, substitute, Ctx, Obj, Subst};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn read_tasks(root: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(root.join(".vscode").join("tasks.json")).ok()?;
    serde_json::from_str(&crate::settings::strip_jsonc(&text)).ok()
}

/// A task ready to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub label: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// Background task: (activeOnStart, beginsPattern, endsPattern), when
    /// its matcher says when it's ready.
    pub background: Option<(bool, String, String)>,
    pub is_background: bool,
    pub depends: Vec<String>,
    pub sequence: bool,
    /// Nothing to run itself (only `dependsOn`).
    pub empty: bool,
}

fn label_of(t: &Value) -> Option<String> {
    if let Some(l) = t["label"].as_str() {
        return Some(l.to_string());
    }
    // A task customizing a provider's task: "type: name".
    let ty = t["type"].as_str()?;
    let name = t["script"].as_str().or(t["task"].as_str()).or(t["command"].as_str())?;
    Some(format!("{}: {}", ty, name))
}

/// The label of the default build task (`group: {kind: build, isDefault: true}`).
pub fn default_build_task(doc: &Value) -> Option<String> {
    doc["tasks"].as_array()?.iter().find_map(|t| {
        let g = &t["group"];
        (g["kind"].as_str() == Some("build") && g["isDefault"].as_bool() == Some(true)).then(|| label_of(t))?
    })
}

/// `command` / `args` / `options` of the OS block over `t`'s.
fn with_platform(t: &Value) -> Obj {
    let mut out = t.as_object().cloned().unwrap_or_default();
    if let Some(p) = t.get(platform_key()).and_then(Value::as_object) {
        for (k, v) in p {
            if k == "options" {
                let mut opts = out
                    .get("options")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for (ok, ov) in v.as_object().into_iter().flatten() {
                    if ok == "env" {
                        let mut env = opts.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
                        for (ek, ev) in ov.as_object().into_iter().flatten() {
                            env.insert(ek.clone(), ev.clone());
                        }
                        opts.insert("env".into(), Value::Object(env));
                    } else {
                        opts.insert(ok.clone(), ov.clone());
                    }
                }
                out.insert("options".into(), Value::Object(opts));
            } else {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    for k in ["windows", "osx", "linux"] {
        out.remove(k);
    }
    out
}

/// A quoted part of a shell command line: a string (quoted with `'…'` when
/// it has a space outside quotes) or `{value, quoting}`.
fn shell_part(v: &Value) -> String {
    let (s, quoting) = match v {
        Value::String(s) => (s.clone(), None),
        Value::Object(o) => {
            let val = match o.get("value") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                _ => String::new(),
            };
            (val, o.get("quoting").and_then(Value::as_str).map(str::to_string))
        }
        _ => (String::new(), None),
    };
    match quoting.as_deref() {
        Some("escape") => s
            .chars()
            .map(|c| {
                if matches!(c, ' ' | '"' | '\'') {
                    format!("\\{}", c)
                } else {
                    c.to_string()
                }
            })
            .collect(),
        Some("strong") => format!("'{}'", s),
        Some("weak") => format!("\"{}\"", s),
        _ if space_outside_quotes(&s) => format!("'{}'", s),
        _ => s,
    }
}

fn space_outside_quotes(s: &str) -> bool {
    let mut q: Option<char> = None;
    for c in s.chars() {
        match (q, c) {
            (None, '"' | '\'') => q = Some(c),
            (Some(x), c) if c == x => q = None,
            (None, c) if c.is_whitespace() => return true,
            _ => {}
        }
    }
    false
}

fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// The task called `label`, its variables substituted (`${input:}` asked
/// like a launch configuration's).
pub fn find(doc: &Value, label: &str, ctx: &Ctx) -> Result<Task, Subst> {
    let tasks = doc["tasks"].as_array().cloned().unwrap_or_default();
    let Some(raw) = tasks.iter().find(|t| label_of(t).as_deref() == Some(label)) else {
        return Err(Subst::Error(format!("Could not find the task '{}'.", label)));
    };
    let mut t = with_platform(raw);
    // The file's top-level options (and command) fill what the task leaves out.
    let globals = with_platform(&json!({
        "options": doc["options"].clone(),
        "windows": doc["windows"].clone(),
        "linux": doc["linux"].clone(),
        "osx": doc["osx"].clone(),
    }));
    let gopts = globals
        .get("options")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut opts = t.get("options").and_then(Value::as_object).cloned().unwrap_or_default();
    for k in ["cwd", "env"] {
        if !opts.contains_key(k) {
            if let Some(v) = gopts.get(k) {
                opts.insert(k.into(), v.clone());
            }
        }
    }
    let mut shell = opts
        .get("shell")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (k, v) in gopts.get("shell").and_then(Value::as_object).into_iter().flatten() {
        shell.entry(k.clone()).or_insert(v.clone());
    }
    if !t.contains_key("command") && !t.contains_key("dependsOn") {
        for k in ["command", "args"] {
            if let Some(v) = doc.get(k) {
                t.insert(k.into(), v.clone());
            }
        }
    }
    // Variables: in command, args, cwd, env values and the shell.
    let subject = json!({
        "command": t.get("command").cloned().unwrap_or(Value::Null),
        "args": t.get("args").cloned().unwrap_or(json!([])),
        "cwd": opts.get("cwd").cloned().unwrap_or(Value::Null),
        "env": opts.get("env").cloned().unwrap_or(json!({})),
        "shell": Value::Object(shell),
    });
    let r = substitute(&subject, ctx, &doc["inputs"])?;
    let cwd = match r["cwd"].as_str() {
        Some(c) if Path::new(c).is_absolute() => PathBuf::from(c),
        Some(c) => ctx.root.join(c),
        None => ctx.root.clone(),
    };
    let env: Vec<(String, String)> = r["env"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            (
                k.clone(),
                v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()),
            )
        })
        .collect();
    let ty = t.get("type").and_then(Value::as_str).unwrap_or("process");
    let args: Vec<Value> = r["args"].as_array().cloned().unwrap_or_default();
    let empty = r["command"].is_null() || plain(&r["command"]).is_empty();
    let argv = if empty {
        Vec::new()
    } else if ty == "shell" {
        let line = if args.is_empty() && r["command"].is_string() {
            plain(&r["command"])
        } else {
            std::iter::once(shell_part(&r["command"]))
                .chain(args.iter().map(shell_part))
                .collect::<Vec<_>>()
                .join(" ")
        };
        match r["shell"]["executable"].as_str() {
            Some(exe) => {
                let mut v = vec![exe.to_string()];
                v.extend(
                    r["shell"]["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|a| a.as_str().map(str::to_string)),
                );
                v.push(line);
                v
            }
            None => {
                let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                vec![sh, "-c".into(), line]
            }
        }
    } else {
        std::iter::once(plain(&r["command"]))
            .chain(args.iter().map(plain))
            .collect()
    };
    let depends: Vec<String> = match t.get("dependsOn") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let is_background = t.get("isBackground").and_then(Value::as_bool).unwrap_or(false);
    Ok(Task {
        label: label.to_string(),
        argv,
        cwd,
        env,
        background: background_matcher(t.get("problemMatcher").unwrap_or(&Value::Null)),
        is_background,
        depends,
        sequence: t.get("dependsOrder").and_then(Value::as_str) == Some("sequence"),
        empty,
    })
}

/// A problem matcher's background patterns: (activeOnStart, begins, ends).
fn background_matcher(m: &Value) -> Option<(bool, String, String)> {
    match m {
        Value::Array(a) => a.iter().find_map(background_matcher),
        Value::String(name) => match name.as_str() {
            "$tsc-watch" => Some((
                true,
                r"^\s*(?:message TS6032:|\[?\D*.{1,2}[:.].{1,2}[:.].{1,2}\D*(├\D*\d{1,2}\D+┤)?(?:\]| -)) (Starting compilation in watch mode|File change detected\. Starting incremental compilation)\.\.\.".into(),
                r"^\s*(?:message TS6042:|\[?\D*.{1,2}[:.].{1,2}[:.].{1,2}\D*(├\D*\d{1,2}\D+┤)?(?:\]| -)) (?:Compilation complete\.|Found \d+ errors?\.) Watching for file changes\.".into(),
            )),
            "$tsgo-watch" => Some((true, r"^build starting at .*$".into(), r"^build finished in .*$".into())),
            _ => None,
        },
        Value::Object(o) => {
            let b = o.get("background")?;
            let pat = |k: &str| match &b[k] {
                Value::String(s) => Some(s.clone()),
                v => v["regexp"].as_str().map(str::to_string),
            };
            Some((b["activeOnStart"].as_bool().unwrap_or(false), pat("beginsPattern")?, pat("endsPattern")?))
        }
        _ => None,
    }
}

/// Where a background task is, from its output so far: (active, ready).
pub fn background_state(bg: &(bool, String, String), output: &str, mut active: bool) -> (bool, bool) {
    let (Ok(begins), Ok(ends)) = (regex::Regex::new(&bg.1), regex::Regex::new(&bg.2)) else {
        return (active, false);
    };
    for line in output.lines() {
        if begins.is_match(line) {
            active = true;
        } else if active && ends.is_match(line) {
            return (false, true);
        }
    }
    (active, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx {
            root: PathBuf::from("/w"),
            ..Default::default()
        }
    }

    #[test]
    fn shell_and_process_tasks() {
        let doc = json!({
            "version": "2.0.0",
            "options": { "env": { "G": "global" }, "cwd": "${workspaceFolder}/sub" },
            "tasks": [
                { "label": "build", "type": "shell", "command": "cargo build", "group": { "kind": "build", "isDefault": true } },
                { "label": "say", "type": "shell", "command": "echo", "args": ["a b", "c", { "value": "d e", "quoting": "escape" }],
                  "options": { "env": { "X": "1" } }, "linux": { "options": { "env": { "Y": "2" } } }, "osx": { "options": { "env": { "Y": "2" } } } },
                { "label": "proc", "command": "/bin/run", "args": ["a b"] },
                { "label": "all", "dependsOn": ["build", "say"], "dependsOrder": "sequence" },
                { "type": "npm", "script": "dev" }
            ]
        });
        assert_eq!(default_build_task(&doc).as_deref(), Some("build"));
        let t = find(&doc, "build", &ctx()).unwrap();
        assert_eq!(t.argv[1..], ["-c".to_string(), "cargo build".to_string()]);
        assert_eq!(t.cwd, PathBuf::from("/w/sub"));
        assert_eq!(t.env, vec![("G".to_string(), "global".to_string())]);
        let t = find(&doc, "say", &ctx()).unwrap();
        assert_eq!(t.argv[2], r"echo 'a b' c d\ e");
        // Its own env hides the top-level one; the OS block merges into it.
        if cfg!(unix) {
            assert_eq!(
                t.env,
                vec![("X".to_string(), "1".to_string()), ("Y".to_string(), "2".to_string())]
            );
        }
        let t = find(&doc, "proc", &ctx()).unwrap();
        assert_eq!(t.argv, vec!["/bin/run", "a b"]);
        let t = find(&doc, "all", &ctx()).unwrap();
        assert!(t.empty && t.sequence);
        assert_eq!(t.depends, vec!["build", "say"]);
        assert!(find(&doc, "npm: dev", &ctx()).is_ok());
        assert!(matches!(find(&doc, "nope", &ctx()), Err(Subst::Error(_))));
    }

    #[test]
    fn background_patterns() {
        let doc = json!({ "tasks": [{
            "label": "watch", "type": "shell", "command": "x", "isBackground": true,
            "problemMatcher": { "pattern": { "regexp": "." }, "background": { "beginsPattern": "^start", "endsPattern": "^ready on (\\d+)" } }
        }]});
        let t = find(&doc, "watch", &ctx()).unwrap();
        let bg = t.background.clone().unwrap();
        assert_eq!(background_state(&bg, "noise\nready on 1\n", false), (false, false));
        assert_eq!(background_state(&bg, "start\nbuilding\n", false), (true, false));
        assert_eq!(background_state(&bg, "start\nready on 8000\n", false), (false, true));
        assert!(background_matcher(&json!("$tsc-watch")).unwrap().0);
    }
}
