- An ADL 1.4 template upload whose XML ends with an element still open now
  answers `400`, as for any document that is not well-formed XML. The
  well-formedness gate stopped at the end of input without checking that
  every element was closed, so a truncated upload answered `422`.
