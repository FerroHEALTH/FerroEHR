# Control matrix

FerroEHR is software. It is not a controller, not a processor and not a
certified organisation, so this page makes no compliance claim on anyone's
behalf. It lists the technical controls the product ships or plans, and the
article or clause each one is designed to support. Whether a deployment
satisfies a legal obligation depends on how the deploying organisation runs
it.

Every row comes from the tracker. A control is declared on the issue that
delivers it, as a line in the issue body:

```text
Control: <legal source> <article or clause>
```

The short name resolves to an official publisher URL from a registry inside
the generator, so a legal citation on this page is never free text. An issue
may declare several controls, one per line. The Control column carries the
issue title, unless the body also states the control on its own line, which
the generator prefers so that an issue titled as the defect it fixed does not
read its defect as the control:

```text
Control-text: <the control, stated as what the product does>
```

## How this page is built

`scripts/render/control-matrix.sh` queries the tracker with the GitHub CLI,
joins each declared control to its legal source and to its current state, and
writes this file. A CI job re-runs the generator with `--check` and fails the
build when the committed page no longer matches the tracker, which is what
keeps a shipped control from sitting here as "planned".

- **Shipped:** the issue is closed as completed. The merged pull request that
  closed it is linked in the last column. The one pull request that closes a
  control regenerates this page as it will read after the merge, so the page
  never lags a shipped control.
- **Planned:** the issue is open. Whether work has started is the issue's
  column on the [public roadmap board](https://github.com/orgs/FerroHEALTH/projects/2),
  which this page does not copy: a status that lives in two places disagrees
  the day one of them moves.
- **Not planned:** the issue was closed without the control being built. The
  row stays visible so the record does not quietly lose it.

The page carries no generation timestamp and no build commit. Both change on
every run or every push while the tracker has not moved, which would make the
CI staleness check fail on days when nothing was wrong. When this page was
last regenerated, and from which commit, is the file's own git history.

## Controls

| Legal source | Applies to | Article or clause | Control | Issue | Status | Closing PR |
|---|---|---|---|---|---|---|
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 15(1) | A natural person cannot obtain their own access log: the only retrieval is admin-gated over the whole repository | [#3240](https://github.com/FerroHEALTH/FerroEHR/issues/3240) | Shipped | [#3263](https://github.com/FerroHEALTH/FerroEHR/pull/3263) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 17(1) | Physical deletion of a party removes its externalised multimedia blobs | [#3180](https://github.com/FerroHEALTH/FerroEHR/issues/3180) | Shipped | [#3201](https://github.com/FerroHEALTH/FerroEHR/pull/3201) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 18 | A restriction register at whole-EHR and single-object grain, with the marked object refused on every read, query, export, event and write path while its stored rows stay untouched | [#3324](https://github.com/FerroHEALTH/FerroEHR/issues/3324) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 21(6) | A per-EHR research objection that removes the record from population queries, exports and the event stream while leaving reads for care untouched, with the controller's public-interest override recorded beside it | [#3325](https://github.com/FerroHEALTH/FerroEHR/issues/3325) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 25(1) | Contributor rules for personal data handling and a PR guard for privacy-boundary changes | [#3167](https://github.com/FerroHEALTH/FerroEHR/issues/3167) | Shipped | [#3172](https://github.com/FerroHEALTH/FerroEHR/pull/3172) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 25(2) | Refuse identifying data on the clinical side and constrain the subject reference to a pseudonym namespace | [#3154](https://github.com/FerroHEALTH/FerroEHR/issues/3154) | Shipped | [#3187](https://github.com/FerroHEALTH/FerroEHR/pull/3187) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 25(2) | The compliance layer assumes the Netherlands; it must be jurisdiction-pluggable | [#3185](https://github.com/FerroHEALTH/FerroEHR/issues/3185) | Shipped | [#3187](https://github.com/FerroHEALTH/FerroEHR/pull/3187) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 25(2) | The identifier scanner does not run on verbatim-replay writes (EHR-Extract import, admin load), and the book says every clinical write is scanned | [#3237](https://github.com/FerroHEALTH/FerroEHR/issues/3237) | Shipped | [#3251](https://github.com/FerroHEALTH/FerroEHR/pull/3251) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 25(2) | No database constraint holds the subject-reference shape: the CHECK #3154 promised was never built | [#3241](https://github.com/FerroHEALTH/FerroEHR/issues/3241) | Shipped | [#3265](https://github.com/FerroHEALTH/FerroEHR/pull/3265) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 30(1)(f) | A retention register of the period per content category and jurisdiction with its legal citation, per-EHR anchors, whole-record and per-object holds, a view of what has run out, and a boot-checked ceiling on access-log retention | [#3346](https://github.com/FerroHEALTH/FerroEHR/issues/3346) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1) | Deploy artifacts provision the demographic role split | [#3179](https://github.com/FerroHEALTH/FerroEHR/issues/3179) | Shipped | [#3193](https://github.com/FerroHEALTH/FerroEHR/pull/3193) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1) | Row-level security on demographic.national_identifier | [#3219](https://github.com/FerroHEALTH/FerroEHR/issues/3219) | Shipped | [#3223](https://github.com/FerroHEALTH/FerroEHR/pull/3223) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1) | Schema preparation runs on its own credential, never the clinical runtime role | [#3224](https://github.com/FerroHEALTH/FerroEHR/issues/3224) | Shipped | [#3229](https://github.com/FerroHEALTH/FerroEHR/pull/3229) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1) | A declared deployment profile: production refuses the postures it cannot prove, research says so in red | [#3226](https://github.com/FerroHEALTH/FerroEHR/issues/3226) | Shipped | [#3265](https://github.com/FerroHEALTH/FerroEHR/pull/3265) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1)(a) | Split demographic parties into a `demographic` schema with a non-overlapping runtime role | [#3153](https://github.com/FerroHEALTH/FerroEHR/issues/3153) | Shipped | [#3182](https://github.com/FerroHEALTH/FerroEHR/pull/3182) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1)(a) | National identifiers in the demographic schema: encrypted storage, keyed lookup, audited resolution | [#3155](https://github.com/FerroHEALTH/FerroEHR/issues/3155) | Shipped | [#3189](https://github.com/FerroHEALTH/FerroEHR/pull/3189) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1)(c) | Separate encryption keys and per-schema backup handling for the clinical and demographic domains | [#3157](https://github.com/FerroHEALTH/FerroEHR/issues/3157) | Shipped | [#3198](https://github.com/FerroHEALTH/FerroEHR/pull/3198) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1)(c) | The linkage schema is backed up and its boundary is probed | [#3220](https://github.com/FerroHEALTH/FerroEHR/issues/3220) | Shipped | — |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1)(d) | The storage-parity sweep and node rebuild cover the demographic domain | [#3178](https://github.com/FerroHEALTH/FerroEHR/issues/3178) | Shipped | [#3184](https://github.com/FerroHEALTH/FerroEHR/pull/3184) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 32(1)(d) | The two-DSN credential separation is exercised by test | [#3222](https://github.com/FerroHEALTH/FerroEHR/issues/3222) | Shipped | [#3231](https://github.com/FerroHEALTH/FerroEHR/pull/3231) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 4(5) | Split demographic parties into a `demographic` schema with a non-overlapping runtime role | [#3153](https://github.com/FerroHEALTH/FerroEHR/issues/3153) | Shipped | [#3182](https://github.com/FerroHEALTH/FerroEHR/pull/3182) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 4(5) | Refuse identifying data on the clinical side and constrain the subject reference to a pseudonym namespace | [#3154](https://github.com/FerroHEALTH/FerroEHR/issues/3154) | Shipped | [#3187](https://github.com/FerroHEALTH/FerroEHR/pull/3187) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 4(5) | Separate encryption keys and per-schema backup handling for the clinical and demographic domains | [#3157](https://github.com/FerroHEALTH/FerroEHR/issues/3157) | Shipped | [#3198](https://github.com/FerroHEALTH/FerroEHR/pull/3198) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 4(5) | Linkage service: the party to EHR resolve map as its own schema and role | [#3158](https://github.com/FerroHEALTH/FerroEHR/issues/3158) | Shipped | — |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 4(5) | The subject pseudonym is opt-in: nothing mints it, and an empty subject_namespaces is silent | [#3232](https://github.com/FerroHEALTH/FerroEHR/issues/3232) | Shipped | [#3266](https://github.com/FerroHEALTH/FerroEHR/pull/3266) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 5(1)(b) | Cross-domain cohort queries with a demographic predicate and a clinical selection | [#3159](https://github.com/FerroHEALTH/FerroEHR/issues/3159) | Shipped | [#3275](https://github.com/FerroHEALTH/FerroEHR/pull/3275) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 5(1)(e) | Audit retention has no jurisdictional floor: retention_days=1 erases the access log daily while the chain still verifies | [#3242](https://github.com/FerroHEALTH/FerroEHR/issues/3242) | Shipped | [#3266](https://github.com/FerroHEALTH/FerroEHR/pull/3266) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 5(1)(e) | A retention register of the period per content category and jurisdiction with its legal citation, per-EHR anchors, whole-record and per-object holds, a view of what has run out, and a boot-checked ceiling on access-log retention | [#3346](https://github.com/FerroHEALTH/FerroEHR/issues/3346) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj) | EU | Art. 89(1) | feat(ext): secondary use leaves through FerroBRIDGE to OMOP in batches over AQL: the CDR keeps the restriction, objection and retention marks, serves AQL over VERSION by time_committed as the batch surface and logs each population query; no in-CDR research domain | [#3379](https://github.com/FerroHEALTH/FerroEHR/issues/3379) | Planned | — |
| [EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) | EU | Annex II 3.2 | European logging software component: map EHDS logging requirements onto the access event model and ATNA trail | [#3170](https://github.com/FerroHEALTH/FerroEHR/issues/3170) | Shipped | [#3205](https://github.com/FerroHEALTH/FerroEHR/pull/3205) |
| [EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) | EU | Annex II 3.2 | Auditing off is one info-level line, and fail_mode=open is the silent default: make both visible where an operator looks | [#3238](https://github.com/FerroHEALTH/FerroEHR/issues/3238) | Shipped | [#3260](https://github.com/FerroHEALTH/FerroEHR/pull/3260) |
| [EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) | EU | Annex II 3.2 | The split clinical role holds no grant on the audit schema, so a separated deployment cannot write its own access log | [#3267](https://github.com/FerroHEALTH/FerroEHR/issues/3267) | Shipped | [#3270](https://github.com/FerroHEALTH/FerroEHR/pull/3270) |
| [EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) | EU | Annex II 3.2(a) | The access log records no accessing organisation (EHDS Annex II 3.2(a)) | [#3204](https://github.com/FerroHEALTH/FerroEHR/issues/3204) | Shipped | [#3213](https://github.com/FerroHEALTH/FerroEHR/pull/3213) |
| [EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) | EU | Art. 9 | A natural person cannot obtain their own access log: the only retrieval is admin-gated over the whole repository | [#3240](https://github.com/FerroHEALTH/FerroEHR/issues/3240) | Shipped | [#3263](https://github.com/FerroHEALTH/FerroEHR/pull/3263) |
| [EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) | EU | Chapter IV | feat(ext): secondary use leaves through FerroBRIDGE to OMOP in batches over AQL: the CDR keeps the restriction, objection and retention marks, serves AQL over VERSION by time_committed as the batch surface and logs each population query; no in-CDR research domain | [#3379](https://github.com/FerroHEALTH/FerroEHR/issues/3379) | Planned | — |
| [EDPB 01/2025](https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en) | EU | pseudonymisation domain | Split demographic parties into a `demographic` schema with a non-overlapping runtime role | [#3153](https://github.com/FerroHEALTH/FerroEHR/issues/3153) | Shipped | [#3182](https://github.com/FerroHEALTH/FerroEHR/pull/3182) |
| [EDPB 01/2025](https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en) | EU | pseudonymisation domain | Linkage service: the party to EHR resolve map as its own schema and role | [#3158](https://github.com/FerroHEALTH/FerroEHR/issues/3158) | Shipped | — |
| [EDPB 01/2025](https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en) | EU | pseudonymisation domain | Cross-domain cohort queries with a demographic predicate and a clinical selection | [#3159](https://github.com/FerroHEALTH/FerroEHR/issues/3159) | Shipped | [#3275](https://github.com/FerroHEALTH/FerroEHR/pull/3275) |
| [UAVG](https://wetten.overheid.nl/BWBR0040940) | NL | Art. 46 | Refuse identifying data on the clinical side and constrain the subject reference to a pseudonym namespace | [#3154](https://github.com/FerroHEALTH/FerroEHR/issues/3154) | Shipped | [#3187](https://github.com/FerroHEALTH/FerroEHR/pull/3187) |
| [UAVG](https://wetten.overheid.nl/BWBR0040940) | NL | Art. 46 | National identifiers in the demographic schema: encrypted storage, keyed lookup, audited resolution | [#3155](https://github.com/FerroHEALTH/FerroEHR/issues/3155) | Shipped | [#3189](https://github.com/FerroHEALTH/FerroEHR/pull/3189) |
| [Wabvpz](https://wetten.overheid.nl/BWBR0023864) | NL | Art. 15e | A natural person cannot obtain their own access log: the only retrieval is admin-gated over the whole repository | [#3240](https://github.com/FerroHEALTH/FerroEHR/issues/3240) | Shipped | [#3263](https://github.com/FerroHEALTH/FerroEHR/pull/3263) |
| [NEN 7513](https://www.nen.nl/nen-7513-2018-nl-245399) | NL | actor role | The access record carries no actor role or authorisation basis, which NEN 7513 requires and the RBAC layer already holds | [#3239](https://github.com/FerroHEALTH/FerroEHR/issues/3239) | Shipped | [#3262](https://github.com/FerroHEALTH/FerroEHR/pull/3262) |
| [NEN 7513](https://www.nen.nl/nen-7513-2018-nl-245399) | NL | event content | Per-domain access logging for reads and queries aligned with NEN 7513 and EHDS Art. 9 | [#3156](https://github.com/FerroHEALTH/FerroEHR/issues/3156) | Shipped | [#3188](https://github.com/FerroHEALTH/FerroEHR/pull/3188) |
| [NEN 7513](https://www.nen.nl/nen-7513-2018-nl-245399) | NL | event content | Domain-level access events discard their EmitOutcome, so fail_mode=closed does not cover them | [#3235](https://github.com/FerroHEALTH/FerroEHR/issues/3235) | Shipped | [#3246](https://github.com/FerroHEALTH/FerroEHR/pull/3246) |
| [NEN 7513](https://www.nen.nl/nen-7513-2018-nl-245399) | NL | retention | Audit retention has no jurisdictional floor: retention_days=1 erases the access log daily while the chain still verifies | [#3242](https://github.com/FerroHEALTH/FerroEHR/issues/3242) | Shipped | [#3266](https://github.com/FerroHEALTH/FerroEHR/pull/3266) |
| [BDSG](https://www.gesetze-im-internet.de/bdsg_2018/) | DE | § 35 Abs. 2 and 3 | A restriction register at whole-EHR and single-object grain, with the marked object refused on every read, query, export, event and write path while its stored rows stay untouched | [#3324](https://github.com/FerroHEALTH/FerroEHR/issues/3324) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [SGB V](https://www.gesetze-im-internet.de/sgb_5/) | DE | § 309 Abs. 3 | A retention register of the period per content category and jurisdiction with its legal citation, per-EHR anchors, whole-record and per-object holds, a view of what has run out, and a boot-checked ceiling on access-log retention | [#3346](https://github.com/FerroHEALTH/FerroEHR/issues/3346) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [GDNG](https://www.gesetze-im-internet.de/gdng/) | DE | § 6 Abs. 1 | A retention register of the period per content category and jurisdiction with its legal citation, per-EHR anchors, whole-record and per-object holds, a view of what has run out, and a boot-checked ceiling on access-log retention | [#3346](https://github.com/FerroHEALTH/FerroEHR/issues/3346) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [DSG](https://www.fedlex.admin.ch/eli/cc/2022/491/de) | CH | Art. 25 Abs. 2 lit. d | A retention register of the period per content category and jurisdiction with its legal citation, per-EHR anchors, whole-record and per-object holds, a view of what has run out, and a boot-checked ceiling on access-log retention | [#3346](https://github.com/FerroHEALTH/FerroEHR/issues/3346) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |
| [EPDV](https://www.fedlex.admin.ch/eli/cc/2017/204/de) | CH | Art. 10 | A retention register of the period per content category and jurisdiction with its legal citation, per-EHR anchors, whole-record and per-object holds, a view of what has run out, and a boot-checked ceiling on access-log retention | [#3346](https://github.com/FerroHEALTH/FerroEHR/issues/3346) | Shipped | [#3410](https://github.com/FerroHEALTH/FerroEHR/pull/3410) |

## Legal sources

The short names above resolve to these publishers. The linked text is the
authority; nothing on this page restates it.

| Short name | Applies to | Source |
|---|---|---|
| GDPR | EU | [https://eur-lex.europa.eu/eli/reg/2016/679/oj](https://eur-lex.europa.eu/eli/reg/2016/679/oj) |
| EHDS | EU | [https://eur-lex.europa.eu/eli/reg/2025/327/oj](https://eur-lex.europa.eu/eli/reg/2025/327/oj) |
| CRA | EU | [https://eur-lex.europa.eu/eli/reg/2024/2847/oj](https://eur-lex.europa.eu/eli/reg/2024/2847/oj) |
| EDPB 01/2025 | EU | [https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en](https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en) |
| EDPB 2/2023 | EU | [https://www.edpb.europa.eu/documents/guideline/guidelines-22023-on-technical-scope-of-art-53-of-eprivacy-directive_en](https://www.edpb.europa.eu/documents/guideline/guidelines-22023-on-technical-scope-of-art-53-of-eprivacy-directive_en) |
| UAVG | NL | [https://wetten.overheid.nl/BWBR0040940](https://wetten.overheid.nl/BWBR0040940) |
| Wabvpz | NL | [https://wetten.overheid.nl/BWBR0023864](https://wetten.overheid.nl/BWBR0023864) |
| NEN 7510 | NL | [https://www.nen.nl/nen-7510-1-2024-nl-331311](https://www.nen.nl/nen-7510-1-2024-nl-331311) |
| NEN 7512 | NL | [https://www.nen.nl/nen-7512-2022-nl-297137](https://www.nen.nl/nen-7512-2022-nl-297137) |
| NEN 7513 | NL | [https://www.nen.nl/nen-7513-2018-nl-245399](https://www.nen.nl/nen-7513-2018-nl-245399) |
| BDSG | DE | [https://www.gesetze-im-internet.de/bdsg_2018/](https://www.gesetze-im-internet.de/bdsg_2018/) |
| SGB V | DE | [https://www.gesetze-im-internet.de/sgb_5/](https://www.gesetze-im-internet.de/sgb_5/) |
| GDNG | DE | [https://www.gesetze-im-internet.de/gdng/](https://www.gesetze-im-internet.de/gdng/) |
| StGB | DE | [https://www.gesetze-im-internet.de/stgb/](https://www.gesetze-im-internet.de/stgb/) |
| DSG | CH | [https://www.fedlex.admin.ch/eli/cc/2022/491/de](https://www.fedlex.admin.ch/eli/cc/2022/491/de) |
| DSV | CH | [https://www.fedlex.admin.ch/eli/cc/2022/568/de](https://www.fedlex.admin.ch/eli/cc/2022/568/de) |
| EPDG | CH | [https://www.fedlex.admin.ch/eli/cc/2017/203/de](https://www.fedlex.admin.ch/eli/cc/2017/203/de) |
| EPDV | CH | [https://www.fedlex.admin.ch/eli/cc/2017/204/de](https://www.fedlex.admin.ch/eli/cc/2017/204/de) |
| IHE ATNA | INT | [https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html](https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html) |

`EU` and `INT` apply to every deployment. A two-letter country code is
national law or a national standard, and applies to a deployment in that
country: FerroEHR is an openEHR CDR, openEHR is not a Dutch standard, and a
deployment elsewhere answers to its own equivalents rather than to these.
Adding a jurisdiction is a registry entry plus the controls that cite it.
