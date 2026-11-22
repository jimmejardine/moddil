//! What the runtime asks of the compiler while compiled code runs: the text of a `Trapped`,
//! from the run's sites and files.

#![allow(unsafe_code)]

/// What the compiler gives a run: its sites, and its files' paths and text.
pub(crate) struct HostData<'a> {
    pub(crate) sites: &'a [crate::lower::Site],
    pub(crate) files: &'a [(String, String)],
}

/// The text of the `Trapped` for trap `t`: its class (`which` 0) or where it was, as
/// `file:line:column` (1), the trap reported where the interpreters report it.
pub(crate) fn trap_text(t: &moddil_rt::Trapped, which: u32) -> String {
    let host = moddil_rt::host() as *const HostData<'_>;
    // SAFETY: the run's host data lives while its code runs.
    let Some(h) = (unsafe { host.as_ref() }) else {
        return String::new();
    };
    match crate::jit::reported(h.sites, t) {
        crate::Native::Trap(class, file, range, _) => {
            if which == 0 {
                class.to_string()
            } else {
                let (path, text) = &h.files[file as usize];
                let (line, col) = moddil_syntax::line_col(text, range.start);
                format!("{path}:{line}:{col}")
            }
        }
        _ => String::new(),
    }
}
