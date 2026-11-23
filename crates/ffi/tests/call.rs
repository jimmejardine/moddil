//! Calls into the C runtime through the trampolines, by symbol.

#![allow(unsafe_code)]

use moddil_ffi::{CType, call, lookup};

#[test]
fn the_c_runtime_is_called_by_symbol() {
    let abs = lookup("abs").expect("the C runtime's abs");
    // SAFETY: `int abs(int)`.
    let r = unsafe { call(abs, &[CType::I32], CType::I32, &[(-5i32) as u32 as u64]) };
    assert_eq!(r.map(|w| w as u32 as i32), Ok(5));

    let strlen = lookup("strlen").expect("the C runtime's strlen");
    let text = b"moddil\0";
    // SAFETY: `size_t strlen(const char *)`, with a NUL-terminated string.
    let r = unsafe { call(strlen, &[CType::Ptr], CType::I64, &[text.as_ptr() as u64]) };
    assert_eq!(r, Ok(6));

    let fabs = lookup("fabs").expect("the C runtime's fabs");
    // SAFETY: `double fabs(double)`.
    let r = unsafe { call(fabs, &[CType::F64], CType::F64, &[(-2.5f64).to_bits()]) };
    assert_eq!(r.map(f64::from_bits), Ok(2.5));
}
