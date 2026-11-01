- A directory create or update whose FOLDER carries an empty `folders` array,
  at the root or in a sub-folder, is refused with `400` and commits nothing.
  The ITS-JSON FOLDER schema gives `folders` `minItems: 1`, and the canonical
  JSON reader now refuses the empty array the same way in every document that
  carries a FOLDER, a CONTRIBUTION included. The viewer leaves `folders` out of
  a folder with no sub-folders.
