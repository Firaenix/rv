//! `rv brief --ask`: the brief piped through the configured agent CLI. The
//! one CLI path that writes for an agent — its focus becomes flags by
//! `agent`, and its session id is kept for the next ask.

use anyhow::Result;
use anyhow::bail;
use rv::config;
use rv::session::Review;
use rv::session::flags::save_agent_focus;
use rv_core::agent::Agent;
use rv_core::agent::AgentError;
use rv_core::agent::Reply;
use rv_core::agent::prompt;
use rv_core::agent::session as agent_session;
use rv_core::markdown::brief;

pub fn ask_about_brief(review: &Review, context: u32, no_agent: bool) -> Result<()> {
    if no_agent {
        bail!("--no-agent is set, so there is no agent to ask");
    }
    let Some(agent_config) = config::load_settings()?.agent_config() else {
        let path = config::settings_path().map_or_else(
            || "Config.toml".to_owned(),
            |path| path.display().to_string(),
        );
        bail!(
            "no agent is configured — add an [agent] section with a command to {path} \
             (see the README's \"Agent\" section)"
        );
    };
    let agent = Agent::new(agent_config, review.store.root());
    let document = brief::render(&rv::session::brief::assemble(review, context)?);
    eprintln!("asking {}…", agent.name());
    let reply = agent_session::ask_in_review(&agent, &review.store, &prompt::brief(&document))
        .map_err(|error| explained(&agent, error))?;

    print_reply(&agent, &reply);
    let filed = save_agent_focus(review, &reply.answer.focus)?;
    for flag in &filed.saved {
        println!(
            "flagged {} at {}:{} — {}",
            flag.id, flag.anchor.file, flag.anchor.line, flag.reason
        );
    }
    if filed.already_flagged > 0 {
        println!("{} already flagged", filed.already_flagged);
    }
    for (focus, why) in &filed.skipped {
        eprintln!("skipped {}:{}: {why}", focus.path, focus.line);
    }
    Ok(())
}

fn print_reply(agent: &Agent, reply: &Reply) {
    let answer = &reply.answer;
    if answer.text.is_empty() {
        println!("{}", answer.summary);
    } else {
        println!("{}", answer.text);
    }
    let mut footer = vec![format!("agent: {}", agent.name())];
    if let Some(duration) = reply.duration_ms {
        footer.push(format!("{:.1}s", duration as f64 / 1000.0));
    }
    if let Some(cost) = reply.cost_usd {
        footer.push(format!("${cost:.4}"));
    }
    println!("\n{}", footer.join(" · "));
}

fn explained(agent: &Agent, error: AgentError) -> anyhow::Error {
    match error {
        AgentError::Unavailable(why) => anyhow::anyhow!(
            "the configured agent `{}` could not be run: {why}",
            agent.name()
        ),
        other => anyhow::Error::new(other),
    }
}
