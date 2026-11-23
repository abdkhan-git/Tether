//! LAN discovery over mDNS. Every daemon advertises `_tether._udp` with its
//! device key and QUIC port, and remembers who else it sees. Discovery only
//! finds addresses: whether to trust a device is decided by its key during
//! the TLS handshake, never by what it advertises.
//!
//! Privacy note: the advertisement reveals that this machine runs Tether and
//! its public device key to everyone on the LAN (SECURITY.md, residual risks).

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tether_core::DeviceId;
use tracing::debug;

use crate::ctx::Ctx;

const SERVICE: &str = "_tether._udp.local.";

pub fn start(ctx: Ctx, port: u16) -> Result<ServiceDaemon> {
    let mdns = ServiceDaemon::new()?;
    let me = ctx.me();
    let id = me.to_hex();
    let host = format!("tether-{}.local.", &id[..12]);
    let props = [("id", id.as_str())];
    let info = ServiceInfo::new(SERVICE, &id[..16], &host, "", port, &props[..])?.enable_addr_auto();
    mdns.register(info)?;

    let events = mdns.browse(SERVICE)?;
    std::thread::Builder::new().name("mdns".into()).spawn(move || {
        while let Ok(event) = events.recv() {
            let ServiceEvent::ServiceResolved(info) = event else { continue };
            let Some(peer) = info.get_property_val_str("id").and_then(|s| s.parse::<DeviceId>().ok()) else { continue };
            if peer == me {
                continue;
            }
            let Some(ip) = info.get_addresses().iter().find(|ip| ip.is_ipv4()).copied() else { continue };
            let addr = std::net::SocketAddr::new(ip, info.get_port());
            debug!(%peer, %addr, "seen on the network");
            ctx.lock().nearby.insert(peer, addr);
        }
    })?;
    Ok(mdns)
}
