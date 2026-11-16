//! Embedded terminal for the bottom panel: a shell (or any command, such as
//! `docker exec -it`) on a pseudo-terminal, with the screen kept by `vt100`.
//! Unix only; on Windows `Session::spawn` reports that it isn't supported.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io;
use std::path::Path;

/// Bytes a key sends to a terminal program.
pub fn key_bytes(k: &KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let m = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
    let mut out = Vec::new();
    let csi = |fin: char| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[1;{}{}", m, fin).into_bytes()
        } else if app_cursor {
            format!("\x1bO{}", fin).into_bytes()
        } else {
            format!("\x1b[{}", fin).into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[{};{}~", n, m).into_bytes()
        } else {
            format!("\x1b[{}~", n).into_bytes()
        }
    };
    match k.code {
        KeyCode::Char(c) => {
            if alt {
                out.push(27);
            }
            if ctrl {
                let b = match c {
                    'a'..='z' => c as u8 - b'a' + 1,
                    'A'..='Z' => c as u8 - b'A' + 1,
                    ' ' | '@' | '2' => 0,
                    '[' | '3' => 27,
                    '\\' | '4' => 28,
                    ']' | '5' => 29,
                    '^' | '6' => 30,
                    '_' | '-' | '7' => 31,
                    '?' | '8' => 127,
                    _ => return None,
                };
                out.push(b);
            } else {
                let mut buf = [0u8; 4];
                out.extend(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        KeyCode::Null => out.push(0),
        KeyCode::Enter => {
            if alt {
                out.push(27);
            }
            out.push(b'\r');
        }
        KeyCode::Tab => out.push(b'\t'),
        KeyCode::BackTab => out.extend(b"\x1b[Z"),
        KeyCode::Backspace => {
            if alt || ctrl {
                out.push(27);
            }
            out.push(0x7f);
        }
        KeyCode::Esc => out.push(27),
        KeyCode::Up => out = csi('A'),
        KeyCode::Down => out = csi('B'),
        KeyCode::Right => out = csi('C'),
        KeyCode::Left => out = csi('D'),
        KeyCode::Home => out = csi('H'),
        KeyCode::End => out = csi('F'),
        KeyCode::Insert => out = tilde(2),
        KeyCode::Delete => out = tilde(3),
        KeyCode::PageUp => out = tilde(5),
        KeyCode::PageDown => out = tilde(6),
        KeyCode::F(n @ 1..=4) => out = format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes(),
        KeyCode::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            out = tilde(code);
        }
        _ => return None,
    }
    Some(out)
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{channel, Receiver};
    use std::thread;

    const SCROLLBACK: usize = 5000;

    pub struct Session {
        pub title: String,
        parser: vt100::Parser,
        master: File,
        child: Child,
        rx: Receiver<Vec<u8>>,
        pub exit: Option<i32>,
        size: (u16, u16),
    }

    fn set_cloexec(fd: libc::c_int) {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }

    impl Session {
        /// Runs `cmd` (the first word is the program) on a new pseudo-terminal.
        pub fn spawn(
            cmd: &[String],
            cwd: &Path,
            env: &[(String, String)],
            rows: u16,
            cols: u16,
            title: &str,
        ) -> io::Result<Session> {
            let (rows, cols) = (rows.max(2), cols.max(10));
            let mut master: libc::c_int = 0;
            let mut slave: libc::c_int = 0;
            let mut ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let r = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    // A raw pointer: Linux takes `*const winsize`, macOS `*mut`.
                    &raw mut ws,
                )
            };
            if r != 0 {
                return Err(io::Error::last_os_error());
            }
            set_cloexec(master);
            let slave_file = unsafe { File::from_raw_fd(slave) };
            let master_file = unsafe { File::from_raw_fd(master) };
            let mut c = Command::new(&cmd[0]);
            c.args(&cmd[1..])
                .current_dir(cwd)
                .env("TERM", "xterm-256color")
                .env("COLORTERM", "truecolor")
                .env("TERM_PROGRAM", "ren")
                .stdin(Stdio::from(slave_file.try_clone()?))
                .stdout(Stdio::from(slave_file.try_clone()?))
                .stderr(Stdio::from(slave_file));
            for (k, v) in env {
                c.env(k, v);
            }
            unsafe {
                c.pre_exec(|| {
                    if libc::setsid() < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let child = c.spawn()?;
            drop(c);
            let mut reader = master_file.try_clone()?;
            let (tx, rx) = channel();
            thread::spawn(move || {
                let mut buf = [0u8; 16384];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
            Ok(Session {
                title: title.to_string(),
                parser: vt100::Parser::new(rows, cols, SCROLLBACK),
                master: master_file,
                child,
                rx,
                exit: None,
                size: (rows, cols),
            })
        }

        /// Feeds pending output into the screen. True when something changed.
        pub fn poll(&mut self) -> bool {
            let mut changed = false;
            while let Ok(b) = self.rx.try_recv() {
                self.parser.process(&b);
                changed = true;
            }
            if self.exit.is_none() {
                if let Ok(Some(st)) = self.child.try_wait() {
                    self.exit = Some(st.code().unwrap_or(-1));
                    changed = true;
                }
            }
            changed
        }

        pub fn alive(&self) -> bool {
            self.exit.is_none()
        }

        pub fn resize(&mut self, rows: u16, cols: u16) {
            let (rows, cols) = (rows.max(2), cols.max(10));
            if (rows, cols) == self.size {
                return;
            }
            self.size = (rows, cols);
            self.parser.set_size(rows, cols);
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            use std::os::fd::AsRawFd;
            unsafe {
                libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws);
            }
        }

        pub fn write(&mut self, bytes: &[u8]) {
            if self.exit.is_none() {
                let _ = self.master.write_all(bytes);
                let _ = self.master.flush();
            }
            if self.parser.screen().scrollback() > 0 {
                self.parser.set_scrollback(0);
            }
        }

        pub fn key(&mut self, k: &KeyEvent) {
            let app = self.parser.screen().application_cursor();
            if let Some(b) = key_bytes(k, app) {
                self.write(&b);
            }
        }

        pub fn paste(&mut self, s: &str) {
            if self.parser.screen().bracketed_paste() {
                let mut b = b"\x1b[200~".to_vec();
                b.extend(s.as_bytes());
                b.extend(b"\x1b[201~");
                self.write(&b);
            } else {
                self.write(s.replace('\n', "\r").as_bytes());
            }
        }

        /// Scrolls the view into the scrollback (positive = older output).
        pub fn scroll(&mut self, d: isize) {
            let cur = self.parser.screen().scrollback() as isize;
            self.parser.set_scrollback((cur + d).max(0) as usize);
        }

        pub fn screen(&self) -> &vt100::Screen {
            self.parser.screen()
        }

        /// The tab label: the program's title if it set one.
        pub fn label(&self) -> String {
            let t = self.parser.screen().title();
            if t.is_empty() {
                self.title.clone()
            } else {
                t.chars().take(24).collect()
            }
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            if self.exit.is_none() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

#[cfg(unix)]
pub use imp::Session;

#[cfg(not(unix))]
pub struct Session;

#[cfg(not(unix))]
impl Session {
    pub fn spawn(_: &[String], _: &Path, _: &[(String, String)], _: u16, _: u16, _: &str) -> io::Result<Session> {
        Err(io::Error::other(
            "the embedded terminal is not supported on Windows yet",
        ))
    }
}

/// The user's shell, started as a login shell so PATH from the shell config applies.
pub fn default_shell() -> Vec<String> {
    let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    vec![sh, "-l".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn keys() {
        let n = KeyModifiers::NONE;
        assert_eq!(key_bytes(&k(KeyCode::Char('a'), n), false).unwrap(), b"a");
        assert_eq!(
            key_bytes(&k(KeyCode::Char('c'), KeyModifiers::CONTROL), false).unwrap(),
            vec![3]
        );
        assert_eq!(key_bytes(&k(KeyCode::Up, n), false).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes(&k(KeyCode::Up, n), true).unwrap(), b"\x1bOA");
        assert_eq!(
            key_bytes(&k(KeyCode::Left, KeyModifiers::CONTROL), false).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(key_bytes(&k(KeyCode::Enter, n), false).unwrap(), b"\r");
        assert_eq!(key_bytes(&k(KeyCode::F(5), n), false).unwrap(), b"\x1b[15~");
        assert_eq!(
            key_bytes(&k(KeyCode::Char('b'), KeyModifiers::ALT), false).unwrap(),
            b"\x1bb"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_command() {
        let mut s = Session::spawn(
            &["/bin/sh".into(), "-c".into(), "printf 'hi there'".into()],
            Path::new("/"),
            &[],
            5,
            40,
            "t",
        )
        .unwrap();
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 3 {
            s.poll();
            if s.screen().contents().contains("hi there") && !s.alive() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(s.screen().contents().contains("hi there"));
    }
}
