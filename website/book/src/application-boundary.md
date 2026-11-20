<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# What the CDR leaves to the application

FerroEHR decides every access to an EHR from facts it stores: care
relationships, memberships, representations, a record's confidentiality, the
controller's exclusions and the controller's access policy. It does not
decide those facts. The openEHR Architecture Overview puts that work outside
the EHR: the evaluation of "care deliverer" into the identities of the staff
on a ward "must be done in each care delivery environment, not in the EHR"
(BASE Architecture Overview, Security, The Problem of Roles), and privacy
levels are "defined by standards or agreements within jurisdictions of use"
(Security Policy, Privacy). The EHR Information Model makes the CDR the place
where "all access decisions to data in the EHR must be made in accordance with
the policies and rules" of `EHR_ACCESS`, and nothing more.

This page lists what your applications, your identity provider and your
deployment own, and what each uses in FerroEHR instead.

| Left to the application | Who owns it | What it uses in FerroEHR |
|---|---|---|
| Country and jurisdiction content: lists of professional roles and their names, organisation types, kinship codes, identifier systems, default periods of a relationship | the deployment, on its terminology server | the value-set bindings of the access policy ([Care relationship facts](operational-policy.md#care-relationship-facts)) and its relationship periods; FerroEHR ships no such list of its own |
| Deciding who represents a patient: from which age a minor decides alone, whether a person can decide for themselves, which legal basis applies, what happens when the patient dies | the care delivery environment, under its national law | a stored representation, a `PARTY_RELATIONSHIP` of its own with its basis, its provider and its period ([Representatives in the patient portal](operational-policy.md#representatives-in-the-patient-portal)). You end it by closing its period |
| Deciding who is in a patient's care: admissions, referrals, registrations with a practice, rosters and cover | the admission, rostering or practice system | stored `under_care_of` relationships, each its own `PARTY_RELATIONSHIP` with a role and a provider, naming a professional, a role, a team or a unit, and team memberships ([Caller facts](operational-policy.md#caller-facts-roles-and-care-relationships)); the relationship resources read and write them by subject and by carer ([The care relationship resources](operational-policy.md#the-care-relationship-resources)) |
| Households, family files and family plans | the application that works with families | care relationships to a group and representation relationships; the controller's exclusions and restrictions that bind a representative (planned, #6077) and confidentiality labels on single compositions (planned, #6057) |
| Worklists: a professional's caseload list, a list of relationships due for review | the client application | AQL, narrowed by the `caseload` and `caseload_roles` request members (planned, #6056), and the reverse reading of the relationship resources ([The care relationship resources](operational-policy.md#the-care-relationship-resources)) |
| Matching a staff member's person to a patient's person, so a conflict of interest can be seen | the master patient index or the demographic service | the person identifiers you store and a `related_to` fact; the refusal of the professional path on a conflict is planned (#6077) |
| Screens and workflows for any of the above | the client application | the REST resources over the stored facts |

A fact you store takes effect on the next request: FerroEHR confirms every
relationship it decides on against the database for that request.

The design behind this page is the draft access control package under
`docs/specifications/access-control/` in the repository, which states the
same boundary for any openEHR CDR.
