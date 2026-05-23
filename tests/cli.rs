use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
fn serve_once(html: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        stream.read(&mut request).unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    format!("http://{address}/page")
}
fn temp_directory(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("sitesaver-cli-test-{name}-{suffix}"));
    fs::create_dir_all(&directory).unwrap();
    directory
}
#[test]
fn saves_markdown_file_from_local_server() {
    // GIVEN
    let html = r#"
        <html>
            <head><title>Saved Page</title><style>.hidden{}</style></head>
            <body><nav>Navigation</nav><main><h1>Hello</h1><p>Readable text</p></main></body>
        </html>
    "#;
    let url = serve_once(html);
    let directory = temp_directory("save");
    // WHEN
    let output = Command::new(env!("CARGO_BIN_EXE_sitesaver"))
        .args([url.as_str(), "--output", directory.to_str().unwrap()])
        .output()
        .unwrap();
    // THEN
    let saved_path = String::from_utf8(output.stdout).unwrap().trim().to_string();
    let actual = fs::read_to_string(saved_path).unwrap();
    let expected = "# Hello\n\nReadable text";
    assert!(output.status.success());
    assert_eq!(actual, expected);
}
#[test]
fn writes_to_stdout_without_creating_file() {
    // GIVEN
    let html = r#"
        <html>
            <head><title>Stdout Page</title></head>
            <body><main><p>Only stdout</p><script>bad()</script></main></body>
        </html>
    "#;
    let url = serve_once(html);
    let directory = temp_directory("stdout");
    // WHEN
    let output = Command::new(env!("CARGO_BIN_EXE_sitesaver"))
        .args([
            url.as_str(),
            "--stdout",
            "--output",
            directory.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    // THEN
    let actual = String::from_utf8(output.stdout).unwrap().trim().to_string();
    let expected = "Only stdout";
    let created_file_count = fs::read_dir(directory).unwrap().count();
    assert!(output.status.success());
    assert_eq!(actual, expected);
    assert_eq!(created_file_count, 0);
}
