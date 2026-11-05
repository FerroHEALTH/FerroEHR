- A party of a mapped template is validated against that template on every
  demographic write: a `PERSON` or `GROUP` whose `archetype_details.template_id`
  names a template of the shared party mapping, and a `PARTY_RELATIONSHIP` of
  its membership archetype. A `532|complete|` version must meet every
  constraint, and a `553|incomplete|` one may miss mandatory nodes but nothing
  it carries may be wrong (RM common master06 §Incomplete Content); the refusal
  is a `422` naming each failing path. Every other party keeps the
  reference-model checks alone, so parties created through the Demographic API
  under other templates are accepted as before.
