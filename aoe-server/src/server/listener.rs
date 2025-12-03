//! Ethernet listener for AoE frames
//!
//! Uses pnet to receive and send raw Ethernet frames.

use crate::protocol::{build_response, parse_frame, AoeError, AOE_ETHERTYPE, BROADCAST_MAC};
use crate::server::TargetManager;
use pnet::datalink::{self, Channel, DataLinkReceiver, DataLinkSender, NetworkInterface};
use std::sync::{Arc, Mutex};

/// AoE network listener
pub struct AoeListener {
    interface: NetworkInterface,
    tx: Arc<Mutex<Box<dyn DataLinkSender>>>,
    rx: Box<dyn DataLinkReceiver>,
    targets: Arc<Mutex<TargetManager>>,
}

impl AoeListener {
    /// Create a new listener on the specified interface
    pub fn new(interface_name: &str, targets: TargetManager) -> Result<Self, AoeError> {
        let interfaces = datalink::interfaces();
        let interface = interfaces
            .into_iter()
            .find(|iface| iface.name == interface_name)
            .ok_or_else(|| {
                AoeError::BadArgument(format!("interface not found: {}", interface_name))
            })?;

        let (tx, rx) = match datalink::channel(&interface, Default::default()) {
            Ok(Channel::Ethernet(tx, rx)) => (tx, rx),
            Ok(_) => {
                return Err(AoeError::BadArgument(
                    "unsupported channel type".to_string(),
                ))
            }
            Err(e) => {
                return Err(AoeError::BadArgument(format!(
                    "failed to open channel: {}",
                    e
                )))
            }
        };

        Ok(Self {
            interface,
            tx: Arc::new(Mutex::new(tx)),
            rx,
            targets: Arc::new(Mutex::new(targets)),
        })
    }

    /// Run the main receive loop
    pub fn run(&mut self) -> Result<(), AoeError> {
        log::info!(
            "AoE server listening on {} ({})",
            self.interface.name,
            self.interface
                .mac
                .map(|m| m.to_string())
                .unwrap_or_else(|| "no MAC".to_string())
        );

        loop {
            match self.rx.next() {
                Ok(packet) => {
                    // Copy packet to owned buffer to avoid borrow issues
                    let packet = packet.to_vec();
                    if let Err(e) = self.handle_packet(&packet) {
                        log::warn!("Error handling packet: {}", e);
                    }
                }
                Err(e) => {
                    log::error!("Error receiving packet: {}", e);
                }
            }
        }
    }

    /// Handle a received packet
    fn handle_packet(&self, packet: &[u8]) -> Result<(), AoeError> {
        // Check minimum size and EtherType
        if packet.len() < 14 {
            return Ok(()); // Too short, ignore
        }

        let ethertype = u16::from_be_bytes([packet[12], packet[13]]);
        if ethertype != AOE_ETHERTYPE {
            return Ok(()); // Not AoE, ignore
        }

        // Parse the frame
        let frame = parse_frame(packet)?;

        // Skip responses
        if frame.header.flags.response {
            return Ok(());
        }

        log::debug!(
            "Received AoE frame: shelf={} slot={} cmd={:?} tag={}",
            frame.header.shelf,
            frame.header.slot,
            frame.header.command,
            frame.header.tag
        );

        // Check if we have a target for this address
        let mut targets = self.targets.lock().unwrap();
        let responses = targets.handle_frame(&frame)?;

        // Send responses
        let mut tx = self.tx.lock().unwrap();
        for response_data in responses {
            let response_frame = build_response(&frame, response_data);
            if let Some(ref err) = tx.send_to(&response_frame, None) {
                log::warn!("Error sending response: {:?}", err);
            }
        }

        Ok(())
    }

    /// Get the local MAC address
    pub fn local_mac(&self) -> Option<[u8; 6]> {
        self.interface.mac.map(|m| m.octets())
    }
}

/// Check if a MAC address is broadcast
#[allow(dead_code)]
pub fn is_broadcast_mac(mac: &[u8; 6]) -> bool {
    mac == &BROADCAST_MAC
}
