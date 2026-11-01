- `POST /definition/template/adl1.4` and `POST /definition/template/adl2`
  answer `400` for every invalid template, the only refusal ITS-REST 1.1.0
  declares for both uploads besides the `409` duplicate. An OPT that breaks
  the OPT 1.4 schema, an ADL 2 source that breaks an AOM2 rule such as VCORM
  or VCARM, and an ADL 2 template whose constituent archetype is not held
  were refused with `422`. The problem keeps its `detail` and, with
  `Prefer: return=representation`, its `errors`. Upgrade note: a client that
  read a `422` from a template upload reads a `400`, with the same `detail`
  and `errors`, under the `about:blank` problem type.
