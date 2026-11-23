//! Named pipes with an owner-only DACL.

use std::fmt;
use std::io;
use std::mem;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{LocalFree, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::Stream;

/// Server instances the pipe may have at once (connected clients plus one
/// waiting). The daemon closes clients beyond its own, lower cap.
const MAX_INSTANCES: usize = 32;
/// How long a client waits for a free instance before giving up.
const BUSY_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq, Eq)]
pub struct Address {
    pipe: String,
    sid: String,
}

pub fn address(profile: &str) -> io::Result<Address> {
    let sid = current_user_sid()?;
    Ok(Address { pipe: format!(r"\\.\pipe\tether-{sid}-{profile}"), sid })
}

pub fn display(a: &Address, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&a.pipe)
}

/// Read a NUL-terminated UTF-16 string that Windows allocated with
/// `LocalAlloc`, then free it.
///
/// # Safety
/// `p` must be non-null, NUL-terminated, allocated by `LocalAlloc`, and not
/// used again after this call.
unsafe fn take_local_wide(p: *mut u16) -> String {
    let mut len = 0;
    // SAFETY: the caller guarantees `p` is NUL-terminated, so every index up
    // to and including the terminator is in bounds.
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` elements before the terminator were just read above.
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) });
    // SAFETY: the caller guarantees `p` came from `LocalAlloc` and is not reused.
    unsafe { LocalFree(p.cast()) };
    s
}

/// The current user's SID as a string, for example `S-1-5-21-…-1001`.
pub(crate) fn current_user_sid() -> io::Result<String> {
    let mut raw: HANDLE = null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that is always valid
    // and needs no closing; `raw` is a valid place for the token handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call above succeeded, so `raw` is an open handle that we own
    // and nothing else will close.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };

    let mut len = 0u32;
    // SAFETY: asking for the size: a null buffer with length 0 is allowed; the
    // call fails with ERROR_INSUFFICIENT_BUFFER and writes the size to `len`.
    unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    // u64 elements: TOKEN_USER holds pointers, so the buffer must be 8-aligned.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` is writable, 8-aligned and at least `len` bytes long.
    if unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, buf.as_mut_ptr().cast(), len, &mut len) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success the buffer begins with a TOKEN_USER; its SID pointer
    // points into `buf`, which lives until the end of this function.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut wide = null_mut();
    // SAFETY: `sid` is a valid SID (above); `wide` is a valid out-pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut wide) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success `wide` is a NUL-terminated string from LocalAlloc,
    // used only here.
    Ok(unsafe { take_local_wide(wide) })
}

/// A security descriptor that grants full access to one SID and nothing to
/// anyone else. `P` marks the DACL protected, so no entries are inherited.
struct OwnerOnly(PSECURITY_DESCRIPTOR);

impl OwnerOnly {
    fn new(sid: &str) -> io::Result<Self> {
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})").encode_utf16().chain([0]).collect();
        let mut sd = null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16 that outlives the call; `sd`
        // is a valid out-pointer; the size out-pointer may be null.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut())
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(sd))
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        // SAFETY: the descriptor was allocated by
        // ConvertStringSecurityDescriptorToSecurityDescriptorW with LocalAlloc,
        // and is freed exactly once, here.
        unsafe { LocalFree(self.0) };
    }
}

fn create(addr: &Address, first: bool) -> io::Result<NamedPipeServer> {
    let sd = OwnerOnly::new(&addr.sid)?;
    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).reject_remote_clients(true).max_instances(MAX_INSTANCES);
    // SAFETY: `attrs` is a fully initialised SECURITY_ATTRIBUTES whose
    // descriptor (`sd`) stays alive until after this call returns; Windows
    // copies the descriptor into the new pipe object.
    let server =
        unsafe { options.create_with_security_attributes_raw(&addr.pipe, std::ptr::addr_of_mut!(attrs).cast()) };
    drop(sd);
    server
}

pub struct Listener {
    addr: Address,
    next: NamedPipeServer,
}

impl Listener {
    pub fn bind(addr: &Address) -> io::Result<Self> {
        let next = create(addr, true).map_err(|e| {
            // FILE_FLAG_FIRST_PIPE_INSTANCE reports an existing name this way.
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!(
                        "{} already exists: another tetherd is running for this profile, \
                         or another program has taken the name",
                        addr.pipe
                    ),
                )
            } else {
                e
            }
        })?;
        Ok(Self { addr: addr.clone(), next })
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        let connected = self.next.connect().await;
        // Whatever happened, put a fresh instance in place for the next client.
        let fresh = create(&self.addr, false)?;
        let instance = mem::replace(&mut self.next, fresh);
        connected?;
        Ok(Box::new(instance))
    }
}

pub async fn connect(addr: &Address) -> io::Result<Stream> {
    let deadline = Instant::now() + BUSY_WAIT;
    loop {
        // Identification only: the server may learn who we are but cannot act
        // as us (this is tokio's default too; it is spelled out on purpose).
        match ClientOptions::new().security_qos_flags(SECURITY_IDENTIFICATION).open(&addr.pipe) {
            Ok(client) => return Ok(Box::new(client)),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::tests::unique_profile;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

    /// The DACL of an open pipe, read back from Windows as SDDL.
    fn dacl(pipe: &NamedPipeServer) -> String {
        let mut sd = null_mut();
        // SAFETY: the handle is open for the duration of the call; only the
        // descriptor out-pointer is requested and it is valid.
        let rc = unsafe {
            GetSecurityInfo(
                pipe.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut sd,
            )
        };
        assert_eq!(rc, 0, "GetSecurityInfo failed");
        let mut text = null_mut();
        // SAFETY: `sd` is the valid descriptor returned above; `text` is a
        // valid out-pointer; the length out-pointer may be null.
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                null_mut(),
            )
        };
        assert_ne!(ok, 0, "could not convert the descriptor");
        // SAFETY: `text` came from LocalAlloc and is used only here; `sd` was
        // allocated by GetSecurityInfo with LocalAlloc and is freed once.
        unsafe {
            let s = take_local_wide(text);
            LocalFree(sd);
            s
        }
    }

    #[test]
    fn the_sid_looks_like_a_sid() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-") && sid.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S'), "{sid}");
    }

    #[tokio::test]
    async fn the_pipe_admits_this_user_only() {
        let addr = address(&unique_profile("dacl")).unwrap();
        let mut listener = Listener::bind(&addr).unwrap();
        let sid = current_user_sid().unwrap();
        // Protected (no inherited entries), exactly one entry: this user, full access.
        assert_eq!(dacl(&listener.next), format!("D:P(A;;FA;;;{sid})"));
        // Instances created after the first get the same descriptor.
        let client = connect(&addr).await.unwrap();
        let _served = listener.accept().await.unwrap();
        assert_eq!(dacl(&listener.next), format!("D:P(A;;FA;;;{sid})"));
        drop(client);
    }

    #[test]
    fn the_name_is_per_user() {
        let addr = address("default").unwrap();
        let sid = current_user_sid().unwrap();
        assert_eq!(addr.pipe, format!(r"\\.\pipe\tether-{sid}-default"));
    }
}
