use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::driver::{Driver, Event, PositionRequest, SharedStatus, read_status};
use super::framing::read_frame;
use super::servers::{Language, ServerRegistry, find_program, language_for_path, server_name};
use super::{Answer, LspError, RequestKind, ServerStatus};

const EXIT_GRACE: Duration = Duration::from_secs(1);

/// The eventual answer to one request. Poll it with [`Ticket::try_take`]; it
/// never blocks unless asked to with [`Ticket::wait`].
pub struct Ticket {
    receiver: Option<Receiver<Result<Answer, LspError>>>,
}

impl Ticket {
    fn resolved(result: Result<Answer, LspError>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let _ = sender.send(result);
        Self {
            receiver: Some(receiver),
        }
    }

    /// `None` while the answer is outstanding, and again after it was taken.
    pub fn try_take(&mut self) -> Option<Result<Answer, LspError>> {
        let outcome = match self.receiver.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(went_away()),
        };
        self.receiver = None;
        Some(outcome)
    }

    pub fn wait(&mut self, timeout: Duration) -> Option<Result<Answer, LspError>> {
        let outcome = match self.receiver.as_ref()?.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => return None,
            Err(RecvTimeoutError::Disconnected) => Err(went_away()),
        };
        self.receiver = None;
        Some(outcome)
    }
}

fn went_away() -> LspError {
    LspError::Unavailable("the language server went away".into())
}

struct LanguageServer {
    child: Child,
    events: Sender<Event>,
    status: SharedStatus,
}

impl LanguageServer {
    fn spawn(command: &[String], root: &Path, language: Language) -> Result<Self, String> {
        let name = server_name(command);
        let (program, arguments) = command.split_first().ok_or("empty server command")?;
        let program = find_program(program).ok_or_else(|| format!("{name} not found on PATH"))?;
        let mut child = Command::new(program)
            .args(arguments)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("starting {name}: {error}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            kill_and_reap(&mut child);
            return Err(format!("{name} has no stdio pipes"));
        };
        let (events, inbox) = mpsc::channel();
        let status = Arc::new(Mutex::new(ServerStatus::Starting));
        let driver = Driver::new(
            BufWriter::new(stdin),
            inbox,
            Arc::clone(&status),
            root.to_path_buf(),
            language.language_id,
        );
        let reader_events = events.clone();
        let threads = std::thread::Builder::new()
            .name(format!("lsp-reader-{name}"))
            .spawn(move || forward_frames(stdout, &reader_events))
            .and_then(|_| {
                std::thread::Builder::new()
                    .name(format!("lsp-driver-{name}"))
                    .spawn(move || driver.run())
            });
        if let Err(error) = threads {
            kill_and_reap(&mut child);
            return Err(format!("starting {name} threads: {error}"));
        }
        Ok(Self {
            child,
            events,
            status,
        })
    }
}

fn forward_frames(stdout: ChildStdout, events: &Sender<Event>) {
    let mut reader = BufReader::new(stdout);
    while let Ok(Some(body)) = read_frame(&mut reader) {
        if events.send(Event::Incoming(body)).is_err() {
            return;
        }
    }
    let _ = events.send(Event::ReaderClosed);
}

fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for LanguageServer {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Shutdown);
        let deadline = Instant::now() + EXIT_GRACE;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        kill_and_reap(&mut self.child);
    }
}

enum Slot {
    Running(LanguageServer),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSummary {
    pub language: &'static str,
    pub server: String,
    pub status: ServerStatus,
}

/// One server process per language, spawned on first use. A server that fails
/// to start or dies stays [`ServerStatus::Unavailable`] for the hub's lifetime
/// rather than being respawned into a crash loop.
pub struct LspHub {
    root: PathBuf,
    registry: ServerRegistry,
    slots: HashMap<&'static str, Slot>,
}

impl LspHub {
    /// The root is canonicalised, because servers answer with canonical paths
    /// (`/private/var` rather than `/var` on macOS) and those must still be
    /// recognised as inside the repo.
    pub fn new(repo_root: &Path, registry: ServerRegistry) -> Self {
        let root = repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf());
        Self {
            root,
            registry,
            slots: HashMap::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `path` is repo-relative (or absolute); `line` is 1-based and `column`
    /// a 0-based character offset. Never blocks on the server.
    pub fn request(&mut self, kind: RequestKind, path: &Path, line: u32, column: u32) -> Ticket {
        let absolute_path = self.root.join(path);
        let server = match self.ensure_started(path) {
            Ok(server) => server,
            Err(reason) => return Ticket::resolved(Err(LspError::Unavailable(reason))),
        };
        let (reply, receiver) = mpsc::channel();
        let request = PositionRequest {
            kind,
            absolute_path,
            line,
            column,
            reply,
        };
        if server.events.send(Event::Request(request)).is_err() {
            return Ticket::resolved(Err(went_away()));
        }
        Ticket {
            receiver: Some(receiver),
        }
    }

    /// Spawns the server for `path`'s language ahead of the first request, so
    /// it can index while the reviewer reads. `None` if no server applies.
    pub fn start_for(&mut self, path: &Path) -> Option<ServerStatus> {
        let _ = self.ensure_started(path);
        self.status_for(path)
    }

    /// `None` when no server is configured for `path`'s language.
    pub fn status_for(&self, path: &Path) -> Option<ServerStatus> {
        let language = language_for_path(path)?;
        self.registry.command_for(language.name)?;
        Some(match self.slots.get(language.name) {
            None => ServerStatus::Idle,
            Some(Slot::Failed(reason)) => ServerStatus::Unavailable(reason.clone()),
            Some(Slot::Running(server)) => read_status(&server.status),
        })
    }

    /// Every server started (or attempted) so far, for a status bar.
    pub fn servers(&self) -> Vec<ServerSummary> {
        let mut summaries: Vec<ServerSummary> = self
            .slots
            .iter()
            .map(|(&language, slot)| ServerSummary {
                language,
                server: self
                    .registry
                    .command_for(language)
                    .map(server_name)
                    .unwrap_or_default(),
                status: match slot {
                    Slot::Failed(reason) => ServerStatus::Unavailable(reason.clone()),
                    Slot::Running(server) => read_status(&server.status),
                },
            })
            .collect();
        summaries.sort_by_key(|summary| summary.language);
        summaries
    }

    fn ensure_started(&mut self, path: &Path) -> Result<&LanguageServer, String> {
        let language = language_for_path(path)
            .ok_or_else(|| format!("no language server for {}", path.display()))?;
        let command = self
            .registry
            .command_for(language.name)
            .ok_or_else(|| format!("no language server configured for {}", language.name))?;
        let slot = self.slots.entry(language.name).or_insert_with(|| {
            match LanguageServer::spawn(command, &self.root, language) {
                Ok(server) => Slot::Running(server),
                Err(reason) => Slot::Failed(reason),
            }
        });
        match slot {
            Slot::Running(server) => Ok(server),
            Slot::Failed(reason) => Err(reason.clone()),
        }
    }
}
