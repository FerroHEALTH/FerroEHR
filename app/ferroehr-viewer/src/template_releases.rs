// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A template's releases: the lineage each listed template belongs to, its
//! releases newest first, and what changed in the paths between two of them.
//!
//! A template identifier in the HRID form carries its release as a trailing
//! `.v{version}` (BASE `master05-identification_package.adoc` §Template
//! Identifiers; AOM2 `ARCHETYPE_HRID.release_version`), so the identifier
//! without it names the lineage. A legacy free-text ADL 1.4 identifier
//! carries none; such a template joins the lineage of its concept and of its
//! root archetype without that archetype's version axis (BASE §Archetype
//! Identifiers). Identifiers compare case-insensitively (BASE §Composite
//! Identifiers and Case). No specification governs the grouping, the order
//! or the comparison: our own design.

use std::collections::BTreeMap;

use ferrohealth_ui::view::change::Change;

use crate::adl2::{TemplateFamily, split_hrid, version_key};
use crate::bff::templates::TemplateRow;
use crate::builder::catalog::CatalogNode;

/// One template lineage: every release of it the CDR lists, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Lineage {
    /// The releases, newest first; never empty.
    releases: Vec<TemplateRow>,
}

impl Lineage {
    /// The releases, newest first.
    #[must_use]
    pub fn releases(&self) -> &[TemplateRow] {
        &self.releases
    }

    /// The newest release.
    #[must_use]
    pub fn newest(&self) -> Option<&TemplateRow> {
        self.releases.first()
    }

    /// Whether `template_id` is the newest release.
    #[must_use]
    pub fn is_newest(&self, template_id: &str) -> bool {
        self.newest()
            .is_some_and(|row| row.template_id.eq_ignore_ascii_case(template_id))
    }

    /// The release `template_id` names.
    #[must_use]
    pub fn release(&self, template_id: &str) -> Option<&TemplateRow> {
        self.releases
            .iter()
            .find(|row| row.template_id.eq_ignore_ascii_case(template_id))
    }

    /// The release `label` names in `family` ([`release_label`]).
    #[must_use]
    pub fn labelled(&self, family: TemplateFamily, label: &str) -> Option<&TemplateRow> {
        self.releases
            .iter()
            .find(|row| release_label(family, row).eq_ignore_ascii_case(label))
    }

    /// The release next to `template_id` to compare it with: the one just
    /// older, else the one just newer.
    #[must_use]
    pub fn neighbour(&self, template_id: &str) -> Option<&TemplateRow> {
        let at = self
            .releases
            .iter()
            .position(|row| row.template_id.eq_ignore_ascii_case(template_id))?;
        self.releases.get(at.saturating_add(1)).or_else(|| {
            at.checked_sub(1)
                .and_then(|before| self.releases.get(before))
        })
    }
}

/// The words a release is named by: its release version for an ADL 2
/// template, its whole identifier for an ADL 1.4 one, whose identifier is the
/// address of its page.
#[must_use]
pub fn release_label(family: TemplateFamily, row: &TemplateRow) -> String {
    match (family, split_hrid(&row.template_id)) {
        (TemplateFamily::Adl2, (_, Some(release))) => release.to_owned(),
        _ => row.template_id.clone(),
    }
}

/// The key of the lineage `row` belongs to.
fn lineage_key(row: &TemplateRow) -> String {
    match split_hrid(&row.template_id) {
        (stem, Some(_)) => stem.to_ascii_lowercase(),
        (id, None) if row.concept.trim().is_empty() || row.archetype_id.trim().is_empty() => {
            id.to_ascii_lowercase()
        }
        (_, None) => format!(
            "{}\u{1f}{}",
            row.concept.trim().to_lowercase(),
            split_hrid(&row.archetype_id).0.to_ascii_lowercase()
        ),
    }
}

/// The order of releases, newest first: the higher release version, then the
/// later upload, then the identifier.
fn newest_first(a: &TemplateRow, b: &TemplateRow) -> std::cmp::Ordering {
    let release = |row: &TemplateRow| split_hrid(&row.template_id).1.map(version_key);
    release(b)
        .cmp(&release(a))
        .then_with(|| b.created.cmp(&a.created))
        .then_with(|| a.template_id.cmp(&b.template_id))
}

/// The lineages of `rows`, in the order the listing first names each, every
/// one with its releases newest first.
#[must_use]
pub fn lineages(rows: Vec<TemplateRow>) -> Vec<Lineage> {
    let mut place: BTreeMap<String, usize> = BTreeMap::new();
    let mut found: Vec<Lineage> = Vec::new();
    for row in rows {
        let key = lineage_key(&row);
        if let Some(lineage) = place.get(&key).and_then(|at| found.get_mut(*at)) {
            lineage.releases.push(row);
        } else {
            place.insert(key, found.len());
            found.push(Lineage {
                releases: vec![row],
            });
        }
    }
    for lineage in &mut found {
        lineage.releases.sort_by(newest_first);
    }
    found
}

/// The lineage among `rows` that `template_id` belongs to.
#[must_use]
pub fn lineage_of(rows: Vec<TemplateRow>, template_id: &str) -> Option<Lineage> {
    lineages(rows)
        .into_iter()
        .find(|lineage| lineage.release(template_id).is_some())
}

/// What changed in the paths from one release to another.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Comparison {
    /// How many paths the later release adds.
    pub added: usize,
    /// How many paths it removes.
    pub removed: usize,
    /// How many paths it keeps with another name, type, unit or code.
    pub changed: usize,
    /// Every path added, removed or changed, in path order, each with what it
    /// was and what it is.
    pub changes: Vec<Change>,
}

/// The words that describe the node at a path: its name, its RM type, its
/// units and its codes.
fn described(node: &CatalogNode) -> String {
    let mut words = format!("{} · {}", node.label, node.rm_type);
    if !node.unit_options.is_empty() {
        words.push_str(" · ");
        words.push_str(&node.unit_options.join(", "));
    }
    if !node.code_options.is_empty() {
        words.push_str(" · ");
        let codes: Vec<&str> = node
            .code_options
            .iter()
            .map(|option| option.code.as_str())
            .collect();
        words.push_str(&codes.join(", "));
    }
    words
}

/// Every path under `node` with the words that describe it; two nodes at one
/// path are described together.
fn paths(node: &CatalogNode, into: &mut BTreeMap<String, String>) {
    if !node.aql_path.is_empty() {
        let words = described(node);
        into.entry(node.aql_path.clone())
            .and_modify(|known| {
                known.push_str(" | ");
                known.push_str(&words);
            })
            .or_insert(words);
    }
    for child in &node.children {
        paths(child, into);
    }
}

/// What changed in the paths from the release `from` to the release `to`.
#[must_use]
pub fn compare(from: &CatalogNode, to: &CatalogNode) -> Comparison {
    let (mut before, mut after) = (BTreeMap::new(), BTreeMap::new());
    paths(from, &mut before);
    paths(to, &mut after);
    let keys: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut comparison = Comparison::default();
    for key in keys {
        let (was, is) = (before.get(key), after.get(key));
        match (was, is) {
            (None, Some(_)) => comparison.added = comparison.added.saturating_add(1),
            (Some(_), None) => comparison.removed = comparison.removed.saturating_add(1),
            (Some(was), Some(is)) if was != is => {
                comparison.changed = comparison.changed.saturating_add(1);
            }
            _ => continue,
        }
        comparison.changes.push(Change {
            key: key.clone(),
            from: was.cloned(),
            to: is.cloned(),
        });
    }
    comparison
}

#[cfg(test)]
mod tests {
    use crate::adl2::TemplateFamily;
    use crate::bff::templates::TemplateRow;
    use crate::builder::catalog::{CatalogNode, CodeOption};
    use crate::template_releases::{Comparison, compare, lineage_of, lineages, release_label};

    fn row(id: &str, concept: &str, root: &str, created: &str) -> TemplateRow {
        TemplateRow {
            template_id: id.to_owned(),
            concept: concept.to_owned(),
            archetype_id: root.to_owned(),
            created: created.to_owned(),
        }
    }

    fn node(path: &str, rm_type: &str, children: Vec<CatalogNode>) -> CatalogNode {
        CatalogNode {
            label: path.rsplit('/').next().unwrap_or_default().to_owned(),
            rm_type: rm_type.to_owned(),
            aql_path: path.to_owned(),
            node_id: String::new(),
            selectable: children.is_empty(),
            code_options: Vec::new(),
            unit_options: Vec::new(),
            children,
        }
    }

    #[test]
    fn the_releases_of_one_hrid_are_one_lineage_newest_first() {
        let found = lineages(vec![
            row(
                "openEHR-EHR-COMPOSITION.versioned.v1.0.0",
                "Versioned",
                "openEHR-EHR-COMPOSITION.versioned.v1.0.0",
                "2026-10-01T09:00:00Z",
            ),
            row(
                "openEHR-EHR-COMPOSITION.other.v1.0.0",
                "Other",
                "openEHR-EHR-COMPOSITION.other.v1.0.0",
                "2026-10-01T09:00:00Z",
            ),
            row(
                "openEHR-EHR-COMPOSITION.versioned.v1.10.0",
                "Versioned",
                "openEHR-EHR-COMPOSITION.versioned.v1.10.0",
                "2026-09-01T09:00:00Z",
            ),
            row(
                "openEHR-EHR-COMPOSITION.versioned.v1.9.0",
                "Versioned",
                "openEHR-EHR-COMPOSITION.versioned.v1.9.0",
                "2026-10-02T09:00:00Z",
            ),
        ]);
        assert_eq!(found.len(), 2);
        let first = found.first().expect("two lineages");
        let labels: Vec<String> = first
            .releases()
            .iter()
            .map(|each| release_label(TemplateFamily::Adl2, each))
            .collect();
        assert_eq!(labels, ["1.10.0", "1.9.0", "1.0.0"]);
        assert!(first.is_newest("openEHR-EHR-COMPOSITION.versioned.v1.10.0"));
        assert_eq!(
            first
                .neighbour("openEHR-EHR-COMPOSITION.versioned.v1.10.0")
                .map(|each| each.template_id.as_str()),
            Some("openEHR-EHR-COMPOSITION.versioned.v1.9.0")
        );
        assert_eq!(
            first
                .neighbour("openEHR-EHR-COMPOSITION.versioned.v1.0.0")
                .map(|each| each.template_id.as_str()),
            Some("openEHR-EHR-COMPOSITION.versioned.v1.9.0")
        );
        assert_eq!(
            first
                .labelled(TemplateFamily::Adl2, "1.9.0")
                .map(|each| each.template_id.as_str()),
            Some("openEHR-EHR-COMPOSITION.versioned.v1.9.0")
        );
    }

    #[test]
    fn legacy_adl14_ids_join_by_concept_and_root_archetype_lineage() {
        let rows = vec![
            row(
                "Vital signs",
                "Vital signs",
                "openEHR-EHR-COMPOSITION.encounter.v1",
                "2026-09-01T09:00:00Z",
            ),
            row(
                "Vital signs (2)",
                "vital signs",
                "openEHR-EHR-COMPOSITION.encounter.v2",
                "2026-10-01T09:00:00Z",
            ),
            row(
                "Discharge",
                "Discharge",
                "openEHR-EHR-COMPOSITION.encounter.v1",
                "2026-10-01T09:00:00Z",
            ),
            row(
                "minimal_evaluation.en.v1",
                "Minimal evaluation",
                "openEHR-EHR-COMPOSITION.minimal.v1",
                "2026-09-01T09:00:00Z",
            ),
            row(
                "Minimal_Evaluation.en.v2",
                "Minimal evaluation",
                "openEHR-EHR-COMPOSITION.minimal.v1",
                "2026-08-01T09:00:00Z",
            ),
        ];
        let found = lineages(rows.clone());
        assert_eq!(found.len(), 3);
        let vitals = lineage_of(rows.clone(), "Vital signs").expect("listed");
        assert_eq!(
            vitals.newest().map(|each| each.template_id.as_str()),
            Some("Vital signs (2)"),
            "with no release version the later upload is the newest"
        );
        let minimal = lineage_of(rows, "minimal_evaluation.en.v1").expect("listed");
        assert_eq!(
            minimal
                .releases()
                .iter()
                .map(|each| release_label(TemplateFamily::Adl14, each))
                .collect::<Vec<_>>(),
            ["Minimal_Evaluation.en.v2", "minimal_evaluation.en.v1"],
            "the release version outranks the upload time, and the case of the id does not part a lineage"
        );
    }

    #[test]
    fn a_template_with_no_concept_or_root_is_a_lineage_of_its_own() {
        let found = lineages(vec![
            row("a", "", "", "2026-09-01T09:00:00Z"),
            row("b", "", "", "2026-09-01T09:00:00Z"),
        ]);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn a_comparison_names_the_paths_added_removed_and_changed() {
        let mut coded = node("/content[at0001]/value", "DV_CODED_TEXT", Vec::new());
        coded.code_options = vec![CodeOption {
            code: "at0002".to_owned(),
            label: "Yes".to_owned(),
            ordinal: None,
        }];
        let mut recoded = coded.clone();
        recoded.code_options.push(CodeOption {
            code: "at0003".to_owned(),
            label: "No".to_owned(),
            ordinal: None,
        });
        let from = node(
            "/",
            "COMPOSITION",
            vec![
                coded,
                node("/content[at0004]/value", "DV_TEXT", Vec::new()),
                node("/content[at0005]/value", "DV_COUNT", Vec::new()),
            ],
        );
        let to = node(
            "/",
            "COMPOSITION",
            vec![
                recoded,
                node("/content[at0005]/value", "DV_COUNT", Vec::new()),
                node("/content[at0006]/value", "DV_QUANTITY", Vec::new()),
            ],
        );
        let comparison = compare(&from, &to);
        assert_eq!(
            (comparison.added, comparison.removed, comparison.changed),
            (1, 1, 1)
        );
        let keys: Vec<&str> = comparison
            .changes
            .iter()
            .map(|change| change.key.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "/content[at0001]/value",
                "/content[at0004]/value",
                "/content[at0006]/value"
            ]
        );
        let changed = comparison.changes.first().expect("a change");
        assert_eq!(
            changed.from.as_deref(),
            Some("value · DV_CODED_TEXT · at0002")
        );
        assert_eq!(
            changed.to.as_deref(),
            Some("value · DV_CODED_TEXT · at0002, at0003")
        );
        assert_eq!(compare(&to, &to), Comparison::default());
    }
}
