- A CONTRIBUTION's `versions[]` entry records its merge inputs: the
  `other_input_version_uids` it states are stored on the committed
  `ORIGINAL_VERSION` and served on every read, where the server used to refuse
  the member with **400**. The uids may name versions the server does not hold;
  an empty list or an entry that is not an `OBJECT_VERSION_ID` is a **422**.
- Other members `UPDATE_VERSION` does not declare, such as `item` or an
  extension member, are accepted and ignored, where the server used to refuse
  them with **400**: the released schema leaves the object open, as it does
  `NewContribution` and `UPDATE_AUDIT`. A `uid` or `contribution` on a version
  entry is still refused with **400**, because the server assigns both, and so
  is a `_type` other than `UPDATE_VERSION` or `ORIGINAL_VERSION`.
