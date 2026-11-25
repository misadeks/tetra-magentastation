#include "tetra_ms.h"

#define CHECK(expression) do { if (!(expression)) return __LINE__; } while (0)

int32_t tetra_ms_c_smoke(void) {
    struct tetra_ms_storage storage = {0};
    struct tetra_ms_config config = {sizeof(config), TETRA_MS_ABI_VERSION, 1};
    struct tetra_ms_event event = {0};
    struct tetra_ms_action action = {0};
    struct tetra_ms_status status = {0};
    CHECK(tetra_ms_abi_version() == TETRA_MS_ABI_VERSION);
    CHECK(tetra_ms_core_size() == sizeof(storage));
    CHECK(tetra_ms_core_align() == TETRA_MS_ALIGNOF(struct tetra_ms_storage));
    CHECK(tetra_ms_init(NULL, sizeof(storage), &config) == TETRA_MS_ARGUMENT);
    CHECK(tetra_ms_init(&storage, sizeof(storage) - 1, &config) == TETRA_MS_CAPACITY);
    CHECK(tetra_ms_init(&storage, sizeof(storage), &config) == TETRA_MS_OK);
    event.size = sizeof(event);
    event.abi_version = TETRA_MS_ABI_VERSION;
    event.kind = TETRA_MS_SEND;
    event.command = 17;
    event.ssi = 1;
    event.ssi_type = TETRA_MS_ISSI;
    event.pdu_bits = 5;
    event.pdu[0] = 0x10; /* 00010: characterization detach SDU */
    CHECK(tetra_ms_push_event(&storage, &event) == TETRA_MS_OK);
    event.kind = TETRA_MS_TICK;
    CHECK(tetra_ms_push_event(&storage, &event) == TETRA_MS_OK);
    action.size = sizeof(action);
    action.abi_version = TETRA_MS_ABI_VERSION;
    CHECK(tetra_ms_poll_action(&storage, &action) == TETRA_MS_OK);
    CHECK(action.kind == TETRA_MS_TRANSMIT);
    CHECK(action.command == 17);
    CHECK(action.pdu_bits == 10);
    CHECK(action.pdu[0] == 0x10 && action.pdu[1] == 0x80);
    CHECK(action.receipt_epoch != 0 && action.receipt_sequence != 0);
    CHECK(tetra_ms_poll_action(&storage, &action) == TETRA_MS_EMPTY);
    status.size = sizeof(status);
    status.abi_version = TETRA_MS_ABI_VERSION;
    CHECK(tetra_ms_get_status(&storage, &status) == TETRA_MS_OK);
    CHECK(status.pending == 1);
    event.kind = TETRA_MS_RECEIVE;
    event.pdu_bits = 5;
    event.pdu[0] = 0x30; /* BL-ACK N(R)=0 */
    CHECK(tetra_ms_push_event(&storage, &event) == TETRA_MS_OK);
    CHECK(tetra_ms_poll_action(&storage, &action) == TETRA_MS_OK);
    CHECK(action.kind == TETRA_MS_COMPLETED);
    CHECK(action.completion == TETRA_MS_COMPLETION_ACKNOWLEDGED);
    CHECK(tetra_ms_reset(&storage, &config) == TETRA_MS_OK);
    CHECK(tetra_ms_get_status(&storage, &status) == TETRA_MS_OK);
    CHECK(status.pending == 0 && status.links == 0);
    return 0;
}

#ifdef TETRA_MS_SMOKE_MAIN
int main(void) {
    return tetra_ms_c_smoke();
}
#endif
