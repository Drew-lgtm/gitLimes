//! Hand-rolled argument handling. The flag sets are small and adding a parser
//! crate would cost more binary and startup time than it saves in code.

pub const USAGE: &str = "\
gitlimes - fast, low-memory git history

USAGE:
    gitlimes <COMMAND> [OPTIONS]

COMMANDS:
    log         colored commit history
    branches    branch overview with staleness and ahead/behind
    who         author and contribution stats
    graph       unicode branch graph

GLOBAL OPTIONS:
    --color / --no-color    override tty colour detection
    --pager / --no-pager    override tty pager detection
    -h, --help              show help for a command
    -V, --version           print version
";

/// Splits `--flag=value` into its parts.
pub fn split_eq(arg: &str) -> Option<(&str, &str)> {
    let i = arg.find('=')?;
    Some((&arg[..i], &arg[i + 1..]))
}

pub struct Unknown(pub String);

impl std::fmt::Display for Unknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown option '{}'", self.0)
    }
}

/// Pulls the global colour flags out of the argument list before a command
/// parses the rest, so `--color` works in any position.
pub fn take_color(args: &mut Vec<String>) -> Option<bool> {
    let mut force = None;
    args.retain(|a| match a.as_str() {
        "--color" => {
            force = Some(true);
            false
        }
        "--no-color" => {
            force = Some(false);
            false
        }
        _ => true,
    });
    force
}

/// Pulls the global pager flags out of the argument list, mirroring
/// `take_color`, so `--no-pager` works in any position.
pub fn take_pager(args: &mut Vec<String>) -> gitlimes::pager::Mode {
    let mut mode = gitlimes::pager::Mode::Auto;
    args.retain(|a| match a.as_str() {
        "--pager" => {
            mode = gitlimes::pager::Mode::Always;
            false
        }
        "--no-pager" => {
            mode = gitlimes::pager::Mode::Never;
            false
        }
        _ => true,
    });
    mode
}
