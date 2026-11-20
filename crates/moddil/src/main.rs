//! The `moddil` command. At M0 it will run files and tests on the MIR interpreter; for now it
//! only reports its version. See `crates/moddil/README.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("moddil {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        None | Some("--help" | "-h") => {
            println!(
                "moddil {}

Usage: moddil --version",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("moddil: unknown command `{other}` (there are none yet; see --help)");
            ExitCode::from(2)
        }
    }
}
