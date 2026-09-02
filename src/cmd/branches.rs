use crate::cli;
use gitlimes::fmt::{fit, parse_ts, rel_compact};
use gitlimes::repo;
use gitlimes::style::{c, BOLD, CYAN, DIM, GREEN, RED, RESET, YELLOW};
use std::io::{self, Write};

pub const HELP: &str = "\
gitlimes branches - branch overview

USAGE:
    gitlimes branches [OPTIONS]

OPTIONS:
    -a, --all            include remote-tracking branches
        --stale <DAYS>   mark branches untouched for longer than DAYS
        --vs <REF>       show ahead/behind against REF instead of the upstream
    -h, --help           show this help
";

/// Ref count is bounded by the repository's branch list, so this command
/// collects rather than streams - that is what lets the name column be sized to
/// the actual content instead of a guess.
const FIELDS: &str = "%(HEAD)%1f%(refname:short)%1f%(committerdate:unix)%1f%(authorname)%1f%(upstream:track)%1f%(subject)";

/// With `--vs`, ahead/behind is requested as a seventh field so the whole
/// listing still costs exactly one git process.
fn format_arg(vs: Option<&str>) -> String {
    match vs {
        Some(base) => format!("--format={}%1f%(ahead-behind:{})", FIELDS, base),
        None => format!("--format={}", FIELDS),
    }
}

fn for_each_ref(format: &str, all: bool) -> Vec<&str> {
    let mut argv = vec![
        "for-each-ref",
        format,
        "--sort=-committerdate",
        "refs/heads",
    ];
    if all {
        argv.push("refs/remotes");
    }
    argv
}

struct Branch {
    current: bool,
    name: String,
    ts: i64,
    author: String,
    track: String,
    subject: String,
}

#[derive(Default)]
struct Opts {
    all: bool,
    stale_days: Option<i64>,
    vs: Option<String>,
}

fn parse(args: Vec<String>) -> Result<Option<Opts>, String> {
    let mut o = Opts::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let (key, inline) = match cli::split_eq(&a) {
            Some((k, v)) if k.starts_with("--") => (k.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let value = |it: &mut std::vec::IntoIter<String>| match inline.clone() {
            Some(v) => Ok(v),
            None => it.next().ok_or_else(|| format!("{} needs a value", key)),
        };
        match key.as_str() {
            "-h" | "--help" => {
                print!("{}", HELP);
                return Ok(None);
            }
            "-a" | "--all" => o.all = true,
            "--stale" => {
                let v = value(&mut it)?;
                o.stale_days = Some(
                    v.parse::<i64>()
                        .map_err(|_| format!("--stale expects a number of days, got '{}'", v))?,
                );
            }
            "--vs" => o.vs = Some(value(&mut it)?),
            s => return Err(cli::Unknown(s.to_string()).to_string()),
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

    // `%(ahead-behind:)` landed in git 2.41. Rather than parse `git --version`,
    // which says nothing about how a distribution built it, just ask for the
    // field and see whether git accepts it.
    let plain = format_arg(None);
    let comparing = o.vs.as_deref().map(|base| format_arg(Some(base)));
    let (raw, inline_track) = match &comparing {
        Some(f) => match repo::probe(&for_each_ref(f, o.all))? {
            Some(raw) => (raw, true),
            None => (repo::capture(&for_each_ref(&plain, o.all))?, false),
        },
        None => (repo::capture(&for_each_ref(&plain, o.all))?, false),
    };

    let mut branches = Vec::new();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        let mut f = line.split(repo::FS);
        let (Some(head), Some(name), Some(ts), Some(author), Some(upstream), Some(subject)) =
            (f.next(), f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        // for-each-ref lists the symbolic HEAD alongside real branches.
        if name.ends_with("/HEAD") {
            continue;
        }
        let track = if inline_track {
            // `ahead behind` - the reverse of `rev-list --left-right`. A ref
            // unrelated to the base yields an empty field, not two numbers.
            let mut n = f.next().unwrap_or("").split_whitespace();
            describe(n.next().unwrap_or("0"), n.next().unwrap_or("0"))
        } else {
            clean_track(upstream)
        };
        branches.push(Branch {
            current: head.trim() == "*",
            name: name.to_string(),
            ts: parse_ts(ts),
            author: author.to_string(),
            track,
            subject: subject.to_string(),
        });
    }

    if branches.is_empty() {
        eprintln!("gitlimes: no branches");
        return Ok(());
    }

    // Only reached on git < 2.41: the same answer at one process per branch.
    if let (Some(base), false) = (&o.vs, inline_track) {
        for b in branches.iter_mut() {
            b.track = compare_to(base, &b.name)?;
        }
    }

    let name_w = branches
        .iter()
        .map(|b| b.name.chars().count())
        .max()
        .unwrap_or(10)
        .clamp(10, 40);
    let track_w = branches
        .iter()
        .map(|b| b.track.chars().count())
        .max()
        .unwrap_or(0)
        .min(20);

    let now = gitlimes::fmt::now_secs();
    let stale_cutoff = o.stale_days.map(|d| now - d * 86_400);

    let stdout = io::stdout();
    let mut w = io::BufWriter::new(stdout.lock());

    for b in &branches {
        let stale = stale_cutoff.is_some_and(|cut| b.ts < cut);
        let marker = if b.current {
            format!("{}*{}", c(CYAN), c(RESET))
        } else {
            " ".to_string()
        };
        let name_color = if b.current {
            BOLD
        } else if b.name.contains('/') && o.all {
            RED
        } else {
            GREEN
        };
        write!(
            w,
            "{} {}{}{}  {}{}{}  {}{}{}",
            marker,
            c(name_color),
            fit(&b.name, name_w),
            c(RESET),
            c(DIM),
            fit(&rel_compact(b.ts), 5),
            c(RESET),
            c(CYAN),
            fit(&b.author, 18),
            c(RESET),
        )?;
        if track_w > 0 {
            write!(w, "  {}{}{}", c(YELLOW), fit(&b.track, track_w), c(RESET))?;
        }
        if stale {
            write!(w, "  {}stale{}", c(RED), c(RESET))?;
        }
        writeln!(w, "  {}{}{}", c(DIM), b.subject, c(RESET))?;
    }
    w.flush()
}

/// `%(upstream:track)` is formatted for humans as `[ahead 2, behind 1]`; strip
/// the brackets so it fits a column.
fn clean_track(track: &str) -> String {
    let t = track.trim();
    let t = t.strip_prefix('[').unwrap_or(t);
    let t = t.strip_suffix(']').unwrap_or(t);
    t.to_string()
}

fn describe(ahead: &str, behind: &str) -> String {
    match (ahead, behind) {
        ("0", "0") => String::new(),
        (a, "0") => format!("ahead {}", a),
        ("0", b) => format!("behind {}", b),
        (a, b) => format!("ahead {}, behind {}", a, b),
    }
}

/// The pre-2.41 fallback: one `rev-list` per branch.
fn compare_to(base: &str, branch: &str) -> io::Result<String> {
    let range = format!("{}...{}", base, branch);
    let out = repo::capture(&["rev-list", "--left-right", "--count", &range, "--"])?;
    let mut n = out.split_whitespace();
    // `--left-right` reports the left side first, which is *behind*.
    let behind = n.next().unwrap_or("0");
    let ahead = n.next().unwrap_or("0");
    Ok(describe(ahead, behind))
}
