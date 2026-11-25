#include "tetra_mm.h"

#define CHECK(expression) do { if (!(expression)) return __LINE__; } while (0)

#if defined(__cplusplus)
#define MM_ZERO {}
extern "C"
#else
#define MM_ZERO {0}
#endif
int32_t tetra_ms_mm_c_smoke(void) {
    struct tetra_ms_mm_storage storage = MM_ZERO;
    struct tetra_ms_mm_config config = MM_ZERO;
    struct tetra_ms_mm_event event = MM_ZERO;
    struct tetra_ms_mm_action action = MM_ZERO;
    struct tetra_ms_mm_status status = MM_ZERO;
    config.size = sizeof(config);
    config.abi_version = TETRA_MS_MM_ABI_VERSION;
    config.own_issi = 1000001u;
    config.mcc = 234u;
    config.mnc = 5678u;
    config.initial_la = 100u;
    event.size = sizeof(event);
    event.abi_version = TETRA_MS_MM_ABI_VERSION;
    action.size = sizeof(action);
    action.abi_version = TETRA_MS_MM_ABI_VERSION;
    status.size = sizeof(status);
    status.abi_version = TETRA_MS_MM_ABI_VERSION;
    CHECK(tetra_ms_mm_abi_version() == TETRA_MS_MM_ABI_VERSION);
    CHECK(tetra_ms_mm_storage_size() == sizeof(storage));
    CHECK(tetra_ms_mm_storage_align() == TETRA_MS_MM_ALIGNOF(struct tetra_ms_mm_storage));
    CHECK(tetra_ms_mm_get_status(&storage, &status) == TETRA_MS_MM_UNINITIALIZED);
    CHECK(tetra_ms_mm_init(NULL, sizeof(storage), &config) == TETRA_MS_MM_ARGUMENT);
    CHECK(tetra_ms_mm_init(&storage, sizeof(storage) - 1u, &config) == TETRA_MS_MM_CAPACITY);
    config.group_count = TETRA_MS_MM_MAX_GROUPS + 1u;
    CHECK(tetra_ms_mm_init(&storage, sizeof(storage), &config) == TETRA_MS_MM_GROUP_CAPACITY);
    config.group_count = 0;
    CHECK(tetra_ms_mm_init(&storage, sizeof(storage), &config) == TETRA_MS_MM_OK);
    event.kind = TETRA_MS_MM_REGISTER;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_get_status(&storage, &status) == TETRA_MS_MM_OK);
    CHECK(status.registration == TETRA_MS_MM_REGISTERING);
    CHECK(status.t351_countdown == TETRA_MS_MM_T351_SLOTS && status.attempts == 1u);
    CHECK(status.actions == 2u);
    action.size = 0;
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_ABI_MISMATCH);
    action.size = sizeof(action);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_SEND && action.priority == 6u);
    CHECK(action.pdu_bits == 10u && action.pdu[0] == 0x26u && action.pdu[1] == 0);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_SERVICE && action.service == TETRA_MS_MM_SERVICE_WAITING);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_EMPTY);
    CHECK(action.kind == TETRA_MS_MM_SERVICE);
    event.kind = TETRA_MS_MM_DOWNLINK;
    event.pdu_bits = 8u;
    event.pdu[0] = 0x56u; /* Minimal full registration accept. */
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    event.pdu[0] = 0xffu; /* Borrow ended: queued outputs cannot reference input. */
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_IDENTITIES);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_REGISTRATION_SUCCESS);
    CHECK(tetra_ms_mm_get_status(&storage, &status) == TETRA_MS_MM_OK);
    CHECK(status.registration == TETRA_MS_MM_REGISTERED);

    event.kind = TETRA_MS_MM_SET_GROUPS;
    event.group_count = 2u;
    event.groups[0].gssi = 91u;
    event.groups[1].gssi = 220u;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_get_status(&storage, &status) == TETRA_MS_MM_OK);
    CHECK(status.attached_count == 2u && status.attached[0] == 91u && status.attached[1] == 220u);
    event.kind = TETRA_MS_MM_GROUPS;
    event.flags = TETRA_MS_MM_DETACH_ALL | TETRA_MS_MM_REPORT_REQUESTED;
    event.handle = 42u;
    event.group_count = 1u;
    event.groups[0].gssi = 300u;
    event.groups[0].value = 3u;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_get_status(&storage, &status) == TETRA_MS_MM_OK);
    CHECK((status.flags & TETRA_MS_MM_HAS_GROUP_OPERATION) != 0u);
    CHECK(status.group_handle == 42u && status.t353_countdown == TETRA_MS_MM_T353_SLOTS);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_SEND && action.priority == 3u && action.pdu_bits == 60u);
    event.kind = TETRA_MS_MM_DOWNLINK;
    event.flags = 0;
    event.pdu_bits = 7u;
    event.pdu[0] = 0xb0u; /* Valid empty group acknowledgement. */
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_IDENTITIES && action.attached_count == 0u);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_GROUP_CONFIRM && action.handle == 42u);
    CHECK(action.flags == (TETRA_MS_MM_ACTION_DETACH_ALL | TETRA_MS_MM_ACTION_REPORT_REQUESTED));

    event.kind = TETRA_MS_MM_LINK_BREAK;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_SERVICE && action.service == TETRA_MS_MM_SERVICE_OUT_OF_SERVICE);
    event.kind = TETRA_MS_MM_LINK_REOPEN;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_SERVICE && action.service == TETRA_MS_MM_SERVICE_IN_SERVICE);
    event.kind = TETRA_MS_MM_DEREGISTER;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_SEND && action.pdu_bits == 5u && action.pdu[0] == 0x10u);
    CHECK(action.flags == TETRA_MS_MM_HAS_DETACH_RECEIPT);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_SERVICE);
    event.kind = TETRA_MS_MM_TICK;
    event.elapsed_slots = 1u;
    event.transfer_state = TETRA_MS_MM_TRANSFER_ACKNOWLEDGED;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_DETACH_REPORT && action.flags == TETRA_MS_MM_TRANSFER_SUCCESSFUL);
    CHECK(tetra_ms_mm_poll_action(&storage, &action) == TETRA_MS_MM_OK);
    CHECK(action.kind == TETRA_MS_MM_DETACH_COMPLETE);
    CHECK(tetra_ms_mm_reset(&storage, &config) == TETRA_MS_MM_OK);
    CHECK(tetra_ms_mm_get_status(&storage, &status) == TETRA_MS_MM_OK);
    CHECK(status.registration == TETRA_MS_MM_IDLE && status.actions == 0u);
    event.kind = TETRA_MS_MM_SET_IDENTITY;
    event.own_issi = 222u;
    event.cell_mcc = 1u;
    event.cell_mnc = 2u;
    event.cell_la = 3u;
    CHECK(tetra_ms_mm_push_event(&storage, &event) == TETRA_MS_MM_OK);
    return 0;
}

#ifdef TETRA_MS_MM_SMOKE_MAIN
int main(void) {
    return tetra_ms_mm_c_smoke();
}
#endif
