- `openehr-sdt` gives the `/context` node it synthesizes in a Web Template the
  `min` of the template's existence on `COMPOSITION.context`, and `0` where the
  template leaves `context` unconstrained, since the RM makes `context`
  optional (`0..1`). A client following the Web Template no longer treats the
  context as mandatory when the template does not require it, and the
  `required` template example of such a template carries no `context`.
