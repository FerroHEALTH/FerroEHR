// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared DDL template both pseudonymisation domains are rendered from.
//!
//! The clinical and party domains carry the same change-control and node
//! relations. The first storage generation built the second domain with
//! `CREATE TABLE … (LIKE …)`, which copies checks but neither unique indexes
//! nor foreign keys, so the twin silently lacked invariants the original had
//! and a column added later never reached it. Here one template renders both
//! migration files instead, and a test refuses any difference between the
//! rendered text and the committed files.
//!
//! No openEHR spec governs storage layout — our own design/extension. What the
//! separation realizes is GDPR Art. 4(5) and Art. 32(1)(a)
//! (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>).

/// One domain's substitutions into the shared DDL templates.
#[derive(Debug, Clone, Copy)]
pub struct Domain {
    /// The versioned-object RM types this domain admits; every other kind is
    /// refused by the rendered `CHECK`.
    pub kinds: &'static [&'static str],
    /// Whether the domain owns an `ehr` relation the change-control rows may
    /// reference. The party domain has none, so those foreign keys are not
    /// rendered into its files.
    pub references_ehr: bool,
}

/// The clinical domain: the EHR-owned versioned objects.
pub const CLINICAL: Domain = Domain {
    kinds: &["COMPOSITION", "EHR_STATUS", "EHR_ACCESS", "FOLDER"],
    references_ehr: true,
};

/// The party domain: the demographic versioned objects.
pub const PARTY: Domain = Domain {
    kinds: &[
        "AGENT",
        "GROUP",
        "ORGANISATION",
        "PERSON",
        "ROLE",
        "PARTY_RELATIONSHIP",
    ],
    references_ehr: false,
};

/// The change-control template: commit audit, contribution, version, head and
/// attestations.
pub const CHANGE_CONTROL: &str = include_str!("../../migrations/templates/change_control.sql.in");

/// The node template: the decomposed content of every version.
pub const NODE: &str = include_str!("../../migrations/templates/node.sql.in");

/// The marker opening a block rendered only for a domain that owns an `ehr`
/// relation.
const EHR_BEGIN: &str = "--@ehr-begin";

/// The marker closing such a block.
const EHR_END: &str = "--@ehr-end";

/// The placeholder the domain's admitted kinds are rendered into.
const KINDS: &str = "{{kinds}}";

/// Render one template for one domain.
///
/// The rendering is a pure function of the template text and the [`Domain`]:
/// the `{{kinds}}` placeholder becomes the domain's quoted kind list, and a
/// block between the `--@ehr-begin` and `--@ehr-end` markers is kept only when
/// the domain owns an `ehr` relation. The markers themselves never reach the
/// output.
#[must_use]
pub fn render(template: &str, domain: &Domain) -> String {
    let kinds = domain
        .kinds
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = String::with_capacity(template.len());
    let mut inside_ehr_block = false;
    for line in template.lines() {
        let trimmed = line.trim();
        if trimmed == EHR_BEGIN {
            inside_ehr_block = true;
            continue;
        }
        if trimmed == EHR_END {
            inside_ehr_block = false;
            continue;
        }
        if inside_ehr_block && !domain.references_ehr {
            continue;
        }
        out.push_str(&line.replace(KINDS, &kinds));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rendered file that each template + domain pair is committed as.
    const RENDERED: &[(&str, Domain, &str)] = &[
        (
            "migrations/clinical/0003_change_control.sql",
            CLINICAL,
            CHANGE_CONTROL,
        ),
        (
            "migrations/party/0002_change_control.sql",
            PARTY,
            CHANGE_CONTROL,
        ),
        ("migrations/clinical/0004_node.sql", CLINICAL, NODE),
        ("migrations/party/0003_node.sql", PARTY, NODE),
    ];

    /// Set to `1` to rewrite the committed files from the templates instead of
    /// asserting they match.
    const REGENERATE: &str = "FERROEHR_REGENERATE_MIGRATIONS";

    #[test]
    fn committed_migrations_match_the_templates() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // Reading the switch is what makes this test the generator as well as
        // its own drift guard, the shape snapshot tooling uses.
        #[expect(
            clippy::disallowed_methods,
            reason = "the regeneration switch is developer input to a test, not server configuration"
        )]
        let regenerate = std::env::var(REGENERATE).is_ok_and(|value| value == "1");
        for (path, domain, template) in RENDERED {
            let rendered = render(template, domain);
            let file = root.join(path);
            if regenerate {
                std::fs::write(&file, &rendered).expect("write the rendered migration");
                continue;
            }
            let committed = std::fs::read_to_string(&file).expect("read the committed migration");
            assert_eq!(
                committed, rendered,
                "{path} has drifted from its template; re-run with {REGENERATE}=1"
            );
        }
    }

    #[test]
    fn the_two_domains_differ_only_in_kinds_and_the_clinical_references() {
        for template in [CHANGE_CONTROL, NODE] {
            let clinical = render(template, &CLINICAL);
            let party = render(template, &PARTY);
            let differences: Vec<(&str, &str)> = diff_lines(&clinical, &party);
            for (left, right) in differences {
                let is_kind_check = left.contains("'COMPOSITION'") && right.contains("'PERSON'");
                let is_clinical_reference =
                    right.is_empty() && (left.contains("REFERENCES ehr (id)"));
                assert!(
                    is_kind_check || is_clinical_reference,
                    "unexpected difference between the domains:\n  clinical: {left}\n  party:    {right}"
                );
            }
        }
    }

    /// The lines of `left` that `right` does not carry in the same position,
    /// paired with what `right` carries there (the empty string once `right`
    /// has run out).
    fn diff_lines<'a>(left: &'a str, right: &'a str) -> Vec<(&'a str, &'a str)> {
        let mut right_lines = right.lines().peekable();
        let mut differences = Vec::new();
        for line in left.lines() {
            match right_lines.peek() {
                Some(other) if *other == line => {
                    right_lines.next();
                }
                Some(other) => {
                    // A line the clinical rendering has and the party one does
                    // not: report it against the empty string, and do not
                    // consume the party line, so the two realign afterwards.
                    if line.contains("REFERENCES ehr (id)") {
                        differences.push((line, ""));
                    } else {
                        differences.push((line, *other));
                        right_lines.next();
                    }
                }
                None => differences.push((line, "")),
            }
        }
        differences
    }

    #[test]
    fn the_kind_checks_are_disjoint() {
        for clinical_kind in CLINICAL.kinds {
            assert!(
                !PARTY.kinds.contains(clinical_kind),
                "{clinical_kind} is admitted by both domains"
            );
        }
    }
}
