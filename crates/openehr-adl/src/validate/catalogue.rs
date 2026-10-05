// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM2 validity-code catalogue: [`Severity`] plus one [`ValidationCode`]
//! variant per validity rule.
//!
//! The code set and its phase groupings come from
//! `docs/specs/openehr/AM/docs/AOM2/master08-validation.adoc`; the full rule
//! texts live in `master03-archetype_package.adoc`,
//! `master04.5-constraint_model-class_definitions.adoc`,
//! `master06-rm_overlay.adoc` and `master07-terminology_package.adoc`. Each
//! variant's own doc comment names the spec file + section that defines it and,
//! where the check is not raised by the phase-1 walk, the topic module that
//! raises it.

/// The severity of a [`ValidationIssue`](super::ValidationIssue).
///
/// The `W`-prefixed codes (WACMCL, WOUC) are warnings; every other code is an
/// error. `master08` assigns no explicit severity column, so this follows the
/// `V`/`W` naming convention (the `W` prefix = advisory "should"; see
/// `master04.5` WACMCL "should be" vs VACMCU "must").
///
/// NOTE: no openEHR spec states the `W`→Warning convention normatively; it is
/// inferred from the code naming (`master08-validation` is silent on severity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// A validity error — the archetype is invalid.
    Error,
    /// A validity warning — advisory, does not invalidate the archetype.
    Warning,
}

/// An AOM2 validation code (one typed variant per validity rule).
///
/// Each variant's doc comment names the spec file + section that defines it.
/// The catalogue is the phase-1 set of `docs/specs/openehr/AM/docs/AOM2/`
/// plus the two corpus-adjudicated additions (VRDLA, WOUC — no openEHR spec
/// names either code, NOTE-flagged at their variants and check sites).
/// Deferred variants (their check needs the
/// RM model, the flat parent, or an external terminology service) are present
/// as the vocabulary but not raised in phase 1 — see the phase-1 topic modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValidationCode {
    /// VARDT — archetype definition typename validity (`master03` §Validity Rules).
    Vardt,
    /// VARCN — archetype concept validity (`master03` §Validity Rules).
    Varcn,
    /// STCNT — missing mandatory part, e.g. terminology (`master08` §Phase 1; no
    /// full vendored text — NOTE-flagged).
    Stcnt,
    /// VACSD — archetype concept specialisation depth (`master03` §Validity Rules).
    Vacsd,
    /// VOLT — original language available in terminology (`master08` §Phase 1; no
    /// full vendored text — NOTE-flagged).
    Volt,
    /// VARAV — ADL version validity (`master03` §Validity Rules).
    Varav,
    /// VARRV — RM release validity (`master03` §Validity Rules).
    Varrv,
    /// VOTM — terminology translations validity (`master03` §Validity Rules).
    Votm,
    /// VDIFV — differential path only in specialised archetype (`master04.5`
    /// §`C_ATTRIBUTE`).
    Vdifv,
    /// VDIFP — differential path exists in flat parent (`master04.5` §`C_ATTRIBUTE`;
    /// checked in `specialisation` by resolving the differential path against the flat
    /// parent, with the RM-path half subsumed by that resolution — see [`rm`](super::rm)).
    Vdifp,
    /// VCORM — object constraint type name exists in the RM (`master04.5`
    /// §`C_OBJECT`; checked in [`rm`](super::rm)).
    Vcorm,
    /// VCORMT — object type conforms to the owning attribute's RM type
    /// (`master04.5` §`C_OBJECT`; checked in [`rm`](super::rm)).
    Vcormt,
    /// VCARM — attribute name exists on the enclosing RM type (`master04.5`
    /// §`C_ATTRIBUTE`; checked in [`rm`](super::rm)).
    Vcarm,
    /// VCAEX — attribute existence conforms to the RM existence (`master04.5`
    /// §`C_ATTRIBUTE`; checked in [`rm`](super::rm)).
    Vcaex,
    /// VCACA — attribute cardinality conforms to the RM cardinality (`master04.5`
    /// §`C_ATTRIBUTE`; checked in [`rm`](super::rm)).
    Vcaca,
    /// VCAM — attribute single/multiple arity matches the RM (`master04.5`
    /// §`C_ATTRIBUTE`; checked in [`rm`](super::rm)).
    Vcam,
    /// VCORMEN — enumeration type constraint kind validity: a primitive
    /// constraint on an enumeration-typed RM slot must match the enumeration's
    /// underlying primitive (an integer constraint on a string-based
    /// enumeration, or vice versa, is invalid). `master08` §Phase 2 lists
    /// (VCORMENV, VCORMENU, VCORMEN) with no full vendored text; the V/U/EN
    /// partition below is our reading of that gloss against `master04.2`
    /// §Constraints on Enumeration Types — NOTE-flagged in [`rm`](super::rm).
    Vcormen,
    /// VCORMENV — enumeration integer-value validity: an integer constraint
    /// value on an integer-based enumeration slot must be a declared literal
    /// value (`master08` §Phase 2 + `master04.2` §Constraints on Enumeration
    /// Types; spec-silent full text — NOTE-flagged in [`rm`](super::rm)).
    Vcormenv,
    /// VCORMENU — enumeration string-value validity: a string constraint value
    /// on a string-based enumeration slot must be a declared literal value
    /// (`master08` §Phase 2 + `master04.2` §Constraints on Enumeration Types;
    /// spec-silent full text — NOTE-flagged in [`rm`](super::rm)).
    Vcormenu,
    /// VATCV — terminology code format validity (`master08` §Phase 1; no full
    /// vendored text — NOTE-flagged).
    Vatcv,
    /// VTSD — specialisation level of codes (`master07` §Validity Rules).
    Vtsd,
    /// VTLC — terminology language consistency (`master07` §Validity Rules).
    Vtlc,
    /// VTTBK — term binding key valid (`master07` §Validity Rules).
    Vttbk,
    /// VTCBK — constraint binding key valid (`master07` §Validity Rules).
    Vtcbk,
    /// VETDF — external term validity (`master03` §Validity Rules): a code bound
    /// to an *external* terminology (SNOMED CT, LOINC, …) must exist in that
    /// terminology. `openehr-adl` (a network-free spec engine) cannot hold a
    /// live terminology-service client, so the check is threaded through the
    /// [`bindings::TerminologyResolver`](super::bindings::TerminologyResolver) seam: the application injects a
    /// resolver over its terminology service and [`validate`](super::validate) / [`validate_source`](super::validate_source)
    /// consult it. Entry points that take no resolver do not raise VETDF
    /// (`master03` "subject to tool accessibility; … no verification was
    /// possible").
    Vetdf,
    /// VTVSID — value-set id defined (`master07` §Validity Rules).
    Vtvsid,
    /// VTVSMD — value-set members defined (`master07` §Validity Rules).
    Vtvsmd,
    /// VTVSUQ — value-set members unique (`master07` §Validity Rules).
    Vtvsuq,
    /// VDSEV — slot 'exclude' constraint validity (`master04.5` §`ARCHETYPE_SLOT`).
    Vdsev,
    /// VDSIV — slot 'include' constraint validity (`master04.5` §`ARCHETYPE_SLOT`).
    Vdsiv,
    /// VARXRA — `C_ARCHETYPE_ROOT` validity set (`master08` §Phase 1; umbrella for
    /// VARXNC/VARXAV/VARXR — no full vendored text, NOTE-flagged).
    Varxra,
    /// VARXNC — `C_ARCHETYPE_ROOT` node-id conformance (`master08` §Phase 1).
    Varxnc,
    /// VARXAV — `C_ARCHETYPE_ROOT` archetype-ref validity (`master08` §Phase 1).
    Varxav,
    /// VARXR — external reference resolution (`master08` §Phase 2; checked in
    /// [`slots`](super::slots) by resolving each `use_archetype` reference against the
    /// supplier repository).
    Varxr,
    /// VARXTV — `C_ARCHETYPE_ROOT` type validity (`master08` §Phase 1).
    Varxtv,
    /// VATID — all definition codes defined in terminology (`master08` §Phase 1;
    /// no full vendored text — NOTE-flagged).
    Vatid,
    /// VATCD — archetype code specialisation level validity (`master03` §Validity
    /// Rules).
    Vatcd,
    /// VATDF — value code (at-code) validity (`master03` §Validity Rules; a
    /// non-specialised archetype is checked in `terminology`, the specialised
    /// flat-form half against the flattened terminology in `flat`).
    Vatdf,
    /// VACDF — constraint code (ac-code) validity (`master03` §Validity Rules).
    Vacdf,
    /// VATDA — value-set assumed value code validity (`master03` §Validity Rules).
    Vatda,
    /// VRANP — annotation path valid (`master03` §Validity Rules; the RM-path half
    /// is a reference-model check, [`rm`](super::rm)).
    Vranp,
    /// VOKU — object key unique in keyed lists (`master03` §Validity Rules).
    Voku,
    /// VARID — archetype identifier validity (`master03` §Validity Rules).
    Varid,
    /// VDEOL — original language specified (`master03` §Validity Rules).
    Vdeol,
    /// VARD — description specified (`master03` §Validity Rules).
    Vard,
    /// VASID — specialisation parent identifier validity (`master03` §Validity
    /// Rules; needs a repository: a stated parent that is absent from it, or
    /// that is not the immediate parent, both fail).
    Vasid,
    /// VALC — archetype language conformance (`master03` §Validity Rules; fires
    /// only when the parent is supplied).
    Valc,
    /// VTPL — template/filler language consistency (`master03` §Validity Rules;
    /// checked in [`slots`](super::slots) against the resolved, flattened fillers).
    Vtpl,
    /// VRRLP — rule path valid (`master03` §Validity Rules; the RM-extension half
    /// is a reference-model check, [`rm`](super::rm)).
    Vrrlp,
    /// VCOCD — object constraint definition validity (`master04.5` §`C_OBJECT`).
    Vcocd,
    /// VCOID — object node identifier present (`master04.5` §`C_OBJECT`).
    Vcoid,
    /// VCOSU — object node identifier unique (`master04.5` §`C_OBJECT`).
    Vcosu,
    /// VCATU — sibling attribute uniqueness (`master04.5` §`C_COMPLEX_OBJECT`).
    Vcatu,
    /// VDFAI — archetype id validity in slot definition (`master04.5`
    /// §`ARCHETYPE_SLOT`).
    Vdfai,
    /// VDFPT — path validity in definition: any path mentioned in the
    /// definition section must be valid syntactically and valid with respect
    /// to the hierarchical structure of the definition section (ADL 1.4 only;
    /// `ADL1.4/master08-adl.adoc` §Definition Section validity rules — the
    /// AOM2 mirror is [`ValidationCode::Vunp`] on the flat form).
    Vdfpt,
    /// VOBAV — object node assumed value validity (`master04.5`
    /// §`C_PRIMITIVE_OBJECT`).
    Vobav,
    /// VRMVP — RM-visibility path validity (`master06` §Validity).
    Vrmvp,
    /// VRMVAV — RM-visibility alias validity (`master06` §Validity).
    Vrmvav,
    /// VACSO — single-valued attribute child occurrences validity (`master04.5`
    /// §`C_ATTRIBUTE`).
    Vacso,
    /// VACMCU — cardinality/occurrences upper bound validity (`master04.5`
    /// §`C_ATTRIBUTE`).
    Vacmcu,
    /// VCOC — ADL 1.4 cardinality/occurrences validity: the interval formed by the
    /// sums of the children's occurrences minima..maxima must be inside the
    /// container's cardinality interval (`ADL1.4/master05-cadl.adoc` §Occurrences
    /// L321-324). The ADL 1.4 formalism's own rule, raised only on the 1.4
    /// dialect; the AOM2 successor is the VACMCU/WACMCL pair.
    Vcoc,
    /// VACMCO — cardinality/occurrences orphans: every mandatory child and one
    /// optional child must fit within the container cardinality (`master04.5`
    /// §`C_ATTRIBUTE` VACMCO L158-159; a phase-3 flat-form check, `flat`).
    Vacmco,
    /// VSONIF — object node identification validity in flat siblings (`master04.5`
    /// §`C_OBJECT` VSONIF L356-357; refs the spec-undefined VACMI). Checked in
    /// `specialisation`: a new object node in a specialised container whose flattened
    /// siblings are identified must itself be identified.
    Vsonif,
    /// VRDLA — resource-description language-code consistency (no openEHR spec
    /// governs this — our own design/extension, NOTE-flagged at its check site).
    Vrdla,
    /// WACMCL — cardinality/occurrences lower bound warning (`master04.5`
    /// §`C_ATTRIBUTE`; WARNING).
    Wacmcl,
    /// WOUC — defined terminology code unused in the definition (no openEHR spec
    /// governs this — our own design/extension; WARNING).
    Wouc,
    /// W14DEP — a deprecated ADL 1.4 spelling was used
    /// (`ADL1.4/master05-cadl.adoc` §Symbols `V_C_DOMAIN_TYPE` marks the
    /// paren-less `Type <` domain-block spelling deprecated and the
    /// parenthesised `(Type) <` form "correct ADL 1.4/ADL 1.5"; WARNING).
    ///
    /// NOTE: no openEHR validity code covers deprecated-spelling use — our own
    /// extension (owner ruling 2026-08-01, spec-adherence §NEVER LAX:
    /// deprecations are enforced at exactly the deprecation's strength —
    /// accepted, never silently absorbed).
    W14dep,
    // ── phase-2 specialisation-vs-flat-parent codes (`master04.5` §Validity
    //    Rules: `C_ATTRIBUTE` / `C_OBJECT` / `ARCHETYPE_SLOT` / `C_ARCHETYPE_ROOT` /
    //    `C_COMPLEX_OBJECT_PROXY`; `master08` §Phase 2 → Validate Specialised
    //    Definition). Raised by `specialisation` against the flat parent.
    /// VSANCE — specialised attribute node existence conformance (`master04.5`
    /// §`C_ATTRIBUTE`).
    Vsance,
    /// VSANCC — specialised attribute node cardinality conformance (`master04.5`
    /// §`C_ATTRIBUTE`).
    Vsancc,
    /// VSAM — specialised attribute multiplicity conformance (`master04.5`
    /// §`C_ATTRIBUTE`).
    Vsam,
    /// VSONIN — new object node identifier validity (`master04.5` §`C_OBJECT`).
    Vsonin,
    /// VSSM — specialised sibling order validity (`master04.5` §`C_OBJECT`).
    Vssm,
    /// VSONT — specialised object node meta-type conformance (`master04.5`
    /// §`C_OBJECT`).
    Vsont,
    /// VSONCT — specialised object node reference type conformance (`master04.5`
    /// §`C_OBJECT`).
    Vsonct,
    /// VSONCO — specialised object node occurrences redefinition validity
    /// (`master04.5` §`C_OBJECT` — the collective-occurrences rule).
    Vsonco,
    /// VSONPT — prohibited object node AOM type validity (`master04.5` §`C_OBJECT`).
    Vsonpt,
    /// VSONPI — prohibited object node node-id validity (`master04.5` §`C_OBJECT`).
    Vsonpi,
    /// VSONPO — new object node prohibited occurrences validity (`master04.5`
    /// §`C_OBJECT`).
    Vsonpo,
    /// VSONI — _deprecated_ redefined object node identifier validity (`master04.5`
    /// §`C_OBJECT`; recognise, do not enforce).
    Vsoni,
    /// VSONIR — _deprecated_ redefined object node identifier condition
    /// (`master04.5` §`C_OBJECT`; recognise, do not enforce).
    Vsonir,
    /// VSUNT — `use_node` specialisation parent validity (`master04.5`
    /// §`C_COMPLEX_OBJECT_PROXY`).
    Vsunt,
    /// VUNT — `use_node` reference model type validity (`master04.5`
    /// §`C_COMPLEX_OBJECT_PROXY`).
    Vunt,
    /// VUNP — `use_node` path validity: the proxy target path must resolve to an
    /// object node in the flat form (`master04.5` §`C_COMPLEX_OBJECT_PROXY`
    /// VUNP L482-483; a phase-3 flat-form check, `flat`).
    Vunp,
    /// VDSSID — slot redefinition child node id (`master04.5` §`ARCHETYPE_SLOT`).
    Vdssid,
    /// VDSSM — specialised slot definition match validity (`master04.5`
    /// §`ARCHETYPE_SLOT`).
    Vdssm,
    /// VDSSP — specialised slot definition parent validity (`master04.5`
    /// §`ARCHETYPE_SLOT`).
    Vdssp,
    /// VDSSC — specialised slot definition closed validity (`master04.5`
    /// §`ARCHETYPE_SLOT`).
    Vdssc,
    /// VARXS — external reference slot conformance (`master04.5`
    /// §`C_ARCHETYPE_ROOT`).
    Varxs,
    /// VARXID — external reference slot filling id validity (`master04.5`
    /// §`C_ARCHETYPE_ROOT`).
    Varxid,
    /// VPOV — invalid leaf object value redefinition (`master08` §Phase 2; no full
    /// vendored text — implemented from the gloss via `c_value_conforms_to`,
    /// NOTE-flagged).
    Vpov,
    /// VUNK — invalid leaf object value redefinition (`master08` §Phase 2; no full
    /// vendored text — NOTE-flagged).
    Vunk,
    /// VTPNC — tuple non-conformance to the parent node (`master08` §Phase 2; no
    /// full vendored text — NOTE-flagged).
    Vtpnc,
    /// VTPIN — tuple invalidity against the parent node (`master08` §Phase 2; no
    /// full vendored text — NOTE-flagged).
    Vtpin,
    /// `AUTHORED_RESOURCE.Translations_valid` — a present translations list is
    /// non-empty and never re-states the original language (RM
    /// `org.openehr.rm.common.authored_resource.adoc` §Invariants; the RM
    /// resource package governs ADL 1.4 meta-data, `common/master08` NOTE).
    RmArTranslations,
    /// `AUTHORED_RESOURCE.Description_valid` — every description detail's
    /// language is the original or a listed translation (RM
    /// `org.openehr.rm.common.authored_resource.adoc` §Invariants).
    RmArDescription,
    /// `RESOURCE_DESCRIPTION.Original_author_valid` — `not
    /// original_author.is_empty` (RM
    /// `org.openehr.rm.common.resource_description.adoc` §Invariants).
    RmRdOriginalAuthor,
    /// `RESOURCE_DESCRIPTION.Lifecycle_state_valid` — `not
    /// lifecycle_state.is_empty` (RM
    /// `org.openehr.rm.common.resource_description.adoc` §Invariants).
    RmRdLifecycleState,
    /// `RESOURCE_DESCRIPTION.Details_valid` — a description carries at least
    /// one detail (RM `org.openehr.rm.common.resource_description.adoc`
    /// §Invariants, with `details` 1..1 in the RM class table).
    RmRdDetails,
    /// `RESOURCE_DESCRIPTION_ITEM.Purpose_valid` — `not purpose.is_empty` (RM
    /// `org.openehr.rm.common.resource_description_item.adoc` §Invariants).
    /// Warning on a 1.4 SOURCE: `purpose = <"">` is endemic real-world 1.4
    /// authoring (61 CKM archetypes), and 1.4 tolerance is our own design —
    /// the finding is named, never refused (the `ckm_archetype_packs` sweep
    /// pins both halves).
    RmRdiPurpose,
    /// `RESOURCE_DESCRIPTION_ITEM.Use_valid` — a present `use` is non-empty
    /// (RM `org.openehr.rm.common.resource_description_item.adoc`
    /// §Invariants). Warning on a 1.4 SOURCE: `use = <"">` is the 1.4
    /// ecosystem's spelling of absence (162 CKM archetypes).
    RmRdiUse,
    /// `RESOURCE_DESCRIPTION_ITEM.misuse_valid` — a present `misuse` is
    /// non-empty (RM `org.openehr.rm.common.resource_description_item.adoc`
    /// §Invariants; the spec's own lowercase name). Warning on a 1.4 SOURCE:
    /// `misuse = <"">` is the 1.4 ecosystem's spelling of absence (873 CKM
    /// archetypes).
    RmRdiMisuse,
    /// `AUTHORED_RESOURCE.Original_language_valid` — the original language is
    /// in the openEHR `languages` code set (RM
    /// `org.openehr.rm.common.authored_resource.adoc` §Invariants).
    RmArOriginalLanguage,
    /// `TRANSLATION_DETAILS.Language_valid` — a translation's language is in
    /// the openEHR `languages` code set (RM
    /// `org.openehr.rm.common.translation_details.adoc` §Invariants).
    RmTdLanguage,
    /// `RESOURCE_DESCRIPTION_ITEM.Language_valid` — a description detail's
    /// language is in the openEHR `languages` code set (RM
    /// `org.openehr.rm.common.resource_description_item.adoc` §Invariants).
    RmRdiLanguage,
}

impl ValidationCode {
    /// The bare mnemonic (e.g. `"VARDT"`), as used in the spec catalogue and
    /// the conformance-corpus `regression` tags.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "a pure one-arm-per-code match table; splitting it would \
                  scatter the catalogue"
    )]
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::Vardt => "VARDT",
            Self::Varcn => "VARCN",
            Self::Stcnt => "STCNT",
            Self::Vacsd => "VACSD",
            Self::Volt => "VOLT",
            Self::Varav => "VARAV",
            Self::Varrv => "VARRV",
            Self::Votm => "VOTM",
            Self::Vdifv => "VDIFV",
            Self::Vdifp => "VDIFP",
            Self::Vcorm => "VCORM",
            Self::Vcormt => "VCORMT",
            Self::Vcarm => "VCARM",
            Self::Vcaex => "VCAEX",
            Self::Vcaca => "VCACA",
            Self::Vcam => "VCAM",
            Self::Vcormen => "VCORMEN",
            Self::Vcormenv => "VCORMENV",
            Self::Vcormenu => "VCORMENU",
            Self::Vatcv => "VATCV",
            Self::Vtsd => "VTSD",
            Self::Vtlc => "VTLC",
            Self::Vttbk => "VTTBK",
            Self::Vtcbk => "VTCBK",
            Self::Vetdf => "VETDF",
            Self::Vtvsid => "VTVSID",
            Self::Vtvsmd => "VTVSMD",
            Self::Vtvsuq => "VTVSUQ",
            Self::Vdsev => "VDSEV",
            Self::Vdsiv => "VDSIV",
            Self::Varxra => "VARXRA",
            Self::Varxnc => "VARXNC",
            Self::Varxav => "VARXAV",
            Self::Varxr => "VARXR",
            Self::Varxtv => "VARXTV",
            Self::Vatid => "VATID",
            Self::Vatcd => "VATCD",
            Self::Vatdf => "VATDF",
            Self::Vacdf => "VACDF",
            Self::Vatda => "VATDA",
            Self::Vranp => "VRANP",
            Self::Voku => "VOKU",
            Self::Varid => "VARID",
            Self::Vdeol => "VDEOL",
            Self::Vard => "VARD",
            Self::Vasid => "VASID",
            Self::Valc => "VALC",
            Self::Vtpl => "VTPL",
            Self::Vrrlp => "VRRLP",
            Self::Vcocd => "VCOCD",
            Self::Vcoid => "VCOID",
            Self::Vcosu => "VCOSU",
            Self::Vcatu => "VCATU",
            Self::Vdfai => "VDFAI",
            Self::Vdfpt => "VDFPT",
            Self::Vobav => "VOBAV",
            Self::Vrmvp => "VRMVP",
            Self::Vrmvav => "VRMVAV",
            Self::Vacso => "VACSO",
            Self::Vacmcu => "VACMCU",
            Self::Vcoc => "VCOC",
            Self::Vacmco => "VACMCO",
            Self::Vsonif => "VSONIF",
            Self::Vrdla => "VRDLA",
            Self::Wacmcl => "WACMCL",
            Self::Wouc => "WOUC",
            Self::W14dep => "W14DEP",
            Self::Vsance => "VSANCE",
            Self::Vsancc => "VSANCC",
            Self::Vsam => "VSAM",
            Self::Vsonin => "VSONIN",
            Self::Vssm => "VSSM",
            Self::Vsont => "VSONT",
            Self::Vsonct => "VSONCT",
            Self::Vsonco => "VSONCO",
            Self::Vsonpt => "VSONPT",
            Self::Vsonpi => "VSONPI",
            Self::Vsonpo => "VSONPO",
            Self::Vsoni => "VSONI",
            Self::Vsonir => "VSONIR",
            Self::Vsunt => "VSUNT",
            Self::Vunt => "VUNT",
            Self::Vunp => "VUNP",
            Self::Vdssid => "VDSSID",
            Self::Vdssm => "VDSSM",
            Self::Vdssp => "VDSSP",
            Self::Vdssc => "VDSSC",
            Self::Varxs => "VARXS",
            Self::Varxid => "VARXID",
            Self::Vpov => "VPOV",
            Self::Vunk => "VUNK",
            Self::Vtpnc => "VTPNC",
            Self::Vtpin => "VTPIN",
            Self::RmArTranslations => "AUTHORED_RESOURCE.Translations_valid",
            Self::RmArDescription => "AUTHORED_RESOURCE.Description_valid",
            Self::RmRdOriginalAuthor => "RESOURCE_DESCRIPTION.Original_author_valid",
            Self::RmRdLifecycleState => "RESOURCE_DESCRIPTION.Lifecycle_state_valid",
            Self::RmRdDetails => "RESOURCE_DESCRIPTION.Details_valid",
            Self::RmRdiPurpose => "RESOURCE_DESCRIPTION_ITEM.Purpose_valid",
            Self::RmRdiUse => "RESOURCE_DESCRIPTION_ITEM.Use_valid",
            Self::RmRdiMisuse => "RESOURCE_DESCRIPTION_ITEM.misuse_valid",
            Self::RmArOriginalLanguage => "AUTHORED_RESOURCE.Original_language_valid",
            Self::RmTdLanguage => "TRANSLATION_DETAILS.Language_valid",
            Self::RmRdiLanguage => "RESOURCE_DESCRIPTION_ITEM.Language_valid",
        }
    }

    /// The severity of this code: [`Severity::Warning`] for the `W`-prefixed
    /// codes, [`Severity::Error`] otherwise (see [`Severity`]).
    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            Self::Wacmcl
            | Self::Wouc
            | Self::W14dep
            | Self::RmRdiPurpose
            | Self::RmRdiUse
            | Self::RmRdiMisuse => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl std::fmt::Display for ValidationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.mnemonic())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one line per catalogue code — an exhaustive list whose whole point is to name every code, not logic"
    )]
    fn every_code_has_a_unique_mnemonic_and_severity() {
        let all = [
            ValidationCode::Vardt,
            ValidationCode::Varcn,
            ValidationCode::Stcnt,
            ValidationCode::Vacsd,
            ValidationCode::Volt,
            ValidationCode::Varav,
            ValidationCode::Varrv,
            ValidationCode::Votm,
            ValidationCode::Vdifv,
            ValidationCode::Vdifp,
            ValidationCode::Vcorm,
            ValidationCode::Vcormt,
            ValidationCode::Vcarm,
            ValidationCode::Vcaex,
            ValidationCode::Vcaca,
            ValidationCode::Vcam,
            ValidationCode::Vcormen,
            ValidationCode::Vcormenv,
            ValidationCode::Vcormenu,
            ValidationCode::Vatcv,
            ValidationCode::Vtsd,
            ValidationCode::Vtlc,
            ValidationCode::Vttbk,
            ValidationCode::Vtcbk,
            ValidationCode::Vetdf,
            ValidationCode::Vtvsid,
            ValidationCode::Vtvsmd,
            ValidationCode::Vtvsuq,
            ValidationCode::Vdsev,
            ValidationCode::Vdsiv,
            ValidationCode::Varxra,
            ValidationCode::Varxnc,
            ValidationCode::Varxav,
            ValidationCode::Varxr,
            ValidationCode::Varxtv,
            ValidationCode::Vatid,
            ValidationCode::Vatcd,
            ValidationCode::Vatdf,
            ValidationCode::Vacdf,
            ValidationCode::Vatda,
            ValidationCode::Vranp,
            ValidationCode::Voku,
            ValidationCode::Varid,
            ValidationCode::Vdeol,
            ValidationCode::Vard,
            ValidationCode::Vasid,
            ValidationCode::Valc,
            ValidationCode::Vtpl,
            ValidationCode::Vrrlp,
            ValidationCode::Vcocd,
            ValidationCode::Vcoid,
            ValidationCode::Vcosu,
            ValidationCode::Vcatu,
            ValidationCode::Vdfai,
            ValidationCode::Vobav,
            ValidationCode::Vrmvp,
            ValidationCode::Vrmvav,
            ValidationCode::Vacso,
            ValidationCode::Vacmcu,
            ValidationCode::Vcoc,
            ValidationCode::Vacmco,
            ValidationCode::Vsonif,
            ValidationCode::Vrdla,
            ValidationCode::Wacmcl,
            ValidationCode::Wouc,
            ValidationCode::Vsance,
            ValidationCode::Vsancc,
            ValidationCode::Vsam,
            ValidationCode::Vsonin,
            ValidationCode::Vssm,
            ValidationCode::Vsont,
            ValidationCode::Vsonct,
            ValidationCode::Vsonco,
            ValidationCode::Vsonpt,
            ValidationCode::Vsonpi,
            ValidationCode::Vsonpo,
            ValidationCode::Vsoni,
            ValidationCode::Vsonir,
            ValidationCode::Vsunt,
            ValidationCode::Vunt,
            ValidationCode::Vunp,
            ValidationCode::Vdssid,
            ValidationCode::Vdssm,
            ValidationCode::Vdssp,
            ValidationCode::Vdssc,
            ValidationCode::Varxs,
            ValidationCode::Varxid,
            ValidationCode::Vpov,
            ValidationCode::Vunk,
            ValidationCode::Vtpnc,
            ValidationCode::Vtpin,
        ];
        let mut seen = std::collections::HashSet::new();
        for c in all {
            assert!(seen.insert(c.mnemonic()), "duplicate mnemonic {c}");
            let expected = if c.mnemonic().starts_with('W') {
                Severity::Warning
            } else {
                Severity::Error
            };
            assert_eq!(c.severity(), expected, "{c} severity");
        }
        assert_eq!(seen.len(), 91);
    }
}
