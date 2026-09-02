//! Turns a `Step` into terminal rows.
//!
//! Lanes live in even cells and the gaps between them in odd cells, so a
//! horizontal connector can run underneath other lanes without ever colliding
//! with them: where a horizontal crosses a vertical the two merge into a cross
//! glyph instead of overwriting it.
//!
//! A commit is drawn as up to four rows, in this order:
//!
//! ```text
//! fold_row        lanes converging into this commit, from above
//! commit_row      the dot
//! branch_row      lanes leaving this commit, below
//! shift_row       lanes sliding left into columns that just freed up
//! ```
//!
//! Folds are drawn *above* the dot because a fork point is only known when the
//! shared parent is reached - which is the same thing `git log --graph` does
//! when it prints `|/` on the line before the commit.

use super::lanes::Step;
use crate::style::{c, LANES, RESET};

pub struct Glyphs {
    pub commit: char,
    pub merge: char,
    pub vert: char,
    pub horiz: char,
    pub cross: char,
    /// A lane moving one column to the left.
    pub slide: char,
    /// Lane to the right bends down-left into the commit column.
    pub fold_left: char,
    /// Lane to the left bends down-right into the commit column.
    pub fold_right: char,
    /// Edge leaves the commit column and turns down to the right.
    pub branch_right: char,
    /// Edge leaves the commit column and turns down to the left.
    pub branch_left: char,
    pub tee_right: char,
    pub tee_left: char,
    pub tee_both: char,
}

pub const UNICODE: Glyphs = Glyphs {
    commit: '\u{25cf}',
    merge: '\u{25cb}',
    vert: '\u{2502}',
    horiz: '\u{2500}',
    cross: '\u{253c}',
    slide: '\u{2571}',
    fold_left: '\u{256f}',
    fold_right: '\u{2570}',
    branch_right: '\u{256e}',
    branch_left: '\u{256d}',
    tee_right: '\u{251c}',
    tee_left: '\u{2524}',
    tee_both: '\u{253c}',
};

pub const ASCII: Glyphs = Glyphs {
    commit: '*',
    merge: 'o',
    vert: '|',
    horiz: '-',
    cross: '+',
    slide: '/',
    fold_left: '/',
    fold_right: '\\',
    branch_right: '\\',
    branch_left: '/',
    tee_right: '+',
    tee_left: '+',
    tee_both: '+',
};

/// A row of glyphs plus the lane colour each one should be painted in.
struct Grid<'a> {
    cells: Vec<char>,
    colors: Vec<usize>,
    g: &'a Glyphs,
}

impl<'a> Grid<'a> {
    fn new(lanes: usize, g: &'a Glyphs) -> Grid<'a> {
        let w = if lanes == 0 { 0 } else { lanes * 2 - 1 };
        Grid {
            cells: vec![' '; w],
            colors: vec![0; w],
            g,
        }
    }

    /// Writing a horizontal over a vertical (or the reverse) produces a
    /// crossing rather than clobbering the lane that was already there.
    fn put(&mut self, at: usize, ch: char, color: usize) {
        if at >= self.cells.len() {
            return;
        }
        let existing = self.cells[at];
        let crossing = (existing == self.g.vert && ch == self.g.horiz)
            || (existing == self.g.horiz && ch == self.g.vert);
        self.cells[at] = if existing == ' ' {
            ch
        } else if crossing {
            self.g.cross
        } else {
            ch
        };
        // A crossing keeps the colour of the lane already there, so a lane's
        // colour stays continuous down the page.
        if !crossing {
            self.colors[at] = color;
        }
    }

    fn render(&self, min_width: usize) -> String {
        let mut out = String::new();
        let mut last: Option<usize> = None;
        for (i, ch) in self.cells.iter().enumerate() {
            if *ch == ' ' {
                if last.is_some() {
                    out.push_str(c(RESET));
                    last = None;
                }
                out.push(' ');
                continue;
            }
            let color = self.colors[i];
            if last != Some(color) {
                out.push_str(c(LANES[color % LANES.len()]));
                last = Some(color);
            }
            out.push(*ch);
        }
        if last.is_some() {
            out.push_str(c(RESET));
        }
        for _ in self.cells.len()..min_width {
            out.push(' ');
        }
        out
    }
}

fn cell(lane: usize) -> usize {
    lane * 2
}

fn occupied(row: &[bool], lane: usize) -> bool {
    row.get(lane).copied().unwrap_or(false)
}

/// Falls back to the column index so a lane without a recorded colour still
/// differs from its neighbours.
fn lane_color(map: &[usize], lane: usize) -> usize {
    map.get(lane).copied().unwrap_or(lane)
}

/// Lanes converging into this commit, drawn above the dot. `None` when nothing
/// folds in.
pub fn fold_row(step: &Step, g: &Glyphs, min_width: usize) -> Option<String> {
    if step.closing.is_empty() {
        return None;
    }
    let mut grid = Grid::new(step.width(), g);

    // Lanes that pass straight through this row.
    for j in 0..step.width() {
        if j != step.col && occupied(&step.before, j) && occupied(&step.at, j) {
            grid.put(cell(j), g.vert, lane_color(&step.colors, j));
        }
    }
    connect(&mut grid, g, step, &step.closing, &step.before, true);
    Some(grid.render(min_width))
}

/// The row the commit dot sits on: every open lane draws a vertical, except the
/// commit's own column.
pub fn commit_row(step: &Step, g: &Glyphs, min_width: usize) -> String {
    let mut grid = Grid::new(step.width(), g);
    for j in 0..step.width() {
        if occupied(&step.at, j) {
            grid.put(cell(j), g.vert, lane_color(&step.colors, j));
        }
    }
    let dot = if step.is_merge { g.merge } else { g.commit };
    grid.put(cell(step.col), dot, step.dot_color);
    grid.render(min_width)
}

/// Lanes leaving this commit, drawn below the dot. `None` when nothing branches
/// out.
pub fn branch_row(step: &Step, g: &Glyphs, min_width: usize) -> Option<String> {
    if step.opening.is_empty() {
        return None;
    }
    let mut grid = Grid::new(step.width(), g);

    for j in 0..step.width() {
        if j != step.col && occupied(&step.at, j) && occupied(&step.pre_shift, j) {
            grid.put(cell(j), g.vert, lane_color(&step.colors, j));
        }
    }
    connect(&mut grid, g, step, &step.opening, &step.at, false);
    Some(grid.render(min_width))
}

/// Lanes sliding one column left to close up a freed column. `None` when
/// nothing moves.
pub fn shift_row(step: &Step, g: &Glyphs, min_width: usize) -> Option<String> {
    if step.shifts.is_empty() {
        return None;
    }
    let mut grid = Grid::new(step.width(), g);

    for j in 0..step.width() {
        let moving = step.shifts.iter().any(|(from, _)| *from == j);
        if occupied(&step.pre_shift, j) && !moving {
            grid.put(cell(j), g.vert, lane_color(&step.colors, j));
        }
    }
    // Every move is one column, so each diagonal occupies the gap between the
    // old and new column and can never collide with another.
    for &(from, to) in &step.shifts {
        grid.put(cell(to) + 1, g.slide, lane_color(&step.colors, from));
    }
    Some(grid.render(min_width))
}

/// Draws edges between the commit column and each lane in `targets`, plus the
/// junction that ties them to the commit's own column.
///
/// `existing` says which lanes were already live: an edge reaching a live lane
/// gets a tee (the lane continues past the join) rather than a corner (the lane
/// starts or ends here).
fn connect(
    grid: &mut Grid,
    g: &Glyphs,
    step: &Step,
    targets: &[usize],
    existing: &[bool],
    folding: bool,
) {
    let col = step.col;
    let color = step.dot_color;
    let mut any_left = false;
    let mut any_right = false;

    for &k in targets {
        run(grid, g, col, k, color);
        let right = k > col;
        let corner = if occupied(existing, k) && !folding {
            // The lane was already open and keeps going, so the edge joins it.
            if right {
                g.tee_left
            } else {
                g.tee_right
            }
        } else if folding {
            if right {
                g.fold_left
            } else {
                g.fold_right
            }
        } else if right {
            g.branch_right
        } else {
            g.branch_left
        };
        grid.put(cell(k), corner, color);
        if right {
            any_right = true;
        } else {
            any_left = true;
        }
    }

    let junction = match (any_left, any_right) {
        (true, true) => g.tee_both,
        (false, true) => g.tee_right,
        (true, false) => g.tee_left,
        (false, false) => g.vert,
    };
    grid.put(cell(col), junction, color);
}

/// Draws the horizontal between two lanes, exclusive of both endpoints.
fn run(grid: &mut Grid, g: &Glyphs, from_lane: usize, to_lane: usize, color: usize) {
    let (lo, hi) = if from_lane < to_lane {
        (cell(from_lane), cell(to_lane))
    } else {
        (cell(to_lane), cell(from_lane))
    };
    for p in (lo + 1)..hi {
        grid.put(p, g.horiz, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::lanes::Lanes;

    /// Renders every row of a step in drawing order, trimmed.
    fn rows(step: &Step) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(r) = fold_row(step, &ASCII, 0) {
            v.push(r.trim_end().to_string());
        }
        v.push(commit_row(step, &ASCII, 0).trim_end().to_string());
        if let Some(r) = branch_row(step, &ASCII, 0) {
            v.push(r.trim_end().to_string());
        }
        if let Some(r) = shift_row(step, &ASCII, 0) {
            v.push(r.trim_end().to_string());
        }
        v
    }

    #[test]
    fn linear_history_draws_a_single_column() {
        let mut l = Lanes::new();
        let s = l.advance("a", &["b"]);
        assert_eq!(rows(&s), vec!["*"]);
    }

    #[test]
    fn merge_branches_out_below_the_dot() {
        let mut l = Lanes::new();
        let s = l.advance("m", &["a", "side"]);
        assert_eq!(rows(&s), vec!["o", "+-\\"], "merges get a hollow dot");
    }

    #[test]
    fn fork_folds_in_above_the_dot() {
        // Matches `git log --graph`, which prints the fold before the commit
        // the two lanes converge on.
        let mut l = Lanes::new();
        l.advance("m", &["a", "s"]);
        l.advance("a", &["r"]);
        l.advance("s", &["r"]);
        let s = l.advance("r", &[]);
        assert_eq!(rows(&s), vec!["+-/", "*"]);
    }

    #[test]
    fn a_commit_can_fold_in_and_branch_out_at_once() {
        // The merge commit of a branch that itself had two children: one lane
        // folds in from above and a new one opens below.
        let mut l = Lanes::new();
        l.advance("top", &["m", "b"]); // opens lane 1
        l.advance("b", &["m"]); // lane 1 now also waits for m
        let s = l.advance("m", &["p", "side"]);
        assert_eq!(
            rows(&s),
            vec!["+-/", "o", "+-\\"],
            "fold above, dot, branch below"
        );
    }

    #[test]
    fn a_freed_column_is_closed_up_by_a_diagonal() {
        let mut l = Lanes::new();
        l.advance("m", &["a", "s"]); // lane 0 = a, lane 1 = s
        let s = l.advance("a", &[]); // lane 0 ends, so lane 1 slides into it
        assert_eq!(rows(&s), vec!["* |", " /"]);
    }

    #[test]
    fn horizontal_crossing_a_lane_does_not_erase_it() {
        // A merge reaching past an open middle lane must cross it, not cut it.
        let mut l = Lanes::new();
        l.advance("m", &["a", "s"]); // opens lane 1
        let s = l.advance("a", &["p", "far"]); // opens lane 2, crossing lane 1
        assert_eq!(
            branch_row(&s, &ASCII, 0).unwrap().trim_end(),
            "+-+-\\",
            "lane 1 survives as a crossing"
        );
    }

    #[test]
    fn a_lane_keeps_one_colour_across_a_shift() {
        let mut l = Lanes::new();
        let m = l.advance("m", &["a", "s"]);
        let side_color = m.colors_after[1];
        let s = l.advance("a", &[]); // side lane slides from column 1 to 0
        assert_eq!(s.shifts, vec![(1, 0)]);
        assert_eq!(
            s.colors_after[0], side_color,
            "the branch keeps its colour in its new column"
        );
    }

    #[test]
    fn min_width_pads_so_commit_text_lines_up() {
        let mut l = Lanes::new();
        let s = l.advance("a", &["b"]);
        assert_eq!(commit_row(&s, &ASCII, 5), "*    ");
    }
}
