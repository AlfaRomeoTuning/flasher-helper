use std::sync::{Arc, RwLock};
use std::time::Duration;
use socketcan::{Socket, Frame};
use socketcan::EmbeddedFrame;
use tokio::time::sleep;
use tracing::{info, warn, error};
use anyhow::Result;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StatusData {
    pub battery_voltage: Option<f32>,
    pub ignition_sts: Option<u8>,
    #[serde(skip)]
    pub last_battery_time: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_ignition_time: Option<std::time::Instant>,
}

pub struct SharedState {
    pub vin: RwLock<Option<String>>,
    pub status: RwLock<StatusData>,
}

pub async fn run_can_listener(interface: &str, state: Arc<SharedState>) -> Result<()> {
    loop {
        info!("Opening SocketCAN interface: {}", interface);
        let mut tokio_socket = match socketcan::tokio::CanSocket::open(interface) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to open socket {}: {}. Retrying in 5s...", interface, e);
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        info!("Listening for VIN frames on ID 0x416...");
        
        let mut vin_buffer = vec![0u8; 17];
        let mut received_indices = [false; 3];
        let mut last_status_update = std::time::Instant::now() - Duration::from_secs(1);

        loop {
            match tokio_socket.read_frame().await {
                Ok(frame) => {
                    let (id, data) = match frame {
                        socketcan::CanFrame::Data(f) => (f.raw_id(), f.data().to_vec()),
                        _ => continue,
                    };

                    match id {
                        0x416 => {
                            let seq_index = data[0];
                            let vin_part = &data[1..];

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
                                    {
                                        let mut w = state.vin.write().unwrap();
                                        *w = Some(vin);
                                    }

                                    info!("Pausing CAN listening for 60 seconds...");
                                    sleep(Duration::from_secs(60)).await;
                                    received_indices = [false; 3];
                                } else {
                                    warn!("Reconstructed VIN has invalid length: {} (expected 17)", vin.len());
                                }
                            }
                        }
                        0x41A | 0xFA => {
                            if last_status_update.elapsed() < Duration::from_secs(1) {
                                continue;
                            }

                            let mut status = state.status.write().unwrap();
                            if id == 0x41A {
                                // BatteryVoltageLevelHP : 7|9@0+ (0.025,4)
                                let raw = ((data[0] as u16) << 1) | ((data[1] >> 7) as u16);
                                status.battery_voltage = Some((raw as f32) * 0.025 + 4.0);
                                status.last_battery_time = Some(std::time::Instant::now());
                            } else if id == 0xFA {
                                // KeyInIgnSts : 43|2@0+
                                let raw = (data[5] >> 2) & 0x03;
                                status.ignition_sts = Some(raw);
                                status.last_ignition_time = Some(std::time::Instant::now());
                            }
                            last_status_update = std::time::Instant::now();
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    error!("Error reading CAN frame: {}", e);
                    break;
                }
            }
        }
    }
}
