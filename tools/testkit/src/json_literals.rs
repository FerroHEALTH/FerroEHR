// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared scanner behind each crate's `canonical_json_literals` gate:
//! every production `json!` invocation whose body carries a `"_type"` key,
//! found by walking the calling crate's own `src/` tree.
//!
//! A `json!` literal carrying `"_type"` is a hand-rolled canonical openEHR
//! fragment: `_type` is the discriminator the native codec emits, so nothing
//! else writes it. The per-crate gates own their allowlist adjudications; this
//! module owns only what counts as an offending site.

use std::path::{Path, PathBuf};

/// Returns every `json!` invocation under `<manifest_dir>/src` whose body
/// carries a `"_type"` key, as sorted `(crate-relative path, 1-based line)`.
///
/// Everything from a file's first line-initial `#[cfg(test)]` on is skipped as
/// unit-test fixture territory.
///
/// # Errors
/// Any I/O failure walking or reading the crate's `src/` tree.
pub fn offending_sites(manifest_dir: &str) -> std::io::Result<Vec<(String, usize)>> {
    let src = PathBuf::from(manifest_dir).join("src");
    let mut out = Vec::new();
    collect(&src, "", &mut out)?;
    out.sort();
    Ok(out)
}

/// Walks `dir` (reached at crate-relative `prefix`) recursively, appending each
/// offending site to `out`.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, usize)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        if path.is_dir() {
            collect(&path, &relative, out)?;
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let production = match text.find("\n#[cfg(test)]") {
            Some(at) => text.get(..at).unwrap_or(&text),
            None => &text,
        };
        for start in json_macro_bodies(production) {
            let line = production
                .get(..start)
                .map_or(1, |before| before.matches('\n').count() + 1);
            out.push((relative.clone(), line));
        }
    }
    Ok(())
}

/// Returns the byte offset of every `json!` invocation in `src` whose delimited
/// body contains a `"_type"` key.
fn json_macro_bodies(src: &str) -> Vec<usize> {
    let bytes = src.as_bytes();
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(rel) = src.get(from..).and_then(|rest| rest.find("json!")) {
        let at = from + rel;
        from = at + "json!".len();
        // `serde_json::json!` is the same macro; `foojson!` is not.
        let preceded_by_ident = at
            .checked_sub(1)
            .and_then(|i| bytes.get(i))
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        if preceded_by_ident {
            continue;
        }
        let Some(open) = src
            .get(from..)
            .and_then(|rest| rest.find(|c: char| !c.is_whitespace()).map(|i| from + i))
        else {
            continue;
        };
        if !matches!(bytes.get(open), Some(b'(' | b'{' | b'[')) {
            continue;
        }
        if let Some(end) = matching_delimiter(bytes, open)
            && src.get(open..=end).is_some_and(|b| b.contains("\"_type\""))
        {
            hits.push(at);
        }
    }
    hits
}

/// Returns the index of the delimiter closing the one at `open`, skipping
/// string literals so a brace inside a JSON string cannot unbalance the scan.
fn matching_delimiter(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, byte) in bytes.iter().enumerate().skip(open) {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Returns the sites not covered by `allowlist` — the gate's failing set.
#[must_use]
pub fn unlisted<'a>(
    sites: &'a [(String, usize)],
    allowlist: &[(&str, &str)],
) -> Vec<&'a (String, usize)> {
    sites
        .iter()
        .filter(|(file, _)| !allowlist.iter().any(|(listed, _)| listed == file))
        .collect()
}

/// Returns the `allowlist` entries matching no site — stale licences the gate
/// refuses.
#[must_use]
pub fn stale_entries<'a>(sites: &[(String, usize)], allowlist: &'a [(&str, &str)]) -> Vec<&'a str> {
    allowlist
        .iter()
        .filter(|(listed, _)| !sites.iter().any(|(file, _)| file == listed))
        .map(|(listed, _)| *listed)
        .collect()
}
