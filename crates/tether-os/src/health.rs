//! How this computer is doing, for Pulse: CPU, memory, the disk that holds
//! the user's files, battery, uptime, the OS name, and the hardware addresses
//! of its network adapters (for Wake-on-LAN).
//!
//! CPU, memory, disks, uptime and adapters come from `sysinfo` on every OS.
//! Battery is read here: `GetSystemPowerStatus` on Windows, sysfs on Linux;
//! macOS reports none yet.

use std::net::IpAddr;
use std::path::PathBuf;

use sysinfo::{Disks, Networks, RefreshKind, System};

/// One reading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sample {
    /// Tenths of a percent, all cores together.
    pub cpu_permille: u16,
    pub mem_used: u64,
    pub mem_total: u64,
    pub disk_free: u64,
    pub disk_total: u64,
    /// Percent and whether it is charging; `None` for a desktop.
    pub battery: Option<(u8, bool)>,
    pub uptime_s: u64,
    pub os: String,
    /// Adapters with an address on a network, most likely first.
    pub macs: Vec<[u8; 6]>,
}

/// Keeps what `sysinfo` needs between readings (CPU use is the change
/// between two of them, so the first reading after start says 0).
pub struct Sampler {
    system: Option<System>,
    disks: Option<Disks>,
    networks: Option<Networks>,
    os: String,
}

impl Sampler {
    pub fn new() -> Self {
        let system = System::new_with_specifics(RefreshKind::nothing()).ok();
        let os = System::long_os_version().or_else(|_| System::name()).unwrap_or_default();
        Self {
            system,
            disks: Disks::new_with_refreshed_list().ok(),
            networks: Networks::new_with_refreshed_list().ok(),
            os,
        }
    }

    /// Take a reading. Blocking for a few milliseconds (disks especially):
    /// call it off the async runtime.
    pub fn sample(&mut self) -> Sample {
        let (cpu, mem_used, mem_total) = match self.system.as_mut() {
            Some(system) => {
                system.refresh_cpu_usage();
                system.refresh_memory();
                (system.global_cpu_usage().clamp(0.0, 100.0), system.used_memory(), system.total_memory())
            }
            None => (0.0, 0, 0),
        };
        let (disk_free, disk_total) = self.home_disk();
        Sample {
            cpu_permille: (cpu * 10.0).round() as u16,
            mem_used: mem_used.min(mem_total),
            mem_total,
            disk_free: disk_free.min(disk_total),
            disk_total,
            battery: battery(),
            uptime_s: System::uptime().unwrap_or(0),
            os: self.os.clone(),
            macs: self.macs(),
        }
    }

    /// Free and total space on the disk that holds the user's home folder.
    fn home_disk(&mut self) -> (u64, u64) {
        let Some(disks) = self.disks.as_mut() else { return (0, 0) };
        disks.refresh(true);
        let home = home_dir();
        disks
            .iter()
            .filter(|d| home.as_ref().map_or(true, |h| h.starts_with(d.mount_point())))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .map_or((0, 0), |d| (d.available_space(), d.total_space()))
    }

    /// Adapters that have an IPv4 address on a private network (where a wake
    /// packet can reach them), real hardware addresses only.
    fn macs(&mut self) -> Vec<[u8; 6]> {
        let Some(networks) = self.networks.as_mut() else { return Vec::new() };
        networks.refresh(true);
        let mut out: Vec<[u8; 6]> = Vec::new();
        for data in networks.values() {
            let mac = data.mac_address().0;
            let on_lan = data.ip_networks().iter().any(|n| matches!(n.addr, IpAddr::V4(v4) if v4.is_private()));
            if on_lan && real_mac(mac) && !out.contains(&mac) {
                out.push(mac);
            }
        }
        out.sort_unstable();
        out.truncate(4);
        out
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

/// Not zero, broadcast, multicast or locally administered (virtual adapters,
/// randomised Wi-Fi addresses).
fn real_mac(mac: [u8; 6]) -> bool {
    mac != [0; 6] && mac != [0xFF; 6] && mac[0] & 0b11 == 0
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[cfg(windows)]
fn battery() -> Option<(u8, bool)> {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: SYSTEM_POWER_STATUS is plain data, so all-zero is a valid value.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: `status` is a valid, writable SYSTEM_POWER_STATUS for the call.
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return None;
    }
    // 128: no system battery; 255: unknown. 255 percent: unknown.
    if status.BatteryFlag & 128 != 0 || status.BatteryFlag == 255 || status.BatteryLifePercent > 100 {
        return None;
    }
    Some((status.BatteryLifePercent, status.BatteryFlag & 8 != 0))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn battery() -> Option<(u8, bool)> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" {
            continue;
        }
        let percent: u8 = std::fs::read_to_string(path.join("capacity")).ok()?.trim().parse().ok()?;
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        return Some((percent.min(100), status.trim() == "Charging"));
    }
    None
}

#[cfg(target_os = "macos")]
fn battery() -> Option<(u8, bool)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_is_sane() {
        let mut s = Sampler::new();
        let _ = s.sample();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let r = s.sample();
        assert!(r.cpu_permille <= 1000);
        assert!(r.mem_total > 0 && r.mem_used <= r.mem_total, "{r:?}");
        assert!(r.disk_total > 0 && r.disk_free <= r.disk_total, "{r:?}");
        assert!(r.uptime_s > 0);
        assert!(!r.os.is_empty());
        assert!(r.macs.iter().all(|m| real_mac(*m)) && r.macs.len() <= 4);
        if let Some((percent, _)) = r.battery {
            assert!(percent <= 100);
        }
    }
}
