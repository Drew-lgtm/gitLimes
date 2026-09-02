//! Column helpers. Widths are counted in `char`s, which is right for Latin,
//! Cyrillic and Greek names and one column off per glyph for CJK; that is a
//! deliberate trade to avoid a unicode-width dependency.

/// Truncates with an ellipsis or pads with spaces so the result is exactly
/// `w` characters wide.
pub fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n == w {
        return s.to_string();
    }
    if n < w {
        let mut out = String::with_capacity(s.len() + (w - n));
        out.push_str(s);
        out.extend(std::iter::repeat(' ').take(w - n));
        return out;
    }
    if w == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(w - 1).collect();
    out.push('\u{2026}');
    out
}

/// Strips the domain from an email so author columns stay narrow.
pub fn short_email(email: &str) -> &str {
    email.split('@').next().unwrap_or(email)
}

const BLOCKS: [char; 8] = ['\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];

/// Renders counts as a sparkline. An all-zero series renders as blanks rather
/// than a flat bar, so gaps in activity read as gaps.
pub fn sparkline(counts: &[u32]) -> String {
    let max = counts.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return " ".repeat(counts.len());
    }
    counts
        .iter()
        .map(|&c| {
            if c == 0 {
                ' '
            } else {
                let idx = ((c as u64 * 7) / max as u64) as usize;
                BLOCKS[idx.min(7)]
            }
        })
        .collect()
}

/// Seconds since the unix epoch, or 0 if the clock is before it.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Compact age from a unix timestamp: `3d`, `6w`, `9mo`, `2y`.
///
/// git's own `%ar` is written for prose ("2 years, 9 months ago") and is far
/// too wide for a column, so we derive the age from `%at` instead. These are
/// plain divisions on a duration, not calendar arithmetic.
pub fn rel_compact(ts: i64) -> String {
    let d = now_secs() - ts;
    if d < 0 {
        return "future".to_string();
    }
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;
    match d {
        s if s < MIN => format!("{}s", s),
        s if s < HOUR => format!("{}m", s / MIN),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < WEEK => format!("{}d", s / DAY),
        s if s < MONTH => format!("{}w", s / WEEK),
        s if s < YEAR => format!("{}mo", s / MONTH),
        s => format!("{}y", s / YEAR),
    }
}

/// Parses git's `%at` field; a malformed timestamp sorts as the epoch rather
/// than aborting the whole listing.
pub fn parse_ts(s: &str) -> i64 {
    s.trim().parse::<i64>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_pads_and_truncates() {
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcd", 4), "abcd");
        assert_eq!(fit("abcdef", 4), "abc\u{2026}");
        assert_eq!(fit("", 0), "");
    }

    #[test]
    fn fit_counts_chars_not_bytes() {
        // Four multibyte chars must not be cut mid-codepoint.
        assert_eq!(fit("\u{159}\u{161}\u{10d}\u{17e}", 4).chars().count(), 4);
        assert_eq!(fit("\u{159}\u{161}\u{10d}\u{17e}", 2), "\u{159}\u{2026}");
    }

    #[test]
    fn sparkline_marks_zero_as_gap() {
        assert_eq!(sparkline(&[0, 0, 0]), "   ");
        let s = sparkline(&[1, 8]);
        assert_eq!(s.chars().next(), Some('\u{2581}'));
        assert_eq!(s.chars().nth(1), Some('\u{2588}'));
    }

    #[test]
    fn rel_compact_picks_one_unit() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(rel_compact(now - 30), "30s");
        assert_eq!(rel_compact(now - 3 * 3600), "3h");
        assert_eq!(rel_compact(now - 3 * 86400), "3d");
        assert_eq!(rel_compact(now - 60 * 86400), "2mo");
        assert_eq!(rel_compact(now - 800 * 86400), "2y");
    }

    #[test]
    fn parse_ts_tolerates_garbage() {
        assert_eq!(parse_ts("1700000000"), 1700000000);
        assert_eq!(parse_ts("nonsense"), 0);
    }
}
