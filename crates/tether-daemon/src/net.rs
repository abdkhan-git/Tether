//! QUIC transport. Each request is one bi-directional stream: write a
//! `Message`, finish, read the reply. Streams are independent, so a slow
//! request never blocks another (no head-of-line blocking).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use quinn::crypto::rustls::QuicClientConfig;
use quinn::{ClientConfig, Connection, Endpoint, RecvStream, SendStream, ServerConfig, TransportConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tether_core::Message;
use tracing::{info, warn};

use crate::ctx::Ctx;

/// One endpoint both accepts and dials, on a single UDP port.
pub fn make_endpoint(port: u16) -> Result<Endpoint> {
    let mut transport = TransportConfig::default();
    // Keep idle connections alive so the next copy is one hop away.
    transport.keep_alive_interval(Some(Duration::from_secs(5)));
    let transport = Arc::new(transport);

    let cert = rcgen::generate_simple_self_signed(vec!["tether".to_string()])?;
    let cert_der: CertificateDer<'static> = cert.cert.der().clone();
    let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
    let mut server_config = ServerConfig::with_single_cert(vec![cert_der], key.into())?;
    server_config.transport_config(transport.clone());

    let mut endpoint = Endpoint::server(server_config, SocketAddr::from(([0, 0, 0, 0], port)))?;

    // TODO(stage 1, pairing): replace with Noise-based pairing and pinned device keys.
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(AcceptAnyCert::new())
        .with_no_client_auth();
    let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(crypto)?));
    client_config.transport_config(transport);
    endpoint.set_default_client_config(client_config);
    Ok(endpoint)
}

pub async fn accept_loop(ctx: Ctx, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => {
                    info!(addr = %conn.remote_address(), "accepted connection");
                    run_connection(ctx, conn).await;
                }
                Err(e) => warn!(error = %e, "incoming connection failed"),
            }
        });
    }
}

pub async fn connect(ctx: Ctx, endpoint: Endpoint, addr: SocketAddr) -> Result<()> {
    let conn = endpoint.connect(addr, "tether")?.await.with_context(|| format!("connecting to {addr}"))?;
    info!(%addr, "connected");
    run_connection(ctx, conn).await;
    Ok(())
}

/// Serve one connection until it closes.
async fn run_connection(ctx: Ctx, conn: Connection) {
    // Introduce ourselves and pull anything we missed while apart.
    {
        let (ctx, conn) = (ctx.clone(), conn.clone());
        tokio::spawn(async move {
            if let Err(e) = pull(&ctx, &conn).await {
                warn!(error = %e, "initial sync failed");
            }
        });
    }

    loop {
        match conn.accept_bi().await {
            Ok((send, recv)) => {
                let (ctx, conn) = (ctx.clone(), conn.clone());
                tokio::spawn(async move {
                    if let Err(e) = handle_stream(&ctx, &conn, send, recv).await {
                        warn!(error = %e, "stream failed");
                    }
                });
            }
            Err(e) => {
                info!(addr = %conn.remote_address(), reason = %e, "connection closed");
                break;
            }
        }
    }

    let id = conn.stable_id();
    ctx.state.lock().unwrap().peers.retain(|_, c| c.stable_id() != id);
}

/// Send one message and wait for its reply.
pub async fn request(conn: &Connection, msg: &Message) -> Result<Message> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&msg.encode()).await?;
    send.finish()?;
    let bytes = recv.read_to_end(Message::MAX_FRAME).await?;
    Ok(Message::decode(&bytes)?)
}

/// Send our version vector; the peer replies with what we are missing.
pub async fn pull(ctx: &Ctx, conn: &Connection) -> Result<()> {
    let hello = {
        let s = ctx.state.lock().unwrap();
        Message::Hello { device: s.me, name: s.name.clone(), vv: s.log.version_vector() }
    };
    match request(conn, &hello).await? {
        Message::Ops(ops) => {
            let fetched = ops.len();
            let outcome = ctx.apply(ops);
            info!(fetched, applied = outcome.applied, "caught up with peer");
            Ok(())
        }
        other => bail!("unexpected reply to Hello: {other:?}"),
    }
}

async fn handle_stream(ctx: &Ctx, conn: &Connection, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let bytes = recv.read_to_end(Message::MAX_FRAME).await?;
    let reply = match Message::decode(&bytes)? {
        Message::Hello { device, name, vv } => {
            let mut s = ctx.state.lock().unwrap();
            if s.peers.insert(device, conn.clone()).is_none() {
                info!(peer = %device, %name, "peer joined");
            }
            Message::Ops(s.log.missing_for(&vv))
        }
        Message::Ops(ops) => {
            let outcome = ctx.apply(ops);
            if outcome.gap {
                // We missed something earlier; ask for it in order.
                let (ctx, conn) = (ctx.clone(), conn.clone());
                tokio::spawn(async move {
                    if let Err(e) = pull(&ctx, &conn).await {
                        warn!(error = %e, "gap repair failed");
                    }
                });
            }
            Message::Ack { applied: outcome.applied }
        }
        Message::Ack { .. } => bail!("unexpected Ack on an incoming stream"),
    };
    send.write_all(&reply.encode()).await?;
    send.finish()?;
    Ok(())
}

/// Accepts any server certificate. Stage 1 only: the connection is encrypted
/// but not authenticated until pairing lands.
#[derive(Debug)]
struct AcceptAnyCert(Arc<rustls::crypto::CryptoProvider>);

impl AcceptAnyCert {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
