- An ADL 2 operational template whose included archetype reuses the template's
  node ids uploads again, where it was refused with `422` under VCOSU. Node-id
  uniqueness now holds per archetype and restarts at each `C_ARCHETYPE_ROOT`
  (AOM 2 VCOSU, "unique within the archetype"), and on an operational template
  the codes and interior node ids below a root are judged against that
  constituent's entry in `component_terminologies` (OPT 2 `master03-opt_raw`).
  A duplicate node id inside one archetype is still refused.
