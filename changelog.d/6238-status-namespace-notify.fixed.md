- An `EHR_STATUS` update that moves only the subject's namespace now tells
  every other replica to drop its copy of the EHR's access settings, as a
  moved subject id always did. Each decision already confirmed the subject
  against the stored row, so no access was decided on the old subject; the
  other replica now skips the one refused request it answered before.
