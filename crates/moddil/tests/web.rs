//! `moddil build --wasm`: the files it writes run a program in Node and in a browser (headless
//! Chrome, served from a local HTTP server that the page reports back to), with the same log and
//! exit code as native code.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Uses a task (so JSPI), recursion, the log and an exit code.
const PROGRAM: &str = "\
fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
fn main() -> ExitCode {
    log.info(fmt\"fib(20) is {fib(20)}\");
    let r = scope(|s| { let a = s.spawn(|| fib(10)); let b = s.spawn(|| fib(11)); a.join() + b.join() });
    if r == 144 { ExitCode::Other(3) } else { ExitCode::Software }
}
";

/// Builds `text` (as `name.moddil`) with `--wasm` into a directory of its own: the glue's path.
fn built(name: &str, text: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("web")
        .join(name);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let src = dir.join(format!("{name}.moddil"));
    std::fs::write(&src, text).expect("written");
    let glue = dir.join(format!("{name}.mjs"));
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .arg("build")
        .arg("--wasm")
        .arg(&src)
        .arg("-o")
        .arg(&glue)
        .output()
        .expect("moddil runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for f in [
        format!("{name}.mjs"),
        format!("{name}.wasm"),
        "moddil-rt.wasm".to_string(),
        format!("{name}.html"),
    ] {
        assert!(dir.join(&f).exists(), "{f} written");
    }
    glue
}

/// Node, if it has JSPI (Node 26).
fn node() -> Option<String> {
    let node = std::env::var("MODDIL_NODE").unwrap_or_else(|_| "node".to_string());
    let out = Command::new(&node)
        .args([
            "-e",
            "process.exit(typeof WebAssembly.Suspending === 'function' ? 0 : 1)",
        ])
        .output()
        .ok()?;
    out.status.success().then_some(node)
}

#[test]
fn a_built_program_runs_in_node() {
    let glue = built("hello", PROGRAM);
    let Some(node) = node() else {
        eprintln!("skipped: no Node with JSPI");
        return;
    };
    let out = Command::new(node).arg(&glue).output().expect("node runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("[info] fib(20) is 6765"), "{stderr}");
}

#[test]
fn a_built_program_reports_a_trap_in_node() {
    let glue = built(
        "boom",
        "fn main() { let v = Vec::from_array([1, 2]); let i = 5; let x = v[i]; }\n",
    );
    let Some(node) = node() else {
        eprintln!("skipped: no Node with JSPI");
        return;
    };
    let out = Command::new(node).arg(&glue).output().expect("node runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(70), "{stderr}");
    assert!(stderr.contains("trap: index out of bounds"), "{stderr}");
    assert!(stderr.contains("boom.moddil:1:"), "{stderr}");
}

/// Chrome, Chromium or Edge, from `MODDIL_CHROME`, the path, or their usual places.
fn chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MODDIL_CHROME") {
        return Some(PathBuf::from(p));
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if Command::new(name)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some(PathBuf::from(name));
        }
    }
    [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// Appended to the served `.html` page: once the program has set `data-exit`, it posts the page's
/// DOM to `/done`, so the test waits for the program in real time rather than guessing.
const REPORTER: &str = "<script>
const poll = setInterval(() => {
  if (document.body && document.body.dataset.exit !== undefined) {
    clearInterval(poll);
    fetch('/done', { method: 'POST', body: document.documentElement.outerHTML });
  }
}, 50);
</script>
";

/// Serves the files of `dir` over HTTP on a free local port, with the types browsers need (a
/// module must be JavaScript, a module's bytes `application/wasm`), until the test ends. An `.html`
/// page gets [`REPORTER`] added, and what it posts to `/done` comes out of the receiver.
fn serve(dir: &Path) -> (u16, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
    let port = listener.local_addr().expect("its address").port();
    let dir = dir.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let dir = dir.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().expect("a stream"));
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    return;
                }
                // The headers, up to the blank line, keeping the body's length.
                let mut length = 0;
                let mut h = String::new();
                while reader.read_line(&mut h).is_ok_and(|n| n > 2) {
                    if let Some((name, value)) = h.split_once(':')
                        && name.trim().eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    h.clear();
                }
                let path = request.split(' ').nth(1).unwrap_or("/");
                if request.starts_with("POST ") && path == "/done" {
                    let mut body = vec![0; length];
                    if reader.read_exact(&mut body).is_ok() {
                        let _ = tx.send(String::from_utf8_lossy(&body).into_owned());
                    }
                    let _ = stream.write_all(
                        b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    );
                    return;
                }
                let file = dir.join(path.trim_start_matches('/').split('?').next().unwrap_or(""));
                let (status, kind, body) = match std::fs::read(&file) {
                    Ok(mut b) => {
                        if file.extension().is_some_and(|e| e == "html") {
                            b.extend_from_slice(REPORTER.as_bytes());
                        }
                        let kind = match file.extension().and_then(|e| e.to_str()) {
                            Some("mjs" | "js") => "text/javascript",
                            Some("wasm") => "application/wasm",
                            Some("html") => "text/html; charset=utf-8",
                            _ => "application/octet-stream",
                        };
                        ("200 OK", kind, b)
                    }
                    Err(_) => ("404 Not Found", "text/plain", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    (port, rx)
}

/// A running browser, killed when the test ends, however it ends.
struct Browser(Child);

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_built_program_runs_in_a_browser() {
    let glue = built("page", PROGRAM);
    let Some(chrome) = chrome() else {
        eprintln!("skipped: no Chrome to run the page in");
        return;
    };
    let dir = glue.parent().expect("its directory");
    let (port, done) = serve(dir);
    let profile = dir.join("chrome-profile");
    let _ = std::fs::remove_dir_all(&profile);
    // With no `--dump-dom`, the browser keeps running until it is killed.
    let _browser = Browser(
        Command::new(chrome)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--no-sandbox",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("http://127.0.0.1:{port}/page.html"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("chrome runs"),
    );
    let dom = done
        .recv_timeout(Duration::from_secs(120))
        .expect("the page finishes within 120 s");
    assert!(dom.contains("data-exit=\"3\""), "{dom}");
    assert!(dom.contains("[info] fib(20) is 6765"), "{dom}");
}

/// The Mandelbulb example (`examples/mandelbulb/web`) in a browser: its first frame, drawn by
/// the kernel lowered to WGSL on WebGPU (SwiftShader's, in a headless Chrome), a thread per
/// pixel, and read back, has the colours of the CPU simulator's golden image, within a tolerance
/// for the GPU's own arithmetic; the frames after it are shown with no read-back.
#[test]
fn the_mandelbulb_flies_on_webgpu_in_a_browser() {
    let Some(chrome) = chrome() else {
        eprintln!("skipped: no Chrome to run the page in");
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("web")
        .join("flight");
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mandelbulb/web");
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .args(["build", "--wasm"])
        .arg(&example)
        .arg("-o")
        .arg(dir.join("flight.mjs"))
        .output()
        .expect("moddil runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (port, done) = serve(&dir);
    let profile = dir.join("chrome-profile");
    let _ = std::fs::remove_dir_all(&profile);
    let _browser = Browser(
        Command::new(chrome)
            .args([
                "--headless=new",
                "--no-first-run",
                "--no-sandbox",
                // WebGPU on the CPU, so the test needs no GPU.
                "--enable-unsafe-webgpu",
                "--use-webgpu-adapter=swiftshader",
                // SwiftShader's WebGPU runs over Vulkan, which Linux's Chrome enables on request.
                "--enable-features=Vulkan",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!(
                "http://127.0.0.1:{port}/flight.html?size=48x24&frames=3"
            ))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("chrome runs"),
    );
    // SwiftShader compiles and runs the kernel on the CPU, slowly when the other tests load it.
    let dom = done
        .recv_timeout(Duration::from_secs(300))
        .expect("the page finishes within 300 s");
    assert!(dom.contains("data-exit=\"0\""), "{dom}");
    // The GPU drew it, unless the browser has no WebGPU adapter at all: SwiftShader's WebGPU
    // runs on Vulkan, which Chrome does not ship for macOS, and there the CPU simulator draws
    // (and must match exactly).
    let no_adapter = dom.contains("data-gpu=\"none\"");
    assert!(
        dom.contains("data-gpu=\"used\"") || (no_adapter && cfg!(target_os = "macos")),
        "the GPU did not run: {dom}"
    );
    if no_adapter {
        eprintln!("no WebGPU adapter: the first frame is the CPU simulator's");
    }
    // `first frame 48x24: {r}, {g}, {b}`, each channel's sum, against the simulator's.
    let line = dom
        .lines()
        .find_map(|l| {
            l.split_once("first frame 48x24: ")
                .map(|(_, r)| r.to_string())
        })
        .unwrap_or_else(|| panic!("no first frame: {dom}"));
    let nums: Vec<i64> = line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("a number"))
        .collect();
    let want = [183_982, 199_192, 232_097];
    eprintln!("the GPU's sums: {nums:?} (the simulator's: {want:?})");
    for (got, want) in nums.iter().zip(want) {
        assert!(
            (got - want).abs() <= want * 3 / 100,
            "{nums:?} against {want}"
        );
    }
    assert_eq!(nums.len(), 3, "{line}");
}

/// On WebGPU (SwiftShader's, in a headless Chrome), `int` in a kernel is 64 bits, as on the CPU,
/// and a check that fails in a kernel traps the program at its source (`gpu.int.1`,
/// `gpu.trap.1`): `tests/gpu-checks`' kernels give the simulator's image, then one overflows.
#[test]
fn a_kernel_on_webgpu_has_64_bit_ints_and_traps() {
    let Some(chrome) = chrome() else {
        eprintln!("skipped: no Chrome to run the page in");
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("web")
        .join("gpu-checks");
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/gpu-checks");
    let out = Command::new(env!("CARGO_BIN_EXE_moddil"))
        .args(["build", "--wasm"])
        .arg(&project)
        .arg("-o")
        .arg(dir.join("checks.mjs"))
        .output()
        .expect("moddil runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (port, done) = serve(&dir);
    let profile = dir.join("chrome-profile");
    let _ = std::fs::remove_dir_all(&profile);
    let _browser = Browser(
        Command::new(chrome)
            .args([
                "--headless=new",
                "--no-first-run",
                "--no-sandbox",
                "--enable-unsafe-webgpu",
                "--use-webgpu-adapter=swiftshader",
                "--enable-features=Vulkan",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("http://127.0.0.1:{port}/checks.html"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("chrome runs"),
    );
    let dom = done
        .recv_timeout(Duration::from_secs(300))
        .expect("the page finishes within 300 s");
    if dom.contains("data-gpu=\"none\"") && cfg!(target_os = "macos") {
        eprintln!("skipped: no WebGPU adapter");
        return;
    }
    assert!(
        dom.contains("data-gpu=\"used\""),
        "the GPU did not run: {dom}"
    );
    // The 64-bit arithmetic gave the simulator's image.
    assert!(dom.contains("wide ints agree"), "{dom}");
    // A tile of 16 x 4 is a workgroup of that size, and draws the simulator's image.
    assert!(dom.contains("tiles of 16 x 4 agree"), "{dom}");
    // The overflow trapped the program, at the product in the kernel's source.
    assert!(!dom.contains("not reached"), "{dom}");
    assert!(
        dom.contains("overflow") && dom.contains("main.moddil:37"),
        "{dom}"
    );
    assert!(!dom.contains("data-exit=\"0\""), "{dom}");
}
