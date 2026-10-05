// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Convert a canonical-JSON RM instance to a canonical serialization — the
//! offline generator for corpus fixtures (e.g. the CNF catalogue's request-body
//! fixtures), so every committed fixture is produced by the same gate-proven
//! codec that serves the wire.
//!
//! Usage: `cargo run -p openehr-its --example canonical_convert -- <in.json> <out.{xml,json}>`
//!
//! The output form follows the output path's extension: `.xml` emits canonical
//! XML, `.json` emits canonical JSON (the codec's own attribute order and
//! `_type` self-tagging — the normalizing pass that turns a hand-authored
//! payload into the exact bytes the wire carries).
//!
//! The concrete RM type is dispatched on the document's `_type`
//! (ITS-JSON: the `_type` self-tag names the RM class); the XML root element
//! is the lower-snake class name and the namespace is the canonical
//! `http://schemas.openehr.org/v1` (ITS-XML; vendored XSDs at
//! `crates/openehr-its/schemas/xml/`).

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use std::error::Error;

use openehr_rm::v1_2::common::change_control::original_version::OriginalVersion;
use openehr_rm::v1_2::common::directory::folder::Folder;
use openehr_rm::v1_2::composition::composition::Composition;
use openehr_rm::v1_2::demographic::agent::Agent;
use openehr_rm::v1_2::demographic::group::Group;
use openehr_rm::v1_2::demographic::organisation::Organisation;
use openehr_rm::v1_2::demographic::person::Person;
use openehr_rm::v1_2::demographic::role::Role;
use openehr_rm::v1_2::ehr::ehr_status::EhrStatus;

use openehr_its::xml::runtime::{Namespace, ToXml};

/// The serialization the run emits, with the XML root the RM class documents
/// itself under.
enum Form<'a> {
    /// Canonical XML under `root_tag`. `declared_type` is `Some` when the
    /// root element's DECLARED type is abstract, so the document must name its
    /// concrete subtype with `xsi:type`
    /// (<https://www.w3.org/TR/xmlschema-1/#xsi_type> §2.6.1 + §3.4.6) — the
    /// change-control package publishes exactly one global element,
    /// `ALL/Version.xsd`: `<xs:element name="version" type="VERSION"/>` over
    /// `<xs:complexType name="VERSION" abstract="true">`, so a VERSION
    /// document is only XSD-rooted that way.
    Xml {
        root_tag: &'a str,
        declared_type: Option<&'a str>,
    },
    /// Canonical JSON (ITS-JSON): `_type` first, then BMM attribute order.
    Json,
}

fn emit<T: serde::de::DeserializeOwned + serde::Serialize + ToXml>(
    value: &serde_json::Value,
    form: &Form<'_>,
) -> Result<String, Box<dyn Error>> {
    let typed: T = openehr_its::json::from_canonical_value(value)?;
    Ok(match form {
        Form::Xml {
            root_tag,
            declared_type: None,
        } => openehr_its::xml::to_canonical_xml(&typed, root_tag)?,
        Form::Xml {
            root_tag,
            declared_type: Some(declared),
        } => {
            openehr_its::xml::to_canonical_xml_declared(&typed, root_tag, declared, Namespace::V1)?
        }
        Form::Json => pretty(&openehr_its::json::to_canonical_value(&typed))?,
    })
}

/// The canonical-JSON tree rendered with the corpus's four-space indent. Key
/// order is the codec's (`serde_json` is built with `preserve_order`), so the
/// pretty form carries the same attribute sequence as the compact one.
fn pretty(value: &serde_json::Value) -> Result<String, Box<dyn Error>> {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(value, &mut ser)?;
    Ok(format!("{}\n", String::from_utf8(out)?))
}

/// The output form the destination path asks for.
fn form_of<'a>(output: &str, root_tag: &'a str, declared_type: Option<&'a str>) -> Form<'a> {
    if std::path::Path::new(output)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        Form::Json
    } else {
        Form::Xml {
            root_tag,
            declared_type,
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        return Err("usage: canonical_convert <in.json> <out.{xml,json}>".into());
    };
    let text = std::fs::read_to_string(&input)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let rm_type = value
        .get("_type")
        .and_then(serde_json::Value::as_str)
        .ok_or("input document carries no _type self-tag")?;
    let out = match rm_type {
        "COMPOSITION" => emit::<Composition>(&value, &form_of(&output, "composition", None))?,
        "EHR_STATUS" => emit::<EhrStatus>(&value, &form_of(&output, "ehr_status", None))?,
        "FOLDER" => emit::<Folder>(&value, &form_of(&output, "folder", None))?,
        "PERSON" => emit::<Person>(&value, &form_of(&output, "person", None))?,
        "AGENT" => emit::<Agent>(&value, &form_of(&output, "agent", None))?,
        "GROUP" => emit::<Group>(&value, &form_of(&output, "group", None))?,
        "ORGANISATION" => emit::<Organisation>(&value, &form_of(&output, "organisation", None))?,
        "ROLE" => emit::<Role>(&value, &form_of(&output, "role", None))?,
        "ORIGINAL_VERSION" => emit::<OriginalVersion<Composition>>(
            &value,
            &form_of(&output, "version", Some("VERSION")),
        )?,
        other => return Err(format!("unsupported root _type {other}").into()),
    };
    std::fs::write(&output, out)?;
    Ok(())
}
