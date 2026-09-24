; Queries are aligned with rest-nvim/tree-sitter-http at the pinned revision.
(method) @function.method

(header
  name: (_) @constant)

(variable_declaration
  name: (identifier) @variable)

(comment
  "@" @keyword
  name: (_) @keyword)

(request
  url: (_) @string.special.url)

(http_version) @constant
(status_code) @number
(status_text) @string

[
  "{{"
  "}}"
] @punctuation.bracket

(header
  ":" @punctuation.delimiter)

[
  (comment)
  (request_separator)
] @comment @spell
