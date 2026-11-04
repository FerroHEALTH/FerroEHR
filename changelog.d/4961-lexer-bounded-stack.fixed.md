- `openehr-lang` lexes a long string literal, `--` comment, `<# … #>` plug-in
  block, embedded URI or contained regexp in constant stack in a debug build.
  Before, the `logos` lexer took one stack frame per multi-byte character or
  escape of such a token, so building a template from ADL archetypes could
  overflow a thread's default 2 MiB stack and abort. A 1 MiB token now lexes
  on a 256 KiB thread. The token stream and its spans are unchanged.
