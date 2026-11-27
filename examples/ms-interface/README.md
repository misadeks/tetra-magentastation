# MagentaStation MS external interface

The application schema is **`bluestation-ms-interface-6`**, discovered
through `Management::GetInterfaceVersion`. The transport subprotocols remain
`bluestation-control-v1` and `bluestation-telemetry-v1` for compatibility.
These strings identify the wire contract independently of the executable name.

## Connection direction and encoding

**The stack is a client. The external application listens on two WebSocket
servers.** Configure them with `[command]` and `[telemetry]` respectively.
The `magentastation` executable uses UTF-8 JSON carried in **binary WebSocket frames**,
not JSON text frames or a binary bitcode payload. The crates also retain
bitcode codecs for library users; the executable's workers use JSON.

Enum JSON is externally tagged: `{"Variant": {...}}`. Plane B requests/responses
have a `Management` wrapper. Transport Ping/Pong handles heartbeats separately
from application commands. TLS and HTTP Basic authentication are configurable.

The [Slint MMI](https://github.com/misadeks/tetra-bluestation-mmi) implements the
application side, including audio and PTT. See the [setup guide](../../docs/MS_GETTING_STARTED.md)
to build it, populate its codec tables and connect the two processes.

## Read-only reference server

`reference-client.py` listens on the application side despite its filename:

```sh
python -m pip install "websockets>=14"
python examples/ms-interface/reference-client.py
```

In another terminal:

```sh
cargo run -p magentastation -- example_config/config-offline.toml
```

It binds only `127.0.0.1`, negotiates the existing subprotocols, reads schema,
state and redacted configuration, and prints telemetry. It never writes
configuration, starts a radio procedure or generates audio. `--control-port`
and `--telemetry-port` may be changed together with the TOML endpoints.

The offline backend instantiates no PHY and produces no TDMA ticks. Requests
requiring protocol time remain deferred until a real downlink exists; offline
management is not a radio simulator.

Each entity admits at most 64 queued control commands. MM also defers at most
64 unsynchronized requests; excess requests receive a correlated negative
acknowledgement (or a management `Error`). Speech remains fire-and-forget.
WebSocket messages are limited to one MiB. These ingress bounds are not a
whole-process memory bound.

## Plane B: non-standard management/provisioning

These are implementation-defined, not standardized TETRA PEI commands.
The authoritative payload definitions are
[`management/mod.rs`](../../crates/tetra-entities/src/management/mod.rs).

| Command | Payload | Response / behavior |
| --- | --- | --- |
| `GetInterfaceVersion` | `{handle}` | `InterfaceVersion {handle, version}` |
| `GetState` | `{handle}` | `State {handle, state}` |
| `GetConfig` | `{handle}` | `Config {handle, toml}` with redacted secrets |
| `SetConfig` | `{handle, toml}` | Validates and persists; `Ack` |
| `ApplyConfig` | `{handle}` | If a structural edit is pending: detach drain and exit 75 for supervisor restart; otherwise an acknowledgement without restart |
| `ActivateScanlist` | `{handle, name, active}` | Reconciles desired group affiliation |
| `SetCellSelectionMode` | `{handle, manual}` | Automatic/manual selection; rejects unsafe call-time changes |
| `StartCellScan`, `StopCellScan` | `{handle}` | Receive-only carrier survey; emits results/completion |
| `CampOnCell` | `{handle, carrier_hz, register}` | Operator-selected camp, optionally forced registration |

`Ack` carries `{handle, accepted, restart_required, message}`. `Error` carries
`{handle, message}`. Live state contains registration/service status, own ISSI,
home MCC/MNC, serving LA, optional RSSI, colour code, attached groups, active
scanlists, pending-restart state and manual-selection state.

Examples (shown as JSON; encode to bytes before WebSocket send):

```json
{"Management":{"GetInterfaceVersion":{"handle":1}}}
```

```json
{"Management":{"InterfaceVersion":{"handle":1,"version":"bluestation-ms-interface-6"}}}
```

**Hybrid apply:** changes confined to the operational codeplug apply live.
Structural changes (radio, network identity, ISSI, endpoints, duplex settings)
are staged and require `ApplyConfig`; a supervisor reloads the same TOML after
exit 75. Examples are
[`magentastation.service`](../../example_config/magentastation.service) and
[`magentastation-supervisor.sh`](../../example_config/magentastation-supervisor.sh).
`GetConfig` replaces endpoint passwords with `"********"`; posting an unchanged
sentinel preserves the corresponding active secret. Disk TOML still stores the
real password. The radio must have write access to its startup TOML; write-back
normalizes formatting/comments. See [persistence](../../docs/MS_GETTING_STARTED.md#provisioning-and-persistence).

Shutdown joins the control worker after its final responses are flushed.
Connection/handshake and writes have bounded timeouts; an undeliverable response
is reported as a shutdown delivery failure, not silently treated as delivered.

These management calls work without synchronization. Other commands wait for
real protocol ticks, so the offline backend is useful for provisioning but
not for testing registration, floor grants or voice.

## Plane A: TNMM / TNCC / TNSDS

These application payloads model ETSI TS 100 392-2 SAP primitives; the WebSocket
wrapper, correlation `handle` and call selector are local transport details.

| Family | Inbound control variants | Outbound telemetry |
| --- | --- | --- |
| TNMM, clause 15.3 | `TnmmRegistration`, `TnmmDeregistration`, `TnmmAttachDetachGroupIdentity`, `TnmmStatus`, `TnmmEnergySaving` | Registration, service, group identity, report, status and energy indications/confirms |
| TNCC, clause 11.3.3 | `TnccSetup`, `TnccSetupResponse`, `TnccComplete`, `TnccTx`, `TnccDtmf`, `TnccRelease` | Setup, proceed, alert, complete, floor/Tx, notify, DTMF and release indications/confirms |
| TNSDS, clause 13.3 | `TnsdsUnitdata`, `TnsdsStatus`, `TnsdsSendMessage`, `TnsdsSendReport`, `TnsdsCancel` | Unitdata, status, message and delivery/read-report indications |

`TnmmAck`, `TnccAck` and `TnsdsAck` report **acceptance for processing**, not
on-air success. Their `{handle, accepted, detail}` is distinct from the eventual
asynchronous protocol outcome. Status/energy procedures that are not implemented
remain explicitly dormant/rejected; a schema entry is not a support claim.

Exact payload types:
[control commands](../../crates/tetra-entities/src/net_control/commands.rs),
[telemetry](../../crates/tetra-entities/src/net_telemetry/events.rs),
[TNMM](../../crates/tetra-entities/src/tnmm/mod.rs),
[TNCC](../../crates/tetra-saps/src/tncc/mod.rs),
[TNSDS](../../crates/tetra-saps/src/tnsds/mod.rs).
Use these definitions rather than guessing omitted optional fields.

For example, after registration an application can switch to one talkgroup:

```json
{"TnmmAttachDetachGroupIdentity":{"handle":31,"request":{"group_identity_attach_detach_mode":"DetachTheCurrentlyActiveGroupIdentities","group_identity_request":[{"gtsi":101,"group_identity_attach_detach_type_identifier":"Attachment","class_of_usage":"ClassOfUsage1","group_identity_detachment_request":null}],"group_identity_report":null}}}
```

The control reply is `TnmmAck`; the result arrives as
`TnmmAttachDetachGroupIdentityConfirm` on telemetry. A bare GSSI is accepted
in the low 24 bits of `gtsi`; a full GTSI also contains the network identity.
`Amendment` changes only the listed identities, whereas
`DetachTheCurrentlyActiveGroupIdentities` replaces the affiliation set.

## Speech and survey streams

`MsSpeechFrame` emits decoded downlink type-1 bits:
`{call_identifier, timeslot, sequence, transmitting_party_ssi, frame_bits,
bad_frame, data}`. TCH/S is 274 bits (two 137-bit ACELP frames), **one bit per
byte**, not packed PCM. The application owns the vocoder and bad-frame concealment.
`MsUplinkSpeech {call_identifier, frame_bits, data}` supplies the corresponding
uplink bits; the stack gates them by call/floor state. It is fire-and-forget,
not a per-frame acknowledged control procedure.

To transmit, send a TNCC setup or floor request, wait for the corresponding
grant, and send speech only for that call while its transmit permission is
active. Release the floor on PTT-up. A transport acceptance acknowledgement
is not a floor grant. The radio handles timing and burst scheduling; the
application handles 8 kHz PCM capture, 60 ms codec blocks and playback.

`MsScanResult` carries carrier, MCC/MNC, optional LA/colour/RSSI/registration
requirement and late-entry support. `MsScanComplete {found, scanned}` terminates
a survey. Radio access and decoded measurements require an actual PHY.

Legacy external enum slots remain codec-compatible even where the MS does not
implement their old use. See [capabilities and limits](../../docs/MS_MODE.md#capabilities-and-limits).
