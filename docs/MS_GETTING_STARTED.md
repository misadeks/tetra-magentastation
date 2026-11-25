# Getting Started (MS mode)

A step-by-step guide to build and run **TETRA MagentaStation**, the MS-only
portable-radio stack. Follow
it top to bottom and you'll have a software TETRA radio that scans, camps on a
base station, and registers. No prior Rust experience needed.

> Looking for architecture, supported capabilities, or the config
> reference? See [`MS_MODE.md`](MS_MODE.md) and [`MS_CONFIG.md`](MS_CONFIG.md).

---

## 0. What this is (30 seconds)

`magentastation` is the **radio stack**. In MS mode it drives an SDR as a
portable TETRA subscriber unit: it scans programmed frequencies, camps on a base
station's downlink, derives its uplink over the air, and registers (ITSI attach).

It has **no screen of its own**. The operator screen — call control, contacts,
codeplug, voice — is a separate app, the **MMI**:

- **[tetra-bluestation-mmi](https://github.com/misadeks/tetra-bluestation-mmi)** — the operator UI.

The radio stack talks to the MMI over two local WebSocket connections. Important:
**the stack is the client — it connects *to* the MMI.** So you normally start the
MMI first (or just let the stack retry; it reconnects within ~1 s). You don't
need the MMI to bring the radio up on air — it will scan/camp/register on its own
— but you need it to make calls and see state.

---

## 1. Install the tools

RF operation normally runs on **Linux** (a Raspberry Pi is
the typical target; a Linux PC or WSL works for building). You need **Git**,
**Rust** (a recent stable — the stack uses Rust edition 2024, so 1.85+), and
**SoapySDR** plus the driver module for your radio.

### Linux (Ubuntu/Debian, incl. Raspberry Pi OS)

```bash
# Git + Rust + build essentials
sudo apt update && sudo apt install -y git curl build-essential pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # accept defaults
source "$HOME/.cargo/env"

# SoapySDR runtime + headers (the build links against these)
sudo apt install -y libsoapysdr-dev soapysdr-tools
```

You also need the **SoapySDR driver module for your radio**. This guide assumes
the **SXceiver (SX1255)**, exposed by SoapySX as `driver=sx` — install/build it
per its own instructions. Verify Soapy can see your device:

```bash
SoapySDRUtil --find      # your device should be listed (e.g. driver=sx)
```

> **Windows note:** native builds need a configured SoapySDR SDK (`SOAPY_SDR_ROOT`),
> a compatible C/C++ toolchain and the SDK DLL directory on `PATH`. The portable
> libraries do not depend on SoapySDR. A build linked to a test stub proves
> compilation/offline behavior only, never live radio support.

---

## 2. Get the code

```bash
git clone https://github.com/misadeks/tetra-magentastation.git
cd tetra-magentastation
```

There are **no git submodules** to fetch, and **no speech-codec tables to
generate** — the ACELP voice codec lives in the MMI UI, not here.

---

## 3. Build

```bash
cargo build -p magentastation
```

The first build downloads and compiles dependencies (several minutes — normal).
For a faster, optimized binary:

```bash
cargo build --release -p magentastation   # lands in target/release/magentastation
```

Sanity-check your checkout with the tests (optional):

```bash
cargo test -p tetra-config      # config validation / round-trip
cargo test -p tetra-entities    # MS stack unit + integration tests
```

---

## 4. Start offline, then configure RF if authorized

For interface integration without hardware:

```bash
python examples/ms-interface/reference-client.py
# In a second terminal:
cargo run -p magentastation -- example_config/config-offline.toml
```

The reference server needs `websockets>=14` (`python -m pip install "websockets>=14"`).
This backend opens no radio, generates no protocol ticks, and only services
offline management. Stop it before changing to an RF configuration.

The following RF configuration can transmit registration/uplink signaling.
Use it only with explicit authorization and a suitable RF environment.

Copy the commented example and edit it — it's your radio's "codeplug":

```bash
cp example_config/config-ms.toml config.toml
```

The bare minimum to get on air on your network:

| Setting | What it does |
|---|---|
| `[phy_io.soapysdr].device` | Your SDR selector, e.g. `driver=sx`. Check with `SoapySDRUtil --find`. |
| `[[frequency_list]].frequencies` | The **downlink** carrier(s), in Hz, the radio scans to find a cell. |
| `[net_info]` `mcc` / `mnc` | Your **home network** identity (must match the cell you want to camp on). |
| `[ms].issi` | This radio's own 24-bit address (`1..=16777215`). **Change it** from the default. |
| `[ms].attach_groups` | Talkgroups (GSSIs) to attach to once registered. Empty does not disable registration or other uplink signaling. |

> ⚠️ **Uplink TX gain — the defaults are low, so you may not register.** The
> example ships `tx_gain_dac = 0.0` and `tx_gain_mixer = 0.0` — a deliberately
> low starting point. If the base station can't hear your random-access bursts,
> registration fails (you'll see `random access abandoned: MaxTransmissions` in
> the log). Raise the TX gain (chiefly `tx_gain_mixer`) to suit your RF chain
> before expecting uplink / registration — see the comments in `config-ms.toml`.

Full field-by-field reference: **[`MS_CONFIG.md`](MS_CONFIG.md)**.

Run it:

```bash
./target/debug/magentastation config.toml      # or target/release/... for a release build
```

A healthy start logs, in order: `downlink synchronized` → `selected serving
cell` → (with TX gain set) a registration attempt. For a quiet journal the stack
defaults to `RUST_LOG=info`; bump to `RUST_LOG=debug` (or `trace`) for the full
per-PDU detail when troubleshooting.

---

## 5. Connect the operator UI (MMI)

To place/receive calls, see live state, program contacts, etc., run the
**[tetra-bluestation-mmi](https://github.com/misadeks/tetra-bluestation-mmi)**
app (build/run it from its own
[Getting Started](https://github.com/misadeks/tetra-bluestation-mmi/blob/master/docs/GETTING_STARTED.md)).

The MMI is the **server**; the radio stack connects to it. Tell the stack where
the MMI is by adding `[command]` and `[telemetry]` sections to your `config.toml`
(the MMI listens on control `9102` and telemetry `9101` by default):

```toml
[command]
host = "127.0.0.1"   # the host running the MMI (127.0.0.1 if same machine)
port = 9102

[telemetry]
host = "127.0.0.1"
port = 9101
```

The stack attempts to connect **immediately** on start and retries every ~1 s, so
you can start the two apps in any order — it links up within about a second of
the MMI being available. (Auth/TLS fields exist too — see
[`MS_CONFIG.md`](MS_CONFIG.md).)

---

## 6. Deploy to a Raspberry Pi

For an always-on radio, run the stack under systemd. A ready-made unit and a
supervisor wrapper are in `example_config/`:

- `example_config/magentastation.service` — systemd unit (real-time scheduling,
  config-apply restart handling, `RUST_LOG=info`).
- `example_config/magentastation-supervisor.sh` — a plain-shell alternative.

```bash
sudo cp example_config/magentastation.service /etc/systemd/system/
# edit ExecStart (binary + config paths) and User/Group for your setup, then:
sudo systemctl daemon-reload
sudo systemctl enable --now magentastation
journalctl -u magentastation -f
```

> **Running the MMI on the same Pi?** The receive-timed radio and a GUI/display
> can fight for CPU and memory/DMA bandwidth, which starves the SDR and makes the
> radio drop in and out of service. See the **"Real-time scheduling"** section of
> [`MS_CONFIG.md`](MS_CONFIG.md) for the fix (RT priority, CPU pinning, keeping the
> UI off the radio's cores, and not routing UI audio through the SDR's I2S codec).

---

## Troubleshooting

**`SoapySDRUtil --find` shows nothing / "No devices found"**
Your SDR driver module isn't installed or the device isn't connected. The stack
can't open the radio without it. Install the driver for your hardware (e.g.
SoapySX for `driver=sx`).

**Build fails in `soapysdr-sys` / `pkg-config` can't find SoapySDR**
Install `libsoapysdr-dev` (step 1), or configure the native Windows SoapySDR SDK.

**It syncs and camps but never registers; log shows `random access abandoned: MaxTransmissions`**
The base station isn't hearing your uplink. Most often the **TX gain is too low** —
raise `tx_gain_mixer` (step 4). Also check antenna/duplex and that the cell
actually allows your `subscriber_class`.

**Nothing happens / the UI shows no radio**
The MMI must be running and reachable at the `[command]`/`[telemetry]` host+port
you configured (step 5). The stack logs `Control transport connection failed …`
once, then retries quietly every ~1 s until the MMI is up.

**The radio flaps in/out of service (repeated `MLE-BREAK`) — especially with the UI on the same Pi**
CPU/DMA starvation, not RF. See "Real-time scheduling" in
[`MS_CONFIG.md`](MS_CONFIG.md). Confirm it's not power/thermal with
`vcgencmd get_throttled` (`0x0` = OK).

**journald: "Suppressed N messages from magentastation.service"**
That's from an older build that logged too much. Update — the stack now defaults
to a quiet `info` level (bump with `RUST_LOG=debug` on demand).
