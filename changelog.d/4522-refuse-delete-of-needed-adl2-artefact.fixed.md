- `DELETE /ferroehr/rest/extensions/v1/definition/artefact/adl2/{artefact_id}`
  refuses with `409 Conflict` while another stored ADL 2 artefact does not
  compile without the one deleted (a slot filler, a `use_archetype` reference
  or a specialisation parent, directly or through another artefact), and the
  message names every one of them. Before, the delete succeeded and left each
  template built on the archetype unable to compile.
