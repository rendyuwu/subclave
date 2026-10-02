//! The native messaging host: Chrome and Firefox launch this binary and speak
//! the 4-byte-length-prefixed JSON protocol over stdio. It relays every frame
//! to the app's browser socket, and answers `app-not-running` on its own when
//! the app is closed or the socket drops.
//!
//! No Tauri, no crypto: the app owns every decision. Only framed messages go
//! to stdout; diagnostics go to stderr and only in a debug build.

use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex};

use subclave_proxy::client;
use subclave_proxy::frame::{self, FrameRead, MAX_REQUEST_FRAME, MAX_RESPONSE_FRAME};

/// Shared with the reader thread: the connection, once lazily established.
struct Shared {
    conn: Mutex<Option<client::Connection>>,
    ready: Condvar,
}

fn main() {
    // Resolved only to fail fast: with no socket path the host could never
    // reach the app, and `client::connect` resolves the same address again when
    // a request arrives.
    if subclave_proxy::path::socket_address().is_none() {
        #[cfg(debug_assertions)]
        eprintln!("subclave-proxy: no browser socket path");
        std::process::exit(1);
    }

    let out = Arc::new(Mutex::new(io::stdout()));
    let shared = Arc::new(Shared {
        conn: Mutex::new(None),
        ready: Condvar::new(),
    });

    {
        let shared = shared.clone();
        let out = out.clone();
        std::thread::Builder::new()
            .name("subclave-proxy-socket".into())
            .spawn(move || reader_loop(shared, out))
            .expect("spawn socket reader");
    }

    let mut stdin = io::stdin();
    loop {
        match frame::read_frame(&mut stdin, MAX_REQUEST_FRAME) {
            Ok(FrameRead::Eof) => return,
            Ok(FrameRead::Frame(bytes)) => forward(&shared, &out, &bytes),
            Ok(FrameRead::OverCap { id, desynced }) => {
                write_out(&out, &frame::warn_too_large(&id));
                if desynced {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

/// Answer a request the app cannot receive with `app-not-running`.
fn app_not_running(out: &Mutex<io::Stdout>, id: &str) {
    write_out(
        out,
        &frame::error_frame(id, "app-not-running", "Subclave is not running"),
    );
}

fn write_out(out: &Mutex<io::Stdout>, framed: &[u8]) {
    let mut guard = out.lock().unwrap_or_else(|e| e.into_inner());
    let _ = guard.write_all(framed);
    let _ = guard.flush();
}

/// Frame `payload` before it reaches stdout: the response body alone is not a
/// message, and a caller reading the 4-byte length would desync.
fn write_framed(out: &Mutex<io::Stdout>, payload: &[u8]) {
    let mut guard = out.lock().unwrap_or_else(|e| e.into_inner());
    let _ = frame::write_frame(&mut *guard, payload);
}

/// Relay one request frame: connect lazily on the first request, and answer
/// `app-not-running` when the connect or the write fails so the caller can
/// retry on its next request.
fn forward(shared: &Shared, out: &Mutex<io::Stdout>, bytes: &[u8]) {
    let mut guard = shared.conn.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        match client::connect() {
            Ok(conn) => {
                *guard = Some(conn);
                shared.ready.notify_all();
            }
            Err(_) => {
                app_not_running(out, &frame::request_id(bytes));
                return;
            }
        }
    }
    if let Some(conn) = guard.as_mut() {
        if frame::write_frame(conn, bytes).is_err() {
            *guard = None;
            app_not_running(out, &frame::request_id(bytes));
        }
    }
}

/// Read responses from the socket and relay them to stdout. Waits for the
/// first request to establish the connection, and re-waits after the socket
/// closes so the next request reconnects.
fn reader_loop(shared: Arc<Shared>, out: Arc<Mutex<io::Stdout>>) {
    loop {
        let mut guard = shared.conn.lock().unwrap_or_else(|e| e.into_inner());
        while guard.is_none() {
            guard = shared.ready.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        let Some(mut reader) = guard.as_ref().and_then(|c| c.try_clone().ok()) else {
            *guard = None;
            continue;
        };
        drop(guard);

        loop {
            match frame::read_frame(&mut reader, MAX_RESPONSE_FRAME) {
                Ok(FrameRead::Frame(bytes)) => write_framed(&out, &bytes),
                Ok(FrameRead::OverCap { id, desynced }) => {
                    write_out(&out, &frame::warn_too_large(&id));
                    if desynced {
                        std::process::exit(0);
                    }
                }
                Ok(FrameRead::Eof) => break,
                Err(_) => break,
            }
        }

        // The socket died; forget it so the next request reconnects.
        let mut guard = shared.conn.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }
}
