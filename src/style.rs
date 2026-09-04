//! ANSI colouring. Colour is decided once at startup and every code is routed
//! through `c()`, so a non-tty run emits plain bytes with no branches at the
//! call sites.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(false);

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";

/// Lane palette for the graph, chosen to stay legible on both light and dark
/// backgrounds.
pub const LANES: [&str; 6] = [CYAN, MAGENTA, GREEN, YELLOW, BLUE, RED];

/// `force` is `Some` when the user passed `--color`/`--no-color`; otherwise we
/// honour NO_COLOR and fall back to tty detection.
pub fn init(force: Option<bool>) {
    let on = decide(
        force,
        std::env::var_os("NO_COLOR").as_deref(),
        std::io::stdout().is_terminal(),
    );
    if on {
        enable_vt();
    }
    COLOR.store(on, Ordering::Relaxed);
}

/// Whether colour should be on, given the three inputs that decide it.
///
/// Split out from [`init`] so the rules can be tested; reading the environment
/// and the terminal makes the decision itself untestable, and it is exactly the
/// kind of small logic that goes quietly wrong.
///
/// An explicit `--color` or `--no-color` wins outright. Otherwise `NO_COLOR`
/// disables colour **only when it is set to a non-empty value**: the convention
/// at no-color.org is explicit that an empty value does not count, so that
/// `NO_COLOR=` can neutralise the setting the way an unset variable does.
pub fn decide(force: Option<bool>, no_color: Option<&std::ffi::OsStr>, is_tty: bool) -> bool {
    if let Some(v) = force {
        return v;
    }
    let suppressed = no_color.is_some_and(|v| !v.is_empty());
    !suppressed && is_tty
}

pub fn on() -> bool {
    COLOR.load(Ordering::Relaxed)
}

/// Returns the escape code, or an empty string when colour is off.
pub fn c(code: &str) -> &str {
    if on() {
        code
    } else {
        ""
    }
}

/// conhost needs to be told it speaks ANSI; Windows Terminal already does.
#[cfg(windows)]
fn enable_vt() {
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    type Handle = *mut core::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: u32) -> Handle;
        fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: Handle, mode: u32) -> i32;
    }

    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        // GetStdHandle reports failure as INVALID_HANDLE_VALUE (-1), and
        // returns null only when the process has no such handle at all. Both
        // must be rejected; checking null alone would pass -1 straight into
        // GetConsoleMode.
        if h.is_null() || h as isize == -1 {
            return;
        }
        let mut mode = 0u32;
        if GetConsoleMode(h, &mut mode) != 0 {
            SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

#[cfg(not(windows))]
fn enable_vt() {}

#[cfg(test)]
mod tests {
    use super::decide;
    use std::ffi::OsStr;

    fn no_color(v: &str) -> Option<&OsStr> {
        Some(OsStr::new(v))
    }

    #[test]
    fn an_explicit_flag_beats_everything() {
        assert!(decide(Some(true), no_color("1"), false));
        assert!(!decide(Some(false), None, true));
    }

    #[test]
    fn colour_is_on_only_for_a_terminal() {
        assert!(decide(None, None, true));
        assert!(!decide(None, None, false), "piped output must be plain");
    }

    #[test]
    fn no_color_suppresses_colour_when_it_has_a_value() {
        assert!(!decide(None, no_color("1"), true));
        assert!(!decide(None, no_color("anything"), true));
    }

    #[test]
    fn an_empty_no_color_does_not_count() {
        // no-color.org is explicit that the variable must be present AND
        // non-empty, so `NO_COLOR=` neutralises an inherited setting rather
        // than silently stripping colour.
        assert!(
            decide(None, no_color(""), true),
            "NO_COLOR= should behave like unset"
        );
    }
}
