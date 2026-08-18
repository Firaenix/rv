//! Where a byte range may be cut without splitting a character.
//!
//! tree-sitter reports byte offsets and ratatui draws characters, so every
//! span crosses this boundary. A cut inside a multi-byte character panics a
//! slice and mangles a glyph, and both are reachable from any file with a
//! non-ASCII identifier or comment in it.

/// The end of a line's text: `end` with a CRLF's `\r` removed.
pub(super) fn text_end(source: &[u8], start: usize, end: usize) -> usize {
    if end > start && source[end - 1] == b'\r' {
        end - 1
    } else {
        end
    }
}


/// True for a UTF-8 continuation byte — the bytes that are *not* the start of
/// a character. Works on bytes that are not valid UTF-8 at all, which is the
/// point: this module clamps blobs it has not validated.
pub(super) fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// The smallest index `i` in `at..=ceil` that starts a character.
pub(super) fn ceil_char_boundary(source: &[u8], mut at: usize, ceil: usize) -> usize {
    while at < ceil && source.get(at).copied().is_some_and(is_continuation) {
        at += 1;
    }
    at
}

/// The largest index `i` in `floor..=at` that starts a character (or is the
/// end of the blob, which always is one).
pub(super) fn floor_byte_boundary(source: &[u8], mut at: usize, floor: usize) -> usize {
    while at > floor && source.get(at).copied().is_some_and(is_continuation) {
        at -= 1;
    }
    at
}

// ---------------------------------------------------------------------------
// Grammars
// ---------------------------------------------------------------------------

