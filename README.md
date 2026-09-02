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

## Commands

```
gitlimes log       coloured commit history
gitlimes branches  branch overview with age, tracking and staleness
gitlimes who       author and contribution stats
gitlimes graph     unicode branch graph
```

Every command takes `--help`. `--color` / `--no-color` override tty detection, and `NO_COLOR`
is honoured.

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
against an arbitrary branch instead and is opt-in because it costs one extra git call per branch.

### who

```
gitlimes who [PATH] [--since DATE] [--limit N] [--lines]
```

Commits per author with share, first and last seen, and a 12-month activity sparkline. Authors
are keyed by email, so one person committing under several names stays one row — and the email
column shows which identity it is. Pass a path to ask who owns a directory. `--lines` adds
added/removed line counts and is opt-in because it roughly doubles the work git has to do.

### graph

```
gitlimes graph [REV] [-n N] [-a] [--ascii]
```

Filled dots are ordinary commits, hollow dots are merges. Each branch keeps one colour for its
whole life, even as it changes column. `--ascii` for terminals without box-drawing characters.

The topology matches `git log --graph` exactly, and is usually more compact — where git needs
three rows to untangle a crossing merge, the box-drawing form needs one.

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
is a **342 KB** binary, an empty supply chain, and a build that finishes in seconds.

The one piece of platform code is a ~20 line `kernel32` call to enable ANSI processing on
Windows `conhost`; Windows Terminal, macOS and Linux need nothing.

## Build

```
cargo build --release
```

The binary lands in `target/release/gitlimes` and needs `git` on `PATH`.

Tip: alias it for daily use.

```
alias limes=gitlimes
```

## Tests

```
cargo test
```

50 tests in two layers.

**20 unit tests** cover the pure logic — lane assignment for linear history, merges, octopus
merges, fork folding, lane reuse and compaction; rendering tests that pin the exact glyph output
for each case; and column fitting, relative dates and sparklines.

**30 integration tests** run the real built binary against a real git repository. That is the only
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
