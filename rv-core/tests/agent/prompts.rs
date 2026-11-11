use rv_core::agent::prompt;

const FILE: &str = "fn a() {}\nfn b() {}\nfn c() {}\n";

#[test]
fn orientation_numbers_every_line_and_asks_for_the_contract() {
    let asked = prompt::orientation("src/lib.rs", FILE);
    assert!(asked.contains("`src/lib.rs`"), "{asked}");
    assert!(
        asked.contains("    1 | fn a() {}\n    2 | fn b() {}\n    3 | fn c() {}\n"),
        "{asked}"
    );
    assert!(asked.contains("\"focus\""), "{asked}");
}

#[test]
fn a_selection_shows_only_its_lines_and_the_question() {
    let asked = prompt::selection("src/lib.rs", FILE, 2, 3, Some("why two?"));
    assert!(asked.contains("lines 2–3 of `src/lib.rs`"), "{asked}");
    assert!(asked.contains("Question: why two?"), "{asked}");
    assert!(
        asked.contains("    2 | fn b() {}\n    3 | fn c() {}\n"),
        "{asked}"
    );
    assert!(!asked.contains("fn a()"), "{asked}");
    let default = prompt::selection("src/lib.rs", FILE, 1, 1, Some("  "));
    assert!(default.contains("line 1 of"), "{default}");
    assert!(default.contains("Question: Explain"), "{default}");
}

#[test]
fn a_huge_file_is_cut_with_a_note() {
    let huge = format!("{}\n", "x".repeat(100)).repeat(2_000);
    let asked = prompt::orientation("big.txt", &huge);
    assert!(
        asked.len() < prompt::MAX_FILE_CHARS + 5_000,
        "{}",
        asked.len()
    );
    assert!(asked.contains("it was cut here"));
}
