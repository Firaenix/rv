//! Painting one frame and reading it back out of the buffer.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;
use std::time::SystemTime;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use rv::app::App;
use rv::gradient;
use rv::ui;

/// The instant every frame is painted at unless the test says otherwise.
///
/// Fixed once per process rather than read per frame: a frame is a function of
/// the app *and* the time — toasts fade — so two frames at two instants would
/// answer two different questions. Only the alert tests pass their own.
pub fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// One frame of the reviewer, as a 100x24 `TestBackend` renders it.
pub fn render(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("build a test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app, epoch()))
        .expect("draw a frame");
    terminal.backend().to_string()
}

/// One frame at an arbitrary size, as **cells** rather than as text — a test
/// that only greps the text of a frame passes on an unstyled box.
pub fn frame_at(app: &App, width: u16, height: u16) -> Buffer {
    frame_at_time(app, width, height, epoch())
}

/// The same, painted at an instant the test chooses — which is how a toast's
/// fade is an assertion rather than a sleep.
pub fn frame_at_time(app: &App, width: u16, height: u16, now: Instant) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("build a test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app, now))
        .expect("draw a frame");
    terminal.backend().buffer().clone()
}

/// The frame's rows, one string per terminal row.
pub fn rows_of(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// The whole frame as text, rows separated by newlines.
pub fn buffer_text(buffer: &Buffer) -> String {
    rows_of(buffer).join("\n")
}

/// The last row of the frame, which is where the bar is drawn.
pub fn last_row(buffer: &Buffer) -> String {
    rows_of(buffer).pop().expect("a frame has rows")
}

/// The row `needle` first appears on, as an index into [`rows_of`].
pub fn row_holding(buffer: &Buffer, needle: &str) -> usize {
    rows_of(buffer)
        .iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on screen:\n{}", buffer_text(buffer)))
}

/// The style of the first cell of `needle` on row `y`.
pub fn style_of_text(buffer: &Buffer, y: u16, needle: &str) -> ratatui::style::Style {
    let row: String = (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    let column = row
        .char_indices()
        .position(|(offset, _)| row[offset..].starts_with(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on row {y}: {row:?}"));
    buffer[(u16::try_from(column).expect("a small column"), y)].style()
}

/// One of [`rv::gradient`]'s colours, as `ui` sends it to the terminal.
pub fn colour(gradient::Rgb(red, green, blue): gradient::Rgb) -> Color {
    Color::Rgb(red, green, blue)
}

/// Every file in the **whole workspace**, as `(path relative to the root,
/// mtime, bytes)`, sorted.
///
/// Comparing two of these is how a test says "this action wrote nothing at
/// all". The whole root rather than `.review/`: a guard scoped to one directory
/// only forbids writing *there*, and a mutant that spilled state into a file
/// beside `.review/` passed both collapse guards untouched. The mtime is in
/// there because a rewrite producing identical bytes is still a write, and
/// another program watching `.review/` sees it.
pub fn workspace_tree(root: &Path) -> Vec<(String, SystemTime, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let name = path
                    .strip_prefix(root)
                    .expect("a path under the workspace root")
                    .display()
                    .to_string();
                let Ok(metadata) = fs::metadata(&path) else {
                    // A dangling symlink, or a file that went between the
                    // listing and the stat. Neither is something `rv` wrote.
                    continue;
                };
                let modified = metadata.modified().expect("an mtime");
                files.push((name, modified, fs::read(&path).unwrap_or_default()));
            }
        }
    }
    files.sort();
    files
}
