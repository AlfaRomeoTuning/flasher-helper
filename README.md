# Flasher Helper

A Linux Rust service that monitors a SocketCAN interface for VIN data and provides an HTTP API for VIN retrieval and system shutdown.

## Features
- **CAN Monitoring**: Listens on `can-c1` for VIN frames (ID 0x416).
- **VIN Reconstruction**: Combines multi-frame ISO-TP-like sequence into a 17-char VIN.
- **HTTP API**: 
  - `GET /vin`: Returns the latest reconstructed VIN.
  - `POST /shutdown`: Safely shuts down the host machine.
- **Async Execution**: Built with Tokio and Axum.

## Compilation Guide

Since this application depends on `socketcan`, it can only be compiled natively on a Linux system.

### Native Compilation (Linux)

If you are on a Linux machine with Rust installed:

1.  **Install Build Tools**:
    ```bash
    sudo apt-get update && sudo apt-get install -y build-essential
    ```

2.  **Build**:
    ```bash
    cargo build --release
    ```
    The binary will be located at `target/release/flasher-helper`.

### Cross-Compilation (from Mac)

If you need to build the Linux binary from your Mac:

1.  **Install Cross**:
    ```bash
    cargo install cross --git https://github.com/cross-rs/cross.git
    ```

2.  **Build for Linux (x86_64)**:
    Ensure Docker is running, then run:
    ```bash
    cross build --target x86_64-unknown-linux-gnu --release
    ```
    The binary will be at `target/x86_64-unknown-linux-gnu/release/flasher-helper`.

## Deployment

Transfer the compiled binary to your target Linux machine and run it:

```bash
./flasher-helper
```

> [!IMPORTANT]
> Ensure the `can-c1` interface is up and configured on the target machine:
> `ip link set can-c1 up type can bitrate 500000`
