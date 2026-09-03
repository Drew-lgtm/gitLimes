# gitlimes

[![CI](https://github.com/Drew-lgtm/gitLimes/actions/workflows/ci.yml/badge.svg)](https://github.com/Drew-lgtm/gitLimes/actions/workflows/ci.yml)

*limes — the fortified frontier of the Roman empire; the line that shows you where the borders ran.*

A fast, low-memory CLI for asking a git repository what happened and who did it.

```
○        d98c4b9  7m     Ondrej Martinek   (HEAD -> main) merge: branches and who
├─╮
│ ●      ef98597  7m     Ondrej Martinek   feat(cmd): add branches and who
│ ●      ad98623  12m    Ondrej Martinek   chore: force lf line endings
├─╯
○        b2621f3  14m    Ondrej Martinek   merge: log command
```

## Why

`git log` is fast but hard to scan. TUIs like `tig` and `lazygit` are good but hold state and
take over the terminal. GitHub needs a browser and a network round trip. `gitlimes` prints a
readable, coloured answer and exits.

## Install

Prebuilt binaries for Linux, macOS and Windows are attached to each
[release](https://github.com/Drew-lgtm/gitLimes/releases). Download, extract, put `gitlimes` on
your `PATH`.

From crates.io:

```
cargo install gitlimes
```

From source:

```
git clone https://github.com/Drew-lgtm/gitLimes
cd gitLimes
cargo build --release
```

The binary lands in `target/release/gitlimes` and needs `git` on `PATH`. Tip: `alias limes=gitlimes`.

## Commands

```
gitlimes log       coloured commit history
gitlimes branches  branch overview with age, tracking and staleness
gitlimes who       author and contribution stats
gitlimes graph     unicode branch graph
```

Every command takes `--help` and `--json`. `--color` / `--no-color` and `--pager` / `--no-pager`
override tty detection; `NO_COLOR`, `GITLIMES_PAGER` and `PAGER` are honoured.

### log

```
gitlimes log [REV] [-n N] [--author PAT] [--since DATE] [--oneline] [-- PATH...]
```

Short hash, age, author, ref decorations, subject. Filters are passed straight through to git.

### branches

```
gitlimes branches [-a] [--stale DAYS] [--vs REF]
```

Sorted by last commit. Ahead/behind comes free from the tracked upstream; `--vs REF` compares
against an arbitrary branch instead, in the same single git process via `%(ahead-behind:)`
(git 2.41+, with a per-branch fallback on older versions).

### who

```
gitlimes who [PATH] [--since DATE] [--limit N] [--lines]
```

Commits per author with share, first and last seen, and an activity sparkline. Authors
are keyed by email, so one person committing under several names stays one row — and the email
column shows which identity it is. Pass a path to ask who owns a directory. `--lines` adds
added/removed line counts and is opt-in because it roughly doubles the work git has to do.

The sparkline spans the *whole* history rather than a fixed recent window, so a repository that
went quiet years ago still shows its shape. Its scale adapts - the header says what one block is
worth - because the span is not known until the oldest commit is read, and remembering every
timestamp to find it would make memory scale with history. Instead the buckets start one day wide
and double whenever a commit falls off the left edge.

### graph

```
gitlimes graph [REV] [-n N] [-a] [--ascii]
```

Filled dots are ordinary commits, hollow dots are merges. Each branch keeps one colour for its
whole life, even as it changes column. `--ascii` for terminals without box-drawing characters.

The topology matches `git log --graph` exactly, and is usually more compact — where git needs
three rows to untangle a crossing merge, the box-drawing form needs one.

One honest limitation: with a **commit filter** (`--author`, `--since`, a path, or a shallow
clone) the graph is approximate. git still reports each commit's real parents, but the filter
hides some of them, so lanes are opened for commits that will never arrive. The lane table is
capped at 64 and evicts the lane that has gone longest without a commit, which keeps memory
bounded and the drawing readable — but a filtered graph is a sketch, not the exact topology.
Unfiltered, it is exact.

## Machine-readable output

Every command takes `--json` and emits newline-delimited JSON: one self-contained object per line,
never a wrapping array. An array would have to be closed at the end, which means either buffering
the whole result or emitting something that is invalid until the process exits. NDJSON stays
streamable, survives `head`, and `jq -s` collects it into an array when a consumer wants one.

```console
$ gitlimes log -n 1 --json | jq -r '.subject'
$ gitlimes who --json | jq -s 'map(select(.commits > 10))'
```

`graph --json` carries the lane geometry alongside each commit:

```json
{"short":"6ff6ffe","subject":"merge: json output",
 "graph":{"col":0,"lanes":2,"merge":true,"closing":[],"opening":[1],"shifts":[]}}
```

That is what lets another renderer — an SVG export, a canvas, a TUI — draw the graph without
re-deriving the layout.

### Schema stability

There is deliberately **no version field**. A stamp on every line would cost bytes on every
record and would not prevent the failure it looks like it guards against — a field renamed by
accident. [`tests/cli.rs`](tests/cli.rs) pins the exact key set of every command instead, so a
rename fails CI rather than someone's script; `gitlimes --version` identifies the format.

The contract is **additive**: new keys may appear in any release, so ignore keys you do not
recognise; existing keys are not renamed, retyped or removed without a version bump recorded in
[CHANGELOG.md](CHANGELOG.md). Conditional keys are absent rather than null — `head` on a commit,
`track` on a branch, `added` and `removed` without `--lines`. The full key list per command is
in the changelog.

## Use it as a library

The reusable engine is a library; the CLI is one consumer of it. A TUI, a graphical front end or a
script can link against it instead of shelling out and parsing text.

```rust
use gitlimes::repo::{self, Commit, Records, LOG_FORMAT};

let mut records = Records::spawn(repo::git(&["log", LOG_FORMAT, "--"]))?;
while let Some(record) = records.next_record()? {
    if let Some(commit) = Commit::parse(&record) {
        println!("{} {}", commit.short, commit.subject);
    }
}
records.finish()?;
```

`graph::lanes` turns a commit stream into branch-lane geometry and draws nothing; `graph::draw`
renders that geometry as terminal rows. The split is deliberate — a second renderer reuses the
layout instead of reimplementing it.

## Design premise: flat memory

`gitlimes` spawns `git` and streams its output through a buffer that is reused for every record.
It never collects the history into memory.

| Command    | Peak memory scales with          |
| ---------- | -------------------------------- |
| `log`      | one commit record                |
| `graph`    | simultaneously open branch lanes |
| `who`      | unique authors                   |
| `branches` | refs                             |

Nothing scales with history length. Measured on a 4359-commit, 215-ref repository:

```
log   -n   10  ->  3.52 MB private
log   -n 4000  ->  3.51 MB private
graph -n   10  ->  3.41 MB private
graph -n 4000  ->  3.55 MB private
```

400x the work for 0.1 MB. The floor is the process image; the marginal cost of history is
effectively zero.

## Zero dependencies

No `clap`, no `chrono`, no `crossterm` — only the standard library. Argument parsing, ANSI
escapes and tty detection are all small enough to own, and git formats dates for us. The result
is a **385 KB** binary, an empty supply chain, and a build that finishes in seconds.

The one piece of platform code is a ~20 line `kernel32` call to enable ANSI processing on
Windows `conhost`; Windows Terminal, macOS and Linux need nothing.

## Tests

```
cargo test
```

97 tests in two layers.

**46 unit tests** cover the pure logic — lane assignment for linear history, merges, octopus
merges, fork folding, lane reuse and compaction; rendering tests that pin the exact glyph output
for each case; JSON escaping; pager resolution; and column fitting, relative dates and sparklines.

**51 integration tests** run the real built binary against a real git repository. That is the only
way to cover the streaming record reader in `repo.rs` and the hand-rolled argument parsing, so
they carry the claims that matter: that commit order matches git's, that a subject containing a
pipe, quotes, a backslash and non-ASCII text survives the field-separated record format, that a
multi-line commit body never leaks into the output, that colour is off when stdout is a pipe, and
that every command exits non-zero with a readable message outside a repository.

The fixture builds a repository with a known merge topology and pins everything that would
otherwise vary per machine: no system or global git config, an explicit author, committer and
date on every single call, no signing, and an explicit default branch. Forcing the identity on
*every* call — not just on `git commit` — matters more than it looks: with no configured user git
quietly invents one from the OS username and hostname, so an unattributed `git merge` is authored
by whoever ran the tests and `who` reports a phantom extra author that differs per machine.

Tests never ship. `#[cfg(test)]` compiles them out entirely, and the integration tests live in
`tests/`, which is only built by `cargo test` — the release binary contains no test code at all.

## CI

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on every push and pull request to
`main`:

- **fmt + clippy** — `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
- **test** — the full suite on `ubuntu-latest`, `macos-latest` and `windows-latest`. This is what
  actually earns the cross-platform claim; the project is developed on Windows, so Linux and macOS
  are otherwise untested.
- **smoke test** — builds the release binary (which `cargo test` never exercises, since it tests
  the debug build) and runs every subcommand against *this repository's own history*, checked out
  at full depth. gitlimes is built from `--no-ff` merges, so the graph must contain a merge dot;
  if lane rendering regresses, the job fails.
- **msrv** — builds and tests on the exact toolchain named by `rust-version` in `Cargo.toml`, so
  the stated minimum stays true rather than aspirational.

There is no dependency cache: with zero dependencies a cold build takes seconds, and restoring a
cache would cost more time than it saves.

[`.github/workflows/release.yml`](.github/workflows/release.yml) runs on a `v*.*.*` tag: it builds
for Linux, macOS (both architectures) and Windows, smoke-tests each native binary, and attaches
archives plus SHA-256 checksums to the GitHub Release.

Publishing to crates.io is deliberately not automated. A published version can never be deleted,
only yanked, so it stays a manual, considered act:

```
cargo publish --dry-run
cargo publish
```

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. This is the convention across the Rust ecosystem: MIT is permissive and short,
and Apache-2.0 adds an explicit patent grant.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this work by you shall be dual-licensed as above, without any additional terms or conditions.
