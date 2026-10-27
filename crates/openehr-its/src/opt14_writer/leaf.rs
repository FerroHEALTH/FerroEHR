// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Leaf redefinition: a template's leaf constraint admits no value the
//! archetype's leaf does not (`AM/docs/AOM2/master08-validation.adoc` §Phase
//! 2, VPOV; `AM/docs/ADL2/master09.05-spec_object_redef.adoc` §Primitive
//! Object Redefinition and §Terminology Constraint Redefinition).

use std::cmp::Ordering;

use openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase;
use openehr_base::v1_3::foundation_types::time::iso8601_date::Iso8601Date;
use openehr_base::v1_3::foundation_types::time::iso8601_date_time::Iso8601DateTime;
use openehr_base::v1_3::foundation_types::time::iso8601_duration::Iso8601Duration;
use openehr_base::v1_3::foundation_types::time::iso8601_time::Iso8601Time;

use super::WriteError;
use super::interval;
use super::redefine::Site;
use crate::opt14::types::{
    CBoolean, CCodePhrase, CDate, CDateTime, CDuration, CDvOrdinal, CDvQuantity, CInteger, CObject,
    CPrimitive, CReal, CString, CTime, Intervalofinteger, Intervalofreal, ValidityKind,
};

/// Checks that the leaf `given` admits no value `archetype` does not.
///
/// Only the leaf kinds that carry a value constraint are checked: a primitive
/// object, a `C_CODE_PHRASE`, a `C_DV_QUANTITY` and a `C_DV_ORDINAL`. Over
/// one of those, `given` must be the same kind.
pub(super) fn narrows(
    archetype: &CObject,
    given: &CObject,
    site: &Site<'_>,
) -> Result<(), WriteError> {
    // NOTE: AOM2 master04.5 §Conformance Semantics: C_PRIMITIVE_OBJECT compares a node "of the
    // same AOM type"; a leaf of another kind cannot be shown to narrow the archetype's.
    let check = Check { site };
    match (archetype, given) {
        (CObject::CPrimitiveObject(archetype), CObject::CPrimitiveObject(given)) => {
            match (archetype.item.as_deref(), given.item.as_deref()) {
                (None, _) => Ok(()),
                (Some(archetype), Some(given)) => check.primitive(archetype, given),
                (Some(archetype), None) => {
                    Err(check.widening("leaf constraint", "any".to_owned(), kind_of(archetype)))
                }
            }
        }
        (CObject::CCodePhrase(archetype), CObject::CCodePhrase(given)) => {
            check.code_phrase(archetype, given)
        }
        (CObject::CDvQuantity(archetype), CObject::CDvQuantity(given)) => {
            check.quantity(archetype, given)
        }
        (CObject::CDvOrdinal(archetype), CObject::CDvOrdinal(given)) => {
            check.ordinal(archetype, given)
        }
        (
            CObject::CPrimitiveObject(_)
            | CObject::CCodePhrase(_)
            | CObject::CDvQuantity(_)
            | CObject::CDvOrdinal(_),
            _,
        ) => Err(check.widening(
            "leaf constraint",
            object_kind(given).to_owned(),
            object_kind(archetype).to_owned(),
        )),
        _ => Ok(()),
    }
}

fn object_kind(object: &CObject) -> &'static str {
    match object {
        CObject::ArchetypeInternalRef(_) => "ARCHETYPE_INTERNAL_REF",
        CObject::ArchetypeSlot(_) => "ARCHETYPE_SLOT",
        CObject::ConstraintRef(_) => "CONSTRAINT_REF",
        CObject::CArchetypeRoot(_) => "C_ARCHETYPE_ROOT",
        CObject::CCodePhrase(_) => "C_CODE_PHRASE",
        CObject::CCodeReference(_) => "C_CODE_REFERENCE",
        CObject::CComplexObject(_) => "C_COMPLEX_OBJECT",
        CObject::CDefinedObject(_) => "C_DEFINED_OBJECT",
        CObject::CDvOrdinal(_) => "C_DV_ORDINAL",
        CObject::CDvQuantity(_) => "C_DV_QUANTITY",
        CObject::CDvState(_) => "C_DV_STATE",
        CObject::CPrimitiveObject(_) => "C_PRIMITIVE_OBJECT",
        CObject::TComplexObject(_) => "T_COMPLEX_OBJECT",
    }
}

fn kind_of(primitive: &CPrimitive) -> String {
    match primitive {
        CPrimitive::CBoolean(_) => "C_BOOLEAN",
        CPrimitive::CDate(_) => "C_DATE",
        CPrimitive::CDateTime(_) => "C_DATE_TIME",
        CPrimitive::CDuration(_) => "C_DURATION",
        CPrimitive::CInteger(_) => "C_INTEGER",
        CPrimitive::CReal(_) => "C_REAL",
        CPrimitive::CString(_) => "C_STRING",
        CPrimitive::CTime(_) => "C_TIME",
    }
    .to_owned()
}

/// The checks of one redefinition site.
struct Check<'a, 'b> {
    site: &'a Site<'b>,
}

impl Check<'_, '_> {
    fn widening(&self, what: &'static str, given: String, archetype: String) -> WriteError {
        WriteError::Widening {
            archetype_id: self.site.archetype_id.to_owned(),
            path: self.site.path.to_owned(),
            what,
            given,
            archetype,
        }
    }

    fn holds(
        &self,
        admitted: bool,
        what: &'static str,
        given: impl FnOnce() -> String,
        archetype: impl FnOnce() -> String,
    ) -> Result<(), WriteError> {
        if admitted {
            Ok(())
        } else {
            Err(self.widening(what, given(), archetype()))
        }
    }

    fn primitive(&self, archetype: &CPrimitive, given: &CPrimitive) -> Result<(), WriteError> {
        match (archetype, given) {
            (CPrimitive::CBoolean(archetype), CPrimitive::CBoolean(given)) => {
                self.boolean(archetype, given)
            }
            (CPrimitive::CString(archetype), CPrimitive::CString(given)) => {
                self.string(archetype, given)
            }
            (CPrimitive::CInteger(archetype), CPrimitive::CInteger(given)) => {
                self.integer(archetype, given)
            }
            (CPrimitive::CReal(archetype), CPrimitive::CReal(given)) => self.real(archetype, given),
            (CPrimitive::CDate(archetype), CPrimitive::CDate(given)) => self.date(archetype, given),
            (CPrimitive::CDateTime(archetype), CPrimitive::CDateTime(given)) => {
                self.date_time(archetype, given)
            }
            (CPrimitive::CTime(archetype), CPrimitive::CTime(given)) => self.time(archetype, given),
            (CPrimitive::CDuration(archetype), CPrimitive::CDuration(given)) => {
                self.duration(archetype, given)
            }
            _ => Err(self.widening("leaf constraint", kind_of(given), kind_of(archetype))),
        }
    }

    fn boolean(&self, archetype: &CBoolean, given: &CBoolean) -> Result<(), WriteError> {
        // NOTE: AOM2 master04.5 §Conformance semantics: C_BOOLEAN: each value the node allows is
        // one `other` allows.
        let shown = |b: &CBoolean| {
            let mut values = Vec::new();
            if b.true_valid {
                values.push("True");
            }
            if b.false_valid {
                values.push("False");
            }
            format!("{{{}}}", values.join(", "))
        };
        self.holds(
            (!given.true_valid || archetype.true_valid)
                && (!given.false_valid || archetype.false_valid),
            "boolean values",
            || shown(given),
            || shown(archetype),
        )
    }

    fn string(&self, archetype: &CString, given: &CString) -> Result<(), WriteError> {
        // NOTE: AOM2 master04.5 §Conformance semantics: C_STRING: each string the node allows is
        // one `other` allows; an open list admits any string (AOM 1.4 `C_STRING.list_open`).
        let Some(archetype_set) = StringSet::of(archetype) else {
            return Ok(());
        };
        let given_set = StringSet::of(given);
        let admitted = match (&given_set, &archetype_set) {
            (Some(StringSet::List(given)), StringSet::List(archetype)) => {
                given.iter().all(|value| archetype.contains(value))
            }
            (Some(StringSet::List(given)), StringSet::Pattern(pattern)) => {
                regex::Regex::new(&format!("^(?:{pattern})$"))
                    .is_ok_and(|pattern| given.iter().all(|value| pattern.is_match(value)))
            }
            (Some(StringSet::Pattern(given)), StringSet::Pattern(archetype)) => given == archetype,
            (None | Some(StringSet::Pattern(_)), _) => false,
        };
        self.holds(
            admitted,
            "string constraint",
            || given_set.map_or_else(|| "any".to_owned(), |set| set.to_string()),
            || archetype_set.to_string(),
        )
    }

    fn integer(&self, archetype: &CInteger, given: &CInteger) -> Result<(), WriteError> {
        // NOTE: AOM2 master04.5 §Conformance semantics: C_ORDERED: each value or range the node
        // allows lies in a value or range of `other`.
        let shown = |c: &CInteger| {
            if !c.list.is_empty() {
                list_text(c.list.iter())
            } else if let Some(range) = &c.range {
                interval::display(range)
            } else {
                "any".to_owned()
            }
        };
        let point = |v: i32| Intervalofinteger {
            lower_included: Some(true),
            upper_included: Some(true),
            lower_unbounded: false,
            upper_unbounded: false,
            lower: Some(v),
            upper: Some(v),
        };
        let admitted = if !archetype.list.is_empty() {
            if given.list.is_empty() {
                given
                    .range
                    .as_ref()
                    .is_some_and(|range| match interval::inclusive_bounds(range) {
                        (Some(lower), Some(upper)) => (lower..=upper)
                            .take(archetype.list.len().saturating_add(1))
                            .all(|v| archetype.list.iter().any(|a| i64::from(*a) == v)),
                        _ => false,
                    })
            } else {
                given.list.iter().all(|v| archetype.list.contains(v))
            }
        } else if let Some(range) = &archetype.range {
            if given.list.is_empty() {
                given
                    .range
                    .as_ref()
                    .is_some_and(|given| interval::is_within(given, range))
            } else {
                given
                    .list
                    .iter()
                    .all(|v| interval::is_within(&point(*v), range))
            }
        } else {
            true
        };
        self.holds(
            admitted,
            "integer constraint",
            || shown(given),
            || shown(archetype),
        )
    }

    fn real(&self, archetype: &CReal, given: &CReal) -> Result<(), WriteError> {
        // NOTE: AOM2 master04.5 §Conformance semantics: C_ORDERED: each value or range the node
        // allows lies in a value or range of `other`.
        let shown = |c: &CReal| {
            if !c.list.is_empty() {
                list_text(c.list.iter())
            } else if let Some(range) = &c.range {
                Bounds::of_real(range).to_string()
            } else {
                "any".to_owned()
            }
        };
        let same = |a: f64, b: f64| a.total_cmp(&b).is_eq();
        let admitted = if !archetype.list.is_empty() {
            if given.list.is_empty() {
                given.range.as_ref().is_some_and(|range| {
                    let bounds = Bounds::of_real(range);
                    match bounds.point() {
                        Some(v) => archetype.list.iter().any(|a| same(*a, v)),
                        None => false,
                    }
                })
            } else {
                given
                    .list
                    .iter()
                    .all(|v| archetype.list.iter().any(|a| same(*a, *v)))
            }
        } else if let Some(range) = &archetype.range {
            let outer = Bounds::of_real(range);
            if given.list.is_empty() {
                given
                    .range
                    .as_ref()
                    .is_some_and(|given| Bounds::of_real(given).is_within(&outer))
            } else {
                given
                    .list
                    .iter()
                    .all(|v| Bounds::point_of(*v).is_within(&outer))
            }
        } else {
            true
        };
        self.holds(
            admitted,
            "real constraint",
            || shown(given),
            || shown(archetype),
        )
    }

    fn date(&self, archetype: &CDate, given: &CDate) -> Result<(), WriteError> {
        self.temporal_pattern(
            "date pattern",
            archetype.pattern.as_deref(),
            given.pattern.as_deref(),
        )?;
        self.timezone(archetype.timezone_validity, given.timezone_validity)?;
        self.temporal_range(
            "date range",
            archetype.range.as_ref().map(|r| Bounds::of_text(r, date)),
            given.range.as_ref().map(|r| Bounds::of_text(r, date)),
        )
    }

    fn date_time(&self, archetype: &CDateTime, given: &CDateTime) -> Result<(), WriteError> {
        self.temporal_pattern(
            "date/time pattern",
            archetype.pattern.as_deref(),
            given.pattern.as_deref(),
        )?;
        self.timezone(archetype.timezone_validity, given.timezone_validity)?;
        self.temporal_range(
            "date/time range",
            archetype
                .range
                .as_ref()
                .map(|r| Bounds::of_text(r, date_time)),
            given.range.as_ref().map(|r| Bounds::of_text(r, date_time)),
        )
    }

    fn time(&self, archetype: &CTime, given: &CTime) -> Result<(), WriteError> {
        self.temporal_pattern(
            "time pattern",
            archetype.pattern.as_deref(),
            given.pattern.as_deref(),
        )?;
        self.timezone(archetype.timezone_validity, given.timezone_validity)?;
        self.temporal_range(
            "time range",
            archetype.range.as_ref().map(|r| Bounds::of_text(r, time)),
            given.range.as_ref().map(|r| Bounds::of_text(r, time)),
        )
    }

    fn duration(&self, archetype: &CDuration, given: &CDuration) -> Result<(), WriteError> {
        if let Some(pattern) = &archetype.pattern {
            // NOTE: AOM2 c_temporal_definitions `valid_duration_constraint_replacement`: the
            // parent's pattern contains every element of the child's, date and time parts apart.
            let admitted = given
                .pattern
                .as_deref()
                .is_some_and(|given| duration_pattern_within(given, pattern));
            self.holds(
                admitted,
                "duration pattern",
                || given.pattern.clone().unwrap_or_else(|| "any".to_owned()),
                || pattern.clone(),
            )?;
        }
        self.temporal_range(
            "duration range",
            archetype
                .range
                .as_ref()
                .map(|r| Bounds::of_text(r, duration)),
            given.range.as_ref().map(|r| Bounds::of_text(r, duration)),
        )
    }

    fn temporal_pattern(
        &self,
        what: &'static str,
        archetype: Option<&str>,
        given: Option<&str>,
    ) -> Result<(), WriteError> {
        let Some(archetype) = archetype else {
            return Ok(());
        };
        // NOTE: AOM2 c_temporal_definitions `valid_*_constraint_replacements`: a `??` slot of the
        // parent's pattern becomes required or `XX` in the child; every other slot stays.
        self.holds(
            given.is_some_and(|given| temporal_pattern_within(given, archetype)),
            what,
            || given.unwrap_or("any").to_owned(),
            || archetype.to_owned(),
        )
    }

    fn timezone(
        &self,
        archetype: Option<ValidityKind>,
        given: Option<ValidityKind>,
    ) -> Result<(), WriteError> {
        let admitted = match archetype {
            None | Some(ValidityKind::Optional) => true,
            Some(kind) => given == Some(kind),
        };
        let shown = |v: Option<ValidityKind>| match v {
            None | Some(ValidityKind::Optional) => "optional",
            Some(ValidityKind::Mandatory) => "mandatory",
            Some(ValidityKind::Disallowed) => "disallowed",
        };
        self.holds(
            admitted,
            "timezone validity",
            || shown(given).to_owned(),
            || shown(archetype).to_owned(),
        )
    }

    fn temporal_range<T: PartialOrd + Lexical>(
        &self,
        what: &'static str,
        archetype: Option<Bounds<T>>,
        given: Option<Bounds<T>>,
    ) -> Result<(), WriteError> {
        let Some(archetype) = archetype else {
            return Ok(());
        };
        let admitted = given
            .as_ref()
            .is_some_and(|given| given.is_within(&archetype));
        self.holds(
            admitted,
            what,
            || given.map_or_else(|| "any".to_owned(), |given| given.to_string()),
            || archetype.to_string(),
        )
    }

    fn code_phrase(&self, archetype: &CCodePhrase, given: &CCodePhrase) -> Result<(), WriteError> {
        // NOTE: ADL2 master09.05 §Terminology Constraint Redefinition: "redefinitions may only
        // narrow constraints", so the codes lie in the parent's terminology and value set.
        if let Some(terminology) = &archetype.terminology_id {
            let same = given
                .terminology_id
                .as_ref()
                .is_some_and(|given| given.value == terminology.value);
            self.holds(
                same,
                "terminology",
                || {
                    given
                        .terminology_id
                        .as_ref()
                        .map_or_else(|| "any".to_owned(), |t| t.value.clone())
                },
                || terminology.value.clone(),
            )?;
        }
        if archetype.code_list.is_empty() {
            return Ok(());
        }
        self.holds(
            !given.code_list.is_empty()
                && given
                    .code_list
                    .iter()
                    .all(|code| archetype.code_list.contains(code)),
            "code list",
            || codes_text(&given.code_list),
            || codes_text(&archetype.code_list),
        )
    }

    fn quantity(&self, archetype: &CDvQuantity, given: &CDvQuantity) -> Result<(), WriteError> {
        // NOTE: ADL2 master09.05 §Numeric Primitive Redefinition: a redefined quantity keeps the
        // parent's property and units, and narrows each magnitude and precision.
        if let Some(property) = &archetype.property {
            self.holds(
                given
                    .property
                    .as_ref()
                    .is_some_and(|given| same_code(given, property)),
                "property",
                || {
                    given
                        .property
                        .as_ref()
                        .map_or_else(|| "any".to_owned(), code_text)
                },
                || code_text(property),
            )?;
        }
        if archetype.list.is_empty() {
            return Ok(());
        }
        let units = |q: &CDvQuantity| {
            codes_text(&q.list.iter().map(|i| i.units.clone()).collect::<Vec<_>>())
        };
        if given.list.is_empty() {
            return Err(self.widening("units", "any".to_owned(), units(archetype)));
        }
        for item in &given.list {
            let Some(parent) = archetype.list.iter().find(|a| a.units == item.units) else {
                return Err(self.widening("units", item.units.clone(), units(archetype)));
            };
            if let Some(magnitude) = &parent.magnitude {
                let outer = Bounds::of_real(magnitude);
                self.holds(
                    item.magnitude
                        .as_ref()
                        .is_some_and(|given| Bounds::of_real(given).is_within(&outer)),
                    "magnitude",
                    || {
                        item.magnitude
                            .as_ref()
                            .map_or_else(|| "any".to_owned(), |m| Bounds::of_real(m).to_string())
                    },
                    || outer.to_string(),
                )?;
            }
            if let Some(precision) = &parent.precision {
                self.holds(
                    item.precision
                        .as_ref()
                        .is_some_and(|given| interval::is_within(given, precision)),
                    "precision",
                    || {
                        item.precision
                            .as_ref()
                            .map_or_else(|| "any".to_owned(), interval::display)
                    },
                    || interval::display(precision),
                )?;
            }
        }
        Ok(())
    }

    fn ordinal(&self, archetype: &CDvOrdinal, given: &CDvOrdinal) -> Result<(), WriteError> {
        // NOTE: ADL2 master09.05 §Terminology Constraint Redefinition: "redefinitions may only
        // narrow constraints", so each ordinal is one of the parent's, value and symbol alike.
        if archetype.list.is_empty() {
            return Ok(());
        }
        let shown = |o: &CDvOrdinal| {
            format!(
                "{{{}}}",
                o.list
                    .iter()
                    .map(|o| format!(
                        "{}|{}|",
                        o.value,
                        code_text(&base_code(&o.symbol.defining_code))
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        self.holds(
            !given.list.is_empty()
                && given.list.iter().all(|o| {
                    archetype.list.iter().any(|a| {
                        a.value == o.value
                            && same_code(
                                &base_code(&a.symbol.defining_code),
                                &base_code(&o.symbol.defining_code),
                            )
                    })
                }),
            "ordinal list",
            || shown(given),
            || shown(archetype),
        )
    }
}

/// The strings a `C_STRING` admits, `None` for any string.
enum StringSet<'a> {
    List(&'a [String]),
    Pattern(&'a str),
}

impl<'a> StringSet<'a> {
    fn of(c: &'a CString) -> Option<Self> {
        if c.list.is_empty() {
            c.pattern.as_deref().map(Self::Pattern)
        } else {
            (c.list_open != Some(true)).then_some(Self::List(&c.list))
        }
    }
}

impl std::fmt::Display for StringSet<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::List(list) => f.write_str(&codes_text(list)),
            Self::Pattern(pattern) => write!(f, "/{pattern}/"),
        }
    }
}

/// An interval over an ordered type: each side its value and whether it is
/// included, `None` for an unbounded side.
struct Bounds<T> {
    lower: Option<(T, bool)>,
    upper: Option<(T, bool)>,
}

impl Bounds<f64> {
    fn of_real(interval: &Intervalofreal) -> Self {
        Self {
            lower: interval
                .lower
                .filter(|_| !interval.lower_unbounded)
                .map(|v| (v, interval.lower_included.unwrap_or(true))),
            upper: interval
                .upper
                .filter(|_| !interval.upper_unbounded)
                .map(|v| (v, interval.upper_included.unwrap_or(true))),
        }
    }

    const fn point_of(v: f64) -> Self {
        Self {
            lower: Some((v, true)),
            upper: Some((v, true)),
        }
    }

    fn point(&self) -> Option<f64> {
        match (self.lower, self.upper) {
            (Some((lower, true)), Some((upper, true))) if lower.total_cmp(&upper).is_eq() => {
                Some(lower)
            }
            _ => None,
        }
    }
}

/// The text intervals of the temporal constraints, as `Template.xsd` gives
/// them.
trait TextInterval {
    fn sides(&self) -> (Option<&str>, bool, bool, Option<&str>, bool, bool);
}

macro_rules! text_interval {
    ($($t:ty),*) => {$(
        impl TextInterval for $t {
            fn sides(&self) -> (Option<&str>, bool, bool, Option<&str>, bool, bool) {
                (
                    self.lower.as_deref(),
                    self.lower_unbounded,
                    self.lower_included.unwrap_or(true),
                    self.upper.as_deref(),
                    self.upper_unbounded,
                    self.upper_included.unwrap_or(true),
                )
            }
        }
    )*};
}

text_interval!(
    crate::opt14::types::Intervalofdate,
    crate::opt14::types::Intervalofdatetime,
    crate::opt14::types::Intervaloftime,
    crate::opt14::types::Intervalofduration
);

fn date(value: &str) -> Iso8601Date {
    Iso8601Date {
        value: value.to_owned(),
    }
}

fn date_time(value: &str) -> Iso8601DateTime {
    Iso8601DateTime {
        value: value.to_owned(),
    }
}

fn time(value: &str) -> Iso8601Time {
    Iso8601Time {
        value: value.to_owned(),
    }
}

fn duration(value: &str) -> Iso8601Duration {
    Iso8601Duration {
        value: value.to_owned(),
    }
}

impl<T> Bounds<T> {
    fn of_text(interval: &impl TextInterval, make: fn(&str) -> T) -> Self {
        let (lower, lower_unbounded, lower_included, upper, upper_unbounded, upper_included) =
            interval.sides();
        Self {
            lower: lower
                .filter(|_| !lower_unbounded)
                .map(|v| (make(v), lower_included)),
            upper: upper
                .filter(|_| !upper_unbounded)
                .map(|v| (make(v), upper_included)),
        }
    }
}

impl<T: PartialOrd> Bounds<T> {
    /// Returns whether every value of `self` lies in `outer`; bounds that do
    /// not compare (partial temporal values) do not show it.
    fn is_within(&self, outer: &Self) -> bool {
        let side = |inner: &Option<(T, bool)>, outer: &Option<(T, bool)>, inward: Ordering| match (
            inner, outer,
        ) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some((i, i_in)), Some((o, o_in))) => match i.partial_cmp(o) {
                Some(Ordering::Equal) => *o_in || !*i_in,
                Some(order) => order == inward,
                None => false,
            },
        };
        side(&self.lower, &outer.lower, Ordering::Greater)
            && side(&self.upper, &outer.upper, Ordering::Less)
    }
}

impl<T: Lexical> std::fmt::Display for Bounds<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let side = |s: &Option<(T, bool)>, open: &str| match s {
            None => "*".to_owned(),
            Some((v, true)) => v.lexical(),
            Some((v, false)) => format!("{open}{}", v.lexical()),
        };
        write!(
            f,
            "|{}..{}|",
            side(&self.lower, ">"),
            side(&self.upper, "<")
        )
    }
}

fn temporal_pattern_within(given: &str, archetype: &str) -> bool {
    let given = given.to_ascii_uppercase();
    let archetype = archetype.to_ascii_uppercase();
    given.len() == archetype.len()
        && given
            .chars()
            .zip(archetype.chars())
            .all(|(g, a)| g == a || (a == '?' && (g == 'X' || g.is_ascii_alphabetic())))
}

fn duration_pattern_within(given: &str, archetype: &str) -> bool {
    let parts = |p: &str| {
        let p = p.to_ascii_uppercase();
        let (date, time) = p.split_once('T').unwrap_or((p.as_str(), ""));
        (date.to_owned(), time.to_owned())
    };
    let (given_date, given_time) = parts(given);
    let (archetype_date, archetype_time) = parts(archetype);
    given_date.chars().all(|c| archetype_date.contains(c))
        && given_time.chars().all(|c| archetype_time.contains(c))
}

/// The written form of a value in a diagnostic.
trait Lexical {
    fn lexical(&self) -> String;
}

impl Lexical for f64 {
    fn lexical(&self) -> String {
        self.to_string()
    }
}

macro_rules! lexical_value {
    ($($t:ty),*) => {$(
        impl Lexical for $t {
            fn lexical(&self) -> String {
                self.value.clone()
            }
        }
    )*};
}

lexical_value!(Iso8601Date, Iso8601DateTime, Iso8601Time, Iso8601Duration);

/// An ordinal symbol's code as the property's code is typed.
fn base_code(code: &openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase) -> CodePhrase {
    CodePhrase {
        terminology_id:
            openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId {
                value: code.terminology_id.value.clone(),
            },
        code_string: code.code_string.clone(),
        preferred_term: None,
    }
}

fn same_code(a: &CodePhrase, b: &CodePhrase) -> bool {
    a.terminology_id.value == b.terminology_id.value && a.code_string == b.code_string
}

fn code_text(code: &CodePhrase) -> String {
    format!("[{}::{}]", code.terminology_id.value, code.code_string)
}

fn codes_text(codes: &[String]) -> String {
    list_text(codes.iter().map(|c| format!("{c:?}")))
}

fn list_text<T: std::fmt::Display>(values: impl Iterator<Item = T>) -> String {
    format!(
        "{{{}}}",
        values.map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::narrows;
    use crate::opt14::types::CObject;
    use crate::opt14_writer::WriteError;
    use crate::opt14_writer::redefine::Site;

    const ARCHETYPE: &str = "openEHR-EHR-CLUSTER.leaf.v1";
    const PATH: &str = "/items[at0001]/value";

    const ONE: &str = "<occurrences><lower_included>true</lower_included>\
        <upper_included>true</upper_included><lower_unbounded>false</lower_unbounded>\
        <upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></occurrences>";

    /// A leaf object of `kind` on `rm_type` with the constraint `body`.
    fn leaf(kind: &str, rm_type: &str, body: &str) -> CObject {
        crate::xml::runtime::from_xml(&format!(
            "<children xmlns=\"http://schemas.openehr.org/v1\" \
             xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"{kind}\">\
             <rm_type_name>{rm_type}</rm_type_name>{ONE}<node_id/>{body}</children>"
        ))
        .unwrap_or_else(|e| panic!("a well-formed {kind}: {e}"))
    }

    /// A primitive object on `rm_type` whose item is the `kind` constraint `body`.
    fn primitive(rm_type: &str, kind: &str, body: &str) -> CObject {
        leaf(
            "C_PRIMITIVE_OBJECT",
            rm_type,
            &format!("<item xsi:type=\"{kind}\">{body}</item>"),
        )
    }

    /// A closed interval element `tag` from `lower` to `upper`.
    fn range(tag: &str, lower: &str, upper: &str) -> String {
        format!(
            "<{tag}><lower_included>true</lower_included><upper_included>true</upper_included>\
             <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
             <lower>{lower}</lower><upper>{upper}</upper></{tag}>"
        )
    }

    fn list(values: &[&str]) -> String {
        values
            .iter()
            .map(|v| format!("<list>{v}</list>"))
            .collect::<Vec<_>>()
            .concat()
    }

    fn check(archetype: &CObject, given: &CObject) -> Result<(), WriteError> {
        narrows(
            archetype,
            given,
            &Site {
                archetype_id: ARCHETYPE,
                path: PATH,
            },
        )
    }

    /// Asserts that `given` over `archetype` is refused as a widening of `what`.
    #[track_caller]
    fn refused(archetype: &CObject, given: &CObject, what: &str) {
        match check(archetype, given) {
            Err(WriteError::Widening {
                archetype_id,
                path,
                what: refused,
                ..
            }) => {
                assert_eq!(refused, what);
                assert_eq!(archetype_id, ARCHETYPE);
                assert_eq!(path, PATH);
            }
            other => panic!("expected a widening of the {what}, got {other:?}"),
        }
    }

    #[test]
    fn a_string_list_narrows_to_a_subset_or_a_matching_list() {
        let archetype = primitive("STRING", "C_STRING", &list(&["a", "b"]));
        assert!(check(&archetype, &primitive("STRING", "C_STRING", &list(&["a"]))).is_ok());
        refused(
            &archetype,
            &primitive("STRING", "C_STRING", &list(&["a", "c"])),
            "string constraint",
        );
        refused(
            &archetype,
            &primitive("STRING", "C_STRING", "<pattern>[ab]</pattern>"),
            "string constraint",
        );
        refused(
            &archetype,
            &leaf("C_PRIMITIVE_OBJECT", "STRING", ""),
            "leaf constraint",
        );
        let open = primitive(
            "STRING",
            "C_STRING",
            &format!("{}<list_open>true</list_open>", list(&["a"])),
        );
        assert!(check(&open, &primitive("STRING", "C_STRING", &list(&["z"]))).is_ok());
        let pattern = primitive("STRING", "C_STRING", "<pattern>[a-z]+</pattern>");
        assert!(check(&pattern, &primitive("STRING", "C_STRING", &list(&["abc"]))).is_ok());
        refused(
            &pattern,
            &primitive("STRING", "C_STRING", &list(&["ABC"])),
            "string constraint",
        );
    }

    #[test]
    fn an_integer_constraint_narrows_within_its_range_or_list() {
        let archetype = primitive("INTEGER", "C_INTEGER", &range("range", "0", "10"));
        let given = |body: &str| primitive("INTEGER", "C_INTEGER", body);
        assert!(check(&archetype, &given(&range("range", "2", "5"))).is_ok());
        assert!(check(&archetype, &given(&list(&["3", "10"]))).is_ok());
        refused(
            &archetype,
            &given(&range("range", "0", "20")),
            "integer constraint",
        );
        refused(&archetype, &given(&list(&["11"])), "integer constraint");
        let listed = given(&list(&["1", "2", "3"]));
        assert!(check(&listed, &given(&range("range", "1", "2"))).is_ok());
        refused(
            &listed,
            &given(&range("range", "1", "4")),
            "integer constraint",
        );
    }

    #[test]
    fn a_real_range_narrows_within_its_bounds() {
        let archetype = primitive("REAL", "C_REAL", &range("range", "0.0", "10.0"));
        let given = |body: &str| primitive("REAL", "C_REAL", body);
        assert!(check(&archetype, &given(&range("range", "1.0", "2.5"))).is_ok());
        assert!(check(&archetype, &given(&list(&["10.0"]))).is_ok());
        refused(
            &archetype,
            &given(&range("range", "0.0", "10.5")),
            "real constraint",
        );
        let exclusive = given(
            "<range><lower_included>true</lower_included><upper_included>false</upper_included>\
             <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
             <lower>0.0</lower><upper>10.0</upper></range>",
        );
        refused(
            &exclusive,
            &given(&range("range", "0.0", "10.0")),
            "real constraint",
        );
    }

    #[test]
    fn a_boolean_narrows_to_fewer_values() {
        let archetype = primitive(
            "BOOLEAN",
            "C_BOOLEAN",
            "<true_valid>true</true_valid><false_valid>false</false_valid>",
        );
        let both = primitive(
            "BOOLEAN",
            "C_BOOLEAN",
            "<true_valid>true</true_valid><false_valid>true</false_valid>",
        );
        assert!(check(&archetype, &archetype).is_ok());
        assert!(check(&both, &archetype).is_ok());
        refused(&archetype, &both, "boolean values");
    }

    #[test]
    fn a_date_narrows_its_range_and_its_pattern() {
        let archetype = primitive(
            "DATE",
            "C_DATE",
            &format!(
                "<pattern>yyyy-mm-??</pattern>{}",
                range("range", "2000-01-01", "2020-12-31")
            ),
        );
        let given = |pattern: &str, lower: &str, upper: &str| {
            primitive(
                "DATE",
                "C_DATE",
                &format!(
                    "<pattern>{pattern}</pattern>{}",
                    range("range", lower, upper)
                ),
            )
        };
        assert!(check(&archetype, &given("yyyy-mm-dd", "2010-01-01", "2011-01-01")).is_ok());
        refused(
            &archetype,
            &given("yyyy-mm-dd", "1990-01-01", "2011-01-01"),
            "date range",
        );
        refused(
            &archetype,
            &given("yyyy-??-??", "2010-01-01", "2011-01-01"),
            "date pattern",
        );
    }

    #[test]
    fn a_date_time_narrows_its_range_and_its_timezone() {
        let archetype = primitive(
            "DATE_TIME",
            "C_DATE_TIME",
            &format!(
                "<timezone_validity>1001</timezone_validity>{}",
                range("range", "2000-01-01T00:00:00Z", "2020-12-31T23:59:59Z")
            ),
        );
        let given = |zone: &str, upper: &str| {
            primitive(
                "DATE_TIME",
                "C_DATE_TIME",
                &format!("{zone}{}", range("range", "2010-01-01T00:00:00Z", upper)),
            )
        };
        let mandatory = "<timezone_validity>1001</timezone_validity>";
        assert!(check(&archetype, &given(mandatory, "2011-01-01T00:00:00Z")).is_ok());
        refused(
            &archetype,
            &given(mandatory, "2030-01-01T00:00:00Z"),
            "date/time range",
        );
        refused(
            &archetype,
            &given(
                "<timezone_validity>1002</timezone_validity>",
                "2011-01-01T00:00:00Z",
            ),
            "timezone validity",
        );
    }

    #[test]
    fn a_time_narrows_its_pattern() {
        let archetype = primitive("TIME", "C_TIME", "<pattern>HH:MM:??</pattern>");
        assert!(
            check(
                &archetype,
                &primitive("TIME", "C_TIME", "<pattern>HH:MM:XX</pattern>")
            )
            .is_ok()
        );
        refused(
            &archetype,
            &primitive("TIME", "C_TIME", "<pattern>HH:??:??</pattern>"),
            "time pattern",
        );
        refused(&archetype, &primitive("TIME", "C_TIME", ""), "time pattern");
    }

    #[test]
    fn a_duration_narrows_its_pattern_and_its_range() {
        let archetype = primitive(
            "DURATION",
            "C_DURATION",
            &format!("<pattern>PYMWD</pattern>{}", range("range", "P0D", "P30D")),
        );
        let given = |pattern: &str, upper: &str| {
            primitive(
                "DURATION",
                "C_DURATION",
                &format!(
                    "<pattern>{pattern}</pattern>{}",
                    range("range", "P0D", upper)
                ),
            )
        };
        assert!(check(&archetype, &given("PYD", "P7D")).is_ok());
        refused(&archetype, &given("PYDTH", "P7D"), "duration pattern");
        refused(&archetype, &given("PD", "P60D"), "duration range");
    }

    fn code_phrase(terminology: &str, codes: &[&str]) -> CObject {
        let codes = codes
            .iter()
            .map(|c| format!("<code_list>{c}</code_list>"))
            .collect::<Vec<_>>()
            .concat();
        leaf(
            "C_CODE_PHRASE",
            "CODE_PHRASE",
            &format!("<terminology_id><value>{terminology}</value></terminology_id>{codes}"),
        )
    }

    #[test]
    fn a_code_phrase_narrows_to_codes_of_the_archetype_list() {
        let archetype = code_phrase("local", &["at1", "at2"]);
        assert!(check(&archetype, &code_phrase("local", &["at2"])).is_ok());
        refused(
            &archetype,
            &code_phrase("local", &["at1", "at3"]),
            "code list",
        );
        refused(&archetype, &code_phrase("local", &[]), "code list");
        refused(
            &archetype,
            &code_phrase("SNOMED-CT", &["at1"]),
            "terminology",
        );
        let any_local = code_phrase("local", &[]);
        assert!(check(&any_local, &code_phrase("local", &["at9"])).is_ok());
    }

    fn quantity(property: &str, items: &[(&str, &str, &str)]) -> CObject {
        let items = items
            .iter()
            .map(|(units, lower, upper)| {
                format!(
                    "<list>{}<units>{units}</units></list>",
                    range("magnitude", lower, upper)
                )
            })
            .collect::<Vec<_>>()
            .concat();
        leaf(
            "C_DV_QUANTITY",
            "DV_QUANTITY",
            &format!(
                "<property><terminology_id><value>openehr</value></terminology_id>\
                 <code_string>{property}</code_string></property>{items}"
            ),
        )
    }

    #[test]
    fn a_quantity_narrows_its_units_and_magnitudes() {
        let archetype = quantity(
            "125",
            &[("mm[Hg]", "0.0", "1000.0"), ("kPa", "0.0", "100.0")],
        );
        assert!(check(&archetype, &quantity("125", &[("mm[Hg]", "40.0", "300.0")])).is_ok());
        refused(
            &archetype,
            &quantity("125", &[("mm[Hg]", "0.0", "1500.0")]),
            "magnitude",
        );
        refused(
            &archetype,
            &quantity("125", &[("cm[H2O]", "0.0", "10.0")]),
            "units",
        );
        refused(&archetype, &quantity("125", &[]), "units");
        refused(
            &archetype,
            &quantity("382", &[("mm[Hg]", "40.0", "300.0")]),
            "property",
        );
    }

    fn ordinal(values: &[(&str, &str)]) -> CObject {
        let items = values
            .iter()
            .map(|(value, code)| {
                format!(
                    "<list><value>{value}</value><symbol><value>{code}</value><defining_code>\
                     <terminology_id><value>local</value></terminology_id>\
                     <code_string>{code}</code_string></defining_code></symbol></list>"
                )
            })
            .collect::<Vec<_>>()
            .concat();
        leaf("C_DV_ORDINAL", "DV_ORDINAL", &items)
    }

    #[test]
    fn an_ordinal_narrows_to_ordinals_of_the_archetype_list() {
        let archetype = ordinal(&[("1", "at1"), ("2", "at2"), ("3", "at3")]);
        assert!(check(&archetype, &ordinal(&[("1", "at1"), ("3", "at3")])).is_ok());
        refused(
            &archetype,
            &ordinal(&[("1", "at1"), ("4", "at4")]),
            "ordinal list",
        );
        refused(&archetype, &ordinal(&[("2", "at1")]), "ordinal list");
        refused(&archetype, &ordinal(&[]), "ordinal list");
    }

    #[test]
    fn a_leaf_of_another_kind_is_refused_and_an_unconstrained_leaf_takes_any() {
        let complex = leaf("C_COMPLEX_OBJECT", "CODE_PHRASE", "");
        refused(&code_phrase("local", &["at1"]), &complex, "leaf constraint");
        refused(
            &primitive("STRING", "C_STRING", &list(&["a"])),
            &primitive("STRING", "C_INTEGER", &list(&["1"])),
            "leaf constraint",
        );
        let unconstrained = leaf("C_PRIMITIVE_OBJECT", "STRING", "");
        assert!(
            check(
                &unconstrained,
                &primitive("STRING", "C_STRING", &list(&["a"]))
            )
            .is_ok()
        );
    }
}
