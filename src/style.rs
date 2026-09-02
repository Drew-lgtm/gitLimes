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
    let on = match force {
        Some(v) => v,
        None => std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
    };
    if on {
        enable_vt();
    }
    COLOR.store(on, Ordering::Relaxed);
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
        if h.is_null() {
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
