# Zed integration surface

The extension supplies language association, syntax highlighting, and runnable
markers for `.http` and `.rest` request files. It does not execute requests by
itself and does not create a response panel.

Execution is explicit: install the separate `zed-http` CLI and configure the
`HTTP: run request at cursor` task documented in the [user guide](USER-GUIDE.md).
The task passes the active worktree through `--project-root`, keeps concurrent runs
enabled, and does not ask Zed to open files or buffers. After execution, its
terminal output ends with the absolute path to the persisted response body; open
that path in Zed if desired.
