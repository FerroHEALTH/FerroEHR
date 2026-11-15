- `GET /management/flamegraph` no longer ends the server when it samples while the server signs.
  The profiler walks the frame-pointer chain from its signal handler, checking each read,
  instead of the platform unwinder, which trapped on a sample inside SHA-512 during an Ed25519
  signature. Every build keeps frame pointers (`.cargo/config.toml`).
