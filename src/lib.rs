//! Reading git history without holding it in memory.
//!
//! This is the engine behind the `gitlimes` command line tool, exposed as a
//! library so a TUI, a graphical front end or a script can reuse it instead of
//! shelling out to the binary and parsing its output.
//!
//! # The one rule
//!
//! Nothing here scales with the length of the history. [`repo::Records`] streams
//! `git log` output through a buffer that is reused for every record, and
//! [`graph::lanes::Lanes`] holds one entry per *open branch lane* rather than
//! one per commit. Memory is bounded by how many branches are simultaneously in
//! flight - typically a handful - and by the number of authors or refs, never by
//! the number of commits.
//!
//! Any addition that would break that is a deliberate decision, not an accident.
//!
//! # Layers
//!
//! - [`repo`] spawns git and streams records. It is the only module that knows
//!   git is a subprocess, so swapping in a different backend touches only it.
//! - [`graph::lanes`] turns a stream of commits into branch-lane geometry. It
//!   emits an abstract [`graph::lanes::Step`] per commit and draws nothing.
//! - [`graph::draw`] renders those steps as terminal rows. A second renderer -
//!   SVG, HTML, a canvas - plugs in here and reuses the layout unchanged.
//! - [`json`] emits newline-delimited JSON, the machine-readable form of every
//!   command's output.
//! - [`fmt`] and [`style`] are presentation helpers: column fitting, relative
//!   dates, sparklines, ANSI colour.
//!
//! # Example
//!
//! Walk the current repository's history without collecting it:
//!
//! ```no_run
//! use gitlimes::repo::{self, Commit, Records, LOG_FORMAT};
//!
//! let mut records = Records::spawn(repo::git(&["log", LOG_FORMAT, "--"]))?;
//! while let Some(record) = records.next_record()? {
//!     if let Some(commit) = Commit::parse(&record) {
//!         println!("{} {}", commit.short, commit.subject);
//!     }
//! }
//! records.finish()?;
//! # Ok::<(), std::io::Error>(())
//! ```

pub mod fmt;
pub mod graph;
pub mod json;
pub mod repo;
pub mod style;
