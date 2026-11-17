//! What VS Code's extensions do to a launch configuration before it reaches
//! the debug adapter, so a `.vscode/launch.json` that works in VS Code works
//! the same in ren. Each part follows the extension's source:
//!
//! - VS Code itself: platform blocks (`debugUtils.ts`
//!   `getEffectiveConfigForPlatform`), the order of the configuration list
//!   (`getVisibleAndSorted`), variables and `inputs` (`variableResolver.ts`,
//!   `configurationResolverExpression.ts`).
//! - Python: the Python Debugger extension (`vscode-python-debugger`,
//!   `configuration/resolvers/launch.ts`, `attach.ts`, `base.ts`,
//!   `helper.ts`, `common/variables/environment.ts`).
//! - Go: vscode-go (`goDebugConfiguration.ts`, `goDebugFactory.ts`).
//! - C/C++ `cppdbg` (ms-vscode.cpptools) turned into what gdb's and
//!   lldb-dap's own DAP take.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type Obj = Map<String, Value>;

/// Debugger types ren runs.
pub const TYPES: &[&str] = &[
    "lldb",
    "codelldb",
    "lldb-dap",
    "lldb-vscode",
    "cppdbg",
    "gdb",
    "debugpy",
    "python",
    "go",
];

/// `.vscode/launch.json`, comments and trailing commas allowed. None when
/// there's none; an error when it can't be read as JSON.
pub fn read_launch(root: &Path) -> Option<Result<Value, String>> {
    let text = std::fs::read_to_string(root.join(".vscode").join("launch.json")).ok()?;
    Some(serde_json::from_str(&crate::settings::strip_jsonc(&text)).map_err(|e| format!(".vscode/launch.json: {}", e)))
}

/// The OS key of platform blocks: `osx`, `linux` (WSL too), `windows`.
pub fn platform_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// A configuration with this OS's block over it: its keys replace the
/// configuration's (a platform `env` replaces the whole `env`), except
/// `presentation`, merged key by key. The blocks themselves go.
pub fn effective(cfg: &Value) -> Value {
    let Some(o) = cfg.as_object() else { return cfg.clone() };
    let mut out = o.clone();
    if let Some(p) = o.get(platform_key()).and_then(Value::as_object) {
        for (k, v) in p {
            if k == "presentation" {
                let mut merged = out.get(k).and_then(Value::as_object).cloned().unwrap_or_default();
                for (pk, pv) in v.as_object().into_iter().flatten() {
                    merged.insert(pk.clone(), pv.clone());
                }
                out.insert(k.clone(), Value::Object(merged));
            } else {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    for k in ["windows", "osx", "linux"] {
        out.remove(k);
    }
    Value::Object(out)
}

/// The configurations F5 offers, as VS Code's list orders them: hidden
/// ones out; those with a `presentation.group` first (by group, then
/// `order`), then those with a `presentation` (by `order`), then the rest,
/// each in file order. Only `launch` / `attach` ones of a type ren runs.
pub fn config_names(doc: &Value) -> Vec<String> {
    let mut cfgs: Vec<(usize, Value)> = doc["configurations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(effective)
        .enumerate()
        .filter(|(_, c)| matches!(c["request"].as_str(), Some("launch" | "attach")))
        .filter(|(_, c)| TYPES.contains(&c["type"].as_str().unwrap_or("")))
        .filter(|(_, c)| !c["presentation"]["hidden"].as_bool().unwrap_or(false))
        .collect();
    let rank = |c: &Value| -> (u8, String, f64) {
        let p = &c["presentation"];
        let order = p["order"].as_f64().unwrap_or(f64::MAX);
        match p["group"].as_str() {
            Some(g) => (0, g.to_string(), order),
            None if p.is_object() => (1, String::new(), order),
            None => (2, String::new(), f64::MAX),
        }
    };
    cfgs.sort_by(|(ia, a), (ib, b)| {
        let (ra, rb) = (rank(a), rank(b));
        ra.0.cmp(&rb.0)
            .then_with(|| ra.1.cmp(&rb.1))
            .then_with(|| ra.2.partial_cmp(&rb.2).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| ia.cmp(ib))
    });
    cfgs.into_iter()
        .map(|(_, c)| c["name"].as_str().unwrap_or("(unnamed)").to_string())
        .collect()
}

/// The configuration called `name` (or the first ren runs), its OS block
/// applied.
pub fn config(doc: &Value, name: Option<&str>) -> Option<Value> {
    let names = config_names(doc);
    let want = match name {
        Some(n) => n.to_string(),
        None => names.first()?.clone(),
    };
    doc["configurations"]
        .as_array()?
        .iter()
        .map(effective)
        .find(|c| c["name"].as_str() == Some(want.as_str()) && TYPES.contains(&c["type"].as_str().unwrap_or("")))
}

/// What variables refer to.
#[derive(Default, Clone)]
pub struct Ctx {
    /// The launch's folder (the project on screen).
    pub root: PathBuf,
    /// Open projects (VS Code's workspace folders), for `${workspaceFolder:Name}`.
    pub folders: Vec<PathBuf>,
    pub file: Option<PathBuf>,
    /// 1-based.
    pub line: usize,
    pub column: usize,
    pub selected: String,
    /// `${input:id}` and `${command:id}` answered so far: `input:id` /
    /// `command:id` → value.
    pub answers: HashMap<String, String>,
    /// The Python ren runs (for `${command:python.interpreterPath}`).
    pub python: Option<String>,
    /// `${defaultBuildTask}`: tasks.json's default build task.
    pub default_build_task: Option<String>,
    /// `${config:…}`: `.vscode/settings.json` (only read for this).
    pub settings: Value,
}

/// A value to ask for before the launch can go on.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// `promptString`: (id, description, default, password).
    Prompt(String, String, String, bool),
    /// `pickString`: (id, description, options (label, value), default).
    Pick(String, String, Vec<(String, String)>, Option<String>),
    /// `${command:pickProcess}` and the like: (key, only the user's own).
    Process(String, bool),
}

#[derive(Debug, PartialEq)]
pub enum Subst {
    Ask(Ask),
    Error(String),
}

const PICK_PROCESS: &[&str] = &[
    "pickProcess",
    "pickMyProcess",
    "pickGoProcess",
    "pickRemoteProcess",
    "python.pickLocalProcess",
    "go.debug.pickProcess",
    "go.debug.pickGoProcess",
    "lldb.pickProcess",
    "lldb.pickMyProcess",
];

/// Every `${…}` in `v` (strings, and object keys) resolved as VS Code does.
/// Interactive ones (`${input:}`, `${command:pick…}`) come from
/// `ctx.answers`; the first one missing is returned as `Subst::Ask`.
/// Unknown variables stay as written; ones that can't be resolved (no open
/// file for `${file}`, no such folder) are errors that stop the launch.
pub fn substitute(v: &Value, ctx: &Ctx, inputs: &Value) -> Result<Value, Subst> {
    // The plain variables first (their errors come before any question),
    // then the interactive ones.
    let plain = walk(v, &mut |s| resolve_str(s, ctx, inputs, false))?;
    walk(&plain, &mut |s| resolve_str(s, ctx, inputs, true))
}

fn walk(v: &Value, f: &mut dyn FnMut(&str) -> Result<String, Subst>) -> Result<Value, Subst> {
    Ok(match v {
        Value::String(s) => Value::String(f(s)?),
        Value::Array(a) => Value::Array(a.iter().map(|x| walk(x, f)).collect::<Result<_, _>>()?),
        Value::Object(o) => {
            let mut out = Obj::new();
            for (k, x) in o {
                out.insert(f(k)?, walk(x, f)?);
            }
            Value::Object(out)
        }
        other => other.clone(),
    })
}

fn resolve_str(s: &str, ctx: &Ctx, inputs: &Value, interactive: bool) -> Result<String, Subst> {
    let re = regex::Regex::new(r"\$\{([^{}]+)\}").unwrap();
    let mut out = s.to_string();
    // Inserted text is looked at again (recursive), a few times at most.
    for _ in 0..8 {
        let mut changed = false;
        let mut err = None;
        let next = re
            .replace_all(&out, |c: &regex::Captures| {
                if err.is_some() {
                    return c[0].to_string();
                }
                match variable(&c[1], ctx, inputs, interactive) {
                    Ok(Some(v)) => {
                        changed |= v != c[0];
                        v
                    }
                    Ok(None) => c[0].to_string(),
                    Err(e) => {
                        err = Some(e);
                        c[0].to_string()
                    }
                }
            })
            .into_owned();
        if let Some(e) = err {
            return Err(e);
        }
        out = next;
        if !changed {
            break;
        }
    }
    Ok(out)
}

fn base_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One variable (without `${}`); None leaves it as written.
fn variable(name: &str, ctx: &Ctx, inputs: &Value, interactive: bool) -> Result<Option<String>, Subst> {
    let err = |m: String| Err(Subst::Error(m));
    let (head, arg) = match name.split_once(':') {
        Some((h, a)) => (h, Some(a)),
        None => (name, None),
    };
    if matches!(head, "input" | "command") {
        if !interactive {
            return Ok(None);
        }
        let arg = arg.unwrap_or("");
        let key = format!("{}:{}", head, arg);
        if let Some(v) = ctx.answers.get(&key) {
            return Ok(Some(v.clone()));
        }
        if head == "command" {
            if arg == "python.interpreterPath" {
                return Ok(ctx.python.clone().or(Some("python3".into())));
            }
            if PICK_PROCESS.contains(&arg) {
                let mine = arg.contains("MyProcess");
                return Err(Subst::Ask(Ask::Process(key, mine)));
            }
            return err(format!("${{command:{}}}: ren can't run that VS Code command", arg));
        }
        let Some(list) = inputs.as_array() else {
            return err(format!("${{input:{}}}: there is no \"inputs\" section", arg));
        };
        let Some(i) = list.iter().find(|i| i["id"].as_str() == Some(arg)) else {
            return err(format!("Undefined input variable '{}'", arg));
        };
        let desc = i["description"].as_str().unwrap_or(arg).to_string();
        return match i["type"].as_str() {
            Some("promptString") => Err(Subst::Ask(Ask::Prompt(
                key,
                desc,
                i["default"].as_str().unwrap_or("").to_string(),
                i["password"].as_bool().unwrap_or(false),
            ))),
            Some("pickString") => {
                let opts: Vec<(String, String)> = i["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|o| match o {
                        Value::String(v) => Some((v.clone(), v.clone())),
                        Value::Object(_) => {
                            let v = o["value"].as_str()?.to_string();
                            let l = match o["label"].as_str() {
                                Some(l) => format!("{}: {}", l, v),
                                None => v.clone(),
                            };
                            Some((l, v))
                        }
                        _ => None,
                    })
                    .collect();
                if opts.is_empty() {
                    return err(format!(
                        "Input variable '{}' is of type 'pickString' and must include 'options'.",
                        arg
                    ));
                }
                Err(Subst::Ask(Ask::Pick(
                    key,
                    desc,
                    opts,
                    i["default"].as_str().map(str::to_string),
                )))
            }
            Some("command") => err(format!(
                "${{input:{}}} runs the VS Code command {}, which ren can't",
                arg,
                i["command"].as_str().unwrap_or("?")
            )),
            other => err(format!(
                "Input variable '{}' has an unknown type {:?}",
                arg,
                other.unwrap_or("")
            )),
        };
    }
    if interactive {
        return Ok(None);
    }
    let folder = |arg: Option<&str>| -> Result<PathBuf, Subst> {
        match arg {
            None => Ok(ctx.root.clone()),
            Some(n) => ctx
                .folders
                .iter()
                .rev()
                .find(|f| base_name(f) == n)
                .cloned()
                .ok_or_else(|| Subst::Error(format!("No such folder '{}' (open it as a project)", n))),
        }
    };
    let file = || -> Result<PathBuf, Subst> {
        ctx.file
            .clone()
            .ok_or_else(|| Subst::Error(format!("Variable {} can not be resolved. Please open an editor.", name)))
    };
    let rel = |p: &Path, base: &Path| -> String {
        p.strip_prefix(base)
            .map(|r| r.display().to_string())
            .unwrap_or_else(|_| p.display().to_string())
    };
    let sep = std::path::MAIN_SEPARATOR.to_string();
    Ok(Some(match head {
        "workspaceFolder" | "workspaceRoot" | "cwd" => folder(arg)?.display().to_string(),
        "workspaceFolderBasename" | "workspaceRootFolderName" => base_name(&folder(arg)?),
        "file" => file()?.display().to_string(),
        "fileWorkspaceFolder" | "fileWorkspaceFolderBasename" => {
            let f = file()?;
            let w = ctx
                .folders
                .iter()
                .filter(|d| f.starts_with(d))
                .max_by_key(|d| d.as_os_str().len())
                .cloned()
                .ok_or_else(|| {
                    Subst::Error(format!(
                        "Variable {} can not be resolved: the file isn't in an open project",
                        name
                    ))
                })?;
            if head == "fileWorkspaceFolder" {
                w.display().to_string()
            } else {
                base_name(&w)
            }
        }
        "relativeFile" => rel(&file()?, &folder(arg)?),
        "relativeFileDirname" => {
            let f = file()?;
            let d = f.parent().unwrap_or(Path::new(""));
            let r = rel(d, &folder(arg)?);
            if r.is_empty() {
                ".".into()
            } else {
                r
            }
        }
        "fileBasename" => base_name(&file()?),
        "fileBasenameNoExtension" => file()?
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        "fileExtname" => file()?
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default(),
        "fileDirname" => file()?.parent().map(|d| d.display().to_string()).unwrap_or_default(),
        "fileDirnameBasename" => file()?.parent().map(base_name).unwrap_or_default(),
        "lineNumber" => {
            file()?;
            ctx.line.to_string()
        }
        "columnNumber" => {
            file()?;
            ctx.column.to_string()
        }
        "selectedText" => {
            if ctx.selected.is_empty() {
                return Err(Subst::Error(
                    "Variable selectedText can not be resolved: nothing is selected".into(),
                ));
            }
            ctx.selected.clone()
        }
        "execPath" => std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        "pathSeparator" | "/" => sep,
        "userHome" => crate::settings::home()
            .map(|h| h.display().to_string())
            .ok_or_else(|| Subst::Error("Variable userHome can not be resolved".into()))?,
        "env" => match arg {
            Some(n) => std::env::var(n).unwrap_or_default(),
            None => return Err(Subst::Error("Variable env needs a name: ${env:NAME}".into())),
        },
        "config" => {
            let n = arg.unwrap_or("");
            let v = ctx
                .settings
                .get(n)
                .cloned()
                .or_else(|| n.split('.').try_fold(&ctx.settings, |at, k| at.get(k)).cloned());
            match v {
                Some(Value::String(s)) => s,
                Some(Value::Number(x)) => x.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Array(_) | Value::Object(_)) => {
                    return Err(Subst::Error(format!("Variable config:{} is a structured value", n)))
                }
                _ => {
                    return Err(Subst::Error(format!(
                        "Variable config:{} can not be resolved: the setting isn't in .vscode/settings.json",
                        n
                    )))
                }
            }
        }
        "defaultBuildTask" => match &ctx.default_build_task {
            Some(t) => t.clone(),
            None => return Ok(None),
        },
        _ => return Ok(None),
    }))
}

/// The variables of an env file, read as the Python extension reads one
/// (`parseEnvFile`): `NAME=value` lines (`NAME` is `_*[A-Za-z]\w*`, so
/// `export X=1` and `# comments` don't count), quotes (single or double)
/// taken off with `\n` turned into a newline, quoted values over several
/// lines, and `${NAME}` from earlier lines or `base` (the configuration's
/// `env`), else empty; `\$` is a plain `$`. A `#` after a value stays in it.
pub fn parse_env_file(text: &str, base: &Obj) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut line = String::new();
    let mut in_quotes = false;
    let mut quote = '\0';
    let mut after_eq = false;
    let chars: Vec<char> = text.chars().collect();
    let finish = |line: &str, vars: &mut Vec<(String, String)>| {
        if let Some((name, value)) = env_line(line) {
            let value = substitute_env(&value, vars, base);
            vars.retain(|(k, _)| *k != name);
            vars.push((name, value));
        }
    };
    for (i, &c) in chars.iter().enumerate() {
        if c == '=' && !in_quotes {
            after_eq = true;
            line.push(c);
            continue;
        }
        if (c == '"' || c == '\'') && after_eq {
            let slashes = chars[..i].iter().rev().take_while(|&&x| x == '\\').count();
            if slashes % 2 == 0 {
                if !in_quotes {
                    in_quotes = true;
                    quote = c;
                } else if c == quote {
                    in_quotes = false;
                }
            }
            line.push(c);
            continue;
        }
        if c == '\n' {
            if in_quotes {
                line.push(c);
            } else {
                finish(&line, &mut vars);
                line.clear();
                after_eq = false;
            }
        } else {
            line.push(c);
        }
    }
    if !line.trim().is_empty() {
        finish(&line, &mut vars);
    }
    vars
}

/// `^\s*(_*[a-zA-Z]\w*)\s*=\s*(.*?)?\s*$`, quotes off.
fn env_line(line: &str) -> Option<(String, String)> {
    let l = line.trim_start();
    let name_len = l
        .char_indices()
        .find(|&(i, c)| {
            let lead = l[..i].chars().all(|x| x == '_');
            !(c == '_' || c.is_ascii_alphanumeric()) || (lead && c.is_ascii_digit())
        })
        .map_or(l.len(), |(i, _)| i);
    let name = &l[..name_len];
    if !name
        .trim_start_matches('_')
        .starts_with(|c: char| c.is_ascii_alphabetic())
    {
        return None;
    }
    let rest = l[name_len..].trim_start().strip_prefix('=')?;
    let mut value = rest.trim().to_string();
    let quoted = |q: char| value.len() >= 2 && value.starts_with(q) && value.ends_with(q);
    if quoted('\'') || quoted('"') {
        value = value[1..value.len() - 1].replace("\\n", "\n");
    }
    Some((name.to_string(), value))
}

/// `${NAME}` in an env file value (not after a `\`).
fn substitute_env(value: &str, vars: &[(String, String)], base: &Obj) -> String {
    let re = regex::Regex::new(r"\$\{([a-zA-Z]\w*)?([^}\w][^}]*)?\}").unwrap();
    let mut invalid = false;
    let mut out = String::new();
    let mut last = 0;
    for c in re.captures_iter(value) {
        let m = c.get(0).unwrap();
        out.push_str(&value[last..m.start()]);
        last = m.end();
        if m.start() > 0 && value.as_bytes()[m.start() - 1] == b'\\' {
            out.push_str(m.as_str());
            continue;
        }
        let name = c.get(1).map(|x| x.as_str()).unwrap_or("");
        if c.get(2).is_some() || name.is_empty() {
            invalid = true;
            out.push_str(m.as_str());
            continue;
        }
        let local = vars.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        let global = base.get(name).and_then(Value::as_str).map(str::to_string);
        out.push_str(&local.filter(|v| !v.is_empty()).or(global).unwrap_or_default());
    }
    out.push_str(&value[last..]);
    let value = if invalid { value.to_string() } else { out };
    value.replace("\\$", "$")
}

const PATH_SEP: &str = if cfg!(windows) { ";" } else { ":" };

fn append_path(env: &mut Obj, name: &str, add: Option<&str>) {
    let Some(add) = add.map(str::trim).filter(|a| !a.is_empty()) else {
        return;
    };
    let joined = match env.get(name).and_then(Value::as_str).filter(|v| !v.is_empty()) {
        Some(v) => format!("{}{}{}", v, PATH_SEP, add),
        None => add.to_string(),
    };
    env.insert(name.into(), json!(joined));
}

/// `getDebugEnvironmentVariables`: the env file's variables, the
/// configuration's `env` over them (`PATH` and `PYTHONPATH` appended, not
/// replaced, with the process's own after them), the whole process
/// environment for `internalConsole`, and `PYTHONIOENCODING`,
/// `PYTHONUNBUFFERED` (and `GEVENT_SUPPORT`).
pub fn python_env(cfg: &Obj, process: &[(String, String)]) -> Obj {
    let path_name = if cfg!(windows) { "Path" } else { "PATH" };
    let launch_env: Obj = cfg.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut env = Obj::new();
    if let Some(text) = cfg
        .get("envFile")
        .and_then(Value::as_str)
        .and_then(|f| std::fs::read_to_string(f).ok())
    {
        for (k, v) in parse_env_file(&text, &launch_env) {
            env.insert(k, json!(v));
        }
    }
    for (k, v) in &launch_env {
        if k != "PYTHONPATH" && k != path_name {
            env.insert(k.clone(), v.clone());
        }
    }
    append_path(&mut env, path_name, launch_env.get(path_name).and_then(Value::as_str));
    append_path(
        &mut env,
        "PYTHONPATH",
        launch_env.get("PYTHONPATH").and_then(Value::as_str),
    );
    let own = |k: &str| process.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
    if env
        .get(path_name)
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty())
    {
        append_path(&mut env, path_name, own(path_name));
    }
    if env
        .get("PYTHONPATH")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty())
    {
        append_path(&mut env, "PYTHONPATH", own("PYTHONPATH"));
    }
    if cfg.get("console").and_then(Value::as_str) == Some("internalConsole") {
        for (k, v) in process {
            if k != "PYTHONPATH" && k != path_name && !env.contains_key(k) {
                env.insert(k.clone(), json!(v));
            }
        }
        for k in [path_name, "PYTHONPATH"] {
            if !env.contains_key(k) {
                if let Some(v) = own(k) {
                    env.insert(k.into(), json!(v));
                }
            }
        }
    }
    env.entry("PYTHONIOENCODING").or_insert(json!("UTF-8"));
    env.entry("PYTHONUNBUFFERED").or_insert(json!("1"));
    if cfg.get("gevent").and_then(Value::as_bool).unwrap_or(false) {
        env.insert("GEVENT_SUPPORT".into(), json!("True"));
    }
    env
}

fn debug_option(opts: &mut Vec<Value>, o: &str) {
    if !opts.iter().any(|x| x.as_str() == Some(o)) {
        opts.push(json!(o));
    }
}

fn truthy(cfg: &Obj, k: &str) -> bool {
    cfg.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn module_is(cfg: &Obj, m: &str) -> bool {
    cfg.get("module")
        .and_then(Value::as_str)
        .is_some_and(|x| x.eq_ignore_ascii_case(m))
}

fn client_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else {
        "unix"
    }
}

/// The interpreter a configuration names: `python`, else the legacy
/// `pythonPath`, else `interp` (the project's), as `resolveAndUpdatePythonPath`.
fn python_paths(cfg: &mut Obj, interp: &str) {
    let cmd = "${command:python.interpreterPath}";
    let python_path = match cfg.get("pythonPath").and_then(Value::as_str) {
        None | Some("") => interp.to_string(),
        Some(p) if p == cmd => interp.to_string(),
        Some(p) => p.to_string(),
    };
    let python = match cfg.get("python") {
        Some(Value::String(p)) if p == cmd => json!(interp),
        None | Some(Value::Null) => json!(python_path),
        Some(Value::String(p)) if p.is_empty() => json!(python_path),
        Some(v) => v.clone(),
    };
    for k in ["debugAdapterPython", "debugLauncherPython"] {
        let unset = match cfg.get(k) {
            None => true,
            Some(Value::String(s)) => s == cmd,
            _ => false,
        };
        if unset {
            let fallback = if cfg.contains_key("pythonPath") {
                json!(python_path)
            } else {
                python.clone()
            };
            cfg.insert(k.into(), fallback);
        }
    }
    cfg.insert("python".into(), python);
    cfg.remove("pythonPath");
}

/// `pathMappings` with `localRoot` made absolute from the workspace folder;
/// an empty list becomes `[{root, root}]` (`fixUpPathMappings`).
fn fix_path_mappings(maps: Vec<Value>, root: &Path) -> Vec<Value> {
    let r = root.display().to_string();
    if maps.is_empty() {
        return vec![json!({ "localRoot": r, "remoteRoot": r })];
    }
    maps.into_iter()
        .map(|m| {
            let local = m["localRoot"].as_str().unwrap_or("");
            let local = if Path::new(local).is_absolute() {
                local.to_string()
            } else {
                root.join(local).display().to_string()
            };
            json!({ "localRoot": local, "remoteRoot": m["remoteRoot"].clone() })
        })
        .collect()
}

/// A `debugpy` / `python` launch configuration as the Python extension hands
/// it to debugpy (`resolveDebugConfiguration`, then `provideLaunchDefaults`).
/// `cfg` has its variables substituted; `interp` is the project's Python.
pub fn python_launch(cfg: &mut Obj, root: &Path, interp: &str, process: &[(String, String)]) {
    python_paths(cfg, interp);
    if !cfg.contains_key("clientOS") {
        cfg.insert("clientOS".into(), json!(client_os()));
    }
    if let Some(n) = cfg.remove("consoleName") {
        cfg.insert("consoleTitle".into(), n);
    }
    if !cfg.get("cwd").is_some_and(Value::is_string) {
        cfg.insert("cwd".into(), json!(root.display().to_string()));
    }
    match cfg.get("envFile").and_then(Value::as_str) {
        Some(f) if !Path::new(f).is_absolute() => {
            let abs = root.join(f).display().to_string();
            cfg.insert("envFile".into(), json!(abs));
        }
        Some(_) => {}
        None => {
            let def = root.join(".env").display().to_string();
            cfg.insert("envFile".into(), json!(def));
        }
    }
    if !cfg
        .get("console")
        .and_then(Value::as_str)
        .is_some_and(|c| !c.is_empty())
    {
        cfg.insert("console".into(), json!("integratedTerminal"));
    }
    let env = python_env(cfg, process);
    cfg.insert("env".into(), Value::Object(env));
    if !cfg.get("stopOnEntry").is_some_and(Value::is_boolean) {
        cfg.insert("stopOnEntry".into(), json!(false));
    }
    let show = cfg.get("showReturnValue").and_then(Value::as_bool) != Some(false);
    cfg.insert("showReturnValue".into(), json!(show));
    let console = cfg.get("console").and_then(Value::as_str).unwrap_or("").to_string();
    if console != "internalConsole" && !cfg.contains_key("internalConsoleOptions") {
        cfg.insert("internalConsoleOptions".into(), json!("neverOpen"));
    }
    if !cfg
        .get("terminalQuoteCharacter")
        .and_then(Value::as_str)
        .is_some_and(|q| q.chars().count() == 1)
    {
        cfg.insert(
            "terminalQuoteCharacter".into(),
            json!(if cfg!(windows) { "'" } else { "\"" }),
        );
    }
    let mut opts: Vec<Value> = cfg
        .get("debugOptions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !cfg.contains_key("justMyCode") {
        cfg.insert("justMyCode".into(), json!(true));
    }
    cfg.insert("workspaceFolder".into(), json!(root.display().to_string()));
    if truthy(cfg, "stopOnEntry") {
        debug_option(&mut opts, "StopOnEntry");
    }
    if truthy(cfg, "showReturnValue") {
        debug_option(&mut opts, "ShowReturnValue");
    }
    if truthy(cfg, "django") {
        debug_option(&mut opts, "Django");
    }
    if truthy(cfg, "jinja") {
        debug_option(&mut opts, "Jinja");
    }
    if !cfg.contains_key("redirectOutput") && console == "internalConsole" {
        cfg.insert("redirectOutput".into(), json!(true));
    }
    if truthy(cfg, "redirectOutput") {
        debug_option(&mut opts, "RedirectOutput");
    }
    if truthy(cfg, "sudo") {
        debug_option(&mut opts, "Sudo");
    }
    if cfg.get("subProcess") == Some(&json!(true)) {
        debug_option(&mut opts, "Multiprocess");
    }
    if cfg!(windows) {
        debug_option(&mut opts, "FixFilePathCase");
    }
    let flask = module_is(cfg, "flask");
    let fastapi = module_is(cfg, "fastapi");
    if truthy(cfg, "autoStartBrowser") && (truthy(cfg, "django") || flask) && !cfg.contains_key("serverReadyAction") {
        cfg.insert(
            "serverReadyAction".into(),
            json!({ "pattern": ".*(https?:\\/\\/\\S+:[0-9]+\\/?).*", "uriFormat": "%s", "action": "openExternally" }),
        );
    }
    let jinja_on = opts.iter().any(|o| o.as_str() == Some("Jinja"));
    if (truthy(cfg, "pyramid") || flask || fastapi) && !jinja_on && cfg.get("jinja") != Some(&json!(false)) {
        debug_option(&mut opts, "Jinja");
    }
    if let Some(maps) = cfg.get("pathMappings").and_then(Value::as_array).cloned() {
        if maps.is_empty() {
            cfg.remove("pathMappings");
        } else {
            cfg.insert("pathMappings".into(), json!(fix_path_mappings(maps, root)));
        }
    }
    let mut seen = Vec::new();
    opts.retain(|o| {
        let new = !seen.contains(o);
        seen.push(o.clone());
        new
    });
    cfg.insert("debugOptions".into(), Value::Array(opts));
}

/// A `debugpy` / `python` attach configuration (`provideAttachDefaults`).
pub fn python_attach(cfg: &mut Obj, root: &Path) {
    let mut opts: Vec<Value> = cfg
        .get("debugOptions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !(cfg.contains_key("connect") || cfg.contains_key("listen")) && !cfg.get("host").is_some_and(Value::is_string) {
        cfg.insert("host".into(), json!("localhost"));
    }
    if !cfg.contains_key("justMyCode") {
        cfg.insert("justMyCode".into(), json!(true));
    }
    let show = cfg.get("showReturnValue").and_then(Value::as_bool) != Some(false);
    cfg.insert("showReturnValue".into(), json!(show));
    cfg.insert("workspaceFolder".into(), json!(root.display().to_string()));
    if truthy(cfg, "django") {
        debug_option(&mut opts, "Django");
    }
    if truthy(cfg, "jinja") {
        debug_option(&mut opts, "Jinja");
    }
    if cfg.get("subProcess") == Some(&json!(true)) {
        debug_option(&mut opts, "Multiprocess");
    }
    let jinja_on = opts.iter().any(|o| o.as_str() == Some("Jinja"));
    if truthy(cfg, "pyramid") && !jinja_on && cfg.get("jinja") != Some(&json!(false)) {
        debug_option(&mut opts, "Jinja");
    }
    if cfg.get("redirectOutput").and_then(Value::as_bool) != Some(false) {
        debug_option(&mut opts, "RedirectOutput");
    }
    let host = cfg.get("host").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1");
    if cfg!(windows) && local {
        debug_option(&mut opts, "FixFilePathCase");
    }
    if !cfg.contains_key("clientOS") {
        cfg.insert("clientOS".into(), json!(client_os()));
    }
    if truthy(cfg, "showReturnValue") {
        debug_option(&mut opts, "ShowReturnValue");
    }
    let mut maps: Vec<Value> = cfg
        .get("pathMappings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let (Some(l), Some(r)) = (cfg.get("localRoot"), cfg.get("remoteRoot")) {
        if l.as_str().is_some_and(|s| !s.is_empty()) && r.as_str().is_some_and(|s| !s.is_empty()) {
            maps.push(json!({ "localRoot": l, "remoteRoot": r }));
        }
    }
    if local {
        maps = fix_path_mappings(maps, root);
    }
    if maps.is_empty() {
        cfg.remove("pathMappings");
    } else {
        cfg.insert("pathMappings".into(), Value::Array(maps));
    }
    let mut seen = Vec::new();
    opts.retain(|o| {
        let new = !seen.contains(o);
        seen.push(o.clone());
        new
    });
    cfg.insert("debugOptions".into(), Value::Array(opts));
}

/// An env file as vscode-go and cpptools read one:
/// `^\s*(export\s+)?([\w.\-]+)\s*=\s*(.*)?\s*$`, quotes taken off (`\n` in
/// double quotes is a newline), `${VAR}` from earlier lines or ren's own
/// environment.
pub fn read_env_file(text: &str) -> Vec<(String, String)> {
    let re = regex::Regex::new(r"^\s*(?:export\s+)?([\w.\-]+)\s*=\s*(.*?)\s*$").unwrap();
    let var = regex::Regex::new(r"\$\{([\w.\-]+)\}").unwrap();
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let Some(c) = re.captures(line) else { continue };
        let k = c[1].to_string();
        let mut v = c.get(2).map_or("", |m| m.as_str()).to_string();
        let double = v.len() >= 2 && v.starts_with('"') && v.ends_with('"');
        let single = v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'');
        if double || single {
            v = v[1..v.len() - 1].to_string();
            if double {
                v = v.replace("\\n", "\n");
            }
        }
        if !single {
            v = var
                .replace_all(&v, |m: &regex::Captures| {
                    out.iter()
                        .rev()
                        .find(|(x, _)| *x == m[1])
                        .map(|(_, y)| y.clone())
                        .or_else(|| std::env::var(&m[1]).ok())
                        .unwrap_or_default()
                })
                .into_owned();
        }
        out.retain(|(x, _)| *x != k);
        out.push((k, v));
    }
    out
}

/// `"args": "a 'b c'"` as an array, split the way a shell would.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, '\\') => {
                if let Some(n) = it.next() {
                    cur.push(n);
                    any = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn abs_from(root: &Path, p: &str) -> PathBuf {
    let p = Path::new(p);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

/// What a Go configuration needs besides its arguments.
#[derive(Debug, Default, PartialEq)]
pub struct GoExtra {
    /// Where `dlv dap` starts (vscode-go's `__buildDir`).
    pub build_dir: Option<PathBuf>,
    /// Arguments for `dlv` after `dap` (`dlvFlags`, `showLog`).
    pub dlv_args: Vec<String>,
    /// `port` set: a dlv server already listening, to connect to.
    pub connect: Option<(String, u16)>,
}

/// A `go` configuration as vscode-go hands it to `dlv dap`.
pub fn go(cfg: &mut Obj, root: &Path, file: Option<&Path>) -> Result<GoExtra, String> {
    let mut extra = GoExtra::default();
    let request = cfg
        .get("request")
        .and_then(Value::as_str)
        .unwrap_or("launch")
        .to_string();
    if !cfg.contains_key("mode") {
        cfg.insert("mode".into(), json!(if request == "attach" { "local" } else { "auto" }));
    }
    for k in ["program", "cwd", "output"] {
        if let Some(p) = cfg.get(k).and_then(Value::as_str) {
            let abs = abs_from(root, p).display().to_string();
            cfg.insert(k.into(), json!(abs));
        }
    }
    if cfg.get("mode").and_then(Value::as_str) == Some("auto") {
        let program = cfg.get("program").and_then(Value::as_str).unwrap_or("");
        let probe = if program.ends_with(".go") {
            program.to_string()
        } else {
            file.map(|f| f.display().to_string()).unwrap_or_default()
        };
        let mode = if probe.ends_with("_test.go") { "test" } else { "debug" };
        cfg.insert("mode".into(), json!(mode));
    }
    let mode = cfg.get("mode").and_then(Value::as_str).unwrap_or("debug").to_string();
    if mode == "test" {
        if let Some(p) = cfg
            .get("program")
            .and_then(Value::as_str)
            .filter(|p| p.ends_with("_test.go"))
        {
            let d = Path::new(p)
                .parent()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            cfg.insert("program".into(), json!(d));
        }
    }
    if let Some(port) = cfg.get("port").and_then(Value::as_u64) {
        let host = cfg
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or("127.0.0.1")
            .to_string();
        extra.connect = Some((host, port as u16));
    } else if request == "launch" && matches!(mode.as_str(), "debug" | "test") {
        let program = cfg.get("program").and_then(Value::as_str).unwrap_or("").to_string();
        let p = Path::new(&program);
        if p.is_dir() {
            extra.build_dir = Some(p.to_path_buf());
            cfg.insert("program".into(), json!("."));
        } else if program.ends_with(".go") {
            extra.build_dir = p.parent().map(Path::to_path_buf);
            cfg.insert("program".into(), json!(format!("./{}", base_name(p))));
        } else {
            return Err(format!(
                "The program attribute '{}' must be a valid directory or .go file",
                program
            ));
        }
    }
    // env over envFile (one file, or several: later ones win).
    let files: Vec<String> = match cfg.remove("envFile") {
        Some(Value::String(f)) => vec![f],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let mut env = Obj::new();
    for f in files {
        let path = abs_from(root, &f);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("envFile {}: {}", path.display(), e))?;
        for (k, v) in read_env_file(&text) {
            env.insert(k, json!(v));
        }
    }
    for (k, v) in cfg.get("env").and_then(Value::as_object).into_iter().flatten() {
        env.insert(k.clone(), v.clone());
    }
    if !env.is_empty() {
        cfg.insert("env".into(), Value::Object(env));
    }
    if let Some(a) = cfg.get("args").and_then(Value::as_str).map(split_args) {
        cfg.insert("args".into(), json!(a));
    }
    if request == "attach" && mode == "local" {
        match cfg.get("processId") {
            Some(Value::Number(_)) if cfg["processId"].as_u64() != Some(0) => {}
            Some(Value::String(s)) if s.parse::<u64>().is_ok() => {
                let n: u64 = s.parse().unwrap_or(0);
                cfg.insert("processId".into(), json!(n));
            }
            Some(Value::String(name)) if !name.is_empty() => {
                let pid = pid_of(name).ok_or_else(|| format!("No process named {}", name))?;
                cfg.insert("processId".into(), json!(pid));
            }
            _ => return Err("pick a process: set \"processId\": \"${command:pickProcess}\"".into()),
        }
    }
    extra.dlv_args = cfg
        .get("dlvFlags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    if truthy(cfg, "showLog") {
        extra.dlv_args.push("--log=true".into());
        let out = cfg.get("logOutput").and_then(Value::as_str).unwrap_or("debugger");
        extra.dlv_args.push(format!("--log-output={}", out));
    }
    Ok(extra)
}

/// A running process's pid by its name (the newest one).
fn pid_of(name: &str) -> Option<u64> {
    let o = std::process::Command::new("pgrep")
        .args(["-n", "-x", name])
        .output()
        .ok()?;
    String::from_utf8_lossy(&o.stdout).trim().parse().ok()
}

/// A `cppdbg` configuration (ms-vscode.cpptools) for gdb's own DAP
/// (`gdb -i dap`) or lldb-dap (`MIMode: lldb`): (lldb?, arguments, extra
/// gdb command-line arguments for `setupCommands`).
pub fn cppdbg(cfg: &Obj, root: &Path, process: &[(String, String)]) -> Result<(bool, Obj, Vec<String>), String> {
    let lldb = cfg.get("MIMode").and_then(Value::as_str) == Some("lldb");
    let attach = cfg.get("request").and_then(Value::as_str) == Some("attach");
    let mut out = Obj::new();
    for k in ["program", "args"] {
        if let Some(v) = cfg.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(a) = cfg.get("args").and_then(Value::as_str).map(split_args) {
        out.insert("args".into(), json!(a));
    }
    if !attach {
        let Some(cwd) = cfg.get("cwd").and_then(Value::as_str) else {
            return Err("cppdbg: \"cwd\" is required".into());
        };
        out.insert("cwd".into(), json!(abs_from(root, cwd).display().to_string()));
        // `environment`, then the env file over it (cpptools' order).
        let mut env: Vec<(String, String)> = cfg
            .get("environment")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some((
                    e["name"].as_str()?.to_string(),
                    e["value"].as_str().unwrap_or("").to_string(),
                ))
            })
            .collect();
        if let Some(f) = cfg.get("envFile").and_then(Value::as_str) {
            let path = abs_from(root, f);
            let text = std::fs::read_to_string(&path).map_err(|e| format!("envFile {}: {}", path.display(), e))?;
            for (k, v) in read_env_file(&text) {
                env.retain(|(x, _)| *x != k);
                env.push((k, v));
            }
        }
        let mut obj = Obj::new();
        if !lldb {
            // gdb's DAP replaces the whole environment: start from ren's.
            for (k, v) in process {
                obj.insert(k.clone(), json!(v));
            }
        }
        for (k, v) in env {
            obj.insert(k, json!(v));
        }
        if !obj.is_empty() {
            out.insert("env".into(), Value::Object(obj));
        }
        if truthy(cfg, "stopAtEntry") {
            if lldb {
                out.insert(
                    "preRunCommands".into(),
                    json!(["breakpoint set --name main --one-shot true"]),
                );
            } else {
                out.insert("stopAtBeginningOfMainSubprogram".into(), json!(true));
            }
        }
        if lldb && truthy(cfg, "externalConsole") {
            out.insert("console".into(), json!("integratedTerminal"));
        }
    } else {
        match cfg.get("processId") {
            Some(Value::Number(n)) => {
                out.insert("pid".into(), Value::Number(n.clone()));
            }
            Some(Value::String(s)) if s.parse::<u64>().is_ok() => {
                out.insert("pid".into(), json!(s.parse::<u64>().unwrap_or(0)));
            }
            _ => {
                let prog = cfg
                    .get("program")
                    .and_then(Value::as_str)
                    .map(|p| base_name(Path::new(p)));
                match prog.and_then(|p| pid_of(&p)) {
                    Some(pid) => {
                        out.insert("pid".into(), json!(pid));
                    }
                    None => return Err("No process was selected.".into()),
                }
            }
        }
        if let Some(addr) = cfg.get("miDebuggerServerAddress").and_then(Value::as_str) {
            if lldb {
                let (h, p) = addr.rsplit_once(':').unwrap_or(("localhost", addr));
                out.insert("gdb-remote-hostname".into(), json!(h));
                out.insert("gdb-remote-port".into(), json!(p.parse::<u64>().unwrap_or(0)));
            } else {
                out.insert("target".into(), json!(addr));
            }
        }
    }
    let setup: Vec<String> = cfg
        .get("setupCommands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str().map(str::to_string))
        .collect();
    let mut gdb_args = Vec::new();
    if lldb {
        if !setup.is_empty() {
            out.insert("initCommands".into(), json!(setup));
        }
    } else {
        for t in setup {
            gdb_args.push("-iex".into());
            gdb_args.push(t);
        }
    }
    Ok((lldb, out, gdb_args))
}

/// A CodeLLDB (`lldb` / `codelldb`) configuration: `custom` runs as
/// `launch`, a string `args` is split, `relativePathBase` is the folder;
/// the rest (`envFile`, `terminal`, `sourceMap`, …) the adapter reads itself.
pub fn codelldb(cfg: &mut Obj, root: &Path) {
    if cfg.get("request").and_then(Value::as_str) == Some("custom") {
        cfg.insert("request".into(), json!("launch"));
    }
    if let Some(a) = cfg.get("args").and_then(Value::as_str).map(split_args) {
        cfg.insert("args".into(), json!(a));
    }
    if !cfg.contains_key("relativePathBase") {
        cfg.insert("relativePathBase".into(), json!(root.display().to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_files_as_vscode_reads_them() {
        let base: Obj = serde_json::from_value(json!({ "FROM_LAUNCH": "L" })).unwrap();
        let text = "# comment\nexport SKIPPED=1\nA=1\nB = two words # stays\nC=\"x\\ny ${A}\"\nD='${A} single'\nE=${FROM_LAUNCH}-${HOME}\nF=\\${A}\nMULTI=\"line1\nline2\"\n_X1=ok\n9BAD=no\nA=3\n";
        let v = parse_env_file(text, &base);
        let get = |k: &str| v.iter().find(|(x, _)| x == k).map(|(_, y)| y.as_str());
        assert_eq!(get("SKIPPED"), None);
        assert_eq!(get("A"), Some("3"));
        assert_eq!(get("B"), Some("two words # stays"));
        assert_eq!(get("C"), Some("x\ny 1"));
        assert_eq!(get("D"), Some("1 single"));
        // Only the file and launch.json's env: not the process environment.
        assert_eq!(get("E"), Some("L-"));
        assert_eq!(get("F"), Some("${A}"));
        assert_eq!(get("MULTI"), Some("line1\nline2"));
        assert_eq!(get("_X1"), Some("ok"));
        assert_eq!(get("9BAD"), None);
    }

    #[test]
    fn python_defaults_like_vscode() {
        let dir = std::env::temp_dir().join(format!("ren-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env"), "FROM_FILE=f\nFLASK_ENV=file\nPATH=/file/bin\n").unwrap();
        let mut cfg: Obj = serde_json::from_value(json!({
            "name": "API", "type": "debugpy", "request": "launch",
            "module": "flask", "args": ["run"],
            "env": { "FLASK_ENV": "development", "PATH": "/launch/bin" }
        }))
        .unwrap();
        let process = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/h".to_string()),
        ];
        python_launch(&mut cfg, &dir, "/venv/bin/python", &process);
        assert_eq!(cfg["console"], "integratedTerminal");
        assert_eq!(cfg["internalConsoleOptions"], "neverOpen");
        assert_eq!(cfg["python"], "/venv/bin/python");
        assert_eq!(cfg["debugAdapterPython"], "/venv/bin/python");
        assert_eq!(cfg["cwd"], dir.display().to_string());
        assert_eq!(cfg["justMyCode"], true);
        assert_eq!(cfg["stopOnEntry"], false);
        assert_eq!(cfg["showReturnValue"], true);
        assert!(cfg.get("redirectOutput").is_none());
        assert_eq!(cfg["debugOptions"], json!(["ShowReturnValue", "Jinja"]));
        let env = &cfg["env"];
        assert_eq!(env["FROM_FILE"], "f");
        assert_eq!(env["FLASK_ENV"], "development");
        assert_eq!(env["PATH"], "/file/bin:/launch/bin:/usr/bin");
        assert_eq!(env["PYTHONUNBUFFERED"], "1");
        assert_eq!(env["PYTHONIOENCODING"], "UTF-8");
        // Not the whole environment: the terminal has it already.
        assert!(env.get("HOME").is_none());

        let mut cfg: Obj = serde_json::from_value(json!({
            "type": "debugpy", "request": "launch", "program": "x.py",
            "console": "internalConsole", "jinja": false, "pythonPath": "/legacy/python"
        }))
        .unwrap();
        python_launch(&mut cfg, &dir, "/venv/bin/python", &process);
        assert_eq!(cfg["python"], "/legacy/python");
        assert!(cfg.get("pythonPath").is_none());
        assert_eq!(cfg["redirectOutput"], true);
        assert_eq!(cfg["env"]["HOME"], "/h");
        assert_eq!(cfg["debugOptions"], json!(["ShowReturnValue", "RedirectOutput"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn doc() -> Value {
        json!({
            "configurations": [
                { "name": "plain", "type": "debugpy", "request": "launch", "program": "a.py" },
                { "name": "hidden", "type": "debugpy", "request": "launch", "presentation": { "hidden": true } },
                { "name": "ordered", "type": "go", "request": "launch", "presentation": { "order": 2 } },
                { "name": "g-b", "type": "go", "request": "launch", "presentation": { "group": "b", "order": 1 } },
                { "name": "g-a", "type": "go", "request": "attach", "presentation": { "group": "a" } },
                { "name": "node", "type": "node", "request": "launch" },
                { "name": "plat", "type": "debugpy", "request": "launch",
                  "env": { "A": "1", "B": "2" },
                  "linux": { "env": { "A": "L" } }, "osx": { "env": { "A": "M" } }, "windows": { "env": { "A": "W" } } }
            ]
        })
    }

    #[test]
    fn list_order_and_platforms() {
        assert_eq!(config_names(&doc()), vec!["g-a", "g-b", "ordered", "plain", "plat"]);
        let c = config(&doc(), Some("plat")).unwrap();
        // Shallow: the OS's env replaces the whole env.
        let want = match platform_key() {
            "osx" => "M",
            "linux" => "L",
            _ => "W",
        };
        assert_eq!(c["env"], json!({ "A": want }));
        assert!(c.get("linux").is_none() && c.get("osx").is_none());
        assert_eq!(config(&doc(), None).unwrap()["name"], "g-a");
    }

    #[test]
    fn variables() {
        let ctx = Ctx {
            root: PathBuf::from("/w/api"),
            folders: vec![PathBuf::from("/w/api"), PathBuf::from("/w/web")],
            file: Some(PathBuf::from("/w/api/src/app.py")),
            line: 7,
            column: 3,
            python: Some("/w/api/.venv/bin/python".into()),
            ..Default::default()
        };
        let v = json!({
            "a": "${workspaceFolder}/x", "b": "${workspaceFolder:web}", "c": "${relativeFile}",
            "d": "${relativeFileDirname}", "e": "${fileBasenameNoExtension}${fileExtname}",
            "f": "${fileDirnameBasename}:${lineNumber}", "g": "${unknown} ${env:REN_NOT_SET_X}|",
            "${workspaceFolderBasename}": [ "${command:python.interpreterPath}" ]
        });
        let r = substitute(&v, &ctx, &Value::Null).unwrap();
        assert_eq!(r["a"], "/w/api/x");
        assert_eq!(r["b"], "/w/web");
        assert_eq!(r["c"], "src/app.py");
        assert_eq!(r["d"], "src");
        assert_eq!(r["e"], "app.py");
        assert_eq!(r["f"], "src:7");
        assert_eq!(r["g"], "${unknown} |");
        assert_eq!(r["api"], json!(["/w/api/.venv/bin/python"]));
        let none = Ctx {
            file: None,
            ..ctx.clone()
        };
        assert!(matches!(
            substitute(&json!("${file}"), &none, &Value::Null),
            Err(Subst::Error(_))
        ));
        assert!(matches!(
            substitute(&json!("${workspaceFolder:nope}"), &ctx, &Value::Null),
            Err(Subst::Error(_))
        ));
    }

    #[test]
    fn inputs_are_asked_one_by_one() {
        let inputs = json!([
            { "id": "port", "type": "promptString", "description": "Port?", "default": "8000" },
            { "id": "env", "type": "pickString", "description": "Env", "options": ["dev", { "label": "Prod", "value": "prod" }] }
        ]);
        let v = json!(["${input:port}", "${input:env}", "${command:pickProcess}"]);
        let mut ctx = Ctx {
            root: PathBuf::from("/w"),
            ..Default::default()
        };
        let ask = substitute(&v, &ctx, &inputs).unwrap_err();
        assert_eq!(
            ask,
            Subst::Ask(Ask::Prompt("input:port".into(), "Port?".into(), "8000".into(), false))
        );
        ctx.answers.insert("input:port".into(), "9000".into());
        let ask = substitute(&v, &ctx, &inputs).unwrap_err();
        assert_eq!(
            ask,
            Subst::Ask(Ask::Pick(
                "input:env".into(),
                "Env".into(),
                vec![("dev".into(), "dev".into()), ("Prod: prod".into(), "prod".into())],
                None
            ))
        );
        ctx.answers.insert("input:env".into(), "prod".into());
        assert_eq!(
            substitute(&v, &ctx, &inputs).unwrap_err(),
            Subst::Ask(Ask::Process("command:pickProcess".into(), false))
        );
        ctx.answers.insert("command:pickProcess".into(), "42".into());
        assert_eq!(substitute(&v, &ctx, &inputs).unwrap(), json!(["9000", "prod", "42"]));
    }

    #[test]
    fn go_like_vscode_go() {
        let dir = std::env::temp_dir().join(format!("ren-go-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("cmd/srv")).unwrap();
        std::fs::write(dir.join(".env"), "export A=1\nB=\"${A}-x\"\n").unwrap();
        let mut cfg: Obj = serde_json::from_value(json!({
            "type": "go", "request": "launch", "program": "cmd/srv", "envFile": ".env",
            "env": { "B": "launch" }, "args": "--a 'b c'", "showLog": true
        }))
        .unwrap();
        let x = go(&mut cfg, &dir, Some(&dir.join("cmd/srv/main.go"))).unwrap();
        assert_eq!(cfg["mode"], "debug");
        assert_eq!(cfg["program"], ".");
        assert_eq!(x.build_dir, Some(dir.join("cmd/srv")));
        assert_eq!(cfg["env"], json!({ "A": "1", "B": "launch" }));
        assert!(cfg.get("envFile").is_none());
        assert_eq!(cfg["args"], json!(["--a", "b c"]));
        assert_eq!(x.dlv_args, vec!["--log=true", "--log-output=debugger"]);
        let mut cfg: Obj =
            serde_json::from_value(json!({ "type": "go", "request": "launch", "program": "${fileDirname}" })).unwrap();
        cfg.insert("program".into(), json!(dir.join("cmd/srv").display().to_string()));
        go(&mut cfg, &dir, Some(&dir.join("cmd/srv/x_test.go"))).unwrap();
        assert_eq!(cfg["mode"], "test");
        let mut cfg: Obj =
            serde_json::from_value(json!({ "type": "go", "request": "attach", "mode": "remote", "port": 2345 }))
                .unwrap();
        assert_eq!(
            go(&mut cfg, &dir, None).unwrap().connect,
            Some(("127.0.0.1".into(), 2345))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cppdbg_to_gdb_and_lldb() {
        let cfg: Obj = serde_json::from_value(json!({
            "type": "cppdbg", "request": "launch", "program": "/w/a.out", "args": ["x"], "cwd": ".",
            "environment": [{ "name": "K", "value": "v" }], "stopAtEntry": true,
            "setupCommands": [{ "text": "-enable-pretty-printing" }]
        }))
        .unwrap();
        let process = vec![("PATH".to_string(), "/usr/bin".to_string())];
        let (lldb, out, extra) = cppdbg(&cfg, Path::new("/w"), &process).unwrap();
        assert!(!lldb);
        assert_eq!(out["cwd"], "/w/.");
        assert_eq!(out["env"], json!({ "PATH": "/usr/bin", "K": "v" }));
        assert_eq!(out["stopAtBeginningOfMainSubprogram"], true);
        assert_eq!(extra, vec!["-iex", "-enable-pretty-printing"]);
        let mut l = cfg.clone();
        l.insert("MIMode".into(), json!("lldb"));
        let (lldb, out, _) = cppdbg(&l, Path::new("/w"), &process).unwrap();
        assert!(lldb);
        assert_eq!(out["env"], json!({ "K": "v" }));
        assert_eq!(out["initCommands"], json!(["-enable-pretty-printing"]));
        assert!(out.get("preRunCommands").is_some());
        let mut a = cfg.clone();
        a.remove("cwd");
        assert!(cppdbg(&a, Path::new("/w"), &process).is_err());
    }

    #[test]
    fn args_strings() {
        assert_eq!(
            split_args(r#"run --name "a b" 'c d' e\ f """#),
            vec!["run", "--name", "a b", "c d", "e f", ""]
        );
    }

    #[test]
    fn python_attach_like_vscode() {
        let root = Path::new("/w/app");
        let mut cfg: Obj = serde_json::from_value(json!({
            "type": "debugpy", "request": "attach", "connect": { "host": "localhost", "port": 5678 },
            "pathMappings": [{ "localRoot": "src", "remoteRoot": "/app" }]
        }))
        .unwrap();
        python_attach(&mut cfg, root);
        assert!(cfg.get("host").is_none());
        // Not local by "host", so the mappings stay as written.
        assert_eq!(cfg["pathMappings"][0]["localRoot"], "src");
        assert_eq!(cfg["debugOptions"], json!(["RedirectOutput", "ShowReturnValue"]));

        let mut cfg: Obj =
            serde_json::from_value(json!({ "type": "debugpy", "request": "attach", "port": 5678 })).unwrap();
        python_attach(&mut cfg, root);
        assert_eq!(cfg["host"], "localhost");
        assert_eq!(
            cfg["pathMappings"],
            json!([{ "localRoot": "/w/app", "remoteRoot": "/w/app" }])
        );
    }
}
