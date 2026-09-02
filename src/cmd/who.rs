use crate::cli;
use gitlimes::fmt::{fit, now_secs, parse_ts, rel_compact, short_email, sparkline};
use gitlimes::repo::{self, Records, WHO_FORMAT};
use gitlimes::style::{c, BOLD, CYAN, DIM, GREEN, RESET, YELLOW};
use std::collections::HashMap;
use std::io::{self, Write};

pub const HELP: &str = "\
gitlimes who - author and contribution stats

USAGE:
    gitlimes who [PATH] [OPTIONS]

OPTIONS:
        --since <DATE>   only count commits more recent than DATE
        --limit <N>      show only the top N authors
        --lines          also count added/removed lines (slower)
        --all            include all refs, not just HEAD
    -h, --help           show this help

The activity sparkline covers the last 12 months, one block per month.
";

/// Buckets are 30-day windows counted back from now, which keeps the whole
/// command to a single pass with no calendar arithmetic.
const BUCKETS: usize = 12;
const BUCKET_SECS: i64 = 30 * 86_400;

#[derive(Default)]
struct Author {
    name: String,
    email: String,
    commits: u32,
    first: i64,
    last: i64,
    added: u64,
    removed: u64,
    activity: [u32; BUCKETS],
}

#[derive(Default)]
struct Opts {
    since: Option<String>,
    limit: Option<usize>,
    lines: bool,
    all: bool,
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
            "--since" => o.since = Some(value(&mut it)?),
            "--limit" => {
                let v = value(&mut it)?;
                o.limit = Some(
                    v.parse::<usize>()
                        .map_err(|_| format!("--limit expects a number, got '{}'", v))?,
                );
            }
            "--lines" => o.lines = true,
            "--all" => o.all = true,
            s if s.starts_with('-') => return Err(cli::Unknown(s.to_string()).to_string()),
            s => o.paths.push(s.to_string()),
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

    let mut argv: Vec<String> = vec!["log".into(), WHO_FORMAT.into()];
    if let Some(s) = &o.since {
        argv.push(format!("--since={}", s));
    }
    if o.all {
        argv.push("--all".into());
    }
    if o.lines {
        argv.push("--numstat".into());
    }
    argv.push("--".into());
    argv.extend(o.paths.iter().cloned());

    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut rec = Records::spawn(repo::git(&refs))?;

    // Bounded by the number of distinct authors, not by history length.
    let mut authors: HashMap<String, Author> = HashMap::new();
    let now = now_secs();
    let mut total = 0u32;

    while let Some(record) = rec.next_record()? {
        // With --numstat, git appends this commit's stat lines after the
        // fields; the leading separator keeps them inside the same record.
        let (head, stats) = match record.split_once('\n') {
            Some((h, rest)) => (h, Some(rest)),
            None => (record.as_ref(), None),
        };
        let mut f = head.split(repo::FS);
        let (Some(name), Some(email), Some(ts)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let ts = parse_ts(ts);
        let key = email.to_ascii_lowercase();
        let e = authors.entry(key).or_insert_with(|| Author {
            name: name.to_string(),
            email: email.to_string(),
            first: ts,
            last: ts,
            ..Default::default()
        });
        e.commits += 1;
        total += 1;
        e.first = e.first.min(ts);
        e.last = e.last.max(ts);
        let age = now - ts;
        if age >= 0 {
            let bucket = (age / BUCKET_SECS) as usize;
            if bucket < BUCKETS {
                e.activity[bucket] += 1;
            }
        }
        if let Some(stats) = stats {
            let (a, r) = sum_numstat(stats);
            e.added += a;
            e.removed += r;
        }
    }
    rec.finish()?;

    if authors.is_empty() {
        eprintln!("gitlimes: no commits");
        return Ok(());
    }

    let mut list: Vec<Author> = authors.into_values().collect();
    list.sort_by(|a, b| b.commits.cmp(&a.commits).then(b.last.cmp(&a.last)));
    if let Some(n) = o.limit {
        list.truncate(n);
    }

    let name_w = list
        .iter()
        .map(|a| a.name.chars().count())
        .max()
        .unwrap_or(12)
        .clamp(12, 28);
    let count_w = list[0].commits.to_string().len().max(7);
    // One person often commits under several identities; the email is what
    // actually distinguishes them, so it gets its own column.
    let mail_w = list
        .iter()
        .map(|a| short_email(&a.email).chars().count())
        .max()
        .unwrap_or(10)
        .clamp(10, 24);

    let stdout = io::stdout();
    let mut w = io::BufWriter::new(stdout.lock());

    write!(
        w,
        "{}{}  {}  {}  {}  {}  {}{}",
        c(BOLD),
        fit("COMMITS", count_w),
        fit("AUTHOR", name_w),
        fit("EMAIL", mail_w),
        fit("SHARE", 5),
        fit("LAST", 5),
        fit("FIRST", 5),
        c(RESET),
    )?;
    if o.lines {
        write!(w, "  {}  {}", fit("+LINES", 8), fit("-LINES", 8))?;
    }
    writeln!(w, "  ACTIVITY (12mo, newest right)")?;

    for a in &list {
        let share = format!("{:.0}%", (a.commits as f64 / total as f64) * 100.0);
        write!(
            w,
            "{}{}{}  {}{}{}  {}{}{}  {}{}{}  {}  {}  ",
            c(YELLOW),
            fit(&a.commits.to_string(), count_w),
            c(RESET),
            c(CYAN),
            fit(&a.name, name_w),
            c(RESET),
            c(DIM),
            fit(short_email(&a.email), mail_w),
            c(RESET),
            c(DIM),
            fit(&share, 5),
            c(RESET),
            fit(&rel_compact(a.last), 5),
            fit(&rel_compact(a.first), 5),
        )?;
        if o.lines {
            write!(
                w,
                "{}{}{}  {}{}{}  ",
                c(GREEN),
                fit(&format!("+{}", a.added), 8),
                c(RESET),
                c(DIM),
                fit(&format!("-{}", a.removed), 8),
                c(RESET),
            )?;
        }
        // Oldest bucket first so the line reads left-to-right as time passing.
        let mut series = a.activity;
        series.reverse();
        writeln!(w, "{}{}{}", c(GREEN), sparkline(&series), c(RESET))?;
    }
    w.flush()
}

/// numstat rows are `added<TAB>removed<TAB>path`, with `-` for binary files.
fn sum_numstat(block: &str) -> (u64, u64) {
    let mut added = 0;
    let mut removed = 0;
    for line in block.lines() {
        let mut f = line.split('\t');
        let (Some(a), Some(r)) = (f.next(), f.next()) else {
            continue;
        };
        added += a.parse::<u64>().unwrap_or(0);
        removed += r.parse::<u64>().unwrap_or(0);
    }
    (added, removed)
}
