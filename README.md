# Flasher Helper

A Linux Rust service that listens on multiple SocketCAN interfaces, decodes DBC-defined frames in real time, and exposes VIN/status/CAN data through an HTTP API.
Designed for working with Alfa Romeo Giulia and Stelvio vehicle platforms.

## Features
- **Concurrent multi-bus listening**: Continuously listens on `can-bh`, `can-c1`, and `can-c2` with automatic retry when a CAN socket cannot be opened.
- **VIN reconstruction with timeout/cooldown**: Reconstructs a 17-character VIN from `can-c1` frame `0x416`, resets incomplete VIN sessions after timeout, and applies a cooldown after a successful VIN capture.
- **Live vehicle status extraction**: Extracts battery voltage (`0x41A`) and ignition state (`0xFA`) on `can-c1`, with freshness handling via the `/status` endpoint.
- **DBC-driven decoding**: Parses 3 DBC files at build time and generates static Rust decoder tables used for runtime frame decoding.
- **Latest-frame snapshots per interface**: Keeps the latest decoded snapshot per frame ID for each interface, including raw value, scaled value, and unit metadata.
- **HTTP API (Axum + Tokio)**:
  - `GET /vin` — returns the latest reconstructed VIN.
  - `GET /status` — returns battery/ignition status with timeout-aware freshness.
  - `GET /can/{interface}/{bu}` — returns decoded TX messages for the requested bus unit, with optional `messages=` filtering.
  - `GET /can/{interface}/metadata` — returns available bus units and per-BU message names.
  - `POST /shutdown` — initiates host shutdown only when `FLASHER_SHUTDOWN_TOKEN` is configured and provided via `x-shutdown-token`.

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

## Credits

- Project by AlfaRomeoTuning — https://alfatuning.app/
