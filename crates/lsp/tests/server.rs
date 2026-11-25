//! The language server over an in-memory connection: incremental edits, and the problems
//! published after them by the analysis thread.

use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use serde_json::{Value, json};

fn file() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("moddil-lsp-server-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let path = dir.join("main.moddil");
    std::fs::write(&path, "fn f() -> int {\n    1\n}\n").expect("the file");
    path
}

/// The next `textDocument/publishDiagnostics` the server sends: its diagnostics' codes.
fn next_codes(client: &Connection) -> Vec<String> {
    loop {
        let msg = client
            .receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("a message from the server");
        if let Message::Notification(n) = msg
            && n.method == "textDocument/publishDiagnostics"
        {
            return n.params["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .map(|d| d["code"].as_str().unwrap_or_default().to_string())
                .collect();
        }
    }
}

fn notify(client: &Connection, method: &str, params: Value) {
    client
        .sender
        .send(Message::Notification(Notification::new(
            method.to_string(),
            params,
        )))
        .expect("sent");
}

#[test]
fn edits_are_applied_in_turn_problems_published_and_documents_formatted() {
    let (server, client) = Connection::memory();
    let thread = std::thread::spawn(move || moddil_lsp::serve(&server));
    client
        .sender
        .send(Message::Request(Request::new(
            RequestId::from(1),
            "initialize".to_string(),
            json!({ "capabilities": {} }),
        )))
        .expect("sent");
    let init = client
        .receiver
        .recv_timeout(Duration::from_secs(60))
        .expect("the reply");
    let Message::Response(r) = init else {
        panic!("a response: {init:?}")
    };
    // Incremental sync (`TextDocumentSyncKind::INCREMENTAL` is 2).
    let result = r.response_result.expect("a result");
    assert_eq!(result["capabilities"]["textDocumentSync"], 2);
    notify(&client, "initialized", json!({}));

    let path = file();
    let uri = moddil_lsp::path_uri(&path);
    notify(
        &client,
        "textDocument/didOpen",
        json!({ "textDocument": {
            "uri": uri, "languageId": "moddil", "version": 1,
            "text": "fn f() -> int {\n    1\n}\n" } }),
    );
    assert_eq!(next_codes(&client), Vec::<String>::new());
    // `1` becomes `true`: a mismatch. Then `true` becomes `2` and `3` in two more edits.
    let edit = |version: i32, line: u32, from: u32, to: u32, text: &str| {
        json!({
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{
                "range": {
                    "start": { "line": line, "character": from },
                    "end": { "line": line, "character": to } },
                "text": text }] })
    };
    notify(&client, "textDocument/didChange", edit(2, 1, 4, 5, "true"));
    assert_eq!(next_codes(&client), vec!["E0201".to_string()]);
    notify(&client, "textDocument/didChange", edit(3, 1, 4, 8, "2"));
    notify(&client, "textDocument/didChange", edit(4, 1, 4, 5, "3"));
    // The last publish is of the text after both edits: no problem.
    assert_eq!(next_codes(&client), Vec::<String>::new());

    // Formatting: the whole document, in `moddil fmt`'s layout.
    notify(
        &client,
        "textDocument/didChange",
        json!({
        "textDocument": { "uri": uri, "version": 5 },
        "contentChanges": [{ "text": "fn f()->int{3}
" }] }),
    );
    client
        .sender
        .send(Message::Request(Request::new(
            RequestId::from(3),
            "textDocument/formatting".to_string(),
            json!({ "textDocument": { "uri": uri }, "options": { "tabSize": 4, "insertSpaces": true } }),
        )))
        .expect("sent");
    let edits = loop {
        match client
            .receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("the reply")
        {
            Message::Response(r) if r.id == RequestId::from(3) => {
                break r.response_result.expect("edits");
            }
            _ => {}
        }
    };
    assert_eq!(
        edits[0]["newText"],
        "fn f() -> int {
    3
}
"
    );
    assert_eq!(edits[0]["range"]["end"]["line"], 1);

    client
        .sender
        .send(Message::Request(Request::new(
            RequestId::from(2),
            "shutdown".to_string(),
            Value::Null,
        )))
        .expect("sent");
    notify(&client, "exit", Value::Null);
    thread.join().expect("the server thread").expect("served");
}
