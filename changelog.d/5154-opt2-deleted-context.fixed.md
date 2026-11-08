- An ADL2 template that deletes an attribute (`existence matches {0}`), for
  example `COMPOSITION.context`, keeps the prohibition after it compiles to its
  operational template: `openehr-adl`'s `create_opt` now returns a `RawOpt`
  that names each removed attribute beside the OPT, and
  `build_web_template_v2_4` takes that list. A template that deletes `context`
  gets no `/context` node, and a commit that carries one is refused, as it is
  for an OPT 1.4 template that prohibits `context`. An OPT2 uploaded already
  compiled carries no such list and reads a deleted attribute at its
  reference-model existence. Breaking for crate users: `create_opt` returns
  `RawOpt` and `build_web_template_v2_4` takes the prohibited attributes.
