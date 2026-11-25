# Portable MS components

**Supported MS protocol ownership is portable.** The desktop MS is an adapter
over allocator-free `no_std` LLC, MAC/access/reassembly, MLE, MM, CMCE/floor and
SDS components. Pure logical-channel coding algorithms are in `tetra-phy-core`.
Hardware, audio and application services remain outside. Previously unsupported
procedures are not newly implemented. No encryption or TEA1 behavior has changed.

## Repository inventory and extraction boundary

The MS-only workspace consists of the portable types/protocol/coding/C ABI
crates, desktop `tetra-core`, `tetra-config`, `tetra-saps`, `tetra-pdus`,
`tetra-entities`, and the `magentastation` executable. It accepts only the MS
role; base-station and monitor configuration values are rejected.

| Responsibility | Source and platform coupling | Extraction state |
| --- | --- | --- |
| TDMA calculations and addresses | `tetra-core/src/{tdma_time,address}.rs` | Implementation moved to `tetra-ms-types`; old paths reexport it |
| Carrier/duplex arithmetic | `tetra-core/src/freqs.rs`, configuration DTOs | Standardized spacing table, offset mappings and checked Hz arithmetic moved to `tetra-ms-types::frequency`; desktop DTO delegates |
| Packed bits | `tetra-core/src/bitbuffer.rs`, allocating `Vec<u8>` | Checked borrowed views and fixed packed storage added; desktop buffer retained |
| LLC sequencing, ACKs, retries, uplink encoding, downlink parsing | `tetra-entities/src/llc/llc_ms.rs`, SAP queues, `TxReporter`, configuration, logging | MS execution moved to `tetra-ms-core::llc`; desktop SAP/receipt translation in `llc/ms_adapter.rs` |
| FCS | `llc/components/fcs.rs`, desktop bit buffer | Algorithm moved to `tetra-ms-core::fcs`; old API wraps it |
| UMAC, access, uplink scheduling | `umac/umac_ms.rs`, `umac/subcomp/ms_random_access.rs`, shared runtime configuration, heap queues, RNG | Supported state, scheduling, allocation/filtering, codecs and random access extracted; entropy and reporter sidecars remain in desktop |
| Fragmentation/reassembly | `umac/subcomp/ms_defrag.rs`, allocating buffers | Downlink reassembly moved to bounded `tetra-ms-core::reassembly`; desktop only translates buffers and logs |
| Cell selection, scan, MLE routing | `mle/mle_ms.rs`, configuration, telemetry and control | Supported selection/survey/service state, routing and codecs moved to `mle::MleCore`; desktop converts SAPs/configuration |
| Registration, rejection, detach, group attachment and retries | `mm/mm_ms.rs`, configuration locks, codeplug, telemetry/control and persistence | Supported MM states, timers, group reconciliation and codecs moved to `mm::MmCore`; desktop retains persistence/operator conversion |
| Group/private calls and floor control | `cmce/subentities/cc_ms/`, SAP queues, audio/control integration and shared state | Supported downlink decoding, call/floor/restoration state, timers, orchestration, demultiplexing and uplink codecs moved to `cmce::CmceCore`; audio FIFOs are desktop sidecars |
| SDS | `cmce/subentities/sds_ms.rs`, heap-backed SAP payloads | Wire/TL handling, reports/ACKs and 256-reference tracking extracted to `sds::SdsCore` |
| Other PDU encoders/decoders | `tetra-pdus`, depends on desktop core/configuration/SAP crates | Supported MS wire paths live in portable MAC/MM/MLE/CMCE/SDS modules; shared network PDUs remain for MS reception, DTO conversion and reference tests |
| Logical channels and coding | `lmac_ms.rs`, `lmac/components/`, heap buffers and desktop interfaces | CRC, scrambling, interleaving, puncturing, convolutional/Viterbi and RM coding, traffic reorder and error-control algorithms extracted to `tetra-phy-core`; desktop adapters retain SAP/buffer translation |
| RF/sample transport | `phy_ms.rs`, `phy_io/`, SDR, streams, process timing/threads | Remains platform-owned; pure algorithms still need separating |
| Operator state and interfaces | `net_control`, `net_telemetry`, `management`, `network`, JSON/binary codecs and sockets | Remains desktop-owned |
| Provisioning and storage | `tetra-config`, codeplug/TOML, filesystem and shared configuration | Remains desktop-owned |
| Audio and speech transport | Desktop entities and SAPs | Remains desktop-owned |

No private BS LLC implementation remains. Its six historical MS-specific cases
are covered through the supported public adapter instead: automatic ACK identity
policy covers own/foreign individual identities and non-individual addresses;
stop-and-wait/early-ACK cases cover detach unblocking and own-ISSI floor ACKs;
wire assertions cover acknowledged traffic stealing and link/metadata propagation.
The three BS-only LLC cases are not part of this workspace.

### Dependency graph

```text
magentastation
  -> tetra-entities
       -> tetra-ms-core -> tetra-ms-types
       -> tetra-pdus -> tetra-core, tetra-saps, tetra-config
       -> tetra-core, tetra-saps, tetra-config
  -> tetra-core, tetra-saps, tetra-config

tetra-saps   -> tetra-core
tetra-config -> tetra-core
tetra-core   -> tetra-ms-types [desktop-codecs]
tetra-ms-ffi -> tetra-ms-core -> tetra-ms-types
tetra-entities -> tetra-phy-core (no dependencies)
```

Default portable dependencies stop at `tetra-ms-types`: neither portable crate
uses `alloc`, an allocator, `std`, a clock, a scheduler, logging, locks,
networking, storage, audio or an SDR. The optional `desktop-codecs` feature on
types enables the existing serde/bitcode representations, not another protocol
implementation. The optional `std` features enable desktop conveniences and
examples. `tetra-ms-ffi` is optional simply by not depending on it.

### MAC access and reassembly

`random_access::MsRandomAccess` now owns access attempts, permission checks,
IMM windows, frame-marker/subslot counting, WT responses, retries and terminal
outcomes. `AccessParamStore` owns access-code precedence. No RNG callback enters
the portable engine: `entropy_needed` reports a typed `EntropyRequest`, the
adapter draws exactly the requested value, and `poll_downlink_slot` consumes
`RaChoices`. Entropy probing copies only a small bounded access state, never a
PDU or the LLC engine. Invalid time/parameters/entropy and initiation of a busy
attempt are explicit failures. First-try expiry is now wrap-safe at the TDMA
cycle boundary, with a regression test.

`reassembly::MsReassembly` owns four independent timeslot chains, each capped at
4,096 bits in a four-byte-aligned `FragmentBits` buffer. Replacement, timeout,
invalid time, capacity and aborted overflow are explicit. Its historical timeout
remains strictly more than 40 slots since the last fragment. Continuation
overflow aborts that chain as before; an oversized first fragment is rejected.
Completed data is borrowed until `release`; reset clears storage in place.
Fragment metadata is fixed-width. Encryption metadata remains opaque and no
encryption implementation has changed. A pure Rust integration test verifies
fragmented golden bits through reassembly, LLC delivery and automatic ACK.

The shared `BitStorage<N>` backing type serves the existing 336-byte LLC
`PackedBits` and 512-byte MAC `FragmentBits` aliases without changing LLC/C ABI
layout. Larger backing capacities cannot cause hidden allocation.

## Rust boundary and ownership

The basic-link engine is `tetra_ms_core::llc::LlcCore`:

```rust,ignore
LlcCore::new(LlcConfig { own_issi }) -> Result<LlcCore, LlcError>
LlcCore::init_in_place(&mut MaybeUninit<LlcCore>, config) -> Result<&mut LlcCore, LlcError>
core.reset(config) -> Result<(), LlcError>
core.handle(now: TdmaTime, event: &LlcEvent<'_>) -> Result<(), LlcError>
core.poll_action() -> Option<LlcAction>
core.status() -> LlcStatus
```

`mm::MmCore` exposes `new`/`init_in_place`, `handle(Event)`, `poll_action`,
`state` and service status. Recovered elapsed slots and the current detach
transfer disposition are typed `Tick` input, never a global clock. It owns
T351/T353, retries/rejection lockout, group reconciliation, detach draining and
supported MM wire codecs. Groups are capped at 63 and actions at eight.

`mle::MleCore` exposes `new`/in-place initialization, `handle(config, event)`,
`pop_action` and typed state. Configuration borrows last only for the handle
call; active scan/survey carrier lists are bounded snapshots. It owns 256
candidate carriers, 64 identities/groups and eight actions. Synchronization,
system information, decoded LLC delivery, surveys, dwell/monitor indications
and uplink requests are explicit inputs. Existing unsupported internal MLE and
management procedures remain explicit errors rather than invented behavior.

The portable registration integration test exercises serving-cell acquisition,
MM registration encoding, MLE protocol envelopes, LLC sequencing and receipts,
MAC-ACCESS wrapping, and the reverse D-LOCATION-UPDATE-ACCEPT path without any
desktop crate or platform service.

The other Rust entry points are:

| Component | Caller-driven boundary |
| --- | --- |
| `mac::MsMac` | `init_in_place`/`reset`, `submit_sdu`, `tick`, `resource_event`, `aach`, `allocate`, `grant_fragment`, `reserved`, `traffic`, `snapshot`; typed submit/burst/receipt outcomes |
| `random_access::MsRandomAccess` | `initiate`, `entropy_needed`, `poll_downlink_slot`, `on_mac_resource`, `cancel`, `status`; typed caller entropy and `RaAction` |
| `reassembly::MsReassembly` | `init_in_place`/`reset`, `first`, `next`, `age`, `completed`, `release`, `status`; borrowed completed PDU |
| `cmce::CmceCore` | `init_in_place`/`reset`, `push_event(now, CmceEvent)`, `pop_action`, `status`, `receive_traffic`, `transmit_target`; call/floor/SDS-routing and speech metadata actions |
| `sds::SdsCore` | `init_in_place`/`reset`, `push_event(SdsEvent)`, `pop_action`, `status`, `cancel`; typed data, reports, ACKs and reference tracking |
| `tetra_phy_core` | Checked control/speech encode/decode and Viterbi APIs with caller-owned workspaces; pure CRC, scrambling, interleaving, puncturing and reorder helpers |

`MsMac` uses fixed `u32` receipt handles. Desktop reporters and their atomic/heap
ownership live only in adapter sidecars. Generic MAC utilities admit only
`Copy` handles and never clone or drop platform payloads. CMCE calls cannot
contain generic application storage: audio FIFOs are separate desktop state.

Additional native bounds are 16 MAC queued transfers, six reserved slots, four
audio frames, 16 stolen blocks and 256 filter groups; 16 CMCE calls and 16
originations with 40 actions; and four SDS actions with 256 tracked references.
Historical MAC drop-oldest behavior returns the displaced handle explicitly so
the adapter can notify the owner. It is not a silent overwrite.

Inputs are `Send`, `Receive`, `Transmission` and `Tick`. Outputs are `Transmit`,
`Deliver` and `Completed`. Status reports queue use, tracked links and ignored
ACKs. Unsupported LLC types, invalid configuration/time, truncation, invalid
FCS, capacity, queue exhaustion and stale receipts have distinct errors.

The caller supplies ordered TDMA time and an end-of-tick event; there is no
implicit wall clock or sleeping. Time arithmetic is the original TDMA
implementation, including wraparound. T251 remains 16 slots and N252 remains
three retransmissions. `Transmitted` or `Rejected` feedback starts T251.
`Deferred` retains ownership without starting it: the transport must retain the
request and eventually provide another disposition for that same attempt.

An acknowledged request owns a caller command key until its `Completed` action
has been consumed. An unacknowledged request releases its key when its
`Transmit` action is consumed. Every acknowledged MAC attempt has a distinct
`Receipt { epoch, sequence }`. Feed back the receipt of the actual attempt, not
an older attempt or merely an enqueued PDU. Normal unmatched network ACKs are
counted in status; stale local transport receipts return an error.

Input `BitView`s borrow bytes only for `handle`; callers can reuse them when it
returns. Views explicitly use MSB-first bytes and fixed-width bit offsets,
without unaligned casts or a host-endian representation. Actions own bounded
packed PDUs; retransmissions retain their own encoded copy. No borrowed input
or arbitrary callback escapes into the protocol state.

Drain actions after every successful event and before another input. Execute
transport, audio, logging or application work after returning from the core.
An error does not consume an input command or partially mutate protocol state.
Queue backpressure is explicit, never an allocating fallback.

### Bounded storage

| Storage | Capacity | Full behavior |
| --- | ---: | --- |
| Acknowledged uplink | 8 | `QueueFull` |
| Unacknowledged/group uplink | 8 | `QueueFull` |
| Scheduled automatic ACKs | 8 | `QueueFull` |
| Tracked SSI sequences | 8 | `CapacityExceeded`; retained until reset, no implicit eviction |
| Action queue | 24 | `QueueFull`, including preflight of a tick's upper bound |
| Encoded PDU | 2,688 bits / 336 bytes | `CapacityExceeded` |

PDU capacity includes its LLC header. Maximum outbound SDU sizes are 2,683 bits
for BL-DATA, 2,682 for piggybacked BL-ADATA and 2,684 for BL-UDATA. Inputs at the
boundary are tested, including a one-bit overflow. These bounds cover this
basic-link component only, not a general SDS or MAC-fragment capacity.

Reset validates configuration first, cancels all queued commands/actions,
clears sequence state in place and advances the receipt epoch. The adapter must
notify its own callers about cancellations and discard outstanding transport
work. Old receipts cannot complete new commands. Generation exhaustion is
explicit. Do not initialize a live storage object again in place of reset:
fresh initialization starts a new receipt namespace.

### Desktop integration

`llc/llc_ms.rs` constructs only the MS adapter. `ms_adapter.rs` translates SAPs, preserves
metadata, bridges shared transport reporters to receipt events and converts
actions back to SAPs. It does not own sequence, ACK or retry decisions.

Configuration, `BTreeMap` metadata, heap-backed SAPs and reporters are desktop
services. Identity changes reset the core and explicitly cancel stored
requests. Other roles cannot be represented in the MS-only configuration enum.
Borrowing `BitBuffer::as_bit_view()` avoids copying input PDUs;
cursor/window behavior has dedicated bridge tests.

Run the minimal Rust caller with:

```text
cargo run -p tetra-ms-core --features std --example desktop
```

### Embedded Rust integration

[`examples/embedded.rs`](../crates/tetra-ms-core/examples/embedded.rs) is a
`#![no_std]` library example with initialization, downlink input, MAC feedback,
tick and action-polling functions. It imposes no board runtime or panic handler:

```text
cargo build -p tetra-ms-core --no-default-features --example embedded --target thumbv8m.main-none-eabihf
```

Firmware should give `init_in_place` exclusive, aligned `MaybeUninit<LlcCore>`
storage from its task-owned/static memory. Do not create a full temporary
`LlcCore::new()` on a small task stack and then move it to static memory.
Serialize access externally. Use the board's recovered TDMA time, not loop
iteration speed, as `now`. Handle every error and retain transport ownership
until its disposition is reported. This example does not implement a radio,
PHY, MAC scheduler, registration or calls.

## Versioned C ABI

The optional ABI is a set of independently owned component objects. Every
component uses version 1; each header documents its complete integer event and
action vocabulary, bounds and errors. The caller routes typed actions between
components and executes platform work after returning from protocol calls.
It need not implement packet codecs, retry rules or protocol state transitions.

| Component | Header | Export prefix | Operations after the prefix |
| --- | --- | --- | --- |
| Basic LLC | `tetra_ms.h` | `tetra_ms_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| MAC and random access | `tetra_mac.h` | `tetra_ms_mac_` | `abi_version`, `storage_size`, `storage_align`, `init`, `reset`, `push_event`, `entropy_needed`, `poll_action`, `get_status` |
| MLE | `tetra_mle.h` | `tetra_mle_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `configure`, `push_event`, `poll_action`, `get_status` |
| MM | `tetra_mm.h` | `tetra_ms_mm_` | `abi_version`, `storage_size`, `storage_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| CMCE | `tetra_cmce.h` | `tetra_cmce_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| SDS | `tetra_sds.h` | `tetra_sds_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| MAC reassembly | `tetra_reassembly.h` | `tetra_ms_reassembly_` | `size`, `init`, `reset`, `push`, `poll`; shares the common ABI version |

Headers live under `crates/tetra-ms-ffi/include`. The original basic-link ABI is
unchanged. For example, its complete export surface remains:

```text
tetra_ms_abi_version
tetra_ms_core_size
tetra_ms_core_align
tetra_ms_init
tetra_ms_reset
tetra_ms_push_event
tetra_ms_poll_action
tetra_ms_get_status
```

The size/alignment queries describe required opaque storage, not Rust layout.
Lengths, enums and booleans are fixed-width integers. Structures have size and
version fields; C and Rust assert their sizes, alignment and PDU offsets.
No `usize`, Rust references, Rust enums, trait objects, callbacks or ownership
of an allocated Rust object cross the boundary.

For example, with a firmware-owned, serialized static object:

```c
#include "tetra_ms.h"
static struct tetra_ms_storage llc_storage;

int32_t start_llc(uint32_t own_issi) {
    struct tetra_ms_config config = {
        sizeof(config), TETRA_MS_ABI_VERSION, own_issi
    };
    return tetra_ms_init(&llc_storage, sizeof(llc_storage), &config);
}
```

The complete linked C caller is
[`tests/smoke.c`](../crates/tetra-ms-ffi/tests/smoke.c); it submits an SDU,
checks exact encoded bytes, acknowledges its request, polls status and resets.
Use static/task-owned storage instead of the smoke test's automatic storage on
a small firmware stack. Zero-initialize event/output structures, set their
size/version, check each result and poll until `TETRA_MS_EMPTY`.

The independent MAC-reassembly ABI is declared in
[`tetra_reassembly.h`](../crates/tetra-ms-ffi/include/tetra_reassembly.h), with
its own 2,304-byte aligned storage and fixed 548-byte event/action structures.
It exposes size, init, reset, push and poll operations. Replacement is reported
as a positive result, timeout as a bitmask, and continuation overflow as
`TETRA_MS_FRAGMENT_OVERFLOW_ABORTED`; these are never hidden drops. Polling a
completed chain consumes it only after output-header validation. Only the
output size/version header must be initialized before polling; unused payload
bytes are not read. A real linked C smoke test verifies exact reassembled golden
bytes, error non-consumption, empty-output preservation, overflow and reset.

Storage must remain live and correctly aligned for every call. Other pointers
must identify correctly sized live objects and must not overlap storage or each
other. Null, alignment, length, discriminant, structure-version and overlap
checks do not validate arbitrary dangling pointers. The private magic marker
is a diagnostic, not permission to use uninitialized storage without `init`.
An invalid output argument does not consume an action; `EMPTY` leaves it
untouched.

The `std` build catches unwinding at the ABI boundary and reports `PANIC`; a
panicked instance must not be used for further protocol work. Embedded builds
do not unwind. Firmware supplies its panic policy. The optional `standalone`
feature supplies a halting panic handler for a freestanding archive and must
not be combined with a firmware's own panic handler:

```text
cargo rustc -p tetra-ms-ffi --lib --release --target thumbv8m.main-none-eabihf --features standalone --crate-type staticlib -- -C panic=abort
```

## Memory and performance

Measurements carried from the extraction snapshot used Rust 1.95.0 / LLVM 22
on ARM64 Windows and compiled for Cortex-M33.
The target is `thumbv8m.main-none-eabihf`. Compile-time assertions enforce engine/queue/type
ceilings. Rust layout is private and is not an ABI promise.

| Object | Cortex-M33 bytes | Ceiling |
| --- | ---: | ---: |
| `LlcCore` (all queues included) | 14,916 | 16,384 |
| `LlcEvent` | 28 | 64 |
| `LlcAction` | 368 | 384 |
| `LlcStatus` | 16 | 16 |
| Pending uplink queue | 3,012 | 3,072 |
| Unacknowledged uplink queue | 2,852 | 3,072 |
| Action queue | 8,836 | 9,216 |
| ACK queue / tracked links | 100 each | 128 each |
| `PackedBits` / packed bytes | 340 / 336 | fixed |
| C storage, alignment | 16,384 / 4 | fixed ABI v1 |
| Basic-link C config / event / action / status | 12 / 388 / 392 / 24 | fixed ABI v1 |
| Reassembly C storage / event / action | 2,304 / 548 / 548 | fixed ABI v1 |
| Embedded core heap | 0 | 0 |
| Random-access state | 12 | 32 |
| Access-parameter store | 104 | 256 |
| Four MAC fragment chains, including metadata | 2,208 | 2,304 |
| One `FragmentBits` object | 516 | fixed |
| MM core | 4,520 | checked by MM module |
| MLE core / action / state | 6,412 / 536 / 68 | checked by MLE module |
| MAC core / snapshot | 14,292 / 6 | bounded module storage |
| CMCE core / SDS core | 19,548 / 3,464 | bounded module storage |
| Coding decoder workspace | 2,880, alignment 4 | caller-owned |
| SB1 traceback workspace | 1,728, alignment 4 | caller-owned |

The default native component-object sum is **65,476 bytes**, available as
`tetra_ms_core::DEFAULT_COMPONENT_STATE_BYTES`. A 64 KiB aggregate ceiling and
the exact 32-bit sum are compile-time checked. This is not a single serialized
object or a complete firmware RAM budget: add inter-object alignment, caller
configuration, actions in flight, coding workspace, audio/DMA buffers and task
stacks. No opaque application payload or audio FIFO belongs in these objects.

Complete C ABI layout, in bytes:

| Component | Opaque storage / alignment | Config | Event | Action | Status |
| --- | ---: | ---: | ---: | ---: | ---: |
| LLC | 16,384 / 4 | 12 | 388 | 392 | 24 |
| MAC/RA | 32,768 / 4 | 1,072 | 1,712 | 1,288 | 3,444 |
| MLE | 8,192 / 4 | 272 | 1,584 | 880 | 112 |
| MM | 4,608 / 4 | 272 | 884 | 880 | 324 |
| CMCE | 20,480 / 8 | 16 | 392 | 444 | 1,180 |
| SDS | 4,096 / 8 | 8 | 372 | 372 | 1,068 |
| Reassembly | 2,304 / 4 | N/A | 548 | 548 | N/A |

All seven opaque storage budgets total **88,832 bytes**; MAC includes RA and its
access-parameter state. Event/output objects are additional caller-owned memory.
The MAC entropy-query result is 20 bytes. On a constrained firmware task, keep
large input/output/status objects in task-owned/static storage rather than
adding them to a small stack. Each ABI enforces its complete storage budget
for overlap checks, not just the current private Rust object size.

The 64-bit ARM64 host has the same core/action/status sizes, with a 48-byte
borrowed Rust event. The x86-64 Windows build also passes the type assertions.

There is no persistent parser scratch workspace. Encoded PDU temporaries and
queue element movement use bounded stack storage. Release Thumb assembly
inspection found a roughly 26 KB initializer temporary before introducing
fieldwise initialization. Afterwards `tetra_ms_init` is a leaf with 16 bytes of
saved registers and no local stack allocation. Reset clears queues in place.
The fieldwise initializer's unsafe invariant is that every field and every
queue's `Option` slot is initialized before exposing a reference.

Observed release frames, including saved registers, include approximately
1,528 bytes for `LlcCore::handle`, 440 for its action poller and 416 for its
largest queue removal. MAC C submission's helper uses 3,056 bytes excluding
callees; MAC init/reset wrappers use 40, shared initialization 144 and native
initialization 24 bytes. Reassembly C polling uses about 640 bytes and in-place
initialization has no local stack allocation.

MM parsing uses validated borrowed group views rather than repeated large
decoded values: the measured native path totals 1,536 bytes, or 2,088 including
its C entry. SDS preflights borrowed input and encodes directly into final
actions; its dispatcher uses 208 bytes, normal emitters at most 408 and cold
rollback 424. Neither optimization adds persistent scratch or changes ABI.

A conservative full-protocol/C-boundary working estimate is **16 KiB stack** in
addition to caller-owned event/output objects and static state. This is **not a
proven worst-case bound** or hardware watermark. Include runtime helpers,
platform execution and interrupt nesting in the board's budget; remeasure for
different compiler/optimization settings. Do not put 32 KiB opaque objects or
large C status snapshots on a small firmware task stack.

Coding scratch is caller-owned rather than nested automatic arrays. Initialize
`tetra_phy_core::errorcontrol::Workspace` and
`tetra_phy_core::viterbi::Sb1Workspace` in static storage or through their
in-place `initialize` functions. Desktop adapters own boxed initialized storage
outside the portable crate. Checked Viterbi operations convert hard bits during
metric accumulation without an intermediate soft-bit array.

After this change, observed release Cortex-M33 own frames (including saved
registers) were 192 bytes for control-channel decoding, 184 for speech decoding
and 576 for a checked standalone SB1 probe. In-place workspace initializers used
eight bytes each. Known nested speech frames totaled 520 bytes, excluding
runtime helpers/callers/interrupts. These compiler measurements do not replace
hardware stack-watermark validation.

All seven component C smoke callers also link together against the ARM archive
with Clang/LLD, without libc or OS libraries: **148,266 bytes text, zero data,
48,496 bytes BSS**, with no undefined symbols. This is link validation, not
executed or bootable firmware. Some fixture objects are automatic rather than
static, so BSS is not the total RAM budget. The archive's compiler builtins and
debug/object data are not the linked flash footprint.

### Packed-bit measurements

The generic implementation reads byte chunks into `u32`, has an inlined
single-bit path and retains a bit-at-a-time reference implementation.
Exhaustive offset/width tests verify exact equivalence, including malformed
ranges, partial bytes, mutable windows and word byte order.

One release run, one million reads per width on the ARM64 Windows host:

| Width | Reference ms | Byte chunks ms |
| --- | ---: | ---: |
| 1 | 1.5082 | 1.2824 |
| 8 | 5.2867 | 3.1369 |
| 16 | 11.1736 | 4.7659 |
| 24 | 15.7763 | 6.2995 |
| 32 | 21.3650 | 8.0228 |

```text
cargo run -p tetra-ms-types --features std --release --example bit_access --quiet
```

These are microbenchmarks, not end-to-end throughput or MCU cycle counts.
No architecture-specific intrinsics or unmeasured `cfg(target_pointer_width)`
optimization has been added. Actual Cortex-M33 cycle measurements and
architecture-specific optimization remain outstanding.

## Behavior and validation

Characterization was added before switching the desktop LLC path. Golden
vectors cover registration (`0010011000`), detach (`00010`), group attach,
LLC sequence changes, stop-and-wait, early ACK, piggyback, automatic ACK and
retransmission. Existing behavior is preserved even where surprising: duplicate
downlink data is delivered again, and an outbound FCS flag does not append an
FCS. No duplicate-suppression protocol change was introduced during extraction.

The FCS algorithm is shared, including short-input initialization. Empty FCS
computation now returns the defined all-ones initial value instead of shifting
by 32 and panicking. Receiving a truncated FCS remains an explicit error.

Validation commands (desktop tests require native SoapySDR linkage, not a radio):

The MS-only import passed **677 tests**, with **five pre-existing ignored
tests** unchanged. Both C ABI smoke modes passed 36 tests each. Strict portable
Clippy, whole-workspace formatting, M33 no-default builds and embedded example,
explicit x86-64 compilation, all 14 freestanding C11/C++11 header checks, and
the seven-component no-libc link also passed. The linked text/data/BSS sizes
match the measurements above. The real executable's offline control test,
non-MS role-rejection test, no-tick router regression and read-only Python
reference-server loopback were exercised without a radio. CI is configured
for MS-only checks and read-only build artifacts, not automatic releases.

```text
cargo test --workspace --no-fail-fast --quiet
cargo test -p tetra-ms-types -p tetra-ms-core --features std --quiet
cargo test -p tetra-ms-ffi --features c-smoke --quiet
cargo test -p tetra-ms-ffi --features std,c-smoke --quiet
cargo clippy -p tetra-ms-types -p tetra-ms-core -p tetra-ms-ffi -p tetra-phy-core --all-targets --all-features -- -D warnings
cargo build -p tetra-ms-core -p tetra-ms-ffi -p tetra-phy-core --no-default-features
cargo build -p tetra-ms-core -p tetra-ms-ffi -p tetra-phy-core --no-default-features --target thumbv8m.main-none-eabihf
cargo build -p tetra-ms-core -p tetra-ms-ffi -p tetra-phy-core --no-default-features --target x86_64-pc-windows-msvc
cargo build -p tetra-ms-core --no-default-features --example embedded --target thumbv8m.main-none-eabihf
cargo rustc -p tetra-ms-ffi --lib --release --target thumbv8m.main-none-eabihf --features standalone --crate-type staticlib -- -C panic=abort
cargo fmt --all -- --check
cargo test -p magentastation --test offline_ms --quiet
cargo test -p tetra-entities --test test_offline_router --quiet
cargo run -p magentastation --quiet -- --help
git diff --check
```

C11 and C++11 layouts also compile freestanding for Cortex-M33:

```text
clang --target=arm-none-eabi -mcpu=cortex-m33 -mthumb -ffreestanding -std=c11 -fsyntax-only -Werror -Icrates/tetra-ms-ffi/include crates/tetra-ms-ffi/tests/smoke.c
clang --target=arm-none-eabi -mcpu=cortex-m33 -mthumb -ffreestanding -x c++ -std=c++11 -fsyntax-only -Werror -Icrates/tetra-ms-ffi/include crates/tetra-ms-ffi/tests/smoke.c
```

The optional embedded link check after building the archive is:

```text
clang --target=arm-none-eabihf -mcpu=cortex-m33 -mthumb -mfloat-abi=hard -mfpu=fpv5-sp-d16 -Os -ffunction-sections -fdata-sections -ffreestanding -std=c11 -Icrates/tetra-ms-ffi/include -nostdlib -fuse-ld=lld "-Wl,-e,main" "-Wl,--gc-sections" crates/tetra-ms-ffi/tests/all_smoke_main.c crates/tetra-ms-ffi/tests/*smoke.c target/thumbv8m.main-none-eabihf/release/libtetra_ms_ffi.a -o portable-ms-smoke.elf
llvm-size portable-ms-smoke.elf
llvm-nm --undefined-only portable-ms-smoke.elf
```

Use native path separators and expand the source glob explicitly on Windows.
For example, pass `@(Get-ChildItem crates\tetra-ms-ffi\tests -Filter '*smoke.c' | ForEach-Object FullName)`
as the smoke-source argument list. The commands in the portable CI workflow
cover host tests, strict lint, embedded Rust/archive builds and C/C++ layouts;
remote CI has not been run as part of this local work.

Existing control/telemetry codecs and integration suites pass. The offline MS
process is also exercised through its real loopback control WebSocket: schema
discovery, runtime state, configuration readback and repeated requests all
succeed without a recovered TDMA clock. The test uses binary WebSocket frames
carrying JSON, matching the existing transport, and cleans up its child process.
Windows process tests require the configured SoapySDR DLL directory on PATH
because the executable is linked against it; no device is opened with backend
None. RF operation was not exercised and no hardware was flashed.

For offline process validation, use `stack_mode = "Ms"` and
`[phy_io] backend = "None"`, omit `[phy_io.soapysdr]`, and configure the existing
control client with `[command]` (not `[control]`). Native DLL linkage still
applies, but no SDR constructor is executed. CI runs both the live-process and
no-tick router tests with this explicit backend.

## Historical extraction review

Before this MS-only import, the extraction snapshot recorded a Claude Opus 5.5
review of the new crates, desktop adapters,
all seven C headers against Rust field/discriminant/error definitions, CI and
reachable production call paths. It found one medium-severity correctness
regression, not a security vulnerability: an ACK arriving after MAC completion
but before LLC end-of-tick propagation could strand the caller's reporter as
Pending and falsely time out an acknowledged detach.

The regression was reproduced by a failing test, then fixed by propagating an
already-observed MAC Transmitted state before acknowledging the caller. A MAC
that is genuinely still Pending retains the original early-ACK behavior.
Claude verified the fix and the five adapter characterization tests, then
completed the remaining production-adapter/header review with no additional
confirmed bugs or security vulnerabilities in that extraction.
This is not exhaustive proof or fuzz/hardware validation; not every legacy
test body was reviewed line by line.

The reviewer also noted unchanged legacy behaviors, deliberately not redesigned
in this extraction: repeated U-DISCONNECT after a call timer expires until call
removal; ineffective SDS group-delivery detection from the MAC address class;
an existing class-of-usage mapping discrepancy; and unspecified break-time
arbitration order. These are compatibility observations, not newly introduced
regressions or newly claimed protocol support.

## Preserved unsupported scope and validation limits

This extracts supported BlueStation procedures; it does not invent previously
unsupported security/energy-saving procedures, internal MLE/management
primitives, network-initiated group attachment, encrypted-fragment handling or
SNDCP behavior. Unsupported inputs remain explicit errors or unchanged desktop
diagnostics. Physical modulation/sample processing, drivers, real-time sample
movement and vocoding are platform responsibilities; the portable stack can
consume already-decoded logical-channel data.

The public Rust boundary follows the existing protocol-component seams rather
than hiding them behind a second facade/state machine. Cross-layer action
routing is demonstrated by `tests/portable_registration.rs`; platform adapters
execute typed results and retain their own metadata. There is one supported MS
protocol implementation per layer, shared by desktop and firmware callers.

No actual radio was accessed or programmed. SoapySDR remains the RF backend.
Explicit `PhyBackend::None` now supports offline MS management with a paced
platform loop and no recovered clock: it never fabricates protocol ticks or
radio completion. A live loopback control test verifies this process boundary.
Live RF behavior and board timing are still unverified. An embedded platform must supply its own HAL,
transport, audio, scheduling and panic policy and validate stack watermarks and
cycle costs on hardware.
