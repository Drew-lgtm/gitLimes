use crate::cli;
use gitlimes::fmt::{fit, parse_ts, rel_compact, short_email, span_label, sparkline};
use gitlimes::json::Obj;
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
        --json           one JSON object per author (newline delimited)
    -h, --help           show this help

The activity sparkline spans the whole history shown, oldest block first.
";

/// The activity sparkline spans the whole history rather than a fixed recent
/// window, so a repository that went quiet years ago still shows its shape
/// instead of twelve blanks.
///
/// The span is not known up front - git yields commits newest first, so the
/// oldest is seen last - and remembering every timestamp to find it would make
/// memory scale with history, which is the one thing this tool does not do.
/// Instead the buckets start one day wide and double whenever a commit falls
/// off the left edge, merging neighbours as they go. That is a fixed
/// `BUCKETS` slots per author, one pass, and it ends up spanning the full
/// history at the coarsest resolution that fits.
const BUCKETS: usize = 12;
const START_WIDTH_SECS: i64 = 86_400;

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
    json: bool,
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
            "--json" => o.json = true,
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
    let mut total = 0u32;
    // The newest commit anchors the sparkline; buckets grow to reach the oldest.
    let mut anchor: Option<i64> = None;
    let mut width = START_WIDTH_SECS;

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

        // Settle the scale before touching any author, so widening - which
        // rewrites every author's buckets - has the map to itself.
        //
        // Author dates do not have to descend the way commit dates do, so a
        // commit newer than the anchor is clamped into the first bucket rather
        // than producing a negative index.
        let anchor = *anchor.get_or_insert(ts);
        let age = (anchor - ts).max(0);
        while age / width >= BUCKETS as i64 {
            for a in authors.values_mut() {
                widen(&mut a.activity);
            }
            width *= 2;
        }
        let bucket = (age / width) as usize;

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
        e.activity[bucket.min(BUCKETS - 1)] += 1;

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

    if o.json {
        let stdout = io::stdout();
        let mut w = io::BufWriter::new(stdout.lock());
        for a in &list {
            let mut obj = Obj::new();
            obj.str("name", &a.name)
                .str("email", &a.email)
                .num("commits", a.commits as i64)
                .num("first", a.first)
                .num("last", a.last)
                // Oldest bucket first, matching the rendered sparkline.
                .nums("activity", a.activity.iter().rev().map(|v| *v as i64))
                .num("bucket_seconds", width);
            if o.lines {
                obj.num("added", a.added as i64)
                    .num("removed", a.removed as i64);
            }
            writeln!(w, "{}", obj.finish())?;
        }
        return w.flush();
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
    // Say what a block is worth: the scale adapts to the history, so leaving
    // it implicit would make two repositories look comparable when they are not.
    writeln!(
        w,
        "  ACTIVITY (oldest to newest, 1 block = {})",
        span_label(width)
    )?;

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

/// Halves the resolution: each pair of neighbouring buckets becomes one, so the
/// same slots now cover twice the time. Counts are preserved, never dropped.
fn widen(activity: &mut [u32; BUCKETS]) {
    let mut merged = [0u32; BUCKETS];
    for i in 0..BUCKETS / 2 {
        merged[i] = activity[2 * i] + activity[2 * i + 1];
    }
    *activity = merged;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widening_halves_resolution_without_losing_commits() {
        let mut a = [1u32, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let before: u32 = a.iter().sum();
        widen(&mut a);
        assert_eq!(a[..6], [3, 7, 11, 15, 19, 23], "neighbours merge in pairs");
        assert_eq!(a[6..], [0; 6], "the freed half is cleared");
        assert_eq!(a.iter().sum::<u32>(), before, "no commit is dropped");
    }

    #[test]
    fn repeated_widening_still_conserves_the_total() {
        let mut a = [0u32; BUCKETS];
        a[0] = 5;
        a[BUCKETS - 1] = 7;
        for _ in 0..4 {
            widen(&mut a);
        }
        assert_eq!(a.iter().sum::<u32>(), 12);
        assert_eq!(a[0], 12, "everything collapses into the newest bucket");
    }

    /// The scale must be chosen so the oldest commit lands inside the window.
    #[test]
    fn buckets_widen_until_the_whole_span_fits() {
        let day = 86_400i64;
        for span_days in [1i64, 5, 30, 400, 4_000] {
            let mut width = START_WIDTH_SECS;
            while span_days * day / width >= BUCKETS as i64 {
                width *= 2;
            }
            let bucket = span_days * day / width;
            assert!(
                bucket < BUCKETS as i64,
                "a {}-day span still overflows at width {}",
                span_days,
                width
            );
        }
    }
}
