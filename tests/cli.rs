//! End-to-end tests: they run the real built binary against a real git
//! repository, which is the only way to cover `repo.rs` (the streaming record
//! reader) and the argument parsing in `cmd/`.

mod fixture;

use fixture::{
    empty_repo, graph_shape, has_ansi, non_repo, repo, strip_ansi, ALICE_COMMITS, BOB_COMMITS,
    BODY_MARKER, MAIN_COMMITS, SIDE_BRANCH, TRICKY_SUBJECT,
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
    let f = repo();
    let out = f.ok(&["branches", "--vs", "main", "--no-color"]);
    let stale = out
        .lines()
        .find(|l| l.contains(SIDE_BRANCH))
        .expect("the side branch is listed");
    // It forked at main~2 and added one commit of its own.
    assert!(
        stale.contains("ahead 1") && stale.contains("behind"),
        "expected ahead/behind counts, got: {}",
        stale
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
