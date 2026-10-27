- **Breaking: the viewer listens on `server.listen`, not `LEPTOS_SITE_ADDR`.**
  The viewer reads the family's `[server]` table from `ferrohealth-server`
  0.3.5, defaulting to `127.0.0.1:3000` (the image sets `0.0.0.0:3000` through
  `FERROEHR_VIEWER__SERVER__LISTEN`). A `LEPTOS_SITE_ADDR` that names another
  address refuses the start and names the key that replaced it; unset it.
  `server.base_path` and `server.caller_rate` are refused, since the viewer's
  pages are served at `/` and it mounts no per-caller limit.
- **Breaking: `ferroehr-viewer healthcheck` asks `/health/readiness`** over the
  loopback (over TLS when `server.tls` is set) and exits `0` only on `200`,
  where it asked `/login` at `LEPTOS_SITE_ADDR` and took any `2xx` or `3xx`. A
  draining viewer is now unhealthy. The Helm chart's viewer probes ask
  `/health/liveness` and `/health/readiness`, where all three asked `/`.
- **Breaking: the viewer's browser security headers are the family console's.**
  `Referrer-Policy` is `no-referrer` (was `strict-origin-when-cross-origin`),
  `Cross-Origin-Opener-Policy: same-origin` is new, and the content-hashed
  bundle under `/pkg/` is `Cache-Control: no-cache`, revalidated on each load,
  where it was `public, max-age=31536000, immutable`. Every document stays
  `no-store` under the nonce Content-Security-Policy.
- **Breaking: every request to the viewer that is not a safe method passes the
  family same-origin check**, server function calls and the result export
  included, where only page form posts compared `Origin` with `Host`. The
  browser's `Sec-Fetch-Site: same-origin` passes; without it `Origin` or
  `Referer` must name `auth.oidc.public_base_url`, and a request that shows
  neither is refused `403`. A post without a session now returns to the page
  it was posted to after the sign-in, rather than to the page its `Referer`
  named.
- **Breaking: the viewer's CDR and OIDC clients speak TLS 1.3 at least** by
  default, as every family client does; `cdr.tls.min_version` and
  `auth.oidc.tls.min_version` set to `"1.2"` lower it. A request now runs under
  the stack's `server.request_timeout_ms` (60 s) and `server.body_limit_bytes`
  (16 MiB), and a form post's ceiling is that limit.
