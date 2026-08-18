//! Painting a frame and reading it back.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;
use std::time::SystemTime;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use rv::app::App;
use rv::app::Mode;
use rv::layout::Chrome;
use rv::layout::Split;
use rv::layout::layout;
use rv::ui;
use rv_core::diff::LineKind;

/// Every file in the **whole workspace**, as `(path relative to the root,
/// mtime, bytes)`, sorted: a snapshot of everything on disk an action could
/// have touched.
///
/// Used to assert that an action wrote *nothing at all*, which is a stronger
/// and more durable claim than checking one filename — it holds whatever the
/// store keeps its comments in, so it survives the move to `session.toml`.
///
/// The whole root rather than `.review/`, which is what this used to walk. A
/// guard scoped to one directory only forbids writing *there*: a mutant that
/// spilled the fold set into `rv-folds.txt` beside `.review/` — one level up,
/// in the workspace the reviewer is reading — passed both collapse guards
/// untouched. "Nothing reached disk" is the claim, and the workspace is where
/// disk is.
///
/// The mtime is part of the snapshot on purpose. Bytes alone would let a
/// rewrite that produced the same document pass as "nothing happened", and a
/// rewrite *is* something that happened: the export exists to be read by
/// another program, which sees the write whatever the bytes say.
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

/// The geometry `ui::draw` lays out, asked of the same function it paints
/// from, only to know *where* to look in the buffer. Nothing about the layout
/// is under test here — `rv/tests/layout.rs` owns that.
pub fn diff_area(width: u16, height: u16, mode: Mode) -> Rect {
    let bar_rows = if mode == Mode::Browse { 1 } else { 3 };
    layout(
        Rect::new(0, 0, width, height),
        Split::default(),
        Chrome {
            bar_rows,
            help_open: false,
            toast: false,
            sidebar_hidden: false,
        },
    )
    .diff
}

/// Whether a comment box is drawn inside the diff pane at `width` x `height`.
///
/// Inside the pane rather than across the frame: the panes are drawn with
/// rounded corners, so `╭` is a frame at the edge of the screen and a box only
/// within one.
pub fn box_drawn(app: &App, width: u16, height: u16) -> bool {
    let terminal = render(app, width, height);
    let buffer = terminal.backend().buffer();
    let area = diff_area(width, height, app.mode());
    ((area.y + 1)..area.bottom().saturating_sub(1)).any(|y| {
        ((area.x + 1)..area.right().saturating_sub(1)).any(|x| buffer[(x, y)].symbol() == "╭")
    })
}

pub fn render(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app, epoch()))
        .expect("draw a frame");
    terminal
}

/// The instant every frame in this file is painted at.
///
/// One instant for the whole process, because `drawing_never_panics_at_any_size`
/// claims a frame is a function of the app alone: a toast fades, so painting the
/// same app at two instants is allowed to paint two frames, and a clock read per
/// call would turn that determinism claim into a race.
pub fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// The five-character line number printed on the highlighted row of the diff
/// pane at a `width` x `height` terminal, found by the **selection's
/// background** rather than by matching text — so a duplicated line cannot be
/// mistaken for the selected one.
///
/// It used to be found by the `REVERSED` modifier. The wave that put syntax
/// colours inside the diff took the reverse away deliberately: reversing swaps
/// the foreground and the background, so on a tinted line it turns the syntax
/// colours into the wash and the wash into the text. The selection is now a
/// *brighter* version of the line's own tint, and `ui::line_background` is the
/// one function that decides both — so this asks it rather than keeping a
/// second copy of the palette here.
///
/// The geometry is a parameter because it is the interesting variable: the
/// pane's window only scrolls once the diff is taller than the pane, so a
/// number checked at one generous height says nothing about what a reviewer on
/// a short terminal is shown.
pub fn printed_number(app: &App, width: u16, height: u16) -> Option<u32> {
    let terminal = render(app, width, height);
    let area = diff_area(width, height, app.mode());
    let buffer = terminal.backend().buffer().clone();

    let selected: Vec<Color> = [LineKind::Added, LineKind::Removed, LineKind::Context]
        .into_iter()
        .filter_map(|kind| ui::line_background(kind, true))
        .collect();
    let inner_x = area.x + 1;
    let row = (area.y + 1..area.y + area.height.saturating_sub(1)).find(|y| {
        buffer[(inner_x, *y)]
            .style()
            .bg
            .is_some_and(|background| selected.contains(&background))
    })?;
    let text: String = (inner_x..inner_x + 5)
        .map(|x| buffer[(x, row)].symbol().to_owned())
        .collect();
    text.trim().parse().ok()
}
