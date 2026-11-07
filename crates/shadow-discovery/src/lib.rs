//! Local-network discovery.
//!
//! Receiving devices broadcast a small signed-format beacon (not authenticated;
//! the pairing proof later binds the real certificate fingerprint) only while
//! receiving is enabled. Sending devices listen for a short, explicit discovery
//! window rather than scanning continuously. Manual IP entry always works when
//! broadcast is blocked by a router or firewall.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use shadow_protocol::{validate_device_name, validate_fingerprint};
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

pub const DISCOVERY_PORT: u16 = 47830;
pub const BEACON_MAGIC: &str = "SHADOW-TRANSFER-BEACON/1";
pub const MAX_BEACON_BYTES: usize = 1024;

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid beacon")]
    InvalidBeacon,
    #[error("discovery socket unavailable: {0}")]
    SocketUnavailable(String),
    #[error("invalid advertisement: {0}")]
    InvalidAdvertisement(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Beacon {
    pub magic: String,
    pub name: String,
    pub port: u16,
    pub fingerprint: String,
    pub pairing: bool,
}

impl Beacon {
    pub fn validate(&self) -> Result<(), DiscoveryError> {
        if self.magic != BEACON_MAGIC
            || self.port == 0
            || validate_device_name(&self.name).is_err()
            || validate_fingerprint(&self.fingerprint).is_err()
        {
            return Err(DiscoveryError::InvalidBeacon);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AdvertiseConfig {
    pub name: String,
    pub port: u16,
    pub fingerprint: String,
    pub pairing: bool,
    pub interval: Duration,
}

impl AdvertiseConfig {
    pub fn new(name: impl Into<String>, port: u16, fingerprint: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            port,
            fingerprint: fingerprint.into(),
            pairing: true,
            interval: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub name: String,
    pub addr: SocketAddr,
    pub fingerprint: String,
    pub pairing: bool,
    pub last_seen: Instant,
}

/// Sends beacons to every IPv4 broadcast domain until cancelled.
pub async fn advertise(
    config: AdvertiseConfig,
    cancel: CancellationToken,
) -> Result<(), DiscoveryError> {
    let beacon = Beacon {
        magic: BEACON_MAGIC.to_string(),
        name: config.name.clone(),
        port: config.port,
        fingerprint: config.fingerprint.clone(),
        pairing: config.pairing,
    };
    beacon.validate().map_err(|e| match e {
        DiscoveryError::InvalidBeacon => {
            DiscoveryError::InvalidAdvertisement("beacon fields invalid".into())
        }
        other => other,
    })?;
    let payload = serde_json::to_vec(&beacon)
        .map_err(|e| DiscoveryError::InvalidAdvertisement(e.to_string()))?;
    if payload.len() > MAX_BEACON_BYTES {
        return Err(DiscoveryError::InvalidAdvertisement(
            "beacon too large".into(),
        ));
    }

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;
    let targets = broadcast_targets();
    info!(targets = ?targets, port = config.port, "advertising receive availability");

    loop {
        for target in &targets {
            let destination = SocketAddr::from((*target, DISCOVERY_PORT));
            if let Err(err) = socket.send_to(&payload, destination).await {
                debug!(target = %destination, error = %err, "beacon send failed");
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(config.interval) => {}
        }
    }
}

/// Listens for beacons for at most `window`, returning the newest sighting of
/// each device. No continuous background scanning.
pub async fn discover_for(
    window: Duration,
    cancel: CancellationToken,
) -> Result<Vec<DiscoveredDevice>, DiscoveryError> {
    let socket = bind_listener()?;
    let mut seen: HashMap<String, DiscoveredDevice> = HashMap::new();
    let deadline = tokio::time::Instant::now() + window;
    let mut buffer = [0u8; MAX_BEACON_BYTES];

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::select! {
            _ = cancel.cancelled() => break,
            received = tokio::time::timeout(remaining, socket.recv_from(&mut buffer)) => {
                match received {
                    Err(_) => break,
                    Ok(Err(err)) => return Err(DiscoveryError::Io(err)),
                    Ok(Ok((len, source))) => {
                        if len > MAX_BEACON_BYTES {
                            continue;
                        }
                        let Ok(beacon) = serde_json::from_slice::<Beacon>(&buffer[..len]) else {
                            debug!(peer = %source, "ignoring malformed beacon");
                            continue;
                        };
                        if beacon.validate().is_err() {
                            continue;
                        }
                        let addr = SocketAddr::from((source.ip(), beacon.port));
                        seen.insert(
                            beacon.fingerprint.clone(),
                            DiscoveredDevice {
                                name: beacon.name,
                                addr,
                                fingerprint: beacon.fingerprint,
                                pairing: beacon.pairing,
                                last_seen: Instant::now(),
                            },
                        );
                    }
                }
            }
        }
    }

    let mut devices: Vec<_> = seen.into_values().collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

fn bind_listener() -> Result<UdpSocket, DiscoveryError> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from(([0, 0, 0, 0], DISCOVERY_PORT)).into())?;
    socket.set_nonblocking(true)?;
    let std_socket: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(std_socket).map_err(|e| DiscoveryError::SocketUnavailable(e.to_string()))
}

pub fn broadcast_targets() -> Vec<Ipv4Addr> {
    let mut targets = HashSet::new();
    targets.insert(Ipv4Addr::BROADCAST);
    // Loopback keeps discovery working on one machine for testing and for
    // desktop-to-desktop use.
    targets.insert(Ipv4Addr::LOCALHOST);
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for interface in interfaces {
            if let if_addrs::IfAddr::V4(v4) = interface.addr {
                if !v4.ip.is_loopback() {
                    let ip = u32::from(v4.ip);
                    let mask = u32::from(v4.netmask);
                    targets.insert(Ipv4Addr::from(ip | !mask));
                }
            }
        }
    }
    let mut result: Vec<_> = targets.into_iter().collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beacon_json() -> Vec<u8> {
        serde_json::to_vec(&Beacon {
            magic: BEACON_MAGIC.into(),
            name: "Tab".into(),
            port: 47831,
            fingerprint: "ab".repeat(32),
            pairing: true,
        })
        .unwrap()
    }

    #[test]
    fn beacon_validation_is_strict() {
        let beacon: Beacon = serde_json::from_slice(&beacon_json()).unwrap();
        beacon.validate().unwrap();
        let mut bad = beacon.clone();
        bad.fingerprint = "nope".into();
        assert!(bad.validate().is_err());
        let mut bad = beacon.clone();
        bad.magic = "OTHER".into();
        assert!(bad.validate().is_err());
        let unknown = br#"{"magic":"SHADOW-TRANSFER-BEACON/1","name":"x","port":1,"fingerprint":"ab","pairing":true,"extra":1}"#;
        assert!(serde_json::from_slice::<Beacon>(unknown).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn advertises_and_discovers_on_loopback() {
        let port = 47831;
        let fingerprint = "cd".repeat(32);
        let config = AdvertiseConfig {
            name: "Loopback Tablet".into(),
            port,
            fingerprint: fingerprint.clone(),
            pairing: true,
            interval: Duration::from_millis(200),
        };
        let cancel = CancellationToken::new();
        let advertiser = tokio::spawn(advertise(config, cancel.clone()));
        let devices = discover_for(Duration::from_secs(3), CancellationToken::new())
            .await
            .unwrap();
        cancel.cancel();
        advertiser.await.unwrap().unwrap();
        assert!(
            devices.iter().any(|d| d.fingerprint == fingerprint),
            "advertised device not found: {devices:?}"
        );
        assert_eq!(
            devices
                .iter()
                .find(|d| d.fingerprint == fingerprint)
                .unwrap()
                .addr
                .port(),
            port
        );
    }
}
