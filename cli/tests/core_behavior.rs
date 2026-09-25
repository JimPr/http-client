use std::{
    collections::BTreeMap,
    fs,
    io::Cursor,
    time::Duration,
};

use zed_http_runner::{
    execute, load_environment, merge_variable_sources, parse_document, render_response,
    resolve_request, select_environment, select_request, EnvironmentOptions, HttpMethod,
    ResponseData, Selection,
};

fn temporary_directory(label: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "zed-http-runner-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock works")
            .as_nanos()
    ));
    fs::create_dir_all(&directory).expect("directory creates");
    directory
}

fn environment_options(directory: &std::path::Path) -> EnvironmentOptions {
    EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: None,
        project_root: Some(directory.to_owned()),
        config: None,
        private_config: None,
    }
}

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
fn snapshots_initial_inline_declarations_and_keeps_literal_values() {
    let request = parse_document(
        "@host = api.example.test\n@quoted = \"still quoted\"\n@template = {{not-resolved}}\n\nGET https://{{host}}/{{template}}\n",
    )
    .expect("document parses")
    .remove(0);

    assert_eq!(
        request.inline_variables,
        BTreeMap::from([
            ("host".into(), "api.example.test".into()),
            ("quoted".into(), "\"still quoted\"".into()),
            ("template".into(), "{{not-resolved}}".into()),
        ])
    );
    let resolved = resolve_request(&request, &request.inline_variables).expect("variables resolve");
    assert_eq!(resolved.url, "https://api.example.test/{{not-resolved}}");
    assert_eq!(
        request.inline_variables.get("template").map(String::as_str),
        Some("{{not-resolved}}")
    );
}

#[test]
fn scopes_inline_declarations_forward_across_sections_without_retroactivity() {
    let requests = parse_document(
        "@host = first.test\n###\n@token = one\n###\nGET https://{{host}}/first\n###\n@host = second.test\nGET https://{{host}}/second\n",
    )
    .expect("document parses");

    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].inline_variables,
        BTreeMap::from([
            ("host".into(), "first.test".into()),
            ("token".into(), "one".into()),
        ])
    );
    assert_eq!(
        requests[1].inline_variables,
        BTreeMap::from([
            ("host".into(), "second.test".into()),
            ("token".into(), "one".into()),
        ])
    );
}

#[test]
fn preserves_at_lines_in_a_request_body() {
    let request = parse_document(
        "@host = api.example.test\nPOST https://{{host}}/events\nContent-Type: text/plain\n\n@not-a-declaration = body-content\n",
    )
    .expect("document parses")
    .remove(0);

    assert_eq!(
        request.inline_variables,
        BTreeMap::from([("host".into(), "api.example.test".into())])
    );
    assert_eq!(
        request.body.as_deref(),
        Some("@not-a-declaration = body-content")
    );
}

#[test]
fn rejects_invalid_inline_declarations_without_echoing_values() {
    let error = parse_document("@api_key value-that-must-not-appear\nGET https://example.test/")
        .expect_err("invalid declaration rejects");

    assert!(error.to_string().contains("invalid inline variable declaration"));
    assert!(!error.to_string().contains("value-that-must-not-appear"));
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
fn loads_public_and_private_values_for_an_explicit_environment() {
    let directory = temporary_directory("public-private");
    fs::write(
        directory.join("http-client.environments.json"),
        r#"{"version":2,"environments":[{"name":"local","variables":{"HOST":"public.test","TOKEN":"public-token"}}]}"#,
    )
    .expect("public config writes");
    fs::write(
        directory.join("http-client.environments.private.json"),
        r#"{"version":2,"environments":[{"name":"local","variables":{"TOKEN":"private-token"}}]}"#,
    )
    .expect("private config writes");

    let values = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        project_root: Some(directory.clone()),
        config: None,
        private_config: None,
    })
    .expect("environment loads");

    assert_eq!(values.get("HOST").map(String::as_str), Some("public.test"));
    assert_eq!(values.get("TOKEN").map(String::as_str), Some("private-token"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn rejects_default_environment_as_an_unknown_v2_field() {
    let directory = temporary_directory("default");
    let config = directory.join("environments.json");
    fs::write(
        &config,
        r#"{"version":2,"defaultEnvironment":"staging","environments":[{"name":"staging","variables":{"HOST":"staging"}},{"name":"local","variables":{"HOST":"local"}}]}"#,
    )
    .expect("config writes");
    let error = select_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: None,
        project_root: None,
        config: Some(config),
        private_config: None,
    }, &mut Cursor::new(b"\n"), &mut Vec::new())
    .expect_err("v2 does not support persistent defaults");
    assert!(error.to_string().contains("invalid"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn selects_an_environment_from_injected_terminal_input() {
    let directory = temporary_directory("selector");
    let config = directory.join("environments.json");
    fs::write(
        &config,
        r#"{"version":2,"environments":[{"name":"zebra","variables":{"HOST":"zebra.test"}},{"name":"alpha","variables":{"HOST":"alpha.test"}}]}"#,
    )
    .expect("config writes");
    let options = EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: None,
        project_root: None,
        config: Some(config),
        private_config: None,
    };

    let mut rendered = Vec::new();
    let default = select_environment(&options, &mut Cursor::new(b"\n"), &mut rendered)
        .expect("empty input picks default");
    assert_eq!(default.name.as_deref(), Some("zebra"));
    assert_eq!(default.values.get("HOST").map(String::as_str), Some("zebra.test"));
    assert_eq!(
        String::from_utf8(rendered).expect("output utf8"),
        "Select environment:\n  1. zebra (preselected)\n  2. alpha\nEnter a number [1]: "
    );

    let selected = select_environment(
        &options,
        &mut Cursor::new(b"2\n"),
        &mut Vec::new(),
    )
    .expect("numeric selection succeeds");
    assert_eq!(selected.name.as_deref(), Some("alpha"));

    for input in [b"0\n".as_slice(), b"invalid\n".as_slice(), b"".as_slice()] {
        let error = select_environment(&options, &mut Cursor::new(input), &mut Vec::new())
            .expect_err("invalid or cancelled selection cannot proceed");
        assert!(error.to_string().contains("selection"));
    }
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn public_configuration_controls_names_order_while_private_overrides_values() {
    let directory = temporary_directory("public-source");
    let public = directory.join("public.json");
    let private = directory.join("private.json");
    fs::write(
        &public,
        r#"{"version":2,"environments":[{"name":"zebra","variables":{"HOST":"public-zebra","TOKEN":"public-token"}},{"name":"local","variables":{"HOST":"public-local"}}]}"#,
    )
    .expect("public config writes");
    fs::write(
        &private,
        r#"{"version":2,"environments":[{"name":"local","variables":{"HOST":"private-local"}},{"name":"zebra","variables":{"TOKEN":"private-token"}}]}"#,
    )
    .expect("private config writes");

    let mut output = Vec::new();
    let values = select_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: None,
        project_root: None,
        config: Some(public),
        private_config: Some(private),
    }, &mut Cursor::new(b"\n"), &mut output)
    .expect("picker chooses the first public environment");

    assert_eq!(values.values.get("HOST").map(String::as_str), Some("public-zebra"));
    assert_eq!(values.values.get("TOKEN").map(String::as_str), Some("private-token"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn selecting_without_configuration_does_not_render_or_read_a_prompt() {
    let directory = temporary_directory("selector-no-config");
    let options = EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: None,
        project_root: Some(directory.clone()),
        config: None,
        private_config: None,
    };
    let mut output = Vec::new();

    let selected = select_environment(&options, &mut Cursor::new(b""), &mut output)
        .expect("historical no-configuration mode remains non-interactive");

    assert_eq!(selected.name, None);
    assert!(selected.values.is_empty());
    assert!(output.is_empty());
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn first_environment_is_only_the_picker_enter_preselection() {
    let directory = temporary_directory("picker-preselection");
    fs::write(
        directory.join("http-client.environments.json"),
        r#"{"version":2,"environments":[{"name":"first","variables":{}},{"name":"second","variables":{}}]}"#,
    )
    .expect("configuration writes");
    let mut output = Vec::new();
    let selected = select_environment(
        &environment_options(&directory),
        &mut Cursor::new(b"\n"),
        &mut output,
    )
    .expect("enter chooses first picker item");
    assert_eq!(selected.name.as_deref(), Some("first"));
    assert!(String::from_utf8(output).expect("utf8").contains("1. first (preselected)"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn rejects_unknown_names_invalid_models_and_unknown_versions_without_values() {
    let directory = temporary_directory("invalid");
    let config = directory.join("environments.json");
    fs::write(
        &config,
        r#"{"version":2,"environments":[{"name":"local","variables":{"TOKEN":"do-not-display"}}]}"#,
    )
    .expect("config writes");
    let unknown = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("missing".into()),
        project_root: None,
        config: Some(config.clone()),
        private_config: None,
    })
    .expect_err("unknown environment rejects");
    assert!(unknown.to_string().contains("unknown environment `missing`"));

    fs::write(&config, r#"{"version":1,"environments":{"local":{"TOKEN":"do-not-display"}}}"#)
        .expect("config writes");
    let legacy = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        project_root: None,
        config: Some(config.clone()),
        private_config: None,
    })
    .expect_err("version rejects");
    assert!(legacy.to_string().contains("migrate to version 2"));
    assert!(!legacy.to_string().contains("do-not-display"));

    fs::write(&config, r#"{"version":2,"environments":[{"name":"local","variables":{"TOKEN":3}}]}"#)
        .expect("config writes");
    let invalid = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        project_root: None,
        config: Some(config),
        private_config: None,
    })
    .expect_err("invalid model rejects");
    assert!(!invalid.to_string().contains("do-not-display"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn rejects_invalid_v2_structures_without_disclosing_variable_values() {
    let directory = temporary_directory("invalid-v2");
    let config = directory.join("environments.json");
    for invalid in [
        r#"{"version":2,"environments":[]}"#,
        r#"{"version":2,"environments":[{"name":"","variables":{"TOKEN":"do-not-display"}}]}"#,
        r#"{"version":2,"environments":[{"name":"local","variables":{}},{"name":"local","variables":{}}]}"#,
        r#"{"version":2,"environments":[{"name":"local","variables":{"TOKEN":7}}]}"#,
        r#"{"version":2,"environments":[{"name":"local","variables":{},"extra":true}]}"#,
    ] {
        fs::write(&config, invalid).expect("config writes");
        let error = load_environment(&EnvironmentOptions {
            request_path: directory.join("request.http"),
            environment: Some("local".into()),
            project_root: None,
            config: Some(config.clone()),
            private_config: None,
        })
        .expect_err("invalid v2 model rejects");
        assert!(error.to_string().contains("environment configuration"));
        assert!(!error.to_string().contains("do-not-display"));
    }
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn returns_no_values_without_configuration_and_discovery_stops_at_project_root() {
    let directory = temporary_directory("discovery");
    let outside = directory.join("outside");
    fs::create_dir_all(&outside).expect("outside directory creates");
    fs::write(
        outside.join("http-client.environments.json"),
        r#"{"version":2,"environments":[{"name":"local","variables":{"HOST":"outside"}}]}"#,
    )
    .expect("outside config writes");
    let project = directory.join("project");
    let nested = project.join("nested");
    fs::create_dir_all(&nested).expect("nested directory creates");

    let values = load_environment(&EnvironmentOptions {
        request_path: nested.join("request.http"),
        environment: None,
        project_root: Some(project.clone()),
        config: None,
        private_config: None,
    })
    .expect("no config preserves historical behavior");
    assert!(values.is_empty());

    let unknown = load_environment(&EnvironmentOptions {
        request_path: nested.join("request.http"),
        environment: Some("local".into()),
        project_root: Some(project),
        config: None,
        private_config: None,
    })
    .expect_err("explicit selection needs configuration");
    assert!(unknown.to_string().contains("no environment configuration"));

    let outside_request = outside.join("request.http");
    let values = load_environment(&EnvironmentOptions {
        request_path: outside_request,
        environment: None,
        project_root: Some(directory.join("project")),
        config: None,
        private_config: None,
    })
    .expect("request outside the root cannot load configuration");
    assert!(values.is_empty());

    let values = load_environment(&EnvironmentOptions {
        request_path: outside.join("request.http"),
        environment: None,
        project_root: Some(directory.join("missing-project")),
        config: None,
        private_config: None,
    })
    .expect("an invalid root cannot enable unbounded discovery");
    assert!(values.is_empty());
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn gives_explicit_variables_precedence_over_inline_private_and_public_configuration() {
    let variables = merge_variable_sources(
        BTreeMap::from([
            ("HOST".into(), "public".into()),
            ("PUBLIC_ONLY".into(), "public".into()),
        ]),
        BTreeMap::from([
            ("HOST".into(), "private".into()),
            ("PRIVATE_ONLY".into(), "private".into()),
        ]),
        BTreeMap::from([
            ("HOST".into(), "inline".into()),
            ("INLINE_ONLY".into(), "inline".into()),
        ]),
        BTreeMap::from([
            ("HOST".into(), "explicit".into()),
            ("EXPLICIT_ONLY".into(), "explicit".into()),
        ]),
    );

    assert_eq!(variables.get("HOST").map(String::as_str), Some("explicit"));
    assert_eq!(variables.get("PUBLIC_ONLY").map(String::as_str), Some("public"));
    assert_eq!(variables.get("PRIVATE_ONLY").map(String::as_str), Some("private"));
    assert_eq!(variables.get("INLINE_ONLY").map(String::as_str), Some("inline"));
    assert_eq!(variables.get("EXPLICIT_ONLY").map(String::as_str), Some("explicit"));
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
