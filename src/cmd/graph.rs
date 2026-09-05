use crate::cli;
use crate::cmd::log::commit_json;
use gitlimes::fmt::{fit, parse_ts, rel_compact};
use gitlimes::graph::draw::{branch_row, commit_row, fold_row, shift_row, Glyphs, ASCII, UNICODE};
use gitlimes::graph::lanes::Lanes;
use gitlimes::pager;
use gitlimes::repo::{self, Commit, Records, LOG_FORMAT};
use gitlimes::style::{c, BOLD, CYAN, DIM, GREEN, RED, RESET, YELLOW};
use std::io::{self, Write};

pub const HELP: &str = "\
gitlimes graph - unicode branch graph

USAGE:
    gitlimes graph [REV] [OPTIONS] [-- PATH...]

OPTIONS:
    -n, --max-count <N>   limit to N commits (default 40)
    -a, --all             include all refs, not just HEAD
        --ascii           draw with ASCII instead of box characters
        --json            one JSON object per commit, with lane geometry
        --since <DATE>    only commits more recent than DATE
        --author <PAT>    only commits whose author matches PAT
    -h, --help            show this help

Filled dots are ordinary commits, hollow dots are merges.
";

/// Reserve space for a few lanes so the commit text does not jitter on every
/// branch. Wider graphs push the text right, exactly as `git log --graph` does.
const MIN_GRAPH_CELLS: usize = 7;

#[derive(Default)]
struct Opts {
    max: Option<String>,
    all: bool,
    ascii: bool,
    json: bool,
    since: Option<String>,
    author: Option<String>,
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
            "-a" | "--all" => {
                cli::no_value(&key, &inline)?;
                o.all = true
            }
            "--ascii" => {
                cli::no_value(&key, &inline)?;
                o.ascii = true
            }
            "--json" => {
                cli::no_value(&key, &inline)?;
                o.json = true
            }
            "--since" => o.since = Some(value(&mut it)?),
            "--author" => o.author = Some(value(&mut it)?),
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

    let glyphs: &Glyphs = if o.ascii { &ASCII } else { &UNICODE };

    // topo-order keeps a branch's commits contiguous, which is what makes the
    // lanes read as continuous lines instead of interleaving by date.
    let mut argv: Vec<String> = vec!["log".into(), LOG_FORMAT.into(), "--topo-order".into()];
    argv.push("-n".into());
    argv.push(o.max.clone().unwrap_or_else(|| "40".into()));
    if o.all {
        argv.push("--all".into());
    }
    if let Some(s) = &o.since {
        argv.push(format!("--since={}", s));
    }
    if let Some(a) = &o.author {
        argv.push(format!("--author={}", a));
    }
    argv.extend(o.revs.iter().cloned());
    argv.push("--".into());
    argv.extend(o.paths.iter().cloned());

    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut rec = Records::spawn_log(repo::git(&refs))?;

    let mut lanes = Lanes::new();
    let mut w = pager::out();

    while let Some(line) = rec.next_record()? {
        let Some(commit) = Commit::parse(&line) else {
            continue;
        };
        let parents: Vec<&str> = commit.parent_iter().collect();
        let step = lanes.advance(commit.hash, &parents);
        let width = (step.width() * 2).max(MIN_GRAPH_CELLS);

        if o.json {
            // Lane geometry travels with the commit so another renderer - SVG,
            // a canvas, a TUI - can draw the graph without re-deriving it.
            let mut c = commit_json(&commit);
            let mut g = gitlimes::json::Obj::new();
            g.num("col", step.col as i64)
                .num("lanes", step.width() as i64)
                .bool("merge", step.is_merge)
                .nums("closing", step.closing.iter().map(|v| *v as i64))
                .nums("opening", step.opening.iter().map(|v| *v as i64))
                .raw("shifts", &shifts_json(&step.shifts));
            c.obj("graph", g);
            writeln!(w, "{}", c.finish())?;
            continue;
        }

        // Folds are drawn above the dot, branches below it.
        if let Some(row) = fold_row(&step, glyphs, width) {
            writeln!(w, "{}", row)?;
        }

        writeln!(
            w,
            "{}  {}{}{}  {}{}{}  {}{}{}  {}{}",
            commit_row(&step, glyphs, width),
            c(YELLOW),
            commit.short,
            c(RESET),
            c(DIM),
            fit(&rel_compact(parse_ts(commit.timestamp)), 5),
            c(RESET),
            c(CYAN),
            fit(commit.author, 16),
            c(RESET),
            decorate(commit.refs),
            commit.subject
        )?;

        if let Some(row) = branch_row(&step, glyphs, width) {
            writeln!(w, "{}", row)?;
        }
        if let Some(row) = shift_row(&step, glyphs, width) {
            writeln!(w, "{}", row)?;
        }
    }
    w.finish()?;

    // Say so rather than let an approximation pass for the real thing. Goes to
    // stderr so it never contaminates `--json` or a piped drawing.
    if lanes.evicted() {
        eprintln!(
            "gitlimes: more than {} branches were open at once; some edges are not drawn",
            gitlimes::graph::lanes::MAX_LANES
        );
    }
    rec.finish()
}

/// Colours `%D` output: HEAD bold, tags yellow, remotes red, locals green.
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

/// `[[from,to],...]`, a shape the typed array helpers do not cover.
fn shifts_json(shifts: &[(usize, usize)]) -> String {
    let mut out = String::from("[");
    for (i, (from, to)) in shifts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{},{}]", from, to));
    }
    out.push(']');
    out
}
