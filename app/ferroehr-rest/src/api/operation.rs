// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The classification of every operation the REST layer dispatches.
//!
//! No specification governs this: our own design.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::extensions::access::authz::classify::{
        access_of, class_of, is_write, kind_of, mode_of,
    };
    use crate::extensions::access::ehr_access::is_composition_read;
    use crate::extensions::access::pep::{smart_composition_op, smart_composition_write};
    use crate::smart::routes::permission_of_op;
    use crate::system_log::classify::lookup;

    /// The committed classification of every operation id, one row each.
    fn golden_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/operation-classification.tsv")
    }

    /// Whether `op` is in a generated ITS-REST `ROUTES` table.
    fn generated(op: &str) -> bool {
        [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
            openehr_its::rest::generated::system::ROUTES,
        ]
        .iter()
        .any(|table| table.iter().any(|(_method, _path, id)| *id == op))
    }

    /// The classification row of `op`, each column over the operations its
    /// callers pass: the RBAC class and the write class over the generated
    /// tables the RBAC route map is built from, the SMART permission over those
    /// and the COMPOSITION operations the policy-enforcement point checks, and
    /// every other column over every operation.
    fn row(op: &str) -> String {
        let generated = generated(op);
        let smart_read = smart_composition_op(op);
        let smart_write = smart_composition_write(op);
        let dash = || "-".to_owned();
        [
            op.to_owned(),
            generated.to_string(),
            if generated {
                format!("{:?}", class_of(op))
            } else {
                dash()
            },
            format!("{:?}", kind_of(op)),
            format!("{:?}", access_of(op)),
            if generated {
                is_write(op).to_string()
            } else {
                dash()
            },
            format!("{:?}", mode_of(op)),
            format!("{:?}", lookup(op)),
            if generated || smart_read || smart_write {
                format!("{:?}", permission_of_op(op))
            } else {
                dash()
            },
            smart_read.to_string(),
            smart_write.to_string(),
            is_composition_read(op).to_string(),
        ]
        .join("\t")
    }

    /// The operation ids of the committed classification, in its order.
    fn golden_ops(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split('\t').next())
            .collect()
    }

    /// Every operation classifies exactly as the committed classification
    /// records, column by column.
    #[test]
    fn every_operation_classifies_as_recorded() {
        let text = std::fs::read_to_string(golden_path()).expect("read the classification");
        let mut differing = Vec::new();
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let op = line.split('\t').next().unwrap_or_default();
            let now = row(op);
            if now != line {
                differing.push(format!("recorded: {line}\nnow:      {now}"));
            }
        }
        assert!(
            differing.is_empty(),
            "operations that classify differently from the record:\n{}",
            differing.join("\n")
        );
    }

    /// Every generated operation is in the committed classification.
    #[test]
    fn every_generated_operation_is_recorded() {
        let text = std::fs::read_to_string(golden_path()).expect("read the classification");
        let ops = golden_ops(&text);
        let mut missing = Vec::new();
        for table in [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
            openehr_its::rest::generated::system::ROUTES,
        ] {
            for (_method, _path, op) in table {
                if !ops.contains(op) {
                    missing.push(*op);
                }
            }
        }
        assert!(
            missing.is_empty(),
            "generated operations missing from the record: {missing:?}"
        );
    }

    /// Rewrites the committed classification from the current code.
    #[test]
    #[ignore = "the classification writer: run it only for a deliberate change of a classification"]
    fn write_the_operation_classification() {
        let text = std::fs::read_to_string(golden_path()).expect("read the classification");
        let header = "# op\tgenerated\tclass\tkind\taccess\tis_write\tpep_mode\taudit\tsmart_permission\tsmart_read\tsmart_write\tcomposition_read";
        let mut out = vec![header.to_owned()];
        out.extend(golden_ops(&text).into_iter().map(row));
        out.push(String::new());
        std::fs::write(golden_path(), out.join("\n")).expect("write the classification");
    }
}
