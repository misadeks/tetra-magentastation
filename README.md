# TETRA MagentaStation

<img src="contrib/logo/magentastation.svg" alt="MagentaStation emblem" width="160" />

<sub>* MagentaStation is not affiliated with BlueStation or Midnight Blue.</sub>

MagentaStation is a TETRA mobile-station (MS) stack. It uses an SDR to receive a
cell's downlink, register, attach to talkgroups, and handle calls and short data
messages. The separate [Slint MMI](https://github.com/misadeks/tetra-bluestation-mmi)
provides the operator interface and microphone/speaker audio, using
[libtetra-acelp](https://github.com/misadeks/libtetra-acelp), a Rust implementation
of the ETSI TETRA full-rate speech codec (EN 300 395-2). The radio transports
speech bits; vocoding runs in the MMI.

MagentaStation is derived from
[BlueStation](https://github.com/MidnightBlueLabs/tetra-bluestation), the
base-station (BS) implementation and counterpart, by Wouter Bokslag / Midnight
Blue and contributors.

## Build and run

Use current stable Rust and the SoapySDR development libraries. On Debian/Ubuntu:

```sh
sudo apt update
sudo apt install git build-essential pkg-config libsoapysdr-dev soapysdr-tools clang libclang-dev
git clone https://github.com/misadeks/tetra-magentastation.git
cd tetra-magentastation
cargo build --locked --release -p magentastation
```

For offline management without an SDR:

```sh
./target/release/magentastation example_config/config-offline.toml
```

Start the MMI first, or use the [read-only reference server](examples/ms-interface/README.md).
The radio connects to the application's control server on port **9102** and
telemetry server on **9101**. Configure these in the radio's `[command]` and
`[telemetry]` sections.

For RF operation, install the SoapySDR driver for your device, copy
[`config-ms.toml`](example_config/config-ms.toml) to `config.toml`, and set the
SDR, network identity, ISSI, downlink frequencies and gains for your setup:

```sh
cp example_config/config-ms.toml config.toml
./target/release/magentastation config.toml
```

The [setup guide](docs/MS_GETTING_STARTED.md) covers the radio and MMI builds,
codec tables, audio/PTT, configuration, services and troubleshooting.

## Capabilities

The stack implements cell acquisition, random/reserved access, registration
and detach, group affiliation, group/individual/duplex call control, floor
control, TCH/S speech transport, DTMF, SDS/status and SDS-TL reports.

It is a partial TETRA implementation. Authentication, OTAR, air-interface
encryption, DMO, SNDCP packet data, full mobility/reselection and full
supplementary services are not implemented. There is no BS or monitor runtime.
The offline backend handles management only, not calls or simulated RF.
See [architecture and limitations](docs/MS_MODE.md) for details.

## Libraries and documentation

The radio uses allocator-free `no_std` protocol and type libraries, pure
channel-coding algorithms, and `std`-based application support and platform
adapters for SDR, network and filesystem I/O. These also run in headless and
embedded-Linux applications; they do not require a graphical desktop.
An optional C ABI exposes seven independently owned protocol components.

- [Setup: radio, MMI and audio](docs/MS_GETTING_STARTED.md)
- [Radio configuration reference](docs/MS_CONFIG.md)
- [MS architecture and limitations](docs/MS_MODE.md)
- [Control, telemetry, provisioning and speech interface](examples/ms-interface/README.md)
- [Portable Rust/C integration and memory budgets](docs/PORTABLE_MS.md)
- [Portable MLE component](docs/PORTABLE_MLE.md)

## License

Licensed under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for
provenance and attribution details.

## Acknowledgments

Thanks to [tetra-bluestation](https://github.com/MidnightBlueLabs/tetra-bluestation)
and [all its contributors](https://github.com/MidnightBlueLabs/tetra-bluestation/graphs/contributors)
for the foundation of MagentaStation.

Thanks also to Harald Welte and the Osmocom community for their foundational
TETRA work, Tatu Peltola for SoapySDR timestamp support and Viterbi work, and
Stichting NLnet / RETETRA3 for supporting upstream development.
