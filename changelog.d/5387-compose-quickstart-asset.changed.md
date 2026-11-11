- The quickstart `docker-compose.yml` each release attaches now carries its
  `.sha256sum`; the README and the book download it with `curl -fsSLO` from
  the latest release and start it with `docker compose up --wait`, and the
  README check refuses a host path the file would read from the repository
  beyond the backup directories.
