//! The only module that knows git is a subprocess.
//!
//! Commits are read one record at a time into a buffer that is reused for every
//! record, so peak memory is the size of the largest single commit record and
//! does not grow with history length.

use std::borrow::Cow;
use std::io::{self, BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

/// Field separator.
///
/// NUL, because git refuses outright to store one: "a NUL byte in commit log
/// message not allowed". The obvious-looking choices are not safe - git will
/// happily store 0x1f or 0x1e in a subject if you commit one with `-F`, and
/// they then split the record and silently truncate the subject.
pub const FS: char = '\0';

/// Record separator for formats that append their own lines, such as
/// `--numstat`, where a newline cannot delimit records.
pub const RS: u8 = 0x1e;

/// Record separator for the log format: git already writes a newline after each
/// record, and no field in `LOG_FORMAT` can contain one. `%s` is collapsed to a
/// single line by git, and git forbids newlines in author idents and ref names.
pub const LINE: u8 = b'\n';

/// Field order must match `Commit::parse`. Framed by [`LINE`], separated by
/// [`FS`], so no byte a commit can carry will break it.
pub const LOG_FORMAT: &str = "--format=%H%x00%h%x00%P%x00%an%x00%at%x00%D%x00%s";

/// Leading separator, used when git appends extra lines after each record
/// (`--numstat`). Those lines then arrive at the head of the *next* record,
/// where one split on the first newline separates them cleanly.
pub const WHO_FORMAT: &str = "--format=%x1e%an%x00%ae%x00%at";

/// Number of fields in [`LOG_FORMAT`].
const LOG_FIELDS: usize = 7;

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
        // splitn, not split: the subject is last, so anything separator-shaped
        // inside it stays part of the subject instead of spilling into a field
        // that no longer exists.
        let mut f = rec.splitn(LOG_FIELDS, FS);
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
    /// Taken before waiting, to close our end of the pipe. See `finish`.
    out: Option<BufReader<ChildStdout>>,
    buf: Vec<u8>,
    sep: u8,
    /// Set once the stream has been read to EOF. Until then git's exit status
    /// says nothing useful, because we are the ones who ended it.
    drained: bool,
}

impl Records {
    /// Reads records framed by [`RS`], for formats that carry extra lines.
    pub fn spawn(cmd: Command) -> io::Result<Records> {
        Records::spawn_framed(cmd, RS)
    }

    /// Reads [`LOG_FORMAT`] output: one commit per line, fields separated by
    /// [`FS`].
    ///
    /// The pairing of format and framing has to match, and getting it wrong is
    /// silent - `spawn` on this format would read the entire history as one
    /// record - so it is a single call rather than two things to line up.
    pub fn spawn_log(cmd: Command) -> io::Result<Records> {
        Records::spawn_framed(cmd, LINE)
    }

    /// Reads records framed by `sep`. Use [`LINE`] with [`LOG_FORMAT`].
    pub fn spawn_framed(mut cmd: Command, sep: u8) -> io::Result<Records> {
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
            out: Some(out),
            buf: Vec::with_capacity(1024),
            sep,
            drained: false,
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
        let Some(out) = self.out.as_mut() else {
            return Ok(None);
        };
        let start;
        loop {
            self.buf.clear();
            if out.read_until(self.sep, &mut self.buf)? == 0 {
                self.drained = true;
                return Ok(None);
            }
            if self.buf.last() == Some(&self.sep) {
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
        // Close our end of the pipe before waiting. A caller that stopped
        // reading early leaves git blocked writing into a full pipe, and
        // waiting on it in that state deadlocks; dropping the reader gives git
        // EPIPE so it can exit.
        drop(self.out.take());
        let status = self.child.wait()?;
        // Only report git's status if we actually read to the end. A caller
        // that stopped early killed git with EPIPE by closing the pipe, and
        // reporting that back as a failure would blame git for our choice.
        if self.drained && !status.success() {
            return Err(io::Error::other(GitExit(status.code().unwrap_or(1))));
        }
        Ok(())
    }
}

impl Drop for Records {
    /// A `Records` dropped without `finish` still has a child to reap; without
    /// this it would be left behind as a zombie.
    fn drop(&mut self) {
        drop(self.out.take());
        let _ = self.child.wait();
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
    fn the_field_separator_is_one_git_cannot_store() {
        // git rejects a NUL in a commit message outright, which is what makes
        // it safe. 0x1f and 0x1e are not safe: git stores them happily, and
        // they used to split the record and silently truncate the subject.
        assert_eq!(FS, '\0');
        assert!(LOG_FORMAT.contains("%x00"));
        assert!(
            !LOG_FORMAT.contains("%x1f") && !LOG_FORMAT.contains("%x1e"),
            "the log format must not rely on a byte a commit can carry"
        );
    }

    #[test]
    fn a_subject_containing_a_separator_stays_whole() {
        // splitn, not split: everything after the last field boundary belongs
        // to the subject, whatever bytes it happens to contain.
        let rec = "HASH\u{0}SHORT\u{0}P1 P2\u{0}Alice\u{0}123\u{0}refs\u{0}subject with \u{1f} and \u{1e}";
        let c = Commit::parse(rec).expect("parses");
        assert_eq!(c.hash, "HASH");
        assert_eq!(c.author, "Alice");
        assert_eq!(c.subject, "subject with \u{1f} and \u{1e}");
    }

    #[test]
    fn a_truncated_record_is_rejected_rather_than_half_parsed() {
        assert!(Commit::parse("HASH\u{0}SHORT\u{0}P1").is_none());
        assert!(Commit::parse("").is_none());
    }

    #[test]
    fn abandoning_a_reader_partway_neither_hangs_nor_leaks() {
        // A library caller may stop reading early. finish() must close the pipe
        // before waiting, or git stays blocked writing into a full one and the
        // wait never returns.
        let mut rec =
            Records::spawn_framed(git(&["log", LOG_FORMAT, "--"]), LINE).expect("spawn git log");
        assert!(
            rec.next_record().expect("read").is_some(),
            "this repository has commits"
        );
        // Deliberately stop here, with git still producing output.
        rec.finish().expect("finish must not hang or error");
    }

    #[test]
    fn dropping_a_reader_without_finishing_still_reaps_git() {
        let rec =
            Records::spawn_framed(git(&["log", LOG_FORMAT, "--"]), LINE).expect("spawn git log");
        drop(rec);
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
