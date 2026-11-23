//! LAN discovery over mDNS. Every daemon advertises `_tether._udp` with its
//! device id and QUIC port, and browses for others.

use std::net::SocketAddr;

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use quinn::Endpoint;
use tether_core::DeviceId;
use tracing::{info, warn};

use crate::ctx::Ctx;
use crate::net;

const SERVICE: &str = "_tether._udp.local.";

pub fn start(ctx: Ctx, endpoint: Endpoint, port: u16) -> Result<ServiceDaemon> {
    let mdns = ServiceDaemon::new()?;
    let me = ctx.me();
    let instance = me.0.simple().to_string();
    let host = format!("tether-{}.local.", &instance[..8]);
    let id = me.0.to_string();
    let props = [("id", id.as_str())];
    let info = ServiceInfo::new(SERVICE, &instance, &host, "", port, &props[..])?.enable_addr_auto();
    mdns.register(info)?;

    let events = mdns.browse(SERVICE)?;
    let runtime = tokio::runtime::Handle::current();
    std::thread::Builder::new().name("mdns".into()).spawn(move || {
        while let Ok(event) = events.recv() {
            let ServiceEvent::ServiceResolved(info) = event else { continue };
            let Some(peer) = info.get_property_val_str("id").and_then(|s| s.parse::<DeviceId>().ok()) else {
                continue;
            };
            // Exactly one side dials (the lower id), so a pair of devices
            // never opens two connections to each other.
            if peer <= me {
                continue;
            }
            let Some(ip) = info.get_addresses().iter().find(|ip| ip.is_ipv4()).copied() else { continue };
            {
                let mut s = ctx.state.lock().unwrap();
                if s.peers.contains_key(&peer) || !s.dialing.insert(peer) {
                    continue;
                }
            }
            let addr = SocketAddr::new(ip, info.get_port());
            info!(%peer, %addr, "discovered peer");
            let (ctx, endpoint) = (ctx.clone(), endpoint.clone());
            runtime.spawn(async move {
                if let Err(e) = net::connect(ctx.clone(), endpoint, addr).await {
                    warn!(%peer, error = %e, "could not connect");
                }
                // Connection ended or failed: allow rediscovery to dial again.
                ctx.state.lock().unwrap().dialing.remove(&peer);
            });
        }
    })?;
    Ok(mdns)
}
