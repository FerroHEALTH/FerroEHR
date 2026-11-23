// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Typed RM construction and the canonical-JSON seams of the Simplified Formats.
//!
//! The build fills RM objects by attribute name through
//! `openehr_rm::v1_2::access`; [`RawJson`] and [`canonical_of`] write `|raw`
//! output. No openEHR specification governs this construction: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "a client datum and a |raw fragment arrive as JSON (ITS-REST Simplified Formats \
              master04 §Raw canonical JSON), the edge this module reads them across (#1694, #6003)"
)]

use openehr_rm::v1_2::access::{self, AccessError, FromRm, RmObject, RmRef, RmValue, Site, schema};
use serde_json::Value;

use crate::flat::error::FlatError;
use crate::flat::sim::RawDatum;

/// An RM object under construction, or a finished one.
pub(crate) type Obj = Box<dyn RmObject>;

/// An object of class `rm_type` with `attributes` written in order.
///
/// # Errors
/// [`FlatError::Rm`] for an unknown class or a refused attribute.
pub(crate) fn make<'a>(
    rm_type: &str,
    attributes: impl IntoIterator<Item = (&'a str, RmValue)>,
) -> Result<Obj, FlatError> {
    let mut o = access::construct(rm_type)?;
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
/// `None` for a JSON `null`. A number keeps its JSON kind.
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

/// The RM classes a `|raw` datum carries whole (master04 §Raw canonical
/// JSON): the ones a Simplified-Formats attribute takes as one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawClass {
    DataValue,
    Locatable,
    PartyProxy,
    CodePhrase,
    Link,
    FeederAudit,
    Participation,
    ObjectRef,
    HierObjectId,
    ObjectVersionId,
    GenericId,
    PartyRef,
    IsmTransition,
    TermMapping,
    InstructionDetails,
    FeederAuditDetails,
    EventContext,
}

impl RawClass {
    /// The class an object of `rm_type` is read and written as.
    fn of(rm_type: &str) -> Option<Self> {
        let is_a = |ancestor: &str| openehr_rm::v1_2::model::is_a(rm_type, ancestor);
        Some(if is_a("DATA_VALUE") {
            Self::DataValue
        } else if is_a("LOCATABLE") {
            Self::Locatable
        } else if is_a("PARTY_PROXY") {
            Self::PartyProxy
        } else {
            match rm_type {
                "CODE_PHRASE" => Self::CodePhrase,
                "LINK" => Self::Link,
                "FEEDER_AUDIT" => Self::FeederAudit,
                "PARTICIPATION" => Self::Participation,
                "OBJECT_REF" => Self::ObjectRef,
                "HIER_OBJECT_ID" => Self::HierObjectId,
                "OBJECT_VERSION_ID" => Self::ObjectVersionId,
                "GENERIC_ID" => Self::GenericId,
                "PARTY_REF" => Self::PartyRef,
                "ISM_TRANSITION" => Self::IsmTransition,
                "TERM_MAPPING" => Self::TermMapping,
                "INSTRUCTION_DETAILS" => Self::InstructionDetails,
                "FEEDER_AUDIT_DETAILS" => Self::FeederAuditDetails,
                "EVENT_CONTEXT" => Self::EventContext,
                _ => return None,
            }
        })
    }

    /// Whether a client's `|raw` fragment may carry the class: the classes a
    /// Simplified-Formats attribute takes whole on input.
    fn read(self) -> bool {
        matches!(
            self,
            Self::DataValue
                | Self::Locatable
                | Self::PartyProxy
                | Self::CodePhrase
                | Self::Link
                | Self::FeederAudit
                | Self::Participation
                | Self::PartyRef
        )
    }
}

/// Reads `$text` as the class `$class` names into a typed object, through
/// the reader `$read`.
macro_rules! read_raw {
    ($class:expr, $read:ident, $text:expr) => {{
        use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
        use openehr_rm::v1_2::access::AsRm;
        use openehr_rm::v1_2::common::archetyped::feeder_audit::FeederAudit;
        use openehr_rm::v1_2::common::archetyped::link::Link;
        use openehr_rm::v1_2::common::archetyped::locatable::Locatable;
        use openehr_rm::v1_2::common::generic::participation::Participation;
        use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
        use openehr_rm::v1_2::data_types::basic::data_value::DataValue;
        use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
        let object: Option<Result<Obj, _>> = match $class {
            RawClass::DataValue => Some($read::<DataValue>($text).map(|v| v.as_rm().clone_box())),
            RawClass::Locatable => Some($read::<Locatable>($text).map(|v| v.as_rm().clone_box())),
            RawClass::PartyProxy => Some($read::<PartyProxy>($text).map(|v| v.as_rm().clone_box())),
            RawClass::CodePhrase => {
                Some($read::<CodePhrase>($text).map(|v| -> Obj { Box::new(v) }))
            }
            RawClass::Link => Some($read::<Link>($text).map(|v| -> Obj { Box::new(v) })),
            RawClass::FeederAudit => {
                Some($read::<FeederAudit>($text).map(|v| -> Obj { Box::new(v) }))
            }
            RawClass::Participation => {
                Some($read::<Participation>($text).map(|v| -> Obj { Box::new(v) }))
            }
            RawClass::PartyRef => Some($read::<PartyRef>($text).map(|v| -> Obj { Box::new(v) })),
            _ => None,
        };
        object
    }};
}

/// The typed object a `|raw` datum carries (master04 §Raw canonical JSON),
/// read by the strict canonical reader of its class.
///
/// # Errors
/// [`FlatError::RawWithoutType`] for a fragment without `_type`,
/// [`FlatError::InvalidRaw`] for a class no Simplified-Formats attribute
/// takes whole, and [`FlatError::RawDecode`] for one the reader refuses.
pub(crate) fn raw_object(raw: &RawDatum, path: &str) -> Result<Obj, FlatError> {
    let without_type = || FlatError::RawWithoutType {
        path: path.to_owned(),
    };
    let decode_err = |source| FlatError::RawDecode {
        path: path.to_owned(),
        source,
    };
    let rm_type = raw_type(raw).ok_or_else(without_type)?;
    let read = RawClass::of(&rm_type).filter(|class| class.read());
    let invalid = || FlatError::InvalidRaw {
        path: path.to_owned(),
        reason: format!("no simplified attribute takes a {rm_type} whole"),
    };
    match raw {
        RawDatum::Rm(object) => read.map(|_| object.clone_box()).ok_or_else(invalid),
        RawDatum::Json(value) => {
            use openehr_its::json::from_canonical_value;
            let class = read.ok_or_else(invalid)?;
            read_raw!(class, from_canonical_value, value)
                .ok_or_else(invalid)?
                .map_err(decode_err)
        }
        RawDatum::Text(text) => {
            // NOTE: the fragment's read is its part of the FLAT document's one parse (#6003).
            use serde_json::from_str;
            let class = read.ok_or_else(invalid)?;
            read_raw!(class, from_str, text.get())
                .ok_or_else(invalid)?
                .or_else(|_| {
                    // NOTE: the strict reader runs again on a refusal only, for the path it names.
                    use openehr_its::json::from_canonical_json;
                    read_raw!(class, from_canonical_json, text.get())
                        .ok_or_else(invalid)?
                        .map_err(decode_err)
                })
        }
    }
}

/// The `_type` a `|raw` datum names, `None` for one without it or no object.
pub(crate) fn raw_type(raw: &RawDatum) -> Option<std::borrow::Cow<'_, str>> {
    #[derive(serde::Deserialize)]
    struct Tagged<'a> {
        #[serde(rename = "_type", borrow)]
        rm_type: Option<std::borrow::Cow<'a, str>>,
    }
    match raw {
        RawDatum::Rm(object) => Some(std::borrow::Cow::Borrowed(object.rm_type())),
        RawDatum::Json(value) => value
            .get("_type")
            .and_then(Value::as_str)
            .map(std::borrow::Cow::Borrowed),
        // NOTE: a fragment that is no object names no type, the refusal the build gives it.
        RawDatum::Text(text) => text
            .get()
            .trim_start()
            .starts_with('{')
            .then(|| serde_json::from_str::<Tagged<'_>>(text.get()).ok())
            .flatten()
            .and_then(|tagged| tagged.rm_type),
    }
}

/// The `|raw` datum of a typed object, `None` for an object outside the
/// classes a `|raw` output carries.
pub(crate) fn raw_of(object: &dyn RmObject) -> Option<RawDatum> {
    RawClass::of(object.rm_type()).map(|_| RawDatum::Rm(object.clone_box()))
}

/// Converts `$object` into the concrete type of its [`RawClass`] and
/// evaluates `$body` with `$value` bound to it; `$refused` maps the
/// [`AccessError`] of a partial object or a class no `|raw` output carries.
macro_rules! with_raw_value {
    ($object:expr, $refused:expr, |$value:ident| $body:expr) => {{
        use openehr_base::v1_3::base_types::identification::generic_id::GenericId;
        use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
        use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
        use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
        use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
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

        let object: &dyn RmObject = $object;
        let rm_type = object.rm_type();
        let at = Site {
            rm_type,
            attribute: "",
            declared: rm_type,
        };
        let copy = object.clone_box();
        match RawClass::of(rm_type) {
            Some(RawClass::DataValue) => {
                let $value = DataValue::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::Locatable) => {
                let $value = Locatable::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::PartyProxy) => {
                let $value = PartyProxy::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::CodePhrase) => {
                let $value = CodePhrase::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::Link) => {
                let $value = Link::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::FeederAudit) => {
                let $value = FeederAudit::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::Participation) => {
                let $value = Participation::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::ObjectRef) => {
                let $value = ObjectRef::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::HierObjectId) => {
                let $value = HierObjectId::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::ObjectVersionId) => {
                let $value = ObjectVersionId::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::GenericId) => {
                let $value = GenericId::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::PartyRef) => {
                let $value = PartyRef::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::IsmTransition) => {
                let $value = IsmTransition::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::TermMapping) => {
                let $value = TermMapping::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::InstructionDetails) => {
                let $value = InstructionDetails::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::FeederAuditDetails) => {
                let $value = FeederAuditDetails::from_rm(copy, at).map_err($refused)?;
                $body
            }
            Some(RawClass::EventContext) => {
                let $value = EventContext::from_rm(copy, at).map_err($refused)?;
                $body
            }
            None => Err($refused(AccessError::UnknownType(rm_type.to_owned()))),
        }
    }};
}

/// A typed object written as its canonical JSON, the `|raw` output of the
/// FLAT text (master04 §Raw canonical JSON), with no JSON value built.
pub(crate) struct RawJson<'a>(pub(crate) &'a dyn RmObject);

impl serde::Serialize for RawJson<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        with_raw_value!(self.0, S::Error::custom, |value| value
            .serialize(serializer))
    }
}

/// The canonical JSON of a typed object, as the `|raw` output of a FLAT map
/// or a STRUCTURED document carries it (master04 §Raw canonical JSON).
///
/// # Errors
/// [`FlatError::Rm`] for an object outside the classes a `|raw` output
/// carries, or a partial one.
pub(crate) fn canonical_of(object: &dyn RmObject) -> Result<Value, FlatError> {
    use openehr_its::json::to_canonical_value;
    with_raw_value!(object, FlatError::Rm, |value| Ok(to_canonical_value(
        &value
    )))
}

/// The canonical-JSON rendering of a tree that may still be partial: `_type`
/// first, then every attribute holding a value in declaration order.
///
/// `ITEM_TREE`, `ITEM_LIST` and `CLUSTER` `items` and `ITEM_TABLE` `rows` render
/// as an empty array when empty.
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

/// An `OBJECT_REF` in `namespace` to an object of `rm_type` whose id is the
/// `GENERIC_ID` `id` under `scheme` (BASE identification §OBJECT_REF).
///
/// # Errors
/// [`FlatError::Rm`], never for these classes.
pub(crate) fn generic_object_ref(
    namespace: &str,
    rm_type: &str,
    id: &str,
    scheme: &str,
) -> Result<Obj, FlatError> {
    make(
        "OBJECT_REF",
        [
            ("namespace", s(namespace)),
            ("type", s(rm_type)),
            (
                "id",
                o(make(
                    "GENERIC_ID",
                    [("value", s(id)), ("scheme", s(scheme))],
                )?),
            ),
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
    identifier(HierObjectId::new(value), path)
}

/// The identifier a validating constructor `made`, or its refusal as the
/// invalid value at `path`.
fn identifier<T: RmObject, E: std::fmt::Display>(
    made: Result<T, E>,
    path: &str,
) -> Result<Obj, FlatError> {
    made.map(|id| -> Obj { Box::new(id) })
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
    identifier(ObjectVersionId::new(value), path)
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
