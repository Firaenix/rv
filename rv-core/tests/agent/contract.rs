use rv_core::agent::Importance;
use rv_core::agent::parse_answer;

#[test]
fn the_last_contract_block_wins_and_other_code_blocks_stay_in_the_prose() {
    let raw = "See:\n```rust\nlet x = 1;\n```\n```json\n{\"summary\": \"first\"}\n```\n\
               ```json\n{\"summary\": \"second\", \"focus\": []}\n```";
    let answer = parse_answer(raw);
    assert_eq!(answer.summary, "second");
    assert!(answer.text.contains("let x = 1;"), "{}", answer.text);
    assert!(answer.text.contains("\"first\""), "{}", answer.text);
    assert!(!answer.text.contains("second"), "{}", answer.text);
}

#[test]
fn a_bare_json_reply_is_the_contract() {
    let answer = parse_answer(
        "{\"summary\": \"bare\", \"focus\": [{\"path\": \"./a.rs\", \"line\": 1, \"why\": \"w\", \"importance\": \"bogus\"}]}",
    );
    assert!(answer.structured);
    assert_eq!(answer.summary, "bare");
    assert_eq!(answer.text, "");
    assert_eq!(answer.focus[0].path, "a.rs");
    assert_eq!(answer.focus[0].importance, Importance::Medium);
}

#[test]
fn json_without_contract_keys_is_just_prose() {
    let answer = parse_answer("```json\n{\"other\": 1}\n```");
    assert!(!answer.structured);
    assert!(answer.text.contains("\"other\""));
}

#[test]
fn an_empty_reply_is_an_empty_answer() {
    let answer = parse_answer("");
    assert_eq!(answer.summary, "");
    assert!(answer.focus.is_empty());
}

#[test]
fn focus_is_capped_and_long_strings_are_clipped() {
    let entries: Vec<String> = (1..=80)
        .map(|line| {
            format!(
                "{{\"path\": \"a.rs\", \"line\": {line}, \"why\": \"{}\"}}",
                "w".repeat(1000)
            )
        })
        .collect();
    let answer = parse_answer(&format!(
        "{{\"summary\": \"{}\", \"focus\": [{}]}}",
        "s".repeat(5000),
        entries.join(",")
    ));
    assert_eq!(answer.focus.len(), rv_core::agent::contract::MAX_FOCUS);
    assert_eq!(
        answer.focus[0].why.chars().count(),
        rv_core::agent::contract::MAX_WHY_CHARS
    );
    assert_eq!(
        answer.summary.chars().count(),
        rv_core::agent::contract::MAX_SUMMARY_CHARS
    );
}
