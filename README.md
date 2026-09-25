# HTTP Client Files for Zed

This repository contains two separate components:

- the Zed language extension at the repository root (`extension.toml` and
  `languages/`), with no Cargo manifest so that Zed installs it as a language
  extension;
- the native Rust CLI, `zed-http`, under `cli/`, which explicitly executes
  requests from IntelliJ-style `.http` and `.rest` files.

The published extension provides language association, syntax highlighting, and a
runnable gutter arrow beside supported HTTP request methods. It does not include
Rust or WASM code, a request runner, automatic execution, or a response panel.
Install the CLI separately when you want to run requests.

Follow the [user guide](docs/USER-GUIDE.md) to install the CLI, configure Zed, run
the public examples, and select a repository environment.

## Install the CLI

The CLI is required **only to execute requests**; language association,
highlighting, and runnable markers do not require it. The only installation method
currently guaranteed is from a repository checkout root:

```sh
cargo install --path cli --locked
```

This installs `zed-http` into Cargo's bin directory, which must be on `PATH`. Zed
inherits its `PATH` when it starts: fully restart Zed after changing `PATH` so the
task can find the newly installed binary.

PowerShell users can install and make Cargo's usual bin directory available to the
current session with:

```powershell
cargo install --path cli --locked
$env:Path = "$HOME\.cargo\bin;$env:Path"
```

The Zed task shown in this repository is a POSIX shell task; this PowerShell example
does not make that task portable to PowerShell.

Install **HTTP: run request at cursor** in `~/.config/zed/tasks.json` to make it
available in every Zed project. This global task resolves the multi-project case:
the published extension cannot inject the repository's `.zed/tasks.json` into
consumer worktrees. The repository task remains a project-specific example for
users who prefer local configuration.

The global and local task must use the `http-client-request` tag exactly. It matches
the tag emitted by `languages/http/runnables.scm`, allowing the extension's request
runnables to offer this task. Its command must pass `"$ZED_FILE"` and
`"$((ZED_ROW + 1))"` to `zed-http`: Zed's zero-based row must be converted because
the CLI expects a one-based line. Click the gutter arrow, or run **HTTP: run request
at cursor** from the task picker, to explicitly launch it. The task calls `zed-http`
from `PATH`; the integrated terminal displays the final URL, status, duration, size,
headers, and formatted body.

### Missing CLI diagnostic

Before it runs a request, the POSIX task checks whether `zed-http` is on `PATH`. If
it is absent, it writes an actionable installation message to standard error, exits
with code `127`, and sends no request. It does not install, download, or otherwise
attempt to obtain the CLI. Install the CLI from the checkout root, ensure the
updated `PATH` is inherited by a restarted Zed process, then run the task again.

## Install the development Zed extension

Select this repository's root directory as a development extension. It deliberately
contains no `Cargo.toml`, so Zed treats it as the language extension declared by
`extension.toml` and does not attempt to compile a Rust or WASM extension. The CLI
remains an independent Cargo project under `cli/`.

## Environments and usage

The public examples in [`examples/github-api.http`](examples/github-api.http) require
no environment configuration:

```sh
zed-http examples/github-api.http --name get-environment-example
zed-http examples/github-api.http --name render-markdown
```

The first request downloads this repository's example configuration from GitHub. The
second sends Markdown to GitHub's public rendering endpoint.

Repository environments are configured in the versioned
`http-client.environments.json`; copy
`http-client.environments.example.json` to start one. Keep secrets only in
`http-client.environments.private.json`, which is strictly Git-ignored. The private
file overlays the public file for the selected environment:

```json
{
  "version": 2,
  "environments": [
    { "name": "local", "variables": { "API_BASE_URL": "http://127.0.0.1:3000" } },
    { "name": "staging", "variables": { "API_BASE_URL": "https://staging.example.test" } },
    { "name": "production", "variables": { "API_BASE_URL": "https://api.example.test" } }
  ]
}
```

Only version `2` is accepted. Unknown fields, duplicate or empty names, and
non-string variable values are rejected. Whenever a
catalogue exists, `--select-environment` always displays its ordered terminal picker.
Pressing Enter chooses the first item only for that request; no choice is persisted.
With no configuration, it does not read stdin or render a picker, preserving
historical behavior and supplying no values. `--environment NAME` remains available
for direct technical CLI use.

The single repository task passes `--select-environment` and bounds discovery to
`"$ZED_WORKTREE_ROOT"`. Zed has no native select box, so the choice happens in the
integrated terminal for every request. No task contains an environment name, value,
or secret.

`--config PATH` and `--private-config PATH` select explicit public and private JSON
environment configuration files.
Otherwise the CLI searches upward from the request file and never goes above
`--project-root`. Values resolve with this precedence: `--var` > inline declaration
> private JSON configuration > public JSON configuration > process environment.

Inline declarations have the form `@name = value` and appear before a request:

```http
@request_path = /status
# @name get-status
GET {{API_BASE_URL}}{{request_path}}
```

They have forward lexical scope: a declaration applies to following requests,
continues across `###`, and a later declaration does not alter an earlier request.
The value is literal, including quotes and any `{{...}}` text. Inline declarations
in request bodies are body content rather than variable declarations. They are
stored in the `.http` file, so credentials do not belong in them.

See the [format and security rules](docs/HTTP-FORMAT-AND-SECURITY.md) and the
[Zed surface actually provided](docs/ZED-SURFACE.md). Requests are never run
automatically.
