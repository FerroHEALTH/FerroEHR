- A CONTRIBUTION's `versions[]` entry accepts and ignores the members
  `UPDATE_VERSION` does not declare, such as `contribution`, `item`,
  `other_input_version_uids` or an extension member, where the server used to
  refuse them with **400**: the released schema leaves the object open, as it
  does `NewContribution` and `UPDATE_AUDIT`. A member that states its own
  `uid` is still refused with **400**, because the server assigns the identity
  of each version it commits, and so is a `_type` other than `UPDATE_VERSION`
  or `ORIGINAL_VERSION`.
