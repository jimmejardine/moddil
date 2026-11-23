//! The target the C sources are compiled for: the one this crate is built for.
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    println!("cargo:rustc-env=MODDIL_FFI_TARGET={target}");
}
