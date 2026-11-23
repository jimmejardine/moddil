//! `moddil lsp`: an editor's session over the protocol. It opens a file with an error, gets the
//! published diagnostics, fixes it, hovers, goes to a definition and completes, then shuts the
//! server down.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

/// The server, killed when the test ends, however it ends.
struct Server {
    child: Child,
    stdin: ChildStdin,
    from: mpsc::Receiver<Value>,
    next_id: i64,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start() -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_moddil"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("moddil lsp starts");
        let stdin = child.stdin.take().expect("its input");
        let mut out = BufReader::new(child.stdout.take().expect("its output"));
        let (tx, from) = mpsc::channel();
        // Each message: `Content-Length: n`, a blank line, then n bytes of JSON.
        std::thread::spawn(move || {
            loop {
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if out.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(n) = line.strip_prefix("Content-Length:") {
                        length = n.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; length];
                if out.read_exact(&mut body).is_err() {
                    return;
                }
                let Ok(v) = serde_json::from_slice(&body) else {
                    return;
                };
                if tx.send(v).is_err() {
                    return;
                }
            }
        });
        Server {
            child,
            stdin,
            from,
            next_id: 1,
        }
    }

    fn send(&mut self, v: &Value) {
        let body = v.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).expect("sent");
        self.stdin.flush().expect("flushed");
    }

    fn notify(&mut self, method: &str, params: Value) {
        let mut m = json!({ "jsonrpc": "2.0", "method": method });
        m["params"] = params;
        self.send(&m);
    }

    /// The next message the server sends that `want` accepts.
    fn wait_for(&mut self, want: impl Fn(&Value) -> bool) -> Value {
        loop {
            let v = self
                .from
                .recv_timeout(Duration::from_secs(60))
                .expect("the server answers within 60 s");
            if want(&v) {
                return v;
            }
        }
    }

    /// Sends request `method` and gives its result.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut m = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        m["params"] = params;
        self.send(&m);
        let r = self.wait_for(|v| v["id"] == json!(id));
        assert!(r.get("error").is_none(), "{method}: {r}");
        r["result"].clone()
    }

    /// The next diagnostics published for `uri`.
    fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        let v = self.wait_for(|v| {
            v["method"] == "textDocument/publishDiagnostics" && v["params"]["uri"] == uri
        });
        v["params"]["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}

const FIXED: &str = "\
pub struct Point {
    pub x: int,
    pub y: int,
}

fn norm(p: Point) -> int {
    p.x * p.x + p.y * p.y
}

fn main() -> int {
    let p = Point { x: 3, y: 4 };
    norm(p)
}
";

#[test]
fn an_editor_session_over_the_protocol() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lsp-session");
    std::fs::create_dir_all(&dir).expect("a directory");
    let path = dir.join("session.moddil");
    let broken = FIXED.replace("norm(p)\n}", "norm(p) + true\n}");
    std::fs::write(&path, &broken).expect("written");
    let uri = moddil_lsp_uri(&path);

    let mut s = Server::start();
    let init = s.request(
        "initialize",
        json!({ "processId": null, "rootUri": null, "capabilities": {} }),
    );
    let caps = &init["capabilities"];
    assert_eq!(caps["hoverProvider"], json!(true), "{init}");
    assert_eq!(caps["definitionProvider"], json!(true), "{init}");
    assert!(caps["completionProvider"].is_object(), "{init}");
    s.notify("initialized", json!({}));

    // The file with an error: its diagnostic, with its code, where the error is.
    s.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "moddil", "version": 1, "text": broken } }),
    );
    let ds = s.diagnostics(&uri);
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert!(
        ds[0]["code"].as_str().is_some_and(|c| c.starts_with('E')),
        "{ds:?}"
    );
    assert_eq!(ds[0]["range"]["start"]["line"], json!(11), "{ds:?}");

    // Fixed: the diagnostics are cleared.
    s.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": FIXED }] }),
    );
    assert!(s.diagnostics(&uri).is_empty());

    // Hover over `norm` in `main` (line 11, column 4).
    let pos = |line: u32, character: u32| json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } });
    let hover = s.request("textDocument/hover", pos(11, 5));
    let text = hover["contents"]["value"].as_str().unwrap_or_default();
    assert!(text.contains("fn norm(p: Point) -> int"), "{hover}");

    // Its definition: line 5, where `norm` is declared.
    let def = s.request("textDocument/definition", pos(11, 5));
    assert_eq!(
        def["range"]["start"],
        json!({ "line": 5, "character": 3 }),
        "{def}"
    );

    // Completion after `p.` in `norm` (line 6, after `p.`).
    let items = s.request("textDocument/completion", pos(6, 6));
    let labels: Vec<&str> = items
        .as_array()
        .map(|a| a.iter().filter_map(|i| i["label"].as_str()).collect())
        .unwrap_or_default();
    assert!(labels.contains(&"x") && labels.contains(&"y"), "{items}");

    let done = s.request("shutdown", Value::Null);
    assert_eq!(done, Value::Null);
    s.notify("exit", Value::Null);
    // It exits by itself, promptly.
    let mut status = None;
    for _ in 0..300 {
        status = s.child.try_wait().expect("its status");
        if status.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = status.expect("the server exits within 30 s of `exit`");
    assert!(status.success(), "{status}");
}

/// The URI an editor gives `path`.
fn moddil_lsp_uri(path: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(path).expect("the file exists");
    moddil_lsp::path_uri(&canonical)
}
