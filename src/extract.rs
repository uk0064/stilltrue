//! Stage 1: parse Markdown into typed claims with exact spans.

use std::path::Path;

use tree_sitter::{Node, Tree};
use tree_sitter_md::MarkdownParser;

use crate::claim::{Claim, ClaimKind};

/// Runners whose commands we know how to check. An unrecognised runner yields no
/// claim at all, which is the point: we only speak about what we can verify.
const RUNNERS: &[&str] = &[
    "make", "npm", "pnpm", "yarn", "cargo", "go", "uv", "uvx", "just", "docker", "python",
    // `deno task` names a task the config must define. `poetry run` and `uv run`
    // deliberately stay out: they run an arbitrary binary from the environment far more
    // often than a declared script, so checking them would be a false-positive machine.
    "deno",
];

/// Extract every claim a document makes.
///
/// Returns an empty vector if the document cannot be parsed; a linter never fails a
/// build because it choked.
pub fn claims(source: &str, file: &Path) -> Vec<Claim> {
    extract(source, file).claims
}

/// A suppression marker that suppressed nothing.
#[derive(Debug, Clone)]
pub struct UnusedSuppression {
    /// 1-based, like a claim's.
    pub line: usize,
    pub column: usize,
}

/// Everything a document yields: its claims, and the markers that turned out not to be
/// covering anything.
#[derive(Default)]
pub struct Extraction {
    pub claims: Vec<Claim>,
    pub unused_suppressions: Vec<UnusedSuppression>,
    /// Suppression markers found, a whole-file marker included. What a marker covers
    /// was never extracted, so this is the only honest count of it (ADR-0019).
    pub markers: usize,
    /// A whole-file marker removed this document from extraction.
    pub whole_file: bool,
    /// The parser produced no tree, so nothing could be extracted — which is not the
    /// same as a document that makes no claims.
    pub unparseable: bool,
}

pub fn extract(source: &str, file: &Path) -> Extraction {
    let mut parser = MarkdownParser::default();
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Extraction {
            unparseable: true,
            ..Extraction::default()
        };
    };

    let lines = LineIndex::new(source);
    let mut claims = Vec::new();

    // Inline content only: paragraphs, headings, list items, table cells. Fenced code
    // content is not `inline`, so it has no inline tree — which is precisely how
    // ADR-0002 ("no claims inside non-shell fences") is enforced, structurally rather
    // than by a filter that could be forgotten.
    for inline_tree in tree.inline_trees() {
        collect_inline(inline_tree, source, file, &lines, &mut claims);
    }

    collect_shell_fences(&tree, source, file, &lines, &mut claims);
    collect_versions(source, file, &lines, &mut claims);

    let mut unused_suppressions = Vec::new();
    let (suppression, markers) = suppressions(&tree, source);
    match suppression {
        Suppression::WholeFile => {
            return Extraction {
                markers,
                whole_file: true,
                ..Extraction::default()
            };
        }
        Suppression::Blocks(blocks) => {
            for block in &blocks {
                // A marker covering nothing is worth saying so — but only where the
                // author asked for the noisier tier (ADR-0016).
                if !claims.iter().any(|c| block.range.contains(&c.span.start)) {
                    let (line, column) = lines.locate(block.marker);
                    unused_suppressions.push(UnusedSuppression { line, column });
                }
            }
            claims.retain(|c| {
                !blocks
                    .iter()
                    .any(|block| block.range.contains(&c.span.start))
            });
        }
    }

    claims.sort_by_key(|c| c.span.start);
    Extraction {
        claims,
        unused_suppressions,
        markers,
        ..Extraction::default()
    }
}

fn collect_inline(
    tree: &Tree,
    source: &str,
    file: &Path,
    lines: &LineIndex<'_>,
    out: &mut Vec<Claim>,
) {
    walk(tree.root_node(), &mut |node| match node.kind() {
        "code_span" => {
            if let Some(claim) = code_span_claim(node, source, file, lines) {
                out.push(claim);
            }
        }
        "link_destination" => {
            if let Some(claim) = link_claim(node, source, file, lines) {
                out.push(claim);
            }
        }
        _ => {}
    });
}

/// Tools whose versions we recognise in prose, with the capitalisation documents
/// actually use.
const VERSION_TOOLS: &[&str] = &[
    "Node.js", "Node", "Python", "Rust", "Go", "pnpm", "npm", "yarn", "uv",
];

/// `Version` is the one claim type read from prose and from every fence.
pub(crate) fn collect_versions(
    source: &str,
    file: &Path,
    lines: &LineIndex<'_>,
    out: &mut Vec<Claim>,
) {
    for tool in VERSION_TOOLS {
        let mut from = 0usize;
        while let Some(found) = source[from..].find(tool) {
            let start = from + found;
            let end = start + tool.len();
            from = end;

            let preceded_by_word =
                start > 0 && source.as_bytes()[start - 1].is_ascii_alphanumeric();
            if preceded_by_word {
                continue;
            }
            // `Node` must not swallow the `Node` inside `Node.js`.
            if source[end..].starts_with(".js") {
                continue;
            }
            let rest = &source[end..];
            let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            if spaces == 0 {
                continue;
            }
            let rest = &rest[spaces..];
            let prefix = usize::from(rest.starts_with('v'));
            let rest = &rest[prefix..];
            let digits = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            let version = rest[..digits].trim_end_matches('.');
            if version.is_empty() || !version.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            // `2.x` and `18.*` name a family, not a version, and a family cannot
            // contradict a pin — the same reasoning as the placeholder glob clause,
            // where `docs/**` is not a claim that `docs/**` exists. The wildcard has to
            // stand where a number would, so `3.10` keeps its meaning and only a
            // trailing component is read this way.
            if rest[..digits].ends_with('.') && rest[digits..].starts_with(['x', 'X', '*']) {
                continue;
            }
            // `Rust 2024` is the 2024 edition — a revision of the language, which any
            // current toolchain supports — not a compiler version, so a toolchain pin
            // cannot contradict it (ADR-0024). An edition is a bare year; a shape rather
            // than a list, so the next one needs no change here.
            if *tool == "Rust"
                && version.len() == 4
                && version.starts_with("20")
                && version.bytes().all(|b| b.is_ascii_digit())
            {
                continue;
            }

            let (line, column) = lines.locate(start);
            // The claim is "Rust 1.98", not "Rust": the end position drives SARIF's
            // endColumn and the Action's annotation, so it has to cover the version
            // the claim is actually about.
            let claim_end = end + spaces + prefix + version.len();
            let (end_line, end_column) = lines.locate(claim_end);
            let text = source[start..end].to_string();
            out.push(Claim {
                kind: ClaimKind::Version {
                    tool: text.clone(),
                    version: version.to_string(),
                },
                text: format!("{text} {version}"),
                file: file.to_path_buf(),
                line,
                column,
                end_line,
                end_column,
                span: start..claim_end,
            });
        }
    }
}

/// What an author has asked us not to judge.
enum Suppression {
    WholeFile,
    Blocks(Vec<SuppressedBlock>),
}

/// A marker and the block it covers, kept together so an unused one can be located.
struct SuppressedBlock {
    /// Byte offset of the marker itself, for reporting.
    marker: usize,
    range: std::ops::Range<usize>,
}

/// Suppression is block-anchored: the marker suppresses every claim in the next
/// block-level node, which is what makes a claim inside a fence suppressible at all.
fn suppressions(tree: &tree_sitter_md::MarkdownTree, source: &str) -> (Suppression, usize) {
    let mut ranges = Vec::new();
    let mut whole_file = false;
    let mut markers = 0;

    walk(tree.block_tree().root_node(), &mut |node| {
        if node.kind() != "html_block" {
            return;
        }
        let text = &source[node.byte_range()];
        if !text.contains("stilltrue:ignore") {
            return;
        }
        markers += 1;
        if text.contains("stilltrue:ignore-file") {
            whole_file = true;
            return;
        }
        let mut next = node.next_named_sibling();
        // An HTML block swallows the blank line after it, so the marker's own node may
        // already be the last child of its section; step out to find the next block.
        if next.is_none() {
            next = node.parent().and_then(|p| p.next_named_sibling());
        }
        // Stepping out can land on the next *section*, which holds a heading and every
        // block beneath it. Suppression is one block wide — taking the section would
        // let a single marker silently disable the rest of the document.
        while let Some(block) = next
            && block.kind() == "section"
        {
            next = block.named_child(0);
        }
        if let Some(block) = next {
            ranges.push(SuppressedBlock {
                marker: node.start_byte(),
                range: block.byte_range(),
            });
        }
    });

    let suppression = if whole_file {
        Suppression::WholeFile
    } else {
        Suppression::Blocks(ranges)
    };
    (suppression, markers)
}

/// Fence info strings that mark a block as shell. Only these produce claims, and only
/// `Command` claims (ADR-0002).
const SHELL_TAGS: &[&str] = &["bash", "sh", "shell", "console", "zsh"];

fn collect_shell_fences(
    tree: &tree_sitter_md::MarkdownTree,
    source: &str,
    file: &Path,
    lines: &LineIndex<'_>,
    out: &mut Vec<Claim>,
) {
    walk(tree.block_tree().root_node(), &mut |node| {
        if node.kind() != "fenced_code_block" {
            return;
        }
        let Some(info) = child_of_kind(node, "info_string") else {
            return;
        };
        let tag = source[info.byte_range()].trim().to_ascii_lowercase();
        let tag = tag.split_whitespace().next().unwrap_or_default();
        if !SHELL_TAGS.contains(&tag) {
            return;
        }
        let Some(content) = child_of_kind(node, "code_fence_content") else {
            return;
        };
        collect_commands(
            &source[content.byte_range()],
            content.start_byte(),
            file,
            lines,
            out,
        );
    });
}

fn child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();

    node.children(&mut cursor).find(|c| c.kind() == kind)
}

/// Turn shell fence content into `Command` claims. Deliberately shallow — never a real
/// shell parse.
fn collect_commands(
    content: &str,
    content_start: usize,
    file: &Path,
    lines: &LineIndex<'_>,
    out: &mut Vec<Claim>,
) {
    let mut cursor = 0usize;
    for logical in logical_lines(content) {
        for segment in logical.split(['\n']).flat_map(split_segments) {
            let Some((text, runner, args)) = command_of(&segment) else {
                continue;
            };
            // Anchor at the exact text when the segment survives normalisation
            // unchanged; otherwise at the runner token, which always exists verbatim.
            let (offset, len) = match content[cursor..].find(&text) {
                Some(i) => (content_start + cursor + i, text.len()),
                None => match content[cursor..].find(&runner) {
                    Some(i) => (content_start + cursor + i, runner.len()),
                    None => continue,
                },
            };
            cursor = (offset - content_start) + len;
            let (line, column) = lines.locate(offset);
            let (end_line, end_column) = lines.locate(offset + len);
            out.push(Claim {
                kind: ClaimKind::Command { runner, args },
                text,
                file: file.to_path_buf(),
                line,
                column,
                end_line,
                end_column,
                span: offset..offset + len,
            });
        }
    }
}

/// Drop comment lines and join `\` continuations.
fn logical_lines(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let (body, continues) = match trimmed.strip_suffix('\\') {
            Some(head) => (head.trim_end(), true),
            None => (trimmed, false),
        };
        let joined = match pending.take() {
            Some(mut acc) => {
                acc.push(' ');
                acc.push_str(body.trim_start());
                acc
            }
            None => body.to_string(),
        };
        if continues {
            pending = Some(joined);
        } else if !joined.is_empty() {
            out.push(joined);
        }
    }
    if let Some(last) = pending
        && !last.is_empty()
    {
        out.push(last);
    }
    out
}

/// Split on `;`, `&&`, `||`, and `|`.
pub(crate) fn split_segments(line: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let two = line.get(i..i + 2);
        if two == Some("&&") || two == Some("||") {
            segments.push(std::mem::take(&mut current));
            i += 2;
            continue;
        }
        if bytes[i] == b';' || bytes[i] == b'|' {
            segments.push(std::mem::take(&mut current));
            i += 1;
            continue;
        }
        current.push(bytes[i] as char);
        i += 1;
    }
    segments.push(current);
    segments
}

/// Strip leading `VAR=value` assignments and `sudo`, then recognise the runner.
/// Returns the normalised command text, the runner, and its arguments.
pub(crate) fn command_of(segment: &str) -> Option<(String, String, Vec<String>)> {
    let segment = segment.trim();
    let mut tokens = segment.split_whitespace().peekable();

    // A transcript writes the prompt as part of the line. `$` on its own is a prompt,
    // not runtime-computed text, and treating the two alike loses most of the commands
    // anyone actually writes down.
    if tokens
        .peek()
        .is_some_and(|t| *t == "$" || *t == "%" || *t == ">")
    {
        tokens.next();
    }
    while let Some(token) = tokens.peek() {
        let is_assignment = token
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'));
        if is_assignment || *token == "sudo" {
            tokens.next();
        } else {
            break;
        }
    }
    let rest: Vec<String> = tokens.map(str::to_string).collect();
    let runner = rest.first()?.clone();
    if !RUNNERS.contains(&runner.as_str()) {
        return None;
    }
    // A `$` or a backtick means no claim, judged on the runner and its target rather
    // than the whole segment: a name computed at runtime cannot be provably broken, but
    // a computed *flag* beside a real target says nothing about the target.
    //
    // Which token is the target depends on the runner — `npm run build` names it
    // third, because the second is the literal `run`. This has to agree with
    // `resolve::command::target_of`, or the guard reads a word the resolver ignores.
    let target = match (runner.as_str(), rest.get(1).map(String::as_str)) {
        ("npm" | "pnpm" | "yarn", Some("run")) => rest.get(2),
        _ => rest.get(1),
    };
    let computed = |t: &str| t.contains('$') || t.contains('`');
    if computed(&runner) || target.is_some_and(|t| computed(t)) {
        return None;
    }
    Some((rest.join(" "), runner, rest[1..].to_vec()))
}

fn code_span_claim(node: Node, source: &str, file: &Path, lines: &LineIndex<'_>) -> Option<Claim> {
    let raw = &source[node.byte_range()];
    let ticks = raw.chars().take_while(|c| *c == '`').count();
    // A `code_span` node always opens with a backtick, so this only fires if the
    // grammar hands back something else entirely.
    if ticks == 0 {
        return None;
    }
    // `ticks` never exceeds `raw.len()`, so `end_byte - ticks` cannot underflow; and
    // when a span is too short to hold both delimiters the range inverts, which `get`
    // reports as None. An explicit length check here would be a second spelling of the
    // same condition — one that no input can reach independently, and that every
    // mutation run would report as untested for the rest of the project's life.
    let inner_start = node.start_byte() + ticks;
    let inner_end = node.end_byte() - ticks;
    let raw_text = source.get(inner_start..inner_end)?.trim();
    if raw_text.is_empty() {
        return None;
    }

    // `trim` may have moved the start; recover the true offset of the first character.
    let offset = inner_start + source[inner_start..inner_end].find(raw_text)?;
    let text = unwrap_code_span(raw_text);
    let kind = classify(&text)?;
    let (line, column) = lines.locate(offset);
    let (end_line, end_column) = lines.locate(offset + raw_text.len());

    Some(Claim {
        kind,
        text,
        file: file.to_path_buf(),
        line,
        column,
        end_line,
        end_column,
        // The span stays in source terms, where the name may still be wrapped.
        span: offset..offset + raw_text.len(),
    })
}

/// CommonMark converts a line ending inside a code span to a space, so a span the
/// author wrapped is still one name. ripgrep's README wraps `cargo binstall` across two
/// lines; keeping the newline corrupts the claim and puts a line break in the middle of
/// a finding, breaking the one-finding-per-line contract of human output.
fn unwrap_code_span(text: &str) -> String {
    if !text.contains('\n') {
        return text.to_string();
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A Markdown link destination. Relative targets only: external URLs are `lychee`'s
/// job, and a bare `#anchor` addresses the document itself rather than the
/// repository.
fn link_claim(node: Node, source: &str, file: &Path, lines: &LineIndex<'_>) -> Option<Claim> {
    let raw = source.get(node.byte_range())?.trim();
    let dest = raw.trim_start_matches('<').trim_end_matches('>');
    if dest.is_empty()
        || dest.contains("://")
        || dest.starts_with("mailto:")
        || dest.starts_with('/')
    {
        return None;
    }

    // A bare `#anchor` addresses this document. It is still a claim — a table of
    // contents is the commonest link shape in a README, and every entry in one points
    // here. `#` alone is a link to the top of the page and names nothing.
    let dest = match dest.strip_prefix('#') {
        Some("") => return None,
        Some(anchor) => {
            let target = file.file_name()?.to_string_lossy().into_owned();
            let offset = node.start_byte() + source[node.byte_range()].find(dest)?;
            let (line, column) = lines.locate(offset);
            let (end_line, end_column) = lines.locate(offset + dest.len());
            return Some(Claim {
                kind: ClaimKind::Link {
                    target,
                    anchor: Some(anchor.to_string()),
                },
                text: dest.to_string(),
                file: file.to_path_buf(),
                line,
                column,
                end_line,
                end_column,
                span: offset..offset + dest.len(),
            });
        }
        None => dest,
    };

    let (target, anchor) = match dest.split_once('#') {
        Some((t, a)) if !a.is_empty() => (t.to_string(), Some(a.to_string())),
        Some((t, _)) => (t.to_string(), None),
        None => (dest.to_string(), None),
    };
    if target.is_empty() {
        return None;
    }

    let offset = node.start_byte() + source[node.byte_range()].find(dest)?;
    let (line, column) = lines.locate(offset);
    let (end_line, end_column) = lines.locate(offset + dest.len());
    Some(Claim {
        kind: ClaimKind::Link { target, anchor },
        text: dest.to_string(),
        file: file.to_path_buf(),
        line,
        column,
        end_line,
        end_column,
        span: offset..offset + dest.len(),
    })
}

/// File extensions that make a dotless-slash span a `Path`. Bounded on purpose: a
/// bare "contains a dot" rule would misclassify most dotted symbols.
const PATH_EXTENSIONS: &[&str] = &[
    "md", "toml", "json", "yaml", "yml", "py", "ts", "rs", "go", "sh", "txt", "lock", "cfg", "ini",
    "rst",
];

/// Classify an inline code span. Precedence order, first match wins.
/// `None` means Unclassified — it never becomes a claim.
pub fn classify(text: &str) -> Option<ClaimKind> {
    if is_placeholder(text) {
        return None;
    }

    let single_token = !text.contains(char::is_whitespace);
    // A path needs something to name; `/` and `::` are punctuation being discussed,
    // not files.
    let names_something = text.chars().any(|c| c.is_alphanumeric());

    // 1. Path.
    if single_token && names_something && (text.contains('/') || has_path_extension(text)) {
        if is_bare_reference(text) {
            return None;
        }
        return Some(ClaimKind::Path);
    }

    // 2. Command.
    let mut tokens = text.split_whitespace();
    let head = tokens.next()?;
    if RUNNERS.contains(&head) {
        return Some(ClaimKind::Command {
            runner: head.to_string(),
            args: tokens.map(str::to_string).collect(),
        });
    }

    // Anything else multi-token is Unclassified; the remaining rules describe
    // single identifiers.
    if !single_token {
        return None;
    }

    // 3. EnvVar.
    if is_env_var(text) {
        return Some(ClaimKind::EnvVar);
    }

    // 4. Symbol.
    if is_symbol(text) {
        return Some(ClaimKind::Symbol);
    }

    // 5. Unclassified.
    None
}

/// Placeholder spans are Unclassified before any other rule runs, because
/// `packages/<name>/index.ts` contains a `/` and would otherwise be a `Path`.
///
/// Deliberately mechanical: there is no metasyntactic list, because `foo`, `bar`, and
/// `baz` are real identifiers in real repositories.
fn is_placeholder(text: &str) -> bool {
    if text.contains('<') || text.contains('>') || text.contains("...") || text.contains('\u{2026}')
    {
        return true;
    }
    if text.contains("{{") || (text.contains('{') && text.contains('}')) {
        return true;
    }
    // External URLs are somebody else's problem — that is `lychee`.
    if text.contains("://") {
        return true;
    }
    // Glob patterns describe a set of paths, not a path. `docs/**` is not a claim
    // that `docs/**` exists.
    if text.contains('*') || text.contains('?') || (text.contains('[') && text.contains(']')) {
        return true;
    }
    text.split(['/', ' ']).any(|segment| {
        // `cargo mutants` reports `==` → `!=` here as surviving, and it always will:
        // any text containing "path/to" has a "to" segment that is not "path", so
        // `any` finds a match either way. The mutant is equivalent, not a missing test.
        segment == "path" && text.contains("path/to")
            || segment.starts_with("your-")
            || segment.starts_with("YOUR_")
            || segment.starts_with("my-")
    })
}

/// A slash-separated reference with no extension and no positional marker —
/// `actions/checkout`, `@scope/pkg`, `stilltrue/command/rot`. These have the shape of a
/// path and none of the substance, and real documents are full of them.
///
/// The escape hatch is to say you mean a path: a trailing `/`, or a leading `./`,
/// `../`, or `/`.
fn is_bare_reference(text: &str) -> bool {
    if text.starts_with("./") || text.starts_with("../") || text.starts_with('/') {
        return false;
    }
    if text.ends_with('/') {
        return false;
    }
    text.contains('/') && !has_path_extension(text)
}

fn has_path_extension(text: &str) -> bool {
    match text.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && PATH_EXTENSIONS.contains(&ext),
        None => false,
    }
}

/// An env var reference. The underscore is required: `^[A-Z][A-Z0-9_]{2,}$` alone also
/// matches `README`, `TODO`, `API`, and `JSON`, and an all-caps word with no underscore
/// is genuinely ambiguous between an acronym and a variable. `PATH` and `CI` are lost;
/// when in doubt, stay silent.
pub fn is_env_var(text: &str) -> bool {
    text.contains('_')
        && text.starts_with(|c: char| c.is_ascii_uppercase())
        && text.len() >= 3
        && text
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_symbol(text: &str) -> bool {
    if !is_identifier_shaped(text) {
        return false;
    }
    if let Some(stem) = text.strip_suffix("()") {
        return !stem.is_empty();
    }
    if has_qualified_separator(text) {
        return true;
    }
    // A bare `download_file` is a word with an underscore in it, not a reference to
    // code. Requiring a call or a namespace is what separates naming a function from
    // explaining one — see ADR-0017, which this rule cost six false positives to learn.
    false
}

/// A `.` or `::` sitting between two identifier characters.
fn has_qualified_separator(text: &str) -> bool {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    for (i, &b) in bytes.iter().enumerate() {
        if i == 0 || i + 1 >= bytes.len() {
            continue;
        }
        if b == b'.' && ident(bytes[i - 1]) && ident(bytes[i + 1]) {
            return true;
        }
        if b == b':'
            && bytes.get(i + 1) == Some(&b':')
            && ident(bytes[i - 1])
            && bytes.get(i + 2).is_some_and(|&n| ident(n))
        {
            return true;
        }
    }
    false
}

/// Every segment of a name has to be spellable as an identifier. poetry documents
/// `project.requires-python`, a `pyproject.toml` table key: a hyphen cannot appear in a
/// Python name, so that was never a symbol claim.
fn is_identifier_shaped(text: &str) -> bool {
    let text = text.strip_suffix("()").unwrap_or(text);
    let segments = text.split(['.', ':']).filter(|s| !s.is_empty());
    let mut any = false;
    for segment in segments {
        any = true;
        if segment.starts_with(|c: char| c.is_ascii_digit()) {
            return false;
        }
        if !segment.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return false;
        }
    }
    any
}

/// Depth-first walk over a tree-sitter tree.
fn walk<'a>(node: Node<'a>, f: &mut impl FnMut(Node<'a>)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, f);
    }
}

/// Byte offset → 1-based line and column, computed once per document.
pub(crate) struct LineIndex<'a> {
    source: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Self { source, starts }
    }

    /// Line and column, both 1-based, with the column counted in **Unicode code
    /// points**.
    ///
    /// Not bytes. A byte column misplaces the annotation on every line containing
    /// non-ASCII, and it does so identically in human output, in SARIF, and in the
    /// Action's `::error col=` — so the three agree with each other and are wrong
    /// together. SARIF permits only `utf16CodeUnits` or `unicodeCodePoints`
    /// (§3.14.27); bytes is not a legal unit, and `report::sarif` declares which of
    /// the two this is.
    pub(crate) fn locate(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        let start = self.starts[line - 1];
        let column = self.source[start..offset].chars().count() + 1;
        (line, column)
    }
}
