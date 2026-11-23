//! Running a module in Node ([`host.cjs`](host.cjs)) and reading what it reports: each entry's
//! status and trap, and the values it observed, rendered from snapshots of its memory with the
//! native backend's renderer.

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use cranelift_codegen::ir::types;
use moddil_mir::TrapClass;
use moddil_native::layout::Layouts;
use moddil_native::show::{Memory, Show};
use moddil_sema::{Db, Program, Ty};
use moddil_syntax::TextRange;

use crate::emit::{Compiled, OUT, STACK_TOP, Site, compile};

/// Why a run gave no result.
#[derive(Debug)]
pub enum Wasm {
    /// Something the program reaches is not compiled to WASM yet: what.
    Unsupported(String),
    /// A check failed: its class, and where (the file, by index, and its source).
    Trap(TrapClass, u32, TextRange),
    /// There is no host to run it in (Node): why.
    NoHost(String),
    /// The engine stopped it (a bug in the backend): how.
    Crash(String),
}

/// An observed `// =>` value: the file, the expression and its value as Moddil source.
pub type Observed = (u32, TextRange, String);

/// A snapshot of part of a module's memory, from `base`.
struct Snapshot {
    base: u64,
    bytes: Vec<u8>,
}

impl Memory for Snapshot {
    fn bits(&self, addr: u64, n: u32) -> u128 {
        let mut out = [0u8; 16];
        for (k, b) in out.iter_mut().enumerate().take(n as usize) {
            *b = self.byte(addr + k as u64);
        }
        u128::from_le_bytes(out)
    }

    fn bytes(&self, addr: u64, n: usize) -> Vec<u8> {
        (0..n as u64).map(|k| self.byte(addr + k)).collect()
    }
}

impl Snapshot {
    fn byte(&self, addr: u64) -> u8 {
        addr.checked_sub(self.base)
            .and_then(|k| self.bytes.get(k as usize).copied())
            .unwrap_or(0)
    }
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .filter_map(|k| u8::from_str_radix(&s[2 * k..2 * k + 2], 16).ok())
        .collect()
}

/// One line the host wrote: its fields, as text or numbers.
type Line = HashMap<String, Value>;

#[derive(Debug, Clone)]
enum Value {
    Num(i64),
    Str(String),
}

impl Value {
    fn num(&self) -> i64 {
        match self {
            Value::Num(n) => *n,
            Value::Str(_) => 0,
        }
    }
    fn str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            Value::Num(_) => "",
        }
    }
}

/// Parses a flat JSON object of strings and integers, as the host writes them.
fn parse(line: &str) -> Option<Line> {
    let mut out = HashMap::new();
    let mut cs = line.trim().chars().peekable();
    let string = |cs: &mut std::iter::Peekable<std::str::Chars<'_>>| -> Option<String> {
        if cs.next()? != '"' {
            return None;
        }
        let mut s = String::new();
        loop {
            match cs.next()? {
                '"' => return Some(s),
                '\\' => match cs.next()? {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'u' => {
                        let h: String = (0..4).filter_map(|_| cs.next()).collect();
                        s.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
                    }
                    c => s.push(c),
                },
                c => s.push(c),
            }
        }
    };
    if cs.next()? != '{' {
        return None;
    }
    loop {
        match cs.peek()? {
            '}' => return Some(out),
            ',' => {
                cs.next();
            }
            _ => {
                let key = string(&mut cs)?;
                if cs.next()? != ':' {
                    return None;
                }
                let v = if *cs.peek()? == '"' {
                    Value::Str(string(&mut cs)?)
                } else {
                    let mut n = String::new();
                    while let Some(&c) = cs.peek() {
                        if c == ',' || c == '}' {
                            break;
                        }
                        n.push(c);
                        cs.next();
                    }
                    Value::Num(n.trim().parse().ok()?)
                };
                out.insert(key, v);
            }
        }
    }
}

/// The Node to run modules with: `MODDIL_NODE`, or `node` on the path.
fn node() -> String {
    std::env::var("MODDIL_NODE").unwrap_or_else(|_| "node".to_string())
}

/// Whether there is a Node to run modules with.
pub fn host_available() -> bool {
    Command::new(node())
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs `compiled`'s `entries` (each `test<n>`, or `main:<result size>`) in Node: the lines it
/// wrote.
fn run_module(compiled: &Compiled<'_>, entries: &[String]) -> Result<Vec<Line>, Wasm> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "moddil-wasm-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| Wasm::NoHost(e.to_string()))?;
    let module = dir.join("module.wasm");
    let host = dir.join("host.cjs");
    std::fs::write(&module, &compiled.bytes).map_err(|e| Wasm::NoHost(e.to_string()))?;
    std::fs::write(&host, include_str!("host.cjs")).map_err(|e| Wasm::NoHost(e.to_string()))?;
    let out = Command::new(node())
        .arg("--stack-size=8000")
        .arg(&host)
        .arg(&module)
        .arg(STACK_TOP.to_string())
        .arg(OUT.to_string())
        .args(entries)
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let out = out.map_err(|e| Wasm::NoHost(format!("running {}: {e}", node())))?;
    let lines: Vec<Line> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(parse)
        .collect();
    if let Some(f) = lines.iter().find_map(|l| l.get("fatal")) {
        return Err(Wasm::Crash(f.str().to_string()));
    }
    Ok(lines)
}

/// The renderer for a module's values: 32-bit pointers.
fn show<'a>(compiled: &Compiled<'a>) -> Show<'a> {
    Show {
        items: compiled.items,
        layouts: Layouts::with_pointer(compiled.items, types::I32),
        dyn_types: &[],
        ptr_bytes: 4,
    }
}

/// What a run of tests gives: each test's values observed, and how it ended.
pub type TestRuns = Vec<(Vec<Observed>, Result<(), Wasm>)>;

/// Runs tests `tests` of `program` in Node, all in one module: what each observed and how it
/// ended.
///
/// # Errors
///
/// [`Wasm::Unsupported`] if any of them reaches code not compiled to WASM yet, and
/// [`Wasm::NoHost`] if there is no Node to run them in.
pub fn run_tests(db: &dyn Db, program: Program, tests: &[u32]) -> Result<TestRuns, Wasm> {
    let compiled = compile(db, program, tests, None).map_err(Wasm::Unsupported)?;
    let entries: Vec<String> = tests.iter().map(|t| format!("test{t}")).collect();
    let lines = run_module(&compiled, &entries)?;
    let mut sh = show(&compiled);
    let mut out = Vec::new();
    let mut observed = Vec::new();
    for l in lines {
        if let Some(site) = l.get("o") {
            if let Some(Site::Observe(file, range, ty)) = compiled.sites.get(site.num() as usize) {
                let snap = Snapshot {
                    base: l.get("base").map_or(0, Value::num) as u64,
                    bytes: unhex(l.get("hex").map_or("", Value::str)),
                };
                let addr = l.get("a").map_or(0, Value::num) as u64;
                let text = sh.render(&snap, &ty.clone(), addr);
                observed.push((*file, *range, text));
            }
        } else if l.contains_key("e") {
            out.push((std::mem::take(&mut observed), ended(&compiled, &l)));
        }
    }
    Ok(out)
}

/// How an entry ended, from its line.
fn ended(compiled: &Compiled<'_>, l: &Line) -> Result<(), Wasm> {
    if let Some(c) = l.get("crash") {
        return Err(Wasm::Crash(c.str().to_string()));
    }
    match l.get("s").map_or(0, Value::num) {
        0 => Ok(()),
        _ => match compiled
            .sites
            .get(l.get("t").map_or(-1, Value::num) as usize)
        {
            Some(Site::Trap(class, file, range)) => Err(Wasm::Trap(*class, *file, *range)),
            _ => Err(Wasm::Crash("a trap without a site".to_string())),
        },
    }
}

/// Runs `fn main()` (function `index`) of `program` in Node: its result, as Moddil source.
///
/// # Errors
///
/// As [`run_tests`], and [`Wasm::Trap`] if it traps.
pub fn run_main(db: &dyn Db, program: Program, index: u32) -> Result<String, Wasm> {
    let compiled = compile(db, program, &[], Some(index)).map_err(Wasm::Unsupported)?;
    let ret: Ty = compiled.main.clone().unwrap_or(Ty::Unit);
    let size = Layouts::with_pointer(compiled.items, types::I32)
        .of(&ret)
        .map_or(0, |l| l.size);
    let lines = run_module(&compiled, &[format!("main:{size}")])?;
    let l = lines
        .iter()
        .find(|l| l.contains_key("e"))
        .ok_or_else(|| Wasm::Crash("no result".to_string()))?;
    ended(&compiled, l)?;
    let snap = Snapshot {
        base: u64::from(OUT),
        bytes: unhex(l.get("hex").map_or("", Value::str)),
    };
    Ok(show(&compiled).render(&snap, &ret, u64::from(OUT)))
}
