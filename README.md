# Flasher Helper

A Linux Rust service that monitors a SocketCAN interface for VIN data and provides an HTTP API for VIN retrieval and system shutdown.

## Features
- **CAN Monitoring**: Listens on `can-c1` for VIN frames (ID 0x416).
- **VIN Reconstruction**: Combines multi-frame ISO-TP-like sequence into a 17-char VIN.
- **DBC-Based Decoding**: Uses 3 static DBC files (BH/C1/C2), converted at build time into Rust decoder tables.
- **Multi-Interface Listening**: Listens concurrently on `can-bh`, `can-c1`, and `can-c2`.
- **HTTP API**: 
  - `GET /vin`: Returns the latest reconstructed VIN.
    - `GET /status`: Returns latest battery and ignition status with timeout-aware freshness.
  - `GET /can/{interface}/{bu}`: Returns latest decoded messages for a BU on a given interface.
- `GET /can/{interface}/metadata`: Returns available bus units and message names for the requested interface.
- `POST /shutdown`: Safely shuts down the host machine.
- **Async Execution**: Built with Tokio and Axum.

## DBC Files

The project loads the following DBC sources at build time:

- `dbc/P952_E3A_R1_BHCAN.dbc` -> `can-bh`
- `dbc/P952_E3A_R1_C1CAN.dbc` -> `can-c1`
- `dbc/P952_E3A_R1_C2CAN.dbc` -> `can-c2`

Generated decoder metadata is compiled into the binary via `build.rs` for low runtime overhead.

## API: Decoded CAN Data

### `GET /can/{interface}/{bu}`

Returns latest decoded message snapshots (latest per frame id) for the requested interface and bus unit.

Path params:

- `interface`: one of `can-bh`, `can-c1`, `can-c2`
- `bu`: bus unit name exactly as defined in that interface's DBC `BU_:` list

Query params:

- `messages`: optional comma-separated message-name filter

Note: the endpoint is TX-only and returns only messages where the requested `bu` is the source/transmitter.

Example:

```bash
curl "http://127.0.0.1:3000/can/can-c1/BCM?messages=BATTERY_INFO,BCM_COMMAND"
```

Example response:

```json
{
  "interface": "can-c1",
  "bu": "BCM",
  "messages": [
    {
      "message_name": "BATTERY_INFO",
      "dlc": 7,
      "timestamp_ms": 1712233445566,
      "signals": {
        "BatteryVoltageLevelHP": {
          "raw": 342,
          "value": 12.55,
          "unit": "Volt"
        }
      }
    }
  ]
}
```

### `GET /can/{interface}/metadata`

Returns the available bus units and message names defined for the requested interface.

Path params:

- `interface`: one of `can-bh`, `can-c1`, `can-c2`

Example:

```bash
curl "http://127.0.0.1:3000/can/can-c1/metadata"
```

Example response:

```json
{
  "interface": "can-c1",
  "bus_units": ["BCM", "ECM", "ABS"],
  "messages_by_bu": {
    "BCM": ["BATTERY_INFO", "BCM_COMMAND"],
    "ECM": ["ENGINE_STATUS", "RPM_INFO"],
    "ABS": ["BRAKE_STATUS"]
  }
}
```

## Runtime Configuration

The service supports runtime configuration using environment variables:

- `FLASHER_BIND_ADDR` (default: `127.0.0.1`)
- `FLASHER_PORT` (default: `3000`)

CAN interfaces are currently fixed to `can-bh`, `can-c1`, and `can-c2` and are all started concurrently.

You can still provide the port as the first CLI argument, which takes precedence over `FLASHER_PORT`.

Example:

```bash
FLASHER_BIND_ADDR=0.0.0.0 FLASHER_PORT=8080 ./flasher-helper
```

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
