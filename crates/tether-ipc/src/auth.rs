//! Mutual authentication of a control connection.
//!
//! The daemon makes a fresh 256-bit token at every start and keeps it in the
//! OS credential store (see [`crate::token`]). Each end proves it knows the
//! token without sending it:
//!
//! ```text
//! server -> client   ServerHello { magic, version, server_nonce }
//! client -> server   ClientProof { version, client_nonce, MAC(k_client, nonces) }
//! server -> client   ServerProof { MAC(k_server, nonces) }
//! ```
//!
//! `k_client` and `k_server` are derived from the token with BLAKE3's key
//! derivation under different contexts, so neither proof can be replayed as
//! the other; fresh nonces from both sides stop replays across connections.
//! A server that gets a bad proof closes without answering. A client that
//! gets a bad proof (someone else holds the endpoint) stops before sending
//! any request.

use std::fmt;
use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use zeroize::Zeroizing;

use crate::frame::{recv, send};
use crate::proto::PROTOCOL_VERSION;

const MAGIC: [u8; 8] = *b"tether-c";
const HANDSHAKE_MAX: usize = 128;
const CLIENT_CONTEXT: &str = "tether 2026-11-23 control channel client proof v1";
const SERVER_CONTEXT: &str = "tether 2026-11-23 control channel server proof v1";

/// The daemon's control token. Zeroed on drop and never printed.
#[derive(Clone)]
pub struct Token(Zeroizing<[u8; 32]>);

impl Token {
    pub fn generate() -> io::Result<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut()).map_err(|e| io::Error::other(format!("no secure randomness: {e}")))?;
        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Parse 64 hex digits (used only for the test override, see `token`).
    pub fn from_hex(text: &str) -> Option<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(text.trim(), bytes.as_mut()).ok()?;
        Some(Self(bytes))
    }

    pub fn to_hex(&self) -> Zeroizing<String> {
        Zeroizing::new(hex::encode(self.0.as_ref()))
    }

    fn proof(&self, context: &str, server_nonce: &[u8; 32], client_nonce: &[u8; 32]) -> blake3::Hash {
        let key = Zeroizing::new(blake3::derive_key(context, self.0.as_ref()));
        let mut mac = blake3::Hasher::new_keyed(&key);
        mac.update(server_nonce);
        mac.update(client_nonce);
        mac.finalize()
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[derive(Serialize, Deserialize)]
struct ServerHello {
    magic: [u8; 8],
    version: u16,
    nonce: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct ClientProof {
    version: u16,
    nonce: [u8; 32],
    proof: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct ServerProof {
    proof: [u8; 32],
}

fn nonce() -> io::Result<[u8; 32]> {
    let mut n = [0u8; 32];
    getrandom::fill(&mut n).map_err(|e| io::Error::other(format!("no secure randomness: {e}")))?;
    Ok(n)
}

fn denied(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, msg.to_string())
}

/// Daemon side. On error the caller must close the connection without
/// sending anything else. Callers bound this with a timeout.
pub async fn server_handshake<S: AsyncRead + AsyncWrite + Unpin + ?Sized>(s: &mut S, token: &Token) -> io::Result<()> {
    let server_nonce = nonce()?;
    send(s, &ServerHello { magic: MAGIC, version: PROTOCOL_VERSION, nonce: server_nonce }, HANDSHAKE_MAX).await?;
    let hello: ClientProof = recv(s, HANDSHAKE_MAX).await?;
    if hello.version != PROTOCOL_VERSION {
        return Err(denied("the client speaks another protocol version"));
    }
    // blake3::Hash compares in constant time.
    if token.proof(CLIENT_CONTEXT, &server_nonce, &hello.nonce) != hello.proof {
        return Err(denied("the client does not know the token"));
    }
    let proof = *token.proof(SERVER_CONTEXT, &server_nonce, &hello.nonce).as_bytes();
    send(s, &ServerProof { proof }, HANDSHAKE_MAX).await
}

/// Client side. Succeeds only if the other end proved it knows the token.
pub async fn client_handshake<S: AsyncRead + AsyncWrite + Unpin + ?Sized>(s: &mut S, token: &Token) -> io::Result<()> {
    let hello: ServerHello = recv(s, HANDSHAKE_MAX).await?;
    if hello.magic != MAGIC {
        return Err(denied("the program on the endpoint is not tetherd"));
    }
    if hello.version != PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("tetherd speaks control protocol v{}, this client v{PROTOCOL_VERSION}; update both", hello.version),
        ));
    }
    let client_nonce = nonce()?;
    let proof = *token.proof(CLIENT_CONTEXT, &hello.nonce, &client_nonce).as_bytes();
    send(s, &ClientProof { version: PROTOCOL_VERSION, nonce: client_nonce, proof }, HANDSHAKE_MAX).await?;
    let reply: ServerProof = match recv(s, HANDSHAKE_MAX).await {
        Ok(reply) => reply,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(denied("tetherd refused this client's token (was the daemon restarted?)"));
        }
        Err(e) => return Err(e),
    };
    if token.proof(SERVER_CONTEXT, &hello.nonce, &client_nonce) != reply.proof {
        return Err(denied("the program on the endpoint does not know the token; it is not your tetherd"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt};

    #[tokio::test]
    async fn both_sides_accept_when_they_share_the_token() {
        let token = Token::generate().unwrap();
        let (mut c, mut s) = duplex(1024);
        let t = token.clone();
        let server = tokio::spawn(async move { server_handshake(&mut s, &t).await });
        client_handshake(&mut c, &token).await.unwrap();
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_wrong_token_gets_nothing_after_the_hello() {
        let (mut c, mut s) = duplex(1024);
        let server = tokio::spawn(async move {
            let r = server_handshake(&mut s, &Token::generate().unwrap()).await;
            drop(s); // what the daemon does on failure
            r
        });
        let e = client_handshake(&mut c, &Token::generate().unwrap()).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(server.await.unwrap().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let mut rest = Vec::new();
        c.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "the server sent something after refusing");
    }

    #[tokio::test]
    async fn a_server_that_does_not_know_the_token_is_refused() {
        // A squatter: says hello, accepts any proof, answers with a guess.
        let (mut c, mut s) = duplex(1024);
        tokio::spawn(async move {
            send(&mut s, &ServerHello { magic: MAGIC, version: PROTOCOL_VERSION, nonce: [7; 32] }, 128).await.unwrap();
            let _: ClientProof = recv(&mut s, 128).await.unwrap();
            send(&mut s, &ServerProof { proof: [9; 32] }, 128).await.unwrap();
            // Keep the stream open so the client's verdict is about the proof.
            let _ = s.read_u8().await;
        });
        let e = client_handshake(&mut c, &Token::generate().unwrap()).await.unwrap_err();
        assert!(e.to_string().contains("not your tetherd"), "{e}");
    }

    #[tokio::test]
    async fn a_client_proof_is_not_a_server_proof() {
        // Reflecting the client's own proof back must not pass.
        let (mut c, mut s) = duplex(1024);
        tokio::spawn(async move {
            send(&mut s, &ServerHello { magic: MAGIC, version: PROTOCOL_VERSION, nonce: [1; 32] }, 128).await.unwrap();
            let p: ClientProof = recv(&mut s, 128).await.unwrap();
            send(&mut s, &ServerProof { proof: p.proof }, 128).await.unwrap();
            let _ = s.read_u8().await;
        });
        assert!(client_handshake(&mut c, &Token::generate().unwrap()).await.is_err());
    }

    #[tokio::test]
    async fn a_silent_client_gets_only_the_hello() {
        let (mut c, mut s) = duplex(1024);
        let token = Token::generate().unwrap();
        let server = tokio::spawn(async move { server_handshake(&mut s, &token).await });
        let hello: ServerHello = recv(&mut c, 128).await.unwrap();
        assert_eq!((hello.magic, hello.version), (MAGIC, PROTOCOL_VERSION));
        drop(c);
        assert!(server.await.unwrap().is_err());
    }

    #[test]
    fn tokens_are_never_printed_and_parse_strictly() {
        let t = Token::generate().unwrap();
        assert_eq!(format!("{t:?}"), "Token(<redacted>)");
        assert_eq!(Token::from_hex(&t.to_hex()).unwrap().as_bytes(), t.as_bytes());
        assert!(Token::from_hex("abcd").is_none());
        assert!(Token::from_hex(&"zz".repeat(32)).is_none());
    }
}
