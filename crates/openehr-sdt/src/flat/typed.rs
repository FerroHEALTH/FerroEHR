// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Typed RM construction and the canonical-JSON seams of the Simplified
//! Formats (#6003).
//!
//! The FLAT and STRUCTURED build fills RM objects by attribute name through
//! the reflective accessor of `openehr_rm::v1_2::access`: [`new`] starts an
//! object with every attribute unset, [`Build`] writes into it, and the
//! caller's `finish` turns the tree into the real struct. A datum the client
//! wrote as JSON enters through [`Build::put_datum`], which reads it as the
//! attribute's primitive; a `|raw` fragment through [`decode_raw`], the strict
//! canonical reader of its class. [`canonical_of`] renders a typed object back
//! to canonical JSON, the one form `|raw` output takes. No openEHR
//! specification governs this construction: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "a client datum and a |raw fragment arrive as JSON (ITS-REST Simplified Formats \
              master04 §Raw canonical JSON), the edge this module reads them across (#1694, #6003)"
)]

use openehr_rm::v1_2::access::FromRm;
use openehr_rm::v1_2::access::{self, AccessError, RmObject, RmRef, RmValue, Site, schema};
use serde_json::Value;

use crate::flat::error::FlatError;

/// An RM object under construction, or a finished one.
pub(crate) type Obj = Box<dyn RmObject>;

/// Starts an object of the concrete class `rm_type`, every attribute unset.
///
/// # Errors
/// [`FlatError::Rm`] for a name that is no concrete RM class.
pub(crate) fn new(rm_type: &str) -> Result<Obj, FlatError> {
    Ok(access::construct(rm_type)?)
}

/// An object of class `rm_type` with `attributes` written in order.
///
/// # Errors
/// [`FlatError::Rm`] for an unknown class or a refused attribute.
pub(crate) fn make<'a>(
    rm_type: &str,
    attributes: impl IntoIterator<Item = (&'a str, RmValue)>,
) -> Result<Obj, FlatError> {
    let mut o = new(rm_type)?;
    for (attr, value) in attributes {
        o.put(attr, value)?;
    }
    Ok(o)
}

/// A string value.
pub(crate) fn s(value: impl Into<String>) -> RmValue {
    RmValue::String(value.into())
}

/// An object value.
pub(crate) fn o(object: Obj) -> RmValue {
    RmValue::Object(object)
}

/// The read-only view of an object, for the [`crate::flat::instance`] walks.
pub(crate) fn view(object: &dyn RmObject) -> RmRef<'_> {
    RmRef::Object(object)
}

/// Writes into an RM object by attribute name.
pub(crate) trait Build {
    /// Write `value` into the single-valued `attr`.
    ///
    /// # Errors
    /// [`FlatError::Rm`] when the class refuses the attribute or the value.
    fn put(&mut self, attr: &str, value: RmValue) -> Result<(), FlatError>;

    /// Write `value` into `attr` when `attr` holds nothing yet.
    ///
    /// # Errors
    /// [`FlatError::Rm`] when the class refuses the attribute or the value,
    /// and whatever `value` returns.
    fn or_put(
        &mut self,
        attr: &str,
        value: impl FnOnce() -> Result<RmValue, FlatError>,
    ) -> Result<(), FlatError>;

    /// Append `value` to the container `attr`.
    ///
    /// # Errors
    /// [`FlatError::Rm`] when the class refuses the attribute or the value.
    fn push_obj(&mut self, attr: &str, value: Obj) -> Result<(), FlatError>;

    /// Write the client's JSON datum `value` into `attr` as the primitive the
    /// attribute declares; a JSON `null` writes nothing.
    ///
    /// # Errors
    /// [`FlatError::Rm`] for a JSON array or object, or an attribute the
    /// class does not declare. A primitive of the wrong kind is refused when
    /// the tree is finished.
    fn put_datum(&mut self, attr: &str, value: &Value) -> Result<(), FlatError>;

    /// The string attribute `attr`.
    fn str_of(&self, attr: &str) -> Option<&str>;
}

impl Build for dyn RmObject {
    fn put(&mut self, attr: &str, value: RmValue) -> Result<(), FlatError> {
        Ok(self.set(attr, value)?)
    }

    fn or_put(
        &mut self,
        attr: &str,
        value: impl FnOnce() -> Result<RmValue, FlatError>,
    ) -> Result<(), FlatError> {
        if self.is_set(attr) {
            return Ok(());
        }
        let value = value()?;
        self.put(attr, value)
    }

    fn push_obj(&mut self, attr: &str, value: Obj) -> Result<(), FlatError> {
        Ok(self.push(attr, value)?)
    }

    fn put_datum(&mut self, attr: &str, value: &Value) -> Result<(), FlatError> {
        if let Some(v) = datum(self, attr, value)? {
            self.put(attr, v)?;
        }
        Ok(())
    }

    fn str_of(&self, attr: &str) -> Option<&str> {
        match self.attribute(attr)? {
            RmRef::String(s) => Some(s),
            _ => None,
        }
    }
}

/// The primitive a client's JSON `value` writes into `attr` of `object`;
/// `None` for a JSON `null`.
///
/// A number keeps its JSON kind: the finish reads an integer into a `Real`
/// attribute the way the canonical reader does, and refuses a real number in
/// an `Integer` one.
///
/// # Errors
/// [`FlatError::Rm`] for an undeclared attribute, or a JSON array or object.
pub(crate) fn datum(
    object: &dyn RmObject,
    attr: &str,
    value: &Value,
) -> Result<Option<RmValue>, FlatError> {
    let rm_type = object.rm_type();
    let Some(info) = schema::attribute(rm_type, attr) else {
        return Err(FlatError::Rm(AccessError::UndeclaredAttribute {
            rm_type,
            attribute: attr.to_owned(),
        }));
    };
    let refuse = |given: &str| {
        FlatError::Rm(AccessError::NotConforming {
            rm_type,
            attribute: attr.to_owned(),
            declared: info.declared_type,
            given: given.to_owned(),
        })
    };
    Ok(Some(match value {
        Value::Null => return Ok(None),
        // NOTE: ITS-JSON writes a `Character` (TERM_MAPPING.match) as a one-character string.
        Value::String(s) if info.declared_type == "Character" => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => RmValue::Character(c),
                _ => return Err(refuse("a string of other than one character")),
            }
        }
        Value::String(s) => RmValue::String(s.clone()),
        Value::Bool(b) => RmValue::Boolean(*b),
        // NOTE: a number keeps its JSON kind; a `Real` attribute reads an integer at the finish.
        Value::Number(n) => match n.as_i64() {
            Some(i) => RmValue::Integer(i),
            None => RmValue::Real(n.as_f64().ok_or_else(|| refuse("a number"))?),
        },
        Value::Array(_) => return Err(refuse("a JSON array")),
        Value::Object(_) => return Err(refuse("a JSON object")),
    }))
}

/// Finishes the tree `root` as the real `T`.
///
/// # Errors
/// [`FlatError::Rm`] naming the first mandatory attribute left unset, or a
/// value of the wrong kind.
pub(crate) fn finish<T: FromRm>(root: Obj) -> Result<T, FlatError> {
    Ok(access::finish::<T>(root)?)
}

/// The typed object a `|raw` canonical-JSON fragment carries (master04 §Raw
/// canonical JSON), read by the strict canonical reader of its class.
///
/// # Errors
/// [`FlatError::RawWithoutType`] for a fragment without `_type`,
/// [`FlatError::InvalidRaw`] for a class no Simplified-Formats attribute
/// takes whole, and [`FlatError::RawDecode`] for one the reader refuses.
pub(crate) fn decode_raw(raw: &Value, path: &str) -> Result<Obj, FlatError> {
    use openehr_its::json::from_canonical_value;
    use openehr_rm::v1_2::access::AsRm;
    use openehr_rm::v1_2::common::archetyped::feeder_audit::FeederAudit;
    use openehr_rm::v1_2::common::archetyped::link::Link;
    use openehr_rm::v1_2::common::archetyped::locatable::Locatable;
    use openehr_rm::v1_2::common::generic::participation::Participation;
    use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
    use openehr_rm::v1_2::data_types::basic::data_value::DataValue;
    use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;

    let Some(rm_type) = raw.get("_type").and_then(Value::as_str) else {
        return Err(FlatError::RawWithoutType {
            path: path.to_owned(),
        });
    };
    let decode_err = |source| FlatError::RawDecode {
        path: path.to_owned(),
        source,
    };
    let is_a = |ancestor: &str| schema::is_a(rm_type, ancestor);
    let object: Obj = if is_a("DATA_VALUE") {
        from_canonical_value::<DataValue>(raw)
            .map_err(decode_err)?
            .as_rm()
            .clone_box()
    } else if is_a("LOCATABLE") {
        from_canonical_value::<Locatable>(raw)
            .map_err(decode_err)?
            .as_rm()
            .clone_box()
    } else if is_a("PARTY_PROXY") {
        from_canonical_value::<PartyProxy>(raw)
            .map_err(decode_err)?
            .as_rm()
            .clone_box()
    } else {
        match rm_type {
            "CODE_PHRASE" => Box::new(from_canonical_value::<CodePhrase>(raw).map_err(decode_err)?),
            "LINK" => Box::new(from_canonical_value::<Link>(raw).map_err(decode_err)?),
            "FEEDER_AUDIT" => {
                Box::new(from_canonical_value::<FeederAudit>(raw).map_err(decode_err)?)
            }
            "PARTICIPATION" => {
                Box::new(from_canonical_value::<Participation>(raw).map_err(decode_err)?)
            }
            "PARTY_REF" => Box::new(
                from_canonical_value::<
                    openehr_base::v1_3::base_types::identification::party_ref::PartyRef,
                >(raw)
                .map_err(decode_err)?,
            ),
            other => {
                return Err(FlatError::InvalidRaw {
                    path: path.to_owned(),
                    reason: format!("no simplified attribute takes a {other} whole"),
                });
            }
        }
    };
    Ok(object)
}

/// The canonical JSON of a typed object, as the `|raw` output of the FLAT and
/// STRUCTURED read carries it (master04 §Raw canonical JSON).
///
/// # Errors
/// [`FlatError::Rm`] for an object outside the classes a `|raw` output
/// carries, or a partial one.
pub(crate) fn canonical_of(object: &dyn RmObject) -> Result<Value, FlatError> {
    use openehr_base::v1_3::base_types::identification::generic_id::GenericId;
    use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
    use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
    use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
    use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
    use openehr_its::json::to_canonical_value;
    use openehr_rm::v1_2::common::archetyped::feeder_audit::FeederAudit;
    use openehr_rm::v1_2::common::archetyped::feeder_audit_details::FeederAuditDetails;
    use openehr_rm::v1_2::common::archetyped::link::Link;
    use openehr_rm::v1_2::common::archetyped::locatable::Locatable;
    use openehr_rm::v1_2::common::generic::participation::Participation;
    use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
    use openehr_rm::v1_2::composition::content::entry::instruction_details::InstructionDetails;
    use openehr_rm::v1_2::composition::content::entry::ism_transition::IsmTransition;
    use openehr_rm::v1_2::composition::event_context::EventContext;
    use openehr_rm::v1_2::data_types::basic::data_value::DataValue;
    use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
    use openehr_rm::v1_2::data_types::text::term_mapping::TermMapping;

    let rm_type = object.rm_type();
    let at = Site {
        rm_type,
        attribute: "",
        declared: rm_type,
    };
    let copy = object.clone_box();
    let is_a = |ancestor: &str| schema::is_a(rm_type, ancestor);
    Ok(if is_a("DATA_VALUE") {
        to_canonical_value(&DataValue::from_rm(copy, at)?)
    } else if is_a("LOCATABLE") {
        to_canonical_value(&Locatable::from_rm(copy, at)?)
    } else if is_a("PARTY_PROXY") {
        to_canonical_value(&PartyProxy::from_rm(copy, at)?)
    } else {
        match rm_type {
            "CODE_PHRASE" => to_canonical_value(&CodePhrase::from_rm(copy, at)?),
            "LINK" => to_canonical_value(&Link::from_rm(copy, at)?),
            "FEEDER_AUDIT" => to_canonical_value(&FeederAudit::from_rm(copy, at)?),
            "PARTICIPATION" => to_canonical_value(&Participation::from_rm(copy, at)?),
            "OBJECT_REF" => to_canonical_value(&ObjectRef::from_rm(copy, at)?),
            "HIER_OBJECT_ID" => to_canonical_value(&HierObjectId::from_rm(copy, at)?),
            "OBJECT_VERSION_ID" => to_canonical_value(&ObjectVersionId::from_rm(copy, at)?),
            "GENERIC_ID" => to_canonical_value(&GenericId::from_rm(copy, at)?),
            "PARTY_REF" => to_canonical_value(&PartyRef::from_rm(copy, at)?),
            "ISM_TRANSITION" => to_canonical_value(&IsmTransition::from_rm(copy, at)?),
            "TERM_MAPPING" => to_canonical_value(&TermMapping::from_rm(copy, at)?),
            "INSTRUCTION_DETAILS" => to_canonical_value(&InstructionDetails::from_rm(copy, at)?),
            "FEEDER_AUDIT_DETAILS" => to_canonical_value(&FeederAuditDetails::from_rm(copy, at)?),
            "EVENT_CONTEXT" => to_canonical_value(&EventContext::from_rm(copy, at)?),
            _ => return Err(FlatError::Rm(AccessError::UnknownType(rm_type.to_owned()))),
        }
    })
}

/// The canonical-JSON rendering of a tree that may still be partial: `_type`
/// first, then every attribute that holds a value in declaration order, the
/// shape the canonical-JSON build entry points return unchecked.
///
/// The structure classes whose member list the canonical-JSON build always
/// wrote (`ITEM_TREE`, `ITEM_LIST` and `CLUSTER` `items`, `ITEM_TABLE`
/// `rows`) carry it as an empty array when it holds no member, as that build
/// wrote it.
pub(crate) fn render(object: &dyn RmObject) -> Value {
    let rm_type = object.rm_type();
    let mut out = serde_json::Map::new();
    out.insert("_type".to_owned(), Value::from(rm_type));
    // NOTE: a class built through its validating constructor (an identifier) declares only `value`.
    let names = schema::attribute_names(rm_type).unwrap_or(&["value"]);
    for name in names {
        if let Some(items) = object.items(name) {
            let members: Vec<Value> = items.map(render).collect();
            if !members.is_empty() {
                out.insert((*name).to_owned(), Value::Array(members));
            }
            continue;
        }
        let value = match object.attribute(name) {
            Some(RmRef::Object(o)) => render(o),
            Some(RmRef::String(s)) => Value::from(s),
            Some(RmRef::Integer(i)) => Value::from(i),
            Some(RmRef::Real(r)) => Value::from(r),
            Some(RmRef::Boolean(b)) => Value::from(b),
            Some(RmRef::Character(c)) => Value::from(c.to_string()),
            None => continue,
        };
        out.insert((*name).to_owned(), value);
    }
    let member_list = match rm_type {
        "ITEM_TREE" | "ITEM_LIST" | "CLUSTER" => Some("items"),
        "ITEM_TABLE" => Some("rows"),
        _ => None,
    };
    if let Some(list) = member_list {
        out.entry(list.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    Value::Object(out)
}

// ── the leaf shapes the build writes most ──────────────────────────────────

/// A `TERMINOLOGY_ID`.
///
/// # Errors
/// [`FlatError::Rm`], never for this class.
pub(crate) fn terminology_id(value: &str) -> Result<Obj, FlatError> {
    make("TERMINOLOGY_ID", [("value", s(value))])
}

/// A `CODE_PHRASE` (RM data_types §CODE_PHRASE).
///
/// # Errors
/// [`FlatError::Rm`], never for this class.
pub(crate) fn code_phrase(terminology: &str, code: &str) -> Result<Obj, FlatError> {
    make(
        "CODE_PHRASE",
        [
            ("terminology_id", o(terminology_id(terminology)?)),
            ("code_string", s(code)),
        ],
    )
}

/// A `DV_TEXT`.
///
/// # Errors
/// [`FlatError::Rm`], never for this class.
pub(crate) fn dv_text(value: &str) -> Result<Obj, FlatError> {
    make("DV_TEXT", [("value", s(value))])
}

/// A `DV_CODED_TEXT` from its display value and `(terminology, code)`.
///
/// # Errors
/// [`FlatError::Rm`], never for this class.
pub(crate) fn dv_coded_text(value: &str, terminology: &str, code: &str) -> Result<Obj, FlatError> {
    make(
        "DV_CODED_TEXT",
        [
            ("value", s(value)),
            ("defining_code", o(code_phrase(terminology, code)?)),
        ],
    )
}

/// A `DV_DATE_TIME`.
///
/// # Errors
/// [`FlatError::Rm`], never for this class.
pub(crate) fn dv_date_time(value: &str) -> Result<Obj, FlatError> {
    make("DV_DATE_TIME", [("value", s(value))])
}

/// A `HIER_OBJECT_ID` (or the `OBJECT_VERSION_ID` a `::`-bearing value names
/// when `version` is set), through its validating constructor.
///
/// # Errors
/// [`FlatError::InvalidValue`] at `path` for a value the identifier grammar
/// refuses (BASE `master09-identification`).
pub(crate) fn hier_object_id(value: &str, path: &str) -> Result<Obj, FlatError> {
    use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
    HierObjectId::new(value)
        .map(|id| -> Obj { Box::new(id) })
        .map_err(|e| FlatError::InvalidValue {
            path: path.to_owned(),
            reason: e.to_string(),
        })
}

/// An `OBJECT_VERSION_ID`, through its validating constructor.
///
/// # Errors
/// [`FlatError::InvalidValue`] at `path` for a value the identifier grammar
/// refuses (BASE `master09-identification`).
pub(crate) fn object_version_id(value: &str, path: &str) -> Result<Obj, FlatError> {
    use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
    ObjectVersionId::new(value)
        .map(|id| -> Obj { Box::new(id) })
        .map_err(|e| FlatError::InvalidValue {
            path: path.to_owned(),
            reason: e.to_string(),
        })
}

/// The string at the single-valued path `steps` below `object`.
pub(crate) fn str_at<'a>(object: &'a dyn RmObject, steps: &[&str]) -> Option<&'a str> {
    let (last, parents) = steps.split_last()?;
    let mut cur = object;
    for step in parents {
        match cur.attribute(step)? {
            RmRef::Object(o) => cur = o,
            _ => return None,
        }
    }
    match cur.attribute(last)? {
        RmRef::String(s) => Some(s),
        _ => None,
    }
}
