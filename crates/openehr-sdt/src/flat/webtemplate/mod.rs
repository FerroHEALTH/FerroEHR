// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Web Template model and builder.
//!
//! A Web Template is the processed representation of an operational template —
//! simplified node identifiers, AQL paths, input type definitions, localized
//! labels, and multiplicity constraints — defined by `ITS-REST
//! simplified_formats master04-basic_concepts.adoc` §"Web Template Metadata".
//! [`builder::build_web_template`] turns a parsed
//! [`openehr_its::opt14::types::OperationalTemplate`] into a [`model::WebTemplate`]. See
//! `builder` for the walk and the recorded scope boundaries, `id` for the
//! master04 §"Node ID Generation Rules" algorithm, and `inputs` for the
//! per-RM-type input mapping.

pub mod builder;
pub mod builder_v2_4;
mod id;
mod inputs;
pub mod model;
mod shape;

pub(crate) use inputs::PROPORTION_KINDS;
