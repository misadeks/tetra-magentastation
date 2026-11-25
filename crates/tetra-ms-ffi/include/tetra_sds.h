#ifndef TETRA_SDS_H
#define TETRA_SDS_H
#include <stdint.h>
#include <stddef.h>
#define TETRA_SDS_ABI_VERSION 1u
#define TETRA_SDS_STORAGE_SIZE 4096u
#define TETRA_SDS_STORAGE_ALIGN 8u
#define TETRA_SDS_PDU_BYTES 336u
#define TETRA_SDS_MAX_TYPE4_BITS 2047u
#define TETRA_SDS_OK 0
#define TETRA_SDS_EMPTY 1
#define TETRA_SDS_ARGUMENT (-1)
#define TETRA_SDS_ABI_MISMATCH (-2)
#define TETRA_SDS_UNINITIALIZED (-3)
#define TETRA_SDS_INVALID_IDENTITY (-4)
#define TETRA_SDS_TRUNCATED (-7)
#define TETRA_SDS_UNSUPPORTED (-9)
#define TETRA_SDS_CAPACITY (-10)
#define TETRA_SDS_QUEUE_FULL (-11)
#define TETRA_SDS_PANIC (-16)
#define TETRA_SDS_INVALID_VALUE (-301)
#define TETRA_SDS_INVALID_WIDTH (-302)
#define TETRA_SDS_DOWNLINK 1u
#define TETRA_SDS_UNITDATA 2u
#define TETRA_SDS_STATUS 3u
#define TETRA_SDS_MESSAGE 4u
#define TETRA_SDS_REPORT 5u
#define TETRA_SDS_CANCEL 6u
#define TETRA_SDS_UPLINK 1u
#define TETRA_SDS_UNITDATA_INDICATION 2u
#define TETRA_SDS_STATUS_INDICATION 3u
#define TETRA_SDS_MESSAGE_INDICATION 4u
#define TETRA_SDS_REPORT_INDICATION 5u
#define TETRA_SDS_GROUP 1u
#define TETRA_SDS_SHORT 2u
#if defined(__cplusplus)
#define TETRA_SDS_ALIGN(n) alignas(n)
#define TETRA_SDS_ASSERT(x) static_assert(x,#x)
#define TETRA_SDS_ALIGNOF(t) alignof(t)
extern "C" {
#elif defined(_MSC_VER)
#define TETRA_SDS_ALIGN(n) __declspec(align(n))
#define TETRA_SDS_ASSERT(x) _Static_assert(x,#x)
#define TETRA_SDS_ALIGNOF(t) __alignof(t)
#else
#define TETRA_SDS_ALIGN(n) _Alignas(n)
#define TETRA_SDS_ASSERT(x) _Static_assert(x,#x)
#define TETRA_SDS_ALIGNOF(t) _Alignof(t)
#endif
struct tetra_sds_storage { TETRA_SDS_ALIGN(8) uint8_t bytes[TETRA_SDS_STORAGE_SIZE]; };
struct tetra_sds_config { uint32_t size,abi_version; };
struct tetra_sds_event {
    uint32_t size,abi_version,kind,ssi,ssi_type,flags,data_type;
    uint16_t status,pdu_bits;
    uint8_t protocol_id,reference,report_request,delivery_status;
    uint8_t pdu[TETRA_SDS_PDU_BYTES];
};
struct tetra_sds_action {
    uint32_t size,abi_version,kind,ssi,ssi_type,flags,data_type;
    uint16_t status,pdu_bits;
    uint8_t protocol_id,reference,report_request,delivery_status;
    uint8_t pdu[TETRA_SDS_PDU_BYTES];
};
struct tetra_sds_status {
    uint32_t size,abi_version;
    uint16_t outstanding,actions;
    uint8_t present[32];
    uint32_t destination[256];
};
/* All pointers live/aligned/sufficiently sized and disjoint from the entire
 * storage budget; serialize calls per instance. init accepts uninitialized
 * storage; reset requires a live state. Both initialize directly in place.
 * No pointers are retained. Storage is not serialized/persistent data.
 * Zero-initialize Config/Event, then set size/version. For poll/status initialize
 * only output size/version; remaining bytes may be uninitialized. Errors/EMPTY
 * preserve output; poll errors never consume actions. Rejected events preserve
 * protocol state/actions. QUEUE_FULL is retryable after draining actions.
 * ssi_type: 2 ISSI,3 GSSI for transmitted destinations. Downlink uses GROUP flag
 * to distinguish individual/group delivery. Other flags are invalid.
 * data_type for UNITDATA: 0=16-bit,1=32-bit,2=64-bit,3=Type4; pdu_bits must match
 * fixed types, Type4 <=2047 bits. All packed bytes are MSB-first, including the
 * 64-bit user data: no C uint64_t/alignment assumptions are required.
 * DOWNLINK pdu is a complete CMCE D-SDS-DATA/D-STATUS. MESSAGE pdu is opaque
 * application data (<=2023 bits); protocol_id, reference and report_request0..3
 * form the SDS-TL header. REPORT uses destination ssi, reference/delivery_status.
 * CANCEL uses reference and returns EMPTY if it was not tracked.
 * Actions: UPLINK has complete CMCE PDU plus ssi/type; UNITDATA_INDICATION
 * contains raw user data/type; MESSAGE_INDICATION contains stripped application
 * data and SDS-TL fields. STATUS uses status; REPORT uses delivery_status and
 * optionally SHORT. Group indications set GROUP. Other output fields are zero.
 * Four pending actions, all 256 outstanding reference values supported.
 * Status present[reference/8] bit(reference%8) marks destination[reference].
 * Tracking intentionally retains historical reference-only correlation; reuse
 * replaces a destination and any report for that reference clears tracking.
 * No report timeout/retransmission behavior is invented. */
uint32_t tetra_sds_abi_version(void);
uint32_t tetra_sds_core_size(void);
uint32_t tetra_sds_core_align(void);
int32_t tetra_sds_init(void*,uint32_t,const struct tetra_sds_config*);
int32_t tetra_sds_reset(void*,const struct tetra_sds_config*);
int32_t tetra_sds_push_event(void*,const struct tetra_sds_event*);
int32_t tetra_sds_poll_action(void*,struct tetra_sds_action*);
int32_t tetra_sds_get_status(const void*,struct tetra_sds_status*);
TETRA_SDS_ASSERT(sizeof(struct tetra_sds_storage)==TETRA_SDS_STORAGE_SIZE);
TETRA_SDS_ASSERT(TETRA_SDS_ALIGNOF(struct tetra_sds_storage)==TETRA_SDS_STORAGE_ALIGN);
TETRA_SDS_ASSERT(sizeof(struct tetra_sds_config)==8);
TETRA_SDS_ASSERT(sizeof(struct tetra_sds_event)==372);
TETRA_SDS_ASSERT(sizeof(struct tetra_sds_action)==372);
TETRA_SDS_ASSERT(sizeof(struct tetra_sds_status)==1068);
TETRA_SDS_ASSERT(TETRA_SDS_ALIGNOF(struct tetra_sds_event)==4);
TETRA_SDS_ASSERT(TETRA_SDS_ALIGNOF(struct tetra_sds_action)==4);
TETRA_SDS_ASSERT(TETRA_SDS_ALIGNOF(struct tetra_sds_status)==4);
TETRA_SDS_ASSERT(offsetof(struct tetra_sds_event,pdu)==36);
TETRA_SDS_ASSERT(offsetof(struct tetra_sds_action,pdu)==36);
#ifdef __cplusplus
}
#endif
#endif
