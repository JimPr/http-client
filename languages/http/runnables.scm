; Expose each HTTP request method as a gutter runnable in Zed.
(
  (request
    method: (method) @run)
  (#any-of? @run "GET" "POST" "PUT" "PATCH" "DELETE" "HEAD" "OPTIONS")
  (#set! tag "http-client-request")
)
