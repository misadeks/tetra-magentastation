# MS architecture and limitations

MagentaStation implements the mobile-station side of TETRA V+D.
[BlueStation](https://github.com/MidnightBlueLabs/tetra-bluestation) is its
original project and base-station counterpart. The implementation references
ETSI TS 100 392-2 V3.10.1 (2023-03).

## Components

```text
Slint MMI or another application (UI, microphone/speaker, ACELP)
    | control / telemetry / speech: binary WebSocket JSON
Desktop adapters (tetra-entities, tetra-config)
    | events and actions
tetra-ms-core (LLC, MAC/RA/reassembly, MM, MLE, CMCE, SDS)
    |
tetra-ms-types (TDMA, identities, packed bits, frequencies)

Desktop PHY/LMAC -> tetra-phy-core (logical-channel coding)
    |
SoapySDR: receive downlink, transmit granted uplink bursts
```

The protocol state machines live in the allocator-free core. Desktop adapters
translate SAPs/buffers, track receipts, queue speech, log events, and handle
configuration, threads and drivers. The optional C ABI exposes independently
owned protocol components; it is not another implementation of the stack.

`MessageRouter` takes its protocol clock from recovered downlink slots.
While unsynchronized it can service configuration/state management, but does
not invent TDMA ticks. `backend = "None"` is an offline management backend,
not a radio simulator.

The SDR receives the serving-cell downlink and transmits discontinuous uplink
bursts. The uplink carrier comes from SYSINFO; PHY rejects bursts before the
TX retune. Speech coding runs in the external application, not the radio.

## Capabilities and limits

| Area | Implemented | Not implemented / limits |
| --- | --- | --- |
| PHY/LMAC | Downlink sync/demodulation, NUB/CUB uplink, control/TCH/S coding and STCH stealing | Other modulation modes; sample transport remains driver-dependent |
| MAC | AACH/SYSINFO/RESOURCE/FRAG/END receive, identity filtering, random/reserved access and fragmentation | Encrypted fragments, extended allocation procedures; bounded queues |
| LLC | Basic-link sequencing, early/piggyback/automatic ACKs, FCS and retries | Duplicate downlink delivery is retained; an outbound FCS flag does not append an FCS |
| MLE | Initial selection, ordered scanning, manual survey/camp, network/class suitability, BREAK/REOPEN and routing | Full neighbor reselection/handover; no strongest-cell ranking |
| MM | ITSI attach, retry/reject handling, group affiliation, detach and supported location updates | Alias adoption, authentication/OTAR, energy economy and enable/disable |
| CMCE | Group, individual and duplex calls, floor control, restoration paths, DTMF and concurrent-call arbitration | Full supplementary services |
| SDS | Data/status, SDS-TL transfer/report/ACK tracking, delivery/read reports and cancellation | Concatenated messages |
| Speech | Decoded downlink bits/BFI and grant-gated uplink bits | No in-stack ACELP codec or audio device |
| Interfaces | TNMM/TNCC/TNSDS, state, codeplug/config persistence and supervised restart | Management is application-defined, not standardized PEI |

There is no BS or monitor runtime, DMO, air-interface encryption or SNDCP
packet-data service. Carrier override hints (including `rx_only`) are stored
but not enforced by the radio; see the [configuration reference](MS_CONFIG.md#carrier-overrides).
Schema entries for dormant procedures do not imply those procedures work.

Software regression and cross-compilation do not establish live RF
interoperability. Driver/sample timing, RF performance and embedded stack
watermarks must be measured on the target setup.

## Source map

Desktop paths below are relative to `crates/tetra-entities/src`; portable paths
are relative to `crates`:

| Layer | Desktop adapter | Portable implementation |
| --- | --- | --- |
| LLC | `llc/llc_ms.rs`, `llc/ms_adapter.rs` | `tetra-ms-core/src/llc.rs` |
| MAC/RA | `umac/umac_ms.rs`, `umac/subcomp/ms_*.rs` | `tetra-ms-core/src/mac/`, `random_access.rs`, `reassembly.rs` |
| MLE | `mle/mle_ms.rs` | `tetra-ms-core/src/mle.rs` |
| MM | `mm/mm_ms.rs` | `tetra-ms-core/src/mm.rs` |
| CMCE/SDS | `cmce/cmce_ms.rs`, `cmce/subentities/` | `tetra-ms-core/src/cmce.rs`, `sds.rs` |
| Coding | `lmac/lmac_ms.rs`, `lmac/components/` | `tetra-phy-core/src/` |
| PHY | `phy/phy_ms.rs`, `phy/components/` | Platform adapter |

Shared network PDU parsers remain because receiving infrastructure messages is
part of MS operation. Existing `bluestation-*` interface identifiers and legacy
external enum slots are compatibility vocabulary, not BS runtime selection.

See [setup](MS_GETTING_STARTED.md), [configuration](MS_CONFIG.md),
[application interface](../examples/ms-interface/README.md) and
[portable Rust/C integration](PORTABLE_MS.md).
