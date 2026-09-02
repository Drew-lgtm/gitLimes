//! Sends long output through a pager, the way git does.
//!
//! Without this, `gitlimes log` on a large repository dumps thousands of lines
//! past the top of the terminal. With it, output is scrollable and quitting the
//! pager early stops the work - the write fails with `BrokenPipe`, which the
//! binary already treats as a clean exit.
//!
//! Paging never changes what is written, only where it goes, and it degrades
//! quietly: no tty, no pager on `PATH`, or a pager that refuses to start all
//! fall back to plain stdout rather than failing the command.

use std::io::{self, IsTerminal, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};

/// Whether to page, before the environment gets a say.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Page when stdout is a terminal and a pager can be found.
    Auto,
    /// Always page, even off a terminal. Mainly so the path is testable.
    Always,
    Never,
}

static MODE: AtomicU8 = AtomicU8::new(0);

/// Records the mode once at startup, mirroring [`crate::style::init`], so the
/// commands do not each have to thread it through.
pub fn init(mode: Mode) {
    MODE.store(
        match mode {
            Mode::Auto => 0,
            Mode::Always => 1,
            Mode::Never => 2,
        },
        Ordering::Relaxed,
    );
}

/// The output sink for a command, honouring the mode set by [`init`].
pub fn out() -> Out {
    Out::new(match MODE.load(Ordering::Relaxed) {
        1 => Mode::Always,
        2 => Mode::Never,
        _ => Mode::Auto,
    })
}

/// A writer that is either stdout or a pager's stdin.
pub struct Out {
    writer: Option<Box<dyn Write>>,
    child: Option<Child>,
}

impl Out {
    pub fn new(mode: Mode) -> Out {
        let wanted = match mode {
            Mode::Never => false,
            Mode::Always => true,
            Mode::Auto => io::stdout().is_terminal(),
        };
        if wanted {
            if let Some(out) = Out::spawn() {
                return out;
            }
        }
        Out {
            writer: Some(Box::new(io::BufWriter::with_capacity(
                32 * 1024,
                io::stdout(),
            ))),
            child: None,
        }
    }

    fn spawn() -> Option<Out> {
        let (program, args) = pager_command()?;
        let mut cmd = Command::new(&program);
        cmd.args(&args).stdin(Stdio::piped());
        // less needs to keep its own terminal for output and key handling.
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        if program.contains("less") && std::env::var_os("LESS").is_none() {
            // Quit if it fits on one screen, keep colour, do not clear on exit.
            cmd.env("LESS", "FRX");
        }
        let mut child = cmd.spawn().ok()?;
        let stdin = child.stdin.take()?;
        Some(Out {
            writer: Some(Box::new(io::BufWriter::with_capacity(32 * 1024, stdin))),
            child: Some(child),
        })
    }

    /// Flushes, closes the pager's stdin and waits for it to exit.
    ///
    /// Dropping the writer first is what sends EOF; without it the pager would
    /// sit waiting for input that never comes.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some(mut w) = self.writer.take() {
            w.flush()?;
        }
        if let Some(mut child) = self.child.take() {
            child.wait()?;
        }
        Ok(())
    }
}

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.writer.as_mut() {
            Some(w) => w.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.writer.as_mut() {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        // Only runs on an error path; the success path goes through finish().
        drop(self.writer.take());
        if let Some(child) = self.child.as_mut() {
            let _ = child.wait();
        }
    }
}

/// Resolves the pager, honouring `GITLIMES_PAGER` then `PAGER`, and falling
/// back to `less`. An explicit empty value means "no pager", matching how git
/// and most tools read `PAGER=`.
fn pager_command() -> Option<(String, Vec<String>)> {
    let configured = std::env::var("GITLIMES_PAGER")
        .or_else(|_| std::env::var("PAGER"))
        .ok();

    let line = match configured {
        Some(s) if s.trim().is_empty() => return None,
        Some(s) => s,
        None => "less".to_string(),
    };

    // A pager setting may carry arguments, e.g. "less -S".
    let mut parts = line.split_whitespace().map(str::to_string);
    let program = parts.next()?;
    Some((program, parts.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises the env mutations below; these tests share process state.
    fn with_pager_env<T>(gitlimes: Option<&str>, pager: Option<&str>, f: impl FnOnce() -> T) -> T {
        // Safety: the test harness is multi-threaded, so these are guarded by a
        // mutex to keep one test's environment out of another's.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old_g = std::env::var_os("GITLIMES_PAGER");
        let old_p = std::env::var_os("PAGER");
        unsafe {
            match gitlimes {
                Some(v) => std::env::set_var("GITLIMES_PAGER", v),
                None => std::env::remove_var("GITLIMES_PAGER"),
            }
            match pager {
                Some(v) => std::env::set_var("PAGER", v),
                None => std::env::remove_var("PAGER"),
            }
        }
        let out = f();
        unsafe {
            match old_g {
                Some(v) => std::env::set_var("GITLIMES_PAGER", v),
                None => std::env::remove_var("GITLIMES_PAGER"),
            }
            match old_p {
                Some(v) => std::env::set_var("PAGER", v),
                None => std::env::remove_var("PAGER"),
            }
        }
        out
    }

    #[test]
    fn defaults_to_less() {
        let (p, args) = with_pager_env(None, None, pager_command).expect("a default pager");
        assert_eq!(p, "less");
        assert!(args.is_empty());
    }

    #[test]
    fn gitlimes_pager_wins_over_pager() {
        let (p, _) = with_pager_env(Some("mypager"), Some("other"), pager_command).unwrap();
        assert_eq!(p, "mypager");
    }

    #[test]
    fn a_pager_setting_may_carry_arguments() {
        let (p, args) = with_pager_env(Some("less -S -N"), None, pager_command).unwrap();
        assert_eq!(p, "less");
        assert_eq!(args, vec!["-S", "-N"]);
    }

    #[test]
    fn an_empty_setting_disables_paging() {
        assert!(with_pager_env(Some(""), None, pager_command).is_none());
        assert!(with_pager_env(Some("   "), None, pager_command).is_none());
        assert!(with_pager_env(None, Some(""), pager_command).is_none());
    }

    #[test]
    fn never_writes_straight_to_stdout() {
        let out = Out::new(Mode::Never);
        assert!(out.child.is_none(), "Mode::Never must not spawn anything");
    }
}
