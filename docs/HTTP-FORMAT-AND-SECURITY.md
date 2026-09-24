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
passed through `--env-file`, then the process environment. A missing variable is an
explicit error. Dotenv files accept `NAME=VALUE`, `export NAME=VALUE`, and simply
quoted values.

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

Never commit secrets. `.env` and `.env.*` are ignored, while `.env.example` is kept
as a secret-free template.
