// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 4, render: the text producers, the only stage that writes code.

pub(crate) mod bound_presence;
pub(crate) mod doc_prose;
pub(crate) mod emit;
pub(crate) mod emit_access;
pub(crate) mod emit_drafts;
pub(crate) mod emit_json;
pub(crate) mod emit_opt;
pub(crate) mod emit_released;
pub(crate) mod emit_rest;
pub(crate) mod emit_rest_headers;
pub(crate) mod emit_rm_model;
pub(crate) mod emit_rm_model_api;
pub(crate) mod emit_templates;
pub(crate) mod emit_validate;
pub(crate) mod emit_walk;
pub(crate) mod emit_xml;
pub(crate) mod emit_xml_guards;
pub(crate) mod generics;
pub(crate) mod json_bounds;
pub(crate) mod model_query;
pub(crate) mod model_types;
pub(crate) mod module_tree;
pub(crate) mod naming;
pub(crate) mod spdx;
