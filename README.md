# gitlimes

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

The graph algorithm is the part worth testing and carries most of the suite: lane assignment for
linear history, merges, octopus merges, fork folding, lane reuse and compaction, plus rendering
tests that pin the exact glyph output for each case.
