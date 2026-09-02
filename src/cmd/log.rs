use crate::cli;
use gitlimes::fmt::{fit, parse_ts, rel_compact};
use gitlimes::json::Obj;
use gitlimes::pager;
use gitlimes::repo::{self, Commit, Records, LOG_FORMAT};
use gitlimes::style::{c, BOLD, CYAN, DIM, GREEN, RED, RESET, YELLOW};
use std::io::{self, Write};

pub const HELP: &str = "\
gitlimes log - colored commit history

USAGE:
    gitlimes log [REV] [OPTIONS] [-- PATH...]

OPTIONS:
    -n, --max-count <N>   limit to N commits (passed through to git)
        --author <PAT>    only commits whose author matches PAT
        --since <DATE>    only commits more recent than DATE
        --until <DATE>    only commits older than DATE
        --all             include all refs, not just HEAD
        --oneline         hash and subject only
        --json            one JSON object per commit (newline delimited)
    -h, --help            show this help
";

#[derive(Default)]
struct Opts {
    max: Option<String>,
    author: Option<String>,
    since: Option<String>,
    until: Option<String>,
    all: bool,
    oneline: bool,
    json: bool,
    revs: Vec<String>,
    paths: Vec<String>,
}

fn parse(args: Vec<String>) -> Result<Option<Opts>, String> {
    let mut o = Opts::default();
    let mut it = args.into_iter();
    let mut after_ddash = false;

    while let Some(a) = it.next() {
        if after_ddash {
            o.paths.push(a);
            continue;
        }
        let (key, inline) = match cli::split_eq(&a) {
            Some((k, v)) if k.starts_with("--") => (k.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let value = |it: &mut std::vec::IntoIter<String>| match inline.clone() {
            Some(v) => Ok(v),
            None => it.next().ok_or_else(|| format!("{} needs a value", key)),
        };
        match key.as_str() {
            "--" => after_ddash = true,
            "-h" | "--help" => {
                print!("{}", HELP);
                return Ok(None);
            }
            "-n" | "--max-count" => o.max = Some(value(&mut it)?),
            "--author" => o.author = Some(value(&mut it)?),
            "--since" => o.since = Some(value(&mut it)?),
            "--until" => o.until = Some(value(&mut it)?),
            "--all" => o.all = true,
            "--oneline" => o.oneline = true,
            "--json" => o.json = true,
            s if s.starts_with('-') => return Err(cli::Unknown(s.to_string()).to_string()),
            s => o.revs.push(s.to_string()),
        }
    }
    Ok(Some(o))
}

pub fn run(args: Vec<String>) -> io::Result<()> {
    let o = match parse(args) {
        Ok(None) => return Ok(()),
        Ok(Some(o)) => o,
        Err(e) => {
            eprintln!("gitlimes: {}", e);
            std::process::exit(2);
        }
    };

    let mut argv: Vec<String> = vec!["log".into(), LOG_FORMAT.into()];
    if let Some(n) = &o.max {
        argv.push("-n".into());
        argv.push(n.clone());
    }
    if let Some(a) = &o.author {
        argv.push(format!("--author={}", a));
    }
    if let Some(s) = &o.since {
        argv.push(format!("--since={}", s));
    }
    if let Some(u) = &o.until {
        argv.push(format!("--until={}", u));
    }
    if o.all {
        argv.push("--all".into());
    }
    argv.extend(o.revs.iter().cloned());
    // Always terminate options so a path or rev can never be read as a flag.
    argv.push("--".into());
    argv.extend(o.paths.iter().cloned());

    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut rec = Records::spawn(repo::git(&refs))?;

    let mut w = pager::out();

    while let Some(line) = rec.next_record()? {
        let Some(commit) = Commit::parse(&line) else {
            continue;
        };
        if o.json {
            writeln!(w, "{}", commit_json(&commit).finish())?;
        } else if o.oneline {
            writeln!(
                w,
                "{}{}{} {}",
                c(YELLOW),
                commit.short,
                c(RESET),
                commit.subject
            )?;
        } else {
            writeln!(
                w,
                "{}{}{}  {}{}{}  {}{}{}  {}{}",
                c(YELLOW),
                commit.short,
                c(RESET),
                c(DIM),
                fit(&rel_compact(parse_ts(commit.timestamp)), 5),
                c(RESET),
                c(CYAN),
                fit(commit.author, 18),
                c(RESET),
                decorate(commit.refs),
                commit.subject
            )?;
        }
    }
    w.finish()?;
    rec.finish()
}

/// Colours `%D` output: HEAD bold cyan, tags yellow, remotes red, locals green.
fn decorate(refs: &str) -> String {
    if refs.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push('(');
    for (i, r) in refs.split(", ").enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let color = if r.starts_with("HEAD") {
            BOLD
        } else if r.starts_with("tag: ") {
            YELLOW
        } else if r.starts_with("origin/") || r.starts_with("upstream/") {
            RED
        } else {
            GREEN
        };
        out.push_str(c(color));
        out.push_str(r);
        out.push_str(c(RESET));
    }
    out.push_str(") ");
    out
}

/// The shared commit shape for `--json`, also used by `graph`.
pub fn commit_json(commit: &Commit<'_>) -> Obj {
    let mut o = Obj::new();
    o.str("hash", commit.hash)
        .str("short", commit.short)
        .strs("parents", commit.parent_iter())
        .str("author", commit.author)
        .num("date", parse_ts(commit.timestamp))
        .strs("refs", ref_names(commit.refs))
        .str("subject", commit.subject);
    // Which ref is checked out is lost once the decorations are stripped, and
    // a consumer almost always wants it back.
    if commit.refs.contains("HEAD") {
        o.bool("head", true);
    }
    o
}

/// Splits `%D` into individual ref names, dropping git's decoration prefixes so
/// consumers get `main` and `v1.0` rather than `HEAD -> main` and `tag: v1.0`.
fn ref_names(refs: &str) -> impl Iterator<Item = &str> {
    refs.split(", ").filter(|r| !r.is_empty()).map(|r| {
        r.rsplit(" -> ")
            .next()
            .unwrap_or(r)
            .strip_prefix("tag: ")
            .unwrap_or_else(|| r.rsplit(" -> ").next().unwrap_or(r))
    })
}
