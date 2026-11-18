// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

//! Reflective access to typed RM by attribute name (#6002).
//!
//! The Simplified Formats place and read values generically, by RM attribute
//! name, over every class a template reaches. [`RmObject`] is that view of an
//! RM value, implemented by every RM and BASE struct of the release model
//! (`access/generated.rs`, emitted by `openehr-codegen -- emit-validate`) and
//! by [`Partial`], the object a build fills before its mandatory attributes are
//! all written. [`construct`] starts one, [`finish`] turns a tree of them into
//! the real struct and refuses the first mandatory attribute still unset.
//!
//! The real structs are the RM model unchanged: a [`Partial`] holds one slot
//! per attribute in BMM declaration order, and its generated conversion moves
//! each slot into the struct's field. No openEHR specification governs this
//! access: our own design.

use std::any::Any;

use crate::v1_2::model;

mod generated;

/// A value written into an attribute.
#[derive(Debug)]
pub enum RmValue {
    /// An RM object: a real struct or a [`Partial`].
    Object(Box<dyn RmObject>),
    /// A `String`.
    String(String),
    /// An `Integer` or `Integer64`.
    Integer(i64),
    /// A `Real` or `Double`.
    Real(f64),
    /// A `Boolean`.
    Boolean(bool),
    /// A `Character`.
    Character(char),
}

impl RmValue {
    fn kind(&self) -> String {
        match self {
            Self::Object(o) => o.rm_type().to_owned(),
            Self::String(_) => "String".to_owned(),
            Self::Integer(_) => "Integer".to_owned(),
            Self::Real(_) => "Real".to_owned(),
            Self::Boolean(_) => "Boolean".to_owned(),
            Self::Character(_) => "Character".to_owned(),
        }
    }
}

/// A borrowed attribute value.
#[derive(Debug, Clone, Copy)]
pub enum RmRef<'a> {
    /// An RM object.
    Object(&'a dyn RmObject),
    /// A `String`.
    String(&'a str),
    /// An `Integer` or `Integer64`.
    Integer(i64),
    /// A `Real` or `Double`.
    Real(f64),
    /// A `Boolean`.
    Boolean(bool),
    /// A `Character`.
    Character(char),
}

/// What the release model declares about one attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeInfo {
    /// The declared RM type; for a container, the member type.
    pub declared_type: &'static str,
    /// A `List`, `Set` or `Array` attribute.
    pub multiple: bool,
    /// A `1..1` or `1..*` attribute.
    pub mandatory: bool,
}

/// A refused access.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccessError {
    /// The class declares no such attribute.
    #[error("{rm_type} declares no attribute `{attribute}`")]
    UndeclaredAttribute {
        /// The class.
        rm_type: &'static str,
        /// The attribute named.
        attribute: String,
    },
    /// The value is not of the attribute's declared type.
    #[error("`{rm_type}.{attribute}` is declared {declared} and was given {given}")]
    NotConforming {
        /// The class.
        rm_type: &'static str,
        /// The attribute.
        attribute: String,
        /// The declared type.
        declared: &'static str,
        /// What was given.
        given: String,
    },
    /// A single value for a container attribute.
    #[error("`{rm_type}.{attribute}` is a container")]
    NotSingle {
        /// The class.
        rm_type: &'static str,
        /// The attribute.
        attribute: String,
    },
    /// A member pushed onto a single-valued attribute.
    #[error("`{rm_type}.{attribute}` is single-valued")]
    NotMultiple {
        /// The class.
        rm_type: &'static str,
        /// The attribute.
        attribute: String,
    },
    /// An abstract class cannot be constructed.
    #[error("{0} is abstract")]
    Abstract(String),
    /// No RM class of the release model has this name.
    #[error("{0} is not an RM class")]
    UnknownType(String),
    /// A class whose values are only built through its validating constructor
    /// (an identifier with a released lexical form).
    #[error("{0} is built through its constructor, not attribute by attribute")]
    NotBuildable(&'static str),
    /// An attribute the reflective view does not carry (a map, or a free-form
    /// value).
    #[error("`{rm_type}.{attribute}` is not settable by name")]
    Unsupported {
        /// The class.
        rm_type: &'static str,
        /// The attribute.
        attribute: &'static str,
    },
    /// A mandatory attribute still unset at [`finish`].
    #[error("mandatory attribute `{attribute}` of {rm_type} is unset")]
    Unset {
        /// The class.
        rm_type: &'static str,
        /// The attribute.
        attribute: &'static str,
    },
}

/// The children of an object: attribute, list index, child.
pub type Children<'a> =
    Box<dyn Iterator<Item = (&'static str, Option<usize>, &'a dyn RmObject)> + 'a>;

/// The mutable children of an object: attribute, child.
pub type ChildrenMut<'a> = Box<dyn Iterator<Item = (&'static str, &'a mut dyn RmObject)> + 'a>;

/// The members of a container attribute.
pub type Items<'a> = Box<dyn Iterator<Item = &'a dyn RmObject> + 'a>;

/// The mutable members of a container attribute.
pub type ItemsMut<'a> = Box<dyn Iterator<Item = &'a mut dyn RmObject> + 'a>;

/// An RM value seen by attribute name.
pub trait RmObject: std::fmt::Debug + Send + Sync + 'static {
    /// The concrete class.
    fn rm_type(&self) -> &'static str;
    /// The value, to downcast.
    fn as_any(&self) -> &dyn Any;
    /// The value, to downcast mutably.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// The value, to downcast by value.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
    /// Whether `attr` holds a value (a present optional, a non-empty list, a
    /// mandatory attribute of a real struct).
    fn is_set(&self, attr: &str) -> bool;
    /// The single value of `attr`; `None` when undeclared, unset, or a
    /// container (read those with [`RmObject::items`]).
    fn attribute(&self, attr: &str) -> Option<RmRef<'_>>;
    /// The single object of `attr`, to change in place.
    fn object_mut(&mut self, attr: &str) -> Option<&mut dyn RmObject>;
    /// Write `value` into the single-valued `attr`.
    ///
    /// # Errors
    /// [`AccessError`] when `attr` is undeclared, a container, not settable,
    /// or `value` does not conform to its declared type.
    fn set(&mut self, attr: &str, value: RmValue) -> Result<(), AccessError>;
    /// Append `value` to the container `attr`.
    ///
    /// # Errors
    /// [`AccessError`] when `attr` is undeclared, single-valued, or `value`
    /// does not conform to its member type.
    fn push(&mut self, attr: &str, value: Box<dyn RmObject>) -> Result<(), AccessError>;
    /// The members of the container `attr`.
    fn items(&self, attr: &str) -> Option<Items<'_>>;
    /// The members of the container `attr`, to change in place.
    fn items_mut(&mut self, attr: &str) -> Option<ItemsMut<'_>>;
    /// Every set object-valued attribute in declaration order, a container's
    /// members in list order.
    fn children(&self) -> Children<'_>;
    /// A deep copy.
    fn clone_box(&self) -> Box<dyn RmObject>;
    /// [`RmObject::children`], to change in place.
    fn children_mut(&mut self) -> ChildrenMut<'_>;
}

/// The [`RmObject`] view of a value held in a field: the struct itself, the
/// payload of an enum, the content of a box.
pub trait AsRm {
    /// The view.
    fn as_rm(&self) -> &dyn RmObject;
    /// The view, mutably.
    fn as_rm_mut(&mut self) -> &mut dyn RmObject;
}

impl<T: AsRm + ?Sized> AsRm for Box<T> {
    fn as_rm(&self) -> &dyn RmObject {
        T::as_rm(self)
    }
    fn as_rm_mut(&mut self) -> &mut dyn RmObject {
        T::as_rm_mut(self)
    }
}

/// A field value made from an [`RmObject`]: the real struct it holds, a
/// [`Partial`] of its class finished, or the enum variant its class selects.
pub trait FromRm: Sized {
    /// Convert, naming `rm_type.attribute` in a refusal.
    ///
    /// # Errors
    /// [`AccessError`] when the object is of another class or a partial one
    /// leaves a mandatory attribute unset.
    fn from_rm(object: Box<dyn RmObject>, at: Site) -> Result<Self, AccessError>;
}

impl<T: FromRm> FromRm for Box<T> {
    fn from_rm(object: Box<dyn RmObject>, at: Site) -> Result<Self, AccessError> {
        T::from_rm(object, at).map(Box::new)
    }
}

/// Where a value goes, for the refusal that names it.
#[derive(Debug, Clone, Copy)]
pub struct Site {
    /// The class holding the attribute.
    pub rm_type: &'static str,
    /// The attribute.
    pub attribute: &'static str,
    /// Its declared type.
    pub declared: &'static str,
}

impl Site {
    fn refuse(self, given: &str) -> AccessError {
        AccessError::NotConforming {
            rm_type: self.rm_type,
            attribute: self.attribute.to_owned(),
            declared: self.declared,
            given: given.to_owned(),
        }
    }
}

/// The refusal of an attribute `rm_type` does not declare.
fn undeclared(rm_type: &'static str, attr: &str) -> AccessError {
    AccessError::UndeclaredAttribute {
        rm_type,
        attribute: attr.to_owned(),
    }
}

/// The site of `rm_type.attribute`, for a refusal.
fn site(rm_type: &'static str, attribute: &'static str) -> Site {
    Site {
        rm_type,
        attribute,
        declared: schema::attribute(rm_type, attribute).map_or("", |a| a.declared_type),
    }
}

/// One attribute of a [`Partial`].
#[derive(Debug, Default)]
enum Slot {
    #[default]
    Unset,
    Single(RmValue),
    Many(Vec<Box<dyn RmObject>>),
}

/// An RM object being built: one slot per attribute of its class, in
/// declaration order, unset until written.
#[derive(Debug)]
pub struct Partial {
    rm_type: &'static str,
    attributes: &'static [&'static str],
    slots: Vec<Slot>,
}

impl Partial {
    fn index(&self, attr: &str) -> Option<usize> {
        self.attributes.iter().position(|a| *a == attr)
    }

    fn info(&self, attr: &str) -> Result<(usize, AttributeInfo), AccessError> {
        let undeclared = || AccessError::UndeclaredAttribute {
            rm_type: self.rm_type,
            attribute: attr.to_owned(),
        };
        let i = self.index(attr).ok_or_else(undeclared)?;
        let info = schema::attribute(self.rm_type, attr).ok_or_else(undeclared)?;
        Ok((i, info))
    }

    fn take(&mut self, i: usize) -> Slot {
        self.slots
            .get_mut(i)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    fn unset(&self, attribute: &'static str) -> AccessError {
        AccessError::Unset {
            rm_type: self.rm_type,
            attribute,
        }
    }

    fn site(&self, attribute: &'static str) -> Site {
        site(self.rm_type, attribute)
    }

    /// The object of a mandatory single attribute.
    pub(crate) fn object<T: FromRm>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<T, AccessError> {
        self.opt_object(i, attribute)?
            .ok_or_else(|| self.unset(attribute))
    }

    /// The object of an optional single attribute.
    pub(crate) fn opt_object<T: FromRm>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<Option<T>, AccessError> {
        let site = self.site(attribute);
        match self.take(i) {
            Slot::Unset => Ok(None),
            Slot::Single(RmValue::Object(o)) => T::from_rm(o, site).map(Some),
            Slot::Single(other) => Err(site.refuse(&other.kind())),
            Slot::Many(_) => Err(site.refuse("a list")),
        }
    }

    /// The members of a container attribute, `None` when never written.
    pub(crate) fn objects<T: FromRm>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<Option<Vec<T>>, AccessError> {
        let site = self.site(attribute);
        match self.take(i) {
            Slot::Unset => Ok(None),
            Slot::Many(members) => members
                .into_iter()
                .map(|m| T::from_rm(m, site))
                .collect::<Result<Vec<T>, _>>()
                .map(Some),
            Slot::Single(other) => Err(site.refuse(&other.kind())),
        }
    }

    /// The members of a mandatory container attribute.
    pub(crate) fn vec<T: FromRm>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<Vec<T>, AccessError> {
        self.objects(i, attribute).map(Option::unwrap_or_default)
    }

    /// The members of a mandatory `1..*` container attribute.
    pub(crate) fn non_empty<T: FromRm>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<openehr_base::containers::NonEmptyVec<T>, AccessError> {
        let members = self.vec(i, attribute)?;
        match openehr_base::containers::NonEmptyVec::new(members) {
            Ok(members) => Ok(members),
            // NOTE: an empty `1..*` container is the mandatory attribute left unset.
            Err(_empty) => Err(self.unset(attribute)),
        }
    }

    /// The members of an optional `1..*` container attribute.
    pub(crate) fn opt_non_empty<T: FromRm>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<Option<openehr_base::containers::NonEmptyVec<T>>, AccessError> {
        Ok(self
            .objects(i, attribute)?
            .and_then(openehr_base::containers::present_nonempty))
    }

    /// The primitive of an optional single attribute.
    pub(crate) fn opt_primitive<T: FromPrimitive>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<Option<T>, AccessError> {
        let site = self.site(attribute);
        match self.take(i) {
            Slot::Unset => Ok(None),
            Slot::Single(value) => T::from_value(value, site).map(Some),
            Slot::Many(_) => Err(site.refuse("a list")),
        }
    }

    /// The primitive of a mandatory single attribute.
    pub(crate) fn primitive<T: FromPrimitive>(
        &mut self,
        i: usize,
        attribute: &'static str,
    ) -> Result<T, AccessError> {
        self.opt_primitive(i, attribute)?
            .ok_or_else(|| self.unset(attribute))
    }

    /// Whether this partial object is of class `rm_type`.
    #[must_use]
    pub fn is_of(&self, rm_type: &str) -> bool {
        self.rm_type == rm_type
    }
}

/// A primitive field made from an [`RmValue`].
pub trait FromPrimitive: Sized {
    /// Convert.
    ///
    /// # Errors
    /// [`AccessError::NotConforming`] for a value of another kind or range.
    fn from_value(value: RmValue, at: Site) -> Result<Self, AccessError>;
}

impl FromPrimitive for String {
    fn from_value(value: RmValue, at: Site) -> Result<Self, AccessError> {
        match value {
            RmValue::String(s) => Ok(s),
            other => Err(at.refuse(&other.kind())),
        }
    }
}

impl FromPrimitive for i64 {
    fn from_value(value: RmValue, at: Site) -> Result<Self, AccessError> {
        match value {
            RmValue::Integer(i) => Ok(i),
            other => Err(at.refuse(&other.kind())),
        }
    }
}

impl FromPrimitive for i32 {
    fn from_value(value: RmValue, at: Site) -> Result<Self, AccessError> {
        match value {
            RmValue::Integer(i) => {
                i32::try_from(i).map_err(|_out_of_range| at.refuse("an Integer out of range"))
            }
            other => Err(at.refuse(&other.kind())),
        }
    }
}

impl FromPrimitive for f64 {
    fn from_value(value: RmValue, at: Site) -> Result<Self, AccessError> {
        match value {
            RmValue::Real(r) => Ok(r),
            // NOTE: ITS-JSON reads an integer-written number into a `Real` (the canonical reader's `f64`).
            #[expect(
                clippy::as_conversions,
                clippy::cast_precision_loss,
                reason = "the canonical reader reads a JSON integer into f64 the same way (serde_json)"
            )]
            RmValue::Integer(i) => Ok(i as f64),
            other => Err(at.refuse(&other.kind())),
        }
    }
}

impl FromPrimitive for bool {
    fn from_value(value: RmValue, at: Site) -> Result<Self, AccessError> {
        match value {
            RmValue::Boolean(b) => Ok(b),
            other => Err(at.refuse(&other.kind())),
        }
    }
}

impl FromPrimitive for char {
    fn from_value(value: RmValue, at: Site) -> Result<Self, AccessError> {
        match value {
            RmValue::Character(c) => Ok(c),
            other => Err(at.refuse(&other.kind())),
        }
    }
}

impl RmObject for Partial {
    fn rm_type(&self) -> &'static str {
        self.rm_type
    }
    fn clone_box(&self) -> Box<dyn RmObject> {
        let slots = self
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Unset => Slot::Unset,
                Slot::Single(value) => Slot::Single(match value {
                    RmValue::Object(o) => RmValue::Object(o.clone_box()),
                    RmValue::String(s) => RmValue::String(s.clone()),
                    RmValue::Integer(i) => RmValue::Integer(*i),
                    RmValue::Real(r) => RmValue::Real(*r),
                    RmValue::Boolean(b) => RmValue::Boolean(*b),
                    RmValue::Character(c) => RmValue::Character(*c),
                }),
                Slot::Many(members) => Slot::Many(members.iter().map(|m| m.clone_box()).collect()),
            })
            .collect();
        Box::new(Partial {
            rm_type: self.rm_type,
            attributes: self.attributes,
            slots,
        })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
    fn is_set(&self, attr: &str) -> bool {
        match self.index(attr).and_then(|i| self.slots.get(i)) {
            Some(Slot::Single(_)) => true,
            Some(Slot::Many(m)) => !m.is_empty(),
            Some(Slot::Unset) | None => false,
        }
    }
    fn attribute(&self, attr: &str) -> Option<RmRef<'_>> {
        match self.index(attr).and_then(|i| self.slots.get(i))? {
            Slot::Single(value) => Some(match value {
                RmValue::Object(o) => RmRef::Object(o.as_ref()),
                RmValue::String(s) => RmRef::String(s),
                RmValue::Integer(i) => RmRef::Integer(*i),
                RmValue::Real(r) => RmRef::Real(*r),
                RmValue::Boolean(b) => RmRef::Boolean(*b),
                RmValue::Character(c) => RmRef::Character(*c),
            }),
            Slot::Many(_) | Slot::Unset => None,
        }
    }
    fn object_mut(&mut self, attr: &str) -> Option<&mut dyn RmObject> {
        let i = self.index(attr)?;
        match self.slots.get_mut(i)? {
            Slot::Single(RmValue::Object(o)) => Some(o.as_mut()),
            _ => None,
        }
    }
    fn set(&mut self, attr: &str, value: RmValue) -> Result<(), AccessError> {
        let (i, info) = self.info(attr)?;
        if info.multiple {
            return Err(AccessError::NotSingle {
                rm_type: self.rm_type,
                attribute: attr.to_owned(),
            });
        }
        if let RmValue::Object(o) = &value
            && !model::is_a(o.rm_type(), info.declared_type)
        {
            return Err(AccessError::NotConforming {
                rm_type: self.rm_type,
                attribute: attr.to_owned(),
                declared: info.declared_type,
                given: o.rm_type().to_owned(),
            });
        }
        if let Some(slot) = self.slots.get_mut(i) {
            *slot = Slot::Single(value);
        }
        Ok(())
    }
    fn push(&mut self, attr: &str, value: Box<dyn RmObject>) -> Result<(), AccessError> {
        let (i, info) = self.info(attr)?;
        if !info.multiple {
            return Err(AccessError::NotMultiple {
                rm_type: self.rm_type,
                attribute: attr.to_owned(),
            });
        }
        if !model::is_a(value.rm_type(), info.declared_type) {
            return Err(AccessError::NotConforming {
                rm_type: self.rm_type,
                attribute: attr.to_owned(),
                declared: info.declared_type,
                given: value.rm_type().to_owned(),
            });
        }
        if let Some(slot) = self.slots.get_mut(i) {
            match slot {
                Slot::Many(members) => members.push(value),
                other => *other = Slot::Many(vec![value]),
            }
        }
        Ok(())
    }
    fn items(&self, attr: &str) -> Option<Items<'_>> {
        match self.index(attr).and_then(|i| self.slots.get(i))? {
            Slot::Many(members) => Some(Box::new(members.iter().map(|m| &**m))),
            _ => None,
        }
    }
    fn items_mut(&mut self, attr: &str) -> Option<ItemsMut<'_>> {
        let i = self.index(attr)?;
        match self.slots.get_mut(i)? {
            Slot::Many(members) => Some(Box::new(members.iter_mut().map(|m| &mut **m))),
            _ => None,
        }
    }
    fn children(&self) -> Children<'_> {
        let mut out: Vec<(&'static str, Option<usize>, &dyn RmObject)> = Vec::new();
        for (name, slot) in self.attributes.iter().zip(&self.slots) {
            match slot {
                Slot::Single(RmValue::Object(o)) => out.push((name, None, o.as_ref())),
                Slot::Many(members) => {
                    for (i, m) in members.iter().enumerate() {
                        out.push((name, Some(i), m.as_ref()));
                    }
                }
                _ => {}
            }
        }
        Box::new(out.into_iter())
    }
    fn children_mut(&mut self) -> ChildrenMut<'_> {
        let mut out: Vec<(&'static str, &mut dyn RmObject)> = Vec::new();
        for (name, slot) in self.attributes.iter().zip(self.slots.iter_mut()) {
            match slot {
                Slot::Single(RmValue::Object(o)) => out.push((name, o.as_mut())),
                Slot::Many(members) => {
                    for m in members.iter_mut() {
                        out.push((name, m.as_mut()));
                    }
                }
                _ => {}
            }
        }
        Box::new(out.into_iter())
    }
}

/// Start an object of the concrete class `rm_type`, every attribute unset.
///
/// # Errors
/// [`AccessError::UnknownType`] for a name the release model does not have,
/// [`AccessError::Abstract`] for an abstract class.
pub fn construct(rm_type: &str) -> Result<Box<dyn RmObject>, AccessError> {
    let Some(class) = model::class(rm_type) else {
        return Err(AccessError::UnknownType(rm_type.to_owned()));
    };
    let Some((name, attributes)) = generated::attributes(rm_type) else {
        return Err(if class.is_abstract {
            AccessError::Abstract(rm_type.to_owned())
        } else {
            AccessError::UnknownType(rm_type.to_owned())
        });
    };
    let mut slots = Vec::with_capacity(attributes.len());
    slots.resize_with(attributes.len(), Slot::default);
    Ok(Box::new(Partial {
        rm_type: name,
        attributes,
        slots,
    }))
}

/// Turn a built tree into the real `T`, refusing the first mandatory
/// attribute still unset.
///
/// # Errors
/// [`AccessError::Unset`] naming the class and the attribute, or
/// [`AccessError::NotConforming`] when the root is not a `T`.
pub fn finish<T: FromRm>(root: Box<dyn RmObject>) -> Result<T, AccessError> {
    let rm_type = root.rm_type();
    T::from_rm(
        root,
        Site {
            rm_type,
            attribute: "",
            declared: rm_type,
        },
    )
}

/// The [`RmObject`] view of a typed RM value.
#[must_use]
pub fn as_rm_object<T: AsRm + ?Sized>(value: &T) -> &dyn RmObject {
    value.as_rm()
}

/// What the release model declares about attributes.
pub mod schema {
    use super::AttributeInfo;
    use crate::v1_2::model;

    /// The declaration of `attr` on `rm_type`, through inheritance.
    #[must_use]
    pub fn attribute(rm_type: &str, attr: &str) -> Option<AttributeInfo> {
        let a = model::attribute(rm_type, attr)?;
        // NOTE: ITS-JSON carries a `List<Octet>` (`DV_MULTIMEDIA.data`) as one base64 string.
        if a.declared_type == "Octet" {
            return Some(AttributeInfo {
                declared_type: "String",
                multiple: false,
                mandatory: a.is_mandatory,
            });
        }
        Some(AttributeInfo {
            declared_type: a.declared_type,
            multiple: !matches!(a.container, model::Container::None),
            mandatory: a.is_mandatory,
        })
    }

    /// The attributes of the concrete class `rm_type`, in declaration order;
    /// `None` for an abstract class or one built through its constructor.
    #[must_use]
    pub fn attribute_names(rm_type: &str) -> Option<&'static [&'static str]> {
        super::generated::attributes(rm_type).map(|(_, names)| names)
    }

    /// Whether `rm_type` is `ancestor` or one of its descendants.
    #[must_use]
    pub fn is_a(rm_type: &str, ancestor: &str) -> bool {
        model::is_a(rm_type, ancestor)
    }
}
