# Verified Zed surface

As of September 23, 2026, Zed's official documentation confirms that an extension
can declare a language and a Tree-sitter grammar in `extension.toml`. This extension
therefore provides `.http`/`.rest` file association and syntax highlighting based on
`rest-nvim/tree-sitter-http`, pinned by SHA.

Zed also supports project tasks in `.zed/tasks.json`. They receive `ZED_FILE` and
`ZED_ROW` and display output in the integrated terminal. The example
`HTTP: run request at cursor` task calls the local `zed-http` binary with the cursor
line converted from Zed's line index to the CLI's human-facing, one-based line
number.

The published extension provides language support and highlighting only. It **does
not** provide an in-editor button, response panel, or a WASM command that accesses
the network. This is deliberate: the documented and reliable execution point is an
explicit task that launches the separately installed CLI in a terminal. A request
never starts when a file is opened, parsed, or highlighted.

## Extension / CLI separation

The repository root is deliberately a pure language extension: it contains
`extension.toml`, the language configuration, and grammar queries, but no Cargo
manifest. Consequently, installing it as a Zed development extension does not
compile a Rust or WASM extension.

The native HTTP client is a distinct Cargo project under `cli/`. Install it with:

```sh
cargo install --path cli --locked
```

The repository's `.zed/tasks.json` is project-specific example configuration. It is
not activated or injected into users' worktrees by the published extension. It does
not reference a repository-specific path: it calls `zed-http` from `PATH`, so a
user can deliberately copy it into any project containing `.http` or `.rest` files.

Official references:

- <https://zed.dev/docs/extensions/developing-extensions>
- <https://zed.dev/docs/extensions/languages>
- <https://zed.dev/docs/tasks>
- <https://zed.dev/docs/extensions/capabilities>
