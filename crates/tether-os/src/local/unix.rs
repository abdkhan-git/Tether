//! Unix domain sockets in a private directory.

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tokio::net::{UnixListener, UnixStream};

use super::Stream;

pub type Address = PathBuf;

/// Our effective user id.
fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions, touches no memory and cannot fail.
    unsafe { libc::geteuid() }
}

/// A directory only we can enter: `$XDG_RUNTIME_DIR/tether` on Linux,
/// `$TMPDIR/tether` on macOS (per-user there), else under the data folder.
fn base_dir() -> io::Result<PathBuf> {
    if let Some(run) = dirs::runtime_dir() {
        return Ok(run.join("tether"));
    }
    if cfg!(target_os = "macos") {
        return Ok(std::env::temp_dir().join("tether"));
    }
    dirs::data_local_dir()
        .map(|d| d.join("Tether").join("run"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))
}

pub fn address(profile: &str) -> io::Result<Address> {
    Ok(base_dir()?.join(format!("{profile}.sock")))
}

pub fn display(a: &Address, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", a.display())
}

/// Create `dir` as `0700`, or check that an existing one is a real directory
/// we own, and tighten its mode. Refuses one owned by anyone else: in a
/// shared folder such as `/tmp`, someone else may have made it first.
fn private_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not a directory", dir.display())));
    }
    if meta.uid() != euid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} belongs to another user", dir.display()),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub struct Listener {
    inner: UnixListener,
    path: PathBuf,
    uid: u32,
}

impl Listener {
    pub fn bind(path: &Address) -> io::Result<Self> {
        let dir = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket has no folder"))?;
        private_dir(dir)?;
        if fs::symlink_metadata(path).is_ok() {
            // A live daemon answers; a stale file from a crash does not.
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("{} is in use: another tetherd is running for this profile", path.display()),
                ));
            }
            fs::remove_file(path)?;
        }
        let inner = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Self { inner, path: path.clone(), uid: euid() })
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        loop {
            let (stream, _) = self.inner.accept().await?;
            // The directory and file modes should already stop other users;
            // checking the peer's uid makes it certain.
            match stream.peer_cred() {
                Ok(cred) if cred.uid() == self.uid => return Ok(Box::new(stream)),
                _ => drop(stream),
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub async fn connect(path: &Address) -> io::Result<Stream> {
    Ok(Box::new(UnixStream::connect(path).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::tests::unique_profile;

    #[test]
    fn the_socket_is_private_in_a_private_folder() {
        let path = address(&unique_profile("modes")).unwrap();
        let _listener = Listener::bind(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let dir = fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!((dir.mode() & 0o777, dir.uid()), (0o700, euid()));
    }

    #[test]
    fn a_stale_socket_file_is_replaced() {
        let path = address(&unique_profile("stale")).unwrap();
        drop(Listener::bind(&path).unwrap());
        // Simulate a crash that left the file behind.
        let _ = std::os::unix::net::UnixListener::bind(&path).map(drop);
        assert!(Listener::bind(&path).is_ok());
    }
}
