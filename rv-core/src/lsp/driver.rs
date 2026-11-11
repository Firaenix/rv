use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::answers;
use super::framing::write_frame;
use super::protocol::{self, Incoming, ProgressUpdate};
use super::uri::path_to_uri;
use super::{Answer, LspError, RequestKind, ServerStatus};

const SHUTDOWN_REPLY_WAIT: Duration = Duration::from_millis(500);

pub(super) type Reply = Sender<Result<Answer, LspError>>;

pub(super) struct PositionRequest {
    pub kind: RequestKind,
    pub absolute_path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub reply: Reply,
}

pub(super) enum Event {
    Request(PositionRequest),
    Incoming(Vec<u8>),
    ReaderClosed,
    Shutdown,
}

pub(super) type SharedStatus = Arc<Mutex<ServerStatus>>;

pub(super) fn read_status(status: &SharedStatus) -> ServerStatus {
    status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

struct ActiveProgress {
    token: String,
    title: String,
    percentage: Option<u32>,
}

struct OpenDocument {
    version: i32,
    text: String,
}

/// Owns the server's stdin and every piece of protocol state. Runs on its own
/// thread so that neither a slow server nor a full pipe can stall the caller.
pub(super) struct Driver<W: Write> {
    writer: W,
    events: Receiver<Event>,
    status: SharedStatus,
    root: PathBuf,
    language_id: &'static str,
    next_id: i64,
    awaiting_initialize: Option<i64>,
    queued: Vec<PositionRequest>,
    pending: HashMap<i64, (RequestKind, Reply)>,
    progress: Vec<ActiveProgress>,
    documents: HashMap<PathBuf, OpenDocument>,
    failure: Option<String>,
}

impl<W: Write> Driver<W> {
    pub fn new(
        writer: W,
        events: Receiver<Event>,
        status: SharedStatus,
        root: PathBuf,
        language_id: &'static str,
    ) -> Self {
        Self {
            writer,
            events,
            status,
            root,
            language_id,
            next_id: 1,
            awaiting_initialize: None,
            queued: Vec::new(),
            pending: HashMap::new(),
            progress: Vec::new(),
            documents: HashMap::new(),
            failure: None,
        }
    }

    pub fn run(mut self) {
        self.send_initialize();
        while let Ok(event) = self.events.recv() {
            match event {
                Event::Request(request) => self.on_request(request),
                Event::Incoming(body) => self.on_incoming(&body),
                Event::ReaderClosed => self.fail("the language server exited"),
                Event::Shutdown => return self.shut_down(),
            }
        }
    }

    fn send_initialize(&mut self) {
        let root_uri = path_to_uri(&self.root);
        let root_name = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let params = protocol::initialize_params(&root_uri, root_name.as_deref().unwrap_or("root"));
        let id = self.allocate_id();
        self.awaiting_initialize = Some(id);
        self.publish_status();
        self.send(&protocol::request(id, "initialize", params));
    }

    fn allocate_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send(&mut self, body: &[u8]) {
        if self.failure.is_some() {
            return;
        }
        if let Err(error) = write_frame(&mut self.writer, body) {
            self.fail(&format!("writing to the language server: {error}"));
        }
    }

    fn fail(&mut self, reason: &str) {
        if self.failure.is_none() {
            self.failure = Some(reason.to_owned());
        }
        let error = LspError::Unavailable(reason.to_owned());
        for request in self.queued.drain(..) {
            let _ = request.reply.send(Err(error.clone()));
        }
        for (_, (_, reply)) in self.pending.drain() {
            let _ = reply.send(Err(error.clone()));
        }
        self.publish_status();
    }

    fn publish_status(&self) {
        let status = if let Some(reason) = &self.failure {
            ServerStatus::Unavailable(reason.clone())
        } else if let Some(latest) = self.progress.last() {
            ServerStatus::Indexing {
                title: latest.title.clone(),
                percentage: latest.percentage,
            }
        } else if self.awaiting_initialize.is_some() {
            ServerStatus::Starting
        } else {
            ServerStatus::Ready
        };
        *self
            .status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = status;
    }

    fn on_request(&mut self, request: PositionRequest) {
        if let Some(reason) = &self.failure {
            let _ = request
                .reply
                .send(Err(LspError::Unavailable(reason.clone())));
        } else if self.awaiting_initialize.is_some() {
            self.queued.push(request);
        } else {
            self.dispatch(request);
        }
    }

    fn dispatch(&mut self, request: PositionRequest) {
        let text = match std::fs::read_to_string(&request.absolute_path) {
            Ok(text) => text,
            Err(error) => {
                let _ = request.reply.send(Err(LspError::Unreadable {
                    path: request.absolute_path,
                    reason: error.to_string(),
                }));
                return;
            }
        };
        let uri = path_to_uri(&request.absolute_path);
        self.sync_document(&request.absolute_path, &uri, text.clone());
        let line = request.line.saturating_sub(1);
        let line_text = protocol::line_of(&text, line).unwrap_or_default();
        let character = protocol::char_column_to_utf16(line_text, request.column);
        let (method, params) = match request.kind {
            RequestKind::Definition => (
                "textDocument/definition",
                protocol::position_params(&uri, line, character),
            ),
            RequestKind::References => (
                "textDocument/references",
                protocol::references_params(&uri, line, character),
            ),
            RequestKind::Hover => (
                "textDocument/hover",
                protocol::position_params(&uri, line, character),
            ),
        };
        let id = self.allocate_id();
        self.pending.insert(id, (request.kind, request.reply));
        self.send(&protocol::request(id, method, params));
    }

    /// Re-sends the whole file whenever it changed on disk since the server
    /// last saw it, so an opened document never goes stale.
    fn sync_document(&mut self, path: &Path, uri: &str, text: String) {
        let notification = match self.documents.get_mut(path) {
            None => {
                let params = protocol::did_open_params(uri, self.language_id, 1, &text);
                self.documents
                    .insert(path.to_path_buf(), OpenDocument { version: 1, text });
                protocol::notification("textDocument/didOpen", params)
            }
            Some(document) if document.text != text => {
                document.version += 1;
                let params = protocol::did_change_params(uri, document.version, &text);
                document.text = text;
                protocol::notification("textDocument/didChange", params)
            }
            Some(_) => return,
        };
        self.send(&notification);
    }

    fn on_incoming(&mut self, body: &[u8]) {
        match protocol::parse_incoming(body) {
            Some(Incoming::Response { id, outcome }) => self.on_response(&id, outcome),
            Some(Incoming::Request { id, method, params }) => {
                self.answer_server_request(&id, &method, &params)
            }
            Some(Incoming::Notification { method, params }) if method == "$/progress" => {
                self.on_progress(&params)
            }
            _ => {}
        }
    }

    fn on_response(&mut self, id: &Value, outcome: Result<Value, protocol::ResponseError>) {
        let Some(id) = id.as_i64() else { return };
        if self.awaiting_initialize == Some(id) {
            return self.on_initialized(outcome);
        }
        let Some((kind, reply)) = self.pending.remove(&id) else {
            return;
        };
        let answer = match outcome {
            Ok(result) => answers::convert(kind, result, &self.root),
            Err(error) => Err(LspError::Server {
                code: error.code,
                message: error.message,
            }),
        };
        let _ = reply.send(answer);
    }

    fn on_initialized(&mut self, outcome: Result<Value, protocol::ResponseError>) {
        self.awaiting_initialize = None;
        if let Err(error) = outcome {
            return self.fail(&format!("initialize failed: {}", error.message));
        }
        self.send(&protocol::notification(
            "initialized",
            serde_json::json!({}),
        ));
        self.publish_status();
        for request in std::mem::take(&mut self.queued) {
            self.on_request(request);
        }
    }

    /// Every server-initiated request is answered: rust-analyzer will not
    /// report progress on a token whose `workDoneProgress/create` went unanswered.
    fn answer_server_request(&mut self, id: &Value, method: &str, params: &Value) {
        let body = match method {
            "workspace/configuration" => {
                let items = params
                    .get("items")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                protocol::response(id, Value::Array(vec![Value::Null; items]))
            }
            "window/workDoneProgress/create"
            | "client/registerCapability"
            | "client/unregisterCapability"
            | "window/showMessageRequest"
            | "workspace/workspaceFolders" => protocol::response(id, Value::Null),
            _ => protocol::error_response(id, protocol::METHOD_NOT_FOUND, method),
        };
        self.send(&body);
    }

    fn on_progress(&mut self, params: &Value) {
        let Some((token, update)) = protocol::parse_progress(params) else {
            return;
        };
        match update {
            ProgressUpdate::Begin { title, percentage } => {
                self.progress.retain(|active| active.token != token);
                self.progress.push(ActiveProgress {
                    token,
                    title,
                    percentage,
                });
            }
            ProgressUpdate::Report { percentage } => {
                if let Some(active) = self
                    .progress
                    .iter_mut()
                    .find(|active| active.token == token)
                {
                    active.percentage = percentage.or(active.percentage);
                }
            }
            ProgressUpdate::End => self.progress.retain(|active| active.token != token),
        }
        self.publish_status();
    }

    fn shut_down(mut self) {
        if self.failure.is_some() {
            return;
        }
        if self.awaiting_initialize.is_none() {
            let id = self.allocate_id();
            self.send(&protocol::request(id, "shutdown", Value::Null));
            if !self.await_reply(id) {
                return;
            }
        }
        self.send(&protocol::notification("exit", Value::Null));
    }

    /// `false` when the server went away while waiting.
    fn await_reply(&mut self, id: i64) -> bool {
        let deadline = Instant::now() + SHUTDOWN_REPLY_WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(remaining) {
                Ok(Event::Incoming(body)) => {
                    if let Some(Incoming::Response { id: reply_id, .. }) =
                        protocol::parse_incoming(&body)
                        && reply_id.as_i64() == Some(id)
                    {
                        return true;
                    }
                }
                Ok(Event::Request(request)) => {
                    let _ = request
                        .reply
                        .send(Err(LspError::Unavailable("shutting down".into())));
                }
                Ok(Event::Shutdown) => {}
                Ok(Event::ReaderClosed) | Err(RecvTimeoutError::Disconnected) => return false,
                Err(RecvTimeoutError::Timeout) => return true,
            }
        }
    }
}
