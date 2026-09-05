//! End-to-end tests: they run the real built binary against a real git
//! repository, which is the only way to cover `repo.rs` (the streaming record
//! reader) and the argument parsing in `cmd/`.

mod fixture;

use fixture::{
    empty_repo, graph_shape, has_ansi, non_repo, repo, strip_ansi, Fixture, ALICE_COMMITS,
    BOB_COMMITS, BODY_MARKER, MAIN_COMMITS, SIDE_BRANCH, TRICKY_SUBJECT,
};

// ---------------------------------------------------------------- fixture

#[test]
fn fixture_builds_the_expected_topology() {
    let f = repo();
    let graph = f.git(&["log", "--graph", "--oneline", "--format=%s"]);
    assert_eq!(
        graph_shape(&graph),
        vec!["*", "*", "|\\", "| *", "| *", "* |", "|/", "*", "*"],
        "fixture history is not the shape the tests assume:\n{}",
        graph
    );
}

// -------------------------------------------------------------------- log

#[test]
fn log_lists_every_reachable_commit_in_git_order() {
    let f = repo();
    let expected = f.git(&["log", "--format=%h"]);
    let out = f.ok(&["log", "--no-color"]);

    let got: Vec<&str> = out
        .lines()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    let want: Vec<&str> = expected.lines().collect();
    assert_eq!(got, want, "log dropped, added or reordered commits");
    assert_eq!(
        got.len(),
        MAIN_COMMITS,
        "main should have 7 reachable commits"
    );
}

#[test]
fn log_never_leaks_the_commit_message_body() {
    // %s is the subject alone. If the record reader mis-split on newlines the
    // body would ride along into the output.
    let f = repo();
    let out = f.ok(&["log", "--no-color"]);
    assert!(
        !out.contains(BODY_MARKER),
        "the message body leaked into log output:\n{}",
        out
    );
    assert!(out.contains("docs: readme"), "the subject is missing");
}

#[test]
fn log_survives_separators_quotes_and_unicode_in_a_subject() {
    let f = repo();
    let out = f.ok(&["log", "--no-color"]);
    assert!(
        out.contains(TRICKY_SUBJECT),
        "a subject with quotes, a pipe, a backslash and unicode was mangled:\n{}",
        out
    );
    assert_eq!(
        out.lines().count(),
        MAIN_COMMITS,
        "the tricky subject split a record"
    );
}

#[test]
fn log_n_limits_output() {
    let f = repo();
    assert_eq!(f.ok(&["log", "-n", "3", "--no-color"]).lines().count(), 3);
    assert_eq!(
        f.ok(&["log", "--max-count=2", "--no-color"])
            .lines()
            .count(),
        2,
        "--flag=value form is not parsed"
    );
}

#[test]
fn log_author_filter_selects_one_persons_commits() {
    let f = repo();
    let out = f.ok(&["log", "--author", "Bob", "--no-color"]);
    assert_eq!(
        out.lines().count(),
        2,
        "Bob has 2 commits reachable from main"
    );
    assert!(out.lines().all(|l| l.contains("Bob")));
}

#[test]
fn log_oneline_prints_hash_and_subject_only() {
    let f = repo();
    let out = f.ok(&["log", "-n", "1", "--oneline", "--no-color"]);
    let line = out.lines().next().unwrap();
    assert!(line.ends_with("chore: release"));
    assert!(!line.contains("Alice"), "--oneline should omit the author");
}

#[test]
fn log_decorates_refs() {
    let f = repo();
    let out = f.ok(&["log", "-n", "1", "--no-color"]);
    assert!(
        out.contains("HEAD -> main"),
        "missing HEAD decoration:\n{}",
        out
    );
    assert!(
        out.contains("tag: v1.0"),
        "missing tag decoration:\n{}",
        out
    );
}

#[test]
fn log_accepts_a_path_after_a_double_dash() {
    let f = repo();
    let out = f.ok(&["log", "--no-color", "--", "c.txt"]);
    assert_eq!(out.lines().count(), 1, "only one commit touched c.txt");
    assert!(out.contains("feat: first side"));
}

#[test]
fn log_all_reaches_commits_off_the_current_branch() {
    let f = repo();
    let out = f.ok(&["log", "--all", "--no-color"]);
    assert!(
        out.contains("wip: abandoned"),
        "--all should reach the unmerged branch"
    );
    assert_eq!(out.lines().count(), MAIN_COMMITS + 1);
}

// --------------------------------------------------------------- branches

#[test]
fn branches_lists_every_local_branch_with_the_current_one_marked() {
    let f = repo();
    let out = f.ok(&["branches", "--no-color"]);
    assert_eq!(out.lines().count(), 3, "main, feature and stale/old");
    let current: Vec<&str> = out.lines().filter(|l| l.starts_with('*')).collect();
    assert_eq!(current.len(), 1, "exactly one branch is current");
    assert!(current[0].contains("main"));
    assert!(out.contains(SIDE_BRANCH));
}

#[test]
fn branches_vs_reports_ahead_and_behind() {
    // The exact numbers matter: `%(ahead-behind:)` reports "ahead behind" while
    // `rev-list --left-right` reports "behind ahead", so an inverted pair is a
    // silent, plausible-looking wrong answer.
    //
    // The side branch forked at E and added G, so it is ahead by 1. It is
    // behind by 4, not 2: main gained F and the merge M, and M also brought C
    // and D into main's reachable set.
    let f = repo();
    let out = f.ok(&["branches", "--vs", "main", "--no-color"]);
    let side = out
        .lines()
        .find(|l| l.contains(SIDE_BRANCH))
        .expect("the side branch is listed");
    assert!(
        side.contains("ahead 1, behind 4"),
        "expected 'ahead 1, behind 4', got: {}",
        side
    );

    let main = out
        .lines()
        .find(|l| l.starts_with('*'))
        .expect("main is listed");
    assert!(
        !main.contains("ahead") && !main.contains("behind"),
        "main compared against itself must show no divergence, got: {}",
        main
    );
}

#[test]
fn branches_marks_old_branches_as_stale() {
    let f = repo();
    // Every fixture commit is dated 2021, so a 1-day threshold marks them all.
    let out = f.ok(&["branches", "--stale", "1", "--no-color"]);
    assert_eq!(
        out.lines().filter(|l| l.contains("stale")).count(),
        3,
        "all three branches are older than a day:\n{}",
        out
    );
    let fresh = f.ok(&["branches", "--stale", "99999", "--no-color"]);
    assert_eq!(
        fresh.lines().filter(|l| l.contains("stale")).count(),
        0,
        "nothing should be marked stale with a huge threshold:
{}",
        fresh
    );
}

// -------------------------------------------------------------------- who

#[test]
fn who_counts_commits_per_author() {
    let f = repo();
    let out = strip_ansi(&f.ok(&["who", "--no-color"]));
    let alice = out
        .lines()
        .find(|l| l.contains("Alice"))
        .expect("Alice listed");
    let bob = out.lines().find(|l| l.contains("Bob")).expect("Bob listed");
    // Alice: init, readme, mainline fix, merge, release. Bob: the two side commits.
    assert!(
        alice.trim_start().starts_with(&ALICE_COMMITS.to_string()),
        "Alice row: {}",
        alice
    );
    assert!(
        bob.trim_start().starts_with(&BOB_COMMITS.to_string()),
        "Bob row: {}",
        bob
    );
    assert!(
        out.contains("alice"),
        "the email column identifies the author"
    );
}

#[test]
fn who_ranks_by_commit_count() {
    let f = repo();
    let out = strip_ansi(&f.ok(&["who", "--no-color"]));
    let rows: Vec<&str> = out.lines().skip(1).collect();
    assert!(rows[0].contains("Alice"), "the busiest author comes first");
    assert!(rows[1].contains("Bob"));
}

#[test]
fn who_limit_truncates_the_table() {
    let f = repo();
    let out = f.ok(&["who", "--limit", "1", "--no-color"]);
    assert_eq!(out.lines().count(), 2, "one header plus one author");
    assert!(out.contains("Alice"));
}

#[test]
fn who_scoped_to_a_path_only_counts_that_path() {
    let f = repo();
    let out = strip_ansi(&f.ok(&["who", "--no-color", "--", "c.txt"]));
    assert!(out.contains("Bob"), "Bob created c.txt");
    assert!(
        !out.contains("Alice"),
        "Alice never touched c.txt:\n{}",
        out
    );
}

#[test]
fn who_lines_counts_added_and_removed() {
    let f = repo();
    let out = strip_ansi(&f.ok(&["who", "--lines", "--no-color"]));
    assert!(out.contains("+LINES") && out.contains("-LINES"));
    let alice = out.lines().find(|l| l.contains("Alice")).unwrap();
    assert!(
        alice.contains('+'),
        "expected an added-lines figure: {}",
        alice
    );
}

// ------------------------------------------------------------------ graph

#[test]
fn graph_topology_matches_git_log_graph() {
    let f = repo();
    let out = strip_ansi(&f.ok(&["graph", "--ascii", "--no-color"]));
    assert_eq!(
        graph_shape(&out),
        vec![
            "*",    // chore: release
            "o",    // merge: feature
            "+-\\", //   branches out to the side lane
            "| *",  //   the tricky-subject commit
            "| *",  //   feat: first side
            "* |",  // fix: mainline fix
            "+-/",  //   the side lane folds back in
            "*",    // docs: readme
            "*",    // chore: init
        ],
        "graph shape drifted:\n{}",
        out
    );
}

#[test]
fn graph_commit_order_matches_git_topo_order() {
    let f = repo();
    let want: Vec<String> = f
        .git(&["log", "--topo-order", "--format=%h"])
        .lines()
        .map(str::to_string)
        .collect();
    let out = strip_ansi(&f.ok(&["graph", "--no-color"]));
    let got: Vec<String> = out
        .lines()
        .filter_map(|l| {
            l.split_whitespace()
                .find(|w| w.len() == 7 && w.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
        })
        .collect();
    assert_eq!(got, want, "graph visited commits in the wrong order");
}

#[test]
fn graph_marks_merges_with_a_hollow_dot() {
    let f = repo();
    let out = strip_ansi(&f.ok(&["graph", "--ascii", "--no-color"]));
    let merge = out.lines().find(|l| l.contains("merge: feature")).unwrap();
    assert!(merge.starts_with('o'), "merge row: {}", merge);
    let normal = out.lines().find(|l| l.contains("chore: release")).unwrap();
    assert!(normal.starts_with('*'), "ordinary commit row: {}", normal);
}

#[test]
fn graph_ascii_and_unicode_describe_the_same_history() {
    let f = repo();
    let ascii = strip_ansi(&f.ok(&["graph", "--ascii", "--no-color"]));
    let unicode = strip_ansi(&f.ok(&["graph", "--no-color"]));
    assert_eq!(
        ascii.lines().count(),
        unicode.lines().count(),
        "the two charsets disagree on row count"
    );
    assert!(
        !unicode.contains('*'),
        "unicode mode should not emit ASCII dots"
    );
    assert!(
        unicode.contains('\u{25cf}') && unicode.contains('\u{2502}'),
        "expected box-drawing glyphs"
    );
}

// ------------------------------------------------------------ colour rules

#[test]
fn output_is_plain_when_redirected() {
    // Output is captured through a pipe, so tty detection must turn colour off
    // even though no flag was passed.
    let f = repo();
    for args in [
        vec!["log", "-n", "1"],
        vec!["branches"],
        vec!["who"],
        vec!["graph", "-n", "1"],
    ] {
        let out = f.ok(&args);
        assert!(!has_ansi(&out), "{:?} emitted colour when redirected", args);
    }
}

#[test]
fn color_flag_forces_colour_even_off_a_tty() {
    let f = repo();
    assert!(has_ansi(&f.ok(&["log", "-n", "1", "--color"])));
    assert!(has_ansi(&f.ok(&["graph", "-n", "1", "--color"])));
}

#[test]
fn no_color_env_suppresses_colour() {
    let f = repo();
    // Forced on, but NO_COLOR in the environment must still win.
    let out = f.run_no_color_env(&["log", "-n", "1"]);
    assert!(!has_ansi(&String::from_utf8_lossy(&out.stdout)));
}

// -------------------------------------------------------- errors and exits

#[test]
fn outside_a_repository_every_command_fails_cleanly() {
    let f = non_repo();
    for cmd in ["log", "branches", "who", "graph"] {
        let out = f.run(&[cmd]);
        assert!(
            !out.status.success(),
            "{} should exit non-zero outside a repository",
            cmd
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.to_lowercase().contains("not a git repository"),
            "{} gave an unhelpful error: {}",
            cmd,
            err
        );
        assert!(out.stdout.is_empty(), "{} printed rows anyway", cmd);
    }
}

#[test]
fn an_empty_repository_does_not_panic() {
    let f = empty_repo();
    for cmd in ["log", "branches", "who", "graph"] {
        let out = f.run(&[cmd]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !err.contains("panicked"),
            "{} panicked on an empty repository: {}",
            cmd,
            err
        );
    }
}

#[test]
fn unknown_commands_and_flags_exit_two() {
    let f = repo();
    assert_eq!(f.run(&["bogus"]).status.code(), Some(2));
    assert_eq!(f.run(&["log", "--bogus"]).status.code(), Some(2));
    assert_eq!(f.run(&[]).status.code(), Some(2), "no command prints usage");
    // A flag that needs a value but has none must be rejected, not ignored.
    assert_eq!(f.run(&["log", "-n"]).status.code(), Some(2));
    assert_eq!(f.run(&["who", "--limit", "abc"]).status.code(), Some(2));
    assert_eq!(
        f.run(&["branches", "--stale", "soon"]).status.code(),
        Some(2)
    );
}

#[test]
fn help_and_version_succeed_for_every_command() {
    let f = repo();
    for cmd in ["log", "branches", "who", "graph"] {
        let out = f.ok(&[cmd, "--help"]);
        assert!(out.contains("USAGE"), "{} --help lacks usage", cmd);
    }
    assert!(f.ok(&["--help"]).contains("gitlimes"));
    assert!(f.ok(&["--version"]).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_revision_is_never_mistaken_for_a_flag() {
    // Everything the user supplies goes after `--`, so a path that looks like a
    // flag cannot reach git as one.
    let f = repo();
    let out = f.run(&["log", "--no-color", "--", "--not-a-real-flag"]);
    assert!(
        out.status.success(),
        "a path shaped like a flag should be treated as a path, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "no commit touches that path");
}

// ------------------------------------------------------------------- json

#[test]
fn json_emits_one_object_per_line() {
    let f = repo();
    let out = f.ok(&["log", "--json"]);
    assert_eq!(out.lines().count(), MAIN_COMMITS);
    for line in out.lines() {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "not a self-contained object: {}",
            line
        );
    }
}

#[test]
fn json_escapes_quotes_and_backslashes_in_a_subject() {
    // The fixture subject contains a quote and a backslash. If either reached
    // the output raw the line would be invalid JSON, and every consumer would
    // break on exactly the commits that are hardest to notice.
    let f = repo();
    let line = f
        .ok(&["log", "--json"])
        .lines()
        .find(|l| l.contains("pipe"))
        .expect("the tricky subject is present")
        .to_string();

    assert!(
        line.contains(r#"\""#),
        "the quote was not escaped: {}",
        line
    );
    assert!(
        line.contains(r"\\"),
        "the backslash was not escaped: {}",
        line
    );
    assert!(
        line.contains("Přílíš žluťoučký kůň"),
        "non-ASCII should pass through unescaped: {}",
        line
    );
    assert_eq!(
        balanced_braces(&line),
        Some(1),
        "brace nesting is broken: {}",
        line
    );
}

#[test]
fn json_never_contains_a_raw_control_character() {
    let f = repo();
    for args in [
        vec!["log", "--json"],
        vec!["branches", "--json"],
        vec!["who", "--json"],
        vec!["graph", "--json"],
    ] {
        let out = f.ok(&args);
        for line in out.lines() {
            assert!(
                !line.chars().any(|c| (c as u32) < 0x20),
                "{:?} emitted a raw control character",
                args
            );
        }
    }
}

#[test]
fn json_output_is_never_coloured() {
    // --color forces colour on, but a JSON consumer would choke on escapes.
    let f = repo();
    for args in [
        vec!["log", "--json", "--color"],
        vec!["branches", "--json", "--color"],
        vec!["who", "--json", "--color"],
        vec!["graph", "--json", "--color"],
    ] {
        assert!(!has_ansi(&f.ok(&args)), "{:?} emitted colour", args);
    }
}

#[test]
fn log_json_carries_the_fields_a_consumer_needs() {
    let f = repo();
    let out = f.ok(&["log", "-n", "1", "--json"]);
    let line = out.lines().next().unwrap();
    for key in [
        r#""hash":"#,
        r#""short":"#,
        r#""parents":"#,
        r#""author":"#,
        r#""date":"#,
        r#""refs":"#,
        r#""subject":"#,
    ] {
        assert!(line.contains(key), "missing {} in {}", key, line);
    }
    assert!(line.contains(r#""head":true"#), "HEAD is not marked");
    assert!(line.contains(r#""v1.0""#), "the tag should appear in refs");
    assert!(
        !line.contains("tag: ") && !line.contains(" -> "),
        "decorations should be stripped from ref names: {}",
        line
    );
}

#[test]
fn graph_json_carries_lane_geometry() {
    // This is what lets a second renderer draw the graph without re-deriving
    // the layout, so it is the part most worth pinning.
    let f = repo();
    let out = f.ok(&["graph", "--json"]);
    assert_eq!(out.lines().count(), MAIN_COMMITS);

    let merge = out
        .lines()
        .find(|l| l.contains("merge: feature"))
        .expect("the merge commit is present");
    assert!(merge.contains(r#""merge":true"#), "merge not flagged");
    assert!(
        merge.contains(r#""opening":[1]"#),
        "the merge should open lane 1: {}",
        merge
    );

    let root = out
        .lines()
        .find(|l| l.contains("chore: init"))
        .expect("the root commit is present");
    assert!(root.contains(r#""merge":false"#));
    assert!(
        root.contains(r#""parents":[]"#),
        "the root has no parents: {}",
        root
    );
}

#[test]
fn who_json_reports_counts_and_the_sparkline_scale() {
    let f = repo();
    let out = f.ok(&["who", "--json"]);
    assert_eq!(out.lines().count(), 2, "two authors");
    let alice = out.lines().find(|l| l.contains("Alice")).unwrap();
    assert!(alice.contains(&format!(r#""commits":{}"#, ALICE_COMMITS)));
    assert!(
        alice.contains(r#""bucket_seconds":"#),
        "the sparkline scale must be stated, not implied: {}",
        alice
    );
    assert!(alice.contains(r#""activity":["#));
    assert!(
        !alice.contains(r#""added":"#),
        "line counts are opt-in via --lines"
    );
    assert!(f.ok(&["who", "--lines", "--json"]).contains(r#""added":"#));
}

#[test]
fn branches_json_marks_the_current_branch() {
    let f = repo();
    let out = f.ok(&["branches", "--json"]);
    assert_eq!(out.lines().count(), 3);
    assert_eq!(
        out.lines()
            .filter(|l| l.contains(r#""current":true"#))
            .count(),
        1,
        "exactly one branch is checked out"
    );
}

/// Returns the maximum brace depth, or `None` if the braces never balance.
/// Quotes and escapes are honoured so a `{` inside a subject does not count.
fn balanced_braces(s: &str) -> Option<usize> {
    let (mut depth, mut max, mut in_str, mut escaped) = (0i32, 0usize, false, false);
    for ch in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_str => escaped = true,
            '"' => in_str = !in_str,
            '{' if !in_str => {
                depth += 1;
                max = max.max(depth as usize);
            }
            '}' if !in_str => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth == 0 && !in_str {
        Some(max)
    } else {
        None
    }
}

// ------------------------------------------------------------------ pager

#[test]
fn paging_does_not_change_the_output() {
    // `cat` stands in for a real pager: same bytes, just routed through a pipe.
    let f = repo();
    let direct = f.ok(&["log", "--no-pager", "--no-color"]);
    let paged = f.run_env(
        &[("GITLIMES_PAGER", "cat")],
        &["log", "--pager", "--no-color"],
    );
    assert!(paged.status.success());
    assert_eq!(
        String::from_utf8_lossy(&paged.stdout),
        direct,
        "paging altered the output"
    );
}

#[test]
fn a_missing_pager_falls_back_instead_of_failing() {
    // A stale PAGER setting is common and must never break the tool.
    let f = repo();
    let out = f.run_env(
        &[("GITLIMES_PAGER", "definitely-not-a-real-program-xyz")],
        &["log", "-n", "2", "--pager", "--no-color"],
    );
    assert!(
        out.status.success(),
        "an unusable pager should degrade quietly, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 2);
}

#[test]
fn an_empty_pager_setting_disables_paging() {
    let f = repo();
    let out = f.run_env(
        &[("PAGER", "")],
        &["log", "-n", "2", "--pager", "--no-color"],
    );
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 2);
}

#[test]
fn output_is_not_paged_when_redirected() {
    // Tests capture stdout through a pipe, so Auto mode must not page - if it
    // did, a nonexistent pager would be spawned on every ordinary run.
    let f = repo();
    let out = f.run_env(
        &[("GITLIMES_PAGER", "definitely-not-a-real-program-xyz")],
        &["log", "-n", "2", "--no-color"],
    );
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 2);
}

#[test]
fn every_command_accepts_the_pager_flags() {
    let f = repo();
    for cmd in ["log", "branches", "who", "graph"] {
        assert!(f.run(&[cmd, "--no-pager"]).status.success(), "{}", cmd);
        assert!(
            f.run_env(&[("GITLIMES_PAGER", "cat")], &[cmd, "--pager"])
                .status
                .success(),
            "{}",
            cmd
        );
    }
}

// ------------------------------------------------- json schema contract

/// The published field names, per command. Renaming or removing any of these
/// breaks every script written against the tool, silently and at a distance.
///
/// Adding a key is allowed by the contract, so a new one here is a prompt to
/// update this list and note it in CHANGELOG.md - not a failure.
#[test]
fn json_field_names_are_a_stable_contract() {
    let f = repo();

    let cases: [(&[&str], &[&str], &[&str]); 4] = [
        (
            &["log", "-n", "1", "--json"],
            &[
                "hash", "short", "parents", "author", "date", "refs", "subject",
            ],
            &["head"],
        ),
        (
            &["branches", "--json"],
            &["name", "current", "date", "author", "subject"],
            &["track"],
        ),
        (
            &["who", "--json"],
            &[
                "name",
                "email",
                "commits",
                "first",
                "last",
                "activity",
                "bucket_seconds",
            ],
            &["added", "removed"],
        ),
        (
            &["graph", "-n", "1", "--json"],
            &[
                "hash", "short", "parents", "author", "date", "refs", "subject", "graph",
            ],
            &["head"],
        ),
    ];

    for (args, required, optional) in cases {
        let out = f.ok(args);
        let line = out.lines().next().expect("at least one record");
        let keys = top_level_keys(line);

        for key in required {
            assert!(
                keys.iter().any(|k| k == key),
                "{:?} lost the required key {:?}; keys were {:?}",
                args,
                key,
                keys
            );
        }
        for key in &keys {
            assert!(
                required.contains(&key.as_str()) || optional.contains(&key.as_str()),
                "{:?} grew an undocumented key {:?}: add it to this list and to CHANGELOG.md",
                args,
                key
            );
        }
    }
}

#[test]
fn graph_json_geometry_keys_are_stable() {
    let f = repo();
    let out = f.ok(&["graph", "-n", "1", "--json"]);
    for key in ["col", "lanes", "merge", "closing", "opening", "shifts"] {
        assert!(
            out.contains(&format!("\"{}\":", key)),
            "graph geometry lost {:?}; a renderer depends on it",
            key
        );
    }
}

/// Top-level keys of a JSON object: the names before each `:` at depth 1.
///
/// Written by hand because the project has no dependencies. It tracks whether
/// the next string is a key or a value, so `"author":"Alice"` yields `author`
/// and not `Alice`, and it ignores anything nested or inside a string.
fn top_level_keys(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    // Commas inside an array separate elements, not key/value pairs.
    let mut array_depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    // At depth 1 a string is a key until a ':' says the next one is a value.
    let mut next_string_is_key = false;

    for ch in text.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if in_string {
            match ch {
                '\\' => escaped = true,
                '"' => {
                    in_string = false;
                    if depth == 1 && next_string_is_key {
                        keys.push(std::mem::take(&mut current));
                    }
                }
                c => current.push(c),
            }
            continue;
        }
        match ch {
            '"' => {
                current.clear();
                in_string = true;
            }
            '{' => {
                depth += 1;
                next_string_is_key = depth == 1 && array_depth == 0;
            }
            '}' => depth -= 1,
            '[' => {
                array_depth += 1;
                next_string_is_key = false;
            }
            ']' => array_depth -= 1,
            ',' => next_string_is_key = depth == 1 && array_depth == 0,
            ':' => next_string_is_key = false,
            _ => {}
        }
    }
    keys
}

#[test]
fn the_key_scanner_reads_keys_and_not_values() {
    // The contract test is only as trustworthy as this helper.
    assert_eq!(
        top_level_keys(r#"{"a":"x","b":1,"c":{"inner":2},"d":["e","f"]}"#),
        vec!["a", "b", "c", "d"],
        "nested keys and string values must not be counted"
    );
    assert_eq!(
        top_level_keys(r#"{"tricky":"has : and , and \" inside"}"#),
        vec!["tricky"],
        "punctuation inside a string must not confuse the scan"
    );
    assert!(top_level_keys("{}").is_empty());
}

// --------------------------------------------------- audit regressions

#[test]
fn a_zero_limit_prints_a_header_instead_of_crashing() {
    // `--limit 0` is accepted by the parser, so it must not then panic. The
    // guard against an empty author list ran before the truncation that
    // emptied it.
    let f = repo();
    let out = f.run(&["who", "--limit", "0", "--no-color", "--no-pager"]);
    assert!(
        out.status.success(),
        "who --limit 0 exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(f.run(&["who", "--limit=0"]).status.success());
}

#[test]
fn stale_rejects_values_that_cannot_mean_a_threshold() {
    // A negative threshold puts the cutoff in the future and marks everything
    // stale; a huge one overflowed the multiply into seconds.
    let f = repo();
    for bad in ["-1", "999999999999999", "1.5", "abc"] {
        let out = f.run(&["branches", "--stale", bad, "--no-color", "--no-pager"]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "--stale {} should be rejected, got {:?}",
            bad,
            out.status.code()
        );
    }
    assert!(f.run(&["branches", "--stale", "0"]).status.success());
    assert!(f
        .run(&["branches", "--stale", "4294967295"])
        .status
        .success());
}

#[test]
fn global_flags_do_not_eat_operands_after_a_double_dash() {
    // A path may legitimately be spelled `--color`. Stripping it as a global
    // flag both changed the colour and silently dropped the path filter.
    let f = repo();
    let out = f.run(&["log", "--no-color", "--", "--color"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !has_ansi(&text),
        "the operand was consumed as a colour flag"
    );
    assert!(
        text.trim().is_empty(),
        "no commit touches a path named --color, got: {}",
        text
    );

    let paged = f.run(&["log", "--no-pager", "--", "--pager"]);
    assert!(paged.status.success());
    assert!(String::from_utf8_lossy(&paged.stdout).trim().is_empty());
}

#[test]
fn boolean_flags_reject_an_inline_value() {
    // `--json=false` silently enabled JSON, because the value was parsed off
    // and then discarded.
    let f = repo();
    for bad in ["--json=false", "--oneline=no", "--all=0"] {
        let out = f.run(&["log", bad, "--no-color", "--no-pager"]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "log {} should be rejected, got {:?}",
            bad,
            out.status.code()
        );
    }
    assert!(f.run(&["branches", "--json=false"]).status.code() == Some(2));
    assert!(f.run(&["who", "--lines=maybe"]).status.code() == Some(2));
    assert!(f.run(&["graph", "--ascii=yes"]).status.code() == Some(2));
}

#[test]
fn only_the_checked_out_commit_is_marked_head() {
    // `head` came from a substring test, so `origin/HEAD` and any branch named
    // like HEAD also claimed to be checked out.
    let f = Fixture::new("head-substring");
    f.git(&["branch", "HEADROOM", "main~2"]);
    f.git(&["branch", "SUBHEADING", "main~3"]);

    let out = f.ok(&["log", "--all", "--json"]);
    let marked: Vec<&str> = out
        .lines()
        .filter(|l| l.contains(r#""head":true"#))
        .collect();
    assert_eq!(
        marked.len(),
        1,
        "exactly one commit is checked out, got {}:\n{}",
        marked.len(),
        marked.join("\n")
    );
    assert!(
        marked[0].contains("chore: release"),
        "the wrong commit was marked: {}",
        marked[0]
    );
}

// ------------------------------------------------------- pager resolution

/// Copies the gitlimes binary itself into `dir` and returns the new path.
///
/// Using the binary under test as the pager keeps this portable: it is a real
/// executable on every platform, and `--version` gives it output of its own
/// that cannot be confused with a log line.
fn pager_stub(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).expect("create pager dir");
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let dest = dir.join(format!("stubpager{}", ext));
    std::fs::copy(env!("CARGO_BIN_EXE_gitlimes"), &dest).expect("copy pager stub");
    dest
}

#[test]
fn a_pager_path_containing_spaces_still_runs() {
    // Splitting the setting on whitespace turned `C:\Program Files\...` into a
    // program plus arguments, which failed to spawn, which silently disabled
    // paging. On Windows most pager paths look like that.
    let f = repo();
    let spaced = f.path().join("pager dir with spaces");
    let stub = pager_stub(&spaced);
    assert!(
        stub.to_string_lossy().contains(' '),
        "the fixture path must contain a space"
    );

    let out = f.run_env(
        &[("GITLIMES_PAGER", &format!("{} --version", stub.display()))],
        &["log", "-n", "2", "--pager", "--no-color"],
    );
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("gitlimes "),
        "the pager never ran; output fell back to the log:\n{}",
        text
    );
}

#[test]
fn a_quoted_pager_path_may_still_take_arguments() {
    let f = repo();
    let spaced = f.path().join("another spaced dir");
    let stub = pager_stub(&spaced);

    let out = f.run_env(
        &[(
            "GITLIMES_PAGER",
            &format!("\"{}\" --version", stub.display()),
        )],
        &["log", "-n", "2", "--pager", "--no-color"],
    );
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("gitlimes "),
        "quoting a spaced path should keep the arguments working"
    );
}

#[test]
fn a_failing_command_does_not_open_a_pager() {
    // The pager used to be spawned before the first record was read, so a
    // command that was going to fail still opened one - over the top of git's
    // error message.
    let f = non_repo();
    let stub = pager_stub(&f.path().join("stub"));

    let out = f.run_env(
        &[("GITLIMES_PAGER", &format!("{} --version", stub.display()))],
        &["log", "--pager", "--no-color"],
    );
    assert!(!out.status.success(), "the command itself must still fail");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !text.contains("gitlimes "),
        "a pager was spawned for a command that produced nothing:\n{}",
        text
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .to_lowercase()
            .contains("not a git repository"),
        "git's own message must survive"
    );
}

#[test]
fn control_bytes_in_a_subject_survive_intact() {
    // git will store 0x1f and 0x1e in a commit subject if you ask it to, and
    // the record format used to be delimited by exactly those bytes - so a
    // subject containing one was silently truncated at it. NUL is used now,
    // which git rejects outright ("a NUL byte in commit log message not
    // allowed"), and the log is framed by newlines, which no field can carry.
    let f = Fixture::new("control-bytes");
    let unit = format!("before{}after unit-sep", '\u{1f}');
    let record = format!("before{}after record-sep", '\u{1e}');
    f.git(&["commit", "-q", "--allow-empty", "-m", &unit]);
    f.git(&["commit", "-q", "--allow-empty", "-m", &record]);

    let out = f.ok(&["log", "-n", "2", "--no-color", "--no-pager"]);
    assert!(
        out.contains(&record),
        "a subject containing 0x1e was mangled:\n{}",
        out
    );
    assert!(
        out.contains(&unit),
        "a subject containing 0x1f was mangled:\n{}",
        out
    );
    assert_eq!(out.lines().count(), 2, "the records must not split");

    // The JSON path must escape them rather than emit raw control bytes.
    let json = f.ok(&["log", "-n", "2", "--json"]);
    assert_eq!(json.lines().count(), 2);
    for line in json.lines() {
        assert!(
            !line.chars().any(|c| (c as u32) < 0x20),
            "a raw control byte reached the JSON output: {}",
            line
        );
    }
    assert!(
        json.contains("\\u001f") && json.contains("\\u001e"),
        "the control bytes should appear as unicode escapes"
    );
}

#[test]
fn a_branch_named_head_is_listed_and_the_symbolic_ref_is_not() {
    // `refs/remotes/origin/HEAD` shortens to `origin`, not `origin/HEAD`, so a
    // name test never caught the symbolic ref it was written for — while
    // `topic/HEAD`, a perfectly legal branch, was thrown away by it.
    let f = Fixture::new("symref");
    f.git(&["branch", "topic/HEAD"]);
    f.git(&["update-ref", "refs/remotes/origin/main", "main"]);
    f.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);

    let out = f.ok(&["branches", "-a", "--no-color", "--no-pager"]);
    assert!(
        out.contains("topic/HEAD"),
        "a real branch named like HEAD was dropped:\n{}",
        out
    );
    assert!(
        out.contains("origin/main"),
        "the real remote branch should be listed:\n{}",
        out
    );

    // The symbolic ref appears as a bare `origin` column entry; it must not.
    let names: Vec<&str> = out
        .lines()
        .filter_map(|l| l.split_whitespace().next_back().map(|_| l))
        .map(|l| l.trim_start_matches('*').trim())
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(
        !names.contains(&"origin"),
        "the symbolic ref leaked in as a branch: {:?}",
        names
    );

    // And the same through --json, which is what a script would read.
    let json = f.ok(&["branches", "-a", "--json"]);
    assert!(json.contains(r#""name":"topic/HEAD""#), "json: {}", json);
    assert!(!json.contains(r#""name":"origin""#), "json: {}", json);
}
