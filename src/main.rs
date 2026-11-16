mod buffer;
mod comments;
mod config;
mod containers;
mod dap;
mod editor;
mod fileops;
mod git;
mod highlight;
mod icons;
mod lsp;
mod markdown;
mod render;
mod screen;
mod settings;
mod term;
mod tools;
mod tree;

use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{self, DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen};
use editor::Editor;
use screen::Screen;
use std::io::{self, BufWriter, Write};
use std::time::Duration;

fn setup() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, DisableLineWrap)?;
    let _ = execute!(out, EnableBracketedPaste);
    // The kitty keyboard protocol lets Ctrl+Esc and Ctrl+` arrive as their own keys.
    if std::env::var_os("REN_NO_KITTY").is_none() && terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
        if execute!(out, PushKeyboardEnhancementFlags(flags)).is_ok() {
            editor::KITTY.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(())
}

fn restore() {
    let mut out = io::stdout();
    if editor::kitty() {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, DisableBracketedPaste);
    let _ = execute!(
        out,
        DisableMouseCapture,
        EnableLineWrap,
        SetCursorStyle::DefaultUserShape,
        Show,
        LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
    let _ = out.flush();
}

/// The read end of an empty pipe that takes the terminal's place on a hang-up.
#[cfg(unix)]
static HUP_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// SIGHUP (the terminal window closed) and SIGTERM set a flag: the loop sees
/// it, the session is saved and ren leaves.
#[cfg(unix)]
fn catch_hangup() {
    use std::sync::atomic::Ordering::Relaxed;
    extern "C" fn on_signal(_: libc::c_int) {
        editor::HANGUP.store(true, Relaxed);
        // crossterm keeps reading a hung-up terminal forever (it gets nothing,
        // never "would block"), so the loop would never see the flag. An
        // empty non-blocking pipe in its place makes the read give up.
        let fd = HUP_FD.load(Relaxed);
        if fd >= 0 {
            unsafe {
                libc::dup2(fd, libc::STDIN_FILENO);
            }
        }
    }
    unsafe {
        let mut p = [0 as libc::c_int; 2];
        if libc::pipe(p.as_mut_ptr()) == 0 {
            for fd in p {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            let fl = libc::fcntl(p[0], libc::F_GETFL);
            libc::fcntl(p[0], libc::F_SETFL, fl | libc::O_NONBLOCK);
            // The write end stays open, so reads say "nothing yet", not "end".
            HUP_FD.store(p[0], Relaxed);
        }
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGHUP, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

#[cfg(not(unix))]
fn catch_hangup() {}

fn run(ed: &mut Editor) -> io::Result<()> {
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout());
    let mut screen = Screen::new(ed.w, ed.h);
    let mut redraw = true;
    loop {
        if editor::hangup() {
            return Ok(());
        }
        if redraw {
            screen.resize(ed.w, ed.h);
            ed.draw(&mut screen);
            screen.flush(&mut out, ed.cursor_pos(), ed.bar_cursor())?;
            redraw = false;
        }
        if ed.quit {
            return Ok(());
        }
        if event::poll(Duration::from_millis(20))? {
            let ev = event::read()?;
            ed.burst = event::poll(Duration::ZERO)?;
            ed.handle(ev);
            while !ed.quit && event::poll(Duration::ZERO)? {
                let ev = event::read()?;
                ed.burst = true;
                ed.handle(ev);
            }
            ed.burst = false;
            redraw = true;
        }
        if ed.lsp_tick() {
            redraw = true;
        }
        if ed.debug_tick() {
            redraw = true;
        }
        if ed.panel_tick() {
            redraw = true;
        }
        if ed.git_tick() {
            redraw = true;
        }
        ed.autosave_tick();
    }
}

fn main() -> io::Result<()> {
    screen::detect_colors();
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("-h") | Some("--help") => {
            println!("ren {}", env!("CARGO_PKG_VERSION"));
            println!("usage: ren [file | folder]");
            println!("       ren name.workspace.json  open the folders it lists ({{\"open\": [\"~/a\", \"/b\"]}})");
            println!("                                as that workspace was left (tabs, unsaved text)");
            println!("       ren --resume [n | n.k]   pick a saved session (or restore session n,");
            println!("                                n.k: its checkpoint k steps back)");
            println!("       ren --init   create ~/.ren/settings.json if missing");
            println!("Type :help inside the editor for all shortcuts.");
            return Ok(());
        }
        Some("--init") => {
            match settings::ensure() {
                Ok(Some(p)) => println!("created {}", p.display()),
                Ok(None) => {
                    if let Some(p) = settings::path() {
                        println!("settings already exist: {}", p.display());
                    }
                }
                Err(e) => {
                    eprintln!("could not create settings: {}", e);
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some("--restore") => {
            // After `:reload`: the session it saved.
            let id = std::env::args().nth(2).unwrap_or_default();
            let (w, h) = terminal::size()?;
            let mut ed = Editor::new(None, w, h);
            ed.reloaded(&id);
            return run_app(ed);
        }
        Some("--resume") | Some("-r") => {
            // The saved sessions, to pick one (also after a crash or a kill).
            let pick = std::env::args().nth(2);
            let (w, h) = terminal::size()?;
            let mut ed = Editor::new(None, w, h);
            ed.resume(pick.as_deref());
            return run_app(ed);
        }
        Some("--session") => {
            // `:reload` from an older ren, which kept one session per folder.
            let root = std::env::args().nth(2);
            let (w, h) = terminal::size()?;
            let mut ed = Editor::new(root, w, h);
            ed.restore_legacy();
            return run_app(ed);
        }
        Some("-V") | Some("--version") => {
            println!("ren {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }
    let (w, h) = terminal::size()?;
    if let Some(file) = arg.as_deref().map(std::path::Path::new) {
        if file.is_file() && editor::is_workspace_file(file) {
            let mut ed = Editor::new(None, w, h);
            ed.open_workspace(file);
            return run_app(ed);
        }
    }
    let mut ed = Editor::new(arg, w, h);
    ed.check_recovery();
    run_app(ed)
}

fn run_app(mut ed: Editor) -> io::Result<()> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        default_hook(info);
    }));

    catch_hangup();
    setup()?;
    let res = run(&mut ed);
    // The terminal went away (or wouldn't take output): keep everything,
    // unsaved text included, for the session list.
    let gone = res.is_err() || editor::hangup();
    if gone {
        ed.hangup_exit();
    } else {
        ed.clean_exit();
    }
    ed.stop_servers();
    restore();
    if ed.reload && res.is_ok() {
        return restart(ed.session_id());
    }
    res
}

/// `:reload`: replaces this process with a fresh ren that restores the session.
fn restart(id: &str) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--restore").arg(id);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = cmd.exec();
        Err(e)
    }
    #[cfg(not(unix))]
    {
        let status = cmd.status()?;
        std::process::exit(status.code().unwrap_or(0));
    }
}
