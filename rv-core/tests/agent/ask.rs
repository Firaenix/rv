use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use rv_core::agent::Agent;
use rv_core::agent::AgentConfig;
use rv_core::agent::AgentError;
use rv_core::agent::Importance;
use rv_core::agent::SingleFlight;

use super::stub;

fn agent(mode: &str) -> Agent {
    Agent::new(AgentConfig::new(stub::command(mode, None)), Path::new("."))
}

fn wait_for<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "nothing arrived in time");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_claude_shaped_reply_yields_the_answer_focus_and_metadata() {
    let reply = agent("contract").ask("orient me", None).expect("a reply");
    assert_eq!(reply.session_id.as_deref(), Some("sess-42"));
    assert_eq!(reply.duration_ms, Some(4123));
    assert_eq!(reply.cost_usd, Some(0.0125));
    assert!(reply.answer.structured);
    assert_eq!(reply.answer.summary, "Parses replies.");
    assert_eq!(reply.answer.text, "This file parses replies.");
    let focus: Vec<_> = reply
        .answer
        .focus
        .iter()
        .map(|entry| (entry.path.as_str(), entry.line, entry.importance))
        .collect();
    assert_eq!(
        focus,
        [
            ("src/b.rs", 9, Importance::High),
            ("src/a.rs", 3, Importance::Low)
        ]
    );
}

#[test]
fn the_prompt_goes_on_stdin_and_configured_flags_are_appended() {
    let mut config = AgentConfig::new(stub::command("echo", None));
    config.model = Some("opus".to_owned());
    config.allowed_tools = Some("Read".to_owned());
    let reply = Agent::new(config, Path::new("."))
        .ask("hello agent", Some("sess-1"))
        .expect("a reply");
    assert_eq!(
        reply.answer.text,
        "args=[--model opus --allowedTools Read -r sess-1] prompt=hello agent"
    );
    assert!(!reply.answer.structured);
    assert!(reply.answer.focus.is_empty());
}

#[test]
fn flags_are_overridable_and_an_unsafe_resume_id_is_never_passed() {
    let mut config = AgentConfig::new(stub::command("echo", None));
    config.model = Some("m".to_owned());
    config.model_flag = "-m".to_owned();
    config.resume_flag = "--session".to_owned();
    let agent = Agent::new(config, Path::new("."));
    let reply = agent.ask("p", Some("--yolo")).expect("a reply");
    assert_eq!(reply.answer.text, "args=[-m m] prompt=p");
    let reply = agent.ask("p", Some("abc")).expect("a reply");
    assert_eq!(reply.answer.text, "args=[-m m --session abc] prompt=p");
}

#[test]
fn plain_stdout_is_the_answer_when_there_is_no_json_envelope() {
    let reply = agent("raw").ask("p", None).expect("a reply");
    assert_eq!(reply.answer.text, "Just prose, no JSON.\nSecond line.");
    assert_eq!(reply.answer.summary, "Just prose, no JSON.");
    assert!(reply.answer.focus.is_empty());
    assert_eq!(reply.session_id, None);
    assert_eq!(reply.cost_usd, None);
}

#[test]
fn malformed_focus_entries_are_dropped_one_by_one() {
    let reply = agent("malformed").ask("p", None).expect("a reply");
    let focus: Vec<_> = reply
        .answer
        .focus
        .iter()
        .map(|entry| (entry.line, entry.why.as_str(), entry.importance))
        .collect();
    assert_eq!(
        focus,
        [
            (7, "string line", Importance::High),
            (2, "fine", Importance::Medium)
        ]
    );
}

#[test]
fn a_broken_block_leaves_the_prose_usable() {
    let reply = agent("broken_block").ask("p", None).expect("a reply");
    assert!(!reply.answer.structured);
    assert!(reply.answer.focus.is_empty());
    assert_eq!(reply.answer.summary, "Prose survives.");
}

#[test]
fn terminal_escapes_never_survive_into_text_summary_or_focus() {
    let reply = agent("ansi").ask("p", None).expect("a reply");
    assert_eq!(reply.answer.text, "red text");
    assert_eq!(reply.answer.summary, "bold");
    assert_eq!(reply.answer.focus[0].why, "clear screen");
}

#[test]
fn an_error_envelope_or_a_failing_exit_is_a_failure_with_its_message() {
    assert_eq!(
        agent("error").ask("p", None),
        Err(AgentError::Failed("model overloaded".to_owned()))
    );
    match agent("exit_nonzero").ask("p", None) {
        Err(AgentError::Failed(message)) => {
            assert!(message.contains("boom on stderr"), "{message}")
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn a_forgotten_session_is_retried_fresh() {
    let reply = agent("fail_on_resume")
        .ask_resuming("p", Some("stale"))
        .expect("the fresh retry answers");
    assert_eq!(reply.answer.text, "fresh args=[]");
}

#[test]
fn an_absent_binary_is_unavailable_not_a_panic() {
    let config = AgentConfig::new(vec!["rv-no-such-agent-binary".to_owned()]);
    let agent = Agent::new(config, Path::new("."));
    assert!(!agent.is_available());
    assert!(matches!(
        agent.ask("p", None),
        Err(AgentError::Unavailable(_))
    ));
    let empty = Agent::new(AgentConfig::new(Vec::new()), Path::new("."));
    assert!(matches!(
        empty.ask("p", None),
        Err(AgentError::Unavailable(_))
    ));
    assert!(matches!(
        empty.spawn_ask("p".to_owned(), None).wait(),
        Err(AgentError::Unavailable(_))
    ));
}

fn is_alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn a_timeout_kills_the_agent() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pid_file = directory.path().join("pid");
    let mut config = AgentConfig::new(stub::command("sleep", Some(&pid_file)));
    config.timeout_secs = 1;
    let started = Instant::now();
    let outcome = Agent::new(config, Path::new(".")).ask("p", None);
    assert_eq!(outcome, Err(AgentError::TimedOut(1)));
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid = std::fs::read_to_string(&pid_file).expect("the stub wrote its pid");
    assert!(!is_alive(&pid), "the stub is still running");
}

#[test]
fn cancel_stops_a_running_ask() {
    let directory = tempfile::tempdir().expect("temp dir");
    let pid_file = directory.path().join("pid");
    let agent = Agent::new(
        AgentConfig::new(stub::command("sleep", Some(&pid_file))),
        Path::new("."),
    );
    let mut handle = agent.spawn_ask("p".to_owned(), None);
    let pid = wait_for(|| {
        std::fs::read_to_string(&pid_file)
            .ok()
            .filter(|pid| !pid.is_empty())
    });
    assert!(handle.try_take().is_none(), "still running");
    handle.cancel();
    assert_eq!(wait_for(|| handle.try_take()), Err(AgentError::Cancelled));
    assert!(handle.try_take().is_none(), "taken once");
    wait_for(|| (!is_alive(&pid)).then_some(()));
}

#[test]
fn single_flight_refuses_a_second_ask_and_hands_back_the_outcome_with_its_tag() {
    let agent = agent("contract");
    let mut flight = SingleFlight::default();
    flight
        .start(&agent, "src/a.rs", "p".to_owned(), None)
        .expect("idle");
    assert_eq!(
        flight.start(&agent, "src/b.rs", "p".to_owned(), None),
        Err("src/b.rs")
    );
    assert_eq!(flight.current(), Some(&"src/a.rs"));
    let (tag, outcome) = wait_for(|| flight.poll());
    assert_eq!(tag, "src/a.rs");
    assert!(outcome.is_ok());
    assert!(!flight.is_busy());

    let sleeper = self::agent("sleep");
    flight
        .start(&sleeper, "slow", "p".to_owned(), None)
        .expect("idle again");
    assert_eq!(flight.cancel(), Some("slow"));
    assert!(!flight.is_busy());
}
