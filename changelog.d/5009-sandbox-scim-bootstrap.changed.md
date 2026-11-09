- The hosted sandbox follows the bootstrap path of party binding: its
  configuration declares the `sandbox-scim` client (`[[scim.client]]`, with
  the development issuer trusted as a `machines` issuer for it), the box's
  new `ferroehr-provision` service provisions the demo `PERSON` through
  `/scim/v2/` after every reset, and `scripts/sandbox/reseed.sh` waits for the
  demo login to bind before it seeds, then checks the committer of the
  marker's `EHR_STATUS` version references a `PERSON`. The deploy probe's new
  `bootstrap` family (`P-BOOTSTRAP-01`) shows the compose quickstart does the
  same from an empty database: the quickstart user is a User of the SCIM
  client, and a commit of the Basic user names that `PERSON`.
