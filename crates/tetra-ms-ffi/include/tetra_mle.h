#ifndef TETRA_MLE_H
#define TETRA_MLE_H
#include <stdint.h>
#include <stddef.h>

#define TETRA_MLE_ABI_VERSION 1u
#define TETRA_MLE_STORAGE_SIZE 8192u
#define TETRA_MLE_STORAGE_ALIGN 4u
#define TETRA_MLE_NETWORK_CAPACITY 64u
#define TETRA_MLE_CANDIDATE_CAPACITY 256u
#define TETRA_MLE_GROUP_CAPACITY 64u
#define TETRA_MLE_PDU_BYTES 512u

#define TETRA_MLE_OK 0
#define TETRA_MLE_EMPTY 1
#define TETRA_MLE_ARGUMENT (-1)
#define TETRA_MLE_ABI_MISMATCH (-2)
#define TETRA_MLE_UNINITIALIZED (-3)
#define TETRA_MLE_TRUNCATED (-7)
#define TETRA_MLE_UNSUPPORTED (-9)
#define TETRA_MLE_CAPACITY (-10)
#define TETRA_MLE_QUEUE_FULL (-11)
#define TETRA_MLE_PANIC (-16)
/* Retain the offending three-bit discriminator or internal PDU type. */
#define TETRA_MLE_INVALID_PROTOCOL(value) (-32 - (int32_t)(value))
#define TETRA_MLE_UNSUPPORTED_PDU(value) (-48 - (int32_t)(value))
#define TETRA_MLE_UNSUPPORTED_MANAGEMENT (-64)

#define TETRA_MLE_DOWNLINK 1u
#define TETRA_MLE_UPLINK 2u
#define TETRA_MLE_SYNC 3u
#define TETRA_MLE_SYSINFO 4u
#define TETRA_MLE_MONITOR 5u
#define TETRA_MLE_DWELL 6u
#define TETRA_MLE_SELECTION_MODE 7u
#define TETRA_MLE_SURVEY 8u
#define TETRA_MLE_CAMP 9u
#define TETRA_MLE_IDENTITIES 10u
#define TETRA_MLE_U_PLANE 11u

#define TETRA_MLE_TUNE 1u
#define TETRA_MLE_CONFIGURE_NETWORK 2u
#define TETRA_MLE_CONFIGURE_IDENTITIES 3u
#define TETRA_MLE_CONFIGURE_U_PLANE 4u
#define TETRA_MLE_ACTIVATE 5u
#define TETRA_MLE_BREAK 6u
#define TETRA_MLE_REOPEN 7u
#define TETRA_MLE_RSSI 8u
#define TETRA_MLE_SCAN_RESULT 9u
#define TETRA_MLE_SCAN_COMPLETE 10u
#define TETRA_MLE_DELIVER 11u
#define TETRA_MLE_TRANSMIT 12u

#define TETRA_MLE_MM 1u
#define TETRA_MLE_CMCE 2u
#define TETRA_MLE_SNDCP 4u
#define TETRA_MLE_LLC 3u
#define TETRA_MLE_MAC 5u

#define TETRA_MLE_ENABLED 1u
#define TETRA_MLE_RSSI_PRESENT 2u
#define TETRA_MLE_PRIORITY_PRESENT 4u
#define TETRA_MLE_EMERGENCY 8u
#define TETRA_MLE_STEALING_PERMISSION 16u
#define TETRA_MLE_TX_GRANT 32u

#define TETRA_MLE_A_LOCATION_PRESENT (1u << 0)
#define TETRA_MLE_A_COLOUR_PRESENT (1u << 1)
#define TETRA_MLE_A_RSSI_PRESENT (1u << 2)
#define TETRA_MLE_A_REGISTRATION_PRESENT (1u << 3)
#define TETRA_MLE_A_REGISTRATION_REQUIRED (1u << 4)
#define TETRA_MLE_A_LATE_ENTRY (1u << 5)
#define TETRA_MLE_A_SYSTEM_WIDE_SERVICES (1u << 6)
#define TETRA_MLE_A_SWITCH_U_PLANE (1u << 7)
#define TETRA_MLE_A_TX_GRANT (1u << 8)
#define TETRA_MLE_A_PRIORITY_PRESENT (1u << 9)
#define TETRA_MLE_A_EMERGENCY (1u << 10)
#define TETRA_MLE_A_STEALING_PERMISSION (1u << 11)
#define TETRA_MLE_A_FCS (1u << 12)
#define TETRA_MLE_A_ENCRYPTION_PRESENT (1u << 13)
#define TETRA_MLE_A_REPEATS_PRESENT (1u << 14)
#define TETRA_MLE_A_REPEATS (1u << 15)
#define TETRA_MLE_A_DATA_CLASS_PRESENT (1u << 16)
#define TETRA_MLE_A_GRACEFUL_PRESENT (1u << 17)
#define TETRA_MLE_A_CHANNEL_RESPONSE (1u << 18)
#define TETRA_MLE_A_CHANNEL_HANDLE_PRESENT (1u << 19)

#define TETRA_MLE_S_SERVING (1u << 0)
#define TETRA_MLE_S_CONFIRMED (1u << 1)
#define TETRA_MLE_S_OUT_OF_SERVICE (1u << 2)
#define TETRA_MLE_S_RSSI_PRESENT (1u << 3)
#define TETRA_MLE_S_SCANNING (1u << 4)
#define TETRA_MLE_S_MANUAL (1u << 5)
#define TETRA_MLE_S_CAMP_ARMED (1u << 6)
#define TETRA_MLE_S_FORCE_REGISTER (1u << 7)
#define TETRA_MLE_S_SURVEY (1u << 8)
#define TETRA_MLE_S_PENDING (1u << 9)

#define TETRA_MLE_CELL_LOCATION (1u << 0)
#define TETRA_MLE_CELL_SUBSCRIBER (1u << 1)
#define TETRA_MLE_CELL_REGISTRATION (1u << 2)
#define TETRA_MLE_CELL_REGISTRATION_REQUIRED (1u << 3)
#define TETRA_MLE_CELL_SYSTEM_WIDE (1u << 4)
#define TETRA_MLE_CELL_SYSTEM_WIDE_SUPPORTED (1u << 5)
#define TETRA_MLE_CELL_LATE_ENTRY (1u << 6)
#define TETRA_MLE_CELL_COLOUR (1u << 7)
#define TETRA_MLE_CELL_RSSI (1u << 8)

#if defined(__cplusplus)
#define TETRA_MLE_ALIGN(n) alignas(n)
#define TETRA_MLE_ASSERT(x) static_assert(x, #x)
#define TETRA_MLE_ALIGNOF(t) alignof(t)
extern "C" {
#elif defined(_MSC_VER)
#define TETRA_MLE_ALIGN(n) __declspec(align(n))
#define TETRA_MLE_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MLE_ALIGNOF(t) __alignof(t)
#else
#define TETRA_MLE_ALIGN(n) _Alignas(n)
#define TETRA_MLE_ASSERT(x) _Static_assert(x, #x)
#define TETRA_MLE_ALIGNOF(t) _Alignof(t)
#endif

struct tetra_mle_storage { TETRA_MLE_ALIGN(4) uint8_t bytes[TETRA_MLE_STORAGE_SIZE]; };
struct tetra_mle_network { uint16_t mcc, mnc; };
struct tetra_mle_config {
    uint32_t size, abi_version;
    uint16_t home_mcc, home_mnc, subscriber_class, network_count;
    struct tetra_mle_network networks[TETRA_MLE_NETWORK_CAPACITY];
};
struct tetra_mle_event {
    uint32_t size, abi_version, kind, source, endpoint_id, link_id;
    uint32_t carrier_hz, issi, flags, priority, rssi_dbfs_bits;
    uint16_t pdu_bits, value_count;
    uint8_t pdu[TETRA_MLE_PDU_BYTES];
    uint32_t values[TETRA_MLE_CANDIDATE_CAPACITY];
};
struct tetra_mle_action {
    uint32_t size, abi_version, kind, target, protocol, flags;
    uint32_t carrier_hz, issi, endpoint_id, link_id, mcc, mnc, location_area, colour_code;
    uint32_t rssi_dbfs_bits, found, scanned, priority;
    int32_t subscriber_class, request_handle, air_interface_encryption, data_class;
    int32_t graceful_degradation, channel_change_handle, cell_type, permitted_services;
    uint32_t handle;
    uint16_t pdu_bits, group_count;
    uint8_t pdu[TETRA_MLE_PDU_BYTES];
    uint32_t groups[TETRA_MLE_GROUP_CAPACITY];
};
struct tetra_mle_cell {
    uint32_t flags, carrier_hz, mcc, mnc, neighbor_cell_broadcast, cell_load_ca;
    uint32_t location_area, subscriber_class, colour_code, rssi_dbfs_bits;
};
struct tetra_mle_status {
    uint32_t size, abi_version, flags, rssi_dbfs_bits, survey_found;
    uint16_t queued_actions, scan_index, survey_index, survey_monitor_ticks, survey_sync_repeats, reserved;
    struct tetra_mle_cell serving_cell, survey_pending;
};

/* Zero-initialize config/event records, then fill size and abi_version. Before
 * poll/status, initialize those output header fields too. All pointers must be
 * live, aligned, sufficiently sized and disjoint from the entire storage budget.
 * The application must serialize calls on each storage object; no pointer is
 * retained. init accepts uninitialized storage, reset requires a live instance.
 * Both initialize the core in place; reset discards all queued actions/state.
 * configure replaces the caller's policy without discarding protocol state or
 * queued actions, matching a new MleConfig snapshot on the native Rust surface.
 * Never modify the opaque storage bytes directly. poll/status only require
 * their output size/version fields initialized; the remaining output bytes may
 * be uninitialized and are completely initialized on success.
 * Opaque storage is live instance memory, not a persisted or wire format.
 *
 * Config subscriber_class: 0 = monitor/no restriction, otherwise 1..16.
 * Home MCC/MNC and each of network_count entries must fit 10/14 bits.
 * Events: ENABLED means monitor available / manual selection / survey start /
 * camp register / U-plane switch. U-plane additionally accepts TX_GRANT.
 * Monitor RSSI_PRESENT interprets rssi_dbfs_bits as IEEE-754 binary32 bits.
 * Uplink source is MM or CMCE; MM accepts PRIORITY_PRESENT and EMERGENCY;
 * CMCE accepts STEALING_PERMISSION. Priority is an 8-bit value when present.
 * Dwell, Survey and Camp consume value_count candidate carriers. Identities
 * consumes value_count attached GSSIs (maximum 64). Borrowed inputs are consumed
 * during the call. PDU bytes are packed MSB-first; pdu_bits excludes padding.
 * Downlink includes its three-bit MLE discriminator; uplink is the L3 payload
 * without that discriminator. SYNC is its 29-bit broadcast body (MCC10, MNC14,
 * neighbour2, load2, late-entry1). SYSINFO is its 42-bit body (LA14,
 * subscriber-class bitmap16, BS-service-details12), without an MLE discriminator.
 * Raw receive PDUs may occupy 4096 bits; uplink including its discriminator is
 * limited to 2688 bits. The caller owns PHY heartbeat and dwell timing.
 * Received addresses/endpoint metadata and application tags stay caller-owned;
 * drain after each event to correlate them with its actions. Input endpoint_id
 * and link_id are uplink fields. Action target is MM/CMCE/LLC/MAC.
 * Unknown event kinds return UNSUPPORTED. Unknown flag bits return ARGUMENT.
 * Core unknown/internal/management protocols retain distinct error outcomes.
 * Drain actions after successful events. QUEUE_FULL is retryable backpressure.
 * All rejected events leave protocol state/actions unchanged. poll EMPTY and
 * all poll/status errors leave output untouched and consume no queued action.
 * Fields not applicable to an output action are zero. Optional values are
 * indicated by the action/cell/status flags, never by native C bool/enum types.
 * Deliver payload excludes its discriminator; Transmit PDU includes it.
 * Poll Tune/ConfigureNetwork/ConfigureIdentities/ConfigureUPlane targets MAC;
 * Transmit targets LLC; other targets are MM/CMCE as explicitly returned.
 */
uint32_t tetra_mle_abi_version(void);
uint32_t tetra_mle_core_size(void);
uint32_t tetra_mle_core_align(void);
int32_t tetra_mle_init(void *storage, uint32_t storage_size, const struct tetra_mle_config *config);
int32_t tetra_mle_reset(void *storage, const struct tetra_mle_config *config);
int32_t tetra_mle_configure(void *storage, const struct tetra_mle_config *config);
int32_t tetra_mle_push_event(void *storage, const struct tetra_mle_event *event);
int32_t tetra_mle_poll_action(void *storage, struct tetra_mle_action *action);
int32_t tetra_mle_get_status(const void *storage, struct tetra_mle_status *status);

TETRA_MLE_ASSERT(sizeof(struct tetra_mle_storage) == TETRA_MLE_STORAGE_SIZE);
TETRA_MLE_ASSERT(TETRA_MLE_ALIGNOF(struct tetra_mle_storage) == TETRA_MLE_STORAGE_ALIGN);
TETRA_MLE_ASSERT(sizeof(struct tetra_mle_network) == 4);
TETRA_MLE_ASSERT(sizeof(struct tetra_mle_config) == 272);
TETRA_MLE_ASSERT(sizeof(struct tetra_mle_event) == 1584);
TETRA_MLE_ASSERT(sizeof(struct tetra_mle_action) == 880);
TETRA_MLE_ASSERT(sizeof(struct tetra_mle_cell) == 40);
TETRA_MLE_ASSERT(sizeof(struct tetra_mle_status) == 112);
TETRA_MLE_ASSERT(TETRA_MLE_ALIGNOF(struct tetra_mle_event) == 4);
TETRA_MLE_ASSERT(TETRA_MLE_ALIGNOF(struct tetra_mle_action) == 4);
TETRA_MLE_ASSERT(offsetof(struct tetra_mle_event, pdu) == 48);
TETRA_MLE_ASSERT(offsetof(struct tetra_mle_event, values) == 560);
TETRA_MLE_ASSERT(offsetof(struct tetra_mle_action, pdu) == 112);
TETRA_MLE_ASSERT(offsetof(struct tetra_mle_action, groups) == 624);
TETRA_MLE_ASSERT(offsetof(struct tetra_mle_status, serving_cell) == 32);

#if defined(__cplusplus)
}
#endif
#endif
