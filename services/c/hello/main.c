/* hello-c: result = "hello:" followed by the job payload (CSR service call `job_payload`). */
#include "ckb_syscalls.h"

#define CSR_JOB_PAYLOAD 7001
#define CSR_RESULT_PUT 7031

/* Results are at most 65,536 bytes; the prefix is 6 bytes. */
#define MAX_PAYLOAD (65536 - 6)

static uint8_t out[6 + MAX_PAYLOAD];

int main(void) {
  memcpy(out, "hello:", 6);
  uint64_t len = MAX_PAYLOAD;
  if (syscall(CSR_JOB_PAYLOAD, out + 6, &len, 0, 0, 0, 0) != 0) {
    return 1;
  }
  if (len > MAX_PAYLOAD) {
    return 2;
  }
  if (syscall(CSR_RESULT_PUT, out, 6 + len, 0, 0, 0, 0) != 0) {
    return 3;
  }
  return 0;
}
