# TETRA MagentaStation

An **MS-only TETRA mobile-station stack** with portable protocol/PHY libraries
and desktop radio, control, telemetry, provisioning and audio-transport adapters.
MagentaStation is derived from
[BlueStation](https://github.com/MidnightBlueLabs/tetra-bluestation), originally
developed by Wouter Bokslag / Midnight Blue, via the
[MS extraction in this fork](https://github.com/misadeks/tetra-bluestation).

This is experimental software for authorized research and interoperability work.
It is not a complete or certified TETRA implementation. There is no base-station
runtime, BS scheduler, Brew/TetraPack bridge or selectable BS/monitor role.
Downlink network PDUs remain: receiving infrastructure signaling is essential
MS functionality.

## What is included

| Component | Responsibility |
| --- | --- |
| `tetra-ms-types` | `no_std` addresses, TDMA time, checked fixed packed bits and carrier/duplex arithmetic |
| `tetra-ms-core` | Allocator-free LLC/FCS, MAC/access/fragmentation, MM registration/group attachment, MLE selection/survey, CMCE calls/floor control and SDS |
| `tetra-phy-core` | Pure channel coding, Viterbi, interleaving, scrambling, CRC/RM and traffic reorder with caller-owned workspaces |
| `tetra-ms-ffi` | Optional versioned C ABI: independently owned LLC, MAC/RA, reassembly, MLE, MM, CMCE and SDS components |
| `tetra-core`, `tetra-saps`, `tetra-pdus`, `tetra-config` | Desktop types, receive/transmit wire vocabulary, SAPs and TOML/codeplug persistence |
| `tetra-entities` | MS desktop adapters, downlink demodulation, discontinuous uplink modulation, SDR and network transports |
| `magentastation` | The MS-only desktop executable |

Supported procedures include cell acquisition, random/reserved access, registration
and detach, group affiliation, group/individual/duplex call control, floor control,
TCH/S speech transport, DTMF, SDS/status and SDS-TL reports. This does **not** add
authentication, OTAR, air-interface encryption, DMO, SNDCP packet data, complete
mobility/reselection, full supplementary services or an ACELP vocoder.
Audio coding and the operator UI remain external; no new UI is included.

## Build and run

Use a current stable Rust toolchain (edition 2024). Desktop builds need SoapySDR
development libraries and a C/C++ toolchain; Linux also needs `pkg-config` and
Clang/libclang. On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libsoapysdr-dev clang libclang-dev
cargo build --release -p magentastation
./target/release/magentastation --help
```

**Start without a radio:**

```sh
cargo run -p magentastation -- example_config/config-offline.toml
```

`backend = "None"` opens no SDR and services offline management only. It never
fabricates TDMA ticks, registration, call progress or transmitted radio frames.
Native SoapySDR linkage is still required. On Windows, configure the installed
SDK through `SOAPY_SDR_ROOT` and put its DLL directory on `PATH`.

For authorized RF use, copy and edit
[`example_config/config-ms.toml`](example_config/config-ms.toml), install your
SDR's SoapySDR driver, and pass that file to the same executable. Configure your
own identity, network, downlink scan list and RF chain before starting: unlike
the offline backend, RF operation may transmit registration and other uplink
signaling. `stack_mode` must be `"Ms"`; `"Bs"` and `"Mon"` are rejected before
device construction.

## Interfaces

The stack is a **WebSocket client** connecting to an external application's
servers: `[command]` for control and `[telemetry]` for events and downlink speech.
Frames carry JSON in binary WebSocket messages, with optional TLS and HTTP Basic
authentication. The application supplies uplink speech using `MsUplinkSpeech`;
the stack emits `MsSpeechFrame` with decoded bits and bad-frame metadata.

The existing wire contracts remain unchanged:
`bluestation-control-v1`, `bluestation-telemetry-v1` and application schema
`bluestation-ms-interface-6`. These are compatibility identifiers, not BS
implementations. Plane A exposes TNMM/TNCC/TNSDS primitives; the non-standard
Plane B provides state, configuration, codeplug and survey management.
Operational codeplug edits apply live; structural edits are persisted and
applied through a supervised restart (exit code 75).

- [Interface catalog and runnable reference server](examples/ms-interface/README.md)
- [Getting started](docs/MS_GETTING_STARTED.md)
- [Configuration reference](docs/MS_CONFIG.md)
- [MS architecture, capabilities and limitations](docs/MS_MODE.md)
- [Portable Rust/C integration, bounds and validation](docs/PORTABLE_MS.md)
- [Portable MLE](docs/PORTABLE_MLE.md)

## Library-only use

Portable default features require neither `std` nor `alloc`, sockets, filesystem,
audio, a logger, an OS scheduler or an SDR. Time, entropy, transport completion
and action routing belong to the caller.

```sh
cargo test -p tetra-ms-types -p tetra-ms-core --features std
cargo test -p tetra-phy-core
cargo test -p tetra-ms-ffi --features std,c-smoke
cargo build -p tetra-ms-core -p tetra-ms-ffi -p tetra-phy-core --no-default-features --target thumbv8m.main-none-eabihf
```

The desktop regression suite is `cargo test --workspace`. CI also checks
portable Clippy, formatting, embedded compilation, C/C++ layouts and freestanding
linking. Tests and cross-builds do not establish live RF interoperability or
board timing; hardware validation must be done separately with explicit approval.

## License and attribution

The original root [Apache-2.0 license](LICENSE) is retained unchanged.
[`NOTICE`](NOTICE) records derivation, modifications, upstream acknowledgments
and the inherited Cargo MIT/Apache metadata discrepancy. Cargo metadata here
follows the shipped root license; this is not a new MIT or dual-license grant.
Dependencies retain their own licenses.

Thanks to the original BlueStation contributors, Harald Welte and the Osmocom
community for the foundational TETRA work, Tatu Peltola for Rust SoapySDR
timestamp support and Viterbi work, and Stichting NLnet / RETETRA3 for supporting
the upstream implementation.
