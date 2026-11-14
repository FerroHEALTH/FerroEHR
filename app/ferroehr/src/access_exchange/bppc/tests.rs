// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The vendored BPPC samples of ITI TF-3 5.1.2.2, their round trip, and the
//! refusals of an acknowledgement FerroEHR cannot read.

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Effect, OperationClass};
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;

use super::{ExportContext, ImportContext, export, import, ts};
use crate::access_exchange::rm::{directive, interval, list, term};
use crate::access_exchange::{Format, Loss, Refused};

/// The two code blocks of the vendored TF-3 5.1 page: Figure 5.1.2.2-1, the
/// sample document, and Figure 5.1.2.2-2, its service event.
fn figures() -> (String, String) {
    let path = format!(
        "{}/../../docs/specs/ihe-consents/profiles.ihe.net/ITI/TF/Volume3/ch-5.1.html",
        env!("CARGO_MANIFEST_DIR")
    );
    let page = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let blocks: Vec<String> = page
        .split("<code class=\"code-block\">")
        .skip(1)
        .map(|rest| {
            let block = rest.split("</code>").next().expect("a closed block");
            // NOTE: the page indents its code blocks with no-break spaces, which are HTML
            // layout and no XML whitespace, so they read as spaces.
            quick_xml::escape::unescape(block)
                .expect("escaped text")
                .replace('\u{a0}', " ")
        })
        .collect();
    assert_eq!(blocks.len(), 2, "the page holds the two figures");
    (blocks[0].clone(), blocks[1].clone())
}

/// Figure 5.1.2.2-1 with the service event of Figure 5.1.2.2-2 where the
/// sample elides the rest of the header (its `:` line).
fn sample() -> String {
    let (document, event) = figures();
    document
        .lines()
        .map(|line| {
            if line.trim() == ":" {
                event.as_str()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The catalogue entry the sample acknowledges: "Chicken Costume Consent",
/// policy `9.8.7.6.5.4.3.2.1` of the domain `9.8.7.6.5.4.3.2.2`.
fn catalogue() -> Vec<AccessDirective> {
    let mut entry = directive(DirectiveKind::Consent, Effect::Permit);
    entry.operations = list(vec![term(OperationClass::Read)]);
    entry.identifiers = list(vec![DvIdentifier {
        issuer: Some("9.8.7.6.5.4.3.2.2".to_owned()),
        assigner: None,
        id: "9.8.7.6.5.4.3.2.1".to_owned(),
        r#type: None,
    }]);
    vec![entry]
}

const EVIDENCE: &str = "ehr://ferroehr/0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f80/compositions/1";

#[test]
fn the_vendored_sample_imports_as_the_catalogue_entry_and_round_trips() {
    let catalogue = catalogue();
    let context = ImportContext {
        catalogue: &catalogue,
        evidence: EVIDENCE,
    };
    let imported = import(&sample(), &context).expect("imports");
    assert_eq!(imported.len(), 1);
    let copy = &imported[0];
    assert_eq!(copy.operations, catalogue[0].operations);
    assert_eq!(
        copy.time_validity,
        Some(interval(Some("2010-11-30".to_owned()), None))
    );
    assert_eq!(
        copy.evidence.as_ref().map(|e| e[0].value.as_str()),
        Some(EVIDENCE)
    );
    assert_eq!(
        copy.instructed.as_ref().map(|i| i.value.as_str()),
        Some("2007-06-19T01:20:05")
    );
    let exported = export(
        &imported,
        &ExportContext {
            catalogue: &catalogue,
            patient: None,
            controller: None,
            now: "2007-06-19T01:20:05",
        },
    );
    assert!(exported.refused.is_empty(), "{:?}", exported.refused);
    assert_eq!(exported.documents.len(), 1);
    let back = import(&exported.documents[0], &context).expect("reads its own output");
    assert_eq!(back, imported);
}

#[test]
fn export_writes_one_document_per_time_validity_and_names_what_it_cannot_state() {
    let catalogue = catalogue();
    let copy = |start: &str| {
        let mut copy = catalogue[0].clone();
        copy.time_validity = Some(interval(Some(start.to_owned()), None));
        copy
    };
    let mut narrowed = copy("2026-01-01T00:00:00Z");
    narrowed.purposes = list(vec![term(OperationClass::Query)]);
    let restriction = directive(DirectiveKind::Restriction, Effect::Deny);
    let directives = vec![
        copy("2026-01-01T00:00:00Z"),
        copy("2026-01-01T00:00:00Z"),
        copy("2026-06-01T00:00:00+02:00"),
        narrowed,
        restriction,
    ];
    let exported = export(
        &directives,
        &ExportContext {
            catalogue: &catalogue,
            patient: Some("2.999.1.1.1"),
            controller: Some("2.999.2.1.1.35"),
            now: "2026-11-14T10:00:00Z",
        },
    );
    assert_eq!(exported.documents.len(), 2);
    let not_stated = |attribute: &str| {
        Refused::new(
            Format::Directive,
            attribute,
            Loss::NoFormatForm(Format::Bppc),
        )
    };
    assert_eq!(
        exported.refused,
        vec![
            (3, not_stated("applies_to")),
            (4, not_stated("identifiers"))
        ]
    );
    assert!(exported.documents[1].contains("20260601000000+0200"));
}

#[test]
fn an_acknowledgement_of_a_policy_the_catalogue_does_not_hold_is_refused() {
    let context = ImportContext {
        catalogue: &[],
        evidence: EVIDENCE,
    };
    assert_eq!(
        import(&sample(), &context),
        Err(Refused::new(
            Format::Bppc,
            "ClinicalDocument/documentationOf/serviceEvent/code",
            Loss::NotInCatalogue("9.8.7.6.5.4.3.2.2|9.8.7.6.5.4.3.2.1".to_owned())
        ))
    );
    let catalogue = catalogue();
    let context = ImportContext {
        catalogue: &catalogue,
        evidence: EVIDENCE,
    };
    let no_low = sample().replace("<low value='20101130'/>", "<high value='20201130'/>");
    assert_eq!(
        import(&no_low, &context).map_err(|r| r.element),
        Err("ClinicalDocument/documentationOf/serviceEvent/effectiveTime/low".to_owned())
    );
    let other = sample().replace("1.3.6.1.4.1.19376.1.5.3.1.1.7'", "1.2.3'");
    assert_eq!(
        import(&other, &context).map_err(|r| r.element),
        Err("ClinicalDocument/templateId".to_owned())
    );
}

#[test]
fn a_ts_and_an_iso_date_time_convert_both_ways() {
    for (iso, hl7) in [
        ("2010-11-30", "20101130"),
        ("2007-06-19T01:20:05", "20070619012005"),
        ("2026-01-01T00:00:00Z", "20260101000000+0000"),
        ("2026-06-01T00:00:00.5+02:00", "20260601000000.5+0200"),
    ] {
        assert_eq!(ts(iso).as_deref(), Some(hl7), "{iso}");
        assert_eq!(super::iso(hl7, "t").as_deref(), Ok(iso), "{hl7}");
    }
    assert_eq!(ts("2026-W01"), None);
}
