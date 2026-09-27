//! Rewriting claims that have exactly one candidate (ADR-0015), planned before it is
//! applied (ADR-0022).
//!
//! The preview and the rewrite are the same plan. `plan` reads each document once and
//! records what it saw; `diff` renders the plan without touching anything; `apply`
//! writes it, but only to a document whose contents are still what was planned against.
//! So a preview is a promise about exactly what `--fix` would write, and a document
//! someone edited in between is left alone rather than overwritten.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::gate::Finding;

/// One rewrite: where, what must be there now, and what goes there instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedEdit {
    pub file: PathBuf,
    /// Byte span within the document.
    pub span: Range<usize>,
    /// The claim text the span held when the plan was made.
    pub expected: String,
    pub replacement: String,
}

/// Why an edit was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// Another edit's span overlaps this one's. Either could be right, so neither is.
    Overlap,
    /// The document could not be read.
    Unreadable,
    /// The span no longer holds the claim text.
    SpanMismatch,
    /// The document changed between planning and applying.
    Changed,
    /// The rewritten document could not be written.
    WriteFailed,
}

impl Rejection {
    pub fn code(self) -> &'static str {
        match self {
            Rejection::Overlap => "overlap",
            Rejection::Unreadable => "unreadable",
            Rejection::SpanMismatch => "span-mismatch",
            Rejection::Changed => "changed",
            Rejection::WriteFailed => "write-failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditStatus {
    /// Planned, and not yet applied — which is where a preview leaves every edit.
    Eligible,
    Applied,
    Rejected(Rejection),
}

impl EditStatus {
    pub fn code(self) -> &'static str {
        match self {
            EditStatus::Eligible => "eligible",
            EditStatus::Applied => "applied",
            EditStatus::Rejected(_) => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRecord {
    pub edit: PlannedEdit,
    pub status: EditStatus,
}

/// The edits for one document, and the contents they were planned against.
#[derive(Debug, Clone)]
pub struct FilePlan {
    pub file: PathBuf,
    pub original: String,
    /// A hash of `original`. Apply compares the document's current contents with it.
    pub identity: String,
    /// Sorted by span, none overlapping.
    pub edits: Vec<PlannedEdit>,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub files: Vec<FilePlan>,
    /// Edits refused while planning.
    pub rejected: Vec<EditRecord>,
}

impl Plan {
    /// Every edit, as a preview leaves it: planned ones eligible, the rest rejected.
    pub fn records(&self) -> Vec<EditRecord> {
        let mut out: Vec<EditRecord> = self
            .files
            .iter()
            .flat_map(|f| &f.edits)
            .map(|edit| EditRecord {
                edit: edit.clone(),
                status: EditStatus::Eligible,
            })
            .collect();
        out.extend(self.rejected.iter().cloned());
        sort_records(&mut out);
        out
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.rejected.is_empty()
    }
}

fn sort_records(records: &mut [EditRecord]) {
    records
        .sort_by(|a, b| (&a.edit.file, a.edit.span.start).cmp(&(&b.edit.file, b.edit.span.start)));
}

/// What a finding can be rewritten to, if anything.
///
/// Exactly one suggestion, never the best of several — a small candidate set is
/// enumerated rather than guessed at, and a set of one is the only case where
/// enumeration and certainty are the same thing.
fn replacement(finding: &Finding) -> Option<String> {
    // Rendered, not raw: a command's suggestion is its bare target, and the claim text
    // it replaces is `make demo`. Going through the reporter means what is written is
    // exactly what was offered.
    match crate::report::suggestions_of(finding).as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// A content hash, for telling whether a document changed under us.
fn identity(contents: &str) -> String {
    crate::cache::fnv1a(contents)
}

/// Plan every unambiguous fix. Reads each document once and writes nothing.
pub fn plan(findings: &[Finding], root: &Path) -> Plan {
    let mut by_file: BTreeMap<PathBuf, Vec<PlannedEdit>> = BTreeMap::new();
    for finding in findings {
        let Some(replacement) = replacement(finding) else {
            continue;
        };
        if replacement == finding.claim.text {
            continue;
        }
        let edit = PlannedEdit {
            file: finding.claim.file.clone(),
            span: finding.claim.span.clone(),
            expected: finding.claim.text.clone(),
            replacement,
        };
        let edits = by_file.entry(edit.file.clone()).or_default();
        // The same claim reported twice asks for the same rewrite twice; that is one
        // edit, not a conflict.
        if !edits.contains(&edit) {
            edits.push(edit);
        }
    }

    let mut plan = Plan::default();
    for (file, mut edits) in by_file {
        let reject = |edit: PlannedEdit, why| EditRecord {
            edit,
            status: EditStatus::Rejected(why),
        };
        let Ok(original) = std::fs::read_to_string(root.join(&file)) else {
            plan.rejected
                .extend(edits.into_iter().map(|e| reject(e, Rejection::Unreadable)));
            continue;
        };

        // The span is where the claim was when it was read. If the bytes there are not
        // the claim any more, this is not our text to rewrite.
        let (valid, mismatched): (Vec<_>, Vec<_>) = edits
            .drain(..)
            .partition(|e| original.get(e.span.clone()) == Some(e.expected.as_str()));
        plan.rejected.extend(
            mismatched
                .into_iter()
                .map(|e| reject(e, Rejection::SpanMismatch)),
        );

        let mut valid = valid;
        valid.sort_by_key(|e| (e.span.start, e.span.end));
        let mut overlapping = vec![false; valid.len()];
        for i in 1..valid.len() {
            // Sorted by start, so an overlap is any edit starting before the furthest
            // end seen so far. Both sides go: which one was meant is a human's call.
            let reach = valid[..i].iter().map(|e| e.span.end).max().unwrap_or(0);
            if valid[i].span.start < reach {
                overlapping[i] = true;
                for j in 0..i {
                    if valid[j].span.end > valid[i].span.start {
                        overlapping[j] = true;
                    }
                }
            }
        }
        let mut kept = Vec::new();
        for (edit, overlaps) in valid.into_iter().zip(overlapping) {
            if overlaps {
                plan.rejected.push(reject(edit, Rejection::Overlap));
            } else {
                kept.push(edit);
            }
        }
        if !kept.is_empty() {
            plan.files.push(FilePlan {
                identity: identity(&original),
                file,
                original,
                edits: kept,
            });
        }
    }
    sort_records(&mut plan.rejected);
    plan
}

/// The document with its planned edits applied.
pub fn render(file: &FilePlan) -> String {
    let mut out = file.original.clone();
    // Last span first: splicing forwards invalidates every offset after the edit.
    for edit in file.edits.iter().rev() {
        out.replace_range(edit.span.clone(), &edit.replacement);
    }
    out
}

/// Lines of context around each change, as `diff -u` uses.
const CONTEXT: usize = 3;

/// The plan as a unified diff, `a/` and `b/` prefixed so `git apply` takes it as is.
pub fn diff(plan: &Plan) -> String {
    let mut out = String::new();
    for file in &plan.files {
        out.push_str(&unified(file));
    }
    out
}

fn unified(file: &FilePlan) -> String {
    let old = &file.original;
    let lines: Vec<(usize, usize)> = {
        let mut start = 0;
        old.split_inclusive('\n')
            .map(|line| {
                let range = (start, start + line.len());
                start += line.len();
                range
            })
            .collect()
    };
    let line_of = |offset: usize| {
        lines
            .iter()
            .position(|&(start, end)| offset >= start && offset < end)
            .unwrap_or(lines.len().saturating_sub(1))
    };

    // A block is a run of lines some edit touches; it is replaced as a whole.
    let mut blocks: Vec<(usize, usize, Vec<&PlannedEdit>)> = Vec::new();
    for edit in &file.edits {
        let first = line_of(edit.span.start);
        let last = line_of(edit.span.end.saturating_sub(1).max(edit.span.start));
        match blocks.last_mut() {
            Some((_, end, edits)) if first <= *end + 1 => {
                *end = (*end).max(last);
                edits.push(edit);
            }
            _ => blocks.push((first, last, vec![edit])),
        }
    }

    // Hunks: blocks close enough that their context would overlap share one.
    let mut hunks: Vec<Vec<(usize, usize, Vec<&PlannedEdit>)>> = Vec::new();
    for block in blocks {
        match hunks.last_mut() {
            Some(hunk) if block.0 <= hunk.last().unwrap().1 + 2 * CONTEXT + 1 => hunk.push(block),
            _ => hunks.push(vec![block]),
        }
    }

    let mut out = format!(
        "--- a/{0}\n+++ b/{0}\n",
        file.file.to_string_lossy().replace('\\', "/")
    );
    let mut delta: isize = 0;
    for hunk in hunks {
        let from = hunk.first().unwrap().0.saturating_sub(CONTEXT);
        let to = (hunk.last().unwrap().1 + CONTEXT).min(lines.len() - 1);
        let mut body = String::new();
        let (mut old_count, mut new_count) = (0usize, 0usize);
        let mut line = from;
        for (first, last, edits) in &hunk {
            while line < *first {
                push_line(&mut body, ' ', &old[lines[line].0..lines[line].1]);
                old_count += 1;
                new_count += 1;
                line += 1;
            }
            let start = lines[*first].0;
            let end = lines[*last].1;
            let mut replaced = old[start..end].to_string();
            for edit in edits.iter().rev() {
                replaced.replace_range(
                    edit.span.start - start..edit.span.end - start,
                    &edit.replacement,
                );
            }
            for removed in old[start..end].split_inclusive('\n') {
                push_line(&mut body, '-', removed);
                old_count += 1;
            }
            for added in replaced.split_inclusive('\n') {
                push_line(&mut body, '+', added);
                new_count += 1;
            }
            line = last + 1;
        }
        while line <= to {
            push_line(&mut body, ' ', &old[lines[line].0..lines[line].1]);
            old_count += 1;
            new_count += 1;
            line += 1;
        }
        let new_start = (from as isize + 1 + delta) as usize;
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            range(from + 1, old_count),
            range(new_start, new_count)
        ));
        out.push_str(&body);
        delta += new_count as isize - old_count as isize;
    }
    out
}

/// A hunk range as `diff -u` writes it: a one-line range is just its line number.
fn range(start: usize, count: usize) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

fn push_line(out: &mut String, prefix: char, line: &str) {
    out.push(prefix);
    out.push_str(line);
    if !line.ends_with('\n') {
        out.push_str("\n\\ No newline at end of file\n");
    }
}

/// Write the plan. A document whose contents are no longer what the plan was made
/// against is not written at all, and each file is replaced atomically — renamed into
/// place — so a reader never sees half a rewrite. There is no transaction across
/// files: one can be applied while another is refused, and the records say which.
pub fn apply(plan: &Plan, root: &Path) -> Vec<EditRecord> {
    let mut records: Vec<EditRecord> = plan.rejected.clone();
    for file in &plan.files {
        let path = root.join(&file.file);
        let status = match std::fs::read_to_string(&path) {
            Err(_) => EditStatus::Rejected(Rejection::Unreadable),
            Ok(current) if identity(&current) != file.identity => {
                eprintln!(
                    "stilltrue: warning: {} changed since it was read; not rewriting it",
                    file.file.display()
                );
                EditStatus::Rejected(Rejection::Changed)
            }
            Ok(_) => match write_atomically(&path, &render(file)) {
                Ok(()) => {
                    for edit in &file.edits {
                        // stderr, like the tally. stdout carries the report, and under
                        // `--format json` a line appended after it is a second document.
                        eprintln!(
                            "stilltrue: {}: `{}` -> `{}`",
                            file.file.display(),
                            edit.expected,
                            edit.replacement
                        );
                    }
                    EditStatus::Applied
                }
                Err(error) => {
                    eprintln!(
                        "stilltrue: warning: could not write {}: {error}",
                        file.file.display()
                    );
                    EditStatus::Rejected(Rejection::WriteFailed)
                }
            },
        };
        records.extend(file.edits.iter().map(|edit| EditRecord {
            edit: edit.clone(),
            status,
        }));
    }
    sort_records(&mut records);
    records
}

/// Replace `path` with `contents` by writing a sibling and renaming it over the
/// original, keeping the original's permissions.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.stilltrue-{}.tmp", std::process::id()));
    let result = (|| {
        std::fs::write(&temporary, contents)?;
        std::fs::set_permissions(&temporary, std::fs::metadata(path)?.permissions())?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// The edit records as the run report carries them.
pub fn records_json(records: &[EditRecord]) -> serde_json::Value {
    let count = |code: &str| records.iter().filter(|r| r.status.code() == code).count();
    serde_json::json!({
        "eligible": count("eligible"),
        "applied": count("applied"),
        "rejected": count("rejected"),
        "records": records.iter().map(|r| serde_json::json!({
            "file": r.edit.file.to_string_lossy(),
            "span": {"start": r.edit.span.start, "end": r.edit.span.end},
            "expected": r.edit.expected,
            "replacement": r.edit.replacement,
            "status": r.status.code(),
            "reason": match r.status {
                EditStatus::Rejected(why) => Some(why.code()),
                _ => None,
            },
        })).collect::<Vec<_>>(),
    })
}
