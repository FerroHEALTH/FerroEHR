// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The artefact header sections: the identification line (`ADL2/master07.04`),
//! `language` (`master07.07`), `description` (`master07.08`), `annotations`
//! (`master07.14`), `rm_overlay` (`master07.12`), and the OPT-only
//! `component_terminologies` block (`OPT2/master10`).

use std::collections::BTreeMap;
use std::fmt::Write;

use openehr_am::v2_4::aom2::rm_overlay::rm_overlay::RmOverlay;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::prelude::{
    ResourceAnnotations, ResourceDescriptionItem, TerminologyCode, TranslationDetails, Uuid,
};

use crate::hrid::hrid_to_string;
use crate::print::odin::{quoted, term_code_str};
use crate::print::{Parts, Printer, Translations};

impl Printer {
    pub(super) fn identification(&mut self, parts: &Parts<'_>) {
        // A flattened artefact prints with the `flat` keyword prefix
        // (`ADL2/master07.04` §Artefact declaration: "The flattened form … starts
        // with the keyword 'flat' followed by the artefact type").
        let keyword_owned;
        let keyword: &str = if parts.flat {
            keyword_owned = format!("flat {}", parts.keyword);
            &keyword_owned
        } else {
            parts.keyword
        };
        let meta = meta_clause(parts);
        if meta.is_empty() {
            self.line(0, keyword);
        } else {
            self.line(0, &format!("{keyword} ({meta})"));
        }
        self.line(1, &hrid_to_string(parts.archetype_id));
    }

    // ── language (master07.07) ──────────────────────────────────────────────
    pub(super) fn language(
        &mut self,
        original: &TerminologyCode,
        translations: Option<&Translations>,
    ) {
        self.blank();
        self.line(0, "language");
        self.line(
            1,
            &format!("original_language = <{}>", term_code_str(original)),
        );
        if let Some(tr) = translations
            && !tr.is_empty()
        {
            self.line(1, "translations = <");
            for (lang, td) in tr {
                self.translation(lang, td, 2);
            }
            self.line(1, ">");
        }
    }

    fn translation(&mut self, lang: &str, td: &TranslationDetails, depth: usize) {
        self.line(depth, &format!("[{}] = <", quoted(lang)));
        self.line(
            depth + 1,
            &format!("language = <{}>", term_code_str(&td.language)),
        );
        self.odin_string_map(depth + 1, "author", &td.author);
        if let Some(a) = &td.accreditation {
            self.line(depth + 1, &format!("accreditation = <{}>", quoted(a)));
        }
        if let Some(v) = &td.version_last_translated {
            self.line(
                depth + 1,
                &format!("version_last_translated = <{}>", quoted(v)),
            );
        }
        self.odin_string_list(
            depth + 1,
            "other_contributors",
            td.other_contributors.as_deref().unwrap_or_default(),
        );
        if let Some(od) = &td.other_details {
            self.odin_string_map(depth + 1, "other_details", od);
        }
        self.line(depth, ">");
    }

    // ── description (master07.08) ───────────────────────────────────────────
    pub(super) fn description(&mut self, d: Option<&ResourceDescription>) {
        let Some(d) = d else { return };
        self.blank();
        self.line(0, "description");
        if let Some(t) = &d.title {
            self.line(1, &format!("title = <{}>", quoted(t)));
        }
        self.odin_string_map(1, "original_author", &d.original_author);
        self.opt_string(1, "original_namespace", d.original_namespace.as_deref());
        self.opt_string(1, "original_publisher", d.original_publisher.as_deref());
        self.odin_string_list(
            1,
            "other_contributors",
            d.other_contributors.as_deref().unwrap_or_default(),
        );
        self.line(
            1,
            &format!("lifecycle_state = <{}>", quoted(&d.lifecycle_state)),
        );
        self.opt_string(1, "custodian_namespace", d.custodian_namespace.as_deref());
        self.opt_string(
            1,
            "custodian_organisation",
            d.custodian_organisation.as_deref(),
        );
        self.opt_string(1, "copyright", d.copyright.as_deref());
        self.opt_string(1, "licence", d.licence.as_deref());
        self.opt_string(1, "resource_package_uri", d.resource_package_uri.as_deref());
        if let Some(m) = &d.ip_acknowledgements {
            self.odin_string_map(1, "ip_acknowledgements", m);
        }
        if let Some(m) = &d.references {
            self.odin_string_map(1, "references", m);
        }
        if let Some(m) = &d.conversion_details {
            self.odin_string_map(1, "conversion_details", m);
        }
        if let Some(details) = &d.details {
            self.line(1, "details = <");
            for (lang, item) in details {
                self.description_item(lang, item, 2);
            }
            self.line(1, ">");
        }
        if let Some(m) = &d.other_details {
            self.odin_string_map(1, "other_details", m);
        }
    }

    fn description_item(&mut self, lang: &str, item: &ResourceDescriptionItem, depth: usize) {
        self.line(depth, &format!("[{}] = <", quoted(lang)));
        self.line(
            depth + 1,
            &format!("language = <{}>", term_code_str(&item.language)),
        );
        self.line(depth + 1, &format!("purpose = <{}>", quoted(&item.purpose)));
        self.odin_string_list(
            depth + 1,
            "keywords",
            item.keywords.as_deref().unwrap_or_default(),
        );
        if let Some(u) = &item.use_ {
            self.line(depth + 1, &format!("use = <{}>", quoted(u)));
        }
        if let Some(m) = &item.misuse {
            self.line(depth + 1, &format!("misuse = <{}>", quoted(m)));
        }
        if let Some(m) = &item.original_resource_uri {
            self.odin_string_map(depth + 1, "original_resource_uri", m);
        }
        if let Some(m) = &item.other_details {
            self.odin_string_map(depth + 1, "other_details", m);
        }
        self.line(depth, ">");
    }

    // ── annotations (master07.14) + rm_overlay (master07.12) ────────────────
    pub(super) fn annotations(&mut self, a: &ResourceAnnotations) {
        if a.documentation.is_empty() {
            return;
        }
        self.blank();
        self.line(0, "annotations");
        self.line(1, "documentation = <");
        for (lang, paths) in &a.documentation {
            self.line(2, &format!("[{}] = <", quoted(lang)));
            for (path, tags) in paths {
                self.line(3, &format!("[{}] = <", quoted(path)));
                for (tag, value) in tags {
                    self.line(4, &format!("[{}] = <{}>", quoted(tag), quoted(value)));
                }
                self.line(3, ">");
            }
            self.line(2, ">");
        }
        self.line(1, ">");
    }

    pub(super) fn rm_overlay(&mut self, rm: &RmOverlay) {
        let Some(vis) = &rm.rm_visibility else { return };
        if vis.is_empty() {
            return;
        }
        self.blank();
        self.line(0, "rm_overlay");
        self.line(1, "rm_visibility = <");
        for (path, v) in vis {
            self.line(2, &format!("[{}] = <", quoted(path)));
            if let Some(visibility) = &v.visibility {
                self.line(
                    3,
                    &format!("visibility = <{}>", quoted(visibility.as_str())),
                );
            }
            if let Some(alias) = &v.alias {
                self.line(3, &format!("alias = <{}>", term_code_str(alias)));
            }
            self.line(2, ">");
        }
        self.line(1, ">");
    }

    pub(super) fn component_terminologies(&mut self, ct: &BTreeMap<String, ArchetypeTerminology>) {
        self.blank();
        self.line(0, "component_terminologies");
        // A bare ODIN keyed-list block (no `attr =`), keyed by archetype id
        // (`master10`; the OPT section holds `id → ARCHETYPE_TERMINOLOGY`).
        self.line(1, "<");
        for (id, term) in ct {
            self.line(2, &format!("[{}] = <", quoted(id)));
            self.terminology_body(term, 3);
            self.line(2, ">");
        }
        self.line(1, ">");
    }
}

/// Append one `key=value` item to the identification-line meta-data list,
/// separating it from any preceding item with `; `.
/// The `(…)` meta-data clause of the artefact declaration: the ADL and RM
/// versions, the identifiers, and the standardised flags, `;`-separated in
/// declaration order (`ADL2/master07.04` §Artefact declaration).
///
/// A nil `uid`/`build_uid` is the absent-value spelling the assembler stores
/// and never prints.
fn meta_clause(parts: &Parts<'_>) -> String {
    let mut meta = String::new();
    if let Some(adl) = parts.adl_version {
        let _ = write!(meta, "adl_version={adl}");
    }
    if let Some(rm) = parts.rm_release
        && !rm.is_empty()
    {
        push_meta(&mut meta, &format!("rm_release={rm}"));
    }
    if let Some(uid) = parts.uid
        && !is_nil(uid)
    {
        push_meta(&mut meta, &format!("uid={}", uid.value()));
    }
    if let Some(build) = parts.build_uid
        && !is_nil(build)
    {
        push_meta(&mut meta, &format!("build_uid={}", build.value()));
    }
    if parts.is_generated {
        push_meta(&mut meta, "generated");
    }
    if let Some(controlled) = parts.is_controlled {
        push_meta(
            &mut meta,
            if controlled {
                "controlled"
            } else {
                "uncontrolled"
            },
        );
    }
    for (k, v) in parts.other_meta_data.into_iter().flatten() {
        push_meta(&mut meta, &format!("{k}={v}"));
    }
    meta
}

fn push_meta(meta: &mut String, item: &str) {
    if !meta.is_empty() {
        meta.push_str("; ");
    }
    meta.push_str(item);
}

/// Whether a UUID is the nil UUID — the absent-value spelling the assembler
/// stores for a missing `uid`/`build_uid`, which never prints.
fn is_nil(u: &Uuid) -> bool {
    u.value().is_nil()
}
