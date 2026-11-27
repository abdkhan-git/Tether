//! TLS 1.3 for QUIC, authenticated by pinned device keys.
//!
//! Each device presents a self-signed certificate holding its Ed25519 device
//! key. There is no certificate authority: a peer is accepted only if the key
//! in its certificate is a trusted group member (or, while a pairing window is
//! open, so that the pairing ceremony can run). Both sides authenticate
//! (mutual TLS), so a stranger's handshake fails before any application data.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{IdleTimeout, TransportConfig, VarInt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, Error as TlsError, SignatureScheme};
use tether_core::limits::MAX_FRAME;
use tether_core::{DeviceId, DeviceKey};
use zeroize::Zeroizing;

pub const ALPN: &[u8] = b"tether/1";

/// Who may complete a handshake right now. Updated whenever membership changes.
#[derive(Debug, Default)]
pub struct Gate {
    pub trusted: HashSet<DeviceId>,
    /// While set and in the future, unknown devices may connect, but only to
    /// attempt pairing. Checked against the clock on every handshake, so the
    /// window closes on time even if nothing else happens.
    pub pairing_until: Option<Instant>,
}

impl Gate {
    pub fn pairing_open(&self) -> bool {
        self.pairing_until.is_some_and(|t| Instant::now() < t)
    }
}

pub type SharedGate = Arc<RwLock<Gate>>;

/// This device's certificate and key for TLS.
pub struct TlsIdentity {
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    /// Key for QUIC stateless reset tokens (RFC 9000 §10.3), derived from the
    /// device key so it survives a restart: when a peer keeps talking to us
    /// over a connection that died with the old process, the new process can
    /// prove the connection is gone and the peer reconnects at once, instead
    /// of waiting out the idle timeout. Anyone holding it could only reset
    /// our connections, and deriving it reveals nothing about the device key.
    pub reset_key: Arc<dyn quinn::crypto::HmacKey>,
    /// Key of the connection ID generator, also stable across restarts.
    /// quinn checks a packet's connection ID against it before considering a
    /// stateless reset, so with a fresh random key per process the restarted
    /// daemon would silently drop the old connection's packets instead.
    /// Not a secret: it only filters out stray packets cheaply.
    pub cid_key: u64,
}

/// Domain separation for the keys derived from the device key.
const RESET_KEY_CONTEXT: &str = "tether 2026-11-23 quic stateless reset key v1";
const CID_KEY_CONTEXT: &str = "tether 2026-11-23 quic connection id key v1";

fn reset_key(key: &DeviceKey) -> Arc<dyn quinn::crypto::HmacKey> {
    let derived = Zeroizing::new(blake3::derive_key(RESET_KEY_CONTEXT, key.secret().as_ref()));
    Arc::new(ring::hmac::Key::new(ring::hmac::HMAC_SHA256, derived.as_ref()))
}

fn cid_key(key: &DeviceKey) -> u64 {
    let derived = Zeroizing::new(blake3::derive_key(CID_KEY_CONTEXT, key.secret().as_ref()));
    u64::from_le_bytes(derived[..8].try_into().expect("8 bytes"))
}

/// PKCS#8 v1 encoding of an Ed25519 private key (RFC 8410).
fn ed25519_pkcs8(secret: &[u8; 32]) -> Zeroizing<Vec<u8>> {
    const PREFIX: [u8; 16] =
        [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];
    let mut der = Zeroizing::new(Vec::with_capacity(48));
    der.extend_from_slice(&PREFIX);
    der.extend_from_slice(secret);
    der
}

pub fn identity(key: &DeviceKey) -> Result<TlsIdentity> {
    let der = ed25519_pkcs8(&key.secret());
    let pkcs8 = PrivatePkcs8KeyDer::from(der.to_vec());
    let pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8, &rcgen::PKCS_ED25519)
        .context("loading the device key for TLS")?;
    let cert = rcgen::CertificateParams::new(vec!["tether".to_string()])?
        .self_signed(&pair)
        .context("creating the device certificate")?;
    let id = TlsIdentity {
        cert: cert.der().clone(),
        key: PrivateKeyDer::Pkcs8(pkcs8),
        reset_key: reset_key(key),
        cid_key: cid_key(key),
    };
    if device_id_of(&id.cert) != Some(key.id()) {
        return Err(anyhow!("the device certificate does not carry the device key"));
    }
    Ok(id)
}

/// The device key inside a certificate. Only Ed25519 certificates qualify.
///
/// Parsed with a real X.509 parser: the key must come from the certificate's
/// SubjectPublicKeyInfo, the same field TLS uses to check the handshake
/// signature, never from anywhere else in the certificate.
pub fn device_id_of(cert: &CertificateDer<'_>) -> Option<DeviceId> {
    let (rest, parsed) = x509_parser::parse_x509_certificate(cert.as_ref()).ok()?;
    if !rest.is_empty() {
        return None;
    }
    let spki = &parsed.tbs_certificate.subject_pki;
    if spki.algorithm.algorithm != x509_parser::oid_registry::OID_SIG_ED25519 {
        return None;
    }
    let key: [u8; 32] = spki.subject_public_key.data.as_ref().try_into().ok()?;
    Some(DeviceId(key))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn verify_ed25519(
    provider: &CryptoProvider,
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, TlsError> {
    if dss.scheme != SignatureScheme::ED25519 {
        return Err(TlsError::PeerMisbehaved(rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme));
    }
    rustls::crypto::verify_tls13_signature(message, cert, dss, &provider.signature_verification_algorithms)
}

fn untrusted() -> TlsError {
    TlsError::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
}

/// Server side: decides which clients may connect.
#[derive(Debug)]
struct ClientVerifier {
    gate: SharedGate,
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for ClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        let id = device_id_of(end_entity).ok_or(TlsError::InvalidCertificate(CertificateError::BadEncoding))?;
        let gate = self.gate.read().map_err(|_| untrusted())?;
        if intermediates.is_empty() && (gate.trusted.contains(&id) || gate.pairing_open()) {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(untrusted())
        }
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 is not supported".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_ed25519(&self.provider, message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// What a client expects of the server it dials.
#[derive(Debug, Clone, Copy)]
pub enum Expect {
    /// Exactly this trusted device.
    Device(DeviceId),
    /// Any trusted device (dialing an address given on the command line).
    AnyTrusted,
    /// Anyone: the pairing ceremony authenticates the other side afterwards.
    Pairing,
}

/// Client side: decides which servers we accept.
#[derive(Debug)]
struct ServerVerifier {
    gate: SharedGate,
    provider: Arc<CryptoProvider>,
    expect: Expect,
}

impl ServerCertVerifier for ServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let id = device_id_of(end_entity).ok_or(TlsError::InvalidCertificate(CertificateError::BadEncoding))?;
        if !intermediates.is_empty() {
            return Err(untrusted());
        }
        let trusted = self.gate.read().map_err(|_| untrusted())?.trusted.contains(&id);
        let ok = match self.expect {
            Expect::Device(want) => id == want && trusted,
            Expect::AnyTrusted => trusted,
            Expect::Pairing => true,
        };
        if ok {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(untrusted())
        }
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 is not supported".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_ed25519(&self.provider, message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// Limits on every connection: few streams, bounded windows, idle timeout.
/// A keep-alive every 5 s; three missed ones (15 s) end the connection, so a
/// peer that vanished without closing (crash, sleep, cable) is noticed and
/// the reconnect loop takes over.
pub fn transport() -> Arc<TransportConfig> {
    let mut t = TransportConfig::default();
    t.keep_alive_interval(Some(Duration::from_secs(5)));
    t.max_idle_timeout(Some(IdleTimeout::from(VarInt::from_u32(15_000))));
    t.max_concurrent_bidi_streams(VarInt::from_u32(16));
    t.max_concurrent_uni_streams(VarInt::from_u32(0));
    t.stream_receive_window(VarInt::from_u32((MAX_FRAME + 64 * 1024) as u32));
    t.receive_window(VarInt::from_u32(16 << 20));
    t.send_window(16 << 20);
    // Heartbeats (pulse.rs) are datagrams: a few small ones a second.
    t.datagram_receive_buffer_size(Some(64 << 10));
    t.datagram_send_buffer_size(64 << 10);
    Arc::new(t)
}

pub fn server_config(id: &TlsIdentity, gate: SharedGate) -> Result<quinn::ServerConfig> {
    let provider = provider();
    let verifier = Arc::new(ClientVerifier { gate, provider: provider.clone() });
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![id.cert.clone()], id.key.clone_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    cfg.transport_config(transport());
    Ok(cfg)
}

pub fn client_config(id: &TlsIdentity, gate: SharedGate, expect: Expect) -> Result<quinn::ClientConfig> {
    let provider = provider();
    let verifier = Arc::new(ServerVerifier { gate, provider: provider.clone(), expect });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous() // our verifier pins device keys instead of trusting a CA
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![id.cert.clone()], id.key.clone_key())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut cfg = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    cfg.transport_config(transport());
    Ok(cfg)
}

/// The device key of whoever is on the other end of an established connection.
pub fn peer_id(conn: &quinn::Connection) -> Option<DeviceId> {
    let certs = conn.peer_identity()?.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().and_then(device_id_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_certificate_carries_the_device_key() {
        let key = DeviceKey::for_test(5);
        let id = identity(&key).unwrap();
        assert_eq!(device_id_of(&id.cert), Some(key.id()));
    }

    #[test]
    fn garbage_certificates_are_refused() {
        assert_eq!(device_id_of(&CertificateDer::from(vec![0u8; 64])), None);
        let mut der = identity(&DeviceKey::for_test(5)).unwrap().cert.to_vec();
        der.push(0); // trailing bytes
        assert_eq!(device_id_of(&CertificateDer::from(der)), None);
    }
}
