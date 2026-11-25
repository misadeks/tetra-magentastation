#include "tetra_mac.h"

#define CHECK(expression) do { if (!(expression)) return __LINE__; } while (0)

static void mac_zero(void *memory, uint32_t count) {
    uint8_t *bytes = (uint8_t *)memory;
    uint32_t i;
    for (i = 0; i < count; ++i) bytes[i] = 0;
}

static void mac_put(uint8_t *bytes, uint32_t offset, uint32_t width, uint32_t value) {
    uint32_t i;
    for (i = 0; i < width; ++i) {
        uint32_t pos = offset + i;
        uint8_t mask = (uint8_t)(1u << (7u - pos % 8u));
        if ((value >> (width - i - 1u)) & 1u) bytes[pos / 8u] |= mask;
        else bytes[pos / 8u] &= (uint8_t)~mask;
    }
}

#if defined(__cplusplus)
extern "C"
#endif
int32_t tetra_ms_mac_c_smoke(void) {
    static struct tetra_ms_mac_storage storage;
    static struct tetra_ms_mac_config config;
    static struct tetra_ms_mac_event event;
    static struct tetra_ms_mac_action action;
    static struct tetra_ms_mac_status status;
    static struct tetra_ms_mac_entropy entropy;
    mac_zero(&config, sizeof(config)); mac_zero(&event, sizeof(event));
    mac_zero(&action, sizeof(action)); mac_zero(&status, sizeof(status)); mac_zero(&entropy, sizeof(entropy));
    config.size = sizeof(config); config.abi_version = TETRA_MS_MAC_ABI_VERSION;
    config.own_issi = 1000001u;
    config.flags = TETRA_MS_MAC_C_FILTERED | TETRA_MS_MAC_C_INDIVIDUAL
        | TETRA_MS_MAC_C_SCRAMBLING | TETRA_MS_MAC_C_CARRIER;
    config.scrambling_code = 0x12345678u; config.serving_carrier = 1593u;
    event.size = sizeof(event); event.abi_version = TETRA_MS_MAC_ABI_VERSION;
    action.size = sizeof(action); action.abi_version = TETRA_MS_MAC_ABI_VERSION;
    status.size = sizeof(status); status.abi_version = TETRA_MS_MAC_ABI_VERSION;
    entropy.size = sizeof(entropy); entropy.abi_version = TETRA_MS_MAC_ABI_VERSION;
    CHECK(tetra_ms_mac_abi_version() == TETRA_MS_MAC_ABI_VERSION);
    CHECK(tetra_ms_mac_storage_size() == sizeof(storage));
    CHECK(tetra_ms_mac_storage_align() == TETRA_MS_MAC_ALIGNOF(struct tetra_ms_mac_storage));
    CHECK(tetra_ms_mac_init(NULL, sizeof(storage), &config) == TETRA_MS_MAC_ARGUMENT);
    CHECK(tetra_ms_mac_init(&storage, sizeof(storage) - 1u, &config) == TETRA_MS_MAC_CAPACITY);
    CHECK(tetra_ms_mac_init(&storage, sizeof(storage), &config) == TETRA_MS_MAC_OK);
    event.kind = TETRA_MS_MAC_SET_PARAMS; event.source = 1u;
    event.params.imm = 15u; event.params.wt = 6u; event.params.nu = 4u;
    CHECK(tetra_ms_mac_push_event(&storage, &event) == TETRA_MS_MAC_OK);
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_OK);
    CHECK(action.kind == TETRA_MS_MAC_ACCESS_PARAMS && (action.flags & TETRA_MS_MAC_F_APPLIED));
    event.kind = TETRA_MS_MAC_SUBMIT;
    event.flags = TETRA_MS_MAC_E_RECEIPT | TETRA_MS_MAC_E_PRIORITY;
    event.receipt = 7u; event.priority = 5u; event.pdu_bits = 20u;
    event.pdu[0] = 0xaau; event.pdu[1] = 0xbbu; event.pdu[2] = 0xc0u;
    CHECK(tetra_ms_mac_push_event(&storage, &event) == TETRA_MS_MAC_OK);
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_OK);
    CHECK(action.kind == TETRA_MS_MAC_SUBMITTED && action.route == 0u);
    event.kind = TETRA_MS_MAC_AACH; event.flags = 0u; event.pdu_bits = 14u;
    event.pdu[0] = 0x42u; event.pdu[1] = 0x08u;
    CHECK(tetra_ms_mac_entropy_needed(&storage, &event, &entropy) == TETRA_MS_MAC_OK);
    CHECK(entropy.kind == TETRA_MS_MAC_ENTROPY_SUBSLOT && entropy.maximum == 2u);
    CHECK(tetra_ms_mac_get_status(&storage, &status) == TETRA_MS_MAC_OK);
    CHECK(status.ra_phase == TETRA_MS_MAC_RA_IDLE);
    event.choice_subslot = 3u;
    CHECK(tetra_ms_mac_push_event(&storage, &event) == TETRA_MS_MAC_RA_INVALID_ENTROPY);
    CHECK(tetra_ms_mac_get_status(&storage, &status) == TETRA_MS_MAC_OK);
    CHECK(status.ra_phase == TETRA_MS_MAC_RA_IDLE && status.actions == 0u);
    event.choice_subslot = 2u;
    CHECK(tetra_ms_mac_push_event(&storage, &event) == TETRA_MS_MAC_OK);
    action.size = 0u;
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_ABI_MISMATCH);
    action.size = sizeof(action);
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_OK);
    CHECK(action.kind == TETRA_MS_MAC_AACH_INFO);
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_OK);
    CHECK(action.kind == TETRA_MS_MAC_RA_TRANSMIT && action.subslot == 2u);
    CHECK(action.now == 2u && action.channel == TETRA_MS_MAC_SCH_HU && action.pdu_bits == 92u);
    CHECK((action.flags & TETRA_MS_MAC_F_MAC_BLOCK) != 0u);
    event.kind = TETRA_MS_MAC_DOWNLINK; event.choice_subslot = 0u; event.pdu_bits = 43u;
    mac_zero(event.pdu, sizeof(event.pdu));
    mac_put(event.pdu, 6u, 1u, 1u); mac_put(event.pdu, 7u, 6u, 2u);
    mac_put(event.pdu, 13u, 3u, 1u); mac_put(event.pdu, 16u, 24u, 1000001u);
    CHECK(tetra_ms_mac_push_event(&storage, &event) == TETRA_MS_MAC_OK);
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_OK);
    CHECK(action.kind == TETRA_MS_MAC_RA_SUCCEEDED && action.receipt == 7u);
    CHECK((action.flags & TETRA_MS_MAC_F_RECEIPT) != 0u);
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_OK);
    CHECK(action.kind == TETRA_MS_MAC_RESOURCE && action.route == 3u);
    CHECK(action.header_bits == 43u && action.payload_end == 43u && action.next_bits == 16u);
    CHECK(tetra_ms_mac_poll_action(&storage, &action) == TETRA_MS_MAC_EMPTY);
    CHECK(action.kind == TETRA_MS_MAC_RESOURCE);
    config.group_count = 257u;
    CHECK(tetra_ms_mac_reset(&storage, &config) == TETRA_MS_MAC_CAPACITY);
    CHECK(tetra_ms_mac_get_status(&storage, &status) == TETRA_MS_MAC_OK);
    CHECK(status.ra_phase == TETRA_MS_MAC_RA_IDLE && status.pending_bits == 0u);
    config.group_count = 0u;
    CHECK(tetra_ms_mac_reset(&storage, &config) == TETRA_MS_MAC_OK);
    CHECK(tetra_ms_mac_get_status(&storage, &status) == TETRA_MS_MAC_OK);
    CHECK(status.actions == 0u && status.param_mask == 0u);
    return 0;
}

#ifdef TETRA_MS_MAC_SMOKE_MAIN
int main(void) { return tetra_ms_mac_c_smoke(); }
#endif
