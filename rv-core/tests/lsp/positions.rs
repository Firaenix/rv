use std::path::Path;

use rv_core::lsp::{char_column_to_utf16, path_to_uri, uri_to_path, utf16_to_char_column};

const LINE: &str = "let é🦀x = \"🦀\";";

#[test]
fn bmp_multibyte_character_is_one_utf16_unit() {
    assert_eq!(char_column_to_utf16(LINE, 5), 5);
    assert_eq!(utf16_to_char_column(LINE, 5), 5);
}

#[test]
fn astral_character_is_two_utf16_units() {
    assert_eq!(char_column_to_utf16(LINE, 6), 7);
    assert_eq!(utf16_to_char_column(LINE, 7), 6);
}

#[test]
fn every_character_column_round_trips_through_utf16() {
    for column in 0..=LINE.chars().count() as u32 + 3 {
        let utf16 = char_column_to_utf16(LINE, column);
        assert_eq!(utf16_to_char_column(LINE, utf16), column, "column {column}");
    }
}

#[test]
fn utf16_offset_inside_a_surrogate_pair_maps_to_that_character() {
    assert_eq!(utf16_to_char_column(LINE, 6), 5);
}

#[test]
fn columns_past_line_end_extend_one_unit_per_character() {
    assert_eq!(char_column_to_utf16("é🦀", 4), 5);
    assert_eq!(utf16_to_char_column("é🦀", 5), 4);
}

#[test]
fn uri_escapes_spaces_and_non_ascii_and_round_trips() {
    let path = Path::new("/tmp/my repo/café#1.rs");
    let uri = path_to_uri(path);
    assert_eq!(uri, "file:///tmp/my%20repo/caf%C3%A9%231.rs");
    assert_eq!(uri_to_path(&uri).unwrap(), path);
}

#[test]
fn non_file_uris_are_not_paths() {
    assert_eq!(uri_to_path("untitled:Untitled-1"), None);
    assert_eq!(uri_to_path("file://host/share/x"), None);
}
