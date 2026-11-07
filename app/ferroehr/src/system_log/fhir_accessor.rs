// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The accessor of one access record (Regulation (EU) 2025/327 Annex II
//! 3.2(a), (b)), as `ehds-logging` writes it.

use ehds_logging::record::{Accessor, Acting, Coded, Professional, Purpose};

use crate::system_log::event::{ActingMode, AuditEvent};
use crate::system_log::fhir::{assurance, nonempty};

/// Who accessed the data (Annex II 3.2(a), (b)), as the authentication layer
/// judged them: a Basic user has no issuer and no client, and an acting mode
/// the deployment did not establish is none.
pub(super) fn accessor(event: &AuditEvent, missing: &str, purposes: Vec<Purpose>) -> Accessor {
    Accessor {
        issuer: event.issuer.clone(),
        subject: nonempty(&event.user_id, missing),
        client_id: event.client_id.clone(),
        audience: None,
        provider: event.organisation.clone(),
        // TODO(#5043): render the external and system modes the renderer cannot map yet.
        acting: event.actor.mode.and_then(|mode| match mode {
            ActingMode::Client | ActingMode::Delegated => Some(Acting::Client),
            ActingMode::Person => Some(Acting::Person),
            ActingMode::External | ActingMode::System => None,
        }),
        // NOTE: NEN 7513:2018 §5 (not vendored, NEN copyright) asks for the role or authority
        // under which the record was accessed, which BALP 1.1.4 §3:5.7.5.4 writes at role.
        roles: event
            .roles
            .iter()
            .map(|role| Coded {
                system: None,
                code: role.clone(),
            })
            .collect(),
        token_id: event.token_id.clone(),
        assurance: event.actor.assurance_level.map(assurance),
        professional: Professional {
            name: None,
            identifier: event.actor.professional_id.clone(),
        },
        alt_id: None,
        purposes,
        // NOTE: Regulation (EU) 2025/327 Art 68 names the data permit; FHIR R4 AuditEvent.agent.policy carries it.
        policies: event.permit.iter().cloned().collect(),
        relayed: None,
        // TODO(#5043): render the binding outcome, the actor chain and the representation.
        binding: None,
        actors: Vec::new(),
        represents: None,
    }
}
