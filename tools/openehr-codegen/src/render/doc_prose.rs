// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The BMM documentation prose as rustdoc lines: the summary paragraph, the
//! fenced and indented blocks, and the prose rustdoc's lints accept.

pub(crate) fn doc_block_summarized(
    b: &mut String,
    doc: Option<&str>,
    indent: &str,
    summary_hint: &str,
) {
    let Some(doc) = doc else { return };
    let mut block = DocBlock::default();
    for line in doc.lines() {
        block.push_line(line.trim_end());
    }
    let mut out = block.finish();
    split_long_first_paragraph(&mut out, summary_hint);

    for line in &out {
        if line.is_empty() {
            b.push_str(&format!("{indent}///\n"));
        } else {
            b.push_str(&format!("{indent}/// {line}\n"));
        }
    }
}

/// The line-by-line rewriter behind [`doc_block_summarized`].
///
/// Spec prose carries example blocks (ODIN snippets, `YYYY-MM-DDTHH:MM:SS` date
/// formats) that rustdoc would compile as Rust doctests and choke on. Both forms
/// it recognizes are neutralized so the docs render as text, never run: a bare
/// triple-backtick fence has its opening tagged `text` (the closing stays bare),
/// and a run of 4-space-indented lines is wrapped in a `text` fence. Prose
/// OUTSIDE those blocks additionally goes through [`sanitize_doc_prose`] (bare
/// URLs, stray brackets/angle brackets — the rustdoc deny-lints).
#[derive(Default)]
struct DocBlock<'a> {
    /// The rewritten lines, in order.
    out: Vec<String>,
    /// Pending prose lines, sanitized as one segment so a code span may span
    /// lines (the BMM has such spans, e.g. `BMM_SCHEMA_DESCRIPTOR.schema_id`).
    prose: Vec<&'a str>,
    /// Inside an explicit triple-backtick fence.
    in_fence: bool,
    /// Inside an auto-wrapped indented block.
    in_indent: bool,
}

impl<'a> DocBlock<'a> {
    /// Sanitizes and emits the pending prose segment, if any.
    fn flush_prose(&mut self) {
        if self.prose.is_empty() {
            return;
        }
        let sanitized = sanitize_doc_prose(&self.prose.join("\n"));
        self.out.extend(sanitized.split('\n').map(str::to_string));
        self.prose.clear();
    }

    /// Takes one already-right-trimmed source line.
    fn push_line(&mut self, line: &'a str) {
        let stripped = line.trim_start();
        if stripped.starts_with("```") && !self.in_indent {
            self.toggle_fence(line, stripped);
            return;
        }
        if self.in_fence {
            self.out.push(line.to_string());
            return;
        }
        let lead = line.len() - stripped.len();
        self.track_indent_block(lead >= 4 && !stripped.is_empty(), stripped.is_empty());
        if self.in_indent {
            self.out.push(line.to_string());
        } else {
            self.prose.push(line);
        }
    }

    /// Opens or closes an explicit fence, tagging a bare opening as `text`.
    fn toggle_fence(&mut self, line: &str, stripped: &str) {
        self.flush_prose();
        if self.in_fence {
            self.in_fence = false;
            self.out.push(line.to_string());
            return;
        }
        self.in_fence = true;
        self.out.push(if stripped == "```" {
            line.replacen("```", "```text", 1)
        } else {
            line.to_string()
        });
    }

    /// Opens or closes the auto-wrapped fence around an indented run.
    fn track_indent_block(&mut self, is_indent_line: bool, is_blank: bool) {
        if is_indent_line && !self.in_indent {
            self.flush_prose();
            self.out.push("```text".to_string());
            self.in_indent = true;
        } else if self.in_indent && !is_indent_line && !is_blank {
            self.out.push("```".to_string());
            self.in_indent = false;
        }
    }

    /// Flushes the tail state and yields the rewritten lines.
    fn finish(mut self) -> Vec<String> {
        self.flush_prose();
        if self.in_indent {
            self.out.push("```".to_string());
        }
        if self.in_fence {
            self.out.push("```".to_string());
        }
        self.out
    }
}

/// Reshapes an over-long first doc paragraph into the RFC 1574 summary form.
///
/// When the first paragraph exceeds `clippy::too_long_first_doc_paragraph`'s
/// 200-character threshold, the first sentence stays as the summary and the
/// remainder moves below a blank line. BMM prose the split cannot apply to
/// (no sentence boundary inside the budget) is left verbatim — the lint then
/// cannot split within the budget gains the caller's synthesized
/// `summary_hint` as its summary paragraph instead (empty hint = verbatim).
fn split_long_first_paragraph(out: &mut Vec<String>, summary_hint: &str) {
    let para_end = out.iter().position(String::is_empty).unwrap_or(out.len());
    let text_len: usize = out
        .iter()
        .take(para_end)
        .map(|l| l.chars().count() + 1)
        .sum::<usize>()
        .saturating_sub(1);
    if text_len <= 200 {
        return;
    }
    let mut budget: usize = 200;
    for li in 0..para_end {
        let Some(line) = out.get(li).cloned() else {
            return;
        };
        if let Some(cut) = sentence_end_within(&line, budget) {
            let (head, tail) = line.split_at_checked(cut).unwrap_or((line.as_str(), ""));
            let head = head.trim_end().to_string();
            let tail = tail.trim_start().to_string();
            let replacement = if tail.is_empty() {
                vec![head, String::new()]
            } else {
                vec![head, String::new(), tail]
            };
            out.splice(li..=li, replacement);
            return;
        }
        // A line that ends the sentence at its very end splits between lines.
        let trimmed = line.trim_end();
        if trimmed.ends_with(['.', '!', '?']) && trimmed.chars().count() <= budget {
            out.insert(li + 1, String::new());
            return;
        }
        budget = budget.saturating_sub(line.chars().count() + 1);
        if budget == 0 {
            break;
        }
    }
    // No sentence boundary inside the budget: prepend the synthesized summary
    // when the caller supplied one; otherwise leave the prose verbatim.
    if !summary_hint.is_empty() {
        out.insert(0, String::new());
        out.insert(0, summary_hint.to_string());
    }
}

/// Returns the byte index just past the first sentence terminator that ends a
/// sentence within the first `budget` characters of `line`, if any.
///
/// A terminator is `.`/`!`/`?` followed by a space, excluding the common
/// abbreviations spec prose uses (`e.g.`, `i.e.`, `etc.`, `vs.`, `cf.`).
fn sentence_end_within(line: &str, budget: usize) -> Option<usize> {
    let mut chars_seen: usize = 0;
    let mut prev_word_end: Option<usize> = None;
    let mut iter = line.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        chars_seen += 1;
        if chars_seen > budget {
            return None;
        }
        if matches!(c, '.' | '!' | '?') && iter.peek().is_some_and(|&(_, n)| n == ' ') {
            let word_start = prev_word_end.map_or(0, |w| w + 1);
            let word = line.get(word_start..i).unwrap_or_default();
            let abbrev = matches!(
                word.trim_start_matches('(').to_ascii_lowercase().as_str(),
                "e.g" | "i.e" | "etc" | "vs" | "cf" | "viz" | "resp" | "incl"
            );
            if !abbrev {
                return Some(i + c.len_utf8());
            }
        }
        if c == ' ' {
            prev_word_end = Some(i);
        }
    }
    None
}

/// Make verbatim openEHR spec prose safe for the workspace's deny-level rustdoc
/// lints, leaving code spans untouched (rustdoc never lints inside them):
///
/// - a bare URL becomes an autolink `<url>`, and the asciidoc `url[label]` form
///   becomes a Markdown link (`rustdoc::bare_urls`);
/// - a literal `[…]` is escaped `\[…\]`, so prose is not read as a broken
///   intra-doc link (`rustdoc::broken_intra_doc_links`);
/// - a literal `<…>` is escaped `\<…\>`, so `"name <email>"` / `VERSION<T>` in
///   prose is not read as an HTML tag (`rustdoc::invalid_html_tags`);
/// - an unpaired backtick run is escaped, so it opens no dangling code span
///   (`rustdoc::unescaped_backticks`).
pub(crate) fn sanitize_doc_prose(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        match c {
            '`' => rest = copy_code_span(rest, &mut out),
            'h' if rest.starts_with("http://") || rest.starts_with("https://") => {
                rest = copy_url(rest, &mut out);
            }
            '[' | ']' | '<' | '>' => {
                out.push('\\');
                out.push(c);
                rest = after(rest, c.len_utf8());
            }
            c => {
                out.push(c);
                rest = after(rest, c.len_utf8());
            }
        }
    }
    out
}

/// Copies a backtick run to `out`, returning the unconsumed remainder.
///
/// A matched pair delimits a code span: it is copied verbatim, closing run
/// included. An unmatched run is escaped instead.
fn copy_code_span<'a>(rest: &'a str, out: &mut String) -> &'a str {
    let open = backtick_run(rest);
    if let Some(offset) = find_backtick_run(after(rest, open), open) {
        let end = open + offset + open;
        out.push_str(upto(rest, end));
        return after(rest, end);
    }
    for _ in 0..open {
        out.push_str("\\`");
    }
    after(rest, open)
}

/// Copies a URL to `out` as Markdown, returning the unconsumed remainder.
///
/// The asciidoc link form `https://host/path[label]` becomes a Markdown link. A
/// parenthesis in the URL would end the Markdown destination early, so such a
/// link stays an autolink with escaped brackets.
fn copy_url<'a>(rest: &'a str, out: &mut String) -> &'a str {
    let url = read_url(rest);
    let tail = after(rest, url.len());
    let plain_dest = !url.contains(['(', ')']);
    if let Some((consumed, label)) = read_link_label(tail).filter(|_| plain_dest) {
        out.push_str(&format!("[{label}]({url})"));
        return after(tail, consumed);
    }
    out.push_str(&format!("<{url}>"));
    tail
}

/// `s` after its first `n` bytes — total (empty when `n` is out of range or not
/// on a char boundary), so no slicing can panic.
fn after(s: &str, n: usize) -> &str {
    s.get(n..).unwrap_or_default()
}

/// The first `n` bytes of `s` — total, like [`after`].
fn upto(s: &str, n: usize) -> &str {
    s.get(..n).unwrap_or_default()
}

/// The length of the backtick run at the start of `s`.
fn backtick_run(s: &str) -> usize {
    s.chars().take_while(|&c| c == '`').count()
}

/// The byte offset in `s` of the next backtick run of exactly `n`, if any.
fn find_backtick_run(s: &str, n: usize) -> Option<usize> {
    let mut rest = s;
    let mut base = 0;
    while let Some(idx) = rest.find('`') {
        let run = backtick_run(after(rest, idx));
        if run == n {
            return Some(base + idx);
        }
        base += idx + run;
        rest = after(rest, idx + run);
    }
    None
}

/// The URL at the start of `s`: up to whitespace or a delimiter, with trailing
/// sentence punctuation left outside the link.
fn read_url(s: &str) -> &str {
    let len = s
        .chars()
        .take_while(|&c| {
            !c.is_whitespace() && !matches!(c, '`' | '[' | ']' | '<' | '>' | '"' | '\'')
        })
        .map(char::len_utf8)
        .sum();
    let mut url = upto(s, len);
    while let Some(trimmed) = url.strip_suffix(['.', ',', ';', ':', ')']) {
        url = trimmed;
    }
    url
}

/// An asciidoc link label `[label]` at the start of `s`, if it is usable as
/// Markdown link text (no nested bracket or code span). Returns
/// `(bytes consumed incl. both brackets, label)`.
fn read_link_label(s: &str) -> Option<(usize, &str)> {
    let body = s.strip_prefix('[')?;
    let close = body.find(']')?;
    let label = upto(body, close);
    if label.is_empty() || label.contains(['[', '`']) {
        return None;
    }
    Some((close + 2, label))
}

#[cfg(test)]
mod tests {
    use super::{doc_block_summarized, sanitize_doc_prose};

    #[test]
    fn bare_url_becomes_an_autolink() {
        assert_eq!(
            sanitize_doc_prose("specified by https://www.rfc-editor.org/rfc/rfc1034."),
            "specified by <https://www.rfc-editor.org/rfc/rfc1034>.",
        );
    }

    #[test]
    fn asciidoc_link_becomes_a_markdown_link() {
        assert_eq!(
            sanitize_doc_prose("Coded using https://example.org/t.xml[openEHR vocabulary]."),
            "Coded using [openEHR vocabulary](https://example.org/t.xml).",
        );
    }

    #[test]
    fn prose_brackets_and_angle_brackets_are_escaped() {
        assert_eq!(
            sanitize_doc_prose(r#"in "name <email>" form, table [ [ String ] ]"#),
            r#"in "name \<email\>" form, table \[ \[ String \] \]"#,
        );
    }

    #[test]
    fn code_spans_are_copied_verbatim() {
        // rustdoc lints nothing inside a code span, so `Interval<T>` and a URL in
        // backticks must survive untouched — including a span crossing lines.
        assert_eq!(
            sanitize_doc_prose("substitutable for `Interval<T>` where needed"),
            "substitutable for `Interval<T>` where needed",
        );
        assert_eq!(
            sanitize_doc_prose("formed by\n\n`create_schema_id(\n  publisher)`\n\ne.g. x"),
            "formed by\n\n`create_schema_id(\n  publisher)`\n\ne.g. x",
        );
    }

    #[test]
    fn an_unpaired_backtick_is_escaped() {
        assert_eq!(
            sanitize_doc_prose("the property `tuple_constraint', and comes"),
            "the property \\`tuple_constraint', and comes",
        );
    }

    #[test]
    fn fenced_and_indented_blocks_are_left_alone() {
        let mut out = String::new();
        doc_block_summarized(
            &mut out,
            Some("Values:\n\n```\n\"YYYY-MM-DD\" -- [full]\n```"),
            "",
            "",
        );
        assert_eq!(
            out,
            "/// Values:\n///\n/// ```text\n/// \"YYYY-MM-DD\" -- [full]\n/// ```\n",
        );
    }
}
