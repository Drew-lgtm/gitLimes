//! The command line front end. Everything reusable lives in the library
//! (see src/lib.rs); this binary only parses arguments and prints.

mod cli;
mod cmd;

use gitlimes::style;
use std::io::ErrorKind;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let force_color = cli::take_color(&mut args);
    style::init(force_color);

    if args.is_empty() {
        print!("{}", cli::USAGE);
        std::process::exit(2);
    }

    let command = args.remove(0);
    let result = match command.as_str() {
        "log" => cmd::log::run(args),
        "branches" | "br" => cmd::branches::run(args),
        "who" => cmd::who::run(args),
        "graph" => cmd::graph::run(args),
        "-h" | "--help" | "help" => {
            print!("{}", cli::USAGE);
            Ok(())
        }
        "-V" | "--version" => {
            println!("gitlimes {}", VERSION);
            Ok(())
        }
        other => {
            eprintln!("gitlimes: unknown command '{}'\n", other);
            eprint!("{}", cli::USAGE);
            std::process::exit(2);
        }
    };

    if let Err(e) = result {
        // `gitlimes log | head` closes our stdout early; that is a successful
        // run, not a crash.
        if e.kind() == ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("gitlimes: {}", e);
        std::process::exit(1);
    }
}
