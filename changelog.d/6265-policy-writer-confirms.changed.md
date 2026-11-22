- The instance that writes a policy register or an access policy version
  confirms its own policy copies at the revision its write produced, read
  inside that write, so its next read of a COMPOSITION or an EHR is no longer
  run a second time.
