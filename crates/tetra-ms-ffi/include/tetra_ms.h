#ifndef TETRA_MS_H
#define TETRA_MS_H
#include <stdint.h>
#include <stddef.h>

#define TETRA_MS_ABI_VERSION 1u
#define TETRA_MS_STORAGE_SIZE 16384u
#define TETRA_MS_STORAGE_ALIGN 4u
#define TETRA_MS_PDU_BYTES 336u
#define TETRA_MS_MAX_PDU_BITS 2688u

#define TETRA_MS_OK 0
#define TETRA_MS_EMPTY 1
#define TETRA_MS_ARGUMENT (-1)
#define TETRA_MS_ABI_MISMATCH (-2)
#define TETRA_MS_UNINITIALIZED (-3)
#define TETRA_MS_INVALID_IDENTITY (-4)
#define TETRA_MS_INVALID_TIME (-5)
#define TETRA_MS_INVALID_PDU (-6)
#define TETRA_MS_TRUNCATED (-7)
#define TETRA_MS_INVALID_FCS (-8)
#define TETRA_MS_UNSUPPORTED (-9)
#define TETRA_MS_CAPACITY (-10)
#define TETRA_MS_QUEUE_FULL (-11)
#define TETRA_MS_RECEIPT_BUSY (-12)
#define TETRA_MS_STALE_RECEIPT (-13)
#define TETRA_MS_INVALID_TRANSITION (-14)
#define TETRA_MS_GENERATION_EXHAUSTED (-15)
#define TETRA_MS_PANIC (-16)

#define TETRA_MS_SEND 1u
#define TETRA_MS_RECEIVE 2u
#define TETRA_MS_TRANSMISSION 3u
#define TETRA_MS_TICK 4u
#define TETRA_MS_TRANSMIT 1u
#define TETRA_MS_DELIVER 2u
#define TETRA_MS_COMPLETED 3u
#define TETRA_MS_ACKNOWLEDGED 0u
#define TETRA_MS_UNACKNOWLEDGED 1u
#define TETRA_MS_TRANSMITTED 0u
#define TETRA_MS_REJECTED 1u
#define TETRA_MS_DEFERRED 2u
#define TETRA_MS_COMPLETION_ACKNOWLEDGED 0u
#define TETRA_MS_COMPLETION_RETRY_EXHAUSTED 1u
#define TETRA_MS_UNKNOWN 0u
#define TETRA_MS_SSI 1u
#define TETRA_MS_ISSI 2u
#define TETRA_MS_GSSI 3u
#define TETRA_MS_USSI 4u
#define TETRA_MS_SMI 5u
#define TETRA_MS_ESI 6u
#define TETRA_MS_EVENT_LABEL 7u

#if defined(__cplusplus)
#define TETRA_MS_ALIGN(n) alignas(n)
#define TETRA_MS_ASSERT(x) static_assert(x, #x)
#define TETRA_MS_ALIGNOF(t) alignof(t)
extern "C" {
#elif defined(_MSC_VER)
#define TETRA_MS_ALIGN(n) __declspec(align(n))
#define TETRA_MS_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MS_ALIGNOF(t) __alignof(t)
#else
#define TETRA_MS_ALIGN(n) _Alignas(n)
#define TETRA_MS_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MS_ALIGNOF(t) _Alignof(t)
#endif

struct tetra_ms_storage { TETRA_MS_ALIGN(4) uint8_t bytes[TETRA_MS_STORAGE_SIZE]; };
struct tetra_ms_config { uint32_t size, abi_version, own_issi; };
struct tetra_ms_event {
    uint32_t size, abi_version, kind, now, command, ssi, ssi_type, service, fcs;
    uint32_t receipt_epoch, receipt_sequence, feedback;
    uint16_t pdu_bits, reserved;
    uint8_t pdu[TETRA_MS_PDU_BYTES];
};
struct tetra_ms_action {
    uint32_t size, abi_version, kind, command, ssi, ssi_type;
    uint32_t receipt_epoch, receipt_sequence, ack_slot, group_data, service, fcs, completion;
    uint16_t pdu_bits, reserved;
    uint8_t pdu[TETRA_MS_PDU_BYTES];
};
struct tetra_ms_status {
    uint32_t size, abi_version;
    uint16_t pending, unacknowledged, scheduled_acks, actions, links, reserved;
    uint32_t ignored_acks;
};

/* All lengths are fixed-width. Flags are 0/1. Zero-initialize boundary objects,
 * then fill size and abi_version. `now` is the absolute TDMA slot, 0..283115519.
 * The application must serialize calls on each initialized storage object.
 * Pointers must be live, properly aligned, correctly sized and non-overlapping.
 * init accepts uninitialized storage; all other operations require successful
 * init. Do not initialize live storage again: use reset, which cancels pending
 * commands and invalidates all old receipts.
 * Event inputs are copied during the call. Drain actions after every event.
 * poll returns EMPTY without modifying output; other errors consume no action.
 * These entry points expose BASIC LINK ONLY. Other portable components use
 * their own headers and independent caller-owned storage objects.
 */
uint32_t tetra_ms_abi_version(void);
uint32_t tetra_ms_core_size(void);
uint32_t tetra_ms_core_align(void);
int32_t tetra_ms_init(void *storage, uint32_t storage_size, const struct tetra_ms_config *config);
int32_t tetra_ms_reset(void *storage, const struct tetra_ms_config *config);
int32_t tetra_ms_push_event(void *storage, const struct tetra_ms_event *event);
int32_t tetra_ms_poll_action(void *storage, struct tetra_ms_action *action);
int32_t tetra_ms_get_status(const void *storage, struct tetra_ms_status *status);

TETRA_MS_ASSERT(sizeof(struct tetra_ms_storage) == TETRA_MS_STORAGE_SIZE);
TETRA_MS_ASSERT(TETRA_MS_ALIGNOF(struct tetra_ms_storage) == TETRA_MS_STORAGE_ALIGN);
TETRA_MS_ASSERT(sizeof(struct tetra_ms_config) == 12);
TETRA_MS_ASSERT(sizeof(struct tetra_ms_event) == 388);
TETRA_MS_ASSERT(sizeof(struct tetra_ms_action) == 392);
TETRA_MS_ASSERT(sizeof(struct tetra_ms_status) == 24);
TETRA_MS_ASSERT(TETRA_MS_ALIGNOF(struct tetra_ms_event) == 4);
TETRA_MS_ASSERT(TETRA_MS_ALIGNOF(struct tetra_ms_action) == 4);
TETRA_MS_ASSERT(offsetof(struct tetra_ms_event, pdu) == 52);
TETRA_MS_ASSERT(offsetof(struct tetra_ms_action, pdu) == 56);

#if defined(__cplusplus)
}
#endif
#endif
