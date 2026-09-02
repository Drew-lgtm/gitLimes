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
pub const LOG_FORMAT: &str =
    "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%ar%x1f%D%x1f%s%x1e";

/// Borrows every field out of the reader's buffer; nothing here is owned.
pub struct Commit<'a> {
    pub hash: &'a str,
    pub short: &'a str,
    pub parents: &'a str,
    pub author: &'a str,
    pub email: &'a str,
    pub timestamp: &'a str,
    pub relative: &'a str,
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
            email: f.next()?,
            timestamp: f.next()?,
            relative: f.next()?,
            refs: f.next()?,
            // The subject is last, so a subject containing FS would still be
            // truncated here; git never emits FS, so this cannot happen.
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

/// Runs a git command and returns its whole stdout. Only for commands whose
/// output is bounded by ref count, never by history length.
pub fn capture(args: &[&str]) -> io::Result<String> {
    let out = git(args).stderr(Stdio::inherit()).output()?;
    if !out.status.success() {
        std::process::exit(out.status.code().unwrap_or(1));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Streams `\x1e`-delimited records from a child git process.
pub struct Records {
    child: Child,
    out: BufReader<ChildStdout>,
    buf: Vec<u8>,
}

impl Records {
    pub fn spawn(mut cmd: Command) -> io::Result<Records> {
        let mut child = cmd.spawn()?;
        let out = BufReader::with_capacity(64 * 1024, child.stdout.take().unwrap());
        Ok(Records {
            child,
            out,
            buf: Vec::with_capacity(1024),
        })
    }

    /// Next record, or `None` at end of stream. The returned string borrows the
    /// internal buffer and is invalidated by the following call.
    pub fn next(&mut self) -> io::Result<Option<Cow<'_, str>>> {
        self.buf.clear();
        if self.out.read_until(RS, &mut self.buf)? == 0 {
            return Ok(None);
        }
        if self.buf.last() == Some(&RS) {
            self.buf.pop();
        }
        // git writes a newline after each record's terminator, so every record
        // after the first arrives with a leading newline.
        let start = self
            .buf
            .iter()
            .position(|b| *b != b'\n' && *b != b'\r')
            .unwrap_or(self.buf.len());
        if start == self.buf.len() {
            return Ok(None);
        }
        // Commit messages are not guaranteed UTF-8; lossy conversion borrows
        // when they are, which is the normal case.
        Ok(Some(String::from_utf8_lossy(&self.buf[start..])))
    }

    /// Propagates git's exit code so `not a git repository` and friends surface
    /// correctly. git has already printed its own message to our stderr.
    pub fn finish(mut self) -> io::Result<()> {
        let status = self.child.wait()?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        Ok(())
    }
}
