// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Injects per-page search metadata into an assembled documentation site.
//!
//! mdBook emits ONE `<meta name="description">` for a whole book and no
//! canonical link at all, so without this pass `/docs/dev/`, `/docs/latest/`
//! and every frozen `/docs/vX.Y.Z/` are indexable copies of the same page
//! sharing one snippet: search engines split ranking signals across the copies
//! and write the snippet themselves.
//!
//! This runs over the BUILT HTML rather than the mdBook theme on purpose. The
//! Handlebars context carries only a chapter's SOURCE path, so a template
//! cannot form the absolute canonical URL, and a canonical injected by
//! JavaScript is not reliably honoured by crawlers.
//!
//! Per page it writes a canonical URL pointing at the canonical version, a
//! description taken from that page's own opening prose, Open Graph and Twitter
//! card tags, `noindex,follow` on every non-canonical version, and a
//! `BreadcrumbList` mirroring the URL hierarchy with real labels.
//!
//! Idempotent: a page already carrying [`MARKER`] is skipped, so re-running
//! over an assembled site changes nothing.
//!
//! # Examples
//!
//! ```text
//! docs-meta _site https://ferroehr.eu ""
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "build-time tool, never shipped in the server: the console IS its \
              user interface (it reports how many pages it stamped, and a bad \
              invocation must say so)"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The first-line marker that makes the pass idempotent.
const MARKER: &str = "<!-- ferroehr:seo -->";
/// The version whose URL every copy of a page points at.
const CANONICAL_VERSION: &str = "latest";
/// The `og:site_name` and breadcrumb root label.
const SITE_NAME: &str = "FerroEHR";
/// The social card, relative to the site root.
const SOCIAL_CARD: &str = "/assets/social-card.png";
/// Search engines truncate a description near 160 characters; allow a little
/// more so the sentence still reads, then cut on a word boundary.
const DESCRIPTION_LIMIT: usize = 300;
/// Below this length an opening paragraph is a caption, not a description.
const DESCRIPTION_MINIMUM: usize = 60;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [site, origin, base] = args.as_slice() else {
        eprintln!("usage: docs-meta <site-dir> <origin> <base-path>");
        return ExitCode::from(2);
    };
    match run(Path::new(site), origin, base) {
        Ok(count) => {
            println!("  wrote search metadata into {count} page(s)");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("docs-meta: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Stamp every book page under `site/docs/<version>/`, returning how many were
/// written.
///
/// # Errors
/// Returns an error when the site tree cannot be read or a page cannot be
/// rewritten.
fn run(site: &Path, origin: &str, base: &str) -> std::io::Result<usize> {
    let docs = site.join("docs");
    if !docs.is_dir() {
        println!("  no docs tree — skipped");
        return Ok(0);
    }
    let mut versions: Vec<PathBuf> = fs::read_dir(&docs)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();

    let mut written = 0usize;
    for version_dir in versions {
        let Some(version) = version_dir.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        let mut pages = Vec::new();
        collect_html(&version_dir, &mut pages)?;
        pages.sort();
        for page in pages {
            let Ok(relative) = page.strip_prefix(&version_dir) else {
                continue;
            };
            let rel = to_slash(relative);
            if rel == "404.html" || rel.starts_with("theme/") {
                continue;
            }
            let body = fs::read_to_string(&page)?;
            if body.contains(MARKER) {
                continue;
            }
            let Some((before_head_end, after_head_end)) = body.split_once("</head>") else {
                continue;
            };
            let block = meta_tags(origin, base, version, &rel, &body);
            let mut out = String::with_capacity(body.len() + block.len() + 16);
            out.push_str(before_head_end);
            out.push_str("  ");
            out.push_str(&block);
            out.push('\n');
            out.push_str("</head>");
            out.push_str(after_head_end);
            fs::write(&page, out)?;
            written += 1;
        }
    }
    Ok(written)
}

/// Collect every `.html` file under `dir`, recursively.
fn collect_html(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_html(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "html") {
            out.push(path);
        }
    }
    Ok(())
}

/// The metadata block for one page, marker first.
fn meta_tags(origin: &str, base: &str, version: &str, rel: &str, page: &str) -> String {
    let title = page_title(page);
    let description = first_paragraph(page);
    let canonical = format!("{origin}{base}/docs/{CANONICAL_VERSION}/{rel}");
    let card = format!("{origin}{base}{SOCIAL_CARD}");
    let mut tags = vec![
        MARKER.to_owned(),
        format!(r#"<link rel="canonical" href="{}">"#, escape(&canonical)),
    ];
    if let Some(text) = description {
        let value = escape(&text);
        tags.push(format!(r#"<meta name="description" content="{value}">"#));
        tags.push(format!(
            r#"<meta property="og:description" content="{value}">"#
        ));
    }
    if let Some(text) = &title {
        tags.push(format!(
            r#"<meta property="og:title" content="{}">"#,
            escape(text)
        ));
    }
    tags.push(r#"<meta property="og:type" content="article">"#.to_owned());
    tags.push(format!(
        r#"<meta property="og:site_name" content="{SITE_NAME}">"#
    ));
    tags.push(format!(
        r#"<meta property="og:url" content="{}">"#,
        escape(&canonical)
    ));
    tags.push(format!(r#"<meta property="og:image" content="{card}">"#));
    tags.push(r#"<meta name="twitter:card" content="summary_large_image">"#.to_owned());
    tags.push(format!(r#"<meta name="twitter:image" content="{card}">"#));
    // A non-canonical version must not compete in the index. robots.txt already
    // disallows crawling those trees; this states it in-page for a crawler that
    // arrives by an inbound link instead.
    if version != CANONICAL_VERSION {
        tags.push(r#"<meta name="robots" content="noindex,follow">"#.to_owned());
    }
    tags.push(format!(
        r#"<script type="application/ld+json">{}</script>"#,
        breadcrumbs(origin, base, version, rel, title.as_deref())
    ));
    tags.join("\n  ")
}

/// The page's `<title>` text, collapsed to one line.
fn page_title(page: &str) -> Option<String> {
    let (_, after_open) = page.split_once("<title>")?;
    let (inner, _) = after_open.split_once("</title>")?;
    let text = collapse(&unescape(inner));
    (!text.is_empty()).then_some(text)
}

/// The page's opening prose, used as its meta description.
///
/// Skips anything shorter than [`DESCRIPTION_MINIMUM`] (a caption or a badge
/// row rather than a summary) and truncates on a word boundary.
fn first_paragraph(page: &str) -> Option<String> {
    let body = strip_non_prose(page);
    let mut rest = body.as_str();
    loop {
        let (_, after_open) = rest.split_once("<p")?;
        // `<p` also prefixes `<pre`: a real paragraph tag ends immediately or
        // continues into attributes, so anything else is a different element.
        if !after_open.starts_with('>') && !after_open.starts_with(char::is_whitespace) {
            rest = after_open;
            continue;
        }
        let (_attributes, after_gt) = after_open.split_once('>')?;
        let (inner, tail) = after_gt.split_once("</p>")?;
        let text = collapse(&unescape(&strip_tags(inner)));
        if text.chars().count() >= DESCRIPTION_MINIMUM {
            return Some(truncate_on_word(&text));
        }
        rest = tail;
    }
}

/// Drop the elements whose text is never page prose.
fn strip_non_prose(page: &str) -> String {
    let mut out = page.to_owned();
    for tag in ["script", "style", "nav", "aside"] {
        out = strip_element(&out, tag);
    }
    out
}

/// Remove every `<tag …>…</tag>` region, case-insensitively.
#[expect(
    clippy::string_slice,
    reason = "every index comes from `find` over `lower`, and \
              `to_ascii_lowercase` rewrites only ASCII bytes in place — it \
              never changes a byte length — so an index is the same char \
              boundary in `page` as in `lower`"
)]
fn strip_element(page: &str, tag: &str) -> String {
    let lower = page.to_ascii_lowercase();
    let open_needle = format!("<{tag}");
    let close_needle = format!("</{tag}>");
    let mut out = String::with_capacity(page.len());
    let mut cursor = 0usize;
    while let Some(found) = lower[cursor..].find(&open_needle) {
        let start = cursor + found;
        let Some(close) = lower[start..].find(&close_needle) else {
            break;
        };
        let end = start + close + close_needle.len();
        out.push_str(&page[cursor..start]);
        out.push(' ');
        cursor = end;
    }
    out.push_str(&page[cursor..]);
    out
}

/// Remove every HTML tag, keeping the text between them.
fn strip_tags(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut depth = 0usize;
    for ch in fragment.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Collapse every run of whitespace to one space and trim the result.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cut to [`DESCRIPTION_LIMIT`] characters on a word boundary, with an ellipsis.
fn truncate_on_word(text: &str) -> String {
    if text.chars().count() <= DESCRIPTION_LIMIT {
        return text.to_owned();
    }
    let cut: String = text.chars().take(DESCRIPTION_LIMIT).collect();
    let kept = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{kept}…")
}

/// Decode the five XML entities mdBook emits in titles and prose.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Escape a value for an HTML double-quoted attribute.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a value for a JSON string literal.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // A lone `<` inside an inline <script> would end the element.
            '<' => out.push_str("\\u003c"),
            // A raw control character is illegal in a JSON string.
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A human label for a URL path segment (`beyond-core` → `Beyond Core`).
fn segment_label(segment: &str) -> String {
    segment
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A `BreadcrumbList` mirroring the page's URL hierarchy, as JSON.
fn breadcrumbs(origin: &str, base: &str, version: &str, rel: &str, title: Option<&str>) -> String {
    let mut items: Vec<(String, String)> = vec![
        (SITE_NAME.to_owned(), format!("{origin}{base}/")),
        (
            "Documentation".to_owned(),
            format!("{origin}{base}/docs/{version}/"),
        ),
    ];
    let mut trail = format!("{origin}{base}/docs/{version}");
    let segments: Vec<&str> = rel.split('/').collect();
    for segment in segments.iter().take(segments.len().saturating_sub(1)) {
        if segment.is_empty() {
            continue;
        }
        trail = format!("{trail}/{segment}");
        items.push((segment_label(segment), format!("{trail}/")));
    }
    let leaf = title
        .and_then(|t| t.split(" - ").next())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Page")
        .to_owned();
    items.push((leaf, format!("{origin}{base}/docs/{version}/{rel}")));

    let list = items
        .iter()
        .enumerate()
        .map(|(index, (name, url))| {
            format!(
                r#"{{"@type":"ListItem","position":{},"name":{},"item":{}}}"#,
                index + 1,
                json_string(name),
                json_string(url)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"@context":"https://schema.org","@type":"BreadcrumbList","itemListElement":[{list}]}}"#
    )
}

/// A relative path as forward-slash text.
fn to_slash(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r"<!DOCTYPE html><html><head><title>Getting started - FerroEHR Documentation</title>
        </head><body><nav><p>Sidebar link text that is long enough to be mistaken for prose content</p></nav>
        <main><p>Short.</p>
        <p>This chapter takes you from nothing to a running server with a template loaded, a stored
        composition, and an <code>AQL</code> query returning results.</p></main></body></html>";

    #[test]
    fn title_is_read_and_collapsed() {
        assert_eq!(
            page_title(PAGE).unwrap(),
            "Getting started - FerroEHR Documentation"
        );
    }

    /// Navigation text is not prose, and a caption-length paragraph is skipped.
    #[test]
    fn description_skips_navigation_and_short_paragraphs() {
        let description = first_paragraph(PAGE).unwrap();
        assert!(
            description.starts_with("This chapter takes you"),
            "{description}"
        );
        assert!(!description.contains("Sidebar"), "{description}");
        assert!(description.contains("AQL"), "inline markup is unwrapped");
    }

    #[test]
    fn description_truncates_on_a_word_boundary() {
        let long = "word ".repeat(200);
        let cut = truncate_on_word(&long);
        assert!(
            cut.chars().count() <= DESCRIPTION_LIMIT + 1,
            "{}",
            cut.len()
        );
        assert!(cut.ends_with('…'));
        assert!(!cut.contains("wor…"), "never cuts mid-word: {cut}");
    }

    /// The canonical URL is the same for every version of a page — that is the
    /// whole point of the pass.
    #[test]
    fn every_version_canonicalizes_to_one_url() {
        let dev = meta_tags("https://x.test", "", "dev", "a/b.html", PAGE);
        let latest = meta_tags("https://x.test", "", "latest", "a/b.html", PAGE);
        let expected = r#"<link rel="canonical" href="https://x.test/docs/latest/a/b.html">"#;
        assert!(dev.contains(expected), "{dev}");
        assert!(latest.contains(expected), "{latest}");
        // …and only the non-canonical copy withdraws from the index.
        assert!(dev.contains(r#"content="noindex,follow""#));
        assert!(!latest.contains("noindex"));
    }

    #[test]
    fn breadcrumbs_mirror_the_url_hierarchy() {
        let json = breadcrumbs(
            "https://x.test",
            "",
            "latest",
            "beyond-core/s3-multimedia.html",
            Some("S3 multimedia - FerroEHR Documentation"),
        );
        assert!(json.contains(r#""name":"FerroEHR""#), "{json}");
        assert!(json.contains(r#""name":"Beyond Core""#), "{json}");
        assert!(json.contains(r#""name":"S3 multimedia""#), "{json}");
        assert!(json.contains(r#""position":4"#), "{json}");
    }

    /// A `<` in a title must not be able to close the inline `<script>`.
    #[test]
    fn json_escapes_a_script_terminator() {
        let escaped = json_string("</script><img src=x>");
        assert!(!escaped.contains("</script>"), "{escaped}");
        assert!(escaped.contains("\\u003c"), "{escaped}");
    }

    #[test]
    fn attribute_values_are_escaped() {
        assert_eq!(escape(r#"a "b" & <c>"#), "a &quot;b&quot; &amp; &lt;c&gt;");
    }
}
