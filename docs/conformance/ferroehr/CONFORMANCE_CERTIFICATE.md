# Conformance Certificate

## System Under Test

| Field | Value |
| --- | --- |
| Solution | ferroehr 4.3.0 |
| Vendor | Cadasto B.V. |
| Runner | veredictum 0.1.5 |
| Infrastructure | ixit.json#/environment |

## Scope of Test

| Dimension | Value |
| --- | --- |
| Functional | CORE, STANDARD, OPTIONS, SEC-BASIC |
| Sec & Priv | SEC-BASIC PASS |
| Performance | class POC (earned) |
| Ext Data Fmt | canonical-json, canonical-xml, wt-flat, wt-structured |

## Profile Report

Result column: ITS its-rest (canonical-json, canonical-xml, wt-flat, wt-structured)

The Realization column says what the row's cases were verified against: `released-wire` = released ITS-REST operations; `extension` = routes this product serves of its own design, which no openEHR specification governs and which therefore never gate an openEHR profile tier (those rows are always OPT).

| Family | Capability | Required in profile | Realization | Result |
| --- | --- | --- | --- | --- |
| Platform | Adl14ArchetypeProvisioning | OPT | extension | pass |
| Platform | Adl14OptProvisioning | Y | released-wire | pass |
| Platform | Adl2ArchetypeProvisioning | OPT | released-wire | pass |
| Platform | Adl2OptProvisioning | OPT | released-wire | pass |
| Platform | TemplateExamples | OPT | released-wire | pass |
| Platform | QueryProvisioning | Y | released-wire | pass |
| Platform | EhrOperations | Y | released-wire | pass |
| Platform | EhrStatus | Y | released-wire | pass |
| Platform | CompositionOps | Y | released-wire | pass |
| Platform | DirectoryOps | Y | released-wire | pass |
| Platform | ChangeSets | Y | released-wire | pass |
| Platform | Versioning | Y | released-wire | pass |
| Platform | ArchetypeValidation | Y | released-wire | pass |
| Platform | PartyOperations | OPT | released-wire | pass |
| Platform | PartyRelationshipOperations | OPT | extension | pass |
| Platform | DemographicArchetypeValidation | OPT | released-wire | pass |
| Platform | AqlBasic | Y | released-wire | pass |
| Platform | AqlAdvanced | OPT | released-wire | pass |
| Platform | AqlTerminology | OPT | released-wire | pass |
| Platform | ActivityReport | OPT | extension | pass |
| Platform | PhysicalDeletion | OPT | released-wire | pass |
| Platform | EhrDumpLoad | OPT | extension | pass |
| Platform | BulkEhrLoad | OPT | released-wire | pass |
| Platform | EhrArchive | OPT | extension | pass |
| Platform | DemographicArchive | OPT | extension | pass |
| Platform | EhrExtract | OPT | extension | pass |
| Platform | Tds | OPT | extension | pass |
| Platform | DefinitionApi | Y | released-wire | pass |
| Platform | EhrApi | Y | released-wire | pass |
| Platform | DemographicApi | OPT | released-wire | pass |
| Platform | QueryApi | Y | released-wire | pass |
| Platform | AdminApi | OPT | released-wire | pass |
| Platform | MessageApi | OPT | extension | pass |
| Platform | SystemApi | OPT | released-wire | pass |
| Platform | ItemTags | OPT | released-wire | pass |
| Platform | Signing | OPT | released-wire | pass |
| Platform | SimplifiedFormats | OPT | released-wire | pass |
| Platform | SmartAppLaunch | OPT | released-wire | pass |
| Security | EhrDemographicSeparation | Y | released-wire | pass |
| Security | AuthenticatedAccess | Y | released-wire | pass |
| Security | AuthorizationSeparation | Y | released-wire | pass |
| Security | AuditAccountability | Y | released-wire | pass |
| Security | AnonymousEhrs | Y | released-wire | pass |

## Workload Coverage

The exercised-capability set of the measured hospital-simulation workload against the claimed matrix. A claimed capability the simulation never touches is either an ADJUDICATED exclusion — the capability-matrix row names the register entry that decided it and the reason is printed in the row — or an undecided catalogue gap, which the `workload-coverage` validate gate fails on, so no published certificate reaches this section carrying one.

| Capability | Claimed | Exercised by workload |
| --- | --- | --- |
| Adl14ArchetypeProvisioning | yes | no — adjudicated exclusion (AMB-170): definition administration, not a sustainable per-patient arrival: archetype provisioning is a design-time operation a hospital simulation would not repeatedly drive (the same family reason the ADL2/OPT provisioning rows carry journeys for is satisfied by the definition-poll journey; this row's own upload/delete churn would grow the definition store unboundedly through a measured hold) |
| Adl14OptProvisioning | yes | yes |
| Adl2ArchetypeProvisioning | yes | yes |
| Adl2OptProvisioning | yes | yes |
| TemplateExamples | yes | yes |
| QueryProvisioning | yes | yes |
| EhrOperations | yes | yes |
| EhrStatus | yes | yes |
| CompositionOps | yes | yes |
| DirectoryOps | yes | yes |
| ChangeSets | yes | yes |
| Versioning | yes | yes |
| ArchetypeValidation | yes | yes |
| PartyOperations | yes | yes |
| PartyRelationshipOperations | yes | yes |
| DemographicArchetypeValidation | yes | yes |
| AqlBasic | yes | yes |
| AqlAdvanced | yes | yes |
| AqlTerminology | yes | yes |
| ActivityReport | yes | yes |
| PhysicalDeletion | yes | no — adjudicated exclusion (AMB-170): destructive mid-measurement: the released admin operations erase an EHR and its whole version history outright, so every arrival would shrink the population-anchored corpus the measured window is bound to and the earned class would no longer describe the declared volumetric class |
| EhrDumpLoad | yes | no — adjudicated exclusion (AMB-170): one-shot by nature: export_ehrs dumps the WHOLE repository to a file system and load_ehrs reads a whole archive back, so neither is a per-patient interaction a hospital simulation could sustain - a repeated arrival would re-dump the entire population-anchored corpus the measured window is bound to, and the load half would additionally grow it. The exclusion is about the SHAPE of the operation, not about routing, so it stands now that the routes exist |
| BulkEhrLoad | yes | no — adjudicated exclusion (AMB-170): the bulk load IS every measured run's own seeding phase: the scale corpus is loaded strictly through the public API before the window opens, so the capability is exercised per-run by the seeder - as a sustained ARRIVAL it would be a second population grower distorting the population-anchored envelope, the same one-shot family as a whole-server dump/load |
| EhrArchive | yes | no — adjudicated exclusion (AMB-170): destructive mid-measurement: archiving is a lifecycle transition applied to a named set of EHRs, so every arrival would move part of the population-anchored corpus the measured window is bound to into the archival tier - the earned class would no longer describe the declared volumetric class. The same one-shot/destructive family as physical deletion, and unlike the read-only extension routes the simulation now polls |
| DemographicArchive | yes | no — adjudicated exclusion (AMB-170): destructive mid-measurement: archiving is a lifecycle transition applied to a named set of parties, so every arrival would move part of the demographic population the measured window is bound to into the archival tier - the same one-shot/destructive family as physical deletion and the EHR archive half |
| EhrExtract | yes | yes |
| Tds | yes | yes |
| DefinitionApi | yes | yes |
| EhrApi | yes | yes |
| DemographicApi | yes | yes |
| QueryApi | yes | yes |
| AdminApi | yes | no — adjudicated exclusion (AMB-170): destructive mid-measurement: the released ADMIN API of ITS-REST 1.1.0 is exactly admin_ehr_delete and admin_ehr_delete_all, so the only wire this claim covers is the same corpus-erasing pair PhysicalDeletion excludes (the further /admin routes this server mounts are extensions no claim rests on) |
| MessageApi | yes | yes |
| SystemApi | yes | yes |
| ItemTags | yes | yes |
| Signing | yes | yes |
| SimplifiedFormats | yes | yes |
| SmartAppLaunch | yes | yes |
| EhrDemographicSeparation | yes | yes |
| AuthenticatedAccess | yes | yes |
| AuthorizationSeparation | yes | yes |
| AuditAccountability | yes | yes |
| AnonymousEhrs | yes | yes |

Claimed capabilities excluded from the measured workload by adjudication (7): Adl14ArchetypeProvisioning, PhysicalDeletion, EhrDumpLoad, BulkEhrLoad, EhrArchive, DemographicArchive, AdminApi. Each row above names its register entry; the exclusion bounds the LOAD instrument only — the functional catalogue still owes every one of them verdict-bearing cases at its `min_cases` floor.

Every claimed capability is exercised by the simulation or carries an adjudicated exclusion — no undecided rows.

## Performance Rating

Classes are EARNED by measurement (never declared); every earned class is bound to the measured environment recorded in the results.

| Class | Case | Claimed | Verdict |
| --- | --- | --- | --- |
| POC | PERF-hospital_sim-class_POC | yes | EARNED |

Environment (PERF-hospital_sim-class_POC): consumer-laptop · 8 cores · 16 GB · nvme · single-node docker compose (8-CPU/8GB Docker VM) on Apple M2, the SMART resource-server posture (docker/sut-smart.yml overlays the base stack) with the external-terminology profile composed beside it (a seeded HAPI FHIR R4 server, docker compose --profile terminology + docker/sut-terminology.yml, fail-open); alongside it a second deployment of the same image in the openPGP version-signing posture (project ferroehr-cnf-pgp, docker/sut-signing-pgp.yml + docker/sut-terminology-failclosed.yml + docker/sut-pgp-parallel.yml, host port 8081) declared as the sut_pgp instance, which carries the fail-closed terminology posture — the measured-performance stage drives the primary deployment alone

