//! The only module that knows git is a subprocess.
//!
//! Commits are read one record at a time into a buffer that is reused for every
//! record, so peak memory is the size of the largest single commit record and
//! does not grow with history length.

use std::borrow::Cow;
use std::io::{self, BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

/// Unit separator: between fields. Cannot occur in commit metadata.
pub const FS: char = '\u{1f}';
/// Record separator: between commits. Cannot occur in commit metadata.
pub const RS: u8 = 0x1e;

/// Field order must match `Commit::parse`.
pub const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%P%x1f%an%x1f%at%x1f%D%x1f%s%x1e";

/// Leading separator, used when git appends extra lines after each record
/// (`--numstat`). Those lines then arrive at the head of the *next* record,
/// where one split on the first newline separates them cleanly.
pub const WHO_FORMAT: &str = "--format=%x1e%an%x1f%ae%x1f%at";

/// Borrows every field out of the reader's buffer; nothing here is owned.
pub struct Commit<'a> {
    pub hash: &'a str,
    pub short: &'a str,
    pub parents: &'a str,
    pub author: &'a str,
    pub timestamp: &'a str,
    pub refs: &'a str,
    pub subject: &'a str,
}

impl<'a> Commit<'a> {
    pub fn parse(rec: &'a str) -> Option<Commit<'a>> {
        let mut f = rec.split(FS);
        Some(Commit {
            hash: f.next()?,
            short: f.next()?,
            parents: f.next()?,
            author: f.next()?,
            timestamp: f.next()?,
            refs: f.next()?,
            subject: f.next()?,
        })
    }

    pub fn parent_iter(&self) -> impl Iterator<Item = &'a str> {
        self.parents.split_whitespace()
    }
}

pub fn git(args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.args(args).stdout(Stdio::piped()).stderr(Stdio::inherit());
    c
}

/// Raised when git itself ran and exited non-zero.
///
/// git has already written its own diagnostic to stderr, so this carries only
/// the code, for a caller that wants to exit with it. It travels inside an
/// `io::Error` so every function here can keep returning `io::Result` - a
/// library must never call `process::exit` and kill its caller.
#[derive(Debug, Clone, Copy)]
pub struct GitExit(pub i32);

impl std::fmt::Display for GitExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "git exited with status {}", self.0)
    }
}

impl std::error::Error for GitExit {}

/// Recovers git's exit code from an error produced by this module, if that is
/// what the error is.
pub fn git_exit_code(e: &io::Error) -> Option<i32> {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<GitExit>())
        .map(|g| g.0)
}

fn spawn_error(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::NotFound {
        // The likeliest first-run failure by a wide margin, and the bare
        // "program not found" says nothing about what to install.
        return io::Error::new(
            io::ErrorKind::NotFound,
            "git was not found on PATH; gitlimes runs git and needs it installed",
        );
    }
    e
}

/// Runs a git command and returns its whole stdout. Only for commands whose
/// output is bounded by ref count, never by history length.
pub fn capture(args: &[&str]) -> io::Result<String> {
    let out = git(args)
        .stderr(Stdio::inherit())
        .output()
        .map_err(spawn_error)?;
    if !out.status.success() {
        return Err(io::Error::other(GitExit(out.status.code().unwrap_or(1))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Like [`capture`], but a non-zero exit is `None` rather than an error, and
/// git's own message is swallowed.
///
/// This is for *feature detection*: asking the installed git whether it
/// supports something by trying it. That is more reliable than parsing
/// `git --version`, which says nothing about how a distribution built it.
pub fn probe(args: &[&str]) -> io::Result<Option<String>> {
    let out = git(args).stderr(Stdio::null()).output()?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// Streams separator-delimited records from a child git process.
pub struct Records {
    child: Child,
    out: BufReader<ChildStdout>,
    buf: Vec<u8>,
}

impl Records {
    pub fn spawn(mut cmd: Command) -> io::Result<Records> {
        let mut child = cmd.spawn().map_err(spawn_error)?;
        // Only `None` if the caller built a Command without a piped stdout.
        // A library returns an error for that; it does not panic on its caller.
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the command must be configured with a piped stdout; use repo::git()",
            )
        })?;
        let out = BufReader::with_capacity(64 * 1024, stdout);
        Ok(Records {
            child,
            out,
            buf: Vec::with_capacity(1024),
        })
    }

    /// Next record, or `None` at end of stream.
    ///
    /// Deliberately not `Iterator`: the returned string borrows the internal
    /// buffer and is invalidated by the following call, which no iterator
    /// signature can express.
    ///
    /// Blank records are skipped rather than reported as end-of-stream: a
    /// format that leads with the separator emits an empty first record, and
    /// git pads records with newlines.
    pub fn next_record(&mut self) -> io::Result<Option<Cow<'_, str>>> {
        let start;
        loop {
            self.buf.clear();
            if self.out.read_until(RS, &mut self.buf)? == 0 {
                return Ok(None);
            }
            if self.buf.last() == Some(&RS) {
                self.buf.pop();
            }
            let s = self
                .buf
                .iter()
                .position(|b| !matches!(b, b'\n' | b'\r'))
                .unwrap_or(self.buf.len());
            if s < self.buf.len() {
                start = s;
                break;
            }
        }
        // Commit messages are not guaranteed UTF-8; lossy conversion borrows
        // when they are, which is the normal case.
        Ok(Some(String::from_utf8_lossy(&self.buf[start..])))
    }

    /// Waits for git and reports its exit code as a [`GitExit`] error, so
    /// `not a git repository` and friends surface correctly. git has already
    /// printed its own message to our stderr.
    pub fn finish(mut self) -> io::Result<()> {
        let status = self.child.wait()?;
        if !status.success() {
            return Err(io::Error::other(GitExit(status.code().unwrap_or(1))));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_without_piped_stdout_errors_rather_than_panicking() {
        // A library consumer can build its own Command. Getting that wrong must
        // produce an error, not abort the caller's process.
        let mut cmd = Command::new("git");
        cmd.arg("--version");
        let err = match Records::spawn(cmd) {
            Ok(_) => panic!("spawn should reject a command without piped stdout"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("piped stdout"));
    }

    #[test]
    fn a_missing_git_says_so_by_name() {
        let mut cmd = Command::new("definitely-not-a-real-program-xyz");
        cmd.stdout(Stdio::piped());
        let err = match Records::spawn(cmd) {
            Ok(_) => panic!("that program should not exist"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains("git was not found on PATH"),
            "unhelpful message: {}",
            err
        );
    }

    #[test]
    fn git_exit_code_round_trips_through_io_error() {
        let e = io::Error::other(GitExit(128));
        assert_eq!(git_exit_code(&e), Some(128));
        // An unrelated error must not be mistaken for a git exit.
        assert_eq!(git_exit_code(&io::Error::other("something else")), None);
    }

    #[test]
    fn nothing_in_this_module_exits_the_process() {
        // Guards the rule, not just today's code: a library that terminates the
        // process kills any TUI or GUI that embeds it. Assembled at runtime so
        // this test's own source cannot match itself.
        let needle = format!("process{}exit", "::");
        let src = include_str!("repo.rs");
        // Only the real code; the test module below is allowed to mention it.
        let code = src.split("#[cfg(test)]").next().unwrap_or(src);
        let offenders: Vec<&str> = code
            .lines()
            .filter(|l| l.contains(&needle) && !l.trim_start().starts_with("//"))
            .collect();
        assert!(
            offenders.is_empty(),
            "the library must not exit the process: {:?}",
            offenders
        );
    }
}
