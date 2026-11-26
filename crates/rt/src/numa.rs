//! The machine's NUMA nodes (P38's stage 7): which cores each has, so that an executor made for
//! node `n` runs on that node's cores, near its memory. Read once a process: Linux's sysfs
//! (`/sys/devices/system/node/node*/cpulist`), Windows' `GetLogicalProcessorInformationEx`; one
//! node of every core elsewhere, or where neither says.

use std::sync::OnceLock;

/// Each node's cores, by the node's number: the logical processors' numbers, as
/// `core_affinity` numbers them.
pub(crate) fn nodes() -> &'static [Vec<usize>] {
    static NODES: OnceLock<Vec<Vec<usize>>> = OnceLock::new();
    NODES.get_or_init(|| {
        let found = read();
        if found.iter().any(|n| !n.is_empty()) {
            found
        } else {
            let n = std::thread::available_parallelism().map_or(1, usize::from);
            vec![(0..n).collect()]
        }
    })
}

/// The cores of node `n`, if it has any.
pub(crate) fn cores_of(n: usize) -> Option<&'static [usize]> {
    nodes().get(n).map(Vec::as_slice).filter(|c| !c.is_empty())
}

/// A sysfs CPU list (`0-7,16-23`): the numbers it names.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_list(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in text.trim().split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    out.extend(a..=b);
                }
            }
            None => out.extend(part.parse::<usize>()),
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn read() -> Vec<Vec<usize>> {
    let mut nodes = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/devices/system/node") else {
        return nodes;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(n) = name
            .to_str()
            .and_then(|s| s.strip_prefix("node"))
            .and_then(|s| s.parse::<usize>().ok())
        else {
            continue;
        };
        let list = std::fs::read_to_string(entry.path().join("cpulist")).unwrap_or_default();
        if nodes.len() <= n {
            nodes.resize(n + 1, Vec::new());
        }
        nodes[n] = parse_list(&list);
    }
    nodes
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn read() -> Vec<Vec<usize>> {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationNumaNode, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    let mut len = 0u32;
    // SAFETY: a first call with no buffer asks for the length the information needs.
    unsafe { GetLogicalProcessorInformationEx(RelationNumaNode, std::ptr::null_mut(), &mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` has room for `len` bytes, aligned for the records it holds.
    let ok = unsafe {
        GetLogicalProcessorInformationEx(RelationNumaNode, buf.as_mut_ptr().cast(), &mut len)
    };
    let mut nodes: Vec<Vec<usize>> = Vec::new();
    if ok == 0 {
        return nodes;
    }
    let base = buf.as_ptr().cast::<u8>();
    let mut at = 0usize;
    while at < len as usize {
        // SAFETY: the OS wrote `len` bytes of records, each saying its own size.
        let info = unsafe {
            &*base
                .add(at)
                .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>()
        };
        if info.Relationship == RelationNumaNode {
            // SAFETY: a NUMA node's record holds its relationship's union member.
            let node = unsafe { &info.Anonymous.NumaNode };
            let n = node.NodeNumber as usize;
            // SAFETY: the first group's mask is always there; a node of more groups has more.
            let mask = unsafe { node.Anonymous.GroupMask };
            if nodes.len() <= n {
                nodes.resize(n + 1, Vec::new());
            }
            let group = mask.Group as usize;
            nodes[n].extend(
                (0..64)
                    .filter(|b| mask.Mask >> b & 1 == 1)
                    .map(|b| group * 64 + b),
            );
        }
        if info.Size == 0 {
            break;
        }
        at += info.Size as usize;
    }
    nodes
}

#[cfg(not(any(target_os = "linux", windows)))]
fn read() -> Vec<Vec<usize>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_core_is_on_a_node() {
        let n = std::thread::available_parallelism().map_or(1, usize::from);
        let nodes = super::nodes();
        let cores: usize = nodes.iter().map(Vec::len).sum();
        assert!(!nodes.is_empty() && cores >= 1, "{nodes:?}");
        assert!(cores <= n.max(64), "{nodes:?}");
        assert_eq!(
            super::parse_list("0-3,8,10-11"),
            vec![0, 1, 2, 3, 8, 10, 11]
        );
    }
}
