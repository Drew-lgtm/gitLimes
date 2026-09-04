# Changelog

Notable changes to gitlimes. Format follows [Keep a Changelog](https://keepachangelog.com/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is below 1.0, a minor bump may contain breaking changes — they are listed under
**Changed** or **Removed** and always called out here.

## [Unreleased]

### Fixed

Findings from an adversarial audit, each reproduced before fixing and pinned by a regression test.

- `who --limit 0` panicked with an index out of bounds. The emptiness guard ran before the
  truncation that emptied the list, and one column width indexed instead of folding.
- `branches --stale` accepted any `i64`: a negative value put the cutoff in the future and marked
  every branch stale, and a large one overflowed the multiply into seconds — a panic in debug, a
  wrapped and silently wrong cutoff in release. The threshold is a `u32` now, so neither is
  representable.
- `--color`, `--no-color`, `--pager` and `--no-pager` were stripped from anywhere in the argument
  list, including after `--`. A path spelled like a flag changed the colour and was silently
  dropped from the filter.
- `head` in `--json` came from a substring test, so `origin/HEAD` and any branch named like
  `HEADROOM` also claimed to be checked out. It now matches a whole decoration.
- Boolean flags accepted and discarded an inline value, so `--json=false` turned JSON on.
- An octopus merge, or a fork where more than one lane folded in, lost the connector for every
  lane except the farthest: the horizontal drawn for one target overwrote the corner already
  drawn for another. Horizontals are now drawn before any corner, and a lane the connector passes
  through gets a tee rather than nothing.
- The lane table grew with the number of commits shown whenever a parent never arrived — under
  `--author`, `--since`, a path filter or a shallow clone — breaking the memory guarantee. It is
  now capped, evicting the stalest lane. See the graph note in the README for what this costs.
- A pager whose path contained a space was split into a program plus arguments, failed to spawn,
  and paging silently turned itself off with no message — and on Windows that describes most
  pager paths. The setting is now read as: a quoted program first, else the longest run of leading
  words that names a real file, else the first word. So
  `C:\Program Files\Git\usr\bin\less.exe -S` works quoted or bare, and `less -S` is unchanged.
- The pager was spawned before the first record was read, so a command that failed early opened
  an empty pager over the top of git's error message. It now starts lazily, on the first byte
  actually written.
- `NO_COLOR=` — present but empty — disabled colour. The convention at no-color.org requires a
  non-empty value, so an empty one now behaves like unset, which is how a user cancels an
  inherited setting.
- A pager child whose stdin could not be taken was dropped without being reaped.
- A commit subject containing `0x1f` or `0x1e` was silently truncated at that byte, because the
  record format was delimited by exactly those bytes — and git stores them quite happily if you
  commit with `-F`. Fields are now separated by NUL, which git rejects outright ("a NUL byte in
  commit log message not allowed"), and the log format is framed by newlines, which no field in
  it can contain. The subject is also parsed with `splitn`, so it keeps everything after the last
  field boundary whatever it contains.
- `Records::finish` could hang a library caller that stopped reading before the end: git stayed
  blocked writing into a full pipe while we waited on it. The read end is now closed first, and
  git's exit status is only reported when the stream was actually read to EOF — otherwise the
  failure being reported is the one we caused.
- A `Records` dropped without `finish` left the git child unreaped.
- `Obj::default()` produced malformed JSON — no opening brace and a leading comma — because
  `Default` was derived rather than delegating to `Obj::new()`.
- `GetStdHandle` failure was tested for null, but it reports failure as `INVALID_HANDLE_VALUE`.

## [0.1.0]

First release.

### Added

- `log` — coloured commit history, with `--author`, `--since`, `--until`, `--all`, `--oneline`
  and path filtering after `--`.
- `branches` — branch overview with age, author, upstream tracking and `--stale`. `--vs REF`
  compares against an arbitrary ref, resolved in the same single git process via
  `%(ahead-behind:)` on git 2.41+, with a per-branch fallback on older versions.
- `who` — commits per author, keyed by email so one person under several names stays one row,
  with an activity sparkline that spans the whole history and states its own scale. `--lines`
  adds added/removed line counts.
- `graph` — unicode branch graph whose topology matches `git log --graph`, with lane compaction
  and a stable colour per branch. `--ascii` for terminals without box-drawing characters.
- `--json` on every command: newline-delimited JSON. See **JSON schema** below.
- Pager support, like git's: `--pager` / `--no-pager`, honouring `GITLIMES_PAGER` and `PAGER`.
- A library (`gitlimes` on docs.rs) exposing the streaming reader, the lane algorithm and the
  renderer, so a TUI or GUI can reuse them instead of parsing this tool's output.

### Notes

- Minimum supported Rust version is 1.82, enforced by CI.
- Zero dependencies; the standard library only.
- Requires `git` on `PATH`.

## JSON schema

The `--json` output carries **no version field**, deliberately. A stamp on every line would cost
bytes on every record and would not prevent the failure it appears to guard against — a field
renamed by accident. `tests/cli.rs` pins the exact key set of every command instead, so a rename
fails CI rather than someone's script. `gitlimes --version` identifies the format.

The contract is **additive**:

- New keys may appear in any release. Consumers must ignore keys they do not recognise.
- Existing keys are not renamed, retyped or removed without a version bump listed here.
- Some keys are conditional and absent rather than null: `head` on a commit, `track` on a
  branch, `added` and `removed` without `--lines`.

Current keys, as of 0.1.0:

| Command    | Keys                                                                      |
| ---------- | ------------------------------------------------------------------------- |
| `log`      | `hash` `short` `parents` `author` `date` `refs` `subject` — plus `head`    |
| `graph`    | the `log` keys, plus `graph` = `col` `lanes` `merge` `closing` `opening` `shifts` |
| `branches` | `name` `current` `date` `author` `subject` — plus `track`                  |
| `who`      | `name` `email` `commits` `first` `last` `activity` `bucket_seconds` — plus `added` `removed` |

`date`, `first` and `last` are unix seconds. `activity` is oldest bucket first, each covering
`bucket_seconds`.

[Unreleased]: https://github.com/Drew-lgtm/gitLimes/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Drew-lgtm/gitLimes/releases/tag/v0.1.0
