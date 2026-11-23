- An instance restarted under load no longer refuses its first requests
  `503`: it opens every pool connection and waits for its policy listeners
  to connect before it answers ready, so no request after ready waits for a
  connection to open. Restarted under 3,000 reads a second, it refused 122
  to 252 of them while its pools grew.
