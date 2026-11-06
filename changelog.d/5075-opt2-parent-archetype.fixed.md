- An OPT2 built from a template that specialises a published archetype now
  carries that archetype, by its full id, as `parent_archetype_id`, as the
  `OPERATIONAL_TEMPLATE` invariant `is_specialised` requires (AOM2 §3.6.6), so
  a reader of the OPT finds the archetype its root specialises. The
  ADL form of an OPT still prints no `specialize` section (OPT2 §Artefact
  Structure), so the parent travels in the canonical JSON form.
