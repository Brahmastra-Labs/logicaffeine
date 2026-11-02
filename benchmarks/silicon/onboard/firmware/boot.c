// boot — the hart's reset vector, in the board's block RAM. It waits for the DDR2's calibration and tests it, reads
// the boot image's header from the flash, refuses an image built for a different bitstream, inflates the image's
// body straight from the flash into the DDR2, checks it against the digest the header carries, installs the shim
// and starts it. Every refusal is named on the log UART and the board stops there.

#include "soc.h"
#include "boot.h"

static inline uint64_t mmio_read(uint64_t addr) { return *(volatile uint64_t *)addr; }
static inline void mmio_write(uint64_t addr, uint64_t v) { *(volatile uint64_t *)addr = v; }

static void out(char c) {
  while (mmio_read(UART_STATUS) & 1) {
  }
  mmio_write(UART_TX, (uint8_t)c);
}

static void say(const char *s) {
  while (*s) out(*s++);
}

static void hex(uint64_t v) {
  say("0x");
  for (int i = 60; i >= 0; i -= 4) out("0123456789abcdef"[(v >> i) & 15]);
}

static void refuse(const char *why, uint64_t detail) {
  say("\nboot: ");
  say(why);
  say(" (");
  hex(detail);
  say(")\n");
  mmio_write(SYS_EXIT, 0x200);
  for (;;) __asm__ volatile("wfi");
}

// The flash, a byte at a time, eight bytes to a read of the streaming port. Every byte the inflater takes is also
// digested.
static uint64_t stream_word;
static int stream_left;
static struct sha256 digest_of_packed;

static uint8_t flash_byte(void) {
  if (stream_left == 0) {
    stream_word = mmio_read(QSPI_DATA);
    stream_left = 8;
  }
  uint8_t b = (uint8_t)stream_word;
  stream_word >>= 8;
  stream_left--;
  return b;
}

static uint8_t packed_byte(void) {
  uint8_t b = flash_byte();
  sha256_byte(&digest_of_packed, b);
  return b;
}

static uint64_t le64(const uint8_t *p) {
  uint64_t v = 0;
  for (int i = 7; i >= 0; i--) v = (v << 8) | p[i];
  return v;
}

// One word in every 64 KB of the DDR2, written with its own address and read back.
static void memtest(void) {
  for (uint64_t a = RAM_BASE; a < RAM_BASE + RAM_SIZE; a += 0x10000) *(volatile uint64_t *)a = a ^ 0x5a5a5a5a5a5a5a5aull;
  for (uint64_t a = RAM_BASE; a < RAM_BASE + RAM_SIZE; a += 0x10000)
    if (*(volatile uint64_t *)a != (a ^ 0x5a5a5a5a5a5a5a5aull)) refuse("the DDR2 does not hold what was written", a);
}

struct section {
  uint32_t kind, flags;
  uint64_t offset, length;
};

void boot_main(void) {
  say("LOGOS boot\n");
  uint64_t ddr;
  for (;;) {
    ddr = mmio_read(DDR_STATUS);
    if (ddr & 1) break;
    if (ddr & 2) refuse("the DDR2 did not calibrate", mmio_read(DDR_CAL));
  }
  say("boot: DDR2 calibrated: status ");
  hex(ddr);
  say(", lanes ");
  hex(mmio_read(DDR_CAL));
  out('\n');
  memtest();
  say("boot: DDR2 ready\n");

  mmio_write(QSPI_START, IMAGE_OFFSET);
  stream_left = 0;
  uint8_t header[IMAGE_HEADER_SIZE];
  for (uint64_t i = 0; i < IMAGE_HEADER_SIZE; i++) header[i] = flash_byte();
  if (le64(header) != IMAGE_MAGIC) refuse("the flash holds no boot image", le64(header));
  if ((le64(header + 8) & 0xffffffff) != IMAGE_VERSION) refuse("the boot image's version is not this ROM's", le64(header + 8));
  if (le64(header + 0x10) != mmio_read(SYS_RTL_MD5_LO) || le64(header + 0x18) != mmio_read(SYS_RTL_MD5_HI))
    refuse("the boot image was built for a different bitstream", le64(header + 0x10));
  uint64_t body_len = le64(header + 0x20), packed_len = le64(header + 0x28);
  if (body_len > STAGING_SIZE) refuse("the boot image is larger than its staging area", body_len);

  say("boot: inflating the boot image\n");
  sha256_init(&digest_of_packed);
  int64_t got = inflate((struct inflate_tables *)BOOT_WORK_BASE, packed_byte, packed_len, (uint8_t *)STAGING_BASE,
                        STAGING_SIZE);
  mmio_write(QSPI_STOP, 0);
  uint8_t digest[32];
  sha256_final(&digest_of_packed, digest);
  if (memcmp(digest, header + 0x30, 32) != 0) refuse("the boot image's digest does not match", le64(digest));
  if (got < 0 || (uint64_t)got != body_len) refuse("the boot image does not inflate", (uint64_t)got);

  const uint8_t *body = (const uint8_t *)STAGING_BASE;
  uint32_t count = *(const uint32_t *)body;
  const struct section *table = (const struct section *)(body + 8);
  for (uint32_t i = 0; i < count; i++) {
    if (table[i].kind != SECTION_SHIM) continue;
    if (table[i].length > SHIM_SIZE - 0x10000) refuse("the shim does not fit its region", table[i].length);
    memcpy((void *)SHIM_BASE, body + table[i].offset, table[i].length);
    __asm__ volatile("fence.i" ::: "memory");
    say("boot: starting the shim\n");
    ((void (*)(const uint8_t *))SHIM_BASE)(body);
  }
  refuse("the boot image holds no shim", count);
}
