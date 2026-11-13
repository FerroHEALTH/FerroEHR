- A specialised ADL 2 archetype whose differential path sits on an attribute
  below the root (`ELEMENT[at0002] matches { /value[at9001]/defining_code
  matches {...} }`) is read from the object that owns the attribute, as AOM2
  `C_ATTRIBUTE.path` defines it. The upload no longer refuses such a path
  with VDIFP when the flat parent carries it, and the flat form overlays the
  constraint on the node the path names instead of adding it as a new
  attribute of the owning object.
