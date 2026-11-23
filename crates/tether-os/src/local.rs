//! A local channel that only the current user can open: a named pipe on
//! Windows, a Unix domain socket elsewhere. The daemon listens on it; the
//! desktop app and the `tether` CLI connect to it.
//!
//! This module only controls who can *reach* the channel. The protocol on top
//! (`tether-ipc`) still authenticates both ends with a token, so a mistake in
//! either layer is not enough on its own.
//!
//! - Windows: the pipe name includes the user's SID; the pipe gets a
//!   protected DACL that grants access to that SID only; the first instance is
//!   created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so if anyone already holds
//!   the name we refuse to start rather than share it; remote clients are
//!   rejected; clients connect with `SECURITY_IDENTIFICATION`, so a fake
//!   server cannot impersonate them.
//! - Unix: the socket lives in a `0700` directory whose owner is checked, has
//!   mode `0600`, and each connection's peer credentials must be our own uid.

use std::fmt;
use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

/// A connected byte stream, either end.
pub trait Duplex: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Duplex for T {}

pub type Stream = Box<dyn Duplex>;

/// Where a profile's daemon listens.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint(imp::Address);

impl Endpoint {
    /// The endpoint for `profile` (letters, digits, '-' and '_', at most 32).
    pub fn for_profile(profile: &str) -> io::Result<Self> {
        let valid = !profile.is_empty()
            && profile.len() <= 32
            && profile.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid profile name"));
        }
        imp::address(profile).map(Self)
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        imp::display(&self.0, f)
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Endpoint({self})")
    }
}

/// The daemon's end. Dropping it stops listening (and on Unix removes the socket file).
pub struct Listener(imp::Listener);

impl Listener {
    /// Start listening. Fails with `AddrInUse` if a daemon (or anything else)
    /// already holds this endpoint.
    pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
        imp::Listener::bind(&endpoint.0).map(Self)
    }

    /// The next client. Only connections the OS let through reach here.
    pub async fn accept(&mut self) -> io::Result<Stream> {
        self.0.accept().await
    }
}

/// Connect to the daemon listening on `endpoint`.
pub async fn connect(endpoint: &Endpoint) -> io::Result<Stream> {
    imp::connect(&endpoint.0).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) fn unique_profile(name: &str) -> String {
        format!("t-{name}-{}", std::process::id())
    }

    #[test]
    fn profile_names_are_checked() {
        for bad in ["", "a/b", "..", "a b", &"x".repeat(33), "名前"] {
            assert!(Endpoint::for_profile(bad).is_err(), "{bad:?}");
        }
        assert!(Endpoint::for_profile("left_2-b").is_ok());
    }

    #[tokio::test]
    async fn a_client_reaches_the_listener_and_bytes_flow() {
        let ep = Endpoint::for_profile(&unique_profile("flow")).unwrap();
        let mut listener = Listener::bind(&ep).unwrap();
        let server = tokio::spawn(async move {
            let mut s = listener.accept().await.unwrap();
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).await.unwrap();
            s.write_all(&buf).await.unwrap();
            buf
        });
        let mut c = connect(&ep).await.unwrap();
        c.write_all(b"hello").await.unwrap();
        let mut back = [0u8; 5];
        c.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"hello");
        assert_eq!(&server.await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn a_second_listener_cannot_take_the_endpoint() {
        let ep = Endpoint::for_profile(&unique_profile("squat")).unwrap();
        let _first = Listener::bind(&ep).unwrap();
        let second = Listener::bind(&ep).err().expect("the endpoint is taken");
        assert_eq!(second.kind(), io::ErrorKind::AddrInUse);
    }

    #[tokio::test]
    async fn connecting_to_nothing_fails() {
        let ep = Endpoint::for_profile(&unique_profile("nothing")).unwrap();
        assert!(connect(&ep).await.is_err());
    }
}
