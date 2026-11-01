- A `422` validation refusal tells the client to send
  `Prefer: return=representation` only when the request did not send it; a
  request that sent it receives the violations in `errors` and a `detail`
  without the advice.
