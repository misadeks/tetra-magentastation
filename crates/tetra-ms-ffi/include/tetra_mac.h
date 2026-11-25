#ifndef TETRA_MAC_H
#define TETRA_MAC_H
#include <stdint.h>
#include <stddef.h>

#define TETRA_MS_MAC_ABI_VERSION 1u
#define TETRA_MS_MAC_STORAGE_SIZE 32768u
#define TETRA_MS_MAC_STORAGE_ALIGN 4u
#define TETRA_MS_MAC_PDU_BYTES 512u
#define TETRA_MS_MAC_MAX_GROUPS 256u
#define TETRA_MS_MAC_MAX_RESERVED_SLOTS 6u
#define TETRA_MS_MAC_ACTION_CAPACITY 8u
#define TETRA_MS_MAC_OK 0
#define TETRA_MS_MAC_EMPTY 1
#define TETRA_MS_MAC_ARGUMENT (-1)
#define TETRA_MS_MAC_ABI_MISMATCH (-2)
#define TETRA_MS_MAC_UNINITIALIZED (-3)
#define TETRA_MS_MAC_INVALID_IDENTITY (-4)
#define TETRA_MS_MAC_INVALID_TIME (-5)
#define TETRA_MS_MAC_INVALID_PDU (-6)
#define TETRA_MS_MAC_TRUNCATED (-7)
#define TETRA_MS_MAC_UNSUPPORTED (-9)
#define TETRA_MS_MAC_CAPACITY (-10)
#define TETRA_MS_MAC_QUEUE_FULL (-11)
#define TETRA_MS_MAC_PANIC (-16)
#define TETRA_MS_MAC_RA_INVALID_TIME (-200)
#define TETRA_MS_MAC_RA_INVALID_PARAMETERS (-201)
#define TETRA_MS_MAC_RA_INVALID_ENTROPY (-202)
#define TETRA_MS_MAC_RA_BUSY (-203)
#define TETRA_MS_MAC_RA_NO_VALID_CODE (-204)
#define TETRA_MS_MAC_RA_NOT_PERMITTED (-205)
#define TETRA_MS_MAC_RA_MAX_TRANSMISSIONS (-206)
#define TETRA_MS_MAC_GRANT_MISSING (-220)
#define TETRA_MS_MAC_GRANT_DELAY (-221)
#define TETRA_MS_MAC_GRANT_CAPACITY (-222)
#define TETRA_MS_MAC_GRANT_MISSED (-223)
#define TETRA_MS_MAC_EXTENDED_ALLOCATION (-240)
#define TETRA_MS_MAC_AUGMENTED_ALLOCATION (-241)
#define TETRA_MS_MAC_SUPPLEMENTARY (-242)
#define TETRA_MS_MAC_U_SIGNAL (-243)
#define TETRA_MS_MAC_BROADCAST_UNSUPPORTED (-244)
#define TETRA_MS_MAC_BIT_INVALID_WIDTH (-260)
#define TETRA_MS_MAC_BIT_INVALID_VALUE (-261)

#define TETRA_MS_MAC_SUBMIT 1u
#define TETRA_MS_MAC_TICK 2u
#define TETRA_MS_MAC_AACH 3u
#define TETRA_MS_MAC_DOWNLINK 4u
#define TETRA_MS_MAC_SYNC 5u
#define TETRA_MS_MAC_IDENTITIES 6u
#define TETRA_MS_MAC_UPLANE 7u
#define TETRA_MS_MAC_AUDIO 8u
#define TETRA_MS_MAC_ALLOCATE 9u
#define TETRA_MS_MAC_BIND_MARKER 10u
#define TETRA_MS_MAC_TRAFFIC_OWNER 11u
#define TETRA_MS_MAC_CANCEL_ACCESS 12u
#define TETRA_MS_MAC_SET_TIME 13u
#define TETRA_MS_MAC_SET_PARAMS 14u
#define TETRA_MS_MAC_RA_INITIATE 15u
#define TETRA_MS_MAC_RA_POLL 16u
#define TETRA_MS_MAC_RA_RESPONSE 17u
#define TETRA_MS_MAC_RA_CANCEL 18u
#define TETRA_MS_MAC_GRANT_FRAGMENT 19u
#define TETRA_MS_MAC_POLL_RESERVED 20u
#define TETRA_MS_MAC_POLL_TRAFFIC 21u
#define TETRA_MS_MAC_PROMOTE 22u
#define TETRA_MS_MAC_ADVANCE 23u
#define TETRA_MS_MAC_CHECK_ADDRESS 24u
#define TETRA_MS_MAC_ENCODE 25u
#define TETRA_MS_MAC_SCRAMBLING 26u
#define TETRA_MS_MAC_CONTINUE_BLOCK 27u

#define TETRA_MS_MAC_E_RECEIPT 1u
#define TETRA_MS_MAC_E_PRIORITY 2u
#define TETRA_MS_MAC_E_EMERGENCY 4u
#define TETRA_MS_MAC_E_STEALING 8u
#define TETRA_MS_MAC_E_INDIVIDUAL 16u
#define TETRA_MS_MAC_E_GROUPS 32u
#define TETRA_MS_MAC_E_SWITCH 64u
#define TETRA_MS_MAC_E_TX_GRANT 128u
#define TETRA_MS_MAC_E_UL_VALID 256u
#define TETRA_MS_MAC_E_ADDRESS_MATCH 512u
#define TETRA_MS_MAC_E_RA_SUCCESS 1024u
#define TETRA_MS_MAC_E_RECOVERED 2048u
#define TETRA_MS_MAC_E_CLEAR_RECOVERED 4096u
#define TETRA_MS_MAC_C_FILTERED 1u
#define TETRA_MS_MAC_C_INDIVIDUAL 2u
#define TETRA_MS_MAC_C_NETWORK 4u
#define TETRA_MS_MAC_C_COLOUR 8u
#define TETRA_MS_MAC_C_SCRAMBLING 16u
#define TETRA_MS_MAC_C_CARRIER 32u
#define TETRA_MS_MAC_C_RECOVERED 64u

#define TETRA_MS_MAC_TRANSMIT 1u
#define TETRA_MS_MAC_SUBMITTED 2u
#define TETRA_MS_MAC_DISCARDED_TRANSFER 3u
#define TETRA_MS_MAC_DISCARDED_STOLEN 4u
#define TETRA_MS_MAC_RA_TRANSMIT 5u
#define TETRA_MS_MAC_RA_SUCCEEDED 6u
#define TETRA_MS_MAC_RA_FAILED 7u
#define TETRA_MS_MAC_RESOURCE 8u
#define TETRA_MS_MAC_FRAGMENT 9u
#define TETRA_MS_MAC_END 10u
#define TETRA_MS_MAC_SYNCED 11u
#define TETRA_MS_MAC_SYSINFO 12u
#define TETRA_MS_MAC_ACCESS_PARAMS 13u
#define TETRA_MS_MAC_CONFIGURE 14u
#define TETRA_MS_MAC_ALLOCATION 15u
#define TETRA_MS_MAC_OWNER 16u
#define TETRA_MS_MAC_AUDIO_DROPPED 17u
#define TETRA_MS_MAC_CANCELLED 18u
#define TETRA_MS_MAC_FRAGMENT_REJECTED 19u
#define TETRA_MS_MAC_PLAN_INSTALLED 20u
#define TETRA_MS_MAC_RESERVED_MISSED 21u
#define TETRA_MS_MAC_ADDRESS_RESULT 22u
#define TETRA_MS_MAC_ENCODED 23u
#define TETRA_MS_MAC_AACH_INFO 24u
#define TETRA_MS_MAC_NEXT_PDU 25u

#define TETRA_MS_MAC_F_RECEIPT 1u
#define TETRA_MS_MAC_F_RECEIPT2 2u
#define TETRA_MS_MAC_F_RESERVED 4u
#define TETRA_MS_MAC_F_TRAFFIC 8u
#define TETRA_MS_MAC_F_SECOND 16u
#define TETRA_MS_MAC_F_PRIORITY 32u
#define TETRA_MS_MAC_F_EMERGENCY 64u
#define TETRA_MS_MAC_F_ADDRESS 128u
#define TETRA_MS_MAC_F_EVENT_LABEL 256u
#define TETRA_MS_MAC_F_MARKER 512u
#define TETRA_MS_MAC_F_GRANT 1024u
#define TETRA_MS_MAC_F_ALLOCATION 2048u
#define TETRA_MS_MAC_F_TRUNCATED 4096u
#define TETRA_MS_MAC_F_SECOND_STOLEN 8192u
#define TETRA_MS_MAC_F_ACKS_OWN 16384u
#define TETRA_MS_MAC_F_HYPERFRAME 32768u
#define TETRA_MS_MAC_F_DEFAULT_ACCESS 65536u
#define TETRA_MS_MAC_F_APPLIED 131072u
#define TETRA_MS_MAC_F_SCRAMBLING 262144u
#define TETRA_MS_MAC_F_MAC_BLOCK 524288u
#define TETRA_MS_MAC_F_ASSIGNED_CONTROL 1048576u
#define TETRA_MS_MAC_F_RANDOM_ACCESS 2097152u
#define TETRA_MS_MAC_F_FILL 4194304u
#define TETRA_MS_MAC_F_ALLOCATION_RESULT 8388608u
#define TETRA_MS_MAC_F_ACCESS 16777216u

#define TETRA_MS_MAC_S_RECOVERED 1u
#define TETRA_MS_MAC_S_INDIVIDUAL 2u
#define TETRA_MS_MAC_S_NETWORK 4u
#define TETRA_MS_MAC_S_COLOUR 8u
#define TETRA_MS_MAC_S_SCRAMBLING 16u
#define TETRA_MS_MAC_S_CARRIER 32u
#define TETRA_MS_MAC_S_TRAFFIC_OWNER 64u
#define TETRA_MS_MAC_S_MARKER 128u
#define TETRA_MS_MAC_S_PENDING 256u
#define TETRA_MS_MAC_S_FRAGMENT 512u
#define TETRA_MS_MAC_S_RESERVED 1024u
#define TETRA_MS_MAC_S_PENDING_RECEIPT 2048u
#define TETRA_MS_MAC_S_FRAGMENT_RECEIPT 4096u
#define TETRA_MS_MAC_S_PRIORITY 8192u
#define TETRA_MS_MAC_S_RA_ACTIVE 16384u
#define TETRA_MS_MAC_S_FILTERED 32768u
#define TETRA_MS_MAC_S_RESERVED_RECEIPT 65536u

#define TETRA_MS_MAC_SCH_HU 1u
#define TETRA_MS_MAC_SCH_F 2u
#define TETRA_MS_MAC_STCH 3u
#define TETRA_MS_MAC_TCH_S 4u
#define TETRA_MS_MAC_RA_IDLE 0u
#define TETRA_MS_MAC_RA_FIRST_TRY 1u
#define TETRA_MS_MAC_RA_AWAIT_MARKER 2u
#define TETRA_MS_MAC_RA_COUNTING 3u
#define TETRA_MS_MAC_RA_AWAIT_RESPONSE 4u
#define TETRA_MS_MAC_ENTROPY_NONE 0u
#define TETRA_MS_MAC_ENTROPY_INDEX 1u
#define TETRA_MS_MAC_ENTROPY_SUBSLOT 2u
#define TETRA_MS_MAC_CODE_A 0u
#define TETRA_MS_MAC_CODE_B 1u
#define TETRA_MS_MAC_CODE_C 2u
#define TETRA_MS_MAC_CODE_D 3u
#define TETRA_MS_MAC_PARAM_SUBSCRIBER_CLASS 1u
#define TETRA_MS_MAC_PARAM_GSSI 2u
#define TETRA_MS_MAC_FRAME_RESERVED 0u
#define TETRA_MS_MAC_FRAME_CLCH 1u
#define TETRA_MS_MAC_FRAME_ONGOING 2u
#define TETRA_MS_MAC_FRAME_MARKER 3u
#define TETRA_MS_MAC_GRANT_FIRST_SUBSLOT 0u
#define TETRA_MS_MAC_GRANT_SECOND_SUBSLOT 1u
#define TETRA_MS_MAC_GRANT_SLOTS 2u
#define TETRA_MS_MAC_ALLOC_EXTENDED 1u
#define TETRA_MS_MAC_ALLOC_CELL_CHANGE 2u
#define TETRA_MS_MAC_ALLOC_DOWNLINK 4u
#define TETRA_MS_MAC_ALLOC_REPLACE 0u
#define TETRA_MS_MAC_ALLOC_ADDITIONAL 1u
#define TETRA_MS_MAC_ROUTE_NO_ADDRESS 0u
#define TETRA_MS_MAC_ROUTE_ENCRYPTED 1u
#define TETRA_MS_MAC_ROUTE_FOREIGN 2u
#define TETRA_MS_MAC_ROUTE_EMPTY 3u
#define TETRA_MS_MAC_ROUTE_DELIVER 4u
#define TETRA_MS_MAC_ROUTE_FIRST_FRAGMENT 5u
#define TETRA_MS_MAC_TRANSFER_IDLE 0u
#define TETRA_MS_MAC_TRANSFER_ACCESS 1u
#define TETRA_MS_MAC_TRANSFER_GRANT 2u
#define TETRA_MS_MAC_TRANSFER_RESERVED 3u
#define TETRA_MS_MAC_END_HU 1u
#define TETRA_MS_MAC_END_UL 2u
#define TETRA_MS_MAC_FRAG_UL 3u

#if defined(__cplusplus)
#define TETRA_MS_MAC_ALIGN(n) alignas(n)
#define TETRA_MS_MAC_ASSERT(x) static_assert(x, #x)
#define TETRA_MS_MAC_ALIGNOF(t) alignof(t)
extern "C" {
#elif defined(_MSC_VER)
#define TETRA_MS_MAC_ALIGN(n) __declspec(align(n))
#define TETRA_MS_MAC_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MS_MAC_ALIGNOF(t) __alignof(t)
#else
#define TETRA_MS_MAC_ALIGN(n) _Alignas(n)
#define TETRA_MS_MAC_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MS_MAC_ALIGNOF(t) _Alignof(t)
#endif

struct tetra_ms_mac_storage { TETRA_MS_MAC_ALIGN(4) uint8_t bytes[TETRA_MS_MAC_STORAGE_SIZE]; };
struct tetra_ms_mac_params {
    uint32_t flags, imm, wt, nu, fl_factor, ts_ptr, min_priority, subscriber_class, gssi;
};
struct tetra_ms_mac_slot_access { uint32_t code_present, code, frame_kind, frame_length; };
struct tetra_ms_mac_grant { uint32_t kind, slots, next_opportunity; };
struct tetra_ms_mac_allocation { uint32_t carrier, flags, kind, slot_bitmap, owner; };
struct tetra_ms_mac_config {
    uint32_t size, abi_version, own_issi, flags, now, recovered_now, mcc, mnc;
    uint32_t colour, scrambling_code, serving_carrier, group_count;
    uint32_t groups[TETRA_MS_MAC_MAX_GROUPS];
};
struct tetra_ms_mac_event {
    uint32_t size, abi_version, kind, now, flags, receipt, ssi, ssi_type;
    uint32_t priority, code, scrambling_code, mcc, mnc, source, value;
    uint32_t choice_index, choice_subslot, group_count, pdu_bits;
    struct tetra_ms_mac_allocation allocation;
    struct tetra_ms_mac_params params;
    struct tetra_ms_mac_grant grant;
    struct tetra_ms_mac_slot_access subslots[2];
    uint32_t groups[TETRA_MS_MAC_MAX_GROUPS];
    uint8_t pdu[TETRA_MS_MAC_PDU_BYTES];
};
struct tetra_ms_mac_action {
    uint32_t size, abi_version, kind, flags, now, receipt, receipt2;
    int32_t reason;
    uint32_t code, subslot, channel, channel2, scrambling_code, scrambling_code2;
    uint32_t route, ssi, ssi_type, event_label, marker, encryption, length_ind;
    uint32_t header_bits, payload_end, next_bits, fill_bits, allocation_result;
    uint32_t mcc, mnc, colour, carrier, band, offset, duplex, reverse, option, option_bits;
    uint32_t fragment_kind, fragment_slots, pdu_bits, pdu2_bits, hyperframe;
    struct tetra_ms_mac_grant grant;
    struct tetra_ms_mac_allocation allocation;
    struct tetra_ms_mac_params params;
    struct tetra_ms_mac_slot_access subslots[2];
    uint8_t pdu[TETRA_MS_MAC_PDU_BYTES], pdu2[TETRA_MS_MAC_PDU_BYTES];
};
struct tetra_ms_mac_entropy { uint32_t size, abi_version, kind, maximum; int32_t admission_failure; };
struct tetra_ms_mac_status {
    uint32_t size, abi_version, flags, now, recovered_now, own_issi, individual_ssi, mcc, mnc;
    uint32_t colour, scrambling_code, serving_carrier, traffic_owner, current_marker;
    uint32_t transfer_status, queued_uplinks, queued_stolen, queued_audio, assigned_slot_bitmap;
    uint32_t tx_granted, actions, ra_phase, ra_code, ra_emergency, ra_transmissions;
    uint32_t group_count, defined_mask, sysinfo_present, param_mask, pending_receipt, fragment_receipt;
    uint32_t pending_priority, pending_emergency, fragment_kind, fragment_slots;
    uint32_t pending_bits, fragment_bits, reserved_count;
    uint32_t pending_channel, pending_scrambling_code, fragment_scrambling_code, reserved_scrambling_code, reserved_receipt;
    uint32_t reserved_times[TETRA_MS_MAC_MAX_RESERVED_SLOTS];
    struct tetra_ms_mac_params params[4];
    uint32_t groups[TETRA_MS_MAC_MAX_GROUPS], marker_mask[8], marker_owners[256];
    uint8_t pending_pdu[TETRA_MS_MAC_PDU_BYTES], fragment_pdu[TETRA_MS_MAC_PDU_BYTES];
};

/*
 * Opaque caller-owned storage is independent of LLC, MLE, MM and reassembly.
 * No allocation, clock, radio operation or RNG callback is performed. Place
 * storage/output buffers in static or otherwise suitably sized memory.
 * Serialize calls per instance. All pointers must designate accessible aligned
 * regions of their full declared size, disjoint from the entire storage budget.
 * Query input/output objects must also be mutually disjoint. Init accepts
 * uninitialized storage; all other operations require successful init. Reset
 * cancels every old external receipt and pending action; caller must invalidate
 * those receipt tokens. Init/reset initialize in place without a full MAC copy.
 * Zero-initialize inputs and set size/abi_version. Outputs need only initialized
 * size/abi_version; successful calls zero every unused byte. EMPTY and errors
 * leave output, action queue and protocol state unchanged. This does not cover
 * internal PANIC: discard/reinitialize after PANIC.
 * A push_event error leaves ownership of the submitted receipt with the caller.
 *
 * Times are absolute TDMA slot integers [0,283115520), not elapsed durations.
 * Payloads are packed MSB-first; pdu_bits <= 4096; every count is fixed-width.
 * Tick.now is recovered downlink time, using native recovered-delta semantics.
 * AACH uses the stored downlink time. SET_TIME.now re-seeds it; E_RECOVERED sets
 * last_recovered to event.value; E_CLEAR_RECOVERED clears that reference.
 * SCRAMBLING.source 0 clears, 1 sets event.scrambling_code without changing
 * existing transfer blocks' captured scrambling codes. It emits CONFIGURE.
 *
 * SUBMIT: pdu, optional receipt/priority/emergency/stealing flags. Native MAC
 * chooses complete/fragmented access or stealing and returns SUBMITTED.route
 * (0 access, 1 fragmented, 2 stolen). Overflow returns complete displaced data
 * in DISCARDED_TRANSFER/STOLEN, including both optional receipt tokens.
 * DISCARDED_TRANSFER.code carries its priority when F_PRIORITY is present.
 * fragment_kind 1/2/3 means END-HU/END-UL/multi-slot continuation; fragment_slots
 * is respectively zero (one subslot), one, or the requested full-slot count.
 * Tokens may be zero: presence flags, never zero values, discriminate them.
 * DOWNLINK: raw signalling PDU; source!=0 means STCH for classification.
 * Resource ACK completion and reserved burst actions precede RESOURCE routing.
 * RESOURCE.route: 0 no address, 1 encrypted, 2 foreign address, 3 empty,
 * 4 deliver to LLC, 5 first fragment. FRAGMENT/END feed existing reassembly ABI:
 * pdu is the stripped SDU, pdu2 the original raw input, header/end/next/fill are
 * original bit offsets. No duplicate reassembly state is stored here.
 * For associated/concatenated PDUs, CONTINUE_BLOCK takes the same raw input
 * and value=next_bits. Native progress/minimum-null-PDU checks return NEXT_PDU
 * with an already MSB-repacked remaining window, or no action at end of block.
 * Feed that window to DOWNLINK; no C bit parser/copy is required. No-address
 * and encrypted resource routes stop the block, as in native UMAC. An END
 * continuation is driven only after the independent reassembly result exists.
 * F_SECOND_STOLEN requests immediate LMAC Block2 stealing before LLC delivery.
 * allocation_result: 0 applied, 1 cross-cell, 2 cross-carrier, 3 unknown carrier,
 * 4 uplink-only, 5 individual call engaged.
 * SYNC: raw BSCH; SYNCED returns colour/time and the remaining MLE SDU.
 * SYSINFO preserves all native RF/default-A fields plus raw 20-bit option
 * information. CONFIGURE time correction precedes SYSINFO SDU delivery.
 * IDENTITIES: mcc/mnc, optional E_INDIVIDUAL(ssi), E_GROUPS(groups).
 * UPLANE: E_SWITCH/E_TX_GRANT; AUDIO: packed source speech (native truncation
 * to 274 bits). AUDIO_DROPPED explicitly reports jitter-buffer displacement.
 * ALLOCATE uses allocation; bitmap bit3=TS1...bit0=TS4. Allocation.flags:
 * extended=1, cell_change=2, downlink=4; kind 0 replace, 1 additional.
 * BIND_MARKER: value=marker byte, ssi=owner. TRAFFIC_OWNER returns OWNER only
 * for a currently matched marker. CHECK_ADDRESS uses ssi/ssi_type and value
 * 0 unfiltered, 1 filtered, 2 configured mode; ADDRESS_RESULT.route is boolean.
 *
 * SET_PARAMS: source 0 SYSINFO default-A (code must be 0), 1 common DEFINE,
 * 2 assigned DEFINE (reported but not adopted). Params.flags indicate optional
 * subscriber class=1/GSSI=2. DEFINE-A permanently takes precedence over default
 * A until reset, exactly as native. Codes 0..3 mean A..D.
 * RA_INITIATE: now, code, priority, optional E_EMERGENCY; uses stored params.
 * RA_POLL: now, E_UL_VALID, subslots[] and choices. frame_kind is 0 reserved,
 * 1 CLCH, 2 ongoing, 3 marker with frame_length in subslots. code_present is
 * boolean. AACH automatically admits the queued default-A transfer first.
 * Call entropy_needed with the SAME AACH/RA_POLL event before pushing. It is
 * pure: admission/queue promotion are simulated without committing. kind 1
 * requests choice_index in 1..maximum; kind 2 requests choice_subslot 1 or 2.
 * No choice is required for kind 0. admission_failure predicts a native
 * permission failure; push then reports RA_FAILED and discarded receipts.
 * Invalid choices are non-consuming and may be corrected/retried.
 * RA_TRANSMIT always includes exact time/subslot, even for standalone RA with
 * no queued MAC block; F_MAC_BLOCK indicates an attached retransmittable block.
 * F_RECEIPT on RA_SUCCEEDED/TRANSMIT means mark transmitted; on RA_FAILED,
 * FRAGMENT_REJECTED, RESERVED_MISSED/CANCELLED or DISCARDED_TRANSFER/STOLEN means
 * mark discarded. These completion rules apply to both receipt presence flags.
 * RA_RESPONSE uses E_ADDRESS_MATCH/E_RA_SUCCESS for standalone RA state only.
 * RA_CANCEL cancels only RA; CANCEL_ACCESS additionally returns both active
 * MAC receipts. Normal MAC completion uses DOWNLINK MAC-RESOURCE instead.
 * GRANT_FRAGMENT: source 0 missing grant, 1 present grant; grant.kind 0 first
 * subslot, 1 second subslot, 2 full slots; next_opportunity is boolean. The
 * initial reserved slot is polled immediately. POLL_RESERVED/TRAFFIC and
 * PROMOTE expose separate native driving; ADVANCE updates clock only.
 * TRANSMIT with F_TRAFFIC uses a TCH burst: when F_SECOND is set, pdu/channel
 * are stolen STCH and pdu2/channel2 speech. Otherwise pdu is full speech.
 * Tick emits reserved results before traffic results. Reserved times are exact.
 * ENCODE.value: 1 access, 2 STCH data, 3 FRAG-UL, 4 END-HU, 5 END-UL,
 * 6 access-fragment (pdu2=remainder), 7 full-slot fragment sequence
 * (source=slot count 1..6). ENCODE uses ssi; returns ENCODED blocks in order,
 * with code equal to the zero-based block index for a sequence.
 *
 * Drain actions regularly. Before mutation each event reserves its documented
 * worst-case output budget (SUBMIT/TICK/AACH/SYSINFO=2, RESOURCE=3,
 * GRANT_FRAGMENT=2, ENCODE sequence=source, other emitting events=1).
 * QUEUE_FULL never partially applies an event. Unknown/unsupported input is an
 * explicit error, not a silently reduced MAC feature set.
 */
uint32_t tetra_ms_mac_abi_version(void);
uint32_t tetra_ms_mac_storage_size(void);
uint32_t tetra_ms_mac_storage_align(void);
int32_t tetra_ms_mac_init(void *, uint32_t, const struct tetra_ms_mac_config *);
int32_t tetra_ms_mac_reset(void *, const struct tetra_ms_mac_config *);
int32_t tetra_ms_mac_push_event(void *, const struct tetra_ms_mac_event *);
int32_t tetra_ms_mac_entropy_needed(const void *, const struct tetra_ms_mac_event *, struct tetra_ms_mac_entropy *);
int32_t tetra_ms_mac_poll_action(void *, struct tetra_ms_mac_action *);
int32_t tetra_ms_mac_get_status(const void *, struct tetra_ms_mac_status *);

TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_storage) == TETRA_MS_MAC_STORAGE_SIZE);
TETRA_MS_MAC_ASSERT(TETRA_MS_MAC_ALIGNOF(struct tetra_ms_mac_storage) == TETRA_MS_MAC_STORAGE_ALIGN);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_params) == 36);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_slot_access) == 16);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_grant) == 12);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_allocation) == 20);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_config) == 1072);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_event) == 1712);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_action) == 1288);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_entropy) == 20);
TETRA_MS_MAC_ASSERT(sizeof(struct tetra_ms_mac_status) == 3444);
TETRA_MS_MAC_ASSERT(offsetof(struct tetra_ms_mac_config, groups) == 48);
TETRA_MS_MAC_ASSERT(offsetof(struct tetra_ms_mac_event, pdu) == 1200);
TETRA_MS_MAC_ASSERT(offsetof(struct tetra_ms_mac_action, pdu) == 264);
TETRA_MS_MAC_ASSERT(offsetof(struct tetra_ms_mac_action, pdu2) == 776);
TETRA_MS_MAC_ASSERT(offsetof(struct tetra_ms_mac_status, groups) == 340);
TETRA_MS_MAC_ASSERT(offsetof(struct tetra_ms_mac_status, pending_pdu) == 2420);

#if defined(__cplusplus)
}
#endif
#endif
