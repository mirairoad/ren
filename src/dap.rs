use crate::lsp::{find_root, which};
use crate::tools::{self, Tool};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    CodeLldb,
    Lldb,
    Gdb,
    Debugpy,
    Delve,
}

#[derive(Clone, Debug)]
pub struct Adapter {
    pub kind: Kind,
    pub name: String,
    pub cmd: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug)]
pub enum PlanErr {
    Missing(Tool),
    Other(String),
    /// A value to ask for first (`${input:…}`, `${command:pickProcess}`).
    Ask(crate::launch::Ask),
}

impl From<String> for PlanErr {
    fn from(s: String) -> Self {
        PlanErr::Other(s)
    }
}

impl From<&str> for PlanErr {
    fn from(s: &str) -> Self {
        PlanErr::Other(s.to_string())
    }
}

#[derive(Clone, Debug)]
pub enum Req {
    Initialize,
    Launch,
    /// A subprocess session's `attach` (debugpy).
    Attach,
    SetBreakpoints(PathBuf),
    ConfigDone,
    StackTrace,
    Scopes,
    Variables(i64),
    Evaluate(EvalKind),
    Step,
    Disconnect,
    Other,
}

#[derive(Clone, Debug)]
pub enum EvalKind {
    Repl,
    Watch(usize),
    Hover,
}

pub enum DapMsg {
    Response {
        req: Req,
        success: bool,
        message: String,
        body: Value,
    },
    Event {
        event: String,
        body: Value,
    },
    Request {
        seq: i64,
        command: String,
        args: Value,
    },
    Closed,
}

enum Raw {
    Msg(Value),
    Eof,
}

pub struct Client {
    /// The adapter's process; None for a session over another one's socket.
    child: Option<Child>,
    writer: Box<dyn Write + Send>,
    seq: i64,
    pending: HashMap<i64, Req>,
    rx: Receiver<Raw>,
    pub caps: Value,
    pub adapter: Adapter,
    pub closed: bool,
    /// A subprocess's session: the `attach` arguments its adapter sent
    /// (debugpy's `debugpyAttach` / `startDebugging`).
    pub attach: Option<Value>,
    /// Configured (breakpoints sent, `configurationDone`).
    pub ready: bool,
}

fn reader(r: impl Read, tx: Sender<Raw>) {
    let mut r = BufReader::new(r);
    loop {
        let mut len: Option<usize> = None;
        loop {
            let mut line = String::new();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(Raw::Eof);
                    return;
                }
                Ok(_) => {}
            }
            let l = line.trim_end();
            if l.is_empty() {
                if len.is_some() {
                    break;
                }
                continue;
            }
            if let Some(v) = l.strip_prefix("Content-Length:") {
                len = v.trim().parse().ok();
            }
        }
        let mut buf = vec![0u8; len.unwrap_or(0)];
        if r.read_exact(&mut buf).is_err() {
            let _ = tx.send(Raw::Eof);
            return;
        }
        crate::lsp::log("<< dap", &String::from_utf8_lossy(&buf));
        if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
            if tx.send(Raw::Msg(v)).is_err() {
                return;
            }
        }
    }
}

fn free_port() -> Option<u16> {
    let l = TcpListener::bind("127.0.0.1:0").ok()?;
    Some(l.local_addr().ok()?.port())
}

#[cfg(unix)]
fn detach(c: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000);
}

fn spawn(cmd: &[String], env: &[(String, String)], cwd: &Path, piped: bool) -> std::io::Result<Child> {
    let exe = PathBuf::from(&cmd[0]);
    let ext = exe
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut c = if ext == "cmd" || ext == "bat" {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&exe);
        c
    } else {
        Command::new(&exe)
    };
    c.args(&cmd[1..]).current_dir(cwd).stderr(Stdio::null());
    for (k, v) in env {
        c.env(k, v);
    }
    detach(&mut c);
    if piped {
        c.stdin(Stdio::piped()).stdout(Stdio::piped());
    } else {
        c.stdin(Stdio::null()).stdout(Stdio::null());
    }
    c.spawn()
}

impl Client {
    pub fn start(adapter: Adapter, cwd: &Path) -> Result<Client, String> {
        let (tx, rx) = channel();
        let (child, writer): (Child, Box<dyn Write + Send>) = if adapter.kind == Kind::Delve {
            let port = free_port().ok_or("no free port")?;
            let mut cmd = adapter.cmd.clone();
            cmd.push(format!("--listen=127.0.0.1:{}", port));
            let child = spawn(&cmd, &adapter.env, cwd, false).map_err(|e| e.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(10);
            let stream = loop {
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(s) => break s,
                    Err(e) if Instant::now() > deadline => return Err(e.to_string()),
                    Err(_) => thread::sleep(Duration::from_millis(100)),
                }
            };
            let rs = stream.try_clone().map_err(|e| e.to_string())?;
            thread::spawn(move || reader(rs, tx));
            (child, Box::new(stream))
        } else {
            let mut child = spawn(&adapter.cmd, &adapter.env, cwd, true).map_err(|e| e.to_string())?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            thread::spawn(move || reader(stdout, tx));
            (child, Box::new(stdin))
        };
        Ok(Client {
            child: Some(child),
            writer,
            seq: 1,
            pending: HashMap::new(),
            rx,
            caps: Value::Null,
            adapter,
            closed: false,
            attach: None,
            ready: false,
        })
    }

    /// A session over a socket: a debug server already listening (debugpy's
    /// `connect`, a remote `dlv`), or a subprocess's port.
    pub fn connect(adapter: Adapter, host: &str, port: u16) -> Result<Client, String> {
        use std::net::ToSocketAddrs;
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("no address")?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        let rs = stream.try_clone().map_err(|e| e.to_string())?;
        let (tx, rx) = channel();
        thread::spawn(move || reader(rs, tx));
        Ok(Client {
            child: None,
            writer: Box::new(stream),
            seq: 1,
            pending: HashMap::new(),
            rx,
            caps: Value::Null,
            adapter,
            closed: false,
            attach: None,
            ready: false,
        })
    }

    fn write(&mut self, v: &Value) {
        let body = v.to_string();
        crate::lsp::log(">> dap", &body);
        let msg = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        if self
            .writer
            .write_all(msg.as_bytes())
            .and_then(|_| self.writer.flush())
            .is_err()
        {
            self.closed = true;
        }
    }

    pub fn request(&mut self, command: &str, args: Value, req: Req) -> i64 {
        let seq = self.seq;
        self.seq += 1;
        self.pending.insert(seq, req);
        self.write(&json!({ "seq": seq, "type": "request", "command": command, "arguments": args }));
        seq
    }

    pub fn respond_error(&mut self, request_seq: i64, command: &str, message: &str) {
        let seq = self.seq;
        self.seq += 1;
        self.write(&json!({
            "seq": seq,
            "type": "response",
            "request_seq": request_seq,
            "success": false,
            "command": command,
            "message": message,
        }));
    }

    pub fn respond(&mut self, request_seq: i64, command: &str, body: Value) {
        let seq = self.seq;
        self.seq += 1;
        self.write(&json!({
            "seq": seq,
            "type": "response",
            "request_seq": request_seq,
            "success": true,
            "command": command,
            "body": body,
        }));
    }

    pub fn poll(&mut self) -> Vec<DapMsg> {
        let mut out = Vec::new();
        while let Ok(raw) = self.rx.try_recv() {
            match raw {
                Raw::Eof => {
                    self.closed = true;
                    out.push(DapMsg::Closed);
                }
                Raw::Msg(v) => match v["type"].as_str() {
                    Some("response") => {
                        let seq = v["request_seq"].as_i64().unwrap_or(-1);
                        let req = self.pending.remove(&seq).unwrap_or(Req::Other);
                        out.push(DapMsg::Response {
                            req,
                            success: v["success"].as_bool().unwrap_or(false),
                            message: v["message"]
                                .as_str()
                                .or_else(|| v["body"]["error"]["format"].as_str())
                                .unwrap_or("")
                                .to_string(),
                            body: v["body"].clone(),
                        });
                    }
                    Some("event") => out.push(DapMsg::Event {
                        event: v["event"].as_str().unwrap_or("").to_string(),
                        body: v["body"].clone(),
                    }),
                    Some("request") => out.push(DapMsg::Request {
                        seq: v["seq"].as_i64().unwrap_or(0),
                        command: v["command"].as_str().unwrap_or("").to_string(),
                        args: v["arguments"].clone(),
                    }),
                    _ => {}
                },
            }
        }
        out
    }

    pub fn kill(&mut self) {
        if let Some(c) = self.child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.closed = true;
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            let _ = c.kill();
        }
    }
}

fn gdb_has_dap() -> bool {
    let Ok(o) = Command::new("gdb").arg("--version").stdin(Stdio::null()).output() else {
        return false;
    };
    let s = String::from_utf8_lossy(&o.stdout);
    let first = s.lines().next().unwrap_or("");
    first
        .split_whitespace()
        .filter_map(|w| w.split('.').next().and_then(|m| m.parse::<u32>().ok()))
        .next_back()
        .is_some_and(|major| major >= 14)
}

fn quiet(p: &Path, args: &[&str]) -> bool {
    Command::new(p)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The project's own Python first (`.venv`, `venv`, `$VIRTUAL_ENV`), so the
/// program runs with its dependencies, then `python3` on PATH.
fn find_python(root: &Path) -> Option<PathBuf> {
    for n in ["python", "python3"] {
        if let Some(p) = crate::lsp::find_exe(n, &[root]) {
            let in_venv = p.components().any(|c| {
                let c = c.as_os_str().to_string_lossy();
                c == ".venv" || c == "venv"
            }) || std::env::var_os("VIRTUAL_ENV").is_some_and(|v| p.starts_with(v));
            if in_venv {
                return Some(p);
            }
        }
    }
    ["python3", "python", "py"].iter().filter_map(|n| which(n)).find(|p| {
        quiet(
            p,
            &["-c", "import sys; sys.exit(0 if sys.version_info >= (3, 8) else 1)"],
        )
    })
}

fn lldb_dap() -> Option<PathBuf> {
    let names = ["lldb-dap", "lldb-vscode"];
    if let Some(p) = names.iter().find_map(|n| which(n)) {
        return Some(p);
    }
    for v in (14..=24).rev() {
        if let Some(p) = which(&format!("lldb-dap-{}", v)) {
            return Some(p);
        }
    }
    if cfg!(target_os = "macos") {
        let o = Command::new("xcrun").args(["-f", "lldb-dap"]).output().ok()?;
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if o.status.success() && !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    None
}

fn adapter(kind: Kind, name: &str, cmd: Vec<String>) -> Adapter {
    Adapter {
        kind,
        name: name.into(),
        cmd,
        env: Vec::new(),
    }
}

pub fn codelldb_adapter() -> Option<Adapter> {
    tools::codelldb_path()
        .or_else(|| which("codelldb"))
        .map(|p| adapter(Kind::CodeLldb, "codelldb", vec![p.display().to_string()]))
}

fn lldb_adapter() -> Option<Adapter> {
    lldb_dap().map(|p| adapter(Kind::Lldb, "lldb-dap", vec![p.display().to_string()]))
}

fn gdb_adapter() -> Option<Adapter> {
    if !gdb_has_dap() {
        return None;
    }
    which("gdb").map(|p| {
        adapter(
            Kind::Gdb,
            "gdb",
            vec![p.display().to_string(), "-i=dap".into(), "-q".into()],
        )
    })
}

pub fn native_adapter(prefer: Option<Kind>, allow_install: bool) -> Result<Adapter, PlanErr> {
    let found = match prefer {
        Some(Kind::Gdb) => gdb_adapter().or_else(lldb_adapter).or_else(codelldb_adapter),
        Some(Kind::Lldb) => lldb_adapter().or_else(codelldb_adapter).or_else(gdb_adapter),
        Some(Kind::CodeLldb) if allow_install => codelldb_adapter(),
        _ => codelldb_adapter().or_else(lldb_adapter).or_else(gdb_adapter),
    };
    match found {
        Some(a) => Ok(a),
        None if allow_install => Err(PlanErr::Missing(Tool::CodeLldb)),
        None => codelldb_adapter()
            .or_else(lldb_adapter)
            .or_else(gdb_adapter)
            .ok_or_else(|| PlanErr::Other("no native debug adapter available".into())),
    }
}

pub fn python_adapter(root: &Path) -> Result<Adapter, PlanErr> {
    let py = find_python(root).ok_or("Python 3 was not found — install it from https://www.python.org/downloads")?;
    let cmd = vec![py.display().to_string(), "-m".into(), "debugpy.adapter".into()];
    if quiet(&py, &["-c", "import debugpy"]) {
        return Ok(adapter(Kind::Debugpy, "debugpy", cmd));
    }
    let Some(dir) = tools::debugpy_dir() else {
        return Err(PlanErr::Missing(Tool::Debugpy));
    };
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut pp = dir.display().to_string();
    if let Ok(old) = std::env::var("PYTHONPATH") {
        if !old.is_empty() {
            pp = format!("{}{}{}", pp, sep, old);
        }
    }
    let mut a = adapter(Kind::Debugpy, "debugpy", cmd);
    a.env.push(("PYTHONPATH".into(), pp));
    Ok(a)
}

pub fn go_adapter() -> Result<Adapter, PlanErr> {
    which("dlv")
        .or_else(tools::dlv_path)
        .map(|p| adapter(Kind::Delve, "dlv", vec![p.display().to_string(), "dap".into()]))
        .ok_or(PlanErr::Missing(Tool::Delve))
}

#[derive(Clone, Debug)]
pub struct LaunchPlan {
    pub adapter: Adapter,
    pub args: Value,
    pub cwd: PathBuf,
    pub cargo: Option<Vec<String>>,
    /// The request sent after `initialize`: `launch` or `attach`.
    pub request: String,
    /// A debug server already listening (debugpy's `connect`, a remote
    /// dlv): ren connects to it instead of starting the adapter.
    pub connect: Option<(String, u16)>,
    /// From launch.json: its `preLaunchTask`, `postDebugTask`, `serverReadyAction`.
    pub pre_task: Option<String>,
    pub post_task: Option<String>,
    pub server_ready: Option<Value>,
    /// ren's own plan (no launch.json): ren's own defaults apply.
    pub auto: bool,
}

impl LaunchPlan {
    fn auto(adapter: Adapter, args: Value, cwd: PathBuf, cargo: Option<Vec<String>>) -> Self {
        LaunchPlan {
            adapter,
            args,
            cwd,
            cargo,
            request: "launch".into(),
            connect: None,
            pre_task: None,
            post_task: None,
            server_ready: None,
            auto: true,
        }
    }
}

pub fn launch_args(kind: Kind, program: &str, args: &[String], cwd: &Path, rust: bool) -> Value {
    let cwd = cwd.display().to_string();
    match kind {
        Kind::CodeLldb => {
            let mut v = json!({ "type": "lldb", "request": "launch", "program": program, "args": args, "cwd": cwd, "terminal": "console" });
            if rust {
                v["sourceLanguages"] = json!(["rust"]);
            }
            v
        }
        Kind::Lldb => json!({ "program": program, "args": args, "cwd": cwd, "stopOnEntry": false }),
        Kind::Gdb => json!({ "program": program, "args": args, "cwd": cwd, "stopAtBeginningOfMainSubprogram": true }),
        Kind::Debugpy => json!({
            "type": "python",
            "request": "launch",
            "program": program,
            "args": args,
            "cwd": cwd,
            "console": "internalConsole",
            "redirectOutput": true,
            "justMyCode": true,
        }),
        Kind::Delve => json!({ "mode": "debug", "program": program, "args": args, "cwd": cwd }),
    }
}

/// The configurations of `.vscode/launch.json` that ren runs, in the order
/// VS Code lists them (`launch.rs`).
pub fn launch_configs(root: &Path) -> Vec<String> {
    match crate::launch::read_launch(root) {
        Some(Ok(doc)) => crate::launch::config_names(&doc),
        _ => Vec::new(),
    }
}

/// The plan for the configuration called `name` (or the first ren runs),
/// resolved as VS Code and the debugger's extension would (`launch.rs`).
/// None when there's no launch.json or nothing in it ren runs.
pub fn from_launch_json(
    ctx: &crate::launch::Ctx,
    allow_install: bool,
    name: Option<&str>,
) -> Option<Result<LaunchPlan, PlanErr>> {
    use crate::launch::Subst;
    let doc = match crate::launch::read_launch(&ctx.root)? {
        Ok(d) => d,
        Err(e) => return Some(Err(e.into())),
    };
    let raw = crate::launch::config(&doc, name)?;
    let cfg = match crate::launch::substitute(&raw, ctx, &doc["inputs"]) {
        Ok(v) => v,
        Err(Subst::Ask(a)) => return Some(Err(PlanErr::Ask(a))),
        Err(Subst::Error(e)) => return Some(Err(e.into())),
    };
    Some(plan_for(cfg, ctx, allow_install))
}

/// debugpy's adapter, run by `py` (the configuration's `debugAdapterPython`)
/// when given, with ren's downloaded debugpy when that Python has none.
fn python_adapter_with(
    root: &Path,
    py: Option<&str>,
    adapter_path: Option<&str>,
    log: bool,
) -> Result<Adapter, PlanErr> {
    let Some(py) = py.map(PathBuf::from).filter(|p| p.is_file()) else {
        return python_adapter(root);
    };
    let mut cmd = vec![py.display().to_string()];
    match adapter_path {
        Some(p) => cmd.push(p.to_string()),
        None => cmd.extend(["-m".to_string(), "debugpy.adapter".to_string()]),
    }
    if log {
        if let Some(d) = crate::settings::dir() {
            cmd.extend(["--log-dir".to_string(), d.join("logs").display().to_string()]);
        }
    }
    let mut a = adapter(Kind::Debugpy, "debugpy", cmd);
    if adapter_path.is_none() && !quiet(&py, &["-c", "import debugpy"]) {
        let Some(dir) = tools::debugpy_dir() else {
            return Err(PlanErr::Missing(Tool::Debugpy));
        };
        let sep = if cfg!(windows) { ";" } else { ":" };
        let mut pp = dir.display().to_string();
        if let Ok(old) = std::env::var("PYTHONPATH") {
            if !old.is_empty() {
                pp = format!("{}{}{}", pp, sep, old);
            }
        }
        a.env.push(("PYTHONPATH".into(), pp));
    }
    Ok(a)
}

fn plan_for(cfg: Value, ctx: &crate::launch::Ctx, allow_install: bool) -> Result<LaunchPlan, PlanErr> {
    use crate::launch;
    let root = ctx.root.as_path();
    let mut cfg: launch::Obj = cfg.as_object().cloned().unwrap_or_default();
    let ty = cfg.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    let request = match cfg.get("request").and_then(Value::as_str) {
        Some("attach") => "attach",
        _ => "launch",
    }
    .to_string();
    let text = |cfg: &launch::Obj, k: &str| cfg.get(k).and_then(Value::as_str).map(str::to_string);
    let on = |cfg: &launch::Obj, k: &str| cfg.get(k).and_then(Value::as_bool).unwrap_or(false);
    let pre_task = text(&cfg, "preLaunchTask");
    let post_task = text(&cfg, "postDebugTask");
    let server_ready = cfg.get("serverReadyAction").cloned();
    let process: Vec<(String, String)> = std::env::vars().collect();
    let mut connect = None;
    let mut cargo = None;
    let mut cwd = root.to_path_buf();
    let adapter = match ty.as_str() {
        "debugpy" | "python" => {
            let interp = ctx
                .python
                .clone()
                .or_else(|| find_python(root).map(|p| p.display().to_string()))
                .unwrap_or_else(|| "python3".into());
            if request == "attach" {
                launch::python_attach(&mut cfg, root);
                // `connect` (or the older `host` / `port`): debugpy is already
                // listening there, so that's the adapter.
                let port = cfg["connect"]["port"]
                    .as_u64()
                    .or(cfg.get("port").and_then(Value::as_u64));
                match port {
                    Some(port) => {
                        let host = match cfg.get("connect") {
                            Some(c) => c["host"].as_str(),
                            None => cfg.get("host").and_then(Value::as_str),
                        }
                        .unwrap_or("127.0.0.1")
                        .to_string();
                        connect = Some((host, port as u16));
                        adapter(Kind::Debugpy, "debugpy", Vec::new())
                    }
                    None => python_adapter_with(
                        root,
                        Some(&interp),
                        text(&cfg, "debugAdapterPath").as_deref(),
                        on(&cfg, "logToFile"),
                    )?,
                }
            } else {
                launch::python_launch(&mut cfg, root, &interp, &process);
                python_adapter_with(
                    root,
                    cfg.get("debugAdapterPython").and_then(Value::as_str),
                    text(&cfg, "debugAdapterPath").as_deref(),
                    on(&cfg, "logToFile"),
                )?
            }
        }
        "lldb" | "codelldb" => {
            launch::codelldb(&mut cfg, root);
            if let Some(a) = cfg.remove("cargo") {
                let args: Vec<String> = a["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect();
                let mut langs = cfg
                    .get("sourceLanguages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if !langs.iter().any(|l| l == "rust") {
                    langs.push(json!("rust"));
                }
                cfg.insert("sourceLanguages".into(), Value::Array(langs));
                cargo = Some(args);
            }
            native_adapter(Some(Kind::CodeLldb), allow_install)?
        }
        "lldb-dap" | "lldb-vscode" => native_adapter(Some(Kind::Lldb), allow_install)?,
        "gdb" => native_adapter(Some(Kind::Gdb), allow_install)?,
        "cppdbg" => {
            let (lldb, mut args, gdb_extra) = launch::cppdbg(&cfg, root, &process)?;
            let mut a = native_adapter(Some(if lldb { Kind::Lldb } else { Kind::Gdb }), allow_install)?;
            if a.kind == Kind::Gdb {
                a.cmd.extend(gdb_extra);
            }
            for k in ["name", "type", "request"] {
                if let Some(v) = cfg.get(k) {
                    args.insert(k.into(), v.clone());
                }
            }
            cfg = args;
            a
        }
        "go" => {
            let extra = launch::go(&mut cfg, root, ctx.file.as_deref())?;
            let mut a = go_adapter()?;
            let at = a.cmd.len().min(2);
            a.cmd.splice(at..at, extra.dlv_args);
            if let Some(d) = extra.build_dir {
                cwd = d;
            }
            connect = extra.connect;
            a
        }
        other => return Err(format!("ren can't debug type \"{}\"", other).into()),
    };
    Ok(LaunchPlan {
        adapter,
        args: Value::Object(cfg),
        cwd,
        cargo,
        request,
        connect,
        pre_task,
        post_task,
        server_ready,
        auto: false,
    })
}

pub fn auto_plan(
    lang: &str,
    file: &Path,
    workspace: &Path,
    program: Option<&str>,
    args: &[String],
    allow_install: bool,
) -> Result<LaunchPlan, PlanErr> {
    let rust = lang == "Rust";
    match lang {
        "Python" => {
            let cwd = find_root("Python", file, workspace);
            let a = python_adapter(&cwd)?;
            let prog = program
                .map(|p| p.to_string())
                .unwrap_or_else(|| file.display().to_string());
            Ok(LaunchPlan::auto(
                a.clone(),
                launch_args(a.kind, &prog, args, &cwd, false),
                cwd,
                None,
            ))
        }
        "Go" => {
            let a = go_adapter()?;
            let cwd = find_root("Go", file, workspace);
            let prog = program
                .map(|p| p.to_string())
                .unwrap_or_else(|| file.parent().unwrap_or(&cwd).display().to_string());
            Ok(LaunchPlan::auto(
                a.clone(),
                launch_args(a.kind, &prog, args, &cwd, false),
                cwd,
                None,
            ))
        }
        "Rust" | "C/C++" => {
            let prefer = if rust { Some(Kind::CodeLldb) } else { None };
            let a = native_adapter(prefer, allow_install)?;
            let cwd = find_root(lang, file, workspace);
            match program {
                Some(p) => Ok(LaunchPlan::auto(
                    a.clone(),
                    launch_args(a.kind, p, args, &cwd, rust),
                    cwd,
                    None,
                )),
                None if rust => Ok(LaunchPlan::auto(
                    a.clone(),
                    launch_args(a.kind, "", args, &cwd, true),
                    cwd,
                    Some(vec!["build".into()]),
                )),
                None => Err("set the program to debug: :debug <program> [args]".into()),
            }
        }
        _ => Err(format!("debugging is not supported for {}", lang).into()),
    }
}

pub enum BuildResult {
    Line(String),
    Ok(String),
    Err(String),
}

pub fn cargo_build(cwd: PathBuf, mut args: Vec<String>, current: Option<PathBuf>) -> Receiver<BuildResult> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        if !args.iter().any(|a| a.starts_with("--message-format")) {
            args.push("--message-format=json-render-diagnostics".into());
        }
        let mut c = Command::new("cargo");
        c.args(&args)
            .current_dir(&cwd)
            .env("CARGO_TERM_COLOR", "never")
            .env("CARGO_TERM_PROGRESS_WHEN", "never")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        detach(&mut c);
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                let _ = tx.send(BuildResult::Err(format!("cargo: {}", e)));
                return;
            }
        };
        let stderr = child.stderr.take();
        let etx = tx.clone();
        let err_thread = thread::spawn(move || {
            let mut tail: Vec<String> = Vec::new();
            if let Some(e) = stderr {
                for line in BufReader::new(e).lines().map_while(Result::ok) {
                    let t = line.trim();
                    if t.starts_with("Compiling")
                        || t.starts_with("Blocking")
                        || t.starts_with("Finished")
                        || t.starts_with("Updating")
                        || t.starts_with("Downloading")
                        || t.starts_with("Downloaded")
                    {
                        let _ = etx.send(BuildResult::Line(t.to_string()));
                    }
                    tail.push(line);
                    if tail.len() > 60 {
                        tail.remove(0);
                    }
                }
            }
            tail
        });
        let mut exes: Vec<(String, String)> = Vec::new();
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if v["reason"] != "compiler-artifact" {
                    continue;
                }
                let is_bin = v["target"]["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|x| x == "bin"));
                if let (true, Some(exe)) = (is_bin, v["executable"].as_str()) {
                    exes.push((
                        exe.to_string(),
                        v["target"]["src_path"].as_str().unwrap_or("").to_string(),
                    ));
                }
            }
        }
        let status = child.wait();
        let tail = err_thread.join().unwrap_or_default();
        let res = match status {
            Err(e) => BuildResult::Err(format!("cargo: {}", e)),
            Ok(st) if !st.success() => BuildResult::Err(tail.join("\n")),
            Ok(_) if exes.is_empty() => BuildResult::Err("cargo build produced no binary target".into()),
            Ok(_) => {
                let cur = current.map(|c| c.display().to_string());
                let pick = exes
                    .iter()
                    .find(|(_, src)| Some(src) == cur.as_ref())
                    .unwrap_or(&exes[0]);
                BuildResult::Ok(pick.0.clone())
            }
        };
        let _ = tx.send(res);
    });
    rx
}
