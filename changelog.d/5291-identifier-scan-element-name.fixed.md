- The identifier scan (`[privacy.identifier_scan]`) reads the `name` of the
  `ELEMENT` that holds a `DV_IDENTIFIER` when the identifier's `type` is
  absent or names neither a person nor an item, with the same word rules. An
  untyped laboratory order number in the `Receiver order identifier` or
  `Requester order identifier` element of `laboratory_test_result.v1` that
  passes the NHS Number check by chance is now stored under the default rule
  set. A person word still wins, so an untyped value in an element named
  `NHS number` or `Patient identifier` stays scanned, and a `type` that names
  a kind takes precedence over the element name.
