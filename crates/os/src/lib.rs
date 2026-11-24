//! The host's side of the `core` mod's `os` module (`reference/2027/os.md`), for the runtimes
//! written in Rust: the MIR interpreter and moddil-rt's native build.
//!
//! What the OS brings arrives as **events**, which a runtime's scheduler puts into the channels
//! a program waits on: a connection a listener accepted, bytes a connection received, its end,
//! or a signal. A scheduler with no task ready asks [`Os::poll`] for them, waiting until the next
//! timer at most. Handles are numbers, so nothing here knows a program's values; bytes are copied
//! into and out of the program's buffers.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// A signal: an interrupt (Ctrl-C, or a `stop` line on standard input) or a request to end.
pub const INTERRUPT: i64 = 1;
/// A request to end (SIGTERM, or the console closing).
pub const TERMINATE: i64 = 2;

/// What the OS brought, for the channels a program waits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A listener (the first id) accepted a connection (the second).
    Accepted(usize, usize),
    /// A connection received bytes, which [`Os::take`] gives: its id, and how many.
    Data(usize, usize),
    /// A connection has ended: nothing more arrives.
    Ended(usize),
    /// A signal: [`INTERRUPT`] or [`TERMINATE`].
    Signal(i64),
}

enum Handle {
    Listener { socket: TcpListener, watched: bool },
    Conn(Conn),
    Closed,
}

struct Conn {
    stream: TcpStream,
    /// What arrived and was not taken yet.
    buf: VecDeque<u8>,
    /// Whether the program waits for its bytes.
    watched: bool,
    ended: bool,
}

/// The OS, as one run sees it.
pub struct Os {
    handles: Vec<Handle>,
    signals: Option<Receiver<i64>>,
    args: Vec<String>,
}

impl Os {
    /// The OS for a run whose program was given `args` (after its name).
    pub fn new(args: Vec<String>) -> Os {
        Os {
            handles: Vec::new(),
            signals: None,
            args,
        }
    }

    /// The program's arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Listens on `port` of the local machine; its id.
    ///
    /// # Errors
    /// Why the port cannot be listened on.
    pub fn listen(&mut self, port: u16) -> Result<usize, String> {
        let socket = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(self.add(Handle::Listener {
            socket,
            watched: false,
        }))
    }

    /// A connection to `port` at `host`; its id.
    ///
    /// # Errors
    /// Why it cannot be made.
    pub fn connect(&mut self, host: &str, port: u16) -> Result<usize, String> {
        let stream = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
        self.add_conn(stream)
    }

    fn add_conn(&mut self, stream: TcpStream) -> Result<usize, String> {
        stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);
        Ok(self.add(Handle::Conn(Conn {
            stream,
            buf: VecDeque::new(),
            watched: false,
            ended: false,
        })))
    }

    fn add(&mut self, h: Handle) -> usize {
        self.handles.push(h);
        self.handles.len() - 1
    }

    /// From now on, `id`'s events are wanted: a listener's connections, or a connection's bytes.
    pub fn watch(&mut self, id: usize) {
        match self.handles.get_mut(id) {
            Some(Handle::Listener { watched, .. }) => *watched = true,
            Some(Handle::Conn(c)) => c.watched = true,
            _ => {}
        }
    }

    /// From now on, signals are wanted: Ctrl-C and termination, and a `stop` line on standard
    /// input.
    pub fn watch_signals(&mut self) {
        if self.signals.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctrl = tx.clone();
        // Only one handler can be set per process; a second run in it shares the first's.
        let _ = ctrlc::set_handler(move || {
            let _ = ctrl.send(INTERRUPT);
        });
        std::thread::spawn(move || {
            let mut line = String::new();
            while std::io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) {
                if line.trim() == "stop" && tx.send(INTERRUPT).is_err() {
                    break;
                }
                line.clear();
            }
        });
        self.signals = Some(rx);
    }

    /// Copies up to `out.len()` of the bytes connection `conn` has received into `out`; how many.
    pub fn take(&mut self, conn: usize, out: &mut [u8]) -> usize {
        let Some(Handle::Conn(c)) = self.handles.get_mut(conn) else {
            return 0;
        };
        let n = out.len().min(c.buf.len());
        for (o, b) in out.iter_mut().zip(c.buf.drain(..n)) {
            *o = b;
        }
        n
    }

    /// Sends `bytes` on connection `conn`, all of them.
    ///
    /// # Errors
    /// Why they cannot be sent.
    pub fn write(&mut self, conn: usize, bytes: &[u8]) -> Result<(), String> {
        let Some(Handle::Conn(c)) = self.handles.get_mut(conn) else {
            return Err("not a connection".to_string());
        };
        c.stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        let r = c.stream.write_all(bytes).and_then(|()| c.stream.flush());
        c.stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        r.map_err(|e| e.to_string())
    }

    /// Closes listener or connection `id`.
    pub fn close(&mut self, id: usize) {
        if let Some(Handle::Conn(c)) = self.handles.get(id) {
            let _ = c.stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(h) = self.handles.get_mut(id) {
            *h = Handle::Closed;
        }
    }

    /// Whether anything watched could still bring an event.
    pub fn waiting(&self) -> bool {
        self.signals.is_some()
            || self.handles.iter().any(|h| match h {
                Handle::Listener { watched, .. } => *watched,
                Handle::Conn(c) => c.watched && !c.ended,
                Handle::Closed => false,
            })
    }

    /// The events since the last poll, waiting up to `wait` (forever if `None`) for at least
    /// one.
    pub fn poll(&mut self, wait: Option<Duration>) -> Vec<Event> {
        let until = wait.map(|w| Instant::now() + w);
        loop {
            let events = self.events();
            if !events.is_empty() || until.is_some_and(|u| Instant::now() >= u) {
                return events;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn events(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if let Some(rx) = &self.signals {
            while let Ok(s) = rx.try_recv() {
                out.push(Event::Signal(s));
            }
        }
        let mut accepted = Vec::new();
        for (id, h) in self.handles.iter_mut().enumerate() {
            match h {
                Handle::Listener {
                    socket,
                    watched: true,
                } => {
                    while let Ok((stream, _)) = socket.accept() {
                        accepted.push((id, stream));
                    }
                }
                Handle::Conn(c) if c.watched && !c.ended => {
                    let mut chunk = [0u8; 4096];
                    let mut n = 0;
                    loop {
                        match c.stream.read(&mut chunk) {
                            Ok(0) => {
                                c.ended = true;
                                break;
                            }
                            Ok(k) => {
                                c.buf.extend(&chunk[..k]);
                                n += k;
                            }
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(e) if e.kind() == ErrorKind::Interrupted => {}
                            Err(_) => {
                                c.ended = true;
                                break;
                            }
                        }
                    }
                    if n > 0 {
                        out.push(Event::Data(id, n));
                    }
                    if c.ended {
                        out.push(Event::Ended(id));
                    }
                }
                _ => {}
            }
        }
        for (listener, stream) in accepted {
            if let Ok(conn) = self.add_conn(stream) {
                out.push(Event::Accepted(listener, conn));
            }
        }
        out
    }
}

/// The system's real time, in nanoseconds since 1970-01-01T00:00:00 UTC; before it, negative.
pub fn wall_clock() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_nanos()).map_or(i64::MIN, |n| -n),
    }
}

/// A random number from the OS's sources: the system's randomly keyed hasher, which is seeded
/// from the OS, over a counter.
pub fn random() -> i64 {
    use std::hash::{BuildHasher, Hasher};
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    (h.finish() >> 1) as i64
}

/// Writes `text` to standard error.
pub fn write_stderr(text: &str) {
    let mut e = std::io::stderr();
    let _ = e.write_all(text.as_bytes());
    let _ = e.flush();
}
