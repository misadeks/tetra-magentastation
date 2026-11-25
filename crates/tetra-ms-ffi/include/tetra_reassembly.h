#ifndef TETRA_REASSEMBLY_H
#define TETRA_REASSEMBLY_H
#include "tetra_ms.h"

#define TETRA_MS_REASSEMBLY_STORAGE_SIZE 2304u
#define TETRA_MS_FRAGMENT_FIRST 1u
#define TETRA_MS_FRAGMENT_NEXT 2u
#define TETRA_MS_FRAGMENT_LAST 3u
#define TETRA_MS_FRAGMENT_TICK 4u
#define TETRA_MS_FRAGMENT_REPLACED 2
#define TETRA_MS_FRAGMENT_EXPIRED_MASK 0x100
#define TETRA_MS_FRAGMENT_OVERFLOW_ABORTED (-17)

struct tetra_ms_reassembly_storage {
    TETRA_MS_ALIGN(4) uint8_t bytes[TETRA_MS_REASSEMBLY_STORAGE_SIZE];
};
struct tetra_ms_fragment_event {
    uint32_t size, abi_version, kind, now, ssi, ssi_type, aie_valid;
    int32_t aie_info;
    uint16_t pdu_bits, reserved;
    uint8_t pdu[512];
};
struct tetra_ms_fragment_action {
    uint32_t size, abi_version, ssi, ssi_type, first, last, aie_valid;
    int32_t aie_info;
    uint16_t pdu_bits, fragments;
    uint8_t pdu[512];
};

/* Same pointer, lifetime and serialization rules as tetra_ms.h. This is an
 * independent storage object, not the basic-link storage. Initialize once,
 * then reset in place. All inputs are consumed during the call.
 * aie_info is opaque metadata; no encryption/decryption is performed.
 * FIRST can return REPLACED; TICK can return EXPIRED_MASK | timeslot_bitmask.
 * Continuation overflow returns OVERFLOW_ABORTED and clears that chain.
 * poll selects the timeslot of `now`, consumes one completed chain, and returns
 * EMPTY without modifying output when none is complete. Invalid outputs do
 * not consume a completed chain. Size/version must be initialized on output.
 */
#if defined(__cplusplus)
extern "C" {
#endif
uint32_t tetra_ms_reassembly_size(void);
int32_t tetra_ms_reassembly_init(void *storage, uint32_t size);
int32_t tetra_ms_reassembly_reset(void *storage);
int32_t tetra_ms_reassembly_push(void *storage, const struct tetra_ms_fragment_event *event);
int32_t tetra_ms_reassembly_poll(void *storage, uint32_t now, struct tetra_ms_fragment_action *action);
#if defined(__cplusplus)
}
#endif

TETRA_MS_ASSERT(sizeof(struct tetra_ms_reassembly_storage) == TETRA_MS_REASSEMBLY_STORAGE_SIZE);
TETRA_MS_ASSERT(TETRA_MS_ALIGNOF(struct tetra_ms_reassembly_storage) == 4);
TETRA_MS_ASSERT(sizeof(struct tetra_ms_fragment_event) == 548);
TETRA_MS_ASSERT(sizeof(struct tetra_ms_fragment_action) == 548);
TETRA_MS_ASSERT(offsetof(struct tetra_ms_fragment_event, pdu) == 36);
TETRA_MS_ASSERT(offsetof(struct tetra_ms_fragment_action, pdu) == 36);
#endif
