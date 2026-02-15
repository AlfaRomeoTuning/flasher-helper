use std::sync::{Arc, RwLock};
use std::time::Duration;
use socketcan::{CanFrame, CanSocket, EmbeddedAddrStruct, Socket, Frame};
use tokio::time::sleep;
use tracing::{info, warn, error};
use anyhow::Result;

pub struct SharedState {
    pub vin: RwLock<Option<String>>,
}

pub async fn run_can_listener(interface: &str, state: Arc<SharedState>) -> Result<()> {
    loop {
        info!("Opening SocketCAN interface: {}", interface);
        let socket = match CanSocket::open(interface) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to open socket {}: {}. Retrying in 5s...", interface, e);
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        // We use the tokio-compatible socket provided by the socketcan crate's tokio feature.
        // However, the standard CanSocket from socketcan 3.3 can be made non-blocking and used with Tokio.
        socket.set_nonblocking(true)?;
        let mut tokio_socket = socketcan::tokio::CanSocket::from_socket(socket)?;

        info!("Listening for VIN frames on ID 0x416...");
        
        let mut vin_buffer = vec![0u8; 17];
        let mut received_indices = [false; 3];

        loop {
            match tokio_socket.read_frame().await {
                Ok(frame) => {
                    if frame.id().as_raw() != 0x416 {
                        continue;
                    }

                    let data = frame.data();
                    if data.is_empty() {
                        continue;
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
                            let len = vin_part.len().min(3); // VIN is 17 chars: 7 + 7 + 3
                            vin_buffer[14..14+len].copy_from_slice(&vin_part[..len]);
                            received_indices[2] = true;
                        }
                        _ => {
                            warn!("Received unexpected sequence index: {:02X}", seq_index);
                            continue;
                        }
                    }

                    if received_indices.iter().all(|&r| r) {
                        // Reconstructed VIN
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
                            
                            // Reset for next reconstruction
                            received_indices = [false; 3];
                        } else {
                            warn!("Reconstructed VIN has invalid length: {} (expected 17)", vin.len());
                            // Optional: Reset indices if we get partial/corrupt data
                        }
                    }
                }
                Err(e) => {
                    error!("Error reading CAN frame: {}", e);
                    break; // Re-open socket
                }
            }
        }
    }
}
