- Breaking: `ferroehr config default` is gone with no alias; the annotated
  template is `app/ferroehr/assets/ferroehr.default.toml` in the repository.
  `ferroehr healthcheck --url` and `FERROEHR_HEALTHCHECK_URL` are gone too: the
  probe follows `server.listen` and `server.base_path`. Upgrade: copy the
  template from the repository, and probe another URL with an HTTP client of
  your own.
