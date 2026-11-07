- The Helm chart (23.1.0) accepts `bridge.cdr.oauth.tokenEndpoint` for an
  authorization server whose metadata FerroBRIDGE cannot read at
  `bridge.cdr.oauth.issuer`. FerroBRIDGE then requests its client
  credentials token from that endpoint, and `issuer` still names the
  `machines` issuer the CDR trusts.
