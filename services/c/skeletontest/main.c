/* skeletontest: puts the job payload as the transaction skeleton (a molecule Transaction).
 * Test service for the skeleton guard (scripts/manual-checks): the payload names the cells
 * the skeleton tries to spend. Result = "skeleton". */
#include "ckb_syscalls.h"

#define CSR_JOB_PAYLOAD 7001
#define CSR_TX_SKELETON_PUT 7030
#define CSR_RESULT_PUT 7031

#define MAX_PAYLOAD 65536

static uint8_t buf[MAX_PAYLOAD];

int main(void) {
  uint64_t len = MAX_PAYLOAD;
  if (syscall(CSR_JOB_PAYLOAD, buf, &len, 0, 0, 0, 0) != 0) {
    return 1;
  }
  if (len > MAX_PAYLOAD) {
    return 2;
  }
  if (len > 0 && syscall(CSR_TX_SKELETON_PUT, buf, len, 0, 0, 0, 0) != 0) {
    return 3;
  }
  if (syscall(CSR_RESULT_PUT, "skeleton", 8, 0, 0, 0, 0) != 0) {
    return 4;
  }
  return 0;
}
