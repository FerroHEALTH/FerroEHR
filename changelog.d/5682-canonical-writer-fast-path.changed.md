- The RFC 8785 canonical form that signs a version and chains each access-log
  entry is written faster: strings are scanned by byte and copied whole where
  nothing needs escaping, integers within 2^53 skip the floating-point
  formatter, names whose first difference is ASCII are sorted by their bytes,
  and every object of a document shares one member buffer. The bytes are the
  same as before, so stored digests and signatures still verify. In the
  `openehr-rm` `canonical_form` bench an access-log-shaped record
  canonicalises about a quarter faster and a composition about a third faster.
