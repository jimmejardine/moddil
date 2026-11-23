//! The LSP loop: requests and notifications over standard input and output, answered from an
//! [`Analysis`]. Documents are synchronised whole; after each change the problems of the changed
//! file's mod are published.

use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument,
    DidSaveTextDocument, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{
    Completion as CompletionRequest, GotoDefinition, HoverRequest, InlayHintRequest, Request as _,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InlayHint, InlayHintLabel,
    InlayHintParams, Location, MarkupContent, MarkupKind, NumberOrString, OneOf, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};

use crate::text::{LineIndex, path_uri, uri_path};
use crate::{Analysis, CompletionKind};

/// Runs the language server on standard input and output until the client shuts it down.
///
/// # Errors
///
/// A broken connection, or a client that does not speak the protocol.
pub fn run() -> Result<(), String> {
    let (conn, io) = Connection::stdio();
    serve(&conn)?;
    // The writer thread ends once its channel closes: the connection goes first.
    drop(conn);
    io.join().map_err(|e| e.to_string())
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Serves `conn` until it is shut down.
fn serve(conn: &Connection) -> Result<(), String> {
    let caps = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
            ..CompletionOptions::default()
        }),
        ..ServerCapabilities::default()
    };
    conn.initialize(serde_json::to_value(caps).map_err(err)?)
        .map_err(err)?;
    let mut s = Server {
        conn,
        analysis: Analysis::new(),
    };
    for msg in &conn.receiver {
        match msg {
            Message::Request(req) => {
                if conn.handle_shutdown(&req).map_err(err)? {
                    return Ok(());
                }
                s.request(req)?;
            }
            Message::Notification(n) => s.notification(n)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

struct Server<'c> {
    conn: &'c Connection,
    analysis: Analysis,
}

fn path_of(uri: &Uri) -> Option<PathBuf> {
    uri_path(uri.as_str())
}

fn uri_of(path: &Path) -> Option<Uri> {
    path_uri(path).parse().ok()
}

impl Server<'_> {
    fn send(&self, m: impl Into<Message>) -> Result<(), String> {
        self.conn.sender.send(m.into()).map_err(err)
    }

    fn notification(&mut self, n: Notification) -> Result<(), String> {
        match n.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let p: lsp_types::DidOpenTextDocumentParams = from(n.params)?;
                if let Some(path) = path_of(&p.text_document.uri) {
                    self.analysis.set_text(&path, &p.text_document.text);
                    self.publish(&path)?;
                }
            }
            DidChangeTextDocument::METHOD => {
                let p: lsp_types::DidChangeTextDocumentParams = from(n.params)?;
                if let (Some(path), Some(change)) =
                    (path_of(&p.text_document.uri), p.content_changes.last())
                {
                    self.analysis.set_text(&path, &change.text);
                    self.publish(&path)?;
                }
            }
            DidSaveTextDocument::METHOD | DidCloseTextDocument::METHOD => {}
            // A manifest changed, or files came or went: read the programs again.
            DidChangeWatchedFiles::METHOD => self.analysis.reload(),
            _ => {}
        }
        Ok(())
    }

    /// Publishes the problems of the mod `path` is in, file by file.
    fn publish(&mut self, path: &Path) -> Result<(), String> {
        for (file, problems) in self.analysis.problems(path) {
            let text = self.analysis.text(&file).unwrap_or_default();
            let ix = LineIndex::new(&text);
            let diagnostics = problems
                .into_iter()
                .map(|p| Diagnostic {
                    range: range(&ix, p.range.start, p.range.end),
                    severity: Some(if p.error {
                        DiagnosticSeverity::ERROR
                    } else {
                        DiagnosticSeverity::WARNING
                    }),
                    code: Some(NumberOrString::String(p.code)),
                    source: Some("moddil".to_string()),
                    message: p.message,
                    ..Diagnostic::default()
                })
                .collect();
            let Some(uri) = uri_of(&file) else { continue };
            let params = PublishDiagnosticsParams {
                uri,
                diagnostics,
                version: None,
            };
            self.send(Notification::new(
                PublishDiagnostics::METHOD.to_string(),
                params,
            ))?;
        }
        Ok(())
    }

    /// The file and byte offset of a position in a document.
    fn locate(&mut self, uri: &Uri, pos: Position) -> Option<(PathBuf, u32)> {
        let path = path_of(uri)?;
        let text = self.analysis.text(&path)?;
        let offset = LineIndex::new(&text).offset(pos.line, pos.character);
        Some((path, offset))
    }

    fn request(&mut self, req: Request) -> Result<(), String> {
        let id = req.id.clone();
        let result = match req.method.as_str() {
            HoverRequest::METHOD => {
                let p: HoverParams = from(req.params)?;
                let at = &p.text_document_position_params;
                let hover = self
                    .locate(&at.text_document.uri, at.position)
                    .and_then(|(path, offset)| self.analysis.hover(&path, offset))
                    .map(|value| Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value,
                        }),
                        range: None,
                    });
                serde_json::to_value(hover)
            }
            GotoDefinition::METHOD => {
                let p: GotoDefinitionParams = from(req.params)?;
                let at = &p.text_document_position_params;
                let found = self
                    .locate(&at.text_document.uri, at.position)
                    .and_then(|(path, offset)| self.analysis.definition(&path, offset))
                    .and_then(|(file, r)| {
                        let text = self.analysis.text(&file)?;
                        let ix = LineIndex::new(&text);
                        Some(GotoDefinitionResponse::Scalar(Location {
                            uri: uri_of(&file)?,
                            range: range(&ix, r.start, r.end),
                        }))
                    });
                serde_json::to_value(found)
            }
            InlayHintRequest::METHOD => {
                let p: InlayHintParams = from(req.params)?;
                let hints = path_of(&p.text_document.uri).map(|path| {
                    let text = self.analysis.text(&path).unwrap_or_default();
                    let ix = LineIndex::new(&text);
                    self.analysis
                        .hints(&path)
                        .into_iter()
                        .map(|h| {
                            let (line, character) = ix.position(h.offset);
                            InlayHint {
                                position: Position { line, character },
                                label: InlayHintLabel::String(h.label),
                                kind: None,
                                text_edits: None,
                                tooltip: None,
                                padding_left: Some(true),
                                padding_right: None,
                                data: None,
                            }
                        })
                        .collect::<Vec<_>>()
                });
                serde_json::to_value(hints)
            }
            CompletionRequest::METHOD => {
                let p: CompletionParams = from(req.params)?;
                let at = &p.text_document_position;
                let items = self
                    .locate(&at.text_document.uri, at.position)
                    .map(|(path, offset)| self.analysis.complete(&path, offset))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| CompletionItem {
                        label: c.label,
                        kind: Some(kind(c.kind)),
                        detail: (!c.detail.is_empty()).then_some(c.detail),
                        ..CompletionItem::default()
                    })
                    .collect();
                serde_json::to_value(CompletionResponse::Array(items))
            }
            _ => {
                return self.send(Response::new_err(
                    id,
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    format!("not supported: {}", req.method),
                ));
            }
        };
        self.send(Response::new_ok(id, result.map_err(err)?))
    }
}

fn from<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(err)
}

fn range(ix: &LineIndex<'_>, start: u32, end: u32) -> Range {
    let (l0, c0) = ix.position(start);
    let (l1, c1) = ix.position(end);
    Range {
        start: Position {
            line: l0,
            character: c0,
        },
        end: Position {
            line: l1,
            character: c1,
        },
    }
}

fn kind(k: CompletionKind) -> CompletionItemKind {
    match k {
        CompletionKind::Field => CompletionItemKind::FIELD,
        CompletionKind::Method => CompletionItemKind::METHOD,
        CompletionKind::Function => CompletionItemKind::FUNCTION,
        CompletionKind::Type => CompletionItemKind::STRUCT,
        CompletionKind::Variant => CompletionItemKind::ENUM_MEMBER,
        CompletionKind::Trait => CompletionItemKind::INTERFACE,
        CompletionKind::Module => CompletionItemKind::MODULE,
        CompletionKind::Local => CompletionItemKind::VARIABLE,
    }
}
