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
type Inner = std::fs::File;

pub fn connect() -> io::Result<Connection> {
    #[cfg(unix)]
    {
        let address = crate::path::socket_address()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no browser socket path"))?;
        std::os::unix::net::UnixStream::connect(address).map(Connection)
    }
    #[cfg(windows)]
    {
        connect_pipe()
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
fn connect_pipe() -> io::Result<Connection> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    let name = crate::path::pipe_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no browser pipe path"))?;
    let wide = to_wide(&name);

    let open = || unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
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
    Ok(Connection(unsafe {
        std::fs::File::from_raw_handle(handle)
    }))
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
