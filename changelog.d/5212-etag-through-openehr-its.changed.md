- `ETag` and `If-Match` values are read through `openehr-its`'s RFC 9110
  entity tag on the server and in the viewer. The server accepts the same
  forms as before. The viewer reads its CDR's `ETag` strictly: a value that
  is no entity tag names no version.
