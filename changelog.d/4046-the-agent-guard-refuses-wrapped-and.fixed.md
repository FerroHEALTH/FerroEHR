- **The agent guard refuses wrapped and re-ordered spellings of a force push
  or a recursive delete** (#4046). `.claude/hooks/block_dangerous.sh` let
  through `git -C . push --force … main`, `/usr/bin/git push`, `\rm -rf`,
  `rm --recursive --force`, `bash -c "rm -rf …"`, `find … -delete` and similar.
  It now normalises the command first (a path or backslash before the binary,
  a launcher such as `env` or `sudo`, a `sh -c` or `eval` wrapper, git's global
  options before the verb), judges every `rm` target and every deleting `find`
  starting point against `/tmp`, and refuses a push that deletes `main` or
  `master`. `block_dangerous_test.sh` runs each spelling.
