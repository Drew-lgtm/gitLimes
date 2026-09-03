# Changelog

Notable changes to gitlimes. Format follows [Keep a Changelog](https://keepachangelog.com/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is below 1.0, a minor bump may contain breaking changes — they are listed under
**Changed** or **Removed** and always called out here.

## [Unreleased]

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
