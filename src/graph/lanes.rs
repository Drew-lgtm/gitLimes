//! Streaming lane assignment.
//!
//! A "lane" is a vertical column in the drawing. Each lane remembers the single
//! commit hash it is currently waiting to see. Because the state is one entry
//! per *open* lane rather than one per commit, memory is bounded by how many
//! branches are simultaneously in flight - typically a handful - and never by
//! the length of the history.

/// What happened at one commit, in terms the renderer can draw without knowing
/// anything about git.
///
/// A commit occupies up to three rows, and the three occupancy snapshots say
/// which lanes are drawn on each:
///
/// ```text
/// before   |/      lanes fold in from above
/// at       *       the commit itself
/// after    |\      lanes branch out below
/// ```
#[derive(Debug, PartialEq)]
pub struct Step {
    /// Column the commit's dot sits in.
    pub col: usize,
    /// Lane occupancy on the fold row, above the dot.
    pub before: Vec<bool>,
    /// Lane occupancy on the dot's own row.
    pub at: Vec<bool>,
    /// Lane occupancy on the next commit's row.
    pub after: Vec<bool>,
    /// Lanes that were also waiting for this commit and fold into `col`.
    /// These are fork points: one parent with two children.
    pub closing: Vec<usize>,
    /// Lanes this commit's extra parents continue in. An edge is drawn from
    /// `col` out to each of them.
    pub opening: Vec<usize>,
    /// True when the commit has more than one parent.
    pub is_merge: bool,
}

impl Step {
    pub fn width(&self) -> usize {
        self.before.len().max(self.at.len()).max(self.after.len())
    }
}

#[derive(Default)]
pub struct Lanes {
    slots: Vec<Option<String>>,
}

impl Lanes {
    pub fn new() -> Lanes {
        Lanes::default()
    }

    /// Number of currently open lanes. This is the whole of the graph's state.
    pub fn open(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
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
            if j != col && self.slots[j].as_deref() == Some(hash) {
                self.slots[j] = None;
                closing.push(j);
            }
        }

        let at = self.snapshot();

        // The first parent inherits this lane, so mainline history stays in a
        // straight column.
        self.slots[col] = parents.first().map(|p| p.to_string());

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

        Step {
            col,
            before,
            at,
            after: self.snapshot(),
            closing,
            opening,
            is_merge: parents.len() > 1,
        }
    }

    fn find(&self, hash: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.as_deref() == Some(hash))
    }

    /// Reuses the leftmost free lane so the drawing stays narrow.
    fn claim(&mut self, hash: &str) -> usize {
        match self.slots.iter().position(|s| s.is_none()) {
            Some(i) => {
                self.slots[i] = Some(hash.to_string());
                i
            }
            None => {
                self.slots.push(Some(hash.to_string()));
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
    fn freed_lanes_are_reused_before_widening() {
        let mut l = Lanes::new();
        l.advance("m", &["a", "s"]);
        l.advance("a", &[]); // lane 0 closes
        // A brand new tip should take the freed lane 0, not open lane 2.
        let t = l.advance("tip", &["x"]);
        assert_eq!(t.col, 0);
        assert_eq!(l.open(), 2);
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
