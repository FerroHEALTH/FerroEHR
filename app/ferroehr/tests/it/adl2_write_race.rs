// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An ADL2 template upload racing the delete of the archetype it uses, against
//! a real `PostgreSQL` 18 (shared testkit harness).
//!
//! AOM2 VARXR (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules: `C_ARCHETYPE_ROOT`) requires every archetype reference to
//! resolve in the repository. No openEHR spec governs how the store orders two
//! racing writes: our own design. Whichever write comes second gets the
//! refusal it gets alone, so no stored template is left without its archetype.

use ferroehr::service::error::ServiceError;
use ferroehr::service::refusal::{Clash, Refusal};

/// How many times the two writes race.
const ROUNDS: usize = 24;

/// An ADL2 archetype `hrid` whose root matches any OBSERVATION.
fn archetype(hrid: &str) -> String {
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    OBSERVATION[id1] matches {{ *}}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n        >\n    >\n"
    )
}

/// An ADL2 template `hrid` filling its content with the archetype `filler`.
fn template_using(hrid: &str, filler: &str) -> String {
    format!(
        "template (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    COMPOSITION[id1] matches {{\n        \
         content cardinality matches {{0..*; unordered}} matches {{\n            \
         use_archetype OBSERVATION[id2, {filler}]\n        }}\n    }}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n            \
         [\"id2\"] = <text = <\"Filler\"> description = <\"Filler.\">>\n        >\n    >\n"
    )
}

/// The upload and the delete run at once, round after round. Either the
/// delete wins and the upload is refused with `VARXR`, as it is with the
/// archetype absent, or the upload wins and the delete is refused with a
/// conflict naming the template, as it is with the template stored. Never
/// both succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_template_upload_racing_the_delete_of_its_archetype_never_both_succeed() {
    let (db, svc) = crate::fixtures::service().await;
    let (mut deletes_won, mut uploads_won) = (0, 0);
    for round in 0..ROUNDS {
        let observation = format!("openEHR-EHR-OBSERVATION.race_obs_{round}.v1.0.0");
        let template = format!("openEHR-EHR-COMPOSITION.race_tpl_{round}.v1.0.0");
        svc.template_adl2_upload(&archetype(&observation))
            .await
            .expect("store the archetype");

        let delete = tokio::spawn({
            let svc = svc.clone();
            let observation = observation.clone();
            async move { svc.delete_artefact(&observation).await }
        });
        let upload = tokio::spawn({
            let svc = svc.clone();
            let source = template_using(&template, &observation);
            async move { svc.template_adl2_upload(&source).await }
        });
        let deleted = delete.await.expect("the delete task");
        let uploaded = upload.await.expect("the upload task");

        let template_stored = crate::fixtures::artefact_stored(&db.pool(), &template).await;
        let observation_stored = crate::fixtures::artefact_stored(&db.pool(), &observation).await;
        match (deleted, uploaded) {
            (Ok(()), Err(ServiceError::ValidationFailed(violations))) => {
                assert!(
                    violations.iter().any(|v| v.path == "VARXR"),
                    "round {round}: the upload after the delete is refused with VARXR: \
                     {violations:?}"
                );
                assert!(!template_stored && !observation_stored, "round {round}");
                deletes_won += 1;
            }
            (
                Err(ServiceError::Conflict(Refusal {
                    kind: Clash::State,
                    message,
                    ..
                })),
                Ok(_),
            ) => {
                assert!(message.contains(&template), "round {round}: {message}");
                assert!(template_stored && observation_stored, "round {round}");
                uploads_won += 1;
            }
            (deleted, uploaded) => panic!(
                "round {round}: delete {deleted:?}, upload {uploaded:?}, template stored \
                 {template_stored}, archetype stored {observation_stored}"
            ),
        }
    }
    assert_eq!(deletes_won + uploads_won, ROUNDS);
}
