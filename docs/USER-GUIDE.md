# How to run HTTP requests in Zed

## Prerequisites

- Zed is installed.
- The **HTTP Client Files** extension is installed from the Zed Extensions panel.
- Rust and Cargo are installed to install the `zed-http` runner.

## Steps

### 1. Install the runner

Clone this repository, then install the runner from its root:

```sh
git clone https://github.com/JimPr/http-client.git
cd http-client
cargo install --path cli --locked
```

The command installs `zed-http` in `~/.cargo/bin`. Add that directory to your
`PATH` if needed, then restart Zed:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

### 2. Add the Zed task

Open the Command Palette and run **`zed: open tasks`**. Add the following task to
your global tasks file:

```json
[
  {
    "label": "HTTP: run request at cursor",
    "command": "if ! command -v zed-http >/dev/null 2>&1; then\n  printf '%s\\n' 'ERROR: zed-http was not found on PATH. It is required to run HTTP requests.' 'Install it from the repository checkout root with: cargo install --path cli --locked' 'Documentation: https://github.com/JimPr/http-client#install-the-cli' 'No request was sent.' >&2\n  exit 127\nfi\nzed-http \"$ZED_FILE\" --line \"$((ZED_ROW + 1))\" --select-environment --project-root \"$ZED_WORKTREE_ROOT\"",
    "tags": ["http-client-request"],
    "save": "current",
    "use_new_terminal": true,
    "allow_concurrent_runs": true,
    "reveal": "always",
    "show_command": false
  }
]
```

> **Note**: Merge this entry with existing tasks. To enable it in one project only,
> add the same task to `<project-root>/.zed/tasks.json` instead.

### 3. Run a request

Create or open a `.http` file:

```http
# @name download-example
@raw_github = https://raw.githubusercontent.com
GET {{raw_github}}/JimPr/http-client/main/http-client.environments.example.json
Accept: application/json
```

Place the cursor on the `GET` line and click the gutter arrow. Select
**HTTP: run request at cursor**. The integrated terminal shows the URL, status,
headers, and response body.

You can also open
[`examples/github-api.http`](../examples/github-api.http) from the cloned
repository. It includes the public `GET` request above and a public GitHub `POST`
request.

### 4. Configure project environments

Create `http-client.environments.json` in the root of the project containing your
`.http` files:

```json
{
  "version": 2,
  "environments": [
    {
      "name": "local",
      "variables": {
        "API_BASE_URL": "http://localhost:8080"
      }
    },
    {
      "name": "staging",
      "variables": {
        "API_BASE_URL": "https://api.staging.example.test"
      }
    }
  ]
}
```

Run a request again. The integrated terminal lists the environments. Enter a number
to select one, or press <kbd>Enter</kbd> to select the first item for that request.

> **Note**: Without `http-client.environments.json`, no picker is displayed and the
> request runs directly.

### 5. Keep secrets out of Git

Create `http-client.environments.private.json` in the same project root for local
tokens and other secrets:

```json
{
  "version": 2,
  "environments": [
    {
      "name": "staging",
      "variables": {
        "API_TOKEN": "local-value"
      }
    }
  ]
}
```

Its values override the selected public environment and the file is ignored by Git.
Use the variables in a request:

```http
GET {{API_BASE_URL}}/status
Authorization: Bearer {{API_TOKEN}}
```

## Verification

After a request finishes, the integrated terminal shows the selected environment,
the final URL, the HTTP status, headers, and the response body.

## Common problems

- **`zed-http was not found on PATH`**: Add `~/.cargo/bin` to `PATH`, then restart
  Zed.
- **No environment picker is shown**: Create `http-client.environments.json` in the
  worktree that contains the `.http` file.
- **A variable is not defined**: Add it to the selected environment, the private
  environment file, or an inline declaration before the request, such as
  `@name = value`.
