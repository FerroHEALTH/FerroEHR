- The viewer's ADL 1.4 template upload opens the stored template's page, read
  from the `ETag` (else the `Location`) of the CDR's `201`, where it went to an
  address with no template id. A refused upload names each violation the CDR
  found, its rule code or path and the CDR's words for it, as the ADL 2 upload
  does: the upload asks for `Prefer: return=representation`, the preference
  under which ITS-REST lets a `4xx` carry its error details.
