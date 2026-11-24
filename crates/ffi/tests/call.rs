//! Calls into the C runtime through the trampolines, by symbol: scalars, structs by value, and
//! a callback C calls back through.

#![allow(unsafe_code)]

use std::sync::Arc;

use moddil_ffi::abi::{CLayout, Scalar};
use moddil_ffi::{CType, call, callback, lookup, with_callbacks};

#[test]
fn the_c_runtime_is_called_by_symbol() {
    let abs = lookup("abs").expect("the C runtime's abs");
    // SAFETY: `int abs(int)`.
    let r = unsafe { call(abs, &[CType::I32], &CType::I32, &[(-5i32) as u32 as u64]) };
    assert_eq!(r.map(|w| w[0] as u32 as i32), Ok(5));

    let strlen = lookup("strlen").expect("the C runtime's strlen");
    let text = b"moddil\0";
    // SAFETY: `size_t strlen(const char *)`, with a NUL-terminated string.
    let r = unsafe { call(strlen, &[CType::Ptr], &CType::I64, &[text.as_ptr() as u64]) };
    assert_eq!(r, Ok(vec![6]));

    let fabs = lookup("fabs").expect("the C runtime's fabs");
    // SAFETY: `double fabs(double)`.
    let r = unsafe { call(fabs, &[CType::F64], &CType::F64, &[(-2.5f64).to_bits()]) };
    assert_eq!(r.map(|w| f64::from_bits(w[0])), Ok(2.5));
}

#[test]
fn structs_come_back_by_value() {
    // `div_t div(int, int)`: two `int`s, 8 bytes.
    let div = lookup("div").expect("the C runtime's div");
    let div_t = CType::Struct(Arc::new(CLayout {
        size: 8,
        align: 4,
        fields: vec![(0, Scalar::Int(4)), (4, Scalar::Int(4))],
    }));
    // SAFETY: `div_t div(int numer, int denom)`.
    let r = unsafe { call(div, &[CType::I32, CType::I32], &div_t, &[17, 5]) }.expect("a call");
    assert_eq!((r[0] as u32, (r[0] >> 32) as u32), (3, 2));

    // `lldiv_t lldiv(long long, long long)`: 16 bytes, in two registers or through a pointer.
    let lldiv = lookup("lldiv").expect("the C runtime's lldiv");
    let lldiv_t = CType::Struct(Arc::new(CLayout {
        size: 16,
        align: 8,
        fields: vec![(0, Scalar::Int(8)), (8, Scalar::Int(8))],
    }));
    let n = (-17i64) as u64;
    // SAFETY: `lldiv_t lldiv(long long numer, long long denom)`.
    let r = unsafe { call(lldiv, &[CType::I64, CType::I64], &lldiv_t, &[n, 5]) }.expect("a call");
    assert_eq!((r[0] as i64, r[1] as i64), (-3, -2));
}

#[test]
fn c_calls_back_into_the_handler() {
    // `qsort` with a comparator made here, whose comparisons the handler answers.
    let qsort = lookup("qsort").expect("the C runtime's qsort");
    let cmp = callback(7, &[CType::Ptr, CType::Ptr], &CType::I32).expect("a callback");
    let mut xs: [i32; 5] = [5, 1, 4, 2, 3];
    let mut calls = 0;
    let mut handler = |id: u64, args: &[u64]| {
        assert_eq!(id, 7);
        calls += 1;
        // SAFETY: qsort passes pointers to two of the elements.
        let (a, b) = unsafe { (*(args[0] as *const i32), *(args[1] as *const i32)) };
        a.cmp(&b) as i32 as u32 as u64
    };
    let args = [xs.as_mut_ptr() as u64, 5, 4, cmp as u64];
    // SAFETY: `void qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *))`.
    let r = with_callbacks(&mut handler, || unsafe {
        call(
            qsort,
            &[CType::Ptr, CType::I64, CType::I64, CType::Ptr],
            &CType::Void,
            &args,
        )
    });
    assert!(r.is_ok());
    assert_eq!(xs, [1, 2, 3, 4, 5]);
    assert!(calls > 0);
}
