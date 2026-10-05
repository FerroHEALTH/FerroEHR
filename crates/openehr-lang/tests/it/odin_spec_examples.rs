// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Pins the ODIN specification's own example documents against the reader:
//! the ch.2 overview exemplar of
//! `docs/specs/openehr/LANG/docs/odin/master02-overview.adoc` as its valid +
//! invalid twins (the #852 ch.2 audit, finding R1), and the ch.4 artefact
//! forms of `master04-odin_artefacts.adoc` — the schema-identifier prefix and
//! the three §Document shapes (the #854 ch.4 audit).

use openehr_lang::v1_1::odin::{
    OdinErrorKind, OdinKey, OdinSchemaId, OdinValue, parse, parse_document,
};

/// The master02 exemplar VERBATIM (its lines 7–22). The line
/// `[01235] = < -- etc >` is lexically self-inconsistent with the spec's own
/// comment rule: `master03-basics.adoc` §Comments makes `--` run to end of
/// line ("Multi-line comments are achieved using the \"--\" leader on each
/// line where the comment continues"), so the comment swallows the block's
/// closing `>` and the document cannot parse. Refusing it is spec-right; the
/// invalid twin stays pinned so a lenient reader fails here.
const CH2_EXEMPLAR_VERBATIM: &str = r#"person = (List<PERSON>) <
    [01234] = <
        name = < -- person's name
            forenames = <"Sherlock">
            family_name = <"Holmes">
            salutation = <"Mr">
        >
        address = < -- person's address
            habitation_number = <"221B">
            street_name = <"Baker St">
            city = <"London">
            country = <"England">
        >
    >
    [01235] = < -- etc >
>"#;

/// The exemplar with the elliptical `-- etc` comment materialized on its own
/// line — the reading master02 illustrates once the master03 §Comments rule
/// is applied consistently. Everything else is byte-identical to the chapter
/// text.
const CH2_EXEMPLAR_MATERIALIZED: &str = r#"person = (List<PERSON>) <
    [01234] = <
        name = < -- person's name
            forenames = <"Sherlock">
            family_name = <"Holmes">
            salutation = <"Mr">
        >
        address = < -- person's address
            habitation_number = <"221B">
            street_name = <"Baker St">
            city = <"London">
            country = <"England">
        >
    >
    [01235] = < -- etc
    >
>"#;

#[test]
fn ch2_exemplar_verbatim_is_refused() {
    let err = parse(CH2_EXEMPLAR_VERBATIM).expect_err(
        "the verbatim master02 exemplar must be refused: the `-- etc` comment \
         swallows its block's closing `>` (master03 §Comments)",
    );
    assert_eq!(err.kind, OdinErrorKind::UnexpectedToken);
}

#[test]
fn ch2_exemplar_materialized_parses_to_the_expected_tree() {
    let parsed = parse(CH2_EXEMPLAR_MATERIALIZED)
        .unwrap_or_else(|e| panic!("materialized master02 exemplar should parse: {e}"));

    let OdinValue::Object(top) = parsed else {
        panic!("expected a top-level attribute object, got {parsed:?}");
    };
    let OdinValue::Typed { rm_type, value } = top.get("person").expect("person attribute") else {
        panic!("expected `person` to carry the (List<PERSON>) cast");
    };
    assert_eq!(rm_type, "List<PERSON>");

    let OdinValue::KeyedList(entries) = value.as_ref() else {
        panic!("expected a keyed container under the cast, got {value:?}");
    };
    assert_eq!(entries.len(), 2, "two container items, [01234] and [01235]");

    let OdinValue::Object(person) = &entries[0].1 else {
        panic!("expected the first container item to be an object");
    };
    let OdinValue::Object(name) = person.get("name").expect("name attribute") else {
        panic!("expected `name` to be an object");
    };
    for (attr, expected) in [
        ("forenames", "Sherlock"),
        ("family_name", "Holmes"),
        ("salutation", "Mr"),
    ] {
        assert_eq!(
            name.get(attr),
            Some(&OdinValue::String(expected.to_owned())),
            "name/{attr}"
        );
    }
    let OdinValue::Object(address) = person.get("address").expect("address attribute") else {
        panic!("expected `address` to be an object");
    };
    for (attr, expected) in [
        ("habitation_number", "221B"),
        ("street_name", "Baker St"),
        ("city", "London"),
        ("country", "England"),
    ] {
        assert_eq!(
            address.get(attr),
            Some(&OdinValue::String(expected.to_owned())),
            "address/{attr}"
        );
    }

    assert_eq!(
        entries[1].1,
        OdinValue::Empty,
        "the elliptical second item is an empty block"
    );
}

/// The `master04-odin_artefacts` §Embedded Fragment example, with the
/// illustrative `leaf_value` barewords materialized as real leaves (the
/// verbatim placeholder form is pinned refused by the vendored
/// `anonymous_odin.txt` adjudication in `vendor_odin.rs`). Fragments carry
/// "no object identifiers nor schema identifier".
#[test]
fn ch4_embedded_fragment_form_parses() {
    let src = r#"--
-- ODIN Embedded Fragment
--
    attr_1 = <
        attr_12 = <
            attr_13 = <"leaf">
        >
    >
    attr_2 = <
        attr_22 = <"leaf">
    >
"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("the fragment form should parse: {e}"));
    let OdinValue::Object(top) = parsed else {
        panic!("expected an attribute object, got {parsed:?}");
    };
    assert_eq!(top.len(), 2);
}

/// The `master04-odin_artefacts` §Anonymous Object Document form — an outer
/// `<>` pair around an embedded fragment; "syntactically more correct, and
/// should be supported by parsers".
#[test]
fn ch4_anonymous_object_document_form_parses() {
    let src = r#"--
-- ODIN Anonymous Object Document
--
<
    attr_1 = <
        attr_12 = <
            attr_13 = <"leaf">
        >
    >
    attr_2 = <
        attr_22 = <"leaf">
    >
>
"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("the anonymous form should parse: {e}"));
    assert!(
        matches!(parsed, OdinValue::Object(_)),
        "the outer block holds the fragment's attribute object"
    );
}

/// The `master04-odin_artefacts` §Identified Object Document form —
/// top-level keyed objects, one per identified object (the docs-text
/// production the vendored `odin.g4` start rule lacks; #1374). The verbatim
/// chapter example keeps a bare top-level `...` ellipsis line and stays
/// refused (pinned by the `identified_object_document.txt` adjudication in
/// `vendor_odin.rs`); this is its materialized twin.
#[test]
fn ch4_identified_object_document_form_parses() {
    let src = r#"--
-- ODIN Identified Object Document
--
["id_1"] = <
    attr_1 = <
        attr_12 = <
            attr_13 = <"leaf">
        >
    >
>

["id_2"] = <
    attr_1 = <
        attr_12 = <
            attr_13 = <"leaf">
        >
    >
>

["id_N"] = <
    attr_1 = <
        attr_12 = <
            attr_13 = <"leaf">
        >
    >
>
"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("the identified form should parse: {e}"));
    let OdinValue::KeyedList(entries) = parsed else {
        panic!("expected a top-level keyed list, got {parsed:?}");
    };
    assert_eq!(
        entries.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        vec![
            OdinKey::String("id_1".to_owned()),
            OdinKey::String("id_2".to_owned()),
            OdinKey::String("id_N".to_owned()),
        ]
    );
}

/// "Identifiers can be values of the String, Integer or any Date/Time
/// primitive types" (`master04-odin_artefacts` §Identified Object Document)
/// — the non-String identifier types at top level.
#[test]
fn ch4_identified_document_identifier_types() {
    let parsed = parse("[1] = <a = <1>>\n[2004-06-11] = <a = <2>>\n[10:30:00] = <a = <3>>")
        .unwrap_or_else(|e| panic!("typed identifiers should parse: {e}"));
    let OdinValue::KeyedList(entries) = parsed else {
        panic!("expected a keyed list");
    };
    assert_eq!(
        entries.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        vec![
            OdinKey::Integer(1),
            OdinKey::Date("2004-06-11".to_owned()),
            OdinKey::Time("10:30:00".to_owned()),
        ]
    );
}

/// `odin_text ::= ( schema_identifier )? main_text` with
/// `schema_identifier ::= '@' schema '=' URI`
/// (`master04-odin_artefacts` intro; #1373 — the chapter gives no example,
/// so the shape here is the adjudicated reading recorded at the parser
/// site). The prefix composes with every main-text form, `parse_document`
/// preserves it, and `parse` discards it.
#[test]
fn ch4_schema_identifier_prefix() {
    let doc = parse_document("@schema = <http://openehr.org/bmm/1.0>\na = <1>")
        .unwrap_or_else(|e| panic!("the schema-identified fragment should parse: {e}"));
    assert_eq!(
        doc.schema,
        Some(OdinSchemaId {
            name: "schema".to_owned(),
            uri: "<http://openehr.org/bmm/1.0>".to_owned(),
        })
    );
    assert!(matches!(doc.root, OdinValue::Object(_)));

    // …with the anonymous and identified forms too.
    for src in [
        "@schema = <http://x.org/s>\n<a = <1>>",
        "@schema = <http://x.org/s>\n[\"id\"] = <a = <1>>",
    ] {
        let doc = parse_document(src)
            .unwrap_or_else(|e| panic!("the schema prefix should compose with {src:?}: {e}"));
        assert!(doc.schema.is_some(), "{src}");
    }

    // `parse` accepts and discards the prefix.
    let root = parse("@schema = <http://x.org/s>\na = <1>")
        .unwrap_or_else(|e| panic!("parse should tolerate the prefix: {e}"));
    assert!(matches!(root, OdinValue::Object(_)));

    // a misplaced `@` is still a parse error.
    assert!(parse("a = <1> @ b").is_err());
}

/// The `master05-content` §General Structure typical structure (its
/// `leaf_value` placeholders materialized) and the COMPLETE 9-path set
/// §Paths extracts from it, verbatim (#1377).
#[test]
fn ch5_typical_structure_and_its_complete_path_set() {
    let src = r#"attr_1 = <
    attr_2 = <
        attr_3 = <"leaf">
        attr_4 = <"leaf">
    >
    attr_5 = <
        attr_3 = <
            attr_6 = <"leaf">
        >
        attr_7 = <"leaf">
    >
>
attr_8 = <...>"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("the §5.1 structure should parse: {e}"));
    assert_eq!(
        parsed.paths(),
        vec![
            "/attr_1",
            "/attr_1/attr_2",
            "/attr_1/attr_2/attr_3",
            "/attr_1/attr_2/attr_4",
            "/attr_1/attr_5",
            "/attr_1/attr_5/attr_3",
            "/attr_1/attr_5/attr_3/attr_6",
            "/attr_1/attr_5/attr_7",
            "/attr_8",
        ]
    );
}

/// The `master05-content` §Container Objects `school_schedule` example. The
/// VERBATIM text is refused — its `weighting = <76%>` uses a percent literal
/// no leaf-data production defines (`master07-leaf_data`'s type set is
/// closed; `%` is not an ODIN token) — and the materialized twin (weighting
/// as the plain Real it is declared as) parses, with the section's two
/// listed container paths present in its path set.
#[test]
fn ch5_school_schedule_twins_and_container_paths() {
    let verbatim_weighting = "s = <weighting = <76%>>";
    let err = parse(verbatim_weighting).expect_err("`76%` is not a leaf of any production");
    assert_eq!(err.kind, OdinErrorKind::UnrecognisedToken);

    let src = r#"school_schedule = <
    lesson_times = <08:30:00, 09:30:00, 10:30:00, ...>
    locations = <
        [1] = <"under the big plane tree">
        [2] = <"under the north arch">
        [3] = <"in a garden">
    >
    subjects = <
        ["philosophy:plato"] = <
            name = <"philosophy">
            teacher = <"plato">
            topics = <"meta-physics", "natural science">
            weighting = <76.0>
        >
        ["philosophy:kant"] = <
            name = <"philosophy">
            teacher = <"kant">
            topics = <"meaning and reason", "meta-physics", "ethics">
            weighting = <80.0>
        >
        ["art"] = <
            name = <"art">
            teacher = <"goya">
            topics = <"technique", "portraiture", "satire">
            weighting = <78.0>
        >
    >
>"#;
    let parsed =
        parse(src).unwrap_or_else(|e| panic!("the materialized schedule should parse: {e}"));
    let paths = parsed.paths();
    for expected in [
        "/school_schedule/locations[1]",
        "/school_schedule/subjects[\"philosophy:kant\"]",
    ] {
        assert!(
            paths.contains(&expected.to_owned()),
            "{expected} missing from {paths:?}"
        );
    }
}

/// The `master05-content` §Nested Container Objects `List<List<String>>`
/// example, with its listed nested-key paths (`/list_of_string_lists[1]/[1]`
/// …) present in the extracted set.
#[test]
fn ch5_nested_containers_and_their_paths() {
    let src = r#"list_of_string_lists = <
    [1] = <
        [1] = <"first string in first list">
        [2] = <"second string in first list">
    >
    [2] = <
        [1] = <"first string in second list">
        [2] = <"second string in second list">
        [3] = <"third string in second list">
    >
    [3] = <
        [1] = <"only string in third list">
    >
>"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("the §5.5 example should parse: {e}"));
    let paths = parsed.paths();
    for expected in [
        "/list_of_string_lists[1]/[1]",
        "/list_of_string_lists[1]/[2]",
        "/list_of_string_lists[2]/[1]",
    ] {
        assert!(
            paths.contains(&expected.to_owned()),
            "{expected} missing from {paths:?}"
        );
    }
}

/// The `master05-content` §Adding Type Information `destinations` example
/// (placeholders materialized as the `<...>` voids the chapter itself
/// writes): dynamic-type casts at keyed and attribute positions, incl. the
/// statically-typed-attribute default (no cast on `hotels`/`attractions`)
/// and the fully-typed `(List<HOTEL>)` variant.
#[test]
fn ch5_adding_type_information_example() {
    let src = r#"destinations = <
    ["seville"] = (TOURIST_DESTINATION) <
        profile = (DESTINATION_PROFILE) <...>
        hotels = <
            ["gran sevilla"] = (HISTORIC_HOTEL) <...>
            ["sofitel"] = (LUXURY_HOTEL) <...>
            ["hotel real"] = (PENSION) <...>
        >
        attractions = <
            ["la corrida"] = (SPORT_VENUE) <...>
            ["Alcázar"] = (HISTORIC_SITE) <...>
        >
    >
>"#;
    let parsed =
        parse(src).unwrap_or_else(|e| panic!("the destinations example should parse: {e}"));
    let paths = parsed.paths();
    assert!(paths.contains(&"/destinations[\"seville\"]/hotels[\"gran sevilla\"]".to_owned()));

    let typed = parse(r#"hotels = (List<HOTEL>) <["gran sevilla"] = (HISTORIC_HOTEL) <...>>"#)
        .unwrap_or_else(|e| panic!("the fully-typed variant should parse: {e}"));
    assert!(matches!(typed, OdinValue::Object(_)));
}

/// The `master06-references` §Within An Object example — associations as
/// fully-qualified reference paths (`</hotels["sofitel"]>`), shared objects
/// under a sibling top-level attribute.
#[test]
fn ch6_within_object_references_example() {
    let src = r#"destinations = <
    ["seville"] = <
        hotels = <
            ["gran sevilla"] = </hotels["gran sevilla"]>
            ["sofitel"] = </hotels["sofitel"]>
            ["hotel real"] = </hotels["hotel real"]>
        >
    >
>

bookings = <
    ["seville:0134"] = <
        customer_id = <"0134">
        period = <...>
        hotel = </hotels["sofitel"]>
    >
>

hotels = <
    ["gran sevilla"] = (HISTORIC_HOTEL) <...>
    ["sofitel"] = (LUXURY_HOTEL) <...>
    ["hotel real"] = (PENSION) <...>
>"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("the §Within example should parse: {e}"));
    let OdinValue::Object(top) = &parsed else {
        panic!("expected a top-level object");
    };
    assert_eq!(top.len(), 3);
    // the booking's hotel association is a single-path reference block.
    let paths = parsed.paths();
    assert!(paths.contains(&"/bookings[\"seville:0134\"]/hotel".to_owned()));
}

/// The `master06-references` §Across Objects example — an Identified Object
/// Document whose reference paths are rooted at object identifiers
/// (`<["tourism_db_13"]/hotels["sofitel"]>`; #1380).
#[test]
fn ch6_across_objects_references_example() {
    let src = r#"["travel_db_0293822"] = <
    destinations = <
        ["seville"] = <
            hotels = <
                ["gran sevilla"] = <["tourism_db_13"]/hotels["gran sevilla"]>
                ["sofitel"] = <["tourism_db_13"]/hotels["sofitel"]>
                ["hotel real"] = <["tourism_db_13"]/hotels["hotel real"]>
            >
        >
    >

    bookings = <
        ["seville:0134"] = <
            customer_id = <"0134">
            period = <...>
            hotel = <["tourism_db_13"]/hotels["sofitel"]>
        >
    >
>

["tourism_db_13"] = <
    hotels = <
        ["gran sevilla"] = (HISTORIC_HOTEL) <...>
        ["sofitel"] = (LUXURY_HOTEL) <...>
        ["hotel real"] = (PENSION) <...>
    >
>"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("the §Across example should parse: {e}"));
    let OdinValue::KeyedList(entries) = &parsed else {
        panic!("expected a top-level identified document");
    };
    assert_eq!(entries.len(), 2);

    // the association carries the key-rooted path verbatim.
    let flat = format!("{parsed:?}");
    assert!(
        flat.contains(r#"[\"tourism_db_13\"]/hotels[\"sofitel\"]"#),
        "key-rooted reference path missing: {flat}"
    );
}

/// The `master07-leaf_data` §Lists of Built-in Types examples — homogeneous
/// leaf lists, singleton `v, ...` lists, and the whitespace-free identity
/// pair — plus the App.B interval-list productions and the homogeneity
/// refusal twins (#1384).
#[test]
fn ch7_lists_of_built_in_types() {
    // the section's three list examples.
    for src in [
        r#"a = <"cyan", "magenta", "yellow", "black">"#,
        "a = <1, 1, 2, 3, 5>",
        "a = <08:02, 08:35, 09:10>",
    ] {
        let parsed = parse(src).unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        let OdinValue::Object(top) = parsed else {
            panic!("expected object");
        };
        assert!(matches!(top.get("a"), Some(OdinValue::List(_))), "{src}");
    }

    // singleton lists carry the continuation marker.
    for src in [
        r#"a = <"en", ...>"#,
        r#"a = <"icd10", ...>"#,
        "a = <[at0200], ...>",
    ] {
        let parsed = parse(src).unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        let OdinValue::Object(top) = parsed else {
            panic!("expected object");
        };
        let Some(OdinValue::List(items)) = top.get("a") else {
            panic!("{src}: expected a list");
        };
        assert_eq!(items.last(), Some(&OdinValue::ListContinue), "{src}");
    }

    // "the following two lists are identical".
    assert_eq!(parse("a = <1,1,2,3>").ok(), parse("a = <1, 1, 2,3>").ok());

    // interval lists (App.B `*_interval_list_value`), closed and open.
    for src in [
        "a = <|0..5|, |8..9|>",
        "a = <|0..5|, ...>",
        "a = <|P1D..P2D|, |P3D..P4D|>",
    ] {
        assert!(parse(src).is_ok(), "{src} should parse");
    }

    // homogeneity refusal twins — every list production is per-type.
    for src in [
        r#"a = <1, "x">"#,
        "a = <1, 2.5>",
        "a = <2004-06-11, 2004-06-11T10:00:00>",
        r#"a = <|0..5|, "x">"#,
    ] {
        assert!(
            parse(src).is_err(),
            "{src} must be refused (mixed-kind list)"
        );
    }
}

/// The `master07-leaf_data` §Dates and Times examples — the four complete
/// forms (incl. the comma-fraction time and the tz-stamped date/time), the
/// ISO partial patterns, and the `??` partial patterns.
#[test]
fn ch7_date_time_forms() {
    for (src, want) in [
        ("d = <1919-01-23>", "Date"),
        ("d = <16:35:04,5>", "Time"),
        ("d = <2001-05-12T07:35:20+1000>", "DateTime"),
        ("d = <P22DT4H15M0S>", "Duration"),
        ("d = <2004-06>", "Date"),
        ("d = <08:30>", "Time"),
        ("d = <2004-06-11T10:30>", "DateTime"),
        ("d = <2004-06-11T10>", "DateTime"),
        ("d = <2004-06-??>", "Date"),
        ("d = <2004-??-??>", "Date"),
        ("d = <10:30:??>", "Time"),
        ("d = <10:??:??>", "Time"),
        ("d = <2004-06-11T10:30:??>", "DateTime"),
        ("d = <2004-06-11T10:??:??>", "DateTime"),
    ] {
        let parsed = parse(src).unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        let OdinValue::Object(top) = parsed else {
            panic!("expected object");
        };
        let got = match top.get("d") {
            Some(OdinValue::Date(_)) => "Date",
            Some(OdinValue::Time(_)) => "Time",
            Some(OdinValue::DateTime(_)) => "DateTime",
            Some(OdinValue::Duration(_)) => "Duration",
            other => panic!("{src}: unexpected {other:?}"),
        };
        assert_eq!(got, want, "{src}");
    }
}

/// The `master07-leaf_data` §Intervals uniform syntax — all ten listed
/// forms parse (the point/one-sided/plus-minus family plus the section's own
/// examples).
#[test]
fn ch7_interval_uniform_syntax() {
    for src in [
        "i = <|0..5|>",
        "i = <|>0..5|>",
        "i = <|0..<5|>",
        "i = <|>0..<5|>",
        "i = <|<5|>",
        "i = <|>5|>",
        "i = <|>=5|>",
        "i = <|<=5|>",
        "i = <|5.0 +/-0.5|>",
        "i = <|5.0 \u{00B1}0.5|>",
        "i = <|5.0\u{00B1}0.5|>",
        "i = <|0.0..1000.0|>",
        "i = <|0.0..<1000.0|>",
        "i = <|08:02..09:10|>",
        "i = <|>=1939-02-01|>",
    ] {
        assert!(parse(src).is_ok(), "{src} should parse");
    }
}

/// The `master07-leaf_data` §Coded Terms examples, incl. the optional
/// version, and §URIs' verbatim-capture forms.
#[test]
fn ch7_coded_terms_and_uris() {
    for src in [
        "t = <[icd10AM::F60.1]>",
        "t = <[snomed_ct::2004950]>",
        "t = <[snomed_ct(3.1)::2004950]>",
    ] {
        let parsed = parse(src).unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        let OdinValue::Object(top) = parsed else {
            panic!("expected object");
        };
        assert!(
            matches!(top.get("t"), Some(OdinValue::TermCode(_))),
            "{src}"
        );
    }
    for uri in [
        "http://openEHR.org/home",
        "ftp://get.this.file.com?file=cats.doc#section_5",
        "http://www.mozilla.org/products/firefox/upgrade/?application=thunderbird",
    ] {
        let src = format!("u = <{uri}>");
        let parsed = parse(&src).unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        let OdinValue::Object(top) = parsed else {
            panic!("expected object");
        };
        assert_eq!(
            top.get("u"),
            Some(&OdinValue::Uri(format!("<{uri}>"))),
            "verbatim capture incl. delimiters"
        );
    }
}

/// `master08-path_syntax` §Semantics: the chapter's typical path is exactly
/// the shape `OdinValue::paths()` emits, and a reference block carries it.
#[test]
fn ch8_typical_path_shape() {
    let parsed = parse(r#"term_definitions = <["en"] = <items = <["at0001"] = <text = <"x">>>>>"#)
        .unwrap_or_else(|e| panic!("should parse: {e}"));
    assert!(
        parsed
            .paths()
            .contains(&"/term_definitions[\"en\"]/items[\"at0001\"]/text".to_owned())
    );
    // …and the same path is legal as an object reference.
    assert!(parse(r#"r = </term_definitions["en"]/items["at0001"]/text>"#).is_ok());
}

/// `master09-plug_in_syntaxes` (#1387): plug-in blocks
/// `attr = (syntax) <# … #>` parse to [`OdinValue::PlugIn`] with the tag from
/// the parentheses and the body carried verbatim ("expressed in some other
/// syntax" — the body is for a plug-in parser, never interpreted here). The
/// chapter's own cADL example is the pinned acceptance twin; a tag-less
/// `<# … #>` stays refused (the general form makes the syntax tag part of
/// the construct), as does a plug-in block under every non-ODIN reading.
#[test]
fn ch9_plug_in_blocks_parse() {
    // the chapter's example: a cADL plug-in section in an archetype.
    let src = "definition = (cadl) <#
    ENTRY[at0000] \u{2208} { -- blood pressure measurement
        name \u{2208} { -- any synonym of BP
            CODED_TEXT \u{2208} {
                code \u{2208} {
                    CODE_PHRASE \u{2208} {[ac0001]}
                }
            }
        }
    }
#>";
    let parsed = parse(src).unwrap_or_else(|e| panic!("the master09 example should parse: {e}"));
    let OdinValue::Object(top) = parsed else {
        panic!("expected a top-level attribute object");
    };
    let Some(OdinValue::PlugIn { syntax, text }) = top.get("definition") else {
        panic!("expected a plug-in block, got {:?}", top.get("definition"));
    };
    assert_eq!(syntax, "cadl");
    assert!(
        text.contains("ENTRY[at0000]") && text.contains("[ac0001]"),
        "the body is carried verbatim: {text}"
    );

    // `#` inside the body does not close the block; only `#>` does.
    let parsed = parse("a = (xml) <# a # b ## c #>").unwrap_or_else(|e| panic!("{e}"));
    let OdinValue::Object(top) = parsed else {
        panic!("expected object");
    };
    assert_eq!(
        top.get("a"),
        Some(&OdinValue::PlugIn {
            syntax: "xml".to_owned(),
            text: " a # b ## c ".to_owned(),
        })
    );

    // the refusal twins: a tag-less block, and a tagged block whose body
    // never closes.
    assert!(parse("a = <# no tag #>").is_err());
    assert!(parse("a = (cadl) <# never closed").is_err());
}

// ── the nesting bound ─────────────────────────────────────────────────────

fn nested_object(levels: usize) -> String {
    format!("{}\"leaf\"{}", "a = <".repeat(levels), ">".repeat(levels))
}

#[test]
fn object_nesting_past_the_bound_is_refused_before_the_parser_recurses() {
    let err = parse(&nested_object(openehr_lang::nesting::MAX_NESTING_DEPTH + 1))
        .expect_err("a document nested past the bound is refused, never recursed into");
    assert_eq!(
        err.kind,
        OdinErrorKind::NestingTooDeep(openehr_lang::nesting::MAX_NESTING_DEPTH)
    );
    // The refusal points at the block that crossed the bound.
    assert_eq!(err.line, 1);
    let err_v1_0 = openehr_lang::v1_0::odin::parse(&nested_object(
        openehr_lang::nesting::MAX_NESTING_DEPTH + 1,
    ))
    .expect_err("the 1.0.0 reader carries the same implementation bound");
    assert_eq!(
        err_v1_0.kind,
        openehr_lang::v1_0::odin::OdinErrorKind::NestingTooDeep(
            openehr_lang::nesting::MAX_NESTING_DEPTH
        )
    );
}

#[test]
fn object_nesting_at_the_bound_parses() {
    // The bound is set for the engine's 256 MiB thread, not the 2 MiB test
    // thread, so the walk at the bound runs on a thread sized like the engine's.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            let doc = parse(&nested_object(openehr_lang::nesting::MAX_NESTING_DEPTH))
                .expect("a document at the bound parses");
            let mut depth = 0usize;
            let mut cur = &doc;
            while let OdinValue::Object(map) = cur {
                depth += 1;
                cur = map.get("a").expect("the single attribute");
            }
            assert_eq!(depth, openehr_lang::nesting::MAX_NESTING_DEPTH);
        })
        .expect("spawn")
        .join()
        .expect("join");
}
