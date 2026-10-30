# Flags as recommended by ckb-c-stdlib's guide.md (single compilation unit).
CC := clang
STDLIB := ../deps/ckb-c-stdlib

CFLAGS := --target=riscv64 -march=rv64imc_zba_zbb_zbc_zbs -O3 \
          -fdata-sections -ffunction-sections \
          -fno-builtin-printf -fno-builtin-memcmp \
          -nostdlib -Wall -Werror \
          -I$(STDLIB) -I$(STDLIB)/libc
LDFLAGS := -static $(LDEXTRA) -Wl,--gc-sections -Wl,--strip-all

build/skeletontest: main.c
	mkdir -p build
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $<

clean:
	rm -rf build

.PHONY: clean
