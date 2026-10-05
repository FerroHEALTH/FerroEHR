// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Phase-1 source-level topic: the two checks that read the RAW parsed source
//! rather than the assembled AOM model.
//!
//! * **VOKU** — within any ODIN keyed list each item must have a unique key
//!   (`docs/specs/openehr/AM/docs/AOM2/master03-archetype_package.adoc`
//!   §Validity Rules). It must run on the parsed ODIN because the assembled
//!   model's `BTreeMap`s already dedupe keys.
//! * **VRRLP** — each path mentioned in a rule must be found within the
//!   archetype (`master03` §Validity Rules). The `rules` section is preserved
//!   as source text, so the path literals are scanned out of it.
//!
//! Both are therefore reachable only from the source-level entry points
//! ([`super::validate_source_integrity`], [`super::validate_adl14_source`],
//! [`super::validate_source`]), which carry
//! the [`SourceArtefact`] alongside the assembled archetype.

use std::collections::BTreeSet;

use openehr_lang::v1_1::odin::OdinValue;

use super::ValidationIssue;
use super::catalogue::ValidationCode;
use crate::artefact::ArchetypeView;
use crate::odin::key_str;
use crate::paths::{Resolution, resolve};
use crate::source::SourceArtefact;

/// VRRLP: each path mentioned in a rule must be found within the archetype
/// (master03 §Validity Rules).
///
/// NOTE: implemented by scanning the raw `rules` section text for node-id-
/// predicated path literals and resolving them; the RM-valid-extension half is
/// a reference-model concern (`super::rm`). Pure-RM rule paths are accepted
/// here.
pub(super) fn check_rule_paths(
    v: &ArchetypeView<'_>,
    src: &SourceArtefact,
    source_text: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(span) = src.rules.as_ref() else {
        return;
    };
    let Some(text) = source_text.get(span.bytes.clone()) else {
        return;
    };
    for path in scan_predicated_paths(text) {
        if resolve(v.definition, &path) == Resolution::NotFound {
            issues.push(
                ValidationIssue::new(
                    ValidationCode::Vrrlp,
                    format!("rule path {path:?} is not found within the archetype"),
                )
                .at_path(path.clone()),
            );
        }
    }
}

/// VOKU: within any ODIN keyed list, each item must have a unique key
/// (master03 §Validity Rules). Checked on the raw parsed ODIN (the assembled
/// model's `BTreeMap`s already dedupe keys).
pub(super) fn check_object_key_unique(src: &SourceArtefact, issues: &mut Vec<ValidationIssue>) {
    for section in [
        // The `language` section is keyed too — `master07.07` types
        // `translations` as `Hash<TRANSLATION_DETAILS, String>`, so a repeated
        // `translations = <["de"] = <…> ["de"] = <…>>` key is the same VOKU
        // violation as a repeated terminology code, and went unreported while
        // this section was left out of the scanned set.
        src.language.as_ref(),
        src.description.as_ref(),
        src.terminology.as_ref(),
        src.annotations.as_ref(),
        src.rm_overlay.as_ref(),
        src.component_terminologies.as_ref(),
        src.revision_history.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        check_odin_key_unique(section, issues);
    }
}

fn check_odin_key_unique(value: &OdinValue, issues: &mut Vec<ValidationIssue>) {
    match value {
        OdinValue::KeyedList(items) => {
            let mut seen = BTreeSet::new();
            for (k, val) in items {
                let key = key_str(k);
                if !seen.insert(key.clone()) {
                    issues.push(ValidationIssue::new(
                        ValidationCode::Voku,
                        format!("duplicate key {key:?} in a keyed list"),
                    ));
                }
                check_odin_key_unique(val, issues);
            }
        }
        OdinValue::Object(map) => {
            for val in map.values() {
                check_odin_key_unique(val, issues);
            }
        }
        OdinValue::List(items) => {
            for val in items {
                check_odin_key_unique(val, issues);
            }
        }
        // A `(TYPE)` cast is a parser hint, not a level of the data
        // (`LANG/docs/odin/master05-content` §Adding Type Information) — walk
        // through it so a cast block's keys are checked like any other.
        OdinValue::Typed { value, .. } => check_odin_key_unique(value, issues),
        _ => {}
    }
}

/// Scan free text for node-id-predicated archetype path literals
/// (`/…[idN]…`), for the raw-text VRRLP check.
fn scan_predicated_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'/') {
            let start = i;
            i += 1;
            while bytes
                .get(i)
                .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'{')
            {
                i += 1;
            }
            if let Some(seg) = text.get(start..i)
                && (seg.contains("[id") || seg.contains("[at"))
            {
                out.push(seg.trim_end_matches(['/', ',']).to_owned());
            }
        } else {
            i += 1;
        }
    }
    out
}

/// W14DEP: the paren-less inline dADL domain-block spelling is deprecated —
/// `ADL1.4/master05-cadl.adoc` §Symbols `V_C_DOMAIN_TYPE` marks `Type <`
/// "deprecated" and `(Type) <` "correct ADL 1.4/ADL 1.5". The form stays
/// ACCEPTED (the normative grammar defines only the paren-less spelling, and
/// the live CKM library uses it exclusively — the docs-vs-grammar inversion is
/// reported upstream); this check surfaces the deprecation at exactly its
/// spec strength: a warning naming the preferred spelling, per occurrence.
///
/// Token-level scan (comments and string literals are not tokens, so neither
/// can false-positive): a lowered domain-type name (`C_DV_QUANTITY`,
/// `C_DV_ORDINAL`, `C_CODE_PHRASE`) followed by `<`, with no `(` immediately
/// before the name.
pub(super) fn check_deprecated_domain_spelling_adl14(
    text: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    use openehr_lang::v1_1::lexer::{Token, lex_adl};
    // A lex failure never reaches here on the validation path (the artefact
    // already parsed), so an Err simply yields no warnings.
    let Ok(tokens) = lex_adl(text) else { return };
    for (i, spanned) in tokens.iter().enumerate() {
        let Token::AlphaUcId(name) = &spanned.token else {
            continue;
        };
        if !crate::adl14::domain::is_adl14_domain_type(name.as_str()) {
            continue;
        }
        let followed_by_lt = matches!(tokens.get(i + 1).map(|s| &s.token), Some(Token::SymLt));
        let preceded_by_paren = i
            .checked_sub(1)
            .and_then(|j| tokens.get(j))
            .is_some_and(|s| matches!(s.token, Token::LParen));
        if followed_by_lt && !preceded_by_paren {
            issues.push(ValidationIssue {
                code: ValidationCode::W14dep,
                severity: ValidationCode::W14dep.severity(),
                message: format!(
                    "the paren-less domain-block spelling `{name} <…>` is deprecated \
                     (master05-cadl §Symbols); write `({name}) <…>`"
                ),
                path: None,
                span: Some(spanned.span.clone()),
            });
        }
    }
}
