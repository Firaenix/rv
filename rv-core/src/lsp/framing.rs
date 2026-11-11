use std::io::{self, BufRead, Write};

/// Larger than any honest response; a garbage header must not become a
/// multi-gigabyte allocation.
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("reading a frame: {0}")]
    Io(#[from] io::Error),
    #[error("frame header has no Content-Length")]
    MissingLength,
    #[error("frame header is malformed: {0}")]
    MalformedHeader(String),
    #[error("frame of {0} bytes exceeds the limit")]
    TooLarge(usize),
    #[error("stream ended inside a frame")]
    Truncated,
}

pub fn write_frame(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

/// `Ok(None)` is a clean end of stream between frames.
pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, FrameError> {
    let mut content_length = None;
    let mut saw_any_header = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return if saw_any_header {
                Err(FrameError::Truncated)
            } else {
                Ok(None)
            };
        }
        saw_any_header = true;
        let text = String::from_utf8_lossy(&line);
        let header = text.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            return Err(FrameError::MalformedHeader(header.to_owned()));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| FrameError::MalformedHeader(header.to_owned()))?;
            content_length = Some(length);
        }
    }
    let length = content_length.ok_or(FrameError::MissingLength)?;
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(length));
    }
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => FrameError::Truncated,
            _ => FrameError::Io(error),
        })?;
    Ok(Some(body))
}
