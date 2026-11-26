<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The xShare Yellow Button

The xShare Yellow Button lets a person download their health data and share
it once with someone of their choice. The xShare implementation guide
defines both functions (`hl7.eu.fhir.xshare-yb` 0.3.0,
<https://hl7.eu/fhir/ig/xshare-yb/conformance.html>):

- **Download:** the person signs in, selects their data and downloads it in
  a European exchange format that passes that format's validation.
- **One-time Share:** the person creates a SMART Health Link (SMART Health
  Cards and Links 1.0.0) whose manifest serves the selected data to the
  receiver.

The data is the Patient Summary, HL7 IPS 2.0.1, with the HL7 Europe Patient
Summary form on request. This page is for you, the deployer: what each
component does, what you configure in FerroEHR, and what the person sees.

## Who does what

| Component | Role |
|---|---|
| FerroSMART | Signs the person in at an eID and issues their SMART tokens, with the eIDAS level of the login in `acr`. Exchanges the person's token for a token at FerroEHR that FerroBRIDGE reads with (RFC 8693). |
| FerroBRIDGE | Serves `GET /fhir/Patient/{id}/$summary` (Download) and `POST`, `GET` and `DELETE /shl` (One-time Share) to the person's app, writes and validates the summary, and serves each link's manifest to the receiver. |
| FerroEHR | Holds the record. Serves the person's own EHR to the exchanged token and records every access naming the person. |
| FerroTERM | Answers the code lookups the summary needs. |

The system has no app of its own. The person uses any SMART app
that signs them in at FerroSMART with a standalone patient launch, and the
app calls FerroBRIDGE. FerroBRIDGE's
[Yellow Button API guide](https://github.com/FerroHEALTH/FerroBRIDGE/blob/main/website/book/src/integrate/yellow-button.md)
is the reference for those calls.

## The eID providers

The xShare Yellow Button, like every access a person has to their own data,
asks for an electronic identification means recognised under eIDAS, at level
substantial at least (EHDS Art. 16(1)). FerroSMART signs the person in at
the eID your Member State provides, such as DigiD over SAML in the
Netherlands, and writes the level of that login into the token as one of the
eIDAS URIs (`http://eidas.europa.eu/LoA/low`, `.../substantial`,
`.../high`). FerroSMART's own documentation lists the eIDs it connects to and
the minimum level it asks of each client.

## The assurance floor

FerroEHR checks the level again on every request for patient data. Set
`[authz.assurance]` to substantial, or to high where your Member State asks
for that; a Dutch deployment sets high:

```toml
[authz.assurance]
minimum = "substantial"

[authz.assurance.levels]
"http://eidas.europa.eu/LoA/low" = "low"
"http://eidas.europa.eu/LoA/substantial" = "substantial"
"http://eidas.europa.eu/LoA/high" = "high"
```

A request below the floor is refused `401` with the RFC 9470 §3 step-up
challenge, and the app sends the person back to FerroSMART to sign in at a
higher level. The floor holds for professionals too. The whole setup behind
FerroSMART is on [Behind FerroSMART](behind-ferrosmart.md).

## What FerroEHR serves the person

FerroBRIDGE reads the record with the person's own exchanged token, so
FerroEHR applies the person's own-access rules exactly as for their own
login:

- Only the EHR whose subject is the person's `PERSON` answers. Any other EHR
  is refused `403` with `refused-not-subject`.
- A composition the [access delay](access-control.md#delayed-access-by-the-person-ehds-art-33)
  holds is left out of the summary until a professional releases it or its
  limit passes (EHDS Art. 3(3)).
- Every read is an access record naming the person, FerroBRIDGE as the
  client, FerroSMART as the issuer, the level of assurance and the category
  of the data (EHDS Annex II 3.2), kept in the [audit trail](audit.md).

## The link limits

FerroBRIDGE sets the limits of a SMART Health Link. By default:

| Limit | Default |
|---|---|
| Expiry | 24 hours after creation, at most 7 days |
| Answers | 1: the link serves its manifest once |
| Wrong passcodes | 5 over the link's life, then the link is disabled |
| File location | used once, and lapses within an hour at most |

FerroBRIDGE encrypts the summary once under a key it hands back in the link
and never keeps, so it holds no readable copy and cannot recover a lost
link. A link leaves its store whole when it expires, is revoked, has given
its last answer or runs out of passcode attempts. FerroBRIDGE's `[shl]`
section sets the limits and the store.

## Revocation

The person lists their active links (`GET /shl`) and revokes one
(`DELETE /shl/{handle}`). A revoked, expired, used or locked link answers
the receiver `404`. Creating a link, each answer, each refused passcode, the
revocation and the end of a link are access records.

## What the person sees

What the person sees is the app's. Through the API the app can show:

- the sign-in at their eID, and a request to sign in again at a higher
  level when FerroEHR answers with the step-up challenge;
- their patient summary, to save as a file;
- each link as a QR code or a URL, with its label, its expiry, whether it
  has a passcode and how many answers it has left;
- a revoke action on each link.

A result under the access delay is absent from the summary until it is
released, so the app can tell the person that new results may appear after
their professional has discussed them.
