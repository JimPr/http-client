use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

use zed_http_runner::{
    execute, load_environment, parse_document, resolve_request, EnvironmentOptions,
};

#[test]
fn executes_a_post_against_a_real_local_server_and_renders_its_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener starts");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request arrives");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout set");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 512];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).expect("request reads");
            if count == 0 {
                break;
            }

            request.extend_from_slice(&buffer[..count]);
        }
        let text = String::from_utf8_lossy(&request);
        assert!(text.starts_with("POST /echo HTTP/1.1"));
        assert!(text.to_ascii_lowercase().contains("x-request-id: test-id"));
        let response = b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{\"received\":\"hello\"}";
        stream.write_all(response).expect("response writes");
    });

    let source = format!(
        "# @name local\nPOST http://{address}/echo\nX-Request-Id: {{{{request_id}}}}\nContent-Type: text/plain\n\nhello"
    );
    let request = parse_document(&source).expect("document parses").remove(0);
    let request = resolve_request(
        &request,
        &BTreeMap::from([("request_id".into(), "test-id".into())]),
    )
    .expect("variables resolve");

    let response = execute(&request, Duration::from_secs(2), 1024).expect("request executes");

    server.join().expect("server succeeds");
    assert_eq!(response.status, 201);
    assert_eq!(response.body, br#"{"received":"hello"}"#);
    assert!(!response.truncated);
}

#[test]
fn executes_against_a_local_server_using_an_explicitly_selected_environment() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener starts");
    let address = listener.local_addr().expect("address");
    let directory = std::env::temp_dir().join(format!(
        "zed-http-runner-selected-environment-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("directory creates");
    let config = directory.join("environments.json");
    std::fs::write(
        &config,
        format!(r#"{{"version":1,"environments":{{"local":{{"HOST":"{address}"}}}}}}"#),
    )
    .expect("config writes");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request arrives");
        let mut request = [0_u8; 512];
        let count = stream.read(&mut request).expect("request reads");
        assert!(std::str::from_utf8(&request[..count])
            .expect("request utf8")
            .starts_with("GET /selected HTTP/1.1"));
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .expect("response writes");
    });

    let values = load_environment(&EnvironmentOptions {
        request_path: directory.join("request.http"),
        environment: Some("local".into()),
        use_default: false,
        project_root: None,
        config: Some(config),
        private_config: None,
    })
    .expect("environment loads");
    let request = parse_document("GET http://{{HOST}}/selected")
        .expect("document parses")
        .remove(0);
    let request = resolve_request(&request, &values).expect("variables resolve");
    let response = execute(&request, Duration::from_secs(2), 1024).expect("request executes");

    server.join().expect("server succeeds");
    assert_eq!(response.status, 204);
    std::fs::remove_dir_all(directory).expect("directory removes");
}
