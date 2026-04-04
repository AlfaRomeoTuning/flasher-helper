use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use socketcan::{EmbeddedFrame, Frame};
use tokio::sync::RwLock;
use tokio::time::{sleep, timeout};
use tracing::{info, warn, error};
use anyhow::Result;

use crate::dbc::{lookup_message, MessageDef};

const VIN_FRAME_ID: u32 = 0x416;
const BATTERY_FRAME_ID: u32 = 0x41A;
const IGNITION_FRAME_ID: u32 = 0xFA;
const VIN_TIMEOUT: Duration = Duration::from_secs(60);
const STATUS_UPDATE_MIN_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StatusData {
    pub battery_voltage: Option<f32>,
    pub ignition_sts: Option<u8>,
    #[serde(skip)]
    pub last_battery_time: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_ignition_time: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuRole {
    Tx,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DecodedSignalValue {
    pub raw: i64,
    pub value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DecodedMessageSnapshot {
    pub frame_id: u32,
    pub message_name: String,
    pub dlc: u8,
    pub transmitter: String,
    pub timestamp_ms: u128,
    pub signals: BTreeMap<String, DecodedSignalValue>,
}

pub type InterfaceSnapshots = HashMap<u32, DecodedMessageSnapshot>;

pub struct SharedState {
    pub vin: RwLock<Option<String>>,
    pub status: RwLock<StatusData>,
    pub decoded: RwLock<HashMap<String, InterfaceSnapshots>>,
}

pub async fn run_can_listener(interface: &str, state: Arc<SharedState>) -> Result<()> {
    loop {
        info!("Opening SocketCAN interface: {}", interface);
        let tokio_socket = match socketcan::tokio::CanSocket::open(interface) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to open socket {}: {}. Retrying in 5s...", interface, e);
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        info!("Listening for VIN frames on ID 0x{VIN_FRAME_ID:X}...");
        
        let mut vin_buffer = vec![0u8; 17];
        let mut received_indices = [false; 3];
        let mut vin_search_start: Option<Instant> = None;
        let mut vin_cooldown_until: Option<Instant> = None;

        loop {
            // Check if VIN search has timed out (60 seconds without completing)
            if let Some(start) = vin_search_start {
                if received_indices.iter().any(|&r| r) && start.elapsed() >= VIN_TIMEOUT {
                    warn!("VIN search timeout after 60 seconds without completing all 3 packets");
                    let mut w = state.vin.write().await;
                    *w = None;
                    received_indices = [false; 3];
                    vin_search_start = None;
                }
            }

            // Use a short timeout (1 second) to allow VIN timeout check to run
            // while still processing status packets independently
            let frame_result = timeout(Duration::from_secs(1), tokio_socket.read_frame()).await;

            match frame_result {
                Ok(Ok(frame)) => {
                    let (id, data) = match frame {
                        socketcan::CanFrame::Data(f) => (f.raw_id(), f.data().to_vec()),
                        _ => continue,
                    };

                    match id {
                        VIN_FRAME_ID => {
                            if interface != "can-c1" {
                                continue;
                            }

                            if matches!(vin_cooldown_until, Some(until) if Instant::now() < until) {
                                continue;
                            }

                            if data.is_empty() {
                                warn!("Received VIN frame with empty payload");
                                continue;
                            }

                            let seq_index = data[0];
                            let vin_part = &data[1..];

                            // Track when VIN search starts after first valid sequence packet.
                            if vin_search_start.is_none() {
                                vin_search_start = Some(Instant::now());
                            }

                            match seq_index {
                                0 => {
                                    let len = vin_part.len().min(7);
                                    vin_buffer[0..len].copy_from_slice(&vin_part[..len]);
                                    received_indices[0] = true;
                                }
                                1 => {
                                    let len = vin_part.len().min(7);
                                    vin_buffer[7..7+len].copy_from_slice(&vin_part[..len]);
                                    received_indices[1] = true;
                                }
                                2 => {
                                    let len = vin_part.len().min(3);
                                    vin_buffer[14..14+len].copy_from_slice(&vin_part[..len]);
                                    received_indices[2] = true;
                                }
                                _ => {
                                    warn!("Received unexpected sequence index: {:02X}", seq_index);
                                    continue;
                                }
                            }

                            if received_indices.iter().all(|&r| r) {
                                let vin = String::from_utf8_lossy(&vin_buffer)
                                    .trim_matches(char::from(0))
                                    .to_string();
                                
                                if vin.len() == 17 {
                                    info!("VIN reconstructed: {}", vin);
                                    let mut w = state.vin.write().await;
                                    *w = Some(vin);
                                    vin_cooldown_until = Some(Instant::now() + VIN_TIMEOUT);
                                } else {
                                    warn!("Reconstructed VIN has invalid length: {} (expected 17)", vin.len());
                                }
                                
                                received_indices = [false; 3];
                                vin_search_start = None;
                            }
                        }
                        BATTERY_FRAME_ID => {
                            if interface != "can-c1" {
                                continue;
                            }

                            let mut status = state.status.write().await;
                            let should_update = status.last_battery_time
                                .map(|t| t.elapsed() >= STATUS_UPDATE_MIN_INTERVAL)
                                .unwrap_or(true);

                            if should_update && data.len() >= 2 {
                                let raw = ((data[0] as u16) << 1) | ((data[1] >> 7) as u16);
                                status.battery_voltage = Some((raw as f32) * 0.025 + 4.0);
                                status.last_battery_time = Some(std::time::Instant::now());
                            }
                        }
                        IGNITION_FRAME_ID => {
                            if interface != "can-c1" {
                                continue;
                            }

                            let mut status = state.status.write().await;
                            let should_update = status.last_ignition_time
                                .map(|t| t.elapsed() >= STATUS_UPDATE_MIN_INTERVAL)
                                .unwrap_or(true);

                            if should_update && data.len() >= 6 {
                                let raw = (data[5] >> 2) & 0x03;
                                status.ignition_sts = Some(raw);
                                status.last_ignition_time = Some(std::time::Instant::now());
                            }
                        }
                        _ => {
                            if let Some(message_def) = lookup_message(interface, id) {
                                let decoded = decode_message(message_def, id, &data);
                                let mut decoded_state = state.decoded.write().await;
                                let iface_map = decoded_state
                                    .entry(interface.to_string())
                                    .or_insert_with(HashMap::new);
                                iface_map.insert(id, decoded);
                            }
                        }
                    }

                    if id == VIN_FRAME_ID || id == BATTERY_FRAME_ID || id == IGNITION_FRAME_ID {
                        if let Some(message_def) = lookup_message(interface, id) {
                            let decoded = decode_message(message_def, id, &data);
                            let mut decoded_state = state.decoded.write().await;
                            let iface_map = decoded_state
                                .entry(interface.to_string())
                                .or_insert_with(HashMap::new);
                            iface_map.insert(id, decoded);
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!("Error reading CAN frame: {}", e);
                    break;
                }
                Err(_) => {
                    // Timeout occurred, loop will check timeout condition above
                    continue;
                }
            }
        }
    }
}

fn decode_message(message: &MessageDef, frame_id: u32, data: &[u8]) -> DecodedMessageSnapshot {
    let mut signals = BTreeMap::new();

    for signal in message.signals {
        if let Some(raw) = extract_signal_raw(data, signal.start_bit, signal.bit_len, signal.little_endian) {
            let signed_raw = if signal.is_signed {
                sign_extend(raw, signal.bit_len)
            } else {
                raw as i64
            };

            let value = (signed_raw as f64) * signal.factor + signal.offset;
            signals.insert(
                signal.name.to_string(),
                DecodedSignalValue {
                    raw: signed_raw,
                    value,
                    unit: if signal.unit.is_empty() {
                        None
                    } else {
                        Some(signal.unit.to_string())
                    },
                },
            );
        }
    }

    DecodedMessageSnapshot {
        frame_id,
        message_name: message.name.to_string(),
        dlc: message.dlc,
        transmitter: message.transmitter.to_string(),
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        signals,
    }
}

fn extract_signal_raw(data: &[u8], start_bit: u16, bit_len: u8, little_endian: bool) -> Option<u64> {
    if bit_len == 0 || bit_len > 64 {
        return None;
    }

    let total_bits = data.len() * 8;

    if little_endian {
        let mut out = 0u64;
        for i in 0..(bit_len as usize) {
            let bit_index = (start_bit as usize).checked_add(i)?;
            if bit_index >= total_bits {
                return None;
            }
            let byte_idx = bit_index / 8;
            let bit_in_byte = bit_index % 8;
            let bit = (data[byte_idx] >> bit_in_byte) & 0x1;
            out |= (bit as u64) << i;
        }
        return Some(out);
    }

    let mut current = start_bit as i32;
    let mut out = 0u64;
    for _ in 0..bit_len {
        if current < 0 {
            return None;
        }

        let bit_index = current as usize;
        if bit_index >= total_bits {
            return None;
        }

        let byte_idx = bit_index / 8;
        let bit_in_byte = bit_index % 8;
        let bit = (data[byte_idx] >> bit_in_byte) & 0x1;

        out = (out << 1) | bit as u64;

        current = if current % 8 == 0 {
            current + 15
        } else {
            current - 1
        };
    }

    Some(out)
}

fn sign_extend(raw: u64, bits: u8) -> i64 {
    if bits == 64 {
        return raw as i64;
    }

    let shift = 64 - bits;
    ((raw << shift) as i64) >> shift
}
