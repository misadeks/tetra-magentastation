# Setting up MagentaStation

The radio and operator interface are separate programs:

| Program | Role | Configuration |
| --- | --- | --- |
| `magentastation` | SDR, TETRA protocol, codeplug and radio state | TOML path passed on the command line |
| `tetra-bluestation-mmi` | Slint UI, calls/PTT, microphone/speaker and ACELP | `config.toml` in the MMI's working directory |

The radio is a WebSocket **client**. The MMI listens for it on control port
**9102** and telemetry port **9101**. Start the MMI first; the radio also retries
if it starts before the MMI. [BlueStation](https://github.com/MidnightBlueLabs/tetra-bluestation)
is the base-station counterpart, not the radio executable used here.

Commands below use Linux shell syntax. The
[MMI setup guide](https://github.com/misadeks/tetra-bluestation-mmi/blob/master/docs/GETTING_STARTED.md)
covers its Windows build, and the [Pi guide](https://github.com/misadeks/tetra-bluestation-mmi/blob/master/PI_SETUP.md)
covers its display, DRM/KMS, audio and service setup.

## Install build tools and the SDR driver

Install current stable Rust from [rustup.rs](https://rustup.rs), then check:

```sh
rustc --version
cargo --version
```

Use **Rust 1.95 or newer** when building the MMI. The radio uses Rust edition
2024; its locked dependencies are built with current stable Rust in CI.

On Debian/Ubuntu or Raspberry Pi OS:

```sh
sudo apt update
sudo apt install git build-essential pkg-config libsoapysdr-dev soapysdr-tools clang libclang-dev
```

SoapySDR itself is not a device driver. Install the matching Soapy module for
your SDR using that driver's instructions. The supplied RF example is for an
SXceiver through SoapySX (`driver=sx`); change the selector, sample rate, antenna
and gain names if you use another device. Check the installed driver and device
when preparing RF operation:

```sh
SoapySDRUtil --info
SoapySDRUtil --find
SoapySDRUtil --probe="driver=sx"
```

These are user-run device checks, not requirements for the offline backend.
On Windows, native radio builds need the SoapySDR SDK (`SOAPY_SDR_ROOT`), a
matching C/C++ toolchain, and the SDK's DLL directory on `PATH`. WSL builds use
Linux dependencies; access to an SDR from WSL depends on the device/driver.

## Build the radio

```sh
git clone https://github.com/misadeks/tetra-magentastation.git
cd tetra-magentastation
cargo build --locked --release -p magentastation
./target/release/magentastation --help
```

The binary is `target/release/magentastation` (`.exe` on Windows). This repository
has no submodules or vocoder tables. The portable libraries can also be used
without SoapySDR; see [portable integration](PORTABLE_MS.md).

## Build the Slint MMI and vocoder

Install the MMI's Linux UI/audio dependencies:

```sh
sudo apt install \
  libasound2-dev libfontconfig1-dev libxkbcommon-dev \
  libdrm-dev libgbm-dev libinput-dev libudev-dev \
  libegl-dev libgles-dev libgl1-mesa-dri \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libwayland-dev
```

Clone it into a separate directory, for example beside `tetra-magentastation`:

```sh
cd ..
git clone --recurse-submodules https://github.com/misadeks/tetra-bluestation-mmi.git
cd tetra-bluestation-mmi
```

If already cloned without submodules, run `git submodule update --init --recursive`.
The MMI depends on the Rust [libtetra-acelp codec](https://github.com/misadeks/libtetra-acelp)
through `third_party/libtetra-acelp`. Generate its tables locally,
then build:

```sh
cd third_party/libtetra-acelp
cargo run -p populate
cd ../..
cargo build --release
```

The populate tool retrieves the ETSI reference tables; they are not shipped in
this repository. For a local archive or updated prerequisites, follow the
[MMI codec setup](https://github.com/misadeks/tetra-bluestation-mmi/blob/master/docs/GETTING_STARTED.md#3-generate-the-speech-codec-tables-one-time).
The vocoder belongs in the MMI, not in the radio. Keep generated codec tables in
that checkout.

## Connect the programs

Keep the two `config.toml` files separate. The MMI reads its file from the
**current working directory**, not automatically from the binary's directory.
Run it from its checkout, or set a service's `WorkingDirectory` accordingly.

In the **MMI** configuration, these sections are listen addresses:

```toml
[command]
host = "127.0.0.1"
port = 9102
use_tls = false

[telemetry]
host = "127.0.0.1"
port = 9101
use_tls = false
```

Edit the existing sections rather than adding duplicates. The shipped MMI file
binds `0.0.0.0`; loopback is enough when both programs run on the same host.
For different hosts, bind the MMI to its network interface and use that host's
address in the radio config. Match TLS/credentials at both ends.

Start the MMI from its directory:

```sh
./target/release/tetra-bluestation-mmi
```

From the **radio** checkout, try offline management:

```sh
./target/release/magentastation example_config/config-offline.toml
```

It connects, reports its interface version and idle state, and serves
configuration reads/writes. It does not open an SDR or create protocol ticks;
registration, calls and voice will not progress. If you only need to inspect
the interface, use the [Python reference server](../examples/ms-interface/README.md#read-only-reference-server)
instead of the MMI, not on the same ports at the same time.

## Configure and start RF operation

Stop the offline radio and copy the RF example:

```sh
cp example_config/config-ms.toml config.toml
```

Edit these fields in the radio's file:

| Setting | What to set |
| --- | --- |
| `[phy_io.soapysdr]` | Device selector, sample rate, antenna names, PPM and RX/TX gains for your SDR |
| `[net_info]` | Home MCC/MNC |
| `[ms]` | Own ISSI, subscriber class and initial group affiliations |
| `[[frequency_list]]` | Downlink carrier frequencies in Hz |
| `[duplex_table]` | Overrides only if your network uses nonstandard duplex spacing |
| `[[talkgroup]]`, `[[scanlist]]` | Named groups and optional affiliation sets |

Add the same `[command]` and `[telemetry]` sections shown above to the **radio**
file; here `host` is the MMI server's address, not a listen address. The RF
example leaves these optional endpoints out. The control key is `[command]`,
not `[control]`.

Leave `stack_mode = "Ms"`. The radio scans candidates in programmed order,
checks network/subscriber-class suitability, camps, and derives the uplink
from the cell's SYSINFO. Registration may transmit even with
`attach_groups = []`; an empty affiliation list is not a receive-only switch.
`carrier_override.rx_only` is stored codeplug data, not an enforced TX inhibit.
See [configuration behavior](MS_CONFIG.md#carrier-overrides) before relying on
individual codeplug hints.

The SX example starts with low TX gains. Set gains for your own RF chain;
one driver's gain values are not portable to another. Then run:

```sh
./target/release/magentastation config.toml
```

Check synchronization, serving-cell and registration messages. The MMI should
show service and registration state. Use `RUST_LOG=debug` for more detail.

## Configure voice and PTT

In the **MMI** checkout, list its audio device names:

```sh
./target/release/tetra-bluestation-mmi --list-audio
```

Edit the MMI's existing `[audio]` section, then restart it:

```toml
[audio]
enabled = true
output_device = "default"
input_device = "default"
sample_rate = 8000
frame_ms = 60
jitter_ms = 0
```

Device selectors accept case-insensitive name substrings. `"default"` uses the
system default. The codec uses 8 kHz speech and two 137-bit subframes per
60 ms block. The example uses zero jitter buffering; increase `jitter_ms` if
network delivery is bursty, at the cost of latency. Missing microphone input
leaves receive audio available but prevents uplink speech.

On a Pi sharing an SXceiver, use a separate USB audio device for MMI sound.
The MMI's [Pi audio instructions](https://github.com/misadeks/tetra-bluestation-mmi/blob/master/PI_SETUP.md#audio-device-use-a-usb-codec-not-onboard-audio)
describe its ALSA configuration. Do not copy card names without checking your
device. `--test-ring` is an MMI speaker test; it is not an RF/voice-link test.

Select/attach a talkgroup in the MMI and wait for registration and service.
Hold its on-screen group PTT to request the floor/start a group call; speak
after the grant and release PTT to release the floor. Individual simplex calls
have their own call PTT; duplex calls use the call's transmit permission.
There is no physical GPIO/evdev PTT binding configured by these instructions.

The radio transports encoded speech bits over the existing control/telemetry
connections. It has no `[audio]` device settings and no separate voice port.
For application integration, see the [speech contract](../examples/ms-interface/README.md#speech-and-survey-streams).

## Provisioning and persistence

The MMI reads the active codeplug through `GetConfig`, edits it, and sends
`SetConfig`. The radio writes to the same TOML path it loaded at startup.
The service account needs write permission there. Comments/layout are replaced
by canonical TOML on write-back; keep a copy of hand-written settings if needed.

Operational codeplug edits apply live. Structural settings such as ISSI, SDR
or endpoint changes need `ApplyConfig` and a restart. The process exits with
code **75**; without a supervisor, run it again with the same config path.
`ApplyConfig` does nothing when no structural change is pending.
Endpoint passwords are redacted on reads and preserved when an unchanged
`"********"` sentinel is posted back.

The MMI's own display/audio preferences and message/call history are separate
from the radio codeplug. Its storage directory defaults to `data` under its
working directory (`[storage].dir` in the MMI config).

## Run as a service

The sample [systemd unit](../example_config/magentastation.service) handles
exit-75 restarts. After configuring the radio:

```sh
sudo install -m 755 target/release/magentastation /usr/local/bin/magentastation
sudo install -d /etc/magentastation
sudo install -m 600 config.toml /etc/magentastation/config-ms.toml
sudo install -m 644 example_config/magentastation.service /etc/systemd/system/magentastation.service
```

Edit the unit's `User`, `Group`, device access, paths and CPU affinity for your
machine; give that account access to the config file for provisioning. The
sample's CPUs `2 3` and FIFO priority `73` are examples, not universal settings.
Then:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now magentastation
journalctl -u magentastation -f
```

For a plain-shell supervisor instead:

```sh
MAGENTASTATION_BIN="$PWD/target/release/magentastation" \
  sh example_config/magentastation-supervisor.sh "$PWD/config.toml"
```

It uses `chrt` if installed, so its default FIFO scheduling may require extra
privileges. Use systemd for managed deployments. The
[MMI Pi guide](https://github.com/misadeks/tetra-bluestation-mmi/blob/master/PI_SETUP.md)
owns the separate UI service/display setup.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `soapysdr-sys`, `pkg-config` or missing-library build errors | Install SoapySDR headers, Clang/libclang and the C/C++ toolchain. On Windows check SDK architecture, `SOAPY_SDR_ROOT` and DLL `PATH`. |
| No SDR found | Check the Soapy driver/module installation, selector and device permissions; compare `--find`/`--probe` output. |
| MMI codec build fails / empty submodule | Initialize submodules, then run `cargo run -p populate` in the MMI's `third_party/libtetra-acelp`. |
| UI disconnected | Start the MMI from the correct working directory; check `[command]` 9102, `[telemetry]` 9101, host, firewall, TLS and credentials. Do not run the Python server on the same ports. |
| UI connected, radio stays Idle | Check that the radio is not using `backend = "None"`. Offline mode does not register. |
| Camps but registration times out | Check network/subscriber class, uplink frequency/duplex spacing, TX gains, antenna and the peer's acceptance/reject messages. A timeout alone does not identify the cause. |
| No speech / no microphone | Check MMI audio enabled/devices/volume, codec tables, active call and floor grant. Speaker ringtone success does not prove speech reception. |
| Repeated lost samples or `MLE-BREAK` | Check RF conditions, CPU load, driver/sample rate, power and thermal throttling. On a shared Pi, separate UI/SDR CPU and audio resources; see [scheduling](MS_CONFIG.md#scheduling-and-logging). |
| `SetConfig` fails or changes vanish on restart | Check file permissions and that the process/service uses the file you edited. Keep MMI config and radio config distinct. |
| Process exits 75 after Apply | Start through the supervisor/service, or restart manually with the same config. |
| Too much logging | Use `RUST_LOG=info`; enable debug temporarily. A configured `debug_log` still writes its separate verbose file. |

See [capabilities and limitations](MS_MODE.md) for features not implemented by
the stack. RF interoperability, driver timing and embedded memory margins need
validation on the actual setup; the offline backend cannot establish those.
