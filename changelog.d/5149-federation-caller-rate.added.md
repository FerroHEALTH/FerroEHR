- A federation caller above its agreement's `caller_rate`, a MyHealth@EU
  contact point's included, is answered `429` with `Retry-After` right after
  authentication and before any patient lookup, keyed on the token's subject
  per replica, and every such refusal is counted into the caller's summary
  record.
