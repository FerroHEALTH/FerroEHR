- **Breaking: `[authz.party_binding.representation]` is removed**, with its `archetype_id` and
  `relationship_codes` (#5446). A representation is recognised by the
  `ferroehr-operations::represents` code it carries in any archetype, and
  names its openEHR `subject_relationship` code itself; the section is now
  an unknown key. The `refused-patient-representative` refusal is gone: a
  `patients` token whose `act` names a representative is admitted while a
  live representation binds them.
