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

```sh
zed-http examples/widgets.http --name list-widgets --env-file .env.private
zed-http examples/widgets.http --line 8 --use-default-environment --project-root "$PWD"
zed-http examples/widgets.http --name list-widgets --environment staging
```

Repository environments are configured in the versioned
`http-client.environments.json`; copy
`http-client.environments.example.json` to start one. Keep secrets only in
`http-client.environments.private.json`, which is strictly Git-ignored. The private
file overlays the public file for the selected environment:

```json
{
  "version": 1,
  "defaultEnvironment": "local",
  "environments": {
    "local": { "API_BASE_URL": "http://127.0.0.1:3000" },
    "staging": { "API_BASE_URL": "https://staging.example.test" },
    "production": { "API_BASE_URL": "https://api.example.test" }
  }
}
```

`version` must be `1`. `defaultEnvironment` is required if a configuration names
more than one environment and must name one of them. Environment values are plain
string key/value pairs. The selected public values are loaded first, then private
values overlay them. `--environment NAME` selects exactly one name;
`--use-default-environment` selects the default. The two selectors cannot be
combined. With no configuration present, `--use-default-environment` deliberately
keeps the historical behavior and adds no values.

The repository task uses the default and bounds configuration discovery to
`"$ZED_WORKTREE_ROOT"`. Use Zed's runnable menu/task picker for an explicit
environment. For example, add variants like these (all retain the
`http-client-request` tag and inject no values into a command):

```json
[
  {
    "label": "HTTP: run request at cursor (local)",
    "command": "zed-http \"$ZED_FILE\" --line \"$((ZED_ROW + 1))\" --environment \"local\" --project-root \"$ZED_WORKTREE_ROOT\"",
    "tags": ["http-client-request"]
  },
  {
    "label": "HTTP: run request at cursor (staging)",
    "command": "zed-http \"$ZED_FILE\" --line \"$((ZED_ROW + 1))\" --environment \"staging\" --project-root \"$ZED_WORKTREE_ROOT\"",
    "tags": ["http-client-request"]
  },
  {
    "label": "HTTP: run request at cursor (production)",
    "command": "zed-http \"$ZED_FILE\" --line \"$((ZED_ROW + 1))\" --environment \"production\" --project-root \"$ZED_WORKTREE_ROOT\"",
    "tags": ["http-client-request"]
  }
]
```

`--config PATH` and `--private-config PATH` select explicit configuration files.
Otherwise the CLI searches upward from the request file and never goes above
`--project-root`. Values resolve with this precedence: `--var` > `--env-file` >
private configuration > public configuration > process environment.

See the [format and security rules](docs/HTTP-FORMAT-AND-SECURITY.md) and the
[Zed surface actually provided](docs/ZED-SURFACE.md). Requests are never run
automatically.
