//! A program's site tables, as `moddil_native` writes them for a program run without the
//! compiler (`moddil build`, and WASM modules): the text of a trap from them.
//!
//! Eight words a site: its kind (0 a check, 1 a call), whether it is tracked, its class's text
//! and its file's (offsets and lengths in the strings), its line and its column.

#![allow(unsafe_code)]

use crate::Trapped;

/// Where a program's site tables are.
#[derive(Clone, Copy, Debug)]
pub struct Tables {
    /// How many sites there are.
    pub count: u64,
    /// Their eight words each.
    pub sites: *const u32,
    /// The text they name.
    pub strings: *const u8,
}

impl Tables {
    /// Site `k`'s eight words.
    fn site(&self, k: u32) -> Option<[u32; 8]> {
        if u64::from(k) >= self.count {
            return None;
        }
        // SAFETY: the tables hold `count` sites of eight words.
        unsafe {
            let p = self.sites.add(8 * k as usize);
            Some(std::array::from_fn(|i| *p.add(i)))
        }
    }

    /// The text at offset `at` (`n` bytes) of the strings.
    fn string(&self, at: u32, n: u32) -> String {
        // SAFETY: the strings hold every offset and length the sites name.
        let bytes =
            unsafe { std::slice::from_raw_parts(self.strings.add(at as usize), n as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// A trap's class and where it is reported, as `file:line:column`.
    pub fn trap_text(&self, t: &Trapped) -> (String, String) {
        let Some(own) = self.site(t.site) else {
            return ("deadlock".to_string(), String::new());
        };
        let at = t.reported_at(|s| self.site(s).is_some_and(|w| w[0] == 1 && w[1] == 0));
        let w = self.site(at).unwrap_or(own);
        (
            self.string(own[2], own[3]),
            format!("{}:{}:{}", self.string(w[4], w[5]), w[6], w[7]),
        )
    }

    /// The text of a `Trapped` (`task.isolate.1`): its class (`which` 0), or where it was (1).
    pub fn trapped(&self, t: &Trapped, which: u32) -> String {
        let (class, at) = self.trap_text(t);
        if which == 0 { class } else { at }
    }
}
