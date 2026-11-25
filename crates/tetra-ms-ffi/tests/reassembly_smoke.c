#include "tetra_reassembly.h"
#define CHECK(expression) do { if (!(expression)) return __LINE__; } while (0)

int32_t tetra_ms_reassembly_c_smoke(void) {
    struct tetra_ms_reassembly_storage storage = {0};
    struct tetra_ms_fragment_event event = {0};
    struct tetra_ms_fragment_action action = {0};
    CHECK(tetra_ms_reassembly_size() == sizeof(storage));
    CHECK(tetra_ms_reassembly_init(NULL, sizeof(storage)) == TETRA_MS_ARGUMENT);
    CHECK(tetra_ms_reassembly_init(&storage, sizeof(storage) - 1) == TETRA_MS_CAPACITY);
    CHECK(tetra_ms_reassembly_init(&storage, sizeof(storage)) == TETRA_MS_OK);
    event.size = sizeof(event);
    event.abi_version = TETRA_MS_ABI_VERSION;
    event.kind = TETRA_MS_FRAGMENT_FIRST;
    event.ssi = 1234;
    event.ssi_type = TETRA_MS_ISSI;
    event.pdu_bits = 3;
    CHECK(tetra_ms_reassembly_push(&storage, &event) == TETRA_MS_OK);
    action.size = sizeof(action);
    action.abi_version = TETRA_MS_ABI_VERSION;
    CHECK(tetra_ms_reassembly_poll(&storage, 0, &action) == TETRA_MS_EMPTY);
    event.kind = TETRA_MS_FRAGMENT_NEXT;
    event.now = 4;
    event.pdu[0] = 0x80;
    CHECK(tetra_ms_reassembly_push(&storage, &event) == TETRA_MS_OK);
    event.kind = TETRA_MS_FRAGMENT_LAST;
    event.now = 8;
    event.pdu[0] = 0x20;
    event.pdu_bits = 4;
    CHECK(tetra_ms_reassembly_push(&storage, &event) == TETRA_MS_OK);
    action.size = 0;
    CHECK(tetra_ms_reassembly_poll(&storage, 8, &action) == TETRA_MS_ABI_MISMATCH);
    action.size = sizeof(action);
    CHECK(tetra_ms_reassembly_poll(&storage, 8, &action) == TETRA_MS_OK);
    CHECK(action.pdu_bits == 10 && action.fragments == 3 && action.ssi == 1234);
    CHECK(action.first == 0 && action.last == 8);
    CHECK(action.pdu[0] == 0x10 && action.pdu[1] == 0x80);
    CHECK(tetra_ms_reassembly_poll(&storage, 8, &action) == TETRA_MS_EMPTY);
    CHECK(action.pdu_bits == 10);
    CHECK(tetra_ms_reassembly_reset(&storage) == TETRA_MS_OK);
    event.kind = TETRA_MS_FRAGMENT_FIRST;
    event.pdu_bits = 4096;
    CHECK(tetra_ms_reassembly_push(&storage, &event) == TETRA_MS_OK);
    event.kind = TETRA_MS_FRAGMENT_LAST;
    event.pdu_bits = 1;
    CHECK(tetra_ms_reassembly_push(&storage, &event) == TETRA_MS_FRAGMENT_OVERFLOW_ABORTED);
    CHECK(tetra_ms_reassembly_poll(&storage, 8, &action) == TETRA_MS_EMPTY);
    CHECK(tetra_ms_reassembly_push(&storage, (const struct tetra_ms_fragment_event *)&storage) == TETRA_MS_ARGUMENT);
    CHECK(tetra_ms_reassembly_reset(&storage) == TETRA_MS_OK);
    return 0;
}
