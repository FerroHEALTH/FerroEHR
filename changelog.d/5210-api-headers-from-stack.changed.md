- The API's response security headers come from the family server stack
  (`ferrohealth-server` 0.6.2) with the same six headers and values. They now
  also reach the management listener on its own port, and
  `Strict-Transport-Security: max-age=31536000` is sent when FerroEHR or the
  viewer serves TLS itself (`[server.tls]`, RFC 6797 §7.2). The viewer gains
  `Cross-Origin-Resource-Policy: same-site`.
