//! Running a compiled module in Node ([`host.cjs`](host.cjs)), as the JIT runner runs native
//! code: each entry's result or error shown, its result dropped, its allocations counted, its
//! trap reported where the interpreters report it, and the values it observes shown while it
//! waits. Values are read from the module's memory over the pipe, a page at a time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use moddil_mir::TrapClass;
use moddil_native::backend::Site;
use moddil_native::layout::Layouts;
use moddil_native::show::{Memory, Show};
use moddil_sema::{Db, Item, Program};
use moddil_syntax::TextRange;

use crate::compile::{Compiled, compile};
use crate::module::RECORD_WORDS;

/// Why a run gave no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wasm {
    /// Something the program reaches is not compiled to WASM yet: what.
    Unsupported(String),
    /// A check failed: its class, where (the file, by index, and its source), and, for a failed
    /// `assert`, each part of its condition with its value if it was evaluated.
    Trap(TrapClass, u32, TextRange, Vec<(TextRange, Option<String>)>),
    /// A run that returned left this many allocations unfreed: a bug in the backend.
    Leak(i64),
    /// An error left the run: where it was raised, and the error as Moddil source.
    Failed(u32, TextRange, String),
    /// There is no host to run it in (Node): why.
    NoHost(String),
    /// The engine stopped it (a bug in the backend): how.
    Crash(String),
}

/// An observed `// =>` value: the file, the expression and its value as Moddil source.
pub type Observed = (u32, TextRange, String);

/// What a run of tests gives: each test's values observed, and how it ended.
pub type TestRuns = Vec<(Vec<Observed>, Result<(), Wasm>)>;

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

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .filter_map(|k| u8::from_str_radix(&s[2 * k..2 * k + 2], 16).ok())
        .collect()
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

/// A host running a module, and the directory its files are in.
struct Host {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    dir: PathBuf,
}

impl Host {
    fn start(c: &Compiled<'_>) -> Result<Host, Wasm> {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "moddil-wasm-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let io = |e: std::io::Error| Wasm::NoHost(e.to_string());
        std::fs::create_dir_all(&dir).map_err(io)?;
        let (rt, module, host) = (
            dir.join("runtime.wasm"),
            dir.join("module.wasm"),
            dir.join("host.cjs"),
        );
        std::fs::write(&rt, crate::runtime::BYTES).map_err(io)?;
        std::fs::write(&module, &c.bytes).map_err(io)?;
        std::fs::write(&host, include_str!("host.cjs")).map_err(io)?;
        let l = c.layout;
        let mut child = Command::new(node())
            .arg(&host)
            .arg(&rt)
            .arg(&module)
            .args(
                [
                    l.pages,
                    l.out,
                    l.err,
                    l.record,
                    RECORD_WORDS,
                    l.data.0,
                    l.data.1,
                ]
                .map(|n| n.to_string()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| Wasm::NoHost(format!("running {}: {e}", node())))?;
        let stdin = child
            .stdin
            .take()
            .ok_or(Wasm::NoHost("no pipe".to_string()))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or(Wasm::NoHost("no pipe".to_string()))?,
        );
        let mut h = Host {
            child,
            stdin,
            stdout,
            dir,
        };
        let l = h.line()?;
        if !l.contains_key("ready") {
            return Err(Wasm::Crash(format!("the host did not start: {l:?}")));
        }
        Ok(h)
    }

    fn send(&mut self, cmd: &str) -> Result<(), Wasm> {
        writeln!(self.stdin, "{cmd}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| Wasm::Crash(format!("the host stopped: {e}")))
    }

    /// The next line the host writes.
    fn line(&mut self) -> Result<Line, Wasm> {
        let mut s = String::new();
        loop {
            s.clear();
            let n = self
                .stdout
                .read_line(&mut s)
                .map_err(|e| Wasm::Crash(e.to_string()))?;
            if n == 0 {
                return Err(Wasm::Crash("the host stopped".to_string()));
            }
            if let Some(l) = parse(&s) {
                if let Some(f) = l.get("fatal") {
                    return Err(Wasm::Crash(f.str().to_string()));
                }
                return Ok(l);
            }
        }
    }

    /// Sends `cmd` and gives the answer.
    fn ask(&mut self, cmd: &str) -> Result<Line, Wasm> {
        self.send(cmd)?;
        self.line()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.send("quit");
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// How much of memory is read at a time.
const PAGE: u64 = 4096;

/// The module's memory, read over the pipe a page at a time while the values are there.
struct Remote<'h> {
    host: RefCell<&'h mut Host>,
    pages: RefCell<HashMap<u64, Vec<u8>>>,
}

impl<'h> Remote<'h> {
    fn new(host: &'h mut Host) -> Self {
        Remote {
            host: RefCell::new(host),
            pages: RefCell::new(HashMap::new()),
        }
    }

    fn byte(&self, addr: u64) -> u8 {
        let page = addr / PAGE;
        let mut pages = self.pages.borrow_mut();
        let p = pages.entry(page).or_insert_with(|| {
            let mut h = self.host.borrow_mut();
            h.ask(&format!("read {} {PAGE}", page * PAGE))
                .map(|l| unhex(l.get("hex").map_or("", Value::str)))
                .unwrap_or_default()
        });
        p.get((addr % PAGE) as usize).copied().unwrap_or(0)
    }
}

impl Memory for Remote<'_> {
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

/// What an entry gave: the values it observed, and its result (as Moddil source, with the exit
/// code for an `ExitCode`), or why there is none.
type Ran = (Vec<Observed>, Result<(String, Option<u8>), Wasm>);

/// Runs entry `k` of `c` in `host`, as the JIT runs one.
fn run_entry(c: &Compiled<'_>, host: &mut Host, k: usize) -> Ran {
    let mut show = Show {
        items: c.items,
        layouts: Layouts::new(c.items),
        dyn_types: &c.dyn_types,
    };
    let mut observed = Vec::new();
    let mut parts: Vec<(TextRange, Option<String>)> = Vec::new();
    let entry = &c.entries[k];
    let status = match (|| -> Result<i64, Wasm> {
        host.send(&format!("run {k}"))?;
        loop {
            let l = host.line()?;
            if let Some(site) = l.get("o") {
                let addr = l.get("a").map_or(0, Value::num) as u64;
                let mem = Remote::new(host);
                match c.sites.get(site.num() as usize) {
                    Some(Site::Observe(file, range, ty)) => {
                        let text = show.render(&mem, ty, addr);
                        observed.push((*file, *range, text));
                    }
                    Some(Site::Part(range, ty, first)) => {
                        if *first {
                            parts.clear();
                        }
                        let text = ty.as_ref().map(|t| show.render(&mem, t, addr));
                        parts.push((*range, text));
                    }
                    _ => {}
                }
                drop(mem);
                host.send("go")?;
            } else if let Some(cr) = l.get("crash") {
                return Err(Wasm::Crash(cr.str().to_string()));
            } else {
                return Ok(l.get("s").map_or(-1, Value::num));
            }
        }
    })() {
        Ok(s) => s,
        Err(e) => return (observed, Err(e)),
    };
    let out = u64::from(c.layout.out);
    let text = {
        let mem = Remote::new(host);
        match status {
            0 => Some(show.render(&mem, &entry.ret, out)),
            2 => Some(show.error(&mem, &entry.errors, u64::from(c.layout.err))),
            _ => None,
        }
    };
    let exit = if status == 0 {
        let mem = Remote::new(host);
        show.exit_code(&mem, &entry.ret, out)
    } else {
        None
    };
    // Once shown, the result is dropped; then a run that returned has freed everything.
    let mut status = status;
    let mut leaked = 0;
    let after = (|| -> Result<(), Wasm> {
        if status == 0 {
            let l = host.ask(&format!("drop {k}"))?;
            if let Some(cr) = l.get("crash") {
                return Err(Wasm::Crash(cr.str().to_string()));
            }
            status = l.get("s").map_or(-1, Value::num);
            if status == 0 {
                leaked = host.ask("live")?.get("n").map_or(0, Value::num);
            }
        }
        Ok(())
    })();
    if let Err(e) = after {
        return (observed, Err(e));
    }
    let result = match text {
        Some(_) if status == 0 && leaked != 0 => Err(Wasm::Leak(leaked)),
        Some(t) if status == 0 => Ok((t, exit)),
        _ => match record(host) {
            Err(e) => Err(e),
            Ok((trap, failed)) => match (status, failed, trap) {
                (2, Some(f), _) => match (c.sites.get(f as usize), text) {
                    (Some(Site::Fail(file, range)), Some(t)) => Err(Wasm::Failed(*file, *range, t)),
                    _ => Err(Wasm::Unsupported("an error without a site".to_string())),
                },
                (_, _, Some(t)) => Err(trap_of(&c.sites, &t, parts)),
                _ => Err(Wasm::Crash(format!(
                    "a run that ended with status {status}"
                ))),
            },
        },
    };
    (observed, result)
}

/// What the run recorded: its trap, and the last `fail`'s site.
fn record(host: &mut Host) -> Result<(Option<moddil_rt::Trapped>, Option<u32>), Wasm> {
    let l = host.ask("record")?;
    let w: Vec<u32> = l
        .get("w")
        .map_or("", Value::str)
        .split(',')
        .filter_map(|x| x.parse().ok())
        .collect();
    let at = |k: usize| w.get(k).copied().unwrap_or(0);
    let trap = (at(0) == 1).then(|| moddil_rt::Trapped {
        site: at(1),
        tracked: at(2) == 1,
        stack: w.iter().skip(6).take(at(5) as usize).copied().collect(),
    });
    let failed = (at(3) == 1).then(|| at(4));
    Ok((trap, failed))
}

/// The trap as the interpreters report it, with a failed `assert`'s parts if it is reported
/// where it happened.
fn trap_of(
    sites: &[Site],
    t: &moddil_rt::Trapped,
    parts: Vec<(TextRange, Option<String>)>,
) -> Wasm {
    match moddil_native::backend::reported(sites, t) {
        moddil_native::Native::Trap(class, file, range, _) => {
            let here = t.reported_at(|_| false) == t.site && !t.tracked;
            Wasm::Trap(class, file, range, if here { parts } else { Vec::new() })
        }
        moddil_native::Native::Unsupported(why) => Wasm::Unsupported(why),
        other => Wasm::Crash(format!("{other:?}")),
    }
}

/// Runs tests `tests` of `program` in Node, all in one module: what each observed and how it
/// ended.
///
/// # Errors
///
/// [`Wasm::Unsupported`] if any of them reaches code not compiled to WASM yet, and
/// [`Wasm::NoHost`] if there is no Node to run them in.
pub fn run_tests(db: &dyn Db, program: Program, tests: &[u32]) -> Result<TestRuns, Wasm> {
    let items: Vec<Item> = tests.iter().map(|t| Item::Test(*t)).collect();
    let c = compile(db, program, &items).map_err(Wasm::Unsupported)?;
    let mut host = Host::start(&c)?;
    let mut out = Vec::new();
    for k in 0..items.len() {
        let (observed, result) = run_entry(&c, &mut host, k);
        // A crash leaves the host in no state to go on.
        let crashed = matches!(result, Err(Wasm::Crash(_)));
        out.push((observed, result.map(|_| ())));
        if crashed && k + 1 < items.len() {
            host = Host::start(&c)?;
        }
    }
    Ok(out)
}

/// Runs `fn main()` (function `index`) of `program` in Node: its result, as Moddil source, and
/// the process's exit code if it gives an `ExitCode` (`main.exit.1`).
///
/// # Errors
///
/// As [`run_tests`], and [`Wasm::Trap`] if it traps or [`Wasm::Failed`] if an error leaves it.
pub fn run_main(db: &dyn Db, program: Program, index: u32) -> Result<(String, Option<u8>), Wasm> {
    let c = compile(db, program, &[Item::Fn(index)]).map_err(Wasm::Unsupported)?;
    let mut host = Host::start(&c)?;
    run_entry(&c, &mut host, 0).1
}
