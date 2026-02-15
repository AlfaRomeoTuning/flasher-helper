use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use socketcan::{Frame};
use socketcan::EmbeddedFrame;
use tokio::time::{sleep, timeout};
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
        let tokio_socket = match socketcan::tokio::CanSocket::open(interface) {
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
        let mut vin_search_start: Option<Instant> = None;

        loop {
            // Check if VIN search has timed out (60 seconds without completing)
            if let Some(start) = vin_search_start {
                if received_indices.iter().any(|&r| r) && start.elapsed() >= Duration::from_secs(60) {
                    warn!("VIN search timeout after 60 seconds without completing all 3 packets");
                    {
                        let mut w = state.vin.write().unwrap();
                        *w = None;
                    }
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
                        0x416 => {
                            // Track when VIN search started
                            if vin_search_start.is_none() && received_indices.iter().any(|&r| r) == false {
                                vin_search_start = Some(Instant::now());
                            }

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
                                } else {
                                    warn!("Reconstructed VIN has invalid length: {} (expected 17)", vin.len());
                                }
                                
                                received_indices = [false; 3];
                                vin_search_start = None;
                            }
                        }
                        0x41A => {
                            let mut status = state.status.write().unwrap();
                            let should_update = status.last_battery_time
                                .map(|t| t.elapsed() >= Duration::from_secs(1))
                                .unwrap_or(true);

                            if should_update && data.len() >= 2 {
                                let raw = ((data[0] as u16) << 1) | ((data[1] >> 7) as u16);
                                status.battery_voltage = Some((raw as f32) * 0.025 + 4.0);
                                status.last_battery_time = Some(std::time::Instant::now());
                            }
                        }
                        0xFA => {
                            let mut status = state.status.write().unwrap();
                            let should_update = status.last_ignition_time
                                .map(|t| t.elapsed() >= Duration::from_secs(1))
                                .unwrap_or(true);

                            if should_update && data.len() >= 6 {
                                let raw = (data[5] >> 2) & 0x03;
                                status.ignition_sts = Some(raw);
                                status.last_ignition_time = Some(std::time::Instant::now());
                            }
                        }
                        _ => {}
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
