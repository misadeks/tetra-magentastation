/* Freestanding link driver; not a board startup image or hardware test. */
#include <stdint.h>

int32_t tetra_ms_c_smoke(void);
int32_t tetra_ms_reassembly_c_smoke(void);
int32_t tetra_ms_mm_c_smoke(void);
int32_t tetra_mle_c_smoke(void);
int32_t tetra_ms_mac_c_smoke(void);
int32_t tetra_cmce_c_smoke(void);
int32_t tetra_sds_c_smoke(void);

int main(void) {
    int32_t result = tetra_ms_c_smoke();
    if (result != 0) return result;
    result = tetra_ms_reassembly_c_smoke();
    if (result != 0) return result;
    result = tetra_ms_mm_c_smoke();
    if (result != 0) return result;
    result = tetra_mle_c_smoke();
    if (result != 0) return result;
    result = tetra_ms_mac_c_smoke();
    if (result != 0) return result;
    result = tetra_cmce_c_smoke();
    if (result != 0) return result;
    return tetra_sds_c_smoke();
}
