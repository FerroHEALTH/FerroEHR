# CRA Annex VII 5: standards applied, and the solutions adopted

CRA Annex VII point 5 (`docs/law/eu/cra/text.html`) asks for "a list of the
harmonised standards applied in full or in part the references of which have
been published in the Official Journal of the European Union, common
specifications as set out in Article 27 of this Regulation or European
cybersecurity certification schemes [...] and, where those harmonised
standards, common specifications or European cybersecurity certification
schemes have not been applied, descriptions of the solutions adopted to meet
the essential cybersecurity requirements set out in Parts I and II of Annex
I, including a list of other relevant technical specifications applied".

## Harmonised standards, common specifications, certification schemes

FerroEHR applies none: no harmonised standard under the CRA, no common
specification under CRA Art. 27 and no European cybersecurity certification
scheme.

## The solutions adopted

Described point by point for Annex I Part I(2) and Part II in the
[risk assessment](../../website/book/src/compliance/cra-risk-assessment.md#annex-i-part-i2-point-by-point),
each with the files that implement it.

## Other technical specifications applied

| Specification | Where |
|---|---|
| TLS 1.3 (rustls), mutual TLS | `[server.tls]`; [server configuration](../../website/book/src/installation/config-server.md) |
| OAuth 2.0 bearer tokens, JWT (RFC 7519), the JWT best current practice (RFC 8725), OpenID Connect discovery | [authentication](../../website/book/src/security.md#authentication) |
| SMART App Launch, with PKCE (RFC 7636) | [SMART App Launch](../../website/book/src/smart-app-launch.md) |
| IHE ATNA (ITI-19, ITI-20, ITI-81), IHE BALP, DICOM PS3.15 §A.5 audit messages, FHIR R4 `AuditEvent`, syslog (RFC 5424) | [audit trail](../../website/book/src/audit.md) |
| SLSA Build Level 3 provenance, Sigstore bundles | [verifying releases](../../website/book/src/verifying-releases.md#what-slsa-level-each-artifact-reaches) |
| CycloneDX and SPDX software bills of materials | [three SBOMs](../../website/book/src/verifying-releases.md#three-sboms-three-questions) |
| OpenVEX | `security/vex/` |
| `security.txt` (RFC 9116) | `website/landing/.well-known/security.txt` |
| OpenSSF Scorecard | `.github/workflows/scorecard.yml` |

The openEHR specifications FerroEHR implements are on the
[information sheet](../../website/book/src/compliance/information-sheet.md#e-standards-formats-and-specifications).

State: available.
