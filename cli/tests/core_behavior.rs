use std::{collections::BTreeMap, fs, time::Duration};

use zed_http_runner::{
    execute, load_env_file, load_environment, merge_variable_sources, parse_document,
    render_response, resolve_request, select_request, EnvironmentOptions, HttpMethod,
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
    let directory = temporary_directory("env-test");
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
fn loads_public_and_private_values_for_an_explicit_environment() {
    let directory = temporary_directory("public-private");
    fs::write(
        directory.join("http-client.environments.json"),
        r#"{"version":1,"defaultEnvironment":"local","environments":{"local":{"HOST":"public.test","TOKEN":"public-token"}}}"#,
    )
    .expect("public config writes");
    fs::write(
        directory.join("http-client.environments.private.json"),
        r#"{"version":1,"environments":{"local":{"TOKEN":"private-token"}}}"#,
    )
    .expect("private config writes");

    let values = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        use_default: false,
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
fn selects_default_and_requires_it_when_multiple_environments_exist() {
    let directory = temporary_directory("default");
    let config = directory.join("environments.json");
    fs::write(
        &config,
        r#"{"version":1,"defaultEnvironment":"staging","environments":{"local":{"HOST":"local"},"staging":{"HOST":"staging"}}}"#,
    )
    .expect("config writes");
    let values = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: None,
        use_default: true,
        project_root: None,
        config: Some(config.clone()),
        private_config: None,
    })
    .expect("default loads");
    assert_eq!(values.get("HOST").map(String::as_str), Some("staging"));

    fs::write(
        &config,
        r#"{"version":1,"environments":{"local":{"HOST":"local"},"staging":{"HOST":"staging"}}}"#,
    )
    .expect("invalid config writes");
    let error = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: None,
        use_default: true,
        project_root: None,
        config: Some(config),
        private_config: None,
    })
    .expect_err("missing default rejects");
    assert!(error.to_string().contains("defaultEnvironment"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn rejects_unknown_names_invalid_models_and_unknown_versions_without_values() {
    let directory = temporary_directory("invalid");
    let config = directory.join("environments.json");
    fs::write(
        &config,
        r#"{"version":1,"environments":{"local":{"TOKEN":"do-not-display"}}}"#,
    )
    .expect("config writes");
    let unknown = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("missing".into()),
        use_default: false,
        project_root: None,
        config: Some(config.clone()),
        private_config: None,
    })
    .expect_err("unknown environment rejects");
    assert!(unknown.to_string().contains("unknown environment `missing`"));

    fs::write(&config, r#"{"version":2,"environments":{}}"#).expect("config writes");
    assert!(load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        use_default: false,
        project_root: None,
        config: Some(config.clone()),
        private_config: None,
    })
    .expect_err("version rejects")
    .to_string()
    .contains("unsupported environment configuration version"));

    fs::write(&config, r#"{"version":1,"environments":{"local":{"TOKEN":3}}}"#)
        .expect("config writes");
    let invalid = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        use_default: false,
        project_root: None,
        config: Some(config),
        private_config: None,
    })
    .expect_err("invalid model rejects");
    assert!(!invalid.to_string().contains("do-not-display"));
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn returns_no_values_without_configuration_and_discovery_stops_at_project_root() {
    let directory = temporary_directory("discovery");
    let outside = directory.join("outside");
    fs::create_dir_all(&outside).expect("outside directory creates");
    fs::write(
        outside.join("http-client.environments.json"),
        r#"{"version":1,"environments":{"local":{"HOST":"outside"}}}"#,
    )
    .expect("outside config writes");
    let project = directory.join("project");
    let nested = project.join("nested");
    fs::create_dir_all(&nested).expect("nested directory creates");

    let values = load_environment(&EnvironmentOptions {
        request_path: nested.join("request.http"),
        environment: None,
        use_default: true,
        project_root: Some(project.clone()),
        config: None,
        private_config: None,
    })
    .expect("no config preserves historical behavior");
    assert!(values.is_empty());

    let unknown = load_environment(&EnvironmentOptions {
        request_path: nested.join("request.http"),
        environment: Some("local".into()),
        use_default: false,
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
        use_default: true,
        project_root: Some(directory.join("project")),
        config: None,
        private_config: None,
    })
    .expect("request outside the root cannot load configuration");
    assert!(values.is_empty());

    let values = load_environment(&EnvironmentOptions {
        request_path: outside.join("request.http"),
        environment: None,
        use_default: true,
        project_root: Some(directory.join("missing-project")),
        config: None,
        private_config: None,
    })
    .expect("an invalid root cannot enable unbounded discovery");
    assert!(values.is_empty());
    fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn gives_explicit_variables_precedence_over_env_file_and_environment_configuration() {
    let variables = merge_variable_sources(
        BTreeMap::from([
            ("HOST".into(), "public".into()),
            ("TOKEN".into(), "private".into()),
        ]),
        BTreeMap::from([("TOKEN".into(), "env-file".into())]),
        BTreeMap::from([("TOKEN".into(), "explicit".into())]),
    );

    assert_eq!(variables.get("HOST").map(String::as_str), Some("public"));
    assert_eq!(variables.get("TOKEN").map(String::as_str), Some("explicit"));
}

#[test]
fn rejects_combining_explicit_and_default_environment_selectors() {
    let directory = temporary_directory("incompatible");
    let error = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        use_default: true,
        project_root: None,
        config: None,
        private_config: None,
    })
    .expect_err("selectors are mutually exclusive");

    assert!(error.to_string().contains("cannot be used together"));
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
