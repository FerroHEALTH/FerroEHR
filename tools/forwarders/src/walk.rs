// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hand-written source files of a tree, and the forwarders in each.
//!
//! The scan reads every `.rs` file under `app/*/src`, `crates/*/src` and
//! `tools/*/src`, and leaves out a file that opens with `// @generated`, a
//! file under a `tests` directory, and a file only a `#[cfg(test)] mod`
//! declaration reaches.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::detect::{self, Forwarder};

/// One reported forwarder with the file it stands in, relative to the root.
#[derive(Debug)]
pub(crate) struct Found {
    /// The path from the root, with `/` separators.
    pub(crate) path: String,
    /// The forwarder itself.
    pub(crate) forwarder: Forwarder,
}

impl Found {
    /// The key the ratchet's baseline records: the path, the owner and the
    /// target, with no line number, so code moving inside a file keeps it.
    pub(crate) fn key(&self) -> String {
        format!(
            "{}: {} -> {}",
            self.path, self.forwarder.owner, self.forwarder.target
        )
    }
}

/// Every forwarder of the tree at `root`, sorted by key and line.
///
/// With `lenient`, a file that does not parse is passed over: the per-edit
/// check runs while another file of the tree may be half written.
///
/// # Errors
///
/// Returns an error when a directory or file cannot be read, or, unless
/// `lenient`, when a file does not parse as Rust.
pub(crate) fn tree(root: &Path, lenient: bool) -> Result<Vec<Found>> {
    let mut files = Vec::new();
    for group in ["app", "crates", "tools"] {
        for member in sorted_entries(&root.join(group))? {
            let src = member.join("src");
            if src.is_dir() {
                collect(&src, &mut files)?;
            }
        }
    }

    let mut scanned = Vec::new();
    let mut test_only: Vec<PathBuf> = Vec::new();
    for file in files {
        let text =
            fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        if text.starts_with("// @generated") {
            continue;
        }
        let parsed = match syn::parse_file(&text) {
            Ok(parsed) => parsed,
            Err(_) if lenient => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("parsing {}", file.display()));
            }
        };
        let report = detect::scan(&parsed);
        let dir = module_dir(&file);
        for module in &report.test_modules {
            test_only.push(
                module
                    .iter()
                    .fold(dir.clone(), |path, name| path.join(name)),
            );
        }
        scanned.push((file, report.forwarders));
    }

    let mut found = Vec::new();
    for (file, forwarders) in scanned {
        if test_only
            .iter()
            .any(|module| file.with_extension("") == *module || file.starts_with(module))
        {
            continue;
        }
        let path = relative(root, &file)?;
        found.extend(forwarders.into_iter().map(|forwarder| Found {
            path: path.clone(),
            forwarder,
        }));
    }
    found.sort_by(|a, b| {
        a.key()
            .cmp(&b.key())
            .then(a.forwarder.line.cmp(&b.forwarder.line))
    });
    Ok(found)
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("listing {}", dir.display()))?;
    entries.sort();
    Ok(entries)
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in sorted_entries(dir)? {
        if entry.is_dir() {
            if entry.file_name().is_some_and(|name| name != "tests") {
                collect(&entry, files)?;
            }
        } else if entry.extension().is_some_and(|ext| ext == "rs") {
            files.push(entry);
        }
    }
    Ok(())
}

/// The directory the out-of-line modules a file declares live in: its own
/// directory for a crate root or a `mod.rs`, else the directory named after it.
fn module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new(""));
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let in_bin = parent.file_name().is_some_and(|dir| dir == "bin");
    if matches!(name, "lib.rs" | "main.rs" | "mod.rs") || in_bin {
        parent.to_path_buf()
    } else {
        file.with_extension("")
    }
}

/// The path of `file` from `root`, with `/` separators.
///
/// # Errors
///
/// Returns an error when `file` is not under `root`.
pub(crate) fn relative(root: &Path, file: &Path) -> Result<String> {
    let rest = file
        .strip_prefix(root)
        .with_context(|| format!("{} is outside {}", file.display(), root.display()))?;
    let parts: Vec<String> = rest
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::tree;

    fn write(root: &Path, path: &str, text: &str) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().expect("a fixture file has a parent"))
            .expect("the fixture directory should be created");
        fs::write(file, text).expect("the fixture should be written");
    }

    #[test]
    fn the_scan_skips_generated_test_only_and_tests_files() {
        let root = std::env::temp_dir().join(format!("forwarders-walk-{}", std::process::id()));
        let forward = "pub fn f(x: u32) -> u32 { g(x) }\n";
        write(
            &root,
            "app/demo/src/lib.rs",
            &format!("mod a;\n#[cfg(test)]\nmod tests;\n{forward}"),
        );
        write(
            &root,
            "app/demo/src/a.rs",
            &format!("#[cfg(test)]\nmod helpers;\n{forward}"),
        );
        write(&root, "app/demo/src/a/helpers.rs", forward);
        write(&root, "app/demo/src/tests.rs", forward);
        write(&root, "app/demo/src/tests/deep.rs", forward);
        write(&root, "app/demo/tests/it.rs", forward);
        write(
            &root,
            "crates/gen/src/lib.rs",
            &format!("// @generated DO NOT EDIT.\n{forward}"),
        );
        write(&root, "tools/kit/src/main.rs", forward);

        let found = tree(&root, false).expect("the fixture tree should scan");
        let keys: Vec<String> = found.iter().map(super::Found::key).collect();
        fs::remove_dir_all(&root).expect("the fixture tree should be removed");
        assert_eq!(
            keys,
            [
                "app/demo/src/a.rs: f -> g(x)",
                "app/demo/src/lib.rs: f -> g(x)",
                "tools/kit/src/main.rs: f -> g(x)",
            ]
        );
    }
}
