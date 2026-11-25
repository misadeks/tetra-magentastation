# MagentaStation MS architecture and scope

MagentaStation contains the mobile-station side of the original
[BlueStation](https://github.com/MidnightBlueLabs/tetra-bluestation) project.
The desktop binary is `magentastation`; `stack_mode = "Ms"` is the only accepted
role. `"Bs"` and `"Mon"` are explicitly unsupported, not alternative routes to
a base-station implementation.

The specification referenced by the retained implementation is ETSI TS 100 392-2
V3.10.1 (2023-03), TETRA V+D Part 2: Air Interface. Supported behavior is
preserved, not a claim of complete conformance.

## Protocol ownership

```text
External application (operator UI, ACELP vocoder, storage policy)
    | control / telemetry / speech: WebSocket binary JSON
Desktop adapters (tetra-entities, tetra-config)
    | translated events and polled actions
tetra-ms-core (LLC, MAC/RA/reassembly, MM, MLE, CMCE, SDS)
    |
tetra-ms-types (TDMA, identities, packed bits, frequencies)

Desktop PHY/LMAC -> tetra-phy-core (pure logical-channel coding)
    |
SoapySDR: receive downlink, transmit granted uplink bursts
```

There is one authoritative supported MS protocol implementation per layer.
Desktop SAP/BitBuffer conversion, logging, receipt sidecars, audio FIFOs,
codeplug enumeration, persistence, threads and drivers remain outside the
allocator-free cores. The optional C ABI exposes seven separately owned
components rather than a second stack.

`MessageRouter` is receive-timed: only recovered downlink slots drive protocol
ticks. No BS free-running clock or runtime remains. During loss of synchronization,
offline-safe management continues to run, but protocol commands requiring time
remain deferred. The explicit `None` backend opens no radio, sleeps between
management polls, and generates no synthetic TDMA time.

The SDR receives the serving-cell downlink and transmits discontinuous uplink
bursts. The uplink carrier is derived from received SYSINFO; `PhyMs` refuses
transmission before the TX retune. The existing recovered-time, reserved-slot,
stealing and receipt semantics are unchanged. No hardware was accessed while
creating this repository.

## Retained procedures and boundaries

| Area | Included | Limits |
| --- | --- | --- |
| PHY/LMAC | Downlink synchronization and demodulation; NUB/CUB uplink; control and TCH/S coding; STCH stealing | Hardware/sample transport stays platform-owned; other modulation modes unsupported |
| MAC | AACH/SYSINFO/RESOURCE/FRAG/END receive, own/group/broadcast filtering, random/reserved access, uplink fragmentation | Bounded queues; encrypted fragments and extended allocation procedures remain unsupported |
| LLC | Basic-link sequencing, early/piggyback/automatic ACKs, FCS and retries | Existing duplicate-delivery and outbound-FCS semantics retained |
| MLE | Network/class suitability, initial selection, scanning, manual survey/camp, BREAK/REOPEN and routing | Full neighbor reselection, handover and restoration procedures not added |
| MM | ITSI attach, rejection/retry timers, group affiliation, deregistration, supported location updates | Alias adoption, authentication/OTAR, energy economy and unsupported management procedures not added |
| CMCE | Group, individual and duplex call control; floor control; restoration paths; DTMF; concurrent-call arbitration | Full supplementary services and unsupported ETSI procedures remain deferred |
| SDS | User data/status, SDS-TL transfer/report/ACK tracking, delivery/read reports and cancellation | Concatenation not implemented |
| Speech | Downlink decoded bits/BFI to application; uplink speech bits gated by call/floor state | No in-stack ACELP codec |
| Desktop interfaces | TNMM/TNCC/TNSDS control, telemetry, runtime state, codeplug, persistent config, supervised restart | Non-standard management plane; not an ETSI PEI implementation |

Authentication, air-interface encryption, enable/disable, DMO and SNDCP packet
data remain unsupported. No cryptography or TEA1 behavior was redesigned.

The source snapshot included reports of RF success in its original environment.
Those are historical upstream reports, **not validation of this derivative**.
The current repository's evidence is software regression, compilation and
offline loopback integration; live RF timing, sample scheduling, RF power,
driver behavior and embedded stack watermarks require separate validation.

## Source map

| Layer | Desktop adapter | Portable implementation |
| --- | --- | --- |
| LLC | `llc/llc_ms.rs`, `llc/ms_adapter.rs` | `tetra-ms-core/src/llc.rs` |
| MAC | `umac/umac_ms.rs`, `umac/subcomp/ms_*.rs` | `mac/`, `random_access.rs`, `reassembly.rs` |
| MLE | `mle/mle_ms.rs` | `mle.rs` |
| MM | `mm/mm_ms.rs` | `mm.rs` |
| CMCE | `cmce/cmce_ms.rs`, `cmce/subentities/cc_ms/` | `cmce.rs` |
| SDS | `cmce/subentities/sds_ms.rs` | `sds.rs` |
| Coding | `lmac/lmac_ms.rs`, `lmac/components/` | `tetra-phy-core/src/` |
| PHY | `phy/phy_ms.rs`, `phy/components/` | Platform adapter, not protocol ownership |

Desktop paths above are relative to `crates/tetra-entities/src`; portable paths
without a crate name are relative to `crates/tetra-ms-core/src`.

## Compatibility and removals

Control, telemetry, TNMM/TNCC/TNSDS, management and speech message shapes and
their schema/subprotocol identifiers are retained. The legacy `TetraEntity::Brew`
wire discriminant is reserved for codec compatibility, but no entity, transport
bridge, scheduler or configuration implements that integration.

Actual BS entities, channel/circuit schedulers, subscriber registry, broadcasting
configuration, network/Brew routing, downlink transmitter and uplink receiver
roles, BS tests/examples and release publishing workflow are absent. Shared
network PDU parsers/encoders remain because they are MS receive vocabulary or
wire-reference fixtures. SNDCP remains an explicit unsupported boundary rather
than a renamed BS implementation.

See [portable integration and validation](PORTABLE_MS.md),
[configuration](MS_CONFIG.md), and the
[external interface catalog](../examples/ms-interface/README.md).
