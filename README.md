# HTTP Client Files for Zed

This repository contains two separate components:

- the Zed language extension at the repository root (`extension.toml` and
  `languages/`), with no Cargo manifest so that Zed installs it as a language
  extension;
- the native Rust CLI, `zed-http`, under `cli/`, which explicitly executes
  requests from IntelliJ-style `.http` and `.rest` files.

The published extension provides language association and syntax highlighting only.
It does not include Rust or WASM code, a request runner, an editor button, or a
response panel. Install the CLI separately when you want to run requests.

## Install the CLI

```sh
cargo install --path cli --locked
```

The binary is then available on `PATH`. The repository's `.zed/tasks.json` is
project-specific example configuration. It is not activated or injected into users'
worktrees by the published extension. To use it in another project, copy it
deliberately into that project's `.zed/tasks.json`. In Zed, run
**HTTP: run request at cursor** from the task picker. The task calls `zed-http` from
`PATH`; the integrated terminal displays the final URL, status, duration, size,
headers, and formatted body.

## Install the development Zed extension

Select this repository's root directory as a development extension. It deliberately
contains no `Cargo.toml`, so Zed treats it as the language extension declared by
`extension.toml` and does not attempt to compile a Rust or WASM extension. The CLI
remains an independent Cargo project under `cli/`.

## Usage

```sh
zed-http examples/widgets.http --name list-widgets --env-file .env.private
zed-http examples/widgets.http --line 8 --timeout-seconds 10
```

See the [format and security rules](docs/HTTP-FORMAT-AND-SECURITY.md) and the
[Zed surface actually provided](docs/ZED-SURFACE.md). Requests are never run
automatically.
