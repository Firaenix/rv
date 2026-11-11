//! A server's hover text made safe for a cell grid: escape sequences and
//! control characters gone, code fences dropped, and clipped to a few rows.

const MAX_LINES: usize = 6;
const MAX_LINE_CHARS: usize = 160;

pub(in crate::app) fn card_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in without_escapes(text).lines() {
        let line = printable(raw);
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed == "---" {
            continue;
        }
        let blank = trimmed.is_empty();
        if blank && lines.last().is_none_or(String::is_empty) {
            continue;
        }
        lines.push(if blank { String::new() } else { line });
        if lines.len() == MAX_LINES {
            break;
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// Drops CSI (`ESC [ … final`), OSC (`ESC ] … BEL` or `ESC \`) and any other
/// two-character escape.
fn without_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('[') => {
                for next in characters.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(next) = characters.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' && characters.peek() == Some(&'\\') {
                        characters.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn printable(line: &str) -> String {
    line.trim_end()
        .chars()
        .flat_map(|character| match character {
            '\t' => vec![' '; 4],
            other if other.is_control() => Vec::new(),
            other => vec![other],
        })
        .take(MAX_LINE_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::card_lines;

    #[test]
    fn fences_escapes_and_controls_are_gone() {
        let text = "```rust\nfn alpha() -> Gamma\n```\n\n---\n\n\u{1b}[31mred\u{1b}[0m \u{1b}]8;;x\u{7}doc\u{7}\u{0}s";
        assert_eq!(card_lines(text), ["fn alpha() -> Gamma", "", "red docs"]);
    }

    #[test]
    fn long_text_is_clipped_to_a_few_rows() {
        let text = (0..20).map(|n| format!("line {n}\n")).collect::<String>();
        assert_eq!(card_lines(&text).len(), 6);
        assert!(card_lines(&"x".repeat(1000))[0].chars().count() <= 160);
    }
}
