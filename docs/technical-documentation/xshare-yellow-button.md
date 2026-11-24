# The xShare Yellow Button: flow, data protection and the Bronze evidence

Cadasto B.V. declares the xShare industry label at Bronze level for the EHR
system FerroEHR forms with FerroBRIDGE, its European interoperability
software component (Regulation (EU) 2025/327, the EHDS, Art. 25(1),
`docs/law/eu/ehds/text.html`), with FerroSMART as its authentication
component. Bronze is a self-declaration on the "xShare Yellow Button
Declaration of Conformity" at the EHRxF Hub. Its statements are the
Provider conformance statements of the xShare Yellow Button implementation
guide, `hl7.eu.fhir.xshare-yb` 0.3.0
(<https://hl7.eu/fhir/ig/xshare-yb/conformance.html>). This page records the
flow those statements describe, the data protection facts of it, and the
evidence for each statement. The declaration itself is submitted by
Cadasto B.V.; nothing in this repository submits it.

The system is offered as an API for patient-facing apps: there is no patient
portal of its own. Any SMART app that signs the person in at FerroSMART
reaches the two Yellow Button functions on FerroBRIDGE.

## The flow

```text
 patient app ── standalone patient launch ──▶ FerroSMART ── login ──▶ eID (DigiD over SAML, ...)
     │                                          │  acr = the eIDAS level of the login
     │ Bearer token (patient/ scopes, acr)      │
     ▼                                          │ RFC 8693 token exchange
 FerroBRIDGE ◀─────────────────────────────────┘ (sub, fhirUser, patient, acr, ehrId; client_id = FerroBRIDGE)
     │  GET /fhir/Patient/{id}/$summary   (Download)
     │  POST, GET, DELETE /shl            (One-time Share)
     │  exchanged token, patient/composition-*.r patient/aql-*.s
     ▼
 FerroEHR (openEHR CDR) ── terminology ──▶ FerroTERM
```

1. The person signs in to a patient app with a standalone patient launch
   (SMART App Launch 2.2). FerroSMART authenticates them at an
   eIDAS-recognised eID and writes the level of that login into the token's
   `acr` (EHDS Art. 16(1)).
2. The app calls FerroBRIDGE with the person's token. FerroBRIDGE checks it
   and exchanges it at FerroSMART (RFC 8693) for a token at FerroEHR's
   openEHR base that names the same person, their EHR and FerroBRIDGE as the
   client, with no `act` and the read and query scopes alone.
3. FerroEHR treats the exchanged token as the person on their own record:
   it serves only the EHR whose subject is the person's `PERSON`, withholds
   what the EHDS Art. 3(3) access delay holds, refuses a login below the
   `[authz.assurance]` floor with the RFC 9470 §3 step-up challenge, and
   records each access naming the person, FerroBRIDGE and the level (EHDS
   Annex II 3.2).
4. **Download.** FerroBRIDGE writes the patient summary as an HL7 IPS 2.0.1
   document `Bundle`, with codes from FerroTERM, validates it against its
   profile and returns it.
5. **One-time Share.** FerroBRIDGE writes the same document, encrypts it
   once under a fresh key it hands back in the SMART Health Link and never
   keeps, and serves the link's manifest to the receiver within the link's
   limits (SMART Health Cards and Links 1.0.0). The person lists and
   revokes their links.

## Data protection

- **Purpose.** The person's right to access their electronic health data
  and to download an electronic copy of it in the European exchange format
  (EHDS Art. 3(1) and (2)), and to give another healthcare provider access to
  it (Art. 7(1)), beside the right of access and to data portability of
  Regulation (EU) 2016/679 (GDPR) Art. 15 and 20.
- **Lawful basis.** GDPR Art. 6(1)(c), a legal obligation of the controller,
  the healthcare provider, under the EHDS Art. 3, with Art. 9(2)(h) for the
  health data. The controller is the healthcare provider who deploys the
  system; Cadasto B.V. is its manufacturer.
- **Minimisation.** The exchanged token reads compositions and runs one
  EHR-scoped query, nothing else, and lives five minutes at most.
  FerroBRIDGE keeps no plaintext document and no key for a link: its link
  store holds the rules, the Argon2id hash of a passcode and the ciphertext,
  and a link leaves the store whole at its expiry, its revocation, its last
  answer or when its passcode attempts run out. A link expires 24 hours after
  creation by default and 7 days at most, answers once by default, and is
  disabled after five wrong passcodes over its life.
- **Records of the shares.** The creation, every answer, every refused
  passcode, the revocation and the end of a link are access records with the
  recipient the receiver named, and so is every read FerroEHR serves the
  exchanged token. They are kept as every access record is, for at least the
  three years EHDS Art. 9(2) asks: FerroEHR's under its retention register,
  FerroBRIDGE's at the Audit Record Repository the deployer configures.

## The Bronze evidence

The statements below are the Provider statements of the guide's Conformance
page, Download (D) and One-time Share (S). Before the form is submitted, its
live wording is compared with this table (FerroHEALTH/FerroEHR#6419). The
Consumer statements are not declared: the system is a Provider.

| # | Statement | Evidence |
|---|---|---|
| D1, S1 | Only authenticated natural persons access the function. | FerroBRIDGE: `tests/it/facade/summary.rs` `a_patient_token_for_another_patient_answers_404`, `tests/it/facade/shl.rs` `a_create_outside_the_limits_or_without_a_person_is_refused`. FerroEHR: `app/ferroehr-rest/tests/it/exchanged_patient_token_http.rs` `the_exchanged_token_reads_its_own_ehr_and_no_other`. |
| D2, S2 | User authentication conforms to the European regulations. | FerroSMART writes the eIDAS level into `acr` (FerroHEALTH/FerroSMART#784). FerroEHR refuses a level below substantial with the step-up challenge: `exchanged_patient_token_http.rs` `the_exchanged_token_below_the_floor_gets_the_step_up_challenge`; book page [The xShare Yellow Button](../../website/book/src/xshare-yellow-button.md). |
| D3, S3 | Only the authenticated person searches for and selects their own data. | FerroEHR holds the token to the person's own EHR and to the access delay: `exchanged_patient_token_http.rs` `the_exchanged_token_reads_its_own_ehr_and_no_other`, `the_access_delay_withholds_from_the_exchanged_token`; FerroBRIDGE `the_person_lists_and_revokes_only_their_own_links`. |
| D4 | The download passes validation for each declared content specification. | Declared: IPS 2.0.1. FerroBRIDGE validates each document before it serves it (`a_document_that_breaks_its_profile_is_refused_with_422`) and its CI holds the served summaries to the HL7 FHIR Validator, attaching the reports to every release (FerroHEALTH/FerroBRIDGE#2015). The xShare 0.3.0 profile joins once the package is published (FerroHEALTH/FerroBRIDGE#2029). |
| D5 | The download is available in the selected target format. | `GET /fhir/Patient/{id}/$summary` answers an IPS 2.0.1 document `Bundle` as `application/fhir+json`: `a_patient_s_summary_is_an_ips_document_with_coded_entries`. |
| S4 | A valid SMART Health Link is generated. | FerroBRIDGE `tests/it/facade/shl.rs` `a_link_with_a_passcode_opens_once_in_a_receiver_and_yields_the_summary`, `a_link_without_a_passcode_is_u_and_answers_one_direct_request`. |
| S5 | The receiver gets the authorised content, the SHL manifest, in the selected format. | Same tests: the receiver opens the manifest's file with the link's key and reads the IPS `Bundle`. |
| S6 | The manifest's content passes validation for each declared content specification. | The file is the document D4 validates; the end-to-end API journey in CI validates it through a link (FerroHEALTH/FerroBRIDGE#2018). |
| Form 1 | The organisation is legally established and the data is authentic and based on real medical data. | Owner's statement. Unticked until a deployment holds real records; the conformance evidence uses synthetic test records. |

### Answers for the self-assessment

| # | English | Nederlands |
|---|---|---|
| D1, S1 | Yes. Every Yellow Button endpoint requires a SMART access token of the person; FerroEHR serves the person's own record only. | Ja. Elk Gele Knop-endpoint vereist een SMART-toegangstoken van de persoon; FerroEHR levert alleen het eigen dossier van de persoon. |
| D2, S2 | Yes. The person signs in with an eIDAS-recognised eID; the level of assurance is checked on every request, at substantial at least. | Ja. De persoon logt in met een eIDAS-erkend eID; het betrouwbaarheidsniveau wordt bij elk verzoek gecontroleerd, minimaal substantieel. |
| D3, S3 | Yes. The person selects their patient summary and manages only their own links; another person's record is refused. | Ja. De persoon kiest de eigen patiëntsamenvatting en beheert alleen de eigen links; het dossier van een ander wordt geweigerd. |
| D4 | Yes. The patient summary is validated against IPS 2.0.1 before it is served and by the HL7 FHIR Validator in every release. | Ja. De patiëntsamenvatting wordt vóór levering gevalideerd tegen IPS 2.0.1 en bij elke release door de HL7 FHIR Validator. |
| D5 | Yes. HL7 IPS 2.0.1 as a FHIR R4 document Bundle in JSON. | Ja. HL7 IPS 2.0.1 als FHIR R4-documentbundel in JSON. |
| S4 | Yes. A SMART Health Link per SMART Health Cards and Links 1.0.0, with or without a passcode, expiring within 7 days. | Ja. Een SMART Health Link volgens SMART Health Cards and Links 1.0.0, met of zonder toegangscode, die binnen 7 dagen verloopt. |
| S5 | Yes. The receiver opens the manifest and decrypts the IPS document with the link's key. | Ja. De ontvanger opent het manifest en ontsleutelt het IPS-document met de sleutel uit de link. |
| S6 | Yes. The shared document is the validated IPS 2.0.1 document. | Ja. Het gedeelde document is het gevalideerde IPS 2.0.1-document. |
| Form 1 | Not ticked until a deployment holds real records. | Niet aangevinkt zolang geen installatie echte gegevens bevat. |
