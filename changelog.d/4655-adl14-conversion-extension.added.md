- Two read-only extension routes convert stored ADL 1.4 definitions to ADL 2:
  `GET /ferroehr/rest/extensions/v1/definition/archetype/adl1.4/{archetype_id}/adl2`
  for a stored archetype and
  `GET /ferroehr/rest/extensions/v1/definition/template/adl1.4/{template_id}/adl2`
  for a stored operational template, which converts to one ADL 2 archetype per
  embedded archetype root. Each answers **200** with `application/json`:
  - the ADL 2 source of every root;
  - each root's conversion log and its `conversion_details`, which name what
    the ADL 2 form drops or changes;
  - the slot fills that joined the roots.

  An unknown id answers **404**. A stored source that no longer converts
  answers **422**. No openEHR specification defines the conversion, so both
  routes are FerroEHR's own design.
