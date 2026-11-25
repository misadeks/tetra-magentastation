# Portable MS libraries

The portable libraries provide allocator-free `no_std` protocol components
and logical-channel coding. The caller supplies time, entropy, transport
completion, configuration and action routing.

| Crate | Contents |
| --- | --- |
| `tetra-ms-types` | Addresses, TDMA arithmetic, bounded packed bits, carrier/duplex arithmetic |
| `tetra-ms-core` | LLC/FCS, MAC/random access/reassembly, MM, MLE, CMCE/floor control and SDS |
| `tetra-phy-core` | CRC, scrambling, interleaving, convolutional/Viterbi, RM coding and traffic reorder |
| `tetra-ms-ffi` | Optional C ABI for seven independently owned components |

Default portable builds use neither `std` nor `alloc`, sockets, filesystem,
audio, a logger, locks or an OS scheduler. Optional `desktop-codecs` on types
enables serde/bitcode representations; `std` enables host conveniences/examples.
Desktop adapters own SAP/BitBuffer conversion, logging, RNG, reporter sidecars,
speech FIFOs, TOML and platform I/O. See the [source map](MS_MODE.md#source-map).

## Dependencies

```text
magentastation -> tetra-entities -> tetra-ms-core -> tetra-ms-types
                   |              tetra-ms-ffi ------^
                   +-> tetra-phy-core
                   +-> tetra-pdus -> tetra-core, tetra-saps, tetra-config
                   +-> tetra-core, tetra-saps, tetra-config

tetra-saps   -> tetra-core
tetra-config -> tetra-core
tetra-core   -> tetra-ms-types [desktop-codecs]
```

`tetra-ms-ffi` is optional: Rust callers can depend directly on the core.
`tetra-phy-core` has no runtime dependencies.

## Rust components

| Component | Caller-driven boundary |
| --- | --- |
| `llc::LlcCore` | `new`/`init_in_place`, `reset`, `handle(now, event)`, `poll_action`, `status` |
| `mac::MsMac` | `init_in_place`/`reset`, `submit_sdu`, `tick`, `resource_event`, `aach`, `allocate`, `grant_fragment`, `reserved`, `traffic`, `snapshot` |
| `random_access::MsRandomAccess` | `initiate`, `entropy_needed`, `poll_downlink_slot`, `on_mac_resource`, `cancel`, `status` |
| `reassembly::MsReassembly` | `init_in_place`/`reset`, `first`, `next`, `age`, `completed`, `release`, `status` |
| `mle::MleCore` | Initialization, `handle(config, event)`, `pop_action`, typed state |
| `mm::MmCore` | Initialization, `handle(event)`, `poll_action`, registration/service state |
| `cmce::CmceCore` | Initialization, `push_event(now, event)`, `pop_action`, `status`, `receive_traffic`, `transmit_target` |
| `sds::SdsCore` | Initialization, `push_event`, `pop_action`, `status`, `cancel` |
| `tetra_phy_core` | Checked control/speech encode/decode and Viterbi operations with caller-owned workspaces |

### Events, actions and ownership

Call the component with typed input, drain its actions, and execute platform
work after the call returns. Input `BitView`s borrow bytes only for the call;
actions own bounded packed buffers. Views are MSB-first with checked fixed-width
offsets, not host-endian casts.

Supply recovered, ordered TDMA time. There is no implicit clock or sleep.
MM ticks carry elapsed slots and detach disposition. MAC entropy requests
describe the required draw; the adapter supplies `RaChoices`. No RNG callback
enters the core. MLE policy slices are borrowed only during `handle`;
scan/survey carrier lists are bounded snapshots.

LLC inputs include send, receive, transmission feedback and end-of-tick.
Actions include transmit, deliver and completed. Acknowledged commands retain
their key until the completed action is consumed. Every MAC attempt has a
distinct `Receipt { epoch, sequence }`: feed back the actual attempt, not an
old receipt or an enqueue event.

`Transmitted` or `Rejected` feedback starts T251 (16 slots); N252 allows three
retransmissions. `Deferred` keeps ownership without starting the timer: the
transport must later provide another disposition for that attempt. An early
network ACK can unblock stop-and-wait before local bookkeeping catches up;
it must not invent a local transmitted receipt.

MAC uses numeric `u32` receipt handles. Desktop atomic/heap reporters are
sidecars, not protocol payloads. Audio FIFOs also remain outside CMCE calls.
Reset cancels pending work, clears state in place and advances the receipt
epoch; the adapter must notify its callers and discard old transport work.
Do not initialize a live object again instead of resetting it.

### Bounds and errors

| Resource | Bound |
| --- | ---: |
| LLC acknowledged / unacknowledged uplinks | 8 / 8 |
| LLC scheduled ACKs / tracked SSI sequences | 8 / 8 |
| LLC actions | 24 |
| Encoded LLC PDU | 2688 bits / 336 bytes |
| MAC queued transfers / reserved slots | 16 / 6 |
| MAC audio frames / stolen blocks / filter groups | 4 / 16 / 256 |
| Downlink fragment chains | 4, each 4096 bits |
| MM groups / actions | 63 / 8 |
| MLE candidates / identities / actions | 256 / 64 / 8 |
| CMCE calls / originations / actions | 16 / 16 / 40 |
| SDS actions / tracked references | 4 / 256 |

LLC PDU capacity includes the header. Maximum SDUs are 2683 bits for BL-DATA,
2682 for piggybacked BL-ADATA and 2684 for BL-UDATA. Capacity and queue errors
are explicit; drain actions before retrying. MAC's drop-oldest queue policy
returns the displaced handle so the adapter can notify its owner.

Each reassembly chain expires strictly more than 40 slots after its last
fragment. An oversized first fragment is rejected; continuation overflow aborts
that chain. Completed data remains borrowed until `release`.

Invalid time/configuration, malformed/truncated bits, invalid FCS, unsupported
PDUs, capacity, queue exhaustion and stale receipts have distinct errors.
Check them at every boundary. Unmatched network ACKs are counted in status;
stale local transport receipts are errors.

LLC still delivers duplicate downlink data, and an outbound FCS flag does not
append an FCS. Empty FCS calculation returns the all-ones initial value;
truncated received FCS is an error. These are behavior details callers should
not replace with assumptions.

### Desktop and embedded Rust callers

The desktop LLC adapter is
[`llc/ms_adapter.rs`](../crates/tetra-entities/src/llc/ms_adapter.rs).
It preserves SAP metadata and bridges transport receipts without owning retry
or sequence decisions. A minimal host caller is:

```sh
cargo run -p tetra-ms-core --features std --example desktop
```

The [embedded example](../crates/tetra-ms-core/examples/embedded.rs) is a
`#![no_std]` library with initialization, input/feedback, ticks and action
polling. It does not impose a board runtime or panic handler:

```sh
rustup target add thumbv8m.main-none-eabihf
cargo build -p tetra-ms-core --no-default-features --example embedded --target thumbv8m.main-none-eabihf
```

Give `init_in_place` exclusive, aligned `MaybeUninit` storage in task-owned or
static memory. Avoid constructing a large temporary with `new()` on a small
task stack and moving it afterward. Serialize access externally. Input/output
objects, actions in flight and coding scratch also need memory budgets.

Cross-layer examples:
[acquisition/registration](../crates/tetra-ms-core/tests/portable_registration.rs),
[fragmented receive and ACK](../crates/tetra-ms-core/tests/portable_receive.rs),
and [MLE details](PORTABLE_MLE.md).

## Versioned C ABI

Every component uses ABI version 1. Headers document event/action integers,
bounds and errors; the caller routes actions between components.

| Component | Header | Export prefix | Operations after prefix |
| --- | --- | --- | --- |
| LLC | `tetra_ms.h` | `tetra_ms_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| MAC/RA | `tetra_mac.h` | `tetra_ms_mac_` | `abi_version`, `storage_size`, `storage_align`, `init`, `reset`, `push_event`, `entropy_needed`, `poll_action`, `get_status` |
| MLE | `tetra_mle.h` | `tetra_mle_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `configure`, `push_event`, `poll_action`, `get_status` |
| MM | `tetra_mm.h` | `tetra_ms_mm_` | `abi_version`, `storage_size`, `storage_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| CMCE | `tetra_cmce.h` | `tetra_cmce_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| SDS | `tetra_sds.h` | `tetra_sds_` | `abi_version`, `core_size`, `core_align`, `init`, `reset`, `push_event`, `poll_action`, `get_status` |
| Reassembly | `tetra_reassembly.h` | `tetra_ms_reassembly_` | `size`, `init`, `reset`, `push`, `poll`; shares the common ABI version |

Headers are in [`crates/tetra-ms-ffi/include`](../crates/tetra-ms-ffi/include).
Size/alignment queries describe required opaque storage, not Rust layout.
Lengths, enums and booleans are fixed-width integers. Structures carry
size/version headers; no Rust references, `usize`, callbacks or allocation
ownership cross the boundary.

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

See the linked [C caller](../crates/tetra-ms-ffi/tests/smoke.c). Initialize
event/output headers, check each result, and poll until `TETRA_MS_EMPTY`.
An invalid output header does not consume an action; `EMPTY` leaves output
untouched. Reassembly polling consumes a completed chain only after validation.

Storage must stay live and aligned. Other pointers must reference correctly
sized live objects and not overlap storage or each other. Null/alignment/range
checks do not validate arbitrary dangling pointers. The private magic marker
does not make uninitialized storage valid.

`std` builds catch unwinding at the ABI boundary and report `PANIC`; do not
reuse a panicked instance. Embedded builds use the firmware's panic policy.
The optional `standalone` feature supplies a halting handler for a freestanding
archive; do not combine it with the firmware's own handler:

```sh
cargo rustc -p tetra-ms-ffi --lib --release --target thumbv8m.main-none-eabihf --features standalone --crate-type staticlib -- -C panic=abort
```

### C ABI storage

Sizes in bytes:

| Component | Opaque storage / alignment | Config | Event | Action | Status |
| --- | ---: | ---: | ---: | ---: | ---: |
| LLC | 16384 / 4 | 12 | 388 | 392 | 24 |
| MAC/RA | 32768 / 4 | 1072 | 1712 | 1288 | 3444 |
| MLE | 8192 / 4 | 272 | 1584 | 880 | 112 |
| MM | 4608 / 4 | 272 | 884 | 880 | 324 |
| CMCE | 20480 / 8 | 16 | 392 | 444 | 1180 |
| SDS | 4096 / 8 | 8 | 372 | 372 | 1068 |
| Reassembly | 2304 / 4 | N/A | 548 | 548 | N/A |

Opaque storage totals **88832 bytes**, excluding caller event/output objects.
MAC includes random access/access parameters. The entropy-query result is
20 bytes. Overlap checks enforce the whole storage budget, not only the current
private Rust size.

## Native memory and coding scratch

The following measurements use Rust 1.95.0 / LLVM 22 for Cortex-M33
`thumbv8m.main-none-eabihf`. Compile-time assertions enforce bounds; Rust
layout is private, not an ABI.

| Object | Bytes |
| --- | ---: |
| LLC core, including queues | 14916 |
| LLC event / action / status | 28 / 368 / 16 |
| LLC pending / unacknowledged / action queues | 3012 / 2852 / 8836 |
| Packed bits / fragment bits | 340 / 516 |
| Random access / access parameters | 12 / 104 |
| Four fragment chains with metadata | 2208 |
| MM core | 4520 |
| MLE core / action / state | 6412 / 536 / 68 |
| MAC core / snapshot | 14292 / 6 |
| CMCE core / SDS core | 19548 / 3464 |
| Coding decoder workspace | 2880, alignment 4 |
| SB1 traceback workspace | 1728, alignment 4 |

`tetra_ms_core::DEFAULT_COMPONENT_STATE_BYTES` is **65476 bytes**. Its 32-bit
sum and 64 KiB aggregate ceiling are compile-time checked. This excludes
inter-object alignment, caller config/events/actions, coding workspace,
audio/DMA buffers and task stacks.

Initialize `tetra_phy_core::errorcontrol::Workspace` and
`tetra_phy_core::viterbi::Sb1Workspace` in caller-owned storage with their
in-place `initialize` functions. Desktop adapters own boxed workspaces outside
the portable crate. Viterbi converts hard bits during metric accumulation
without a separate soft-bit array.

A **16 KiB stack** is a working full-protocol/C-boundary estimate, not a proven
worst case or hardware watermark. Include runtime helpers, callers and nested
interrupts, and remeasure for your compiler/settings. Large C storage/status
objects should not be automatic variables on a small task stack.

The all-component freestanding smoke link with Clang/LLD has measured
148266 bytes text, zero data and 48496 bytes BSS with no undefined symbols.
It is a link fixture, not bootable firmware; some caller objects are automatic,
so BSS is not its whole RAM budget.

The packed-bit implementation has a bit-at-a-time reference and byte-chunk
implementation. Run its host microbenchmark with:

```sh
cargo run -p tetra-ms-types --features std --release --example bit_access --quiet
```

Host timings are not MCU cycle counts or end-to-end throughput.

## Build and test commands

Portable tests and checks:

```sh
cargo test -p tetra-ms-types -p tetra-ms-core --features std
cargo test -p tetra-phy-core
cargo test -p tetra-ms-ffi --features c-smoke
cargo test -p tetra-ms-ffi --features std,c-smoke
cargo clippy -p tetra-ms-types -p tetra-ms-core -p tetra-ms-ffi -p tetra-phy-core --all-targets --all-features -- -D warnings
cargo build -p tetra-ms-core -p tetra-ms-ffi -p tetra-phy-core --no-default-features --target thumbv8m.main-none-eabihf
cargo fmt --all -- --check
```

Desktop tests require native SoapySDR linkage, but the offline fixtures use no
device. Windows child-process tests also need its DLL directory on `PATH`:

```sh
cargo test --workspace --locked
cargo test -p magentastation --test offline_ms
cargo test -p tetra-entities --test test_offline_router
```

After building the standalone archive, a freestanding C link can be checked
with Clang, LLD and LLVM tools:

```sh
clang --target=arm-none-eabihf -mcpu=cortex-m33 -mthumb -mfloat-abi=hard -mfpu=fpv5-sp-d16 -Os -ffunction-sections -fdata-sections -ffreestanding -std=c11 -Icrates/tetra-ms-ffi/include -nostdlib -fuse-ld=lld -Wl,-e,main -Wl,--gc-sections crates/tetra-ms-ffi/tests/all_smoke_main.c crates/tetra-ms-ffi/tests/*smoke.c target/thumbv8m.main-none-eabihf/release/libtetra_ms_ffi.a -o portable-ms-smoke.elf
llvm-size portable-ms-smoke.elf
llvm-nm --undefined-only portable-ms-smoke.elf
```

The [portable workflow](../.github/workflows/portable-ms.yml) includes C11 and
C++11 header-layout checks for every component. On Windows, use native path
separators and explicitly expand the smoke-source glob. Additional targets,
such as `x86_64-pc-windows-msvc`, need their Rust target and linker installed.

Firmware must supply HAL/sample transport, scheduling, audio and panic policy.
The portable components consume logical-channel data; building/linking them
does not establish RF interoperability or target stack margins. See
[capabilities and limits](MS_MODE.md#capabilities-and-limits).
