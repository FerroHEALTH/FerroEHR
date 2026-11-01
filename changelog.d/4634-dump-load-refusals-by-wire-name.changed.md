- The admin dump route's refusals name its parameters by their wire names
  and list the accepted values, not the Service Model class names. A bad
  `logical_format` or `compression_format` now reads "`<field>` must be a
  string, one of …" or "`<field>` names no accepted value: … (expected one of
  …)", and a non-integer `segment_split_size` reads "`segment_split_size` must
  be an integer size in kb". The served OpenAPI text for the route no longer
  names `EXPORT_SPEC` or `EXPORT_FORMAT`.
