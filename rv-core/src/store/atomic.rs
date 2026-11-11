//! The one way this module puts bytes on disk.

use std::io::ErrorKind;
use std::io::Write as _;
use std::path::Path;

use tempfile::Builder;

use super::Error;

/// Prefix on the temp file [`write_atomic`] creates before renaming it into
/// place, so a leftover (only possible if the process is killed between the
/// `fsync` and the `rename` — every other early return drops and so deletes
/// the [`tempfile::NamedTempFile`]) is easy to recognize as `rv`'s.
const ATOMIC_TEMP_PREFIX: &str = ".rv-store-";

/// Writes `contents` to `path` without ever leaving `path` itself partially
/// written.
///
/// The bytes go to a fresh, uniquely-named temp file created in `path`'s own
/// directory — never a shared temp directory, since `rename` is only atomic
/// when source and destination share a filesystem — and are fsynced there
/// (so the write survives a power loss, not just a killed process) before
/// the temp file is renamed onto `path`. `rename` on POSIX is atomic: any
/// reader of `path` sees either the old complete contents or the new
/// complete contents, never a mix. This function does not additionally
/// fsync `path`'s parent directory after the rename, so it does not
/// guarantee the *directory entry* update itself survives a power loss the
/// instant after `persist` returns — closing that gap needs an extra
/// directory fsync this module skips as unwarranted complexity for a local
/// review scratch directory.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), Error> {
    let dir = path.parent().ok_or_else(|| Error::Io {
        path: path.to_owned(),
        source: std::io::Error::new(
            ErrorKind::InvalidInput,
            "path has no parent directory to hold its temp file",
        ),
    })?;

    let mut temp = Builder::new()
        .prefix(ATOMIC_TEMP_PREFIX)
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
    temp.write_all(contents).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    temp.as_file().sync_all().map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    temp.persist(path).map_err(|error| Error::Io {
        path: path.to_owned(),
        source: error.error,
    })?;
    Ok(())
}
