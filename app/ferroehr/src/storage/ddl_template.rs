// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared DDL template both pseudonymisation domains are rendered from.
//!
//! The clinical and party domains carry the same change-control and node
//! relations, so one template renders both migration files, and a test refuses
//! any difference between the rendered text and the committed files.
//!
//! No openEHR spec governs storage layout — our own design/extension. What the
//! separation realizes is GDPR Art. 4(5) and Art. 32(1)(a)
//! (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>).

/// One domain's substitutions into the shared DDL templates.
#[derive(Debug, Clone, Copy)]
pub struct Domain {
    /// The domain's name, which marks the blocks rendered into it alone.
    pub name: &'static str,
    /// The versioned-object RM types this domain admits; every other kind is
    /// refused by the rendered `CHECK`.
    pub kinds: &'static [&'static str],
}

/// The clinical domain: the EHR-owned versioned objects.
pub const CLINICAL: Domain = Domain {
    name: "clinical",
    kinds: &["COMPOSITION", "EHR_STATUS", "EHR_ACCESS", "FOLDER"],
};

/// The party domain: the demographic versioned objects.
pub const PARTY: Domain = Domain {
    name: "party",
    kinds: &[
        "AGENT",
        "GROUP",
        "ORGANISATION",
        "PERSON",
        "ROLE",
        "PARTY_RELATIONSHIP",
    ],
};

/// The domains the templates render into.
pub const DOMAINS: [Domain; 2] = [CLINICAL, PARTY];

/// The change-control template: commit audit, contribution, version, head and
/// attestations.
pub const CHANGE_CONTROL: &str = include_str!("../../migrations/templates/change_control.sql.in");

/// The node template: the decomposed content of every version, with its
/// indexes and extended statistics.
pub const NODE: &str = include_str!("../../migrations/templates/node.sql.in");

/// The prefix of a marker line; `--@<domain>-begin` opens a block rendered
/// into that domain alone and `--@<domain>-end` closes it.
const MARKER: &str = "--@";

/// The placeholder the domain's admitted kinds are rendered into.
const KINDS: &str = "{{kinds}}";

/// Render one template for one domain.
///
/// The rendering is a pure function of the template text and the [`Domain`]:
/// the `{{kinds}}` placeholder becomes the domain's quoted kind list, and a
/// block between `--@<name>-begin` and `--@<name>-end` is kept only for the
/// domain of that name. The markers themselves never reach the output.
#[must_use]
pub fn render(template: &str, domain: &Domain) -> String {
    let kinds = domain
        .kinds
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = String::with_capacity(template.len());
    let mut block: Option<&str> = None;
    for line in template.lines() {
        if let Some(marker) = line.trim().strip_prefix(MARKER) {
            if let Some(name) = marker.strip_suffix("-begin") {
                block = Some(name);
                continue;
            }
            if marker.strip_suffix("-end").is_some() {
                block = None;
                continue;
            }
        }
        if block.is_some_and(|name| name != domain.name) {
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
            "migrations/clinical/0003_baseline_change_control.sql",
            CLINICAL,
            CHANGE_CONTROL,
        ),
        (
            "migrations/party/0002_baseline_change_control.sql",
            PARTY,
            CHANGE_CONTROL,
        ),
        ("migrations/clinical/0004_baseline_node.sql", CLINICAL, NODE),
        ("migrations/party/0003_baseline_node.sql", PARTY, NODE),
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

    /// A template renders the same text into both domains but for the kind
    /// list and the lines of the blocks marked for one domain.
    #[test]
    fn the_two_domains_differ_only_in_kinds_and_their_own_blocks() {
        for template in [CHANGE_CONTROL, NODE] {
            // The template with every marked block taken out, rendered for one
            // domain: what is left differs between the domains only in kinds.
            let mut kept: Vec<&str> = Vec::new();
            let mut inside = false;
            for line in template.lines() {
                if let Some(marker) = line.trim().strip_prefix(MARKER) {
                    inside = marker.ends_with("-begin");
                    continue;
                }
                if !inside {
                    kept.push(line);
                }
            }
            let unmarked = kept.join("\n");
            let shared = |domain: &Domain| render(&unmarked, domain);
            let clinical = shared(&CLINICAL);
            let party = shared(&PARTY);
            assert_eq!(clinical.lines().count(), party.lines().count());
            for (left, right) in clinical.lines().zip(party.lines()) {
                if left != right {
                    assert!(
                        left.contains("'COMPOSITION'") && right.contains("'PERSON'"),
                        "unexpected difference between the domains:\n  clinical: {left}\n  party:    {right}"
                    );
                }
            }
        }
    }

    /// Every block marker names a domain the templates render into, and every
    /// block that opens closes.
    #[test]
    fn every_marker_names_a_domain_and_every_block_closes() {
        for template in [CHANGE_CONTROL, NODE] {
            let mut open: Option<&str> = None;
            for line in template.lines() {
                let Some(marker) = line.trim().strip_prefix(MARKER) else {
                    continue;
                };
                if let Some(name) = marker.strip_suffix("-begin") {
                    assert!(open.is_none(), "a block opens inside another: {line}");
                    assert!(
                        DOMAINS.iter().any(|d| d.name == name),
                        "a block names no domain: {line}"
                    );
                    open = Some(name);
                } else if let Some(name) = marker.strip_suffix("-end") {
                    assert_eq!(open, Some(name), "a block closes another: {line}");
                    open = None;
                } else {
                    panic!("a marker that neither opens nor closes a block: {line}");
                }
            }
            assert!(open.is_none(), "a block never closes");
        }
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
