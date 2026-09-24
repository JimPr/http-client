# Verified Zed surface

As of September 23, 2026, Zed's official documentation confirms that an extension
can declare a language and a Tree-sitter grammar in `extension.toml`. This extension
therefore provides `.http`/`.rest` file association and syntax highlighting based on
`rest-nvim/tree-sitter-http`, pinned by SHA. Its runnable query also supplies a
gutter arrow for each `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, or `OPTIONS`
request method.

Zed tasks receive `ZED_FILE` and `ZED_ROW` and display output in the integrated
terminal. Install `HTTP: run request at cursor` globally in
`~/.config/zed/tasks.json` to make it available in all consumer projects. This is
necessary because the published extension cannot inject its repository-local
`.zed/tasks.json` into other worktrees. The repository task is retained as a
project-specific example.

The global and local task use the exact `http-client-request` tag emitted by the
runnable query; the tag match associates the task with request runnables. Both pass
`"$ZED_FILE"` and `"$((ZED_ROW + 1))"` to `zed-http`, then pass
`--use-default-environment --project-root "$ZED_WORKTREE_ROOT"`. Zed supplies `ZED_ROW` as a
zero-based row, while the CLI expects a one-based line, so the task increments it.
`zed-http` must be installed on `PATH`.

The published extension provides language support, highlighting, and runnable
markers only. It **does not** provide automatic execution, a response panel, or a
WASM command that accesses the network. The gutter arrow invokes the matching
project task; execution remains explicit and the separately installed CLI runs in a
terminal. A request never starts when a file is opened, parsed, or highlighted.

Zed declarative extensions do not support a CTA or a select box for choosing an
environment. This is intentional: choose `local`, `staging`, or `production` by
adding tagged `--environment NAME` task variants and selecting one from the runnable
menu/task picker. The generic task uses the configured default instead. No task
contains environment values or secrets.

## Extension / CLI separation

The repository root is deliberately a pure language extension: it contains
`extension.toml`, the language configuration, and grammar queries, but no Cargo
manifest. Consequently, installing it as a Zed development extension does not
compile a Rust or WASM extension.

The native HTTP client is a distinct Cargo project under `cli/`. It is required only
when executing a request; extension language features and runnable markers work
without it. The only installation method currently guaranteed is from a repository
checkout root:

```sh
cargo install --path cli --locked
```

Cargo's bin directory must be on `PATH`. Zed inherits its `PATH` at startup, so
fully restart Zed after changing `PATH`.

The repository's `.zed/tasks.json` is project-specific example configuration. It
does not reference a repository-specific path: it calls `zed-http` from `PATH`, so
a user can deliberately use it in any project containing `.http` or `.rest` files.
For the multi-project setup, use the global task in `~/.config/zed/tasks.json`
instead. In either location, the task tag must be `http-client-request` so the
gutter runnable can offer it. This is a POSIX shell task.

Before executing the CLI, the task checks that `zed-http` is on `PATH`. If it is
absent, it prints an actionable diagnostic to standard error, exits with code `127`,
and sends no request. This guard never installs or downloads a binary and never
attempts network traffic. After installing from the checkout root and restarting
Zed to inherit the updated `PATH`, run the task again.

Official references:

- <https://zed.dev/docs/extensions/developing-extensions>
- <https://zed.dev/docs/extensions/languages>
- <https://zed.dev/docs/tasks>
- <https://zed.dev/docs/extensions/capabilities>
