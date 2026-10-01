// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Smoke test for the generated ITS-REST contract: the DTOs serde
//! round-trip, the route table is populated, and the server trait is nameable.
use openehr_its::rest::generated::query;

/// The generated server trait is a real, nameable bound.
fn _assert_is_trait<T: query::server::QueryApi>() {}

#[test]
fn query_contract_is_usable() {
    // Route table reflects the OAS operations.
    assert_eq!(query::ROUTES.len(), 6);
    assert!(
        query::ROUTES
            .iter()
            .any(|(m, p, _)| *m == "POST" && *p == "/query/aql")
    );

    // A DTO deserializes from canonical JSON and back.
    let j = serde_json::json!({"q": "SELECT c FROM COMPOSITION c", "offset": 0, "fetch": 10});
    let dto: query::AdhocQueryExecute = serde_json::from_value(j).expect("deserialize DTO");
    assert_eq!(dto.q, "SELECT c FROM COMPOSITION c");
    assert_eq!(dto.fetch, Some(10));

    let rs = query::ResultSet {
        meta: None,
        name: None,
        q: None,
        columns: None,
        rows: vec![],
    };
    let s = serde_json::to_string(&rs).expect("serialize ResultSet");
    assert!(s.contains("\"rows\""));
}

/// An absent optional property is OMITTED, never serialized as `null`.
///
/// No component schema in the vendored ITS-REST bundles
/// (`crates/openehr-its/vendor/rest-oas/`) sets `nullable: true`, and an
/// OpenAPI 3.0 property typed `string` (or `$ref`-ing a string alias) does not
/// admit `null` (<https://spec.openapis.org/oas/v3.0.3#schema-object>:
/// "nullable … Default value is `false`"). So the emitted DTOs must skip their
/// `None` fields — a served `null` would be an undeclared value for the
/// declared type.
#[test]
fn absent_optional_properties_are_omitted() {
    let rs = query::ResultSet {
        meta: None,
        name: None,
        q: None,
        columns: None,
        rows: vec![],
    };
    let value = serde_json::to_value(&rs).expect("serialize ResultSet");
    let object = value
        .as_object()
        .expect("ResultSet serializes to an object");
    assert_eq!(
        object.keys().collect::<Vec<_>>(),
        vec!["rows"],
        "only the required `rows` property may appear when everything else is absent"
    );

    // The same holds one level down, and for a DTO whose every property is
    // optional (`ResultSetMetadata`).
    let meta = query::ResultSetMetadata {
        _href: None,
        _type: Some("RESULTSET".to_owned()),
        _schema_version: None,
        _created: None,
        _generator: None,
        _executed_aql: None,
        additional_properties: std::collections::BTreeMap::new(),
    };
    let value = serde_json::to_value(&meta).expect("serialize ResultSetMetadata");
    let object = value
        .as_object()
        .expect("ResultSetMetadata serializes to an object");
    assert_eq!(object.keys().collect::<Vec<_>>(), vec!["_type"]);

    // A param struct's optional members follow the same rule.
    let params = query::QueryExecuteAdhocQueryParams {
        q: "SELECT c FROM COMPOSITION c".to_owned(),
        ehr_id: None,
        offset: None,
        fetch: None,
        query_parameters: None,
        accept: None,
    };
    let value = serde_json::to_value(&params).expect("serialize the param struct");
    let object = value
        .as_object()
        .expect("the param struct serializes to an object");
    assert_eq!(object.keys().collect::<Vec<_>>(), vec!["q"]);
}

/// `ResultSetMetadata` declares `additionalProperties: true` — its designated
/// extension point — so undeclared members are carried, not dropped, and are
/// written at the object's own level.
#[test]
fn result_set_metadata_carries_additional_properties() {
    let wire = serde_json::json!({
        "_type": "RESULTSET",
        "_vendor_extension": {"trace_id": "abc"},
    });
    let meta: query::ResultSetMetadata =
        serde_json::from_value(wire).expect("deserialize ResultSetMetadata");
    assert_eq!(meta._type.as_deref(), Some("RESULTSET"));
    assert_eq!(
        meta.additional_properties.get("_vendor_extension"),
        Some(&serde_json::json!({"trace_id": "abc"})),
        "an undeclared member is collected into the extension map"
    );

    let value = serde_json::to_value(&meta).expect("serialize ResultSetMetadata");
    assert_eq!(
        value,
        serde_json::json!({"_type": "RESULTSET", "_vendor_extension": {"trace_id": "abc"}}),
        "extension members are flattened back to the object's own level"
    );
}

/// An array property whose OAS `items` is a `$ref` keeps the referenced type:
/// `ResultSet.columns` is `items: $ref ResultSetColumn`, so the emitted field
/// is `Vec<ResultSetColumn>` rather than an untyped `Vec<serde_json::Value>`.
#[test]
fn array_items_keep_their_referenced_type() {
    let columns: Vec<query::ResultSetColumn> = vec![query::ResultSetColumn {
        name: "#0".to_owned(),
        path: Some("/ehr_id/value".to_owned()),
    }];
    let rs = query::ResultSet {
        meta: None,
        name: None,
        q: None,
        columns: Some(columns),
        rows: vec![vec![serde_json::Value::Null]],
    };
    let value = serde_json::to_value(&rs).expect("serialize ResultSet");
    assert_eq!(
        value,
        serde_json::json!({
            "columns": [{"name": "#0", "path": "/ehr_id/value"}],
            "rows": [[null]],
        }),
        "a typed column omits its absent optional property; a row cell keeps its null"
    );

    let back: query::ResultSet = serde_json::from_value(value).expect("deserialize ResultSet");
    let columns = back.columns.expect("columns round-trip");
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].name, "#0");
}
