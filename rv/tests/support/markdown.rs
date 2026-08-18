//! Editing the exported document the way a model replying to a review would.

/// Appends a `**Reply:**` block under every rendered comment body, the way an
/// LLM following the document's own protocol block would.
pub fn insert_reply(document: &str, reply: &str) -> String {
    let mut out = String::new();
    for line in document.lines() {
        out.push_str(line);
        out.push('\n');
        if line.starts_with("**Comment:**") {
            out.push_str(&format!("\n**Reply:** {reply}\n"));
        }
    }
    out
}
