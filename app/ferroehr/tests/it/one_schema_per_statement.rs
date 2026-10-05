// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! No SQL statement this crate emits names relations of two storage domains.
//!
//! The domains are separated by schema, and the ONLY thing that selects one is
//! the pool's `search_path` ([`ferroehr::db::domain::Domain::search_path`]), on
//! which no other domain's schema ever appears. A statement can therefore reach
//! across the boundary in exactly one way: by schema-qualifying a relation of a
//! domain other than the pool's. That is what this suite refuses.
//!
//! It reads the crate's own sources rather than a running database, because the
//! property is about the statements that EXIST, not about the ones a particular
//! test happens to execute: a cross-domain join written into a rarely-taken
//! branch would pass every behavioural test and still be the defect. Once a
//! domain is relocated to a database of its own the statement would fail at
//! runtime, and a failure at that point is a served request that errors rather
//! than a build that refuses.
//!
//! NOTE: no openEHR spec governs schemas or database roles — our own
//! design/extension (GDPR Art. 4(5) and Art. 32(1)(a);
//! <https://eur-lex.europa.eu/eli/reg/2016/679/oj>).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              helpers; a failing fixture must panic at the fixture (the Rust Book ch11)"
)]

use std::path::{Path, PathBuf};

use ferroehr::db::domain::Domain;

/// The crate's `src` tree.
fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src`, recursively.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read the source directory") {
        let path = entry.expect("read a directory entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// One string literal found in a source file: its text and the line it opened
/// on.
#[derive(Debug)]
struct Literal {
    /// The literal's content, escapes left as written.
    text: String,
    /// The 1-based line the literal opened on.
    line: usize,
}

/// Every string literal in `source`, with line comments removed first and
/// adjacent literals joined.
///
/// Comments are dropped because prose about the boundary names several schemas
/// by design — this module's own documentation does. Raw strings (`r"…"`,
/// `r#"…"#`) are read as literals too, since SQL is often written that way. Two
/// literals separated by nothing but whitespace are ONE literal to the compiler,
/// so they are one statement here: measuring them apart would let a two-domain
/// join pass by being split across a line break.
fn literals(source: &str) -> Vec<Literal> {
    let chars: Vec<char> = source.chars().collect();
    let mut out: Vec<Literal> = Vec::new();
    let mut index = 0;
    let mut line = 1;
    // Where the previous literal ended, and whether only whitespace has been
    // seen since — which is what makes the next literal a continuation of it.
    let mut only_whitespace_since_literal = false;
    while index < chars.len() {
        let current = chars.get(index).copied().unwrap_or('\0');
        if current == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if current.is_whitespace() {
            index += 1;
            continue;
        }
        // A line comment runs to the end of the line, whatever it contains, and
        // breaks a concatenation run only if code follows it.
        if current == '/' && chars.get(index + 1) == Some(&'/') {
            while index < chars.len() && chars.get(index) != Some(&'\n') {
                index += 1;
            }
            continue;
        }
        let raw_hashes = (current == 'r').then(|| {
            let mut hashes = 0;
            while chars.get(index + 1 + hashes) == Some(&'#') {
                hashes += 1;
            }
            hashes
        });
        let opens_raw =
            raw_hashes.is_some_and(|hashes| chars.get(index + 1 + hashes) == Some(&'"'));
        if !opens_raw && current != '"' {
            only_whitespace_since_literal = false;
            index += 1;
            continue;
        }

        let opened = line;
        let mut text = String::new();
        let mut cursor;
        if opens_raw {
            let hashes = raw_hashes.unwrap_or(0);
            cursor = index + 2 + hashes;
            while let Some(&character) = chars.get(cursor) {
                if character == '"'
                    && (0..hashes).all(|offset| chars.get(cursor + 1 + offset) == Some(&'#'))
                {
                    cursor += 1 + hashes;
                    break;
                }
                if character == '\n' {
                    line += 1;
                }
                text.push(character);
                cursor += 1;
            }
        } else {
            cursor = index + 1;
            while let Some(&character) = chars.get(cursor) {
                if character == '\\' {
                    // An escape: skip the escaped character, and keep counting
                    // lines through a line continuation.
                    if chars.get(cursor + 1) == Some(&'\n') {
                        line += 1;
                    }
                    cursor += 2;
                    continue;
                }
                if character == '"' {
                    cursor += 1;
                    break;
                }
                if character == '\n' {
                    line += 1;
                }
                text.push(character);
                cursor += 1;
            }
        }
        if only_whitespace_since_literal && let Some(previous) = out.last_mut() {
            previous.text.push(' ');
            previous.text.push_str(&text);
        } else {
            out.push(Literal { text, line: opened });
        }
        only_whitespace_since_literal = true;
        index = cursor;
    }
    out
}

/// The domain schemas a piece of SQL text qualifies a relation with.
fn qualified_domains(text: &str) -> Vec<Domain> {
    let mut found = Vec::new();
    for domain in Domain::ALL {
        let prefix = format!("{}.", domain.schema());
        // A qualifier is followed by an identifier character; `party.` at the
        // end of a sentence or before a space is prose, not a relation.
        let mut rest = text;
        while let Some(at) = rest.find(&prefix) {
            let after = rest
                .get(at + prefix.len()..)
                .and_then(|tail| tail.chars().next());
            if after.is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
                found.push(domain);
                break;
            }
            let Some(tail) = rest.get(at + prefix.len()..) else {
                break;
            };
            rest = tail;
        }
    }
    found
}

/// Every SQL string literal in the crate names relations of at most one storage
/// domain.
///
/// Adjacent literals are joined before the check, because Rust concatenates
/// them and a long statement is usually written as several: measuring them
/// apart would let a two-domain join pass by being split across a line break.
#[test]
fn no_emitted_statement_names_two_domains() {
    let mut files = Vec::new();
    sources(&source_root(), &mut files);
    assert!(
        files.len() > 100,
        "the source sweep found {} files, which is too few to be the crate",
        files.len()
    );

    let mut statements_naming_a_domain = 0_usize;
    let mut breaches: Vec<String> = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).expect("read a source file");
        for literal in literals(&source) {
            let domains = qualified_domains(&literal.text);
            if !domains.is_empty() {
                statements_naming_a_domain += 1;
            }
            if domains.len() > 1 {
                breaches.push(format!(
                    "{}:{} names {:?}",
                    file.display(),
                    literal.line,
                    domains
                ));
            }
        }
    }

    assert!(
        statements_naming_a_domain > 0,
        "the scan found no schema-qualified SQL at all, so it measured nothing"
    );
    assert!(
        breaches.is_empty(),
        "a statement naming two storage domains cannot run once either domain is \
         relocated, and joins across the pseudonymisation boundary while they share a \
         database:\n  {}",
        breaches.join("\n  ")
    );
}
