//! One agent conversation per review: the agent's session id is kept in the
//! review's `session.toml`, so orientation and follow-ups resume it rather
//! than starting cold each time.

use super::Agent;
use super::AgentError;
use super::Reply;
use crate::store::Store;

/// The stored id to resume, if any. An unreadable store resumes nothing.
#[must_use]
pub fn resume_id(store: &Store) -> Option<String> {
    store.agent_session().ok().flatten()
}

/// Keeps the reply's session id for the next ask; a reply without one leaves
/// the stored id as it was.
pub fn remember(store: &Store, reply: &Reply) -> Result<(), AgentError> {
    let Some(id) = reply.session_id.as_deref() else {
        return Ok(());
    };
    if resume_id(store).as_deref() == Some(id) {
        return Ok(());
    }
    store
        .set_agent_session(Some(id))
        .map_err(|error| AgentError::Store(error.to_string()))
}

/// Blocking: resume the review's conversation, ask, and record where it got
/// to. For the CLI; the TUI pairs [`resume_id`], [`Agent::spawn_ask`] and
/// [`remember`] instead.
pub fn ask_in_review(agent: &Agent, store: &Store, prompt: &str) -> Result<Reply, AgentError> {
    let reply = agent.ask_resuming(prompt, resume_id(store).as_deref())?;
    remember(store, &reply)?;
    Ok(reply)
}
