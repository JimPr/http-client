use std::{collections::BTreeMap, fs, time::Duration};

use zed_http_runner::{
    execute, load_env_file, parse_document, render_response, resolve_request, select_request,
    HttpMethod, ResponseData, Selection,
};

#[test]
fn parses_named_requests_headers_and_json_body_separated_by_hashes() {
    let document = r#"
# @name create-widget
POST https://api.example.test/widgets HTTP/1.1
Authorization: Bearer {{token}}
Content-Type: application/json

{"name":"widget"}
###
# @name health
GET https://api.example.test/health
"#;

    let requests = parse_document(document).expect("document parses");

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].name.as_deref(), Some("create-widget"));
    assert_eq!(requests[0].method, HttpMethod::Post);
    assert_eq!(requests[0].headers[0], ("Authorization".into(), "Bearer {{token}}".into()));
    assert_eq!(requests[0].body.as_deref(), Some(r#"{"name":"widget"}"#));
    assert_eq!(requests[1].start_line, 10);
}

#[test]
fn selects_a_request_by_name_index_or_containing_line() {
    let requests = parse_document(
        "# @name one\nGET https://example.test/one\n###\n# @name two\nGET https://example.test/two\n",
    )
    .expect("document parses");

    assert_eq!(
        select_request(&requests, Selection::Name("two".into()))
            .expect("name selection")
            .url,
        "https://example.test/two"
    );
    assert_eq!(
        select_request(&requests, Selection::Index(0))
            .expect("index selection")
            .name
            .as_deref(),
        Some("one")
    );
    assert_eq!(
        select_request(&requests, Selection::Line(5))
            .expect("line selection")
            .name
            .as_deref(),
        Some("two")
    );
}

#[test]
fn resolves_variables_from_explicit_values_before_environment() {
    let request = parse_document(
        "POST https://{{host}}/items/{{item}}\nX-Token: {{token}}\n\n{{payload}}",
    )
    .expect("document parses")
    .remove(0);
    let variables = BTreeMap::from([
        ("host".into(), "example.test".into()),
        ("item".into(), "42".into()),
        ("token".into(), "not-logged".into()),
        ("payload".into(), "{\"ok\":true}".into()),
    ]);

    let resolved = resolve_request(&request, &variables).expect("variables resolve");

    assert_eq!(resolved.url, "https://example.test/items/42");
    assert_eq!(resolved.headers[0].1, "not-logged");
    assert_eq!(resolved.body.as_deref(), Some("{\"ok\":true}"));
}

#[test]
fn rejects_missing_variables_and_invalid_request_lines_explicitly() {
    let request = parse_document("GET https://{{missing}}/").expect("document parses").remove(0);
    let error = resolve_request(&request, &BTreeMap::new()).expect_err("must fail");
    assert!(error.to_string().contains("missing"));

    let error = parse_document("TRACE https://example.test/").expect_err("must fail");
    assert!(error.to_string().contains("unsupported HTTP method"));
}

#[test]
fn renders_metadata_pretty_json_and_truncation_without_binary_body() {
    let rendered = render_response(
        &ResponseData {
            final_url: "https://example.test/final".into(),
            status: 201,
            duration_ms: 12,
            headers: vec![("content-type".into(), "application/json".into())],
            body: br#"{"answer":42}"#.to_vec(),
            truncated: true,
        },
        1024,
    );

    assert!(rendered.contains("URL: https://example.test/final"));
    assert!(rendered.contains("Status: 201"));
    assert!(rendered.contains("\"answer\": 42"));
    assert!(rendered.contains("truncated"));
}

#[test]
fn loads_dotenv_values_without_overriding_explicit_values() {
    let directory = std::env::temp_dir().join(format!(
        "zed-http-runner-env-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("directory creates");
    let path = directory.join(".env.private");
    fs::write(
        &path,
        "# local secret, never committed\nHOST=private.example.test\nQUOTED=\"two words\"\n",
    )
    .expect("env file writes");

    let values = load_env_file(&path).expect("env file loads");

    assert_eq!(values.get("HOST").map(String::as_str), Some("private.example.test"));
    assert_eq!(values.get("QUOTED").map(String::as_str), Some("two words"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn rejects_non_http_schemes_before_attempting_network_access() {
    let request = parse_document("GET ftp://example.test/file")
        .expect("document parses")
        .remove(0);

    let error = execute(&request, Duration::from_secs(1), 1024).expect_err("must reject scheme");

    assert!(error.to_string().contains("only http and https"));
}

#[test]
fn does_not_echo_sensitive_url_content_in_errors() {
    let request = parse_document("GET not-a-url?token=do-not-log-me")
        .expect("document parses")
        .remove(0);

    let error = execute(&request, Duration::from_secs(1), 1024).expect_err("must reject URL");

    assert!(!error.to_string().contains("do-not-log-me"));
}
