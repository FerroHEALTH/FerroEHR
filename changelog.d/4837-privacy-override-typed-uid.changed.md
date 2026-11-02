- A CONTRIBUTION that writes `ferroehr.access_control.v1` settings whose
  privacy override `uid` is neither a `VERSIONED_COMPOSITION` uid nor a version
  uid whose first part is one is refused with `400`, naming the override. Such
  an override matched no composition, since every composition id is a UUID;
  stored settings that still carry one are read without it.
