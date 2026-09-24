# `.http` format and security

## Supported format

A document contains one or more requests separated by a `###` line. A request can
be named with `# @name name`. Supported methods are `GET`, `POST`, `PUT`, `PATCH`,
`DELETE`, `HEAD`, and `OPTIONS`, with `http` or `https` URLs, headers, and text or
JSON bodies.

```http
# @name status
GET {{API_BASE_URL}}/status
Authorization: Bearer {{API_TOKEN}}
```

Select a request explicitly:

```sh
zed-http requests.http --name status --env-file .env.private
zed-http requests.http --index 0
zed-http requests.http --line 12
```

`--index` is zero-based; `--line` is a human-facing, one-based line number. Without
a selector, only the first request is executed.

`{{NAME}}` variables are resolved in this order: `--var NAME=VALUE`, the file
passed through `--env-file`, the selected private environment file, the selected
public environment file, then the process environment. A missing variable is an
explicit error. Dotenv files accept `NAME=VALUE`, `export NAME=VALUE`, and simply
quoted values.

## Repository environments

Use `http-client.environments.json` for non-secret, versioned values and
`http-client.environments.private.json` for local secrets. Both use:

```json
{
  "version": 1,
  "defaultEnvironment": "local",
  "environments": {
    "local": { "API_BASE_URL": "http://127.0.0.1:3000" }
  }
}
```

Only version `1` is accepted. Each environment is a string key/value map.
`defaultEnvironment` is mandatory for more than one environment and must name an
environment. Select one with `--environment NAME`, or select the default with
`--use-default-environment`; the selectors are mutually exclusive. An explicit
name, an absent/invalid default, malformed JSON, an invalid model, or an unsupported
version fails before an HTTP request is attempted. With no configuration,
`--use-default-environment` supplies no values and retains the old CLI behavior.

Without explicit `--config PATH` and `--private-config PATH`, discovery starts beside
the request and walks upward, stopping at `--project-root PATH`. The generic Zed
task passes the worktree root specifically to prevent a parent checkout's
configuration from being used.

## Security

- Execution is always explicit through the CLI or a Zed task.
- No script embedded in a `.http` file is interpreted.
- TLS validation is enabled by default; redirects are limited to 10.
- The default maximum timeout is 30 seconds; adjust it with
  `--timeout-seconds`.
- The response body is capped at 1 MiB by default with `--max-response-bytes`; any
  truncation is reported.
- Output contains the final URL, status, duration, size, headers, and body. JSON is
  formatted; binary bodies are deliberately omitted.
- Variable values, including tokens, are never written to CLI diagnostics.
- The private environment file is Git-ignored. Do not put credentials in the public
  file or in task commands; the included environment example is deliberately
  secret-free.

Never commit secrets. `.env` and `.env.*` are ignored, while `.env.example` is kept
as a secret-free template.
