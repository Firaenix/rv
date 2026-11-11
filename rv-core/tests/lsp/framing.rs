use std::io::{BufReader, Cursor, Read};

use rv_core::lsp::{FrameError, read_frame, write_frame};

fn framed(bodies: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for body in bodies {
        write_frame(&mut bytes, body.as_bytes()).unwrap();
    }
    bytes
}

/// Hands out at most one byte per read, as a slow pipe might.
struct Trickle(Cursor<Vec<u8>>);

impl Read for Trickle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let one = buf.len().min(1);
        self.0.read(&mut buf[..one])
    }
}

#[test]
fn write_frame_emits_content_length_header() {
    assert_eq!(framed(&["{}"]), b"Content-Length: 2\r\n\r\n{}");
}

#[test]
fn single_frame_is_read_then_clean_end_of_stream() {
    let mut reader = Cursor::new(framed(&[r#"{"id":1}"#]));
    assert_eq!(read_frame(&mut reader).unwrap().unwrap(), br#"{"id":1}"#);
    assert!(read_frame(&mut reader).unwrap().is_none());
}

#[test]
fn several_frames_in_one_buffer_are_read_in_order() {
    let mut reader = Cursor::new(framed(&["[1]", "[22]", "\"é🦀\""]));
    let bodies: Vec<Vec<u8>> = std::iter::from_fn(|| read_frame(&mut reader).unwrap()).collect();
    assert_eq!(
        bodies,
        vec![
            b"[1]".to_vec(),
            b"[22]".to_vec(),
            "\"é🦀\"".as_bytes().to_vec()
        ]
    );
}

#[test]
fn frames_split_across_reads_reassemble() {
    let mut reader =
        BufReader::with_capacity(1, Trickle(Cursor::new(framed(&["{\"a\":1}", "{}"]))));
    assert_eq!(read_frame(&mut reader).unwrap().unwrap(), b"{\"a\":1}");
    assert_eq!(read_frame(&mut reader).unwrap().unwrap(), b"{}");
    assert!(read_frame(&mut reader).unwrap().is_none());
}

#[test]
fn extra_headers_and_header_case_are_tolerated() {
    let bytes = b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc\r\n\r\n{}".to_vec();
    assert_eq!(read_frame(&mut Cursor::new(bytes)).unwrap().unwrap(), b"{}");
}

#[test]
fn truncated_body_is_an_error() {
    let bytes = b"Content-Length: 10\r\n\r\n{}".to_vec();
    assert!(matches!(
        read_frame(&mut Cursor::new(bytes)),
        Err(FrameError::Truncated)
    ));
}

#[test]
fn header_without_length_is_an_error() {
    let bytes = b"Content-Type: x\r\n\r\n{}".to_vec();
    assert!(matches!(
        read_frame(&mut Cursor::new(bytes)),
        Err(FrameError::MissingLength)
    ));
}

#[test]
fn absurd_length_is_refused_before_allocating() {
    let bytes = b"Content-Length: 99999999999\r\n\r\n".to_vec();
    assert!(matches!(
        read_frame(&mut Cursor::new(bytes)),
        Err(FrameError::TooLarge(_))
    ));
}
