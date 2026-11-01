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
access-recorded-without-category-data = every access is recorded without the category of the data it reached
multimedia-files-stored-over-connection-without = multimedia files are stored over a connection without encryption
access-records-sent-audit-repository-without = access records are sent to the audit repository without encryption
change-events-published-message-broker-without = change events are published to a message broker without encryption
listener-serves-plain-http-address-reachable = a listener serves plain HTTP on an address reachable from the network
requests-served-without-authentication = requests are served without authentication
schema-preparation-runs-clinical-runtime-credential = schema preparation runs on the clinical runtime credential
access-not-recorded-durable-audit-trail = access is not recorded to a durable audit trail
two-domains-reach-same-postgresql-cluster = two domains reach the same PostgreSQL cluster
clinical-demographic-domains-share-one = the clinical and demographic domains share one database credential
text-csv-charset-utf-8 = text/csv; charset=utf-8
parameters-must-json-object = parameters must be a JSON object
no-stored-query-run-open-one = no stored query to run: open one from the stored-query list
stored-query-needs-name = the stored query needs a name
column-2 = column { $value0 }
export-failed = export failed: { $e }
parameters-must-json-object-2 = parameters must be a JSON object: { $e }
unknown-export-format-csv-json = unknown export format `{ $other }` (csv | json)
unparseable-result-set = unparseable RESULT_SET: { $e }
column-3 = column { $value0 }
attachment-filename = attachment; filename="{ $filename }"
parameters-must-json-object-3 = parameters must be a JSON object: { $e }
stored-query-list-json = stored-query list JSON: { $e }
stored-query-json = stored query JSON: { $e }
parameters-must-json-object-4 = parameters must be a JSON object: { $e }
stored-query-result-json = stored-query result JSON: { $e }
conformance-manifest-json = conformance manifest JSON: { $e }
no-subject-pseudonym-namespace-declared-so = no subject pseudonym namespace is declared, so an EHR subject may carry a real identifier
access-record-audit-queue-cannot-take = an access record the audit queue cannot take is dropped and the request still succeeds
ehr-without-own-access-settings-admits = an EHR without its own access settings admits every caller the role checks let through
role-checks-off-so-every-signed = role checks are off, so every signed-in caller may use every enabled surface, the admin and management endpoints included
not-version-store-stored-query-version = `{ $version }` is not a version to store at: a stored-query version is `major.minor.patch` (for example `1.0.0`). A shorter pattern like `1` or `1.0` selects the latest matching version when READING a query; leave the field empty to let the server assign the version.

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .
