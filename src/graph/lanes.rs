//! Streaming lane assignment.
//!
//! A "lane" is a vertical column in the drawing. Each lane remembers the single
//! commit hash it is currently waiting to see. Because the state is one entry
//! per *open* lane rather than one per commit, memory is bounded by how many
//! branches are simultaneously in flight - typically a handful - and never by
//! the length of the history.

/// One open lane: the commit it waits for, plus a colour that stays with it for
/// its whole life so a branch keeps one colour even as it moves columns.
#[derive(Clone, Debug, PartialEq)]
struct Lane {
    waiting_for: String,
    color: usize,
}

/// What happened at one commit, in terms the renderer can draw without knowing
/// anything about git.
///
/// A commit occupies up to four rows, and the occupancy snapshots say which
/// lanes are drawn on each:
///
/// ```text
/// before   |/      lanes fold in from above
/// at       *       the commit itself
/// (at)     |\      lanes branch out below
/// after     /      lanes slide left into freed columns
/// ```
#[derive(Debug, PartialEq)]
pub struct Step {
    /// Column the commit's dot sits in.
    pub col: usize,
    /// Lane occupancy on the fold row, above the dot.
    pub before: Vec<bool>,
    /// Lane occupancy on the dot's own row.
    pub at: Vec<bool>,
    /// Lane occupancy on the shift row, before compaction slides lanes left.
    pub pre_shift: Vec<bool>,
    /// Lane occupancy once branching and compaction are done.
    pub after: Vec<bool>,
    /// Lanes that were also waiting for this commit and fold into `col`.
    /// These are fork points: one parent with two children.
    pub closing: Vec<usize>,
    /// Lanes this commit's extra parents continue in. An edge is drawn from
    /// `col` out to each of them.
    pub opening: Vec<usize>,
    /// Lanes sliding left to fill freed columns, as `(from, to)` pairs. Every
    /// move is exactly one column, so the shift reads as a single diagonal.
    pub shifts: Vec<(usize, usize)>,
    /// True when the commit has more than one parent.
    pub is_merge: bool,
    /// Lane colours by column, before compaction.
    pub colors: Vec<usize>,
    /// Lane colours by column, after compaction.
    pub colors_after: Vec<usize>,
    /// Colour of the commit's own dot.
    pub dot_color: usize,
}

impl Step {
    pub fn width(&self) -> usize {
        self.before
            .len()
            .max(self.at.len())
            .max(self.pre_shift.len())
            .max(self.after.len())
    }
}

#[derive(Default)]
pub struct Lanes {
    slots: Vec<Option<Lane>>,
    next_color: usize,
}

impl Lanes {
    pub fn new() -> Lanes {
        Lanes::default()
    }

    /// Number of currently open lanes. This is the whole of the graph's state.
    pub fn open(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Highest column in use. Compaction keeps this close to `open()`; without
    /// it, closed lanes would leave holes and the drawing would drift right.
    pub fn width(&self) -> usize {
        self.slots.iter().rposition(|s| s.is_some()).map_or(0, |i| i + 1)
    }

    pub fn advance(&mut self, hash: &str, parents: &[&str]) -> Step {
        let col = match self.find(hash) {
            Some(i) => i,
            // A commit nothing is waiting for is a branch tip; give it a lane.
            None => self.claim(hash),
        };

        let before = self.snapshot();

        // Any other lane waiting for this same commit is a second child; it
        // folds into this column.
        let mut closing = Vec::new();
        for j in 0..self.slots.len() {
            if j != col && self.slots[j].as_ref().is_some_and(|l| l.waiting_for == hash) {
                self.slots[j] = None;
                closing.push(j);
            }
        }

        let at = self.snapshot();
        let dot_color = self.slots[col].as_ref().map_or(0, |l| l.color);

        // The first parent inherits this lane - keeping its colour - so
        // mainline history stays in one straight, consistently coloured column.
        match parents.first() {
            Some(p) => {
                if let Some(lane) = self.slots[col].as_mut() {
                    lane.waiting_for = (*p).to_string();
                }
            }
            None => self.slots[col] = None,
        }

        let mut opening = Vec::new();
        for p in parents.iter().skip(1) {
            let k = match self.find(p) {
                Some(k) => k,
                None => self.claim(p),
            };
            if k != col {
                opening.push(k);
            }
        }

        let colors = self.color_map();
        let pre_shift = self.snapshot();
        let shifts = self.compact();

        Step {
            col,
            before,
            at,
            pre_shift,
            after: self.snapshot(),
            closing,
            opening,
            shifts,
            is_merge: parents.len() > 1,
            colors,
            colors_after: self.color_map(),
            dot_color,
        }
    }

    /// Slides every lane one column left where the column to its left is free.
    /// One pass moves each lane at most once, so the shift is always a single
    /// clean diagonal; repeated commits finish the job for wider gaps.
    fn compact(&mut self) -> Vec<(usize, usize)> {
        let mut shifts = Vec::new();
        for i in 0..self.slots.len().saturating_sub(1) {
            if self.slots[i].is_none() && self.slots[i + 1].is_some() {
                self.slots[i] = self.slots[i + 1].take();
                shifts.push((i + 1, i));
            }
        }
        while matches!(self.slots.last(), Some(None)) {
            self.slots.pop();
        }
        shifts
    }

    fn find(&self, hash: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|l| l.waiting_for == hash))
    }

    /// Reuses the leftmost free lane so the drawing stays narrow.
    fn claim(&mut self, hash: &str) -> usize {
        let lane = Lane {
            waiting_for: hash.to_string(),
            color: self.next_color,
        };
        self.next_color = self.next_color.wrapping_add(1);
        match self.slots.iter().position(|s| s.is_none()) {
            Some(i) => {
                self.slots[i] = Some(lane);
                i
            }
            None => {
                self.slots.push(Some(lane));
                self.slots.len() - 1
            }
        }
    }

    /// Occupancy with trailing empties dropped, so row widths track the lanes
    /// actually in use.
    fn snapshot(&self) -> Vec<bool> {
        let mut v: Vec<bool> = self.slots.iter().map(|s| s.is_some()).collect();
        while matches!(v.last(), Some(false)) {
            v.pop();
        }
        v
    }

    fn color_map(&self) -> Vec<usize> {
        self.slots
            .iter()
            .map(|s| s.as_ref().map_or(0, |l| l.color))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_history_stays_in_one_lane() {
        let mut l = Lanes::new();
        let a = l.advance("a", &["b"]);
        let b = l.advance("b", &["c"]);
        let c = l.advance("c", &[]);
        for s in [&a, &b] {
            assert_eq!(s.col, 0);
            assert_eq!(s.before, vec![true]);
            assert_eq!(s.after, vec![true]);
            assert!(s.closing.is_empty() && s.opening.is_empty());
            assert!(s.shifts.is_empty());
        }
        // A root commit closes its lane.
        assert_eq!(c.at, vec![true]);
        assert_eq!(c.after, Vec::<bool>::new());
    }

    #[test]
    fn merge_opens_a_lane_and_fork_closes_it() {
        // m merges mainline `a` with side branch `s`; both reach root `r`.
        let mut l = Lanes::new();

        let m = l.advance("m", &["a", "s"]);
        assert_eq!(m.col, 0);
        assert!(m.is_merge);
        assert_eq!(m.at, vec![true], "the dot sits alone before branching");
        assert_eq!(m.opening, vec![1], "the side parent gets its own lane");
        assert_eq!(m.after, vec![true, true]);

        let a = l.advance("a", &["r"]);
        assert_eq!(a.col, 0);
        assert!(a.opening.is_empty() && a.closing.is_empty());

        let s = l.advance("s", &["r"]);
        assert_eq!(s.col, 1);
        // Both lanes now wait for r, but nothing folds until r is reached.
        assert_eq!(s.after, vec![true, true]);

        let r = l.advance("r", &[]);
        assert_eq!(r.col, 0, "the fork resolves into the leftmost lane");
        assert_eq!(r.before, vec![true, true], "both lanes are live above it");
        assert_eq!(r.closing, vec![1], "the second lane folds in");
        assert_eq!(r.at, vec![true], "and is gone by the dot's own row");
        assert_eq!(l.open(), 0);
    }

    #[test]
    fn octopus_merge_opens_every_extra_parent() {
        let mut l = Lanes::new();
        let m = l.advance("m", &["p1", "p2", "p3"]);
        assert!(m.is_merge);
        assert_eq!(m.opening, vec![1, 2]);
        assert_eq!(m.after, vec![true, true, true]);
        assert_eq!(l.open(), 3);
    }

    #[test]
    fn a_branch_keeps_its_colour_for_life() {
        let mut l = Lanes::new();
        let m = l.advance("m", &["a", "s"]);
        let side = m.colors_after[1];
        assert_ne!(side, m.dot_color, "a new branch gets its own colour");
        l.advance("a", &["r"]);
        let s = l.advance("s", &["r"]);
        assert_eq!(s.dot_color, side, "the side branch keeps that colour");
    }

    #[test]
    fn a_freed_column_is_closed_up_by_shifting() {
        // Lane 0 ends while lane 1 is still live; lane 1 must slide into it
        // rather than leaving a permanent hole.
        let mut l = Lanes::new();
        l.advance("m", &["a", "s"]);
        let a = l.advance("a", &[]);
        assert_eq!(a.shifts, vec![(1, 0)], "the live lane slides left");
        assert_eq!(a.after, vec![true]);
        assert_eq!(l.width(), 1, "no hole is left behind");
    }

    #[test]
    fn compaction_keeps_width_near_the_number_of_live_lanes() {
        // Open five lanes, then close the leftmost ones and confirm the table
        // closes up instead of drifting right.
        let mut l = Lanes::new();
        l.advance("m", &["p0", "p1", "p2", "p3", "p4"]);
        assert_eq!(l.width(), 5);
        for h in ["p0", "p1", "p2"] {
            l.advance(h, &[]);
        }
        // Two lanes remain; a few more commits let compaction finish closing up.
        for h in ["p3", "p4"] {
            l.advance(h, &[&format!("{}-next", h)]);
        }
        assert_eq!(l.open(), 2);
        assert_eq!(l.width(), 2, "width tracks live lanes, not peak lanes");
    }

    #[test]
    fn state_is_bounded_by_open_lanes_not_history() {
        let mut l = Lanes::new();
        // 10k linear commits must never grow the lane table past one entry.
        for i in 0..10_000u32 {
            let h = i.to_string();
            let p = (i + 1).to_string();
            l.advance(&h, &[p.as_str()]);
            assert_eq!(l.open(), 1);
        }
    }
}
