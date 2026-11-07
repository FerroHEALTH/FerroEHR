- The published server, viewer and PostgreSQL images take their
  `org.opencontainers.image.base.name` and `base.digest` labels and
  annotations from the runtime stage's digest-pinned `FROM` line at build
  time, so a base-image bump changes that one line. An image built directly
  from a Dockerfile no longer carries these two labels.
