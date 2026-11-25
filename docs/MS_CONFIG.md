# Radio configuration reference

Pass the radio's TOML file as its only positional argument:

```sh
magentastation config.toml
```

Start from [config-ms.toml](../example_config/config-ms.toml) for SDR operation
or [config-offline.toml](../example_config/config-offline.toml) for management
without a radio. The Slint MMI has a **different** `config.toml`; its audio,
display and storage settings do not belong in this file.

The parser lives in
[`tetra-config/src/magentastation`](../crates/tetra-config/src/magentastation/mod.rs).
Settings below describe the current implementation, including stored codeplug
hints that are not consumed by the radio.

## Global settings

| Key | Required | Meaning |
| --- | --- | --- |
| `config_version` | yes | `"0.7"` for current files. Legacy `"0.6"` is also accepted. |
| `stack_mode` | yes | `"Ms"`. Other roles are rejected. |
| `debug_log` | no | Path for a separate verbose trace log. Leave unset for normal operation. |

## PHY and SDR

### `[phy_io]`

`backend = "SoapySdr"` opens an SDR. `backend = "None"` opens no PHY and serves
offline management only: no recovered time, registration, calls or speech.
Omit `[phy_io.soapysdr]` for the offline backend. Native SoapySDR linkage is
still needed by the desktop executable.

### `[phy_io.soapysdr]`

Program downlink carriers in `[[frequency_list]]`. The first candidate seeds
RX; MLE retunes while scanning. The MS learns the serving cell's uplink and
duplex spacing from SYSINFO. Fixed initial frequencies are optional.

| Key | Default | Meaning |
| --- | --- | --- |
| `device` | first supported device | SoapySDR selector, e.g. `"driver=sx"` |
| `sample_rate` | device-specific | RX/TX sample rate in Hz; SXceiver defaults to `600000` |
| `ppm_err` | `0` | Frequency-error correction |
| `rx_channel`, `tx_channel` | device-specific | Channel indices |
| `rx_antenna`, `tx_antenna` | device-specific | Antenna port names |
| `rx_gain_<element>` | device-specific | RX gain in dB, with a gain element supported by the selected driver |
| `tx_gain_<element>` | device-specific | TX gain in dB |
| `tx_freq` | first scan candidate | Legacy field name for the **downlink/RX** carrier in Hz |
| `rx_freq` | unset | Legacy field name for the **uplink/TX** carrier in Hz; runtime SYSINFO retunes TX |

The example's `rx_gain_lna`, `rx_gain_pga`, `tx_gain_dac` and `tx_gain_mixer`
are SX-specific. Use `SoapySDRUtil --probe` to inspect your device's names and
ranges. The low example TX gains are starting values, not a radio-independent
power setting. File-replay/debug keys such as `dl_input_file` are not supported.

## Network and MS identity

### `[net_info]`

| Key | Meaning |
| --- | --- |
| `mcc` | Home Mobile Country Code, 10 bits |
| `mnc` | Home Mobile Network Code, 14 bits |

The home network is always eligible. `[[network]]` adds other eligible
MCC/MNC pairs.

### `[ms]`

| Key | Default | Meaning |
| --- | --- | --- |
| `issi` | required | Own 24-bit ISSI, `1..=16777215` |
| `subscriber_class` | `1` | Class `1..=16`, checked against the cell's advertised bitmap |
| `attach_groups` | `[]` | Initial GSSIs to attach after registration |

An empty affiliation list does not disable registration or other uplink
signaling. Scanlist activation can add groups to the desired affiliation set.

### `[duplex_table]`

Optional `overrides` maps a 3-bit duplex index to spacing in Hz. Unlisted entries
use the standard table for the band. Index 7 has no standard spacing:

```toml
[duplex_table]
overrides = [[7, 9400000]]
```

Only add an override that matches the network. The radio uses this table when
deriving the uplink from the serving cell's SYSINFO.

### Legacy `[cell_info]`

Normal MS files omit this section; the cell supplies identity and RF data.
`location_area` and `colour_code` can seed pre-registration/state fields.
Legacy `main_carrier`, `freq_band`, `freq_offset`, `duplex_spacing`,
`custom_duplex_spacing` and `reverse_operation` are accepted but do not
replace received SYSINFO. If the section is present, include `location_area`.
Canonical MS write-back omits it.

BS broadcast/service, subscriber-routing and timezone fields, and `[brew]`,
are not supported configuration sections here.

## Codeplug

The codeplug is the radio's persistent programming data: names, groups,
networks, frequencies, contacts and related hints. It is an application
structure, not a standardized TETRA provisioning protocol.

### Folders and talkgroups

| Table | Fields |
| --- | --- |
| `[[folder]]` | Unique `id`, display `name`, optional `order` |
| `[[talkgroup]]` | 24-bit `gssi`, `name`, optional folder `id` in `folder`, optional `class_of_usage` (`0..=7`) and `order` |

```toml
[[folder]]
id = "work"
name = "Work"
order = 1

[[talkgroup]]
gssi = 101
name = "Dispatch"
folder = "work"
class_of_usage = 0
order = 1
```

Display names/grouping are for the MMI. Group affiliation uses the identities,
not the labels.

### Additional networks

`[[network]]` contains `mcc`, `mnc`, optional `name` and `priority` (default `0`).
The MLE uses the home network plus this list as an eligibility set. `priority`
is stored for application ordering; current cell selection does not rank
networks by it.

### Frequency lists

`[[frequency_list]]` defines named downlink scan candidates:

| Key | Meaning |
| --- | --- |
| `name` | Unique label |
| `mode` | `"List"` or `"Range"` |
| `frequencies` | Frequencies in Hz for `"List"` |
| `dwell_ms` | Positive stored dwell hint; default `1000` |

```toml
[[frequency_list]]
name = "primary"
mode = "List"
frequencies = [439825000, 439850000]
dwell_ms = 800
```

All lists combine into a deduplicated candidate sequence, preserving programmed
order. Automatic acquisition camps on a suitable cell; it does not measure all
candidates and select the strongest. The desktop PHY advances empty-carrier
scanning through its fixed scan-dwell heartbeat, not a per-list `dwell_ms` timer.
Treat `dwell_ms` as a stored hint, not an exact timing control.

For `"Range"`, use one **single-bracket** subtable:

```toml
[[frequency_list]]
name = "band4"
mode = "Range"
dwell_ms = 800

[frequency_list.range]
band = 4
start_carrier = 1500
stop_carrier = 1600
step = 1
offsets = [0, 6250]
```

The carrier bounds are inclusive; `step` is at least 1 in 25 kHz carrier units.
Offsets may be `0`, `6250`, `-6250` or `12500` Hz; absent/empty uses `[0]`.
Portable MLE lists hold at most 256 candidate carriers.

Manual surveys use the same candidates and report results through
`MsScanResult` / `MsScanComplete`. A nonempty programmed candidate set limits
`CampOnCell` choices to that set.

### Carrier overrides

`[[carrier_override]]` stores per-carrier hints for applications. Current
desktop radio selection **does not consume these overrides**: `rx_only` is
not a TX inhibit, `colour_code` is not a camp lock, and the per-carrier duplex
hints do not override received SYSINFO. Use `[duplex_table]` for the implemented
spacing override.

| Key | Meaning |
| --- | --- |
| `name` | Unique label |
| `band`, `carrier`, `freq_offset` | Band/carrier form; offset in Hz |
| `dl_freq` | Absolute downlink frequency alternative |
| `colour_code` | Stored 6-bit colour hint |
| `duplex_index` | Stored 3-bit duplex hint |
| `custom_duplex_spacing` | Stored spacing hint in Hz |
| `rx_only` | Stored receive-only preference; **not enforced by the radio** |

If both absolute and band/carrier forms are supplied, they must agree:

```toml
[[carrier_override]]
name = "site-one"
dl_freq = 439825000
colour_code = 1
rx_only = true
```

### Scanlists

`[[scanlist]]` groups programmed talkgroups for affiliation, not carrier scanning.
Fields are `name`, `talkgroups` (GSSI array), `active` (default `false`) and
`order`. Every GSSI must refer to a `[[talkgroup]]`.

The desired affiliation set is `[ms].attach_groups` plus groups in all active
scanlists. The MMI can toggle a list live with `ActivateScanlist`; removing one
list does not detach a group still needed by another.

```toml
[[scanlist]]
name = "Dispatch"
talkgroups = [101]
active = true
order = 1
```

### Gateways and contacts

`[[gateway]]` defines `id`, `name`, a 24-bit `gateway_issi` and optional
access-code `prefix`. A phone call is an individual call to that ISSI with
external-number digits in U-SETUP; network-side routing belongs to the gateway.

`[[contact]]` has `name`, optional `callsign` and `order`, and exactly one
destination form: `issi` **or** `number` plus `gateway`. The prefix and number
together must fit 24 digits from `0-9 * # +`.

```toml
[[gateway]]
id = "office"
name = "Office PABX"
gateway_issi = 8000002
prefix = "9"

[[contact]]
name = "Dispatch lead"
issi = 2000123
order = 1

[[contact]]
name = "Front desk"
number = "1234"
gateway = "office"
order = 2
```

### Codeplug settings

`[codeplug.home_display]` stores `enabled` (default `false`) and SDS protocol
identifier `pid` (default `130`, range `0..=255`). Applications can use this
when composing home-display messages. The setting does not itself send an SDS.

## Control and telemetry

Both sections describe **servers the radio connects to**, usually the MMI.
They are optional; radio acquisition does not require the UI.

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

| Key | Meaning |
| --- | --- |
| `host`, `port` | Server address; required when the section is present |
| `use_tls` | `false` by default; `true` uses `wss://` |
| `ca_cert` | Optional DER certificate; requires TLS |
| `username`, `password` | Optional HTTP Basic credentials; supply both or neither |

The control section is `[command]`, not `[control]`. Control carries requests
and responses; telemetry carries events and downlink speech. Payloads are UTF-8
JSON in binary WebSocket frames. See the [interface catalog](../examples/ms-interface/README.md).

## Saving and applying changes

`GetConfig` returns the **active** config with endpoint passwords redacted as
`"********"`. `SetConfig` parses and normalizes a complete TOML document, writes
the startup config path, and classifies the change:

| Change | Effect |
| --- | --- |
| Codeplug only: folders, groups, networks, frequency lists, carrier hints, scanlists, gateways, contacts or display settings | Saved and applied live; no restart needed for that edit |
| Structural: SDR/backend, network identity, ISSI, duplex table, endpoints, legacy cell seeds or logging | Saved to disk, active configuration retained until restart; `restart_required = true` |

An unchanged redaction sentinel preserves the corresponding active password.
Removing a credential section removes that setting. Files store real passwords,
and write-back replaces comments/formatting, so set appropriate file permissions.
Write failures return a negative acknowledgement; the radio account must be
able to write the file it loaded.

`ApplyConfig` starts a best-effort detach drain and exits **75** only when a
structural change is pending. A supervisor must start the process again.
With nothing pending it acknowledges without restarting. Offline management
serves `GetInterfaceVersion`, `GetState`, `GetConfig`, `SetConfig` and `ApplyConfig`
before downlink synchronization; other commands wait for protocol ticks.

The MMI's display/audio configuration and local history are separate files;
see [setup](MS_GETTING_STARTED.md#provisioning-and-persistence).

## Scheduling and logging

The MS is receive-timed: missing SDR samples can break synchronization. On a
shared host, watch for `Lost ... samples`, repeated downlink-failure messages,
CPU load and power/thermal throttling. On a Pi, `vcgencmd get_throttled` can help
identify the latter. These symptoms can also have RF/driver causes.

The [sample systemd unit](../example_config/magentastation.service) uses FIFO
priority `73` and CPUs `2 3`; adjust these to the machine. Keep UI/audio work
off the radio's assigned CPUs where possible. Do not copy kernel isolation or
IRQ masks without measuring the relevant hardware. The separate
[MMI Pi guide](https://github.com/misadeks/tetra-bluestation-mmi/blob/master/PI_SETUP.md)
has display/audio deployment details.

Logging uses `tracing`; set `RUST_LOG=info` for normal runs or
`RUST_LOG=info,tetra_entities::mm=debug` for targeted diagnostics. `debug_log`
writes a separate verbose file even when console logging is quiet.
