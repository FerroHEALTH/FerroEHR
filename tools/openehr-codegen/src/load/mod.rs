// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 1, load: the vendored inputs as verbatim models: the BMM ([`bmm`]),
//! XSD ([`xsd`]), OAS ([`oas`]) and the hand-written siblings ([`impls`]).

pub(crate) mod bmm;
pub(crate) mod bmm_function;
pub(crate) mod impls;
pub(crate) mod oas;
pub(crate) mod xsd;
