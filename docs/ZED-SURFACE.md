# Verified Zed surface

Zed extensions can declare a language and a Tree-sitter grammar in
`extension.toml`. This extension therefore provides `.http` and `.rest` file
association, syntax highlighting based on `rest-nvim/tree-sitter-http`, and a
runnable gutter arrow for each supported HTTP request method.

Zed tasks receive `ZED_FILE` and `ZED_ROW` and display their output in the
integrated terminal. Install **HTTP: run request at cursor** globally in
`~/.config/zed/tasks.json` to make it available in all consumer projects. The
published extension cannot inject its repository-local `.zed/tasks.json` into other
worktrees.

The task uses the `http-client-request` tag emitted by the runnable query. It passes
`"$ZED_FILE"`, the one-based line `"$((ZED_ROW + 1))"`, and
`--select-environment --project-root "$ZED_WORKTREE_ROOT"` to `zed-http`.

When an environment catalogue exists, the CLI reads it on every launch and displays
its ordered list in the integrated terminal. Pressing Enter selects the first entry
for that request only; no environment choice is persisted. Without a catalogue, the
task launches without interaction. Zed declarative extensions cannot add a native
select box or an environment indicator to the editor UI.

The published extension provides language support, highlighting, and runnable
markers only. It does not provide automatic execution, a response panel, or a WASM
command that accesses the network. Clicking the gutter arrow invokes the explicit
task, which runs the separately installed CLI in a terminal.

## Extension / CLI separation

The repository root is deliberately a pure language extension: it contains
`extension.toml`, language configuration, and grammar queries, but no root Cargo
manifest. Installing it as a development extension does not compile a Rust or WASM
extension.

The native HTTP client is a distinct Cargo project under `cli/`. It is required only
to execute requests. Install it from the repository checkout root:

```sh
cargo install --path cli --locked
```

Cargo's bin directory must be on `PATH`. Zed inherits its `PATH` at startup, so
restart Zed after changing it.

The repository's `.zed/tasks.json` is a project-specific task example. For a
multi-project setup, add the same task to `~/.config/zed/tasks.json`. The task checks
that `zed-http` is on `PATH`; if it is absent, it prints an actionable diagnostic,
exits with code `127`, and sends no request.

Official references:

- <https://zed.dev/docs/extensions/developing-extensions>
- <https://zed.dev/docs/extensions/languages>
- <https://zed.dev/docs/tasks>
- <https://zed.dev/docs/extensions/capabilities>
