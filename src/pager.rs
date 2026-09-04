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
//!
//! The pager is spawned **lazily**, on the first byte written. A command that
//! fails before producing output - a bad revision, not a repository - therefore
//! never opens a pager over the top of git's error message.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};

/// Whether to page, before the environment gets a say.
#[derive(Clone, Copy, PartialEq, Debug)]
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

/// A writer that is either stdout or a pager's stdin, decided on first write.
pub struct Out {
    mode: Mode,
    writer: Option<Box<dyn Write>>,
    child: Option<Child>,
}

impl Out {
    pub fn new(mode: Mode) -> Out {
        Out {
            mode,
            writer: None,
            child: None,
        }
    }

    /// Decides where output goes, the first time there is any.
    fn start(&mut self) {
        if self.writer.is_some() {
            return;
        }
        let wanted = match self.mode {
            Mode::Never => false,
            Mode::Always => true,
            Mode::Auto => io::stdout().is_terminal(),
        };
        if wanted {
            if let Some((child, stdin)) = spawn_pager() {
                self.child = Some(child);
                self.writer = Some(Box::new(io::BufWriter::with_capacity(32 * 1024, stdin)));
                return;
            }
        }
        self.writer = Some(Box::new(io::BufWriter::with_capacity(
            32 * 1024,
            io::stdout(),
        )));
    }

    /// True once a destination has been chosen. Nothing written, nothing spawned.
    #[cfg(test)]
    fn started(&self) -> bool {
        self.writer.is_some()
    }

    /// Flushes, closes the pager's stdin and waits for it to exit.
    ///
    /// Dropping the writer first is what sends EOF; without it the pager would
    /// sit waiting for input that never comes.
    pub fn finish(mut self) -> io::Result<()> {
        let flushed = match self.writer.take() {
            Some(mut w) => w.flush(),
            // Nothing was ever written, so there is nothing to flush or close.
            None => Ok(()),
        };
        if let Some(mut child) = self.child.take() {
            // Wait even if the flush failed, so the pager is never left behind.
            let _ = child.wait();
        }
        flushed
    }
}

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.start();
        match self.writer.as_mut() {
            Some(w) => w.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.writer.as_mut() {
            // Never flushed into existence: a flush on an unused sink is a no-op.
            None => Ok(()),
            Some(w) => w.flush(),
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

/// Starts the configured pager, returning it and its stdin.
fn spawn_pager() -> Option<(Child, std::process::ChildStdin)> {
    let (program, args) = pager_command()?;
    let mut cmd = Command::new(&program);
    cmd.args(&args).stdin(Stdio::piped());
    // The pager needs its own terminal for output and key handling.
    cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    if program.contains("less") && std::env::var_os("LESS").is_none() {
        // Quit if it fits on one screen, keep colour, do not clear on exit.
        cmd.env("LESS", "FRX");
    }
    let mut child = cmd.spawn().ok()?;
    match child.stdin.take() {
        Some(stdin) => Some((child, stdin)),
        None => {
            // Unreachable with Stdio::piped(), but never leave an orphan behind.
            let _ = child.kill();
            let _ = child.wait();
            None
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
    split_pager(&line, |candidate| Path::new(candidate).is_file())
}

/// Splits a pager setting into a program and its arguments.
///
/// Splitting on whitespace alone breaks every pager whose path contains a
/// space, which on Windows is most of them. The mangled path then fails to
/// spawn and paging silently turns itself off, so the user sees no pager and no
/// reason why. The rules, in order:
///
/// 1. A double-quoted program comes first, so arguments can follow a path that
///    contains spaces.
/// 2. Otherwise the longest run of leading words that names a real file is the
///    program and the rest are its arguments. This rescues an unquoted spaced
///    path, with or without arguments after it.
/// 3. Otherwise the first word is the program, so `less -S` keeps working.
///
/// `exists` is injected so the rules can be tested without touching the disk.
fn split_pager(line: &str, exists: impl Fn(&str) -> bool) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    if let Some(rest) = line.strip_prefix('"') {
        let (program, args) = match rest.split_once('"') {
            Some((p, a)) => (p, a),
            // An unterminated quote: take the remainder as the whole program.
            None => (rest, ""),
        };
        if program.is_empty() {
            return None;
        }
        return Some((
            program.to_string(),
            args.split_whitespace().map(str::to_string).collect(),
        ));
    }

    // Longest prefix of words that names a real file wins, so an unquoted
    // `C:\Program Files\git\less.exe -S` finds the executable and keeps `-S`
    // as an argument. Without this the first space ends the program name.
    let tokens: Vec<&str> = line.split_whitespace().collect();
    for n in (1..=tokens.len()).rev() {
        let candidate = tokens[..n].join(" ");
        if exists(&candidate) {
            return Some((
                candidate,
                tokens[n..].iter().map(|s| s.to_string()).collect(),
            ));
        }
    }

    // Nothing on disk matched, so fall back to the plain reading: the first
    // word is the program. That is the right answer for `less -S`.
    let mut parts = tokens.into_iter().map(str::to_string);
    let program = parts.next()?;
    Some((program, parts.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pretends a path exists when it mentions "real" *and* looks like an
    /// executable. The second half matters: a fake that accepts any string
    /// containing "real" would also accept the path with its arguments still
    /// attached, and the prefix search would never be exercised.
    fn fake_exists(p: &str) -> bool {
        p.contains("real") && p.ends_with(".exe")
    }

    fn split(line: &str) -> Option<(String, Vec<String>)> {
        split_pager(line, fake_exists)
    }

    #[test]
    fn a_bare_program_takes_no_arguments() {
        let (p, args) = split("less").unwrap();
        assert_eq!(p, "less");
        assert!(args.is_empty());
    }

    #[test]
    fn arguments_still_split_on_whitespace() {
        let (p, args) = split("less -S -N").unwrap();
        assert_eq!(p, "less");
        assert_eq!(args, vec!["-S", "-N"]);
    }

    #[test]
    fn an_existing_path_with_spaces_keeps_its_arguments() {
        let (p, args) = split(r"C:\Program Files\real\less.exe -S").unwrap();
        assert_eq!(p, r"C:\Program Files\real\less.exe");
        assert_eq!(args, vec!["-S"], "arguments after a spaced path survive");
    }

    #[test]
    fn an_existing_path_with_spaces_is_not_chopped_into_arguments() {
        // The bug this rule exists for: C:\Program Files\... spawned as
        // program "C:\Program" with an argument, which fails, which silently
        // disables paging.
        let (p, args) = split(r"C:\Program Files\real\less.exe").unwrap();
        assert_eq!(p, r"C:\Program Files\real\less.exe");
        assert!(args.is_empty(), "the path must not become arguments");
    }

    #[test]
    fn a_quoted_program_may_still_take_arguments() {
        let (p, args) = split(r#""C:\Program Files\x\less.exe" -S"#).unwrap();
        assert_eq!(p, r"C:\Program Files\x\less.exe");
        assert_eq!(args, vec!["-S"]);
    }

    #[test]
    fn quoting_wins_even_when_the_path_does_not_exist() {
        let (p, args) = split(r#""/no/such/pager" --flag"#).unwrap();
        assert_eq!(p, "/no/such/pager");
        assert_eq!(args, vec!["--flag"]);
    }

    #[test]
    fn an_unterminated_quote_is_taken_whole_rather_than_dropped() {
        let (p, args) = split(r#""C:\Program Files\less.exe"#).unwrap();
        assert_eq!(p, r"C:\Program Files\less.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn nothing_at_all_disables_paging() {
        assert!(split("").is_none());
        assert!(split("   ").is_none());
        assert!(split(r#"""#).is_none());
    }

    #[test]
    fn a_nonexistent_spaced_command_still_splits() {
        // Not a path, so the old behaviour is the right one here.
        let (p, args) = split("my-pager --wrap").unwrap();
        assert_eq!(p, "my-pager");
        assert_eq!(args, vec!["--wrap"]);
    }

    #[test]
    fn nothing_is_spawned_until_something_is_written() {
        // A command that fails before producing output must not open a pager
        // over the top of git's error message.
        let out = Out::new(Mode::Always);
        assert!(
            !out.started(),
            "constructing an Out must not spawn anything"
        );
        assert!(out.finish().is_ok(), "finishing an unused Out is a no-op");
    }

    #[test]
    fn writing_chooses_a_destination() {
        let mut out = Out::new(Mode::Never);
        assert!(!out.started());
        out.write_all(b"x").unwrap();
        assert!(out.started(), "the first write decides where output goes");
        out.finish().unwrap();
    }
}
