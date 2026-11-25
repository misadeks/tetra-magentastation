# Portable MS MLE

`tetra-ms-core::mle` owns the implemented MS Mobile Link Entity protocol.
`tetra-entities::mle::mle_ms` is its desktop adapter, not another state machine.
The portable module depends only on `tetra-ms-types`; it requires neither
`std`, `alloc`, desktop PDU/SAP crates, a logger, nor a clock.

## Calling the component

Create `MleCore`, then call `handle(config, event)` with a borrowed `MleConfig`
snapshot. The caller enumerates codeplug carriers and supplies the home network,
allowed networks and optional subscriber class. The core decides network/class
suitability itself; the home network remains allowed independently of the list.
Policy slices are borrowed only during `handle`, not retained.

Drain `pop_action()` after successful events. Actions describe retunes, MAC
configuration, MM/CMCE service notifications, survey results, and uplink/downlink
data. The desktop adapter translates these into SAP messages, preserves
transmission reporters/channel-allocation metadata, and logs explicit errors.

The PHY/caller owns time and supplies `Dwell` expiry and `Monitor` heartbeat
events. No timer is started internally. Survey SYSINFO waiting retains the
existing bounds: four repeated SYNCs after the first SYNC, or eight monitoring
heartbeats since the last decodable SYNC. Survey carriers are visited once;
automatic selection wraps its cached candidate set.

## Owned behavior

- D-MLE-SYNC and complete 42-bit D-MLE-SYSINFO parsing, including tolerated
  reserved service-detail bits.
- Serving-cell identity and parameters, subscriber-class suitability,
  initial selection, automatic scanning, manual camp arming and receive-only
  surveys.
- One-shot activation and re-confirmation on location-area or system-wide-service
  changes; registration-flag/subscriber-bitmap changes alone retain the existing
  behavior.
- RSSI caching, BREAK/REOPEN idempotence and ordered MM/CMCE notifications.
- Three-bit protocol-discriminator validation, MM/CMCE/SNDCP dispatch, MM/CMCE
  uplink discriminator encoding, endpoint/link defaults, priority, emergency and
  stealing policy.
- Identity-filter and U-plane configuration routing.

`LinkParameters` and `DeliveryParameters` expose the existing protocol defaults
(including unused-field defaults) in fixed-width portable types. The adapter does
not choose these defaults. `MleState`, `pending_actions()` and the target-specific
`MEMORY_FOOTPRINT` constant support composition and memory budgeting.

Both acknowledged and unacknowledged downlinks retain identical routing.
SNDCP deliberately retains the existing CMCE destination until a separate SNDCP
entity exists. Desktop deliveries preserve the original BitBuffer window and
cursor representation, including the consumed discriminator.

## Bounds and errors

| Persistent resource | Bound |
| --- | --- |
| Automatic-scan carriers | 256 |
| Survey carriers | 256 |
| Groups per identity configuration | 64 |
| Queued actions | 8 |
| Downlink delivered payload | 4096 bits (`FragmentBits`) |
| Uplink PDU including discriminator | 2688 bits (`PackedBits`) |

On the Windows x64 development target, `MEMORY_FOOTPRINT` reports 6,412 bytes
for `MleCore`, 536 per action, 40 per event, and 68 for `MleState`. The action
queue occupies 4,292 bytes; each candidate array occupies 1,024 bytes. These
are target-specific measurements, not ABI layout promises.
ARM `thumbv8m.main-none-eabihf` code generation confirms the same core/action/state
sizes and a 24-byte event.

Candidate order is caller-owned and preserved. Lists are snapshotted at scan or
survey start; later policy updates do not replace an in-progress carrier list.
Oversized candidate/group/bit buffers return `CapacityExceeded`, never truncate.
`QueueFull` is backpressure: drain actions, then retry the same input. Reservation
uses the event's worst-case action count (four for monitoring, two for broadcast
or dwell, one for other emitting events). All failures leave protocol state and
queued actions unchanged, including dwell/monitor counters.

Malformed/truncated discriminators and broadcast PDUs return explicit errors.
Existing unsupported internal MLE PDUs and management traffic return
`UnsupportedMlePdu` / `UnsupportedManagement` without forwarding. No restoration,
handover, network-broadcast interpretation, or management protocol is newly
implemented by this extraction. Previously unimplemented TLMC/TLPD input
primitives remain desktop stubs. The desktop adapter logs rejected events and
marks rejected uplink reporters discarded.

### Remaining desktop ownership

Only configuration/codeplug enumeration, logging, BitBuffer representation,
SAP representation and opaque reporter/channel-allocation correlation remain.
There is no serving-cell, selection, survey, service-state, timer/counter,
discriminator encoding or protocol-dispatch decision retained in the adapter.
The previously unsupported TLMC/TLPD input stubs remain there; unsupported
internal MLE/management PDUs are explicitly rejected in the portable core.
This extracts implemented behavior, not all unimplemented ETSI procedures.

## Focused validation

Before moving ownership, the original desktop implementation passed 31 MLE
tests, including newly added activation/cancellation/unsupported-route
characterization. The preserved tests now observe the portable core.
Three additional adapter regressions cover both downlink services' SAP metadata
and cursor shape, MM priority/reporter handling, and CMCE channel-allocation
correlation. Twelve portable tests cover selection/survey/service behavior,
bit-exact routing, truncation, 4096-bit reassembly and atomic capacity failures.

```text
cargo test -p tetra-ms-core --test mle --no-default-features
cargo test -p tetra-entities mle::mle_ms::tests --lib
cargo clippy -p tetra-ms-core --no-default-features -- -D warnings
cargo check -p tetra-ms-core --no-default-features --target thumbv8m.main-none-eabihf
```
