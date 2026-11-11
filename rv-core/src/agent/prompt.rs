//! The prompts rv sends. Each carries only what the question needs and ends
//! with the reply contract that [`super::parse_answer`] reads.

/// Files beyond this are cut, with a note saying so: a whole generated file
/// costs a lot and orients nobody.
pub const MAX_FILE_CHARS: usize = 60_000;
pub const MAX_SELECTION_LINES: usize = 400;

const CONTRACT: &str = "\
Answer in plain prose first. Then end your reply with exactly one fenced JSON block, \
and nothing after it, in this shape:

```json
{\"summary\": \"one line\", \"focus\": [{\"path\": \"src/x.rs\", \"line\": 42, \"why\": \"one line\", \"importance\": \"high\"}]}
```

`summary` is one line. `focus` lists the lines a reviewer should look at, most important \
first: `path` relative to the repository root, `line` 1-based in the file as shown, \
`importance` one of high, medium, low. Use an empty list when nothing stands out. \
Do not modify any file.";

#[must_use]
pub fn orientation(path: &str, text: &str) -> String {
    format!(
        "You are helping someone review code. For the file `{path}` below: what is this file \
         for, and what in it deserves a careful review?\n\n{}\n\n{CONTRACT}\n",
        numbered_file(path, text, 1, usize::MAX)
    )
}

/// `first_line..=last_line` of `text`, 1-based, shown with its line numbers;
/// `question` is the reviewer's own, where they typed one.
#[must_use]
pub fn selection(
    path: &str,
    text: &str,
    first_line: u32,
    last_line: u32,
    question: Option<&str>,
) -> String {
    let first = first_line.max(1) as usize;
    let last = (last_line as usize).max(first);
    let span = if first == last {
        format!("line {first}")
    } else {
        format!("lines {first}–{last}")
    };
    let question = question
        .map(str::trim)
        .filter(|question| !question.is_empty())
        .map_or_else(
            || "Explain what this code does and whether anything in it looks wrong.".to_owned(),
            str::to_owned,
        );
    let shown_last = last.min(first + MAX_SELECTION_LINES - 1);
    format!(
        "You are helping someone review code. They selected {span} of `{path}`.\n\n\
         Question: {question}\n\n{}\n\n{CONTRACT}\n",
        numbered_file(path, text, first, shown_last)
    )
}

/// `rv brief --ask`: the brief as rendered, which is self-contained by design.
#[must_use]
pub fn brief(document: &str) -> String {
    format!(
        "You are helping someone review code. Below is their review brief: the notes they \
         left and the code around each. Answer each note, and name any other lines in the \
         reviewed files that deserve their attention. You cannot run `rv` from here; your \
         reply is shown to the reviewer.\n\n{document}\n\n{CONTRACT}\n"
    )
}

fn numbered_file(path: &str, text: &str, first: usize, last: usize) -> String {
    let mut shown = String::new();
    let mut cut = false;
    for (number, line) in (1..).zip(text.lines()).skip(first - 1) {
        if number > last {
            break;
        }
        if shown.len() + line.len() > MAX_FILE_CHARS {
            cut = true;
            break;
        }
        shown.push_str(&format!("{number:>5} | {line}\n"));
    }
    let fence = if text.contains("````") {
        "``````"
    } else {
        "````"
    };
    let mut block = format!("{fence}{path}\n{shown}{fence}");
    if cut {
        block.push_str("\n(The file continues; it was cut here to keep the prompt small.)");
    }
    block
}
