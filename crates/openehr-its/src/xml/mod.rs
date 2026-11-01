// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! **ITS-XML** — canonical XML serialization (openEHR ITS-XML).
//!
//! A document is written in either of
//! the two published wire lineages: `http://schemas.openehr.org/v1`
//! (Release-1.0.2v2, the STABLE bundle) and `http://schemas.openehr.org/v2`
//! (Release-2.0.0v2, TRIAL upstream). One generated impl set serves both: the
//! documents it writes differ only by the root `xmlns`, selected at serialize
//! time. The two vendored SCHEMA bundles are not equivalent, though — the v1
//! bundle is frozen at an older RM generation (`schemas/xml/PROVENANCE.md`
//! §"Two lineages"), so a served v1 document can carry RM 1.2.0 members its
//! own schema predates.
//!
//! The `ToXml`/`FromXml` impls for the RM/BASE spec types are **generated** by
//! `openehr-codegen`'s `emit-xml` target into `generated`, driven by
//! the vendored XSDs (`schemas/xml/`) + the BMM field model. This module is the
//! hand-written [`runtime`] (traits + `quick-xml` writer/reader) and the public
//! entry points. Regenerate with `cargo run -p openehr-codegen -- emit-xml`.

pub mod runtime;

// Trait impls for the spec types — compiled for their effect; nothing to export.
mod generated;

/// One published ITS-XML **document element** (a global `xs:element` of the
/// vendored bundles) that this codec serves as a canonical-XML root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedRoot {
    /// The element name (`<xs:element name="…">`).
    pub element: &'static str,
    /// The element's declared XSD type.
    pub declared_type: &'static str,
    /// Whether that declared type is `abstract="true"` — XML Schema Part 1
    /// forbids an instance from using an abstract type directly, so such a
    /// root MUST name its concrete class with `xsi:type`
    /// (<https://www.w3.org/TR/xmlschema-1/#xsi_type>, §2.6.1 + §3.4.6).
    pub type_is_abstract: bool,
}

/// The published document elements served as canonical-XML roots.
///
/// This is the ONE statement of the published-element fact both the REST layer
/// and the admin archive consume (the schemas live in this crate:
/// `schemas/xml/`). Both published lineages spell every entry identically:
///
/// - `<xs:element name="composition" type="COMPOSITION"/>` —
///   `its-xml-1.0.2-nsv1/ALL/Composition.xsd`;
///   `its-xml-2.0.0-nsv2/RM/latest/documents/Composition.xsd`.
/// - `<xs:element name="version" type="VERSION"/>` over
///   `<xs:complexType name="VERSION" abstract="true">` —
///   `…/ALL/Version.xsd`; `…/RM/latest/documents/Version.xsd`.
/// - `<xs:element name="items" type="LOCATABLE"/>` over
///   `<xs:complexType name="LOCATABLE" abstract="true">` —
///   `…/ALL/Structure.xsd`; `…/RM/latest/documents/Structure.xsd`.
///
/// A root name the schemas publish no element for is deliberately absent —
/// the ITS-REST §XML Format MUST ("responses MUST conform to the [published
/// XSDs]") has nothing to bind to there.
pub const PUBLISHED_ROOTS: &[PublishedRoot] = &[
    PublishedRoot {
        element: "composition",
        declared_type: "COMPOSITION",
        type_is_abstract: false,
    },
    PublishedRoot {
        element: "version",
        declared_type: "VERSION",
        type_is_abstract: true,
    },
    PublishedRoot {
        element: "items",
        declared_type: "LOCATABLE",
        type_is_abstract: true,
    },
];

/// The published document element of every `LOCATABLE` that has no element
/// of its own (a FOLDER, an EHR_STATUS, a PARTY, …): `items`, declared
/// `LOCATABLE` (abstract), so the root names its class with `xsi:type`.
///
/// Spelled once so every caller names the element the schemas publish:
/// `…/ALL/Structure.xsd`; `…/RM/latest/documents/Structure.xsd`.
pub const LOCATABLE_ROOT: &str = "items";

/// The declared **abstract** XSD type of published root `element`, when its
/// type is abstract (the instance must then carry `xsi:type` — see
/// [`PublishedRoot::type_is_abstract`]).
///
/// `None` for a concretely-typed root and for a name the schemas publish no
/// element for.
#[must_use]
pub fn declared_abstract_root_type(element: &str) -> Option<&'static str> {
    PUBLISHED_ROOTS
        .iter()
        .find(|r| r.element == element && r.type_is_abstract)
        .map(|r| r.declared_type)
}

/// Deserialize an RM value from a canonical openEHR XML document.
///
/// Reading is namespace-agnostic by construction: the reader dispatches on
/// local element names and `xsi:type`, and never inspects the document's root
/// `xmlns`. Both published lineages therefore parse identically. The 2.0.0
/// restructure changed the namespace the schemas declare
/// (`docs/specs/openehr/ITS-XML/README.adoc` §"Releases and IM Versions":
/// "Simultaneously, the internal namespace used in the schemas is also changed
/// to `http://schemas.openehr.org/v2`") — it did NOT make the two bundles
/// interchangeable: the flat `Release-1.0.2v2` bundle is frozen at an older RM
/// generation and declares neither `FOLDER.details` nor 22 other RM 1.2.0
/// attributes, nor 50 RM 1.2.0 classes at all
/// (`schemas/xml/PROVENANCE.md` §"Two lineages"). runtime::Namespace-agnostic READING
/// stays sound regardless, because every element name and `xsi:type` the
/// reader dispatches on is spelled identically in both lineages; only what the
/// two schemas ACCEPT differs.
///
/// # Errors
/// Propagates parse errors.
pub fn from_canonical_xml<T: runtime::FromXml>(xml: &str) -> Result<T, runtime::XmlError> {
    runtime::from_xml(xml)
}
