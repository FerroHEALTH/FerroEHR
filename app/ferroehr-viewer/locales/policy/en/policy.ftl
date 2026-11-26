# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the access control pages: the reference catalog
# and the one every lookup falls back to. Each message is one `PolicyMsg`
# of the crate's `locale::policy` module.

history-json = The access policy's history is not a JSON list
policy-json = The access policy is not a JSON object
tab-policy = Policy
tab-versions = Versions
policy-subtitle = The controller's rules for every EHR this CDR keeps, as the CDR decides each request by them.
policy-sections = Access policy sections
policy-forbidden = Your account may not read the access policy.
admin-off = The CDR's admin API is off, so the access policy cannot be read here.
policy-refused = The access policy could not be read.
version-missing = No such version of the access policy.
version-missing-line = The access policy has no version { $version }.
section-classifications = Classifications
section-classifications-intro = Which data carry which sensitivities and confidentiality.
section-rules = Controller rules
section-rules-intro = Who may do what with which data, for which purposes (EHDS Art. 11(3)). A deny refuses; a permit clears the sensitivities it names.
section-catalogue = Catalogue
section-catalogue-intro = The named directives a patient accepts by reference.
section-care = Care relationship facts
section-care-intro = Whether a professional's clinical access needs a care relationship, and how one is read. An attribute left out reads as its strictest default.
section-purpose = Purpose of use
section-purpose-intro = Whether a request must declare its purpose before any rule is read.
section-conflicts = Conflict rules
section-conflicts-intro = The conflicts of interest the professional path admits, and for whom.
section-concealed = Concealed confidentialities
section-concealed-intro = The confidentiality levels whose refused records answer as absent.
section-log-restrictions = Log restrictions
section-log-restrictions-intro = What the access log's subject view leaves out, as the Member State's law lays down (EHDS Art. 9(3)).
section-value-sets = Value sets
section-value-sets-intro = The value set on the terminology server each coded attribute is bound to.
section-other = Other attributes
section-other-intro = The rest of what the policy states.
left-out = None: the policy leaves this out.
left-out-short = Left out
fact-name = Name
fact-controller = Controller
this-policy = This policy
write-first = Write the first version
edit-policy = Edit the policy
none-committed = No access policy is committed
none-committed-line = Nothing is tightened: the CDR decides every request by its configuration alone.
document-title = ACCESS_POLICY, version { $version }
own-contribution = Its own
versions-caption = Versions
versions-intro = Every version of the access policy, newest first. Open one to read it.
at-time-opens = the version in force then
fact-version = Version
fact-committer = Committer
fact-committed = Committed
in-force = In force
starter-name = Access policy
saved = Access policy saved
saved-version = Version { $version } is in force on every replica from the next request.
save-refused = The CDR did not save the access policy.
save-first = Save as version 1
reload-none = Reload
editor-label = The access policy, canonical JSON
edit-intro = The whole ACCESS_POLICY. The save replaces the version you opened; if another administrator saved first, the CDR refuses it.
edit-first-intro = The first ACCESS_POLICY. Fill in the controller, the ORGANISATION whose policy this is, then save.
edit-document = Document
edit-first-subtitle = The first version
edit-subtitle = Opened at version { $version }
edit-title = Edit the access policy
