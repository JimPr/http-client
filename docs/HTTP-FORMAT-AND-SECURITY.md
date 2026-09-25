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
zed-http requests.http --name status --select-environment
zed-http requests.http --index 0
zed-http requests.http --line 12
```

`--index` is zero-based; `--line` is a human-facing, one-based line number. Without
a selector, only the first request is executed.

`{{NAME}}` variables are resolved in this order: `--var NAME=VALUE`, visible inline
declarations, the selected private JSON environment, the selected public JSON
environment, then the process environment. A missing variable is an explicit error.

## Inline variables

An inline declaration has this form before a request:

```http
@request_path = /status
@display_value = "literal quotes remain"
GET {{API_BASE_URL}}{{request_path}}
```

The declaration's name is an identifier. Its value is literal: surrounding quotes
remain in the value, and `{{NAME}}` inside a declaration is not resolved again.
The declaration applies to requests that follow it, including requests after `###`.
Redeclaring a name changes the value for later requests only. A section containing
only declarations changes the values visible to subsequent sections. Lines that
start with `@` in a request body remain body content.

For a selected request, precedence is `--var NAME=VALUE`, then its visible inline
declarations, selected private JSON environment values, selected public JSON
environment values, and the process environment.

## Repository environments

Use `http-client.environments.json` for non-secret, versioned values and
`http-client.environments.private.json` for local secrets. Both use:

```json
{
  "version": 2,
  "environments": [
    {
      "name": "local",
      "variables": { "API_BASE_URL": "http://127.0.0.1:3000" }
    }
  ]
}
```

Only version `2` is accepted. Names must be non-empty and unique, variables must be
string-to-string maps, and unknown fields are rejected; persistent defaults are not
supported. With a catalogue, `--select-environment` always prints the numbered list
and Enter chooses its first item for the current request. With no configuration it
remains non-interactive.
`--environment NAME` remains available for technical CLI invocation. The private
environment file overlays values only.

Configuration is loaded at each CLI launch. The success output includes only
`Environment: NAME`, never variable values; it omits that line when no configuration
is selected.

`--config PATH` and `--private-config PATH` select explicit public and private JSON
environment configuration files. Without them, discovery starts beside the request
and walks upward, stopping at `--project-root PATH`. The generic Zed task passes
the worktree root specifically to prevent a parent checkout's configuration from
being used.

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
- Inline declaration values are stored in the request file. Keep secrets in an
  ignored private JSON environment file instead of inline declarations.
- The private environment file is Git-ignored. Do not put credentials in the public
  file or in task commands; the included environment example is deliberately
  secret-free.
Never commit secrets. The private JSON environment file is the only ignored external
configuration file supported by the CLI.
