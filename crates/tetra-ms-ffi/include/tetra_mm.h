#ifndef TETRA_MM_H
#define TETRA_MM_H
#include <stdint.h>
#include <stddef.h>

#define TETRA_MS_MM_ABI_VERSION 1u
#define TETRA_MS_MM_STORAGE_SIZE 4608u
#define TETRA_MS_MM_STORAGE_ALIGN 4u
#define TETRA_MS_MM_MAX_GROUPS 63u
#define TETRA_MS_MM_PDU_BYTES 336u
#define TETRA_MS_MM_MAX_PDU_BITS 2688u
#define TETRA_MS_MM_T351_SLOTS 706u
#define TETRA_MS_MM_T353_SLOTS 706u
#define TETRA_MS_MM_DETACH_DRAIN_SLOTS 144u

#define TETRA_MS_MM_OK 0
#define TETRA_MS_MM_EMPTY 1
#define TETRA_MS_MM_ARGUMENT (-1)
#define TETRA_MS_MM_ABI_MISMATCH (-2)
#define TETRA_MS_MM_UNINITIALIZED (-3)
#define TETRA_MS_MM_INVALID_IDENTITY (-4)
#define TETRA_MS_MM_INVALID_PDU (-6)
#define TETRA_MS_MM_TRUNCATED (-7)
#define TETRA_MS_MM_UNSUPPORTED (-9)
#define TETRA_MS_MM_CAPACITY (-10)
#define TETRA_MS_MM_QUEUE_FULL (-11)
#define TETRA_MS_MM_PANIC (-16)
#define TETRA_MS_MM_INVALID_WIDTH (-101)
#define TETRA_MS_MM_INVALID_VALUE (-102)
#define TETRA_MS_MM_GROUP_CAPACITY (-103)
#define TETRA_MS_MM_NOT_REGISTERED (-104)
#define TETRA_MS_MM_GROUP_OPERATION_PENDING (-105)
#define TETRA_MS_MM_EMPTY_GROUPS (-106)
#define TETRA_MS_MM_REGISTRATION_IN_PROGRESS (-107)
#define TETRA_MS_MM_ALREADY_REGISTERED (-108)
#define TETRA_MS_MM_DETACHING (-109)
#define TETRA_MS_MM_LEFT_SYSTEM (-110)

#define TETRA_MS_MM_ACTIVATE 1u
#define TETRA_MS_MM_REGISTER 2u
#define TETRA_MS_MM_DOWNLINK 3u
#define TETRA_MS_MM_LINK_BREAK 4u
#define TETRA_MS_MM_LINK_REOPEN 5u
#define TETRA_MS_MM_GROUPS 6u
#define TETRA_MS_MM_DEREGISTER 7u
#define TETRA_MS_MM_TICK 8u
#define TETRA_MS_MM_SET_GROUPS 9u
#define TETRA_MS_MM_SET_IDENTITY 10u

#define TETRA_MS_MM_REGISTRATION_REQUIRED 1u
#define TETRA_MS_MM_SYSTEM_WIDE_SERVICES 2u
#define TETRA_MS_MM_FORCE_REGISTER 4u
#define TETRA_MS_MM_DETACH_ALL 8u
#define TETRA_MS_MM_REPORT_REQUESTED 16u
#define TETRA_MS_MM_ATTACH 0u
#define TETRA_MS_MM_DETACH 1u

#define TETRA_MS_MM_TRANSFER_PENDING 0u
#define TETRA_MS_MM_TRANSFER_TRANSMITTED 1u
#define TETRA_MS_MM_TRANSFER_ACKNOWLEDGED 2u
#define TETRA_MS_MM_TRANSFER_DISCARDED 3u
#define TETRA_MS_MM_TRANSFER_LOST 4u

#define TETRA_MS_MM_SEND 1u
#define TETRA_MS_MM_SERVICE 2u
#define TETRA_MS_MM_REGISTRATION_SUCCESS 3u
#define TETRA_MS_MM_REGISTRATION_FAILURE 4u
#define TETRA_MS_MM_IDENTITIES 5u
#define TETRA_MS_MM_GROUP_CONFIRM 6u
#define TETRA_MS_MM_DETACH_REPORT 7u
#define TETRA_MS_MM_DETACH_COMPLETE 8u
#define TETRA_MS_MM_HAS_DETACH_RECEIPT 1u
#define TETRA_MS_MM_ACTION_DETACH_ALL 2u
#define TETRA_MS_MM_ACTION_REPORT_REQUESTED 4u
#define TETRA_MS_MM_TRANSFER_SUCCESSFUL 8u
#define TETRA_MS_MM_HAS_REJECT_CAUSE 16u

#define TETRA_MS_MM_IDLE 0u
#define TETRA_MS_MM_REGISTERING 1u
#define TETRA_MS_MM_REGISTERED 2u
#define TETRA_MS_MM_STATE_DETACHING 3u
#define TETRA_MS_MM_SERVICE_OUT_OF_SERVICE 0u
#define TETRA_MS_MM_SERVICE_WAITING 1u
#define TETRA_MS_MM_SERVICE_IN_SERVICE 2u
#define TETRA_MS_MM_HAS_LEFT_SYSTEM 1u
#define TETRA_MS_MM_TEMPORARY_REGISTRATION 2u
#define TETRA_MS_MM_OUT_OF_SERVICE 4u
#define TETRA_MS_MM_HAS_GROUP_OPERATION 8u
#define TETRA_MS_MM_STATUS_DETACH_ALL 16u
#define TETRA_MS_MM_STATUS_REPORT_REQUESTED 32u
#define TETRA_MS_MM_HAS_CURRENT_CELL 64u
#define TETRA_MS_MM_HAS_REGISTERED_CELL 128u
#define TETRA_MS_MM_HAS_PENDING_CELL 256u

#define TETRA_MS_MM_LU_ROAMING 0u
#define TETRA_MS_MM_LU_MIGRATING 1u
#define TETRA_MS_MM_LU_PERIODIC 2u
#define TETRA_MS_MM_LU_ITSI_ATTACH 3u
#define TETRA_MS_MM_LU_SERVICE_RESTORATION_ROAMING 4u
#define TETRA_MS_MM_LU_SERVICE_RESTORATION_MIGRATING 5u
#define TETRA_MS_MM_LU_DEMAND 6u
#define TETRA_MS_MM_LU_DISABLED_MS 7u

#if defined(__cplusplus)
#define TETRA_MS_MM_ALIGN(n) alignas(n)
#define TETRA_MS_MM_ASSERT(x) static_assert(x, #x)
#define TETRA_MS_MM_ALIGNOF(t) alignof(t)
extern "C" {
#elif defined(_MSC_VER)
#define TETRA_MS_MM_ALIGN(n) __declspec(align(n))
#define TETRA_MS_MM_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MS_MM_ALIGNOF(t) __alignof(t)
#else
#define TETRA_MS_MM_ALIGN(n) _Alignas(n)
#define TETRA_MS_MM_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MS_MM_ALIGNOF(t) _Alignof(t)
#endif

struct tetra_ms_mm_storage { TETRA_MS_MM_ALIGN(4) uint8_t bytes[TETRA_MS_MM_STORAGE_SIZE]; };
struct tetra_ms_mm_group_request { uint32_t gssi; uint8_t operation, value; uint16_t reserved; };
struct tetra_ms_mm_cell { uint16_t mcc, mnc, la, reserved; };
struct tetra_ms_mm_config {
    uint32_t size, abi_version, own_issi;
    uint16_t mcc, mnc, initial_la, group_count;
    uint32_t groups[TETRA_MS_MM_MAX_GROUPS];
};
struct tetra_ms_mm_event {
    uint32_t size, abi_version, kind, handle, elapsed_slots, own_issi;
    uint16_t cell_mcc, cell_mnc, cell_la, group_count;
    uint32_t flags, transfer_state;
    uint16_t pdu_bits, reserved;
    uint8_t pdu[TETRA_MS_MM_PDU_BYTES];
    struct tetra_ms_mm_group_request groups[TETRA_MS_MM_MAX_GROUPS];
};
struct tetra_ms_mm_action {
    uint32_t size, abi_version, kind, handle, flags, service, reject_cause, t353_countdown;
    uint8_t priority, reserved;
    uint16_t pdu_bits, attached_count, detached_count;
    uint8_t pdu[TETRA_MS_MM_PDU_BYTES];
    uint32_t attached[TETRA_MS_MM_MAX_GROUPS], detached[TETRA_MS_MM_MAX_GROUPS];
};
struct tetra_ms_mm_status {
    uint32_t size, abi_version, registration, service, t351_countdown, detach_countdown;
    uint32_t group_handle, t353_countdown, flags;
    uint8_t attempts, system_rejection_count, pending_lu_type, reserved;
    uint16_t serving_la, attached_count, actions, reserved2;
    struct tetra_ms_mm_cell current_cell, registered_cell, pending_cell;
    uint32_t attached[TETRA_MS_MM_MAX_GROUPS];
};

/* The MM instance is independent of basic-link/MLE storage. Query size/alignment
 * or use tetra_ms_mm_storage. No Rust layout crosses this boundary.
 * Do not inspect or modify opaque bytes. Serialize all calls per instance.
 * Pointers must be live, properly aligned,
 * correctly sized and disjoint from the entire queried storage budget.
 * init accepts uninitialized storage; all other operations require successful
 * init. Reset cancels procedures/actions: discard the old external detach
 * receipt too. Invalid config/reset/events preserve the instance and its queue.
 * Zero-initialize inputs, then set size and abi_version. Arrays are inline;
 * inputs are borrowed only during the call. PDU bytes are packed MSB-first.
 * Output needs writable storage and initialized size/abi_version only; unused
 * output bytes are zeroed. EMPTY and errors leave output/action queue untouched.
 * This atomicity promise covers validation/protocol errors, not an internal
 * PANIC: discard/reinitialize the instance after PANIC. std builds catch Rust
 * unwinding; standalone no_std firmware must abort/halt rather than unwind.
 *
 * ACTIVATE: cell_* + REGISTRATION_REQUIRED/SYSTEM_WIDE_SERVICES/FORCE_REGISTER.
 * REGISTER/LINK_BREAK/LINK_REOPEN/DEREGISTER: no payload.
 * DOWNLINK: pdu_bits and pdu, starting at the four-bit MM PDU discriminator.
 * GROUPS: handle, DETACH_ALL/REPORT_REQUESTED, group_count and groups. Attach value
 * is raw class-of-usage 0..7; detach value is raw detachment reason 0..3.
 * TICK: elapsed recovered downlink slots, and current detach transfer_state.
 * Receipt state must describe the detach requested by HAS_DETACH_RECEIPT.
 * Split time batches when that external receipt changes. Drain actions after
 * events; a batch exceeding action capacity returns QUEUE_FULL atomically.
 * SET_GROUPS: replace desired/restored GSSIs using group_count/groups[].gssi;
 * operation/value/reserved must be zero. No on-air transaction is implied.
 * SET_IDENTITY: own_issi, cell_mcc, cell_mnc, cell_la (initial LA); preserves
 * running procedures and groups, exactly like the Rust configure operation.
 * Unknown events/unsupported MM procedures return UNSUPPORTED.
 *
 * SEND is acknowledged clear-mode LMM unitdata, addressed by the configured
 * ISSI; priority and optional detach receipt request are included.
 * IDENTITIES output is the complete sorted unique filter set plus wire-order
 * detached identities. Registration/group confirmations preserve wire-order
 * duplicates. GROUP_CONFIRM flags echo the pending request; t353_countdown is
 * its remaining timer (zero after expiry). Reject causes are raw five-bit values.
 * Status optional cells and group operation are discriminated by HAS_* flags.
 * De-registration is pending exactly when registration == STATE_DETACHING and
 * detach_countdown != 0. Service is available in every status snapshot.
 */
uint32_t tetra_ms_mm_abi_version(void);
uint32_t tetra_ms_mm_storage_size(void);
uint32_t tetra_ms_mm_storage_align(void);
int32_t tetra_ms_mm_init(void *storage, uint32_t storage_size, const struct tetra_ms_mm_config *config);
int32_t tetra_ms_mm_reset(void *storage, const struct tetra_ms_mm_config *config);
int32_t tetra_ms_mm_push_event(void *storage, const struct tetra_ms_mm_event *event);
int32_t tetra_ms_mm_poll_action(void *storage, struct tetra_ms_mm_action *action);
int32_t tetra_ms_mm_get_status(const void *storage, struct tetra_ms_mm_status *status);

TETRA_MS_MM_ASSERT(sizeof(struct tetra_ms_mm_storage) == TETRA_MS_MM_STORAGE_SIZE);
TETRA_MS_MM_ASSERT(TETRA_MS_MM_ALIGNOF(struct tetra_ms_mm_storage) == TETRA_MS_MM_STORAGE_ALIGN);
TETRA_MS_MM_ASSERT(sizeof(struct tetra_ms_mm_config) == 272);
TETRA_MS_MM_ASSERT(sizeof(struct tetra_ms_mm_group_request) == 8);
TETRA_MS_MM_ASSERT(sizeof(struct tetra_ms_mm_cell) == 8);
TETRA_MS_MM_ASSERT(sizeof(struct tetra_ms_mm_event) == 884);
TETRA_MS_MM_ASSERT(sizeof(struct tetra_ms_mm_action) == 880);
TETRA_MS_MM_ASSERT(sizeof(struct tetra_ms_mm_status) == 324);
TETRA_MS_MM_ASSERT(TETRA_MS_MM_ALIGNOF(struct tetra_ms_mm_event) == 4);
TETRA_MS_MM_ASSERT(TETRA_MS_MM_ALIGNOF(struct tetra_ms_mm_action) == 4);
TETRA_MS_MM_ASSERT(offsetof(struct tetra_ms_mm_event, pdu) == 44);
TETRA_MS_MM_ASSERT(offsetof(struct tetra_ms_mm_event, groups) == 380);
TETRA_MS_MM_ASSERT(offsetof(struct tetra_ms_mm_action, pdu) == 40);
TETRA_MS_MM_ASSERT(offsetof(struct tetra_ms_mm_action, attached) == 376);
TETRA_MS_MM_ASSERT(offsetof(struct tetra_ms_mm_status, attached) == 72);

#if defined(__cplusplus)
}
#endif
#endif
