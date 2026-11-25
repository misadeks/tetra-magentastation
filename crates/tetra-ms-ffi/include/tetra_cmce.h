#ifndef TETRA_CMCE_H
#define TETRA_CMCE_H
#include <stdint.h>
#include <stddef.h>
#define TETRA_CMCE_ABI_VERSION 1u
#define TETRA_CMCE_STORAGE_SIZE 20480u
#define TETRA_CMCE_STORAGE_ALIGN 8u
#define TETRA_CMCE_PDU_BYTES 336u
#define TETRA_CMCE_MAX_CALLS 16u
#define TETRA_CMCE_MAX_ORIGINATIONS 16u
#define TETRA_CMCE_MAX_ACTIONS 40u
#define TETRA_CMCE_TIME_WRAP 283115520u
#define TETRA_CMCE_OK 0
#define TETRA_CMCE_EMPTY 1
#define TETRA_CMCE_ARGUMENT (-1)
#define TETRA_CMCE_ABI_MISMATCH (-2)
#define TETRA_CMCE_UNINITIALIZED (-3)
#define TETRA_CMCE_INVALID_IDENTITY (-4)
#define TETRA_CMCE_INVALID_TIME (-5)
#define TETRA_CMCE_TRUNCATED (-7)
#define TETRA_CMCE_UNSUPPORTED (-9)
#define TETRA_CMCE_CAPACITY (-10)
#define TETRA_CMCE_QUEUE_FULL (-11)
#define TETRA_CMCE_PANIC (-16)
#define TETRA_CMCE_UNKNOWN_CALL (-201)
#define TETRA_CMCE_NOT_ALLOWED (-202)
#define TETRA_CMCE_INVALID_VALUE (-203)
#define TETRA_CMCE_INVALID_WIDTH (-204)
/* Event kinds. All input records must be completely initialized. */
#define TETRA_CMCE_IDENTITY 1u
#define TETRA_CMCE_DOWNLINK 2u
#define TETRA_CMCE_ORIGINATE 3u
#define TETRA_CMCE_ALERT 4u
#define TETRA_CMCE_CONNECT 5u
#define TETRA_CMCE_ANSWER 6u
#define TETRA_CMCE_DEMAND 7u
#define TETRA_CMCE_CEASE 8u
#define TETRA_CMCE_DISCONNECT 9u
#define TETRA_CMCE_RELEASE 10u
#define TETRA_CMCE_RESTORE 11u
#define TETRA_CMCE_DTMF 12u
#define TETRA_CMCE_TRAFFIC 13u
#define TETRA_CMCE_TICK 14u
#define TETRA_CMCE_BREAK 15u
#define TETRA_CMCE_REOPEN 16u
#define TETRA_CMCE_TRANSMITTED 17u
#define TETRA_CMCE_HAS_IDENTITY 1u
#define TETRA_CMCE_DUPLEX 2u
#define TETRA_CMCE_REQUEST_TX 4u
#define TETRA_CMCE_HOOK 8u
#define TETRA_CMCE_BASIC_PRESENT 16u
#define TETRA_CMCE_COMPLETE 32u
#define TETRA_CMCE_OWNER_PRESENT 64u
/* Action kinds/flags. Call lifecycle lets adapters maintain audio sidecars. */
#define TETRA_CMCE_UPLINK 1u
#define TETRA_CMCE_CONFIGURE 2u
#define TETRA_CMCE_NOTIFY 3u
#define TETRA_CMCE_SPEECH 4u
#define TETRA_CMCE_CALL 5u
#define TETRA_CMCE_STEALING 1u
#define TETRA_CMCE_REPEATS 2u
#define TETRA_CMCE_SWITCH_U_PLANE 4u
#define TETRA_CMCE_TX_GRANT 8u
#define TETRA_CMCE_U_PLANE_DUPLEX 16u
#define TETRA_CMCE_SPEAKER_PRESENT 32u
#define TETRA_CMCE_CALL_CREATED 0u
#define TETRA_CMCE_CALL_REKEYED 1u
#define TETRA_CMCE_CALL_REMOVED 2u
#define TETRA_CMCE_NOTICE_SETUP 0u
#define TETRA_CMCE_NOTICE_PROCEED 1u
#define TETRA_CMCE_NOTICE_ALERT 2u
#define TETRA_CMCE_NOTICE_SETUP_CONFIRM 3u
#define TETRA_CMCE_NOTICE_COMPLETE_CONFIRM 4u
#define TETRA_CMCE_NOTICE_TX_CONFIRM 5u
#define TETRA_CMCE_NOTICE_TX_GRANTED 6u
#define TETRA_CMCE_NOTICE_TX_CEASED 7u
#define TETRA_CMCE_NOTICE_TX_WAIT 8u
#define TETRA_CMCE_NOTICE_TX_CONTINUE 9u
#define TETRA_CMCE_NOTICE_TX_INTERRUPT 10u
#define TETRA_CMCE_NOTICE_RELEASE_INDICATION 11u
#define TETRA_CMCE_NOTICE_RELEASE_CONFIRM 12u
#define TETRA_CMCE_NOTICE_INFO 13u
/* Optional decoded fields: independent presence and boolean-value masks. */
#define TETRA_CMCE_P_TIMEOUT 1u
#define TETRA_CMCE_P_SETUP_TIMEOUT 2u
#define TETRA_CMCE_P_BASIC 4u
#define TETRA_CMCE_P_PRIORITY 8u
#define TETRA_CMCE_P_OWNERSHIP 16u
#define TETRA_CMCE_P_SPEAKER 32u
#define TETRA_CMCE_P_EXTENSION 64u
#define TETRA_CMCE_P_NOTIFICATION 128u
#define TETRA_CMCE_P_CALL_STATUS 256u
#define TETRA_CMCE_P_NEW_ID 512u
#define TETRA_CMCE_P_POLL_PERCENTAGE 1024u
#define TETRA_CMCE_P_POLL_NUMBER 2048u
#define TETRA_CMCE_P_EXTERNAL 4096u
#define TETRA_CMCE_P_DTMF 8192u
#define TETRA_CMCE_F_HOOK 1u
#define TETRA_CMCE_F_DUPLEX 2u
#define TETRA_CMCE_F_PERMISSION_DENIED 4u
#define TETRA_CMCE_F_OWNERSHIP 8u
#define TETRA_CMCE_F_RESET 16u
#define TETRA_CMCE_F_POLL 32u
#define TETRA_CMCE_F_QUEUED 64u
#define TETRA_CMCE_F_RESUME 128u
#define TETRA_CMCE_F_ENCRYPTED 256u
/* Status flags; per-call flags additionally describe saved WAIT U-plane state. */
#define TETRA_CMCE_S_IDENTITY 1u
#define TETRA_CMCE_S_TRANSMIT_CALL 2u
#define TETRA_CMCE_C_SPEAKER 1u
#define TETRA_CMCE_C_SETUP_DEADLINE 2u
#define TETRA_CMCE_C_CALL_DEADLINE 4u
#define TETRA_CMCE_C_SETUP_TIMEOUT 8u
#define TETRA_CMCE_C_CAUSE 16u
#define TETRA_CMCE_C_REQUEST_ALLOWED 32u
#define TETRA_CMCE_C_REQUEST_PENDING 64u
#define TETRA_CMCE_C_HOOK 128u
#define TETRA_CMCE_C_DUPLEX 256u
#define TETRA_CMCE_C_U_PLANE 512u
#define TETRA_CMCE_C_SWITCH 1024u
#define TETRA_CMCE_C_TX 2048u
#define TETRA_CMCE_C_SAVED_U_PLANE 4096u
#define TETRA_CMCE_C_SAVED_SWITCH 8192u
#define TETRA_CMCE_C_SAVED_TX 16384u
#define TETRA_CMCE_C_SAVED_DUPLEX 32768u
#define TETRA_CMCE_C_U_PLANE_DUPLEX 65536u

#if defined(__cplusplus)
#define TETRA_CMCE_ALIGN(n) alignas(n)
#define TETRA_CMCE_ASSERT(x) static_assert(x,#x)
#define TETRA_CMCE_ALIGNOF(t) alignof(t)
extern "C" {
#elif defined(_MSC_VER)
#define TETRA_CMCE_ALIGN(n) __declspec(align(n))
#define TETRA_CMCE_ASSERT(x) _Static_assert(x,#x)
#define TETRA_CMCE_ALIGNOF(t) __alignof(t)
#else
#define TETRA_CMCE_ALIGN(n) _Alignas(n)
#define TETRA_CMCE_ASSERT(x) _Static_assert(x,#x)
#define TETRA_CMCE_ALIGNOF(t) _Alignof(t)
#endif
struct tetra_cmce_storage { TETRA_CMCE_ALIGN(8) uint8_t bytes[TETRA_CMCE_STORAGE_SIZE]; };
struct tetra_cmce_basic { uint8_t circuit_mode,encrypted,communication,parameter; };
struct tetra_cmce_route { uint32_t ssi,ssi_type,handle,endpoint_id,link_id; };
struct tetra_cmce_config { uint32_t size,abi_version,own_issi,flags; };
struct tetra_cmce_event {
    uint32_t size,abi_version,kind,now;
    struct tetra_cmce_route route;
    uint32_t ssi,flags;
    uint16_t id,pdu_bits;
    struct tetra_cmce_basic basic;
    uint8_t priority,cause;
    uint16_t reserved;
    uint8_t pdu[TETRA_CMCE_PDU_BYTES];
};
struct tetra_cmce_fields {
    uint32_t present,flags,speaker,extension;
    uint16_t id,new_id,external_offset,external_bits,dtmf_offset,dtmf_bits;
    struct tetra_cmce_basic basic;
    uint8_t kind,timeout,setup_timeout,grant,priority,notification,call_status,poll_percentage,poll_number,cause;
    uint16_t reserved;
};
struct tetra_cmce_action {
    uint32_t size,abi_version,kind,flags;
    struct tetra_cmce_route route;
    struct tetra_cmce_basic basic;
    uint16_t id,new_id;
    uint32_t sequence,speaker,called_party;
    uint8_t notice_kind,grant,timeout,change;
    struct tetra_cmce_fields fields;
    uint16_t pdu_bits,reserved;
    uint8_t pdu[TETRA_CMCE_PDU_BYTES];
};
struct tetra_cmce_call_status {
    struct tetra_cmce_route route;
    struct tetra_cmce_basic basic;
    uint16_t id;
    uint8_t state,kind;
    uint32_t flags,speaker,setup_deadline,call_deadline,rx_frames,tx_frames;
    uint8_t grant,timeout,setup_timeout,cause;
};
struct tetra_cmce_pending {
    uint32_t ssi,ssi_type;
    struct tetra_cmce_basic basic;
    uint8_t duplex,reserved[3];
};
struct tetra_cmce_status {
    uint32_t size,abi_version,flags,own_issi,now;
    uint16_t calls,originations,actions,transmit_call;
    struct tetra_cmce_call_status call[TETRA_CMCE_MAX_CALLS];
    struct tetra_cmce_pending pending[TETRA_CMCE_MAX_ORIGINATIONS];
};
/* Pointer contract: serialized access per instance; every pointer live/aligned,
 * sufficiently sized, and disjoint from the ENTIRE storage budget. init accepts
 * uninitialized storage; all other operations require a live initialized state.
 * reset is true in-place initialization, discards pending actions/calls and
 * restores Config. No pointers/callbacks/audio buffers are retained.
 * Zero-initialize Config/Event; set size/version. Before poll/status initialize
 * only output size/version; other output bytes may be uninitialized. Errors
 * and EMPTY preserve outputs; poll errors never consume an action. Rejected
 * events preserve state/actions. QUEUE_FULL is retryable after draining actions.
 * Scalars/byte arrays are native C values, never serialized Rust enums/bools.
 * Unknown event kinds => UNSUPPORTED; unknown flags/reserved !=0 => ARGUMENT.
 * Time is absolute TDMA slots [0,TIME_WRAP). PDU bytes packed MSB-first.
 * ssi_type: 0 Unknown,1 SSI,2 ISSI,3 GSSI,4 USSI,5 SMI,6 ESI,7 EventLabel.
 * Basic: mode0..7, encrypted0/1, communication0..3, parameter0..3; mode0's
 * parameter is speech service, otherwise slots/frame. IDs are 14 bits.
 * IDENTITY uses ssi/HAS_IDENTITY. DOWNLINK uses complete CMCE PDU and route.
 * ORIGINATE uses route.ssi/type (ISSI/GSSI), mandatory basic, DUPLEX/REQUEST_TX/
 * HOOK, and optional external-number digit IE payload in pdu (4 bits/digit).
 * ALERT/ANSWER use BASIC_PRESENT and DUPLEX; ANSWER additionally COMPLETE.
 * CONNECT always uses basic/DUPLEX. DEMAND uses priority0..3. Clearing uses
 * cause0..23. DTMF carries its encoded IE, not a U-INFO. TRAFFIC carries optional
 * owner SSI via ssi/OWNER_PRESENT and yields SPEECH attribution, not audio bytes.
 * TRANSMITTED accounts one caller-supplied frame only while authorized.
 * CALL CREATED discards any previous sidecar for id; REKEYED moves id to new_id
 * replacing an existing target; REMOVED discards id (also safe if absent).
 * NOTIFY includes resolved grant/speaker/basic/timeout and the full original
 * downlink plus scalar Fields; external/DTMF offsets refer into that pdu.
 * Config/Identity absent identity means receive-only. Drain actions after events.
 * Call states: Idle0,MOSetup1,MTSetup2,Active3,Wait4,Restore5,Disconnect6,Release7.
 * Call kinds: Individual0,Group1,AcknowledgedGroup2,Broadcast3.
 * Grant states: None0,Self1,Queued2,Other3,Waiting4,Interrupted5.
 * Pending records are oldest-first. Inapplicable output fields are zero.
 * Opaque storage is live instance memory, never a persisted or wire format. */
uint32_t tetra_cmce_abi_version(void);
uint32_t tetra_cmce_core_size(void);
uint32_t tetra_cmce_core_align(void);
int32_t tetra_cmce_init(void*,uint32_t,const struct tetra_cmce_config*);
int32_t tetra_cmce_reset(void*,const struct tetra_cmce_config*);
int32_t tetra_cmce_push_event(void*,const struct tetra_cmce_event*);
int32_t tetra_cmce_poll_action(void*,struct tetra_cmce_action*);
int32_t tetra_cmce_get_status(const void*,struct tetra_cmce_status*);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_storage)==TETRA_CMCE_STORAGE_SIZE);
TETRA_CMCE_ASSERT(TETRA_CMCE_ALIGNOF(struct tetra_cmce_storage)==TETRA_CMCE_STORAGE_ALIGN);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_config)==16);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_basic)==4);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_route)==20);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_event)==392);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_fields)==44);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_action)==444);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_call_status)==56);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_pending)==16);
TETRA_CMCE_ASSERT(sizeof(struct tetra_cmce_status)==1180);
TETRA_CMCE_ASSERT(TETRA_CMCE_ALIGNOF(struct tetra_cmce_event)==4);
TETRA_CMCE_ASSERT(TETRA_CMCE_ALIGNOF(struct tetra_cmce_action)==4);
TETRA_CMCE_ASSERT(TETRA_CMCE_ALIGNOF(struct tetra_cmce_status)==4);
TETRA_CMCE_ASSERT(offsetof(struct tetra_cmce_event,pdu)==56);
TETRA_CMCE_ASSERT(offsetof(struct tetra_cmce_action,pdu)==108);
#ifdef __cplusplus
}
#endif
#endif
