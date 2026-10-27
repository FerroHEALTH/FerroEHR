# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the server alone: the reference catalog and the one
# every lookup falls back to. Each message is one `ServerMsg` of the crate's
# `locale::server` module; a catalog of another language holds the same
# messages in a directory named by its BCP 47 tag beside this one.

no-ehr-id-delete = no EHR id to delete
stored-query-delete-needs-both-qualified = a stored-query delete needs both a qualified name and a version
no-template-id-delete = no template id to delete
row-order = Row order
multimedia-files-stored-over-connection-without = multimedia files are stored over a connection without encryption
access-records-sent-audit-repository-without = access records are sent to the audit repository without encryption
change-events-published-message-broker-without = change events are published to a message broker without encryption
listener-serves-plain-http-address-reachable = a listener serves plain HTTP on an address reachable from the network
requests-served-without-authentication = requests are served without authentication
schema-preparation-runs-clinical-runtime-credential = schema preparation runs on the clinical runtime credential
access-not-recorded-durable-audit-trail = access is not recorded to a durable audit trail
two-domains-reach-same-postgresql-cluster = two domains reach the same PostgreSQL cluster
clinical-demographic-linkage-domains-share-one = the clinical, demographic and linkage domains share one database credential
text-csv-charset-utf-8 = text/csv; charset=utf-8
parameters-must-json-object = parameters must be a JSON object
no-stored-query-run-open-one = no stored query to run: open one from the stored-query list
stored-query-needs-name = the stored query needs a name
not-cdr-any-more-was-already = { $object } is not in the CDR any more: it was already deleted. Reload this screen.
cdr-rejected-delete = The CDR rejected the delete of { $object }: { $message }.
deleting-failed = Deleting { $object } failed: { $error }
viewer-session-expired-before-was-deleted = The viewer session expired before { $object } was deleted: sign in again and retry.
deleting-failed-2 = Deleting { $object } failed: { $other }
column-2 = column { $value0 }
open-2 = Open: { $gaps }.
export-failed = export failed: { $e }
parameters-must-json-object-2 = parameters must be a JSON object: { $e }
unknown-export-format-csv-json = unknown export format `{ $other }` (csv | json)
unparseable-result-set = unparseable RESULT_SET: { $e }
column-3 = column { $value0 }
attachment-filename = attachment; filename="{ $filename }"
was-not-sent-correct-input-retry = { $object } was not sent: { $message }. Correct the input and retry.
cdr-rejected-fix-reported-problem-retry = The CDR rejected { $object }: { $message }. Fix the reported problem and retry.
cdr-answered-nothing-was-saved = The CDR answered { $status } to { $object }: { $message }. Nothing was saved.
parameters-must-json-object-3 = parameters must be a JSON object: { $e }
stored-query-list-json = stored-query list JSON: { $e }
stored-query-json = stored query JSON: { $e }
parameters-must-json-object-4 = parameters must be a JSON object: { $e }
stored-query-result-json = stored-query result JSON: { $e }
conformance-manifest-json = conformance manifest JSON: { $e }
still-referenced-committed-data-so-cdr = { $object } is still referenced by committed data, so the CDR refused the delete ({ $message }). Delete the versions that reference it first, then retry.
session-may-not-delete-sign-admin = This session may not delete { $object } ({ $message }). Sign in with an ADMIN-role account and retry.
cdr-no-longer-accepts-session-so = The CDR no longer accepts this session, so { $object } was not deleted ({ $message }). Sign in again and retry.
sandbox-deployment-deployment-has-not-made = Sandbox deployment: this deployment has not made the production separations and must not hold real patient data.
no-subject-pseudonym-namespace-declared-so = no subject pseudonym namespace is declared, so an EHR subject may carry a real identifier
access-record-audit-queue-cannot-take = an access record the audit queue cannot take is dropped and the request still succeeds
ehr-without-own-access-settings-admits = an EHR without its own access settings admits every caller the role checks let through
role-checks-off-so-every-signed = role checks are off, so every signed-in caller may use every enabled surface, the admin and management endpoints included
changed-cdr-since-screen-loaded-reload = { $object } changed in the CDR since this screen loaded ({ $message }). Reload the latest version, then reapply your change.
cdr-already-holds-conflicting-object-so = The CDR already holds a conflicting object, so { $object } was not created ({ $message }). Open the existing one, or change the identifier and retry.
not-cdr-may-have-been-deleted = { $object } is not in the CDR ({ $message }): it may have been deleted meanwhile. Reload this screen and retry.
cdr-no-longer-accepts-session-so-2 = The CDR no longer accepts this session, so { $object } was not saved ({ $message }). Sign in again and retry.
session-may-not-write-sign-account = This session may not write { $object } ({ $message }). Sign in with an account that carries the required role and retry.
viewer-session-expired-before-was-saved = The viewer session expired before { $object } was saved: sign in again and retry; your input is still on screen.
cdr-unreachable-so-was-not-saved = The CDR is unreachable ({ $message }), so { $object } was not saved. Check the CDR, then retry.
could-not-saved-retry-if-persists = { $object } could not be saved: { $message }. Retry; if it persists, check the viewer logs.
moved-cdr-since-screen-loaded-so = { $object } moved on in the CDR since this screen loaded, so nothing was deleted ({ $message }). Reload the version history and delete the version that is latest now.
cdr-refused-delete-may-already-deleted = The CDR refused to delete { $object }: { $message }. It may already be deleted: reload this screen to see the current history.
not-cdr-may-have-been-deleted-2 = { $object } is not in the CDR ({ $message }): it may have been deleted meanwhile. Reload this screen.
session-may-not-delete-sign-account = This session may not delete { $object } ({ $message }). Sign in with an account that carries the required role and retry.
cdr-no-longer-accepts-session-so-3 = The CDR no longer accepts this session, so nothing was deleted ({ $message }). Sign in again and retry.
not-version-store-stored-query-version = `{ $version }` is not a version to store at: a stored-query version is `major.minor.patch` (for example `1.0.0`). A shorter pattern like `1` or `1.0` selects the latest matching version when READING a query; leave the field empty to let the server assign the version.

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .
