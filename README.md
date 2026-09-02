# gitlimes

*limes — the fortified frontier of the Roman empire; the line that shows you where the borders ran.*

A fast, low-memory CLI for asking a git repository what happened and who did it.

## Why

`git log` is fast but hard to scan. TUIs like `tig` and `lazygit` are great but hold state.
GitHub needs a browser and a network round trip. `gitlimes` is a single small binary that
prints readable, colored answers in milliseconds and exits.

## Design premise: flat memory

`gitlimes` spawns `git` and streams its output through a reused buffer. It never collects
the history into memory.

| Command    | Peak memory scales with        |
| ---------- | ------------------------------ |
| `log`      | one commit record              |
| `graph`    | simultaneously open branch lanes |
| `who`      | unique authors                 |
| `branches` | refs                           |

Nothing scales with history length: a 500k-commit repo costs the same as a 50-commit one.

Zero dependencies — no `clap`, no `chrono`, no `crossterm`. Just the standard library.

## Commands

```
gitlimes log       colored commit history
gitlimes branches  branch overview with staleness and ahead/behind
gitlimes who       author and contribution stats
gitlimes graph     unicode branch graph
```

## Build

```
cargo build --release
```

The binary lands in `target/release/gitlimes`. Requires `git` on `PATH`.

Tip: alias it for daily use.

```
alias limes=gitlimes
```
