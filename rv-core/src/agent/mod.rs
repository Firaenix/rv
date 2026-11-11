//! A configured agent CLI (`maki -p`, `claude --print`) run as a subprocess:
//! rv writes a prompt to its stdin and reads the reply from its stdout. No
//! model call, key or URL lives here — only the command the user chose.

pub mod contract;
mod envelope;
mod handle;
mod process;
pub mod prompt;
pub mod sanitize;
pub mod session;

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub use contract::Answer;
pub use contract::Focus;
pub use contract::Importance;
pub use contract::parse_answer;
pub use handle::AskHandle;
pub use handle::SingleFlight;

pub const DEFAULT_MODEL_FLAG: &str = "--model";
/// maki takes Claude Code's camel-case spelling as an alias.
pub const DEFAULT_ALLOWED_TOOLS_FLAG: &str = "--allowedTools";
pub const DEFAULT_RESUME_FLAG: &str = "-r";
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_ERROR_CHARS: usize = 2_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// The reviewer never asks by itself; only `rv brief --ask` reaches it.
    #[default]
    Off,
    /// Orient each file the first time it is opened, plus on-demand asks.
    Files,
    /// Ask only when the reviewer presses the key.
    OnDemand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    pub command: Vec<String>,
    pub model: Option<String>,
    pub model_flag: String,
    pub allowed_tools: Option<String>,
    pub allowed_tools_flag: String,
    pub resume_flag: String,
    pub timeout_secs: u64,
    pub orientation: Orientation,
}

impl AgentConfig {
    #[must_use]
    pub fn new(command: Vec<String>) -> Self {
        Self {
            command,
            model: None,
            model_flag: DEFAULT_MODEL_FLAG.to_owned(),
            allowed_tools: None,
            allowed_tools_flag: DEFAULT_ALLOWED_TOOLS_FLAG.to_owned(),
            resume_flag: DEFAULT_RESUME_FLAG.to_owned(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            orientation: Orientation::OnDemand,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentError {
    #[error("agent unavailable: {0}")]
    Unavailable(String),
    #[error("{0}")]
    Spawn(String),
    #[error("the agent gave no answer within {0}s and was stopped")]
    TimedOut(u64),
    #[error("the ask was cancelled")]
    Cancelled,
    #[error("the agent reported an error: {0}")]
    Failed(String),
    #[error("could not record the agent session: {0}")]
    Store(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub answer: Answer,
    pub session_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Agent {
    config: AgentConfig,
    workdir: PathBuf,
}

impl Agent {
    /// `workdir` is where the agent runs, so its own read tools see the repo.
    #[must_use]
    pub fn new(config: AgentConfig, workdir: &Path) -> Self {
        Self {
            config,
            workdir: workdir.to_owned(),
        }
    }

    #[must_use]
    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// The program's file name, for a status bar: `maki`, `claude`.
    #[must_use]
    pub fn name(&self) -> String {
        self.config
            .command
            .first()
            .map(|program| {
                Path::new(program).file_name().map_or_else(
                    || program.clone(),
                    |name| name.to_string_lossy().into_owned(),
                )
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.config
            .command
            .first()
            .is_some_and(|program| crate::lsp::find_program(program).is_some())
    }

    /// The full argv for one call: the configured command, then the model,
    /// tool and resume flags where each is set.
    #[must_use]
    pub fn command_line(&self, resume: Option<&str>) -> Vec<String> {
        let config = &self.config;
        let mut argv = config.command.clone();
        let mut push = |flag: &str, value: Option<&str>| {
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                argv.push(flag.to_owned());
                argv.push(value.to_owned());
            }
        };
        push(&config.model_flag, config.model.as_deref());
        push(&config.allowed_tools_flag, config.allowed_tools.as_deref());
        push(
            &config.resume_flag,
            resume.filter(|id| envelope::is_safe_session_id(id)),
        );
        argv
    }

    pub fn ask(&self, prompt: &str, resume: Option<&str>) -> Result<Reply, AgentError> {
        self.ask_cancellable(prompt, resume, &AtomicBool::new(false))
    }

    /// Resumes `resume` and, where that run fails, asks once more in a fresh
    /// session — a stored id the agent has forgotten must not wedge every
    /// later ask.
    pub fn ask_resuming(&self, prompt: &str, resume: Option<&str>) -> Result<Reply, AgentError> {
        resuming(resume, |resume| self.ask(prompt, resume))
    }

    pub(crate) fn ask_cancellable(
        &self,
        prompt: &str,
        resume: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Reply, AgentError> {
        let timeout = Duration::from_secs(self.config.timeout_secs.max(1));
        let finished = process::run(
            &self.command_line(resume),
            &self.workdir,
            prompt,
            timeout,
            cancelled,
        )?;
        let wrapped = envelope::read(&finished.stdout);
        match wrapped {
            Some(envelope) if envelope.is_error => {
                let said = if envelope.result.trim().is_empty() {
                    &finished.stderr
                } else {
                    &envelope.result
                };
                Err(failure(said, finished.status))
            }
            Some(envelope) => Ok(Reply {
                answer: parse_answer(&envelope.result),
                session_id: envelope.session_id,
                duration_ms: envelope.duration_ms,
                cost_usd: envelope.cost_usd,
            }),
            None if !finished.status.success() || finished.stdout.trim().is_empty() => {
                let said = if finished.stderr.trim().is_empty() {
                    &finished.stdout
                } else {
                    &finished.stderr
                };
                Err(failure(said, finished.status))
            }
            None => Ok(Reply {
                answer: parse_answer(&finished.stdout),
                session_id: None,
                duration_ms: None,
                cost_usd: None,
            }),
        }
    }
}

pub(crate) fn resuming(
    resume: Option<&str>,
    mut ask: impl FnMut(Option<&str>) -> Result<Reply, AgentError>,
) -> Result<Reply, AgentError> {
    match ask(resume) {
        Err(AgentError::Failed(_)) if resume.is_some() => ask(None),
        other => other,
    }
}

fn failure(said: &str, status: std::process::ExitStatus) -> AgentError {
    let said = sanitize::text(said, MAX_ERROR_CHARS);
    if said.is_empty() {
        AgentError::Failed(format!("exited with {status} and said nothing"))
    } else {
        AgentError::Failed(said)
    }
}
