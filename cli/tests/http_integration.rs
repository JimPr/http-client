use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

use zed_http_runner::{execute, parse_document, resolve_request};

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
