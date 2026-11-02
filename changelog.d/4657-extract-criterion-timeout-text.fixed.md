- When an `EXTRACT_SPEC` criterion's AQL runs past the query time budget, the `400`
  body now states the timeout as plain text. It used to start with a control-character
  marker that was only ever meant for the server itself.
