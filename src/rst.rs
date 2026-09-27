//! reStructuredText, for the documentation ecosystems that never adopted Markdown.
//!
//! Deliberately not a parser. reStructuredText is large, and only three of its surfaces
//! carry claims: an inline literal, a `code-block` directive, and a link. Everything
//! else is prose, and a line-oriented reader that understands those three is both
//! enough and far less likely to invent a claim than a half-finished grammar would be.

use std::path::Path;

use crate::claim::{Claim, ClaimKind};
use crate::extract::{Extraction, LineIndex, UnusedSuppression, classify, command_of};

/// Directives whose body is shell, and therefore holds commands (ADR-0002's rule,
/// transposed: only a block that says it is shell is read as shell).
const SHELL_LANGUAGES: &[&str] = &["bash", "sh", "shell", "console", "shell-session", "zsh"];

/// Directives whose body is a record of the past.
///
/// `.. versionadded:: 0.4` followed by ``LOGGER_NAME`` says that key was added in 0.4.
/// It is a true statement about a version, and flask's config page carries a dozen of
/// them naming keys that have since been removed. This is ADR-0012's argument — a
/// changelog is supposed to name things that have moved — except that here the
/// changelog is a paragraph inside a live document rather than a file of its own.
const HISTORICAL_DIRECTIVES: &[&str] = &[
    "versionadded",
    "versionchanged",
    "versionremoved",
    "deprecated",
];

pub fn extract(source: &str, file: &Path) -> Extraction {
    let lines = LineIndex::new(source);
    let mut claims = Vec::new();
    let mut unused_suppressions = Vec::new();
    let mut markers = 0;

    // Byte offset of each line, so a claim can be located without re-scanning.
    let mut offset = 0usize;
    let mut starts = Vec::new();
    for line in source.split_inclusive('\n') {
        starts.push(offset);
        offset += line.len();
    }
    let raw: Vec<&str> = source.split_inclusive('\n').map(|l| l.trim_end()).collect();

    let mut shell_until: Option<usize> = None;
    let mut skip_until: Option<usize> = None;
    let mut pending_ignore: Option<usize> = None;
    let mut suppressing: Option<usize> = None;
    let mut covered = false;
    let mut suppressed = Vec::new();

    for (index, line) in raw.iter().enumerate() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if trimmed == ".. stilltrue:ignore-file" || trimmed.starts_with(".. stilltrue:ignore-file ")
        {
            return Extraction {
                markers: markers + 1,
                whole_file: true,
                ..Extraction::default()
            };
        }
        if trimmed == ".. stilltrue:ignore" || trimmed.starts_with(".. stilltrue:ignore ") {
            markers += 1;
            pending_ignore = Some(index);
            continue;
        }

        // Inside a directive whose body says nothing about the repository as it is now.
        if let Some(body_indent) = skip_until {
            if trimmed.is_empty() || indent > body_indent {
                continue;
            }
            skip_until = None;
        }

        // Inside a directive body: indented more than the directive itself.
        if let Some(body_indent) = shell_until {
            if trimmed.is_empty() {
                continue;
            }
            if indent > body_indent {
                for segment in crate::extract::split_segments(trimmed) {
                    let Some((text, runner, args)) = command_of(&segment) else {
                        continue;
                    };
                    let Some(column_in_line) = line.find(segment.trim()) else {
                        continue;
                    };
                    let start = starts[index] + column_in_line;
                    push(
                        &mut claims,
                        ClaimKind::Command { runner, args },
                        text,
                        start,
                        segment.trim().len(),
                        file,
                        &lines,
                    );
                }
                continue;
            }
            shell_until = None;
        }

        if let Some(language) = directive_language(trimmed)
            && SHELL_LANGUAGES.contains(&language.as_str())
        {
            shell_until = Some(indent);
            continue;
        }
        if is_historical(trimmed) {
            skip_until = Some(indent);
            continue;
        }

        // A blank line is not the block the marker covers — reStructuredText separates
        // paragraphs with them, so the marker's block starts at the next line that has
        // something on it and runs until the next blank.
        if trimmed.is_empty() {
            if let Some(marker) = suppressing.take()
                && !covered
            {
                let (line_no, column) = lines.locate(starts[marker]);
                unused_suppressions.push(UnusedSuppression {
                    line: line_no,
                    column,
                });
            }
            continue;
        }
        if suppressing.is_none()
            && let Some(marker) = pending_ignore.take()
        {
            suppressing = Some(marker);
            covered = false;
        }
        let before = claims.len();

        for (column_in_line, text) in inline_literals(line) {
            let start = starts[index] + column_in_line;
            if let Some(kind) = classify(&text) {
                let length = text.len();
                push(&mut claims, kind, text, start, length, file, &lines);
            }
        }
        for (column_in_line, target) in link_targets(line) {
            let start = starts[index] + column_in_line;
            if let Some(claim) = link_claim(&target, start, file, &lines) {
                claims.push(claim);
            }
        }

        if suppressing.is_some() && claims.len() > before {
            covered = true;
            suppressed.extend(before..claims.len());
        }
    }

    // A marker at the very end of the file, covering nothing.
    if let Some(marker) = suppressing
        && !covered
    {
        let (line_no, column) = lines.locate(starts[marker]);
        unused_suppressions.push(UnusedSuppression {
            line: line_no,
            column,
        });
    }

    // Drop what a marker covered, last index first so the earlier ones stay valid.
    suppressed.sort_unstable();
    for index in suppressed.into_iter().rev() {
        claims.remove(index);
    }

    crate::extract::collect_versions(source, file, &lines, &mut claims);
    claims.sort_by_key(|c| c.span.start);
    Extraction {
        claims,
        unused_suppressions,
        markers,
        ..Extraction::default()
    }
}

fn push(
    claims: &mut Vec<Claim>,
    kind: ClaimKind,
    text: String,
    start: usize,
    length: usize,
    file: &Path,
    lines: &LineIndex<'_>,
) {
    let (line, column) = lines.locate(start);
    let (end_line, end_column) = lines.locate(start + length);
    claims.push(Claim {
        kind,
        text,
        file: file.to_path_buf(),
        line,
        column,
        end_line,
        end_column,
        span: start..start + length,
    });
}

/// `.. code-block:: bash` and its `sourcecode` spelling.
fn directive_language(line: &str) -> Option<String> {
    let rest = line.strip_prefix(".. ")?;
    let (name, argument) = rest.split_once("::")?;
    matches!(name.trim(), "code-block" | "sourcecode" | "code")
        .then(|| argument.trim().to_lowercase())
}

/// ``inline literals``, which are reStructuredText's code spans.
fn inline_literals(line: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if &bytes[i..i + 2] != b"``" {
            i += 1;
            continue;
        }
        let Some(close) = line[i + 2..].find("``") else {
            break;
        };
        let text = &line[i + 2..i + 2 + close];
        if !text.trim().is_empty() {
            let leading = text.len() - text.trim_start().len();
            out.push((i + 2 + leading, text.trim().to_string()));
        }
        i += 2 + close + 2;
    }
    out
}

/// `` `text <target>`_ `` — reStructuredText's inline link.
fn link_targets(line: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut rest = line;
    let mut base = 0usize;
    while let Some(open) = rest.find(" <") {
        let after = &rest[open + 2..];
        let Some(close) = after.find(">`") else { break };
        let target = &after[..close];
        if !target.is_empty() {
            out.push((base + open + 2, target.to_string()));
        }
        base += open + 2 + close;
        rest = &rest[open + 2 + close..];
    }
    out
}

fn link_claim(dest: &str, start: usize, file: &Path, lines: &LineIndex<'_>) -> Option<Claim> {
    if dest.contains("://") || dest.starts_with("mailto:") || dest.starts_with('/') {
        return None;
    }
    let (target, anchor) = match dest.split_once('#') {
        Some((target, anchor)) if !anchor.is_empty() => {
            (target.to_string(), Some(anchor.to_string()))
        }
        _ => (dest.to_string(), None),
    };
    if target.is_empty() {
        return None;
    }
    let (line, column) = lines.locate(start);
    let (end_line, end_column) = lines.locate(start + dest.len());
    Some(Claim {
        kind: ClaimKind::Link { target, anchor },
        text: dest.to_string(),
        file: file.to_path_buf(),
        line,
        column,
        end_line,
        end_column,
        span: start..start + dest.len(),
    })
}

/// `.. versionadded:: 0.4` and its relatives.
fn is_historical(line: &str) -> bool {
    let Some(rest) = line.strip_prefix(".. ") else {
        return false;
    };
    let Some((name, _)) = rest.split_once("::") else {
        return false;
    };
    HISTORICAL_DIRECTIVES.contains(&name.trim())
}
