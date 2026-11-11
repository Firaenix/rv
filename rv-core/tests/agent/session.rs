use std::path::Path;

use rv_core::agent::Agent;
use rv_core::agent::AgentConfig;
use rv_core::agent::session;
use rv_core::model::Side;
use rv_core::store::Flag;
use rv_core::store::FlaggedBy;
use rv_core::store::Store;

use super::stub;

fn store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = Store::open(directory.path(), "trunk").expect("open the store");
    (directory, store)
}

#[test]
fn a_session_toml_from_before_agents_loads_with_no_agent_session() {
    let (directory, store) = store();
    store.set_agent_session(Some("sess-1")).expect("write");
    store
        .append_flag(&Flag {
            id: "f".to_owned(),
            change_id: "c".to_owned(),
            commit_id: "b".to_owned(),
            anchor: rv_core::anchor::create("a.rs", Side::Right, 1, "x\n"),
            reason: "look".to_owned(),
            acknowledged: false,
            by: FlaggedBy::Agent,
        })
        .expect("flag");
    let path = directory.path().join(".review/reviews/trunk/session.toml");
    let written = std::fs::read_to_string(&path).expect("read");
    let legacy: String = written
        .lines()
        .filter(|line| !line.starts_with("by = ") && !line.starts_with("agent_session = "))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_ne!(legacy, written, "both new fields were written");
    std::fs::write(&path, legacy).expect("write a legacy session");
    let review = store.read_review().expect("the old file still loads");
    assert_eq!(review.agent_session, None);
    assert_eq!(review.flags[0].by, FlaggedBy::User);
}

#[test]
fn the_agent_session_round_trips_and_leaves_the_rest_alone() {
    let (_directory, store) = store();
    assert_eq!(session::resume_id(&store), None);
    store.set_agent_session(Some("sess-1")).expect("write");
    assert_eq!(
        store.agent_session().expect("read"),
        Some("sess-1".to_owned())
    );
    store.set_agent_session(None).expect("clear");
    assert_eq!(session::resume_id(&store), None);
}

#[test]
fn asking_in_a_review_resumes_and_records_the_session() {
    let (_directory, store) = store();
    let agent = Agent::new(
        AgentConfig::new(stub::command("echo", None)),
        Path::new("."),
    );
    let first = session::ask_in_review(&agent, &store, "one").expect("a reply");
    assert_eq!(first.answer.text, "args=[] prompt=one");
    assert_eq!(session::resume_id(&store).as_deref(), Some("sess-42"));
    let second = session::ask_in_review(&agent, &store, "two").expect("a reply");
    assert_eq!(second.answer.text, "args=[-r sess-42] prompt=two");
}
