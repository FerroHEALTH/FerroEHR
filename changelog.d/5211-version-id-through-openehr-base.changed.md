- The server and the viewer read the parts of an `OBJECT_VERSION_ID` through
  `openehr-base` and its BASE grammar, never by cutting at `::`. A value that
  is no `OBJECT_VERSION_ID` now names no version: the AQL branch refusal, the
  version item tags and the viewer's version numbers leave it alone, a
  reference to a container is read as the whole value (so `uuid::extension`
  no longer names the container `uuid`), and the viewer's deletes refuse it
  before calling the CDR. Every part keeps its case.
