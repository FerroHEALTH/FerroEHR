- **`ui-screenshot-guard` judges only the pull request's own commits**
  (#4117). It diffed the base and head commits directly, so a branch behind
  main failed on main's viewer changes and passed on main's screenshot
  refresh. `scripts/checks/ui-screenshot.sh` now reads the paths from the
  merge base (`base...head`), and its `--self-test` runs both cases on a
  branch behind main before every verdict.
