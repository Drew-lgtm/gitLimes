// Each integration test binary uses a different subset of these helpers, so
// unused ones are expected rather than a mistake.
#![allow(dead_code)]

//! A throwaway git repository with a known shape, for end-to-end tests.
//!
//! Every git call is fully isolated from the machine it runs on: no system or
//! global config, an explicit identity and date on *every* invocation, no
//! signing, and an explicit default branch.
//!
//! The identity must be forced on every call, not just on `git commit`. With
//! no configured user, git silently invents one from the OS username and
//! hostname - so a `git merge` would be authored by whoever ran the tests, and
//! `who` would report a phantom extra author that differs per machine.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

pub const ALICE: &str = "Alice Example";
pub const ALICE_MAIL: &str = "alice@example.com";
pub const BOB: &str = "Bob Builder";
pub const BOB_MAIL: &str = "bob@example.com";

/// Commits reachable from `main`, and how they split between the two authors.
pub const MAIN_COMMITS: usize = 7;
pub const ALICE_COMMITS: usize = 5;
pub const BOB_COMMITS: usize = 2;

/// A subject with the characters most likely to break a field-separated
/// record format: a pipe, both quote styles, a backslash and non-ASCII text.
pub const TRICKY_SUBJECT: &str = "feat: pipe | quote \" and ' and \\ and Přílíš žluťoučký kůň";

/// The body of commit B. `%s` is the subject alone, so this text must never
/// appear in any command's output - if it does, record parsing is leaking.
pub const BODY_MARKER: &str = "BODY-SHOULD-NEVER-APPEAR-IN-OUTPUT";

/// Deliberately avoids the word "stale" so tests can look for the staleness
/// marker without matching the branch name itself.
pub const SIDE_BRANCH: &str = "legacy/old";

pub struct Fixture {
    pub dir: PathBuf,
    /// Set on shared fixtures, whose lifetime is the whole test binary.
    keep: bool,
}

/// The standard repository, built once per test binary.
///
/// Every test that uses it only reads, so one copy is safe to share - and it
/// turns hundreds of git process spawns into about a dozen.
pub fn repo() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| Fixture::build("shared", true))
}

/// An initialised repository with no commits at all.
pub fn empty_repo() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| {
        let f = Fixture::bare("empty", true);
        f.git(&["init", "-q", "-b", "main"]);
        f
    })
}

/// A directory that is deliberately not a git repository.
pub fn non_repo() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| Fixture::bare("norepo", true))
}

impl Fixture {
    /// Builds this history on `main` (newest first):
    ///
    /// ```text
    /// * F  chore: release            Alice   (tag v1.0)
    /// *   M  merge: feature          Alice   (merge commit)
    /// |\
    /// | * D  <tricky subject>        Bob
    /// | * C  feat: first side        Bob
    /// * | E  fix: mainline fix       Alice
    /// |/
    /// * B  docs: readme              Alice   (multi-line message)
    /// * A  chore: init               Alice   (root)
    /// ```
    ///
    /// plus `legacy/old`, an unmerged branch off B, and the tag `v1.0` on F.
    pub fn new(name: &str) -> Fixture {
        Fixture::build(name, false)
    }

    fn build(name: &str, keep: bool) -> Fixture {
        let f = Fixture::bare(name, keep);
        // -b main: init.defaultBranch differs between git versions and configs.
        f.git(&["init", "-q", "-b", "main"]);

        f.commit(1, ALICE, ALICE_MAIL, "a.txt", "chore: init", None);
        f.commit(
            2,
            ALICE,
            ALICE_MAIL,
            "b.txt",
            "docs: readme",
            Some(BODY_MARKER),
        );

        f.git(&["switch", "-q", "-c", "feature"]);
        f.commit(3, BOB, BOB_MAIL, "c.txt", "feat: first side", None);
        f.commit(4, BOB, BOB_MAIL, "d.txt", TRICKY_SUBJECT, None);

        f.git(&["switch", "-q", "main"]);
        f.commit(5, ALICE, ALICE_MAIL, "e.txt", "fix: mainline fix", None);
        // Authored explicitly: an unattributed merge would inherit the host's
        // identity and show up as a third author.
        f.git_as(
            6,
            ALICE,
            ALICE_MAIL,
            &["merge", "-q", "--no-ff", "-m", "merge: feature", "feature"],
        );
        f.commit(7, ALICE, ALICE_MAIL, "f.txt", "chore: release", None);
        f.git_as(7, ALICE, ALICE_MAIL, &["tag", "v1.0"]);

        // An unmerged branch, so `branches` has something behind to report.
        f.git(&["switch", "-q", "-c", SIDE_BRANCH, "main~2"]);
        f.commit(8, BOB, BOB_MAIL, "g.txt", "wip: abandoned", None);
        f.git(&["switch", "-q", "main"]);

        f
    }

    fn bare(name: &str, keep: bool) -> Fixture {
        let dir = std::env::temp_dir().join(format!("gitlimes-it-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        // An empty file standing in for the global config, so the developer's
        // real one cannot influence the fixture.
        std::fs::write(dir.join("empty-gitconfig"), b"").expect("write stub config");
        Fixture { dir, keep }
    }

    /// Every fixture commit is dated 2021, far enough in the past that
    /// staleness thresholds are unambiguous, and one minute apart so ordering
    /// is deterministic.
    fn date(seq: u32) -> String {
        format!("2021-01-01T12:{:02}:00+00:00", seq)
    }

    fn base_command(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        c.current_dir(&self.dir)
            // Isolate from anything configured on the host machine.
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.dir.join("empty-gitconfig"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("HOME", &self.dir)
            .env("XDG_CONFIG_HOME", &self.dir)
            // Colour must be decided by gitlimes, never inherited.
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE");
        c
    }

    fn git_command(&self, seq: u32, name: &str, email: &str) -> Command {
        let mut c = self.base_command("git");
        let date = Fixture::date(seq);
        c.args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date);
        c
    }

    fn check(out: Output, what: &str) -> String {
        assert!(
            out.status.success(),
            "{} failed: {}",
            what,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Runs git with the default fixture identity.
    pub fn git(&self, args: &[&str]) -> String {
        self.git_as(1, ALICE, ALICE_MAIL, args)
    }

    /// Runs git with an explicit author, committer and date.
    pub fn git_as(&self, seq: u32, name: &str, email: &str, args: &[&str]) -> String {
        let out = self
            .git_command(seq, name, email)
            .args(args)
            .output()
            .expect("run git");
        Fixture::check(out, &format!("git {:?}", args))
    }

    fn commit(
        &self,
        seq: u32,
        name: &str,
        email: &str,
        file: &str,
        subject: &str,
        body: Option<&str>,
    ) {
        std::fs::write(self.dir.join(file), subject.as_bytes()).expect("write file");
        self.git_as(seq, name, email, &["add", "-A"]);
        let mut c = self.git_command(seq, name, email);
        c.args(["commit", "-q", "-m", subject]);
        if let Some(b) = body {
            c.args(["-m", b]);
        }
        Fixture::check(c.output().expect("run git commit"), "git commit");
    }

    /// Runs the real built binary in the fixture repository.
    pub fn run(&self, args: &[&str]) -> Output {
        self.base_command(env!("CARGO_BIN_EXE_gitlimes"))
            .args(args)
            .output()
            .expect("run gitlimes")
    }

    /// Runs the binary with `NO_COLOR` set in the environment.
    pub fn run_no_color_env(&self, args: &[&str]) -> Output {
        self.base_command(env!("CARGO_BIN_EXE_gitlimes"))
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .expect("run gitlimes")
    }

    /// Runs the binary and asserts it succeeded, returning stdout.
    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "gitlimes {:?} exited {:?}\nstderr: {}",
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Shared fixtures live in a OnceLock for the life of the test binary,
        // so this never runs for them; the OS reclaims the temp directory.
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// True if the text contains any ANSI escape sequence.
pub fn has_ansi(s: &str) -> bool {
    s.contains('\u{1b}')
}

/// Strips ANSI SGR sequences so assertions can target the visible text.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        // Skip "[ ... m"; every sequence this tool emits is an SGR one.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// Reduces each row to its leading graph glyphs, dropping the commit text, so
/// assertions describe structure rather than content.
pub fn graph_shape(out: &str) -> Vec<String> {
    out.lines()
        .map(|l| {
            l.chars()
                .take_while(|c| matches!(c, '*' | 'o' | '|' | '/' | '\\' | '+' | '-' | ' ' | '_'))
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .collect()
}
