// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outer ADL artefact parser: sections, kind, meta, spans.
//!
//! Transcribed from the vendored `adl2.g4` (at
//! `crates/openehr-adl/vendor/grammar/`) plus the spec-text section extensions
//! the pinned grammar lacks. It produces a [`SourceArtefact`]: the artefact
//! kind, identification meta + HRID, the specialise parent reference, each
//! ODIN section parsed via `openehr_lang::v1_1::odin`, and the cADL `definition` /
//! `rules` bodies captured as **raw spans** (cADL parsing is a separate pass,
//! `crate::parse`).
//!
//! Section boundaries follow the grammar's `'\n'`-anchoring of the section
//! keywords (`adl_keywords.g4`): a section header is a keyword at column 0
//! (its preceding byte is a newline), so an identical word appearing indented
//! inside an ODIN section or the definition is never mistaken for a header.
//! Because a multi-line `STRING` is one lexer token, a section keyword inside a
//! quoted value can never read as a header either.

use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;

use crate::error::{SyntaxError, SyntaxErrorCode};
use crate::hrid::parse_hrid;
use crate::parse::Dialect;
use openehr_lang::v1_1::lexer::{Spanned, Token};

/// The artefact kind (first keyword of an ADL2 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtefactKind {
    /// `archetype`.
    Archetype,
    /// `template`.
    Template,
    /// `template_overlay`.
    TemplateOverlay,
    /// `operational_template` (also `operational_archetype`, per the
    /// `master07.04` keyword inconsistency).
    OperationalTemplate,
}

/// A raw, unparsed section body: its byte range in the source and its token
/// index range in the whole-file token stream (for a later cADL/rules phase).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSpan {
    /// Byte range of the body in the original source.
    pub bytes: std::ops::Range<usize>,
    /// Token index range of the body in the whole-file token stream.
    pub tokens: std::ops::Range<usize>,
}

/// Identification meta-data (`adl2.g4` `meta_data`; `master07.05`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtefactMeta {
    /// `adl_version=` (mandatory in a well-formed artefact).
    pub adl_version: Option<String>,
    /// `rm_release=` (mandatory in a well-formed artefact).
    pub rm_release: Option<String>,
    /// `uid=` (a GUID or an OID).
    pub uid: Option<String>,
    /// `build_uid=` (a GUID).
    pub build_uid: Option<String>,
    /// `provenance_id=`.
    pub provenance_id: Option<String>,
    /// `controlled` ⇒ `Some(true)`, `uncontrolled` ⇒ `Some(false)`.
    pub controlled: Option<bool>,
    /// The `generated` flag.
    pub generated: bool,
    /// Any other (unknown) meta items, preserved verbatim as `(key, value?)`.
    pub other: Vec<(String, Option<String>)>,
}

/// A parsed ADL2 source artefact (outer structure only; the definition/rules
/// bodies stay raw here, parsed on demand by `crate::parse` /
/// `openehr_lang::v1_1::odin`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceArtefact {
    /// The artefact kind.
    pub kind: ArtefactKind,
    /// Identification meta-data.
    pub meta: ArtefactMeta,
    /// The parsed HRID.
    pub hrid: ArchetypeHrid,
    /// The `specialise`/`specialize` parent reference, if any.
    pub parent_ref: Option<ArchetypeHrid>,
    /// The ADL **1.4** `concept` section's local term code, without its
    /// brackets (`concept [at0000]` ⇒ `"at0000"`).
    ///
    /// `AM/docs/ADL1.4/master08-adl` §Concept Section: "All archetypes
    /// represent some real world concept … The concept is always coded". The
    /// section is mandatory in the 1.4 grammar (§Syntax Specification
    /// `arch_concept`, which — unlike `arch_specialisation`/`arch_language`/
    /// `arch_description`/`arch_invariant` — has no empty alternative) and its
    /// term is the subject of §Validity Rules VARCN. It is captured for every
    /// dialect; ADL2 derives the concept from the HRID instead and ignores it
    /// (`ADL2/master07.09` lists `concept` among the obsolete clauses).
    pub concept: Option<String>,
    /// The `language` section (ODIN).
    pub language: Option<openehr_lang::v1_1::odin::OdinValue>,
    /// The `description` section (ODIN).
    pub description: Option<openehr_lang::v1_1::odin::OdinValue>,
    /// The `terminology` section (ODIN).
    pub terminology: Option<openehr_lang::v1_1::odin::OdinValue>,
    /// The `annotations` section (ODIN).
    pub annotations: Option<openehr_lang::v1_1::odin::OdinValue>,
    /// The `rm_overlay` section (ODIN; `master07.12`).
    pub rm_overlay: Option<openehr_lang::v1_1::odin::OdinValue>,
    /// The `component_terminologies` section (ODIN; OPT only).
    pub component_terminologies: Option<openehr_lang::v1_1::odin::OdinValue>,
    /// The ADL **1.4** `revision_history` section (ODIN;
    /// `AM/docs/ADL1.4/master08-adl` §Revision History Section: "The revision
    /// history section of an archetype shows the audit history of changes to
    /// the archetype, and is expressed in dADL syntax. It is optional, and is
    /// included at the end of the archetype").
    ///
    /// NOTE: it has no landing site in the assembled AOM2 model — deliberate
    /// upstream, not a generated-model gap (`AM/docs/ADL2/master01-preface`
    /// §Changes from ADL 1.4: "the `revision_history` section is removed";
    /// SPECAM-61) — so the section is read and preserved HERE at the 1.4
    /// source level and never carried into the converted ADL2 artefact.
    pub revision_history: Option<openehr_lang::v1_1::odin::OdinValue>,
    /// The `definition` (cADL) body as a raw span.
    pub definition: Option<RawSpan>,
    /// The `rules`/`invariant` body as a raw span.
    pub rules: Option<RawSpan>,
    /// Nested `template_overlay` artefacts (a `template` may carry many).
    pub overlays: Vec<SourceArtefact>,
}

/// Parse an ADL source into a [`SourceArtefact`], reading the outer structure
/// with the rules of `dialect`.
///
/// ODIN sections are delegated to `openehr_lang::v1_1::odin::parse`; the `definition`
/// and `rules` bodies are captured as [`RawSpan`]s. Recoverable errors are
/// collected; the whole error list is returned on any failure.
///
/// Three outer-structure behaviours differ under [`Dialect::Adl14`], each
/// 1.4-only so ADL2 parsing is byte-identical:
/// - Section and artefact keywords are case-insensitive
///   (`AM/docs/ADL1.4/master08-adl` §Syntax Specification/§Symbols). Column-0
///   anchoring is unchanged, so a keyword used as an identifier inside a section
///   is still never a header.
/// - A missing `language` section is accepted when the ontology carries the
///   old-form `primary_language` (§Language Section NOTE), which
///   `crate::assemble::assemble` upgrades; with nothing to upgrade from,
///   [`SyntaxErrorCode::Salan`] stands.
/// - A malformed `concept` clause is refused with [`SyntaxErrorCode::Saco`]
///   (§Syntax Specification `arch_concept`).
///
/// # Errors
/// Returns every [`SyntaxError`] found (lexical, identification, ODIN-section,
/// or missing-required-section). ODIN parse failures surface as
/// [`SyntaxErrorCode::Sdinv`] carrying the section name.
pub fn parse_source(src: &str, dialect: Dialect) -> Result<SourceArtefact, Vec<SyntaxError>> {
    let toks = match openehr_lang::v1_1::lexer::lex_adl(src) {
        Ok(t) => t,
        Err(failure) => return Err(vec![crate::error::lexical(&failure, src)]),
    };
    let mut outer = Outer {
        src,
        toks: &toks,
        pos: 0,
        dialect,
        errors: Vec::new(),
    };
    let artefact = outer.parse_artefact(false);
    if outer.errors.is_empty()
        && let Some(trailing) = outer.toks.get(outer.pos)
    {
        // Trailing tokens after a complete artefact.
        let span = trailing.span.clone();
        outer.push(SyntaxErrorCode::Sunk, "unexpected trailing input", span);
    }
    match artefact {
        Some(a) if outer.errors.is_empty() => Ok(a),
        _ => {
            if outer.errors.is_empty() {
                outer.errors.push(SyntaxError::at(
                    SyntaxErrorCode::Sunk,
                    "empty artefact",
                    0..0,
                    src,
                ));
            }
            Err(outer.errors)
        }
    }
}

/// A section keyword class (column-0 header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKw {
    Specialise,
    Language,
    Description,
    Definition,
    Rules,
    Terminology,
    Annotations,
    ComponentTerminologies,
    RmOverlay,
    /// The ADL 1.4-only `revision_history` clause
    /// (`AM/docs/ADL1.4/master08-adl` §Revision History Section).
    RevisionHistory,
    /// The obsolete `concept` clause (`master07.09`) — recognised as a section
    /// boundary and otherwise ignored in ADL2.
    Concept,
    /// A new artefact keyword (`template_overlay`, …) — an artefact boundary.
    ArtefactBoundary,
}

fn classify_section(s: &str) -> Option<SectionKw> {
    match s {
        "specialize" | "specialise" => Some(SectionKw::Specialise),
        "language" => Some(SectionKw::Language),
        "description" => Some(SectionKw::Description),
        "definition" => Some(SectionKw::Definition),
        // Deprecated `invariant` maps to the rules section (`master07.09`).
        "rules" | "invariant" => Some(SectionKw::Rules),
        // Deprecated `ontology` maps to the terminology section (`master07.09`).
        "terminology" | "ontology" => Some(SectionKw::Terminology),
        "annotations" => Some(SectionKw::Annotations),
        "component_terminologies" => Some(SectionKw::ComponentTerminologies),
        // ADL 1.4-only (`master08` §Revision History Section); removed in ADL2
        // (`ADL2/master01-preface` §Changes from ADL 1.4). Recognised so a
        // spec-valid 1.4 archetype carrying one parses instead of sinking on
        // "expected a section header".
        "revision_history" => Some(SectionKw::RevisionHistory),
        // Obsolete in ADL2 (concept derives from the HRID); recognised as a
        // boundary and ignored (`master07.09`).
        "concept" => Some(SectionKw::Concept),
        // NOTE: `rm_overlay` is a spec-text section (`ADL2/master07.12`) that
        // the vendored adl-antlr `adl2.g4` predates; recognised here as an ODIN
        // section per the spec text.
        "rm_overlay" => Some(SectionKw::RmOverlay),
        "archetype"
        | "template"
        | "template_overlay"
        | "operational_template"
        | "operational_archetype" => Some(SectionKw::ArtefactBoundary),
        _ => None,
    }
}

fn classify_kind(s: &str) -> Option<ArtefactKind> {
    match s {
        "archetype" => Some(ArtefactKind::Archetype),
        "template" => Some(ArtefactKind::Template),
        "template_overlay" => Some(ArtefactKind::TemplateOverlay),
        // NOTE: `operational_archetype` accepted as `operational_template` per
        // the `ADL2/master07.04` keyword inconsistency.
        "operational_template" | "operational_archetype" => Some(ArtefactKind::OperationalTemplate),
        _ => None,
    }
}

struct Outer<'a> {
    src: &'a str,
    toks: &'a [Spanned],
    pos: usize,
    dialect: Dialect,
    errors: Vec<SyntaxError>,
}

impl Outer<'_> {
    fn current(&self) -> Option<&Token> {
        self.toks.get(self.pos).map(|s| &s.token)
    }

    fn span_at(&self, idx: usize) -> std::ops::Range<usize> {
        self.toks
            .get(idx)
            .map_or(self.src.len()..self.src.len(), |s| s.span.clone())
    }

    fn push(
        &mut self,
        code: SyntaxErrorCode,
        msg: impl Into<String>,
        span: std::ops::Range<usize>,
    ) {
        self.errors.push(SyntaxError::at(code, msg, span, self.src));
    }

    /// True if the token at `idx` starts a line (its preceding byte is `\n`).
    fn is_line_start(&self, idx: usize) -> bool {
        let Some(sp) = self.toks.get(idx) else {
            return false;
        };
        let start = sp.span.start;
        start == 0 || self.src.as_bytes().get(start - 1) == Some(&b'\n')
    }

    /// The keyword spelling of the identifier token at `idx`, ready for
    /// classification: the token text as written in ADL2, and case-folded in
    /// the 1.4 dialect, where `master08` §Symbols spells every section keyword
    /// case-insensitively (`^[Aa][Rr][Cc][Hh][Ee][Tt][Yy][Pp][Ee]`, …). An
    /// upper-initial word lexes as [`Token::AlphaUcId`], so 1.4 reads both
    /// identifier tokens and ADL2 only the lower-initial one.
    fn keyword_at(&self, idx: usize) -> Option<String> {
        match self.toks.get(idx).map(|s| &s.token) {
            Some(Token::AlphaLcId(s)) if self.dialect == Dialect::Adl2 => Some(s.clone()),
            Some(Token::AlphaLcId(s) | Token::AlphaUcId(s)) if self.dialect == Dialect::Adl14 => {
                Some(s.to_ascii_lowercase())
            }
            _ => None,
        }
    }

    /// The section-keyword class at `idx`, if that token is a column-0 keyword.
    fn section_kw_at(&self, idx: usize) -> Option<SectionKw> {
        if !self.is_line_start(idx) {
            return None;
        }
        classify_section(&self.keyword_at(idx)?)
    }

    /// The artefact-kind keyword and its span, accepting an optional leading
    /// `flat` (`ADL2/master07.04` §Artefact declaration).
    ///
    /// The span is the KEYWORD's, taken after any `flat` prefix, because the
    /// caller reports missing-section defects against it.
    fn parse_artefact_kind(&mut self) -> Option<(ArtefactKind, std::ops::Range<usize>)> {
        if self.keyword_at(self.pos).as_deref() == Some("flat") {
            self.pos += 1;
        }
        let kind_span = self.span_at(self.pos);
        let Some(kind) = self.keyword_at(self.pos).as_deref().and_then(classify_kind) else {
            self.push(
                SyntaxErrorCode::Sunk,
                "expected an artefact keyword",
                kind_span,
            );
            return None;
        };
        self.pos += 1;
        Some((kind, kind_span))
    }

    /// The artefact's `ARCHETYPE_HRID`, which follows the kind keyword and its
    /// optional meta-data clause.
    fn parse_artefact_hrid(&mut self) -> Option<ArchetypeHrid> {
        let hrid_span = self.span_at(self.pos);
        let Some(Token::ArchetypeId(hrid_str)) = self.current().cloned() else {
            self.push(
                SyntaxErrorCode::Sarid,
                "expected an archetype HRID after the artefact keyword",
                hrid_span,
            );
            return None;
        };
        self.pos += 1;
        match parse_hrid(&hrid_str) {
            Ok(h) => Some(h),
            Err(msg) => {
                self.push(SyntaxErrorCode::Sarid, msg, hrid_span);
                None
            }
        }
    }

    fn parse_artefact(&mut self, overlay: bool) -> Option<SourceArtefact> {
        let (kind, kind_span) = self.parse_artefact_kind()?;
        let mut meta = ArtefactMeta::default();
        if matches!(self.current(), Some(Token::LParen)) {
            self.parse_meta(&mut meta);
        }
        let hrid = self.parse_artefact_hrid()?;

        let mut art = SourceArtefact {
            kind,
            meta,
            hrid,
            parent_ref: None,
            concept: None,
            language: None,
            description: None,
            terminology: None,
            annotations: None,
            rm_overlay: None,
            component_terminologies: None,
            revision_history: None,
            definition: None,
            rules: None,
            overlays: Vec::new(),
        };

        // Sections.
        loop {
            let Some(kw) = self.section_kw_at(self.pos) else {
                if self.pos >= self.toks.len() {
                    break;
                }
                // An unexpected token where a section header was expected.
                let span = self.span_at(self.pos);
                self.push(SyntaxErrorCode::Sunk, "expected a section header", span);
                break;
            };
            if kw == SectionKw::ArtefactBoundary {
                break;
            }
            let header_idx = self.pos;
            self.pos += 1; // consume the section keyword
            let body = self.section_body_range();
            self.process_section(kw, header_idx, body, &mut art);
        }

        // A `template` (and, defensively, any root) collects trailing
        // `template_overlay` blocks (`adl2.g4` `template`).
        if !overlay {
            while self.keyword_at(self.pos).as_deref() == Some("template_overlay")
                && self.is_line_start(self.pos)
            {
                if let Some(ov) = self.parse_artefact(true) {
                    art.overlays.push(ov);
                } else {
                    break;
                }
            }
        }

        self.validate_required(&art, overlay, kind_span);
        Some(art)
    }

    /// The token index range of a section body: from the current position up to
    /// the next column-0 section/artefact keyword (or end of input).
    fn section_body_range(&self) -> std::ops::Range<usize> {
        let start = self.pos;
        let mut end = start;
        while end < self.toks.len() && self.section_kw_at(end).is_none() {
            end += 1;
        }
        start..end
    }

    fn process_section(
        &mut self,
        kw: SectionKw,
        header_idx: usize,
        body: std::ops::Range<usize>,
        art: &mut SourceArtefact,
    ) {
        self.pos = body.end;
        match kw {
            SectionKw::Specialise => {
                if body.len() == 1
                    && let Some(Token::ArchetypeId(s)) = self.toks.get(body.start).map(|t| &t.token)
                {
                    match parse_hrid(s) {
                        Ok(h) => art.parent_ref = Some(h),
                        Err(msg) => {
                            self.push(SyntaxErrorCode::Sasid, msg, self.span_at(body.start));
                        }
                    }
                } else {
                    self.push(
                        SyntaxErrorCode::Sasid,
                        "expected a single parent archetype id in the specialise section",
                        self.span_at(header_idx),
                    );
                }
            }
            SectionKw::Definition => {
                if body.is_empty() {
                    self.push(
                        SyntaxErrorCode::Sadf,
                        "empty definition section",
                        self.span_at(header_idx),
                    );
                } else {
                    art.definition = Some(self.raw_span(body));
                }
            }
            SectionKw::Rules => {
                if !body.is_empty() {
                    art.rules = Some(self.raw_span(body));
                }
            }
            SectionKw::Language => {
                art.language = self.parse_odin(body, header_idx, "language");
            }
            SectionKw::Description => {
                art.description = self.parse_odin(body, header_idx, "description");
            }
            SectionKw::Terminology => {
                art.terminology = self.parse_odin(body, header_idx, "terminology");
            }
            SectionKw::Annotations => {
                art.annotations = self.parse_odin(body, header_idx, "annotations");
            }
            SectionKw::RmOverlay => {
                art.rm_overlay = self.parse_odin(body, header_idx, "rm_overlay");
            }
            SectionKw::ComponentTerminologies => {
                art.component_terminologies =
                    self.parse_odin(body, header_idx, "component_terminologies");
            }
            SectionKw::RevisionHistory => {
                art.revision_history = self.parse_odin(body, header_idx, "revision_history");
            }
            SectionKw::Concept => {
                art.concept = self.parse_concept(&body, header_idx);
            }
            SectionKw::ArtefactBoundary => {}
        }
    }

    /// The `concept` clause's local term code (`master08` §Concept Section;
    /// §Syntax Specification `arch_concept: SYM_CONCEPT V_LOCAL_TERM_CODE_REF`,
    /// lexed `\[[a-zA-Z0-9][a-zA-Z0-9.-]*\]` by §Symbols).
    ///
    /// A body that is not a term-code reference is refused with
    /// [`SyntaxErrorCode::Saco`] in BOTH dialects: the 1.4 grammar's
    /// `SYM_CONCEPT error` alternative, and ADL2's deprecated-but-allowed
    /// form's own rule — "if a concept section is present, it must consist of
    /// the 'concept' keyword and a single local term"
    /// (`ADL2/master07.09-adl_deprecated.adoc` §Concept Section). ADL2 still
    /// derives the concept from the HRID and ignores the captured code.
    fn parse_concept(
        &mut self,
        body: &std::ops::Range<usize>,
        header_idx: usize,
    ) -> Option<String> {
        let tokens: Vec<&Token> = self
            .toks
            .get(body.clone())
            .unwrap_or_default()
            .iter()
            .map(|t| &t.token)
            .collect();
        let code = match tokens.as_slice() {
            // `[at0000]` — the bracketed local term code.
            [Token::LBracket, inner, Token::RBracket] => local_code_text(inner),
            // `[local::at0000]` — the qualified term-code spelling of
            // `ADL1.4/master05-cadl.adoc` §Symbols `V_QUALIFIED_TERM_CODE_REF`
            // (the same bracketed form with a terminology prefix), read
            // tolerantly here for an archetype that writes its concept that way.
            [Token::TermCodeRef(t)] => t
                .trim_start_matches('[')
                .trim_end_matches(']')
                .rsplit("::")
                .next()
                .map(str::to_owned),
            _ => None,
        };
        if code.is_none() {
            // Both dialects diagnose a malformed clause: ADL 1.4 requires the
            // section (`ADL1.4/master08-adl.adoc` §Syntax Specification), and
            // ADL2 keeps SACO for the deprecated-but-allowed form — "if a
            // concept section is present, it must consist of the 'concept'
            // keyword and a single local term"
            // (`ADL2/master07.09-adl_deprecated.adoc` §Concept Section).
            self.push(
                SyntaxErrorCode::Saco,
                "expected a local term code reference in the concept section",
                self.span_at(header_idx),
            );
        }
        code
    }

    fn raw_span(&self, tokens: std::ops::Range<usize>) -> RawSpan {
        let bytes = self.span_at(tokens.start).start..self.span_at(tokens.end - 1).end;
        RawSpan { bytes, tokens }
    }

    /// Parse an ODIN section body (delegated to `openehr_lang::v1_1::odin`), mapping
    /// any failure to [`SyntaxErrorCode::Sdinv`] with the section name.
    fn parse_odin(
        &mut self,
        body: std::ops::Range<usize>,
        header_idx: usize,
        name: &str,
    ) -> Option<openehr_lang::v1_1::odin::OdinValue> {
        if body.is_empty() {
            self.push(
                SyntaxErrorCode::Sdinv,
                format!("empty {name} section"),
                self.span_at(header_idx),
            );
            return None;
        }
        let byte_span = self.span_at(body.start).start..self.span_at(body.end - 1).end;
        let text = self.src.get(byte_span.clone()).unwrap_or("");
        match openehr_lang::v1_1::odin::parse(text) {
            Ok(v) => Some(v),
            Err(e) => {
                // Offset the ODIN-local byte span into the whole-file source.
                let span = (byte_span.start + e.span.start)..(byte_span.start + e.span.end);
                self.push(
                    SyntaxErrorCode::Sdinv,
                    format!("invalid ODIN in {name} section: {e}"),
                    span,
                );
                None
            }
        }
    }

    fn parse_meta(&mut self, meta: &mut ArtefactMeta) {
        self.pos += 1; // consume '('
        while !matches!(self.current(), Some(Token::RParen) | None) {
            if matches!(self.current(), Some(Token::SymSemiColon)) {
                self.pos += 1;
                continue;
            }
            let Some(Token::AlphaLcId(key) | Token::AlphaUnderscoreId(key)) = self.current() else {
                // Unexpected meta token: skip to keep the parse going.
                self.pos += 1;
                continue;
            };
            let key = key.clone();
            self.pos += 1;
            let value = if matches!(self.current(), Some(Token::SymEq)) {
                self.pos += 1;
                let v = self.current().and_then(token_text);
                if v.is_some() {
                    self.pos += 1;
                }
                v
            } else {
                None
            };
            match key.as_str() {
                "adl_version" => meta.adl_version = value,
                "rm_release" => meta.rm_release = value,
                "uid" => meta.uid = value,
                "build_uid" => meta.build_uid = value,
                "provenance_id" => meta.provenance_id = value,
                "generated" => meta.generated = true,
                "controlled" => meta.controlled = Some(true),
                "uncontrolled" => meta.controlled = Some(false),
                _ => meta.other.push((key, value)),
            }
        }
        if matches!(self.current(), Some(Token::RParen)) {
            self.pos += 1;
        }
    }

    fn validate_required(
        &mut self,
        art: &SourceArtefact,
        overlay: bool,
        kind_span: std::ops::Range<usize>,
    ) {
        if art.definition.is_none() {
            self.push(
                SyntaxErrorCode::Sadf,
                "missing definition section",
                kind_span.clone(),
            );
        }
        if art.terminology.is_none() {
            self.push(
                SyntaxErrorCode::Saon,
                "missing terminology section",
                kind_span.clone(),
            );
        }
        // template_overlay inherits language/description from its root
        // (`master07.07` / `master10`); every other kind requires a language
        // section. A 1.4 source whose ontology carries `primary_language` is
        // accepted here and upgraded in `crate::assemble`; with nothing to
        // upgrade from, SALAN stands, and ADL2 keeps SALAN unconditionally.
        //
        // NOTE: ADL 1.4's `arch_language` carries an empty alternative
        // (`master08` §Syntax Specification), and §Language Section directs
        // tools to accept the old `ontology`-only form and upgrade it.
        let old_form_language = self.dialect == Dialect::Adl14
            && art.terminology.as_ref().is_some_and(has_primary_language);
        if !overlay
            && art.kind != ArtefactKind::TemplateOverlay
            && art.language.is_none()
            && !old_form_language
        {
            self.push(
                SyntaxErrorCode::Salan,
                "missing language section",
                kind_span,
            );
        }
    }
}

/// The bare code text of a local term-code reference's inner token
/// (`[at0000]` ⇒ `at0000`), for the forms `master08` §Symbols
/// `V_LOCAL_TERM_CODE_REF` (`\[[a-zA-Z0-9][a-zA-Z0-9.-]*\]`) admits.
fn local_code_text(t: &Token) -> Option<String> {
    match t {
        Token::AtCode(s)
        | Token::AcCode(s)
        | Token::IdCode(s)
        | Token::RootIdCode(s)
        | Token::AlphaLcId(s)
        | Token::AlphaUcId(s)
        | Token::Integer(s) => Some(s.clone()),
        _ => None,
    }
}

/// True if an ODIN `ontology`/`terminology` section carries the old-form
/// `primary_language` statement (`master08` §Ontology Header Statements NOTE).
fn has_primary_language(section: &openehr_lang::v1_1::odin::OdinValue) -> bool {
    matches!(section, openehr_lang::v1_1::odin::OdinValue::Object(map) if map.contains_key("primary_language"))
}

/// The text a meta value token carries, if any.
fn token_text(t: &Token) -> Option<String> {
    match t {
        Token::VersionId(s)
        | Token::Guid(s)
        | Token::String(s)
        | Token::Integer(s)
        | Token::Real(s)
        | Token::ArchetypeId(s)
        | Token::AlphaLcId(s)
        | Token::AlphaUcId(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_archetype() {
        let src = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
                   \topenehr-TEST_PKG-WHOLE.most_minimal.v2.0.0\n\
                   \nlanguage\n\toriginal_language = <[ISO_639-1::en]>\n\
                   \ndescription\n\tlifecycle_state = <\"published\">\n\
                   \ndefinition\n\tWHOLE[id1]\n\
                   \nterminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t\t[\"id1\"] = <\n\t\t\t\ttext = <\"x\">\n\t\t\t\tdescription = <\"x\">\n\t\t\t>\n\t\t>\n\t>\n";
        let a = parse_source(src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse failed: {e:?}"));
        assert_eq!(a.kind, ArtefactKind::Archetype);
        assert_eq!(a.meta.adl_version.as_deref(), Some("2.0.5"));
        assert_eq!(a.meta.rm_release.as_deref(), Some("1.0.2"));
        assert_eq!(a.hrid.rm_publisher, "openehr");
        assert_eq!(a.hrid.rm_package, "TEST_PKG");
        assert_eq!(a.hrid.rm_class, "WHOLE");
        assert_eq!(a.hrid.concept_id, "most_minimal");
        assert_eq!(a.hrid.release_version, "2.0.0");
        assert!(a.language.is_some());
        assert!(a.description.is_some());
        assert!(a.terminology.is_some());
        let def = a.definition.expect("definition present");
        assert!(!def.tokens.is_empty());
        // the raw definition span covers `WHOLE[id1]`.
        assert_eq!(src.get(def.bytes.clone()), Some("WHOLE[id1]"));
    }

    #[test]
    fn missing_definition_is_sadf() {
        let src = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
                   \topenehr-TEST_PKG-WHOLE.x.v1.0.0\n\
                   \nlanguage\n\toriginal_language = <[ISO_639-1::en]>\n\
                   \ndescription\n\tlifecycle_state = <\"published\">\n\
                   \nterminology\n\tterm_definitions = <\n\t\t[\"en\"] = <>\n\t>\n";
        let errs = parse_source(src, Dialect::Adl2).expect_err("should fail");
        assert!(
            errs.iter().any(|e| e.code == SyntaxErrorCode::Sadf),
            "{errs:?}"
        );
    }
}
