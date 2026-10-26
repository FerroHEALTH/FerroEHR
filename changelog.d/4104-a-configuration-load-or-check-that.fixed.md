- **A configuration load or check that fails keeps its cause** (#4104).
  Fifteen sites in the server's commands turned the failure into a string,
  so the error chain ended at the message; they now carry the source error.
