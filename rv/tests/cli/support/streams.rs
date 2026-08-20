//! Assertion-message helpers shared by the `cli` cases.

use std::process::Output;

/// Renders `output`'s streams for an assertion message.
pub fn streams(output: &Output) -> String {
    format!(
        "status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

/// Every file in the workspace, so a test can say "this wrote nothing at all".
pub fn tree(root: &std::path::Path) -> Vec<(String, std::time::SystemTime, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(metadata) = std::fs::metadata(&path) {
                files.push((
                    path.strip_prefix(root)
                        .expect("under the root")
                        .display()
                        .to_string(),
                    metadata.modified().expect("an mtime"),
                    std::fs::read(&path).unwrap_or_default(),
                ));
            }
        }
    }
    files.sort();
    files
}
