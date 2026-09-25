use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
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
        format!(
            r#"{{"version":2,"environments":[{{"name":"local","variables":{{"HOST":"{address}"}}}}]}}"#
        ),
    )
    .expect("config writes");
    let server = thread::spawn(move || {
        listener.set_nonblocking(true).expect("listener is nonblocking");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "request arrives");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("listener accepts: {error}"),
            }
        };
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

#[test]
fn cli_selects_an_environment_from_stdin_before_executing_a_real_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener starts");
    let address = listener.local_addr().expect("address");
    let directory = std::env::temp_dir().join(format!(
        "zed-http-runner-cli-selector-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("directory creates");
    let request_file = directory.join("request.http");
    let config = directory.join("environments.json");
    std::fs::write(&request_file, "GET http://{{HOST}}/selected").expect("request writes");
    std::fs::write(
        &config,
        format!(
            r#"{{"version":2,"environments":[{{"name":"unused","variables":{{"HOST":"127.0.0.1:1"}}}},{{"name":"local","variables":{{"HOST":"{address}"}}}}]}}"#
        ),
    )
    .expect("config writes");
    let server = thread::spawn(move || {
        listener.set_nonblocking(true).expect("listener is nonblocking");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "request arrives");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("listener accepts: {error}"),
            }
        };
        let mut request = [0_u8; 512];
        let count = stream.read(&mut request).expect("request reads");
        assert!(std::str::from_utf8(&request[..count])
            .expect("request utf8")
            .starts_with("GET /selected HTTP/1.1"));
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .expect("response writes");
    });

    let mut child = Command::new(env!("CARGO_BIN_EXE_zed-http"))
        .args([
            request_file.to_str().expect("path utf8"),
            "--config",
            config.to_str().expect("path utf8"),
            "--select-environment",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("CLI starts");
    child
        .stdin
        .take()
        .expect("stdin pipes")
        .write_all(b"2\n")
        .expect("selection writes");
    let output = child.wait_with_output().expect("CLI exits");

    server.join().expect("server succeeds");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Environment: local"));
    let selector = String::from_utf8_lossy(&output.stderr);
    assert!(selector.contains("1. unused (preselected)"));
    assert!(selector.contains("2. local"));
    assert!(!selector.contains(&address.to_string()));
    std::fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn cli_selection_without_configuration_skips_the_prompt_and_executes_directly() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener starts");
    let address = listener.local_addr().expect("address");
    let directory = std::env::temp_dir().join(format!(
        "zed-http-runner-cli-no-environment-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("directory creates");
    let request_file = directory.join("request.http");
    std::fs::write(&request_file, format!("GET http://{address}/without-environment"))
        .expect("request writes");
    let server = thread::spawn(move || {
        listener.set_nonblocking(true).expect("listener is nonblocking");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "request arrives");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("listener accepts: {error}"),
            }
        };
        let mut request = [0_u8; 512];
        let count = stream.read(&mut request).expect("request reads");
        assert!(std::str::from_utf8(&request[..count])
            .expect("request utf8")
            .starts_with("GET /without-environment HTTP/1.1"));
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .expect("response writes");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_zed-http"))
        .args([
            request_file.to_str().expect("path utf8"),
            "--select-environment",
            "--project-root",
            directory.to_str().expect("path utf8"),
        ])
        .stdin(Stdio::null())
        .output()
        .expect("CLI starts");

    server.join().expect("server succeeds");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Status: 204"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Environment:"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Select environment:"));
    std::fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn cli_rejects_default_environment_before_sending_http() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener starts");
    let address = listener.local_addr().expect("address");
    let directory = std::env::temp_dir().join(format!(
        "zed-http-runner-cli-default-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("directory creates");
    let request_file = directory.join("request.http");
    std::fs::write(&request_file, "GET http://{{HOST}}/default").expect("request writes");
    std::fs::write(
        directory.join("http-client.environments.json"),
        format!(
            r#"{{"version":2,"defaultEnvironment":"local","environments":[{{"name":"local","variables":{{"HOST":"{address}"}}}}]}}"#
        ),
    )
    .expect("configuration writes");
    let output = Command::new(env!("CARGO_BIN_EXE_zed-http"))
        .args([
            request_file.to_str().expect("path utf8"),
            "--select-environment",
            "--project-root",
            directory.to_str().expect("path utf8"),
        ])
        .output()
        .expect("CLI starts");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("environment configuration is invalid"));
    listener
        .set_nonblocking(true)
        .expect("listener becomes nonblocking");
    assert_eq!(
        listener.accept().expect_err("rejected default sent no request").kind(),
        std::io::ErrorKind::WouldBlock
    );

    std::fs::remove_dir_all(directory).expect("directory removes");
}

#[test]
fn executes_a_local_request_with_url_header_and_body_from_inline_variables() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener starts");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request arrives");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 512];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).expect("request reads");
            assert_ne!(count, 0, "request contains headers");
            request.extend_from_slice(&buffer[..count]);
        }
        while request.len() < request.windows(4).position(|window| window == b"\r\n\r\n").expect("headers end") + 15 {
            let count = stream.read(&mut buffer).expect("body reads");
            assert_ne!(count, 0, "request contains body");
            request.extend_from_slice(&buffer[..count]);
        }
        let request = String::from_utf8(request).expect("request utf8");
        assert!(request.starts_with("POST /inline HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("x-inline-token: safe-test-value"));
        assert!(request.ends_with("inline body"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .expect("response writes");
    });

    let source = format!(
        "@host = {address}\n@token = safe-test-value\n@payload = inline body\nPOST http://{{{{host}}}}/inline\nX-Inline-Token: {{{{token}}}}\nContent-Type: text/plain\n\n{{{{payload}}}}"
    );
    let request = parse_document(&source).expect("document parses").remove(0);
    let request =
        resolve_request(&request, &request.inline_variables).expect("inline variables resolve");
    let response = execute(&request, Duration::from_secs(2), 1024).expect("request executes");

    server.join().expect("server succeeds");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"ok");
}

#[test]
fn rejects_the_removed_legacy_file_option_before_reading_the_request() {
    let legacy_option = format!("--{}-{}", "env", "file");
    let output = Command::new(env!("CARGO_BIN_EXE_zed-http"))
        .args(["missing.http", legacy_option.as_str(), "ignored"])
        .output()
        .expect("CLI starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown option"),
        "removed option must not be accepted"
    );
}
