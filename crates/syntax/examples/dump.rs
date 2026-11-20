//! Prints the syntax tree and errors of a file, or of the text given: `cargo run --example dump -- file.moddil`.
fn main() {
    let arg = std::env::args().nth(1).expect("a file or text");
    let text = std::fs::read_to_string(&arg).unwrap_or(arg);
    let parse = moddil_syntax::parse(&text);
    print!("{:?}", parse.syntax());
    for e in parse.errors() {
        println!("error: {e}");
    }
}
