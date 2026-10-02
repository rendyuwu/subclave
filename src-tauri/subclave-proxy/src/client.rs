//! Connect to the app's browser endpoint.
//!
//! A connect failure is the `app-not-running` signal the proxy answers with;
//! a later request retries the connect.

use std::io::{self, Read, Write};

/// A bidirectional connection to the app.
pub struct Connection(Inner);

#[cfg(unix)]
type Inner = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Inner = Pipe;

pub fn connect() -> io::Result<Connection> {
    #[cfg(unix)]
    {
        let address = crate::path::socket_address()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no browser socket path"))?;
        std::os::unix::net::UnixStream::connect(address).map(Connection)
    }
    #[cfg(windows)]
    {
        let name = crate::path::pipe_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no browser pipe path"))?;
        connect_pipe(&name)
    }
}

impl Connection {
    /// Duplicate the connection so a second thread can read responses while
    /// the first keeps writing requests.
    pub fn try_clone(&self) -> io::Result<Connection> {
        self.0.try_clone().map(Connection)
    }
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(windows)]
fn connect_pipe(name: &str) -> io::Result<Connection> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    let wide = to_wide(name);

    let open = || unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            // Overlapped, so the reader thread's pending read never holds up a
            // write; see `Pipe`.
            FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };

    let mut handle = open();
    if handle == INVALID_HANDLE_VALUE {
        let err = unsafe { GetLastError() };
        if err == ERROR_PIPE_BUSY {
            // Every pipe instance was taken; wait briefly and try once more.
            unsafe { WaitNamedPipeW(wide.as_ptr(), 1000) };
            handle = open();
        }
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Pipe::new(unsafe { OwnedHandle::from_raw_handle(handle) }).map(Connection)
}

/// A pipe client handle opened with `FILE_FLAG_OVERLAPPED`.
///
/// Windows serializes every operation on a handle opened for synchronous I/O,
/// and a duplicated handle shares that queue. The proxy's reader thread sits
/// in a read for the whole connection, so on a synchronous handle every write
/// after the first waited behind that read and never reached the app. Each
/// call here starts one overlapped operation and waits for it: `Read` and
/// `Write` still block, but only their own thread.
#[cfg(windows)]
struct Pipe {
    handle: std::os::windows::io::OwnedHandle,
    /// The manual-reset event this clone's operations signal. Each thread holds
    /// its own clone, so no two pending operations share an event.
    event: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl Pipe {
    fn new(handle: std::os::windows::io::OwnedHandle) -> io::Result<Self> {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::Threading::CreateEventW;

        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle,
            event: unsafe { OwnedHandle::from_raw_handle(event) },
        })
    }

    fn try_clone(&self) -> io::Result<Self> {
        Self::new(self.handle.try_clone()?)
    }

    /// Start one overlapped operation and wait for it to finish; returns the
    /// bytes it moved.
    fn complete(
        &self,
        start: impl FnOnce(
            windows_sys::Win32::Foundation::HANDLE,
            *mut windows_sys::Win32::System::IO::OVERLAPPED,
        ) -> windows_sys::Win32::Foundation::BOOL,
    ) -> io::Result<usize> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_IO_PENDING};
        use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

        let handle = self.handle.as_raw_handle();
        // All-zero is an OVERLAPPED's initial state; a pipe ignores the offset.
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = self.event.as_raw_handle();
        if start(handle, &mut overlapped) == 0 {
            let err = unsafe { GetLastError() };
            if err != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(err as i32));
            }
        }
        // Waiting for completion keeps `overlapped` and the caller's buffer
        // alive for as long as the kernel uses them.
        let mut moved = 0u32;
        if unsafe { GetOverlappedResult(handle, &overlapped, &mut moved, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(moved as usize)
    }
}

#[cfg(windows)]
impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use windows_sys::Win32::Foundation::ERROR_BROKEN_PIPE;
        use windows_sys::Win32::Storage::FileSystem::ReadFile;

        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let ptr = buf.as_mut_ptr();
        match self.complete(|handle, overlapped| unsafe {
            ReadFile(handle, ptr, len, std::ptr::null_mut(), overlapped)
        }) {
            // The app closed its end, which a pipe reports as end of stream.
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            result => result,
        }
    }
}

#[cfg(windows)]
impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::WriteFile;

        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        self.complete(|handle, overlapped| unsafe {
            WriteFile(handle, buf.as_ptr(), len, std::ptr::null_mut(), overlapped)
        })
    }

    /// Nothing is buffered here: a completed `WriteFile` has handed every byte
    /// to the pipe.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The string form of the current token's user SID, used to name the pipe.
/// `None` when the token cannot be read.
#[cfg(windows)]
pub fn current_user_sid_string() -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::TOKEN_QUERY;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        // The handle is closed on every path, including the early returns.
        let sid = token_user_sid(token);
        CloseHandle(token);
        sid
    }
}

/// The user SID string behind an open token handle. `pipe_name` is consulted on
/// every connect attempt, so the handle must not leak.
#[cfg(windows)]
fn token_user_sid(token: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_USER};

    unsafe {
        let mut needed: u32 = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            return None;
        }
        // `usize` slots, not `u8`: the `TOKEN_USER` read below needs a buffer
        // aligned for a pointer.
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let capacity = (buffer.len() * std::mem::size_of::<usize>()) as u32;
        if GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            capacity,
            &mut needed,
        ) == 0
        {
            return None;
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut wide: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut wide) == 0 {
            return None;
        }
        let mut len = 0;
        while *wide.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(wide, len));
        LocalFree(wide as *mut core::ffi::c_void);
        Some(sid)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The proxy reads responses on one thread and writes requests on another,
    /// over one connection. On a synchronous handle a write made while the
    /// reader sat in its read never finished, so every request after the first
    /// hung, and pairing (`status`, then `associate`) ended in "Subclave is not
    /// running" once the extension's idle timer closed the port.
    #[test]
    fn a_write_completes_while_a_read_is_pending() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{
            GetLastError, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows_sys::Win32::System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT,
        };

        // Its own name: a running `tauri dev` app owns the real `.dev` pipe.
        let name = format!(
            r"\\.\pipe\subclave-proxy-duplex-test-{}",
            std::process::id()
        );
        let wide = to_wide(&name);
        let raw = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                std::ptr::null(),
            )
        };
        assert_ne!(
            raw, INVALID_HANDLE_VALUE,
            "the test pipe could not be created"
        );
        let mut server = std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(raw) });

        let mut client = connect_pipe(&name).expect("connect to the test pipe");
        let connected = unsafe { ConnectNamedPipe(server.as_raw_handle(), std::ptr::null_mut()) };
        assert!(connected != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED);

        let mut reader = client.try_clone().expect("clone the connection");
        let (read_tx, read_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            let _ = read_tx.send(reader.read_exact(&mut byte).map(|()| byte[0]));
        });
        // Let the reader block in its read first, the order the proxy runs in.
        std::thread::sleep(Duration::from_millis(200));

        let (write_tx, write_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = write_tx.send(client.write_all(b"q"));
        });
        write_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a write must not wait for the pending read")
            .expect("the write failed");

        let mut request = [0u8; 1];
        server.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"q");
        server.write_all(b"r").unwrap();
        let reply = read_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the pending read must receive the reply")
            .expect("the read failed");
        assert_eq!(reply, b'r');
    }
}
