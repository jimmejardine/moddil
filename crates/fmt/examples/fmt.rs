//! `cargo run -p moddil-fmt --example fmt -- file.moddil` prints a file formatted, or why it
//! cannot be.

fn main() {
    let path = std::env::args().nth(1).expect("a file to format");
    let text = std::fs::read_to_string(&path).expect("a readable file");
    match moddil_fmt::format(&text) {
        Ok(s) => print!("{s}"),
        Err(e) => eprintln!("{path}: {e}"),
    }
}
