//! End-to-end relay tests for the `subclave-proxy` binary.
//!
//! A fake app is a plain `std::os::unix::net::UnixListener`; the proxy is
//! launched as a child process with `SUBCLAVE_BROWSER_SOCKET` pointed at it,
//! which is the seam the app server never uses in production.

#![cfg(unix)]

use std::os::unix::net::UnixListener;
use std::process::{Child, Command, Stdio};

use subclave_proxy::frame::{
    read_frame, write_frame, FrameRead, MAX_REQUEST_FRAME, MAX_RESPONSE_FRAME,
};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "subclave-proxy-relay-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spawn_proxy(socket: &std::path::Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_subclave-proxy"))
        .args(args)
        .env("SUBCLAVE_BROWSER_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn proxy")
}

fn frame_body(id: &str) -> Vec<u8> {
    format!(r#"{{"v":1,"id":"{id}","action":"status"}}"#).into_bytes()
}

#[test]
fn relays_a_request_and_its_response() {
    let dir = TempDir::new("roundtrip");
    let socket = dir.0.join("browser.sock");
    let listener = UnixListener::bind(&socket).unwrap();

    let mut child = spawn_proxy(&socket, &[]);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let responder = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let request = match read_frame(&mut conn, MAX_REQUEST_FRAME).unwrap() {
            FrameRead::Frame(b) => b,
            other => panic!("expected request frame, got {other:?}"),
        };
        assert_eq!(request, frame_body("1"));
        write_frame(&mut conn, br#"{"v":1,"id":"1","ok":true,"result":{}}"#).unwrap();
    });

    let request = frame_body("1");
    write_frame(&mut stdin, &request).unwrap();
    match read_frame(&mut stdout, MAX_RESPONSE_FRAME).unwrap() {
        FrameRead::Frame(b) => {
            assert_eq!(b, br#"{"v":1,"id":"1","ok":true,"result":{}}"#);
        }
        other => panic!("expected response frame, got {other:?}"),
    }
    responder.join().unwrap();
    drop(stdin);
    let _ = child.wait();
}

#[test]
fn an_over_cap_request_is_answered_locally() {
    let dir = TempDir::new("overcap");
    let socket = dir.0.join("browser.sock");
    // Deliberately no listener: an over-cap frame must never be forwarded.
    let mut child = spawn_proxy(&socket, &[]);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let mut body = br#"{"v":1,"id":"big","action":"get-logins","params":{"pad":""#.to_vec();
    body.resize(70 * 1024, b'x');
    body.extend_from_slice(br#""}}"#);
    assert!(body.len() > MAX_REQUEST_FRAME);
    write_frame(&mut stdin, &body).unwrap();

    let response = match read_frame(&mut stdout, MAX_RESPONSE_FRAME).unwrap() {
        FrameRead::Frame(b) => b,
        other => panic!("expected too-large frame, got {other:?}"),
    };
    let value: serde_json::Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(value["id"], "big");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "too-large");

    // The stream stays in sync: the next request reaches the (absent) app and
    // is answered app-not-running.
    write_frame(&mut stdin, &frame_body("2")).unwrap();
    let response = match read_frame(&mut stdout, MAX_RESPONSE_FRAME).unwrap() {
        FrameRead::Frame(b) => b,
        other => panic!("expected app-not-running frame, got {other:?}"),
    };
    let value: serde_json::Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(value["id"], "2");
    assert_eq!(value["error"]["code"], "app-not-running");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn a_missing_app_answers_app_not_running() {
    let dir = TempDir::new("absent");
    let socket = dir.0.join("browser.sock");
    let mut child = spawn_proxy(&socket, &[]);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    write_frame(&mut stdin, &frame_body("1")).unwrap();
    let response = match read_frame(&mut stdout, MAX_RESPONSE_FRAME).unwrap() {
        FrameRead::Frame(b) => b,
        other => panic!("expected app-not-running frame, got {other:?}"),
    };
    let value: serde_json::Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(value["error"]["code"], "app-not-running");
    assert_eq!(value["error"]["message"], "Subclave is not running");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn command_line_arguments_are_ignored() {
    let dir = TempDir::new("argv");
    let socket = dir.0.join("browser.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut child = spawn_proxy(
        &socket,
        &["chrome-extension://origin/", "/path/to/manifest"],
    );
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let responder = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let _ = read_frame(&mut conn, MAX_REQUEST_FRAME).unwrap();
        write_frame(&mut conn, br#"{"v":1,"id":"1","ok":true,"result":{}}"#).unwrap();
    });

    write_frame(&mut stdin, &frame_body("1")).unwrap();
    assert!(matches!(
        read_frame(&mut stdout, MAX_RESPONSE_FRAME).unwrap(),
        FrameRead::Frame(_)
    ));
    responder.join().unwrap();
    drop(stdin);
    let _ = child.wait();
}
