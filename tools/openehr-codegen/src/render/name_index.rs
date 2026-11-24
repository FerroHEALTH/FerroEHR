// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A name lookup decided at generation time (#6443): the emitted function
//! dispatches on a name's length and then on the bytes that tell the names of
//! that length apart, so a lookup hashes nothing. The caller confirms the
//! candidate by comparing its name.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A name's position in the table and its bytes.
type Named<'a> = (usize, &'a [u8]);

/// Names grouped by the byte at one position.
type ByByte<'a> = BTreeMap<u8, Vec<Named<'a>>>;

/// The source of `fn {fn_name}(name: &str) -> Option<usize>`, which answers
/// the position in `names` that `name` can only be, or `None`.
///
/// # Errors
/// Names a name that occurs twice in `names`.
pub(super) fn emit_index_fn(fn_name: &str, doc: &str, names: &[&str]) -> Result<String, String> {
    let mut by_len: BTreeMap<usize, Vec<Named<'_>>> = BTreeMap::new();
    for (i, name) in names.iter().enumerate() {
        by_len
            .entry(name.len())
            .or_default()
            .push((i, name.as_bytes()));
    }
    let mut b = format!(
        "/// {doc}\n\
         pub(super) fn {fn_name}(name: &str) -> Option<usize> {{\n    \
         let b = name.as_bytes();\n    \
         match b.len() {{\n"
    );
    for (len, group) in &by_len {
        let _ = write!(b, "        {len} => ");
        tree(&mut b, group, 2)?;
        b.push_str(",\n");
    }
    b.push_str("        _ => None,\n    }\n}\n");
    Ok(b)
}

/// One decision over `group`, names of one length: a single name is the
/// answer, more switch on the first byte position that splits them most.
fn tree(b: &mut String, group: &[Named<'_>], depth: usize) -> Result<(), String> {
    let [(only, _)] = group else {
        let mut split: Option<(usize, ByByte<'_>)> = None;
        let len = group.first().map_or(0, |(_, n)| n.len());
        for pos in 0..len {
            let mut by_byte = ByByte::new();
            for &(i, n) in group {
                if let Some(&byte) = n.get(pos) {
                    by_byte.entry(byte).or_default().push((i, n));
                }
            }
            if split
                .as_ref()
                .is_none_or(|(_, best)| by_byte.len() > best.len())
            {
                split = Some((pos, by_byte));
            }
        }
        let Some((pos, by_byte)) = split.filter(|(_, best)| best.len() > 1) else {
            let names: BTreeSet<String> = group
                .iter()
                .map(|(_, n)| String::from_utf8_lossy(n).into_owned())
                .collect();
            return Err(format!("a name occurs twice in the index: {names:?}"));
        };
        let pad = "    ".repeat(depth + 1);
        if pos == 0 {
            b.push_str("match b.first() {\n");
        } else {
            let _ = writeln!(b, "match b.get({pos}) {{");
        }
        for (byte, sub) in &by_byte {
            let _ = write!(b, "{pad}    Some({}) => ", byte_lit(*byte));
            tree(b, sub, depth + 1)?;
            b.push_str(",\n");
        }
        let _ = write!(b, "{pad}    _ => None,\n{pad}}}");
        return Ok(());
    };
    let _ = write!(b, "Some({only})");
    Ok(())
}

fn byte_lit(byte: u8) -> String {
    if byte.is_ascii_alphanumeric() || byte == b'_' {
        format!("b'{}'", char::from(byte))
    } else {
        byte.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::emit_index_fn;

    #[test]
    fn names_of_one_length_switch_on_the_byte_that_splits_them() {
        let src = emit_index_fn("index", "The index.", &["AB", "AC", "XYZ"]).unwrap();
        assert!(src.contains("2 => match b.get(1) {"), "{src}");
        assert!(src.contains("Some(b'B') => Some(0)"), "{src}");
        assert!(src.contains("Some(b'C') => Some(1)"), "{src}");
        assert!(src.contains("3 => Some(2)"), "{src}");
    }

    #[test]
    fn a_name_given_twice_is_refused() {
        assert!(emit_index_fn("index", "The index.", &["AB", "AB"]).is_err());
    }
}
