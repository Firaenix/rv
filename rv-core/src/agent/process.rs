//! One agent run: prompt on stdin, both output pipes drained on their own
//! threads so neither can fill and stall the child, killed on timeout or
//! cancel.

use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use super::AgentError;

const POLL: Duration = Duration::from_millis(20);
/// How long to wait for the pipes once the child is gone. A grandchild that
/// inherited stdout can hold it open past the child's death.
const DRAIN_GRACE: Duration = Duration::from_secs(2);
const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;

pub(super) struct Finished {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

pub(super) fn run(
    command: &[String],
    workdir: &Path,
    prompt: &str,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<Finished, AgentError> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| AgentError::Unavailable("no agent command configured".to_owned()))?;
    if crate::lsp::find_program(program).is_none() {
        return Err(AgentError::Unavailable(format!("{program} is not on PATH")));
    }
    let mut child = Command::new(program)
        .args(args)
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                AgentError::Unavailable(format!("could not run {program}: {error}"))
            }
            _ => AgentError::Spawn(format!("could not run {program}: {error}")),
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        let prompt = prompt.as_bytes().to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&prompt);
        });
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let status = wait(&mut child, timeout, cancelled)?;
    let collect = |receiver: mpsc::Receiver<Vec<u8>>| {
        let bytes = receiver.recv_timeout(DRAIN_GRACE).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(Finished {
        status,
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(pipe) = pipe {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
    }
    receiver
}

fn wait(
    child: &mut Child,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<ExitStatus, AgentError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                kill(child);
                return Err(AgentError::Spawn(format!(
                    "could not wait on the agent: {error}"
                )));
            }
        }
        if cancelled.load(Ordering::Relaxed) {
            kill(child);
            return Err(AgentError::Cancelled);
        }
        if Instant::now() >= deadline {
            kill(child);
            return Err(AgentError::TimedOut(timeout.as_secs()));
        }
        std::thread::sleep(POLL);
    }
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
