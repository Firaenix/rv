//! Agent output is untrusted: it reaches the terminal and the store only after
//! terminal escapes, control characters and bidi overrides are gone and its
//! length is capped.

const ESCAPE: char = '\u{1b}';
const ELLIPSIS: char = '…';

/// Multi-line text: newlines survive, tabs become spaces.
pub fn text(raw: &str, max_chars: usize) -> String {
    let cleaned: String = without_escapes(raw)
        .chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if is_unsafe(c) => None,
            c => Some(c),
        })
        .collect();
    capped(cleaned.trim(), max_chars)
}

/// One line: every run of whitespace, newlines included, becomes one space.
pub fn line(raw: &str, max_chars: usize) -> String {
    let cleaned: String = without_escapes(raw)
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|&c| !is_unsafe(c))
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    capped(&joined, max_chars)
}

fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn capped(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    kept.push(ELLIPSIS);
    kept
}

/// Drops CSI (`ESC [ … final`), OSC (`ESC ] … BEL` or `ESC \`) and two-byte
/// escapes, plus their 8-bit C1 forms.
fn without_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        let introducer = match c {
            ESCAPE => chars.next(),
            '\u{9b}' => Some('['),
            '\u{9d}' => Some(']'),
            _ => {
                out.push(c);
                continue;
            }
        };
        match introducer {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']' | 'P' | '_' | '^') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || c == '\u{9c}' {
                        break;
                    }
                    if c == ESCAPE && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_and_title_escapes_are_removed() {
        let raw = "\u{1b}[31mred\u{1b}[0m \u{1b}]0;title\u{7}done \u{1b}]8;;http://x\u{1b}\\link";
        assert_eq!(text(raw, 100), "red done link");
    }

    #[test]
    fn controls_and_bidi_overrides_are_removed_but_newlines_kept() {
        assert_eq!(text("a\u{0}b\r\nc\u{202e}d\u{8}", 100), "ab\ncd");
        assert_eq!(line("  a\n\n b\tc  ", 100), "a b c");
    }

    #[test]
    fn long_text_is_capped_with_an_ellipsis() {
        assert_eq!(line("abcdef", 4), "abc…");
        assert_eq!(line("abcd", 4), "abcd");
    }
}
