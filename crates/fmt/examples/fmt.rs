//! `cargo run -p moddil-fmt --example fmt -- file.moddil…` prints files formatted, or why they
//! cannot be. With `--check`, prints nothing but the files not in the layout, and fails if
//! there are any (`tools/ci.py quick` checks the files it changed this way, without building
//! the CLI).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    let paths: Vec<&String> = args.iter().filter(|a| *a != "--check").collect();
    assert!(!paths.is_empty(), "a file to format");
    let mut ok = true;
    for path in paths {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{path}: {e}");
                ok = false;
                continue;
            }
        };
        match moddil_fmt::format(&text) {
            // Line endings are the checkout's business, as for `moddil fmt`.
            Ok(s) if check => {
                if s != text.replace("\r\n", "\n") {
                    println!("{path}");
                    ok = false;
                }
            }
            Ok(s) => print!("{s}"),
            Err(e) => {
                eprintln!("{path}: {e}");
                ok = false;
            }
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
