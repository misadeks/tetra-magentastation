#include "tetra_mle.h"

#define CHECK(expression) do { if (!(expression)) return __LINE__; } while (0)

static void clear_bytes(void *memory, size_t count) {
    uint8_t *bytes = (uint8_t *)memory;
    while (count != 0) {
        *bytes++ = 0;
        --count;
    }
}

static void event_init(struct tetra_mle_event *event, uint32_t kind) {
    clear_bytes(event, sizeof(*event));
    event->size = (uint32_t)sizeof(*event);
    event->abi_version = TETRA_MLE_ABI_VERSION;
    event->kind = kind;
}

static void bits(struct tetra_mle_event *event, uint32_t offset, uint32_t width, uint32_t value) {
    uint32_t i;
    for (i = 0; i < width; ++i) {
        uint32_t position = offset + i;
        uint8_t mask = (uint8_t)(1u << (7u - position % 8u));
        if ((value >> (width - i - 1u)) & 1u) event->pdu[position / 8u] |= mask;
        else event->pdu[position / 8u] &= (uint8_t)~mask;
    }
}

#ifdef __cplusplus
extern "C"
#endif
int32_t tetra_mle_c_smoke(void) {
    static struct tetra_mle_storage storage;
    struct tetra_mle_config config;
    struct tetra_mle_event event;
    struct tetra_mle_action action;
    struct tetra_mle_status status;
    uint32_t i;
    clear_bytes(&config, sizeof(config));
    clear_bytes(&action, sizeof(action));
    clear_bytes(&status, sizeof(status));
    config.size = (uint32_t)sizeof(config);
    config.abi_version = TETRA_MLE_ABI_VERSION;
    config.home_mcc = 901;
    config.home_mnc = 9999;
    config.subscriber_class = 1;
    action.size = (uint32_t)sizeof(action);
    action.abi_version = TETRA_MLE_ABI_VERSION;
    status.size = (uint32_t)sizeof(status);
    status.abi_version = TETRA_MLE_ABI_VERSION;
    CHECK(tetra_mle_abi_version() == TETRA_MLE_ABI_VERSION);
    CHECK(tetra_mle_core_size() == sizeof(storage));
    CHECK(tetra_mle_core_align() == TETRA_MLE_ALIGNOF(struct tetra_mle_storage));
    CHECK(tetra_mle_init(NULL, sizeof(storage), &config) == TETRA_MLE_ARGUMENT);
    CHECK(tetra_mle_init(&storage, sizeof(storage) - 1u, &config) == TETRA_MLE_CAPACITY);
    CHECK(tetra_mle_init(&storage, sizeof(storage), &config) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_EMPTY);

    event_init(&event, TETRA_MLE_SURVEY);
    event.flags = TETRA_MLE_ENABLED;
    event.value_count = 2;
    event.values[0] = 390000000;
    event.values[1] = 396000000;
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_TUNE && action.target == TETRA_MLE_MAC);
    CHECK(action.carrier_hz == 390000000);
    event_init(&event, TETRA_MLE_SYNC);
    event.pdu_bits = 29;
    bits(&event, 0, 10, 901);
    bits(&event, 10, 14, 9999);
    bits(&event, 28, 1, 1);
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_get_status(&storage, &status) == TETRA_MLE_OK);
    CHECK((status.flags & (TETRA_MLE_S_SURVEY | TETRA_MLE_S_PENDING)) ==
        (TETRA_MLE_S_SURVEY | TETRA_MLE_S_PENDING));
    CHECK(status.survey_pending.mcc == 901);
    event_init(&event, TETRA_MLE_SYSINFO);
    event.pdu_bits = 42;
    bits(&event, 0, 14, 9);
    bits(&event, 14, 16, 1);
    bits(&event, 30, 12, 0x840);
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_SCAN_RESULT && action.location_area == 9);
    CHECK((action.flags & (TETRA_MLE_A_LOCATION_PRESENT | TETRA_MLE_A_REGISTRATION_PRESENT |
        TETRA_MLE_A_REGISTRATION_REQUIRED | TETRA_MLE_A_LATE_ENTRY)) ==
        (TETRA_MLE_A_LOCATION_PRESENT | TETRA_MLE_A_REGISTRATION_PRESENT |
        TETRA_MLE_A_REGISTRATION_REQUIRED | TETRA_MLE_A_LATE_ENTRY));
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_TUNE && action.carrier_hz == 396000000);
    event_init(&event, TETRA_MLE_DWELL);
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_SCAN_COMPLETE && action.found == 1 && action.scanned == 2);

    event_init(&event, TETRA_MLE_SELECTION_MODE);
    event.flags = TETRA_MLE_ENABLED;
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    event_init(&event, TETRA_MLE_CAMP);
    event.carrier_hz = 396000000;
    event.flags = TETRA_MLE_ENABLED;
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_TUNE);
    event_init(&event, TETRA_MLE_SYNC);
    event.pdu_bits = 29;
    bits(&event, 0, 10, 901);
    bits(&event, 10, 14, 9999);
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_CONFIGURE_NETWORK && action.mcc == 901 && action.mnc == 9999);
    event_init(&event, TETRA_MLE_SYSINFO);
    event.pdu_bits = 42;
    bits(&event, 0, 14, 9);
    bits(&event, 14, 16, 1);
    bits(&event, 30, 12, 0x840);
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_ACTIVATE && action.target == TETRA_MLE_MM);
    CHECK((action.flags & TETRA_MLE_A_SYSTEM_WIDE_SERVICES) != 0);

    event_init(&event, TETRA_MLE_MONITOR);
    event.flags = TETRA_MLE_RSSI_PRESENT;
    event.rssi_dbfs_bits = 0xc1480000u; /* -12.5 binary32 */
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_RSSI && action.rssi_dbfs_bits == 0xc1480000u);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_BREAK && action.target == TETRA_MLE_CMCE);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_BREAK && action.target == TETRA_MLE_MM);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_RSSI && (action.flags & TETRA_MLE_A_RSSI_PRESENT) == 0);

    CHECK(tetra_mle_reset(&storage, &config) == TETRA_MLE_OK);
    event_init(&event, TETRA_MLE_UPLINK);
    event.source = TETRA_MLE_MM;
    event.flags = TETRA_MLE_PRIORITY_PRESENT | TETRA_MLE_EMERGENCY;
    event.priority = 6;
    event.pdu_bits = 5;
    event.pdu[0] = 0x90; /* payload 10010 */
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    action.abi_version = 0;
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_ABI_MISMATCH);
    action.abi_version = TETRA_MLE_ABI_VERSION;
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_TRANSMIT && action.target == TETRA_MLE_LLC);
    CHECK(action.pdu_bits == 8 && action.pdu[0] == 0x32 && action.priority == 6);
    CHECK((action.flags & (TETRA_MLE_A_PRIORITY_PRESENT | TETRA_MLE_A_EMERGENCY)) ==
        (TETRA_MLE_A_PRIORITY_PRESENT | TETRA_MLE_A_EMERGENCY));
    event_init(&event, TETRA_MLE_DOWNLINK);
    event.pdu_bits = 8;
    event.pdu[0] = 0x32;
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_poll_action(&storage, &action) == TETRA_MLE_OK);
    CHECK(action.kind == TETRA_MLE_DELIVER && action.protocol == TETRA_MLE_MM);
    CHECK(action.pdu_bits == 5 && action.pdu[0] == 0x90);
    event.pdu_bits = 3;
    event.pdu[0] = 0;
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_INVALID_PROTOCOL(0));
    event.pdu_bits = 6;
    event.pdu[0] = 0xa8; /* protocol MLE, type 2 */
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_UNSUPPORTED_PDU(2));
    event.pdu_bits = 3;
    event.pdu[0] = 0xc0; /* management */
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_UNSUPPORTED_MANAGEMENT);
    event.kind = 99;
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_UNSUPPORTED);
    CHECK(tetra_mle_poll_action(&storage, (struct tetra_mle_action *)(void *)storage.bytes) == TETRA_MLE_ARGUMENT);
    CHECK(tetra_mle_push_event(&storage, (const struct tetra_mle_event *)(const void *)storage.bytes) == TETRA_MLE_ARGUMENT);

    event_init(&event, TETRA_MLE_U_PLANE);
    for (i = 0; i < 8; ++i) CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_OK);
    CHECK(tetra_mle_push_event(&storage, &event) == TETRA_MLE_QUEUE_FULL);
    CHECK(tetra_mle_get_status(&storage, &status) == TETRA_MLE_OK && status.queued_actions == 8);
    config.network_count = 65;
    CHECK(tetra_mle_configure(&storage, &config) == TETRA_MLE_CAPACITY);
    CHECK(tetra_mle_reset(&storage, &config) == TETRA_MLE_CAPACITY);
    CHECK(tetra_mle_get_status(&storage, &status) == TETRA_MLE_OK && status.queued_actions == 8);
    config.network_count = 0;
    CHECK(tetra_mle_configure(&storage, &config) == TETRA_MLE_OK);
    CHECK(tetra_mle_get_status(&storage, &status) == TETRA_MLE_OK && status.queued_actions == 8);
    CHECK(tetra_mle_reset(&storage, &config) == TETRA_MLE_OK);
    CHECK(tetra_mle_get_status(&storage, &status) == TETRA_MLE_OK);
    CHECK(status.flags == 0 && status.queued_actions == 0);
    return 0;
}

#ifdef TETRA_MLE_SMOKE_MAIN
int main(void) {
    return tetra_mle_c_smoke();
}
#endif
