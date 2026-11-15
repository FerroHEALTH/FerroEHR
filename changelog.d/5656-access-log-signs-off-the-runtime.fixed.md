- With an RSA signing key, requests no longer wait behind the audit drain
  while it signs access-log links. Each RSA signature takes milliseconds, and
  the drain made them on a runtime worker, so a template upload took about
  three times as long under load. The links are now signed on the blocking
  pool. What is signed and stored is unchanged.
