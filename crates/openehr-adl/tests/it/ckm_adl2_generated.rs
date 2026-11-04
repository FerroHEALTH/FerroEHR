// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Gate over CKM's generated ADL 2 export (`corpus/archetypes/ckm/adl2-generated/`,
//! CKM Release 1.22.0, #4973).
//!
//! Every file is CKM's own 1.4->2 conversion, so this measures that conversion
//! and the CKM sources, never FerroEHR's converter. Each file parses or is the
//! adjudicated parse refusal, and validates as an ADL 2 upload is validated
//! (parents first, the production RM) or is refused on its adjudicated code.
//! Each reason opens with what causes it: `conversion:` (CKM's converter),
//! `source:` (the CKM archetype) or `parent:` (a refused parent).
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::SyntaxErrorCode;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::ValidationCode;

use crate::ckm_archetype_packs::{
    LoincServed, ValidityRefusal, file_name, files_with_extension, validate_all,
};

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/archetypes/ckm/adl2-generated")
}

/// The one file of the export that does not parse: CKM writes the 1.4
/// `[local::]` (a terminology constraint with no codes) as
/// `defining_code matches {}`, an empty block ADL 2 has no production for
/// (openEHR/archie#896, the shape whose `[openEHR::]` twin CKM exports empty).
const PARSE_REFUSAL: (&str, SyntaxErrorCode) = (
    "openEHR-EHR-CLUSTER.exclusion_symptom_sign.v0.adls",
    SyntaxErrorCode::Scas,
);

/// The adjudicated validity refusals with no terminology served.
const ADJUDICATED: &[ValidityRefusal] = &[
    (
        "openEHR-DEMOGRAPHIC-ADDRESS.address-provider.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-DEMOGRAPHIC-ADDRESS.address.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication-provider.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-DEMOGRAPHIC-CAPABILITY.individual_credentials.v0.adls",
        ValidationCode::Vacmco,
        "source: five mandatory children and an optional one cannot fit the cardinality upper bound of 5 (master04.5 VACMCO), as in the 1.4 source",
    ),
    (
        "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier-provider.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name-individual_provider.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-DEMOGRAPHIC-PERSON.person-patient.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-DEMOGRAPHIC-PERSON.person.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-ACTION.blood_transfusion_management.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.acquisition_details_on_eye_fundus_images.v0.adls",
        ValidationCode::Vatda,
        "conversion: the assumed value takes the shifted code at8 while value set ac9001 keeps the 1.4 members",
    ),
    (
        "openEHR-EHR-CLUSTER.acquisition_details_on_ophthalmic_tomography.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.acquisition_details_on_visual_field_test.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.activity-running.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.address.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.alcohol_consumption.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.anatomical_location_circle.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.anatomical_location_precise.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.anatomical_location_relative.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.catheter_cuff.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.catheter_lumen.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.catheter_tube.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.cobb_angle.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.conditional_medication_rules.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.corneal_thickness_details.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.dietary_nutrients.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.dietary_phytochemicals.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.dosage.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.endotracheal_tube.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.environmental_conditions.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam-palpation_cervix.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam-pupil.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam-tooth.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam-tympanic_membrane.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam-uterus.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam_lesion.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam_oocyte.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam_wound.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.fetus_abdominal.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.fetus_vaginal.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.findings_glaucoma.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.genomic_copy_number_variant.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.genomic_variant_result.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.geolocation.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.gist_modified_nih.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.hip_arthroplasty_component.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.humidification.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.image_acquisition_details.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.image_reconstruction_details.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-bladder.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-cervix.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-foetus.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-gestational_sac.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-hip_joint.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-lymph_node.v0.adls",
        ValidationCode::Vasid,
        "source: names the parent openEHR-EHR-CLUSTER.imaging_exam.v0, a version CKM does not publish",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-lymph_node_group.v0.adls",
        ValidationCode::Vasid,
        "source: names the parent openEHR-EHR-CLUSTER.imaging_exam.v0, a version CKM does not publish",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-ovary.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-pelvis.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-placenta.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-rectouterine_pouch.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-sacrum.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-spine.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-testicle.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_exam-uterus.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.imaging_myometrial_lesion.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.inspection_body_fluid-sputum.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-CLUSTER.inspection_body_fluid.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-CLUSTER.inspection_body_fluid-urine.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-CLUSTER.inspection_body_fluid.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-CLUSTER.inspired_oxygen.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.intraocular_pressure_test_reliability_indicator.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.intravitreal_injection_details.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.lab_microscopy_parasitology.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.lab_molecular_microbial.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.lens_specification.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.level_of_exertion.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.lymph_node_metastases.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.macronutrients.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.macroscopy_colorectal_carcinoma.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.maximal_blood_pressure.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.micronutrients.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.microscopy_colorectal_carcinoma.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.microscopy_melanoma.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.mydriasis_application.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.ophthalmic_thickness_details.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.pharmacogenetic_test_result.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.physical_dimensions.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.radiotherapy.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.reference_sequence.v1.adls",
        ValidationCode::Varcn,
        "conversion: the 1.4 source roots at at0018, and the conversion keeps it where ADL 2 needs the root code (VARCN)",
    ),
    (
        "openEHR-EHR-CLUSTER.refraction_details.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.severity_rating_scale.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.symptom_sign-cvid.v0.adls",
        ValidationCode::Vasid,
        "source: names the parent openEHR-EHR-CLUSTER.symptom_sign.v1, a version CKM does not publish",
    ),
    (
        "openEHR-EHR-CLUSTER.timing_daily.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.tumour_resection_margins.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-CLUSTER.ventilator_settings2.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.alcohol_consumption_summary.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.goal.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.health_risk-covid.v0.adls",
        ValidationCode::Vasid,
        "parent: its parent openEHR-EHR-EVALUATION.health_risk.v1 is refused above, so no flat parent exists (master03 VASID)",
    ),
    (
        "openEHR-EHR-EVALUATION.health_risk.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.long_term_process_enrollment-antiVEGF_AMD.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-EVALUATION.long_term_process_enrollment.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-EVALUATION.obstetric_summary-JM.v0.adls",
        ValidationCode::Vasid,
        "parent: its parent openEHR-EHR-EVALUATION.obstetric_summary.v1 is refused above, so no flat parent exists (master03 VASID)",
    ),
    (
        "openEHR-EHR-EVALUATION.obstetric_summary.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.peak_intraocular_pressure_assertion.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.physical_activity_summary.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.pregnancy_summary.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.recommendation-DR_treatment.v1.adls",
        ValidationCode::Vasid,
        "source: names the parent openEHR-EHR-EVALUATION.recommendation.v1, a version CKM does not publish",
    ),
    (
        "openEHR-EHR-EVALUATION.recommendation-amd_treatment.v0.adls",
        ValidationCode::Vasid,
        "source: names the parent openEHR-EHR-EVALUATION.recommendation.v1, a version CKM does not publish",
    ),
    (
        "openEHR-EHR-EVALUATION.recommendation-glaucoma_treatment.v0.adls",
        ValidationCode::Vasid,
        "source: names the parent openEHR-EHR-EVALUATION.recommendation.v1, a version CKM does not publish",
    ),
    (
        "openEHR-EHR-EVALUATION.smokeless_tobacco_summary.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-EVALUATION.tobacco_smoking_summary.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-INSTRUCTION.assisted_ventilation.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-INSTRUCTION.service_request-imaging_examination.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-INSTRUCTION.service_request.v1 by major version, but the conversion versions that parent as the pre-release 1.1.3-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-INSTRUCTION.service_request-laboratory_test.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-INSTRUCTION.service_request.v1 by major version, but the conversion versions that parent as the pre-release 1.1.3-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-INSTRUCTION.supplemental_oxygen_order.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-INSTRUCTION.transfusion_order.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.abc_stroke_risk_score.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.acoustic_reflex_result.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.affected_body_surface_area-burn.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.affected_body_surface_area.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.agatston_score.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.alcohol_intake.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.aos_score.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.audiogram_result.v0.adls",
        ValidationCode::Vatda,
        "conversion: the assumed value takes the shifted code at31 while value set ac9011 keeps the 1.4 members",
    ),
    (
        "openEHR-EHR-OBSERVATION.audiology_speech_test_result.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.audiology_speech_test_result_local.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.auditory_brainstem_response_result.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.axial_ocular_biometry.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.behavioural_observation_audiometry_result.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.blood_pressure.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_composition.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_mass_index.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_segment_area.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_segment_circumference.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_segment_discrepancy.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_segment_length.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_surface_area.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_temperature.v2.adls",
        ValidationCode::Vatda,
        "conversion: the assumed value takes the shifted code at34 while value set ac9075 keeps the 1.4 members",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_weight.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.briganti_risk_score.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.capillary_refill.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.categorical_loudness_scaling.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.chaq.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.charlson_comorbidity_index.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.chest_circumference.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.child_growth_indicator.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.contrast_sensitivity_test.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.das28-CRP.v0.adls",
        ValidationCode::Vasid,
        "parent: its parent openEHR-EHR-OBSERVATION.das28.v0 is refused above, so no flat parent exists (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.das28.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.demo.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.dermatology_therapy_summary.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.ecg_result.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.ejection_fraction-left_ventricle.v0.adls",
        ValidationCode::Vasid,
        "parent: its parent openEHR-EHR-OBSERVATION.ejection_fraction.v0 is refused above, so no flat parent exists (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.ejection_fraction.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.empower_meal.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.empower_physical_exercises.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.empower_sleep.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.estimated_glomerular_filtration_rate.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.exclusion-adverse_reactions.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.exclusion.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.exclusion-pregnancy.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.exclusion.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.exophthalmometry.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fact_g-Hep.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.fact_g.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.faecal_output.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fetal_biometry.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fetal_growth.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fetal_heart-monitoring.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.fetal_heart.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fluid_balance.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fluid_input.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fluid_output-blood.v0.adls",
        ValidationCode::Vasid,
        "parent: its parent openEHR-EHR-OBSERVATION.fluid_output.v1 is refused above, so no flat parent exists (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fluid_output.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.food_item.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.g8_screening_tool.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.growth_velocity.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.hand_grip_strength.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.haq.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.harris_hip.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.head_circumference.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.hearing_screening_result.v0.adls",
        ValidationCode::Vatda,
        "conversion: the assumed value takes the shifted code at30 while value set ac9005 keeps the 1.4 members",
    ),
    (
        "openEHR-EHR-OBSERVATION.heartbeat-pulse.v0.adls",
        ValidationCode::Vasid,
        "parent: its parent openEHR-EHR-OBSERVATION.heartbeat.v0 is refused above, so no flat parent exists (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.heartbeat.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.height.v2.adls",
        ValidationCode::Vatda,
        "conversion: the assumed value takes the shifted code at17 while value set ac9015 keeps the 1.4 members",
    ),
    (
        "openEHR-EHR-OBSERVATION.hip_circumference.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.infant_feeding.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.intraocular_pressure.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.iss-revised.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.iss.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.jugular_venous_pressure.v0.adls",
        ValidationCode::Vatda,
        "conversion: the assumed value takes the shifted code at17 while value set ac9004 keeps the 1.4 members",
    ),
    (
        "openEHR-EHR-OBSERVATION.mantoux_test_result.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.map_hand.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.medication_statement-JM.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.medication_statement.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.menstrual_diary.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.mini_bestest.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.mini_nutritional_assessmemt_short_form.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.msfc_score.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.murray_score.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.ophthalmic_tomography_examination.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pasi_score.v1.adls",
        ValidationCode::Vcosu,
        "conversion: 1.4 node codes unique only among siblings (at0005, at0011, at0017) are not re-minted, and ADL 2 needs them unique archetype-wide (VCOSU)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pf_ratio.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.physical_activity.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pulse.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pulse_deficit.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pulse_oximetry.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.refractive_error_measurement.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.respiration.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.sara_scale.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.self_test_result-pregnancy.v0.adls",
        ValidationCode::Vasid,
        "conversion: names its parent openEHR-EHR-OBSERVATION.self_test_result.v0 by major version, but the conversion versions that parent as the pre-release 0.0.1-alpha, which a reference reaches only by its full id (BASE artefact_identification master07-referencing)",
    ),
    (
        "openEHR-EHR-OBSERVATION.specular_microscopy.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.spirometry_result.v2.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.symptom_sign.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.testicular_volume.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.tobacco_use.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.tympanogram_226hz.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.tympanogram_hf.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.uterine_contractions.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.ventilator_record.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.visual_acuity.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.visual_field_measurement.v0.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
    (
        "openEHR-EHR-OBSERVATION.waist_circumference.v1.adls",
        ValidationCode::Vcarm,
        "conversion: constrains `property` on DV_QUANTITY, which no RM release defines; the converter still writes AOM 1.4 C_DV_QUANTITY.property (#4921)",
    ),
];

/// The refusals a LOINC-serving terminology adds.
const ADJUDICATED_LOINC: &[ValidityRefusal] = &[
    (
        "openEHR-EHR-OBSERVATION.braden_scale.v1.adls",
        ValidationCode::Vetdf,
        "source: binds the mistyped LOINC answer code LA9605-4, which LOINC does not define (master03 VETDF)",
    ),
    (
        "openEHR-EHR-OBSERVATION.braden_scale_q.v0.adls",
        ValidationCode::Vetdf,
        "source: binds the mistyped LOINC answer code LA9605-4, which LOINC does not define (master03 VETDF)",
    ),
];

#[test]
fn every_file_parses_but_the_adjudicated_one() {
    let files = files_with_extension(&pack(), "adls");
    assert!(files.len() > 900, "the export holds {} files", files.len());
    for path in &files {
        let src = std::fs::read_to_string(path).expect("read the export");
        let name = file_name(path);
        match parse_artefact(&src, Dialect::Adl2) {
            Ok(_) => assert_ne!(
                name, PARSE_REFUSAL.0,
                "the adjudicated parse refusal now parses"
            ),
            Err(errors) => {
                assert_eq!(name, PARSE_REFUSAL.0, "{name} does not parse: {errors:?}");
                assert!(
                    errors.iter().any(|e| e.code == PARSE_REFUSAL.1),
                    "{errors:?}"
                );
            }
        }
    }
}

#[test]
fn every_parseable_file_validates_or_is_adjudicated() {
    let files = files_with_extension(&pack(), "adls");
    let parseable = files.len() - 1;
    let without = validate_all(&files, ADJUDICATED, &NoTerminologyResolver);
    assert!(
        without.broken_adjudications.is_empty(),
        "{:#?}",
        without.broken_adjudications
    );
    assert!(without.failures.is_empty(), "{:#?}", without.failures);
    assert_eq!(without.refused, ADJUDICATED.len());
    assert_eq!(without.clean + without.refused, parseable);

    let table: Vec<ValidityRefusal> = ADJUDICATED
        .iter()
        .chain(ADJUDICATED_LOINC)
        .copied()
        .collect();
    let with = validate_all(&files, &table, &LoincServed);
    assert!(
        with.broken_adjudications.is_empty(),
        "{:#?}",
        with.broken_adjudications
    );
    assert!(with.failures.is_empty(), "{:#?}", with.failures);
    assert_eq!(
        (with.clean, with.refused),
        (parseable - table.len(), table.len())
    );
}

/// `SHA256SUMS` names exactly the vendored files, so the provenance pin covers
/// the whole pack.
#[test]
fn the_digest_list_names_every_file() {
    let listed: BTreeSet<String> = std::fs::read_to_string(pack().join("SHA256SUMS"))
        .expect("read SHA256SUMS")
        .lines()
        .filter_map(|line| line.split_once("  ").map(|(_, name)| name.to_owned()))
        .collect();
    let present: BTreeSet<String> = files_with_extension(&pack(), "adls")
        .iter()
        .map(|p| file_name(p))
        .collect();
    assert_eq!(listed, present);
}
