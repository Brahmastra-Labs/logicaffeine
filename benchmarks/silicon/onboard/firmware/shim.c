// shim — the board's machine-mode layer under the LOGOS compiler. The compiler is a static riscv64 Linux program,
// built from the same Rust as on the laptop; the shim loads it into the DDR2, gives it the process a Linux kernel
// would (its stack, arguments and auxiliary vector), runs it in user mode behind physical memory protection, and
// answers its traps: the system calls it makes, the floating-point instructions the hart leaves to software, and
// the misaligned accesses it might make. Anything else is reported by name on the log UART and the board stops —
// never a silent hang.

#include "soc.h"
#include "shim.h"

// ── registers and the trap frame ─────────────────────────────────────────────────────────────────────────────
#define CSR_READ(name) ({ uint64_t v_; __asm__ volatile("csrr %0, " #name : "=r"(v_)); v_; })
#define CSR_WRITE(name, v) __asm__ volatile("csrw " #name ", %0" ::"r"((uint64_t)(v)))

static inline uint64_t mmio_read(uint64_t addr) { return *(volatile uint64_t *)addr; }
static inline void mmio_write(uint64_t addr, uint64_t v) { *(volatile uint64_t *)addr = v; }

// ── the log ──────────────────────────────────────────────────────────────────────────────────────────────────
void log_byte(char c) {
  while (mmio_read(UART_STATUS) & 1) {
  }
  mmio_write(UART_TX, (uint8_t)c);
}

void log_str(const char *s) {
  while (*s) log_byte(*s++);
}

void log_hex(uint64_t v) {
  log_str("0x");
  for (int i = 60; i >= 0; i -= 4) log_byte("0123456789abcdef"[(v >> i) & 15]);
}

void log_dec(uint64_t v) {
  char buf[21];
  int n = 0;
  do {
    buf[n++] = (char)('0' + v % 10);
    v /= 10;
  } while (v);
  while (n) log_byte(buf[--n]);
}

void shim_exit(uint64_t status) {
  mmio_write(SYS_EXIT, status);
  for (;;) __asm__ volatile("wfi");
}

void fatal(const char *what, uint64_t a, uint64_t b, uint64_t c) {
  log_str("\nshim: ");
  log_str(what);
  log_str(" (");
  log_hex(a);
  log_str(", ");
  log_hex(b);
  log_str(", ");
  log_hex(c);
  log_str(")\n");
  shim_exit(0x100 | 0x7f);
}

// ── the boot image's body ────────────────────────────────────────────────────────────────────────────────────
struct section {
  uint32_t kind;
  uint32_t flags;
  uint64_t offset;
  uint64_t length;
};

static const uint8_t *find_section(const uint8_t *body, uint32_t kind, uint64_t *length) {
  uint32_t count = *(const uint32_t *)body;
  const struct section *table = (const struct section *)(body + 8);
  for (uint32_t i = 0; i < count; i++) {
    if (table[i].kind == kind) {
      *length = table[i].length;
      return body + table[i].offset;
    }
  }
  return 0;
}

static const uint8_t *section_of(const uint8_t *body, uint32_t kind, uint64_t *length) {
  const uint8_t *p = find_section(body, kind, length);
  if (!p) fatal("the boot image has no section", kind, *(const uint32_t *)body, 0);
  return p;
}

// ── the files the compiler may open: the bundle's sources, read-only ─────────────────────────────────────────
#define MAX_FILES 256
#define MAX_FDS 32
struct file {
  const char *name;
  uint32_t name_length;
  const uint8_t *data;
  uint64_t length;
};
static struct file files[MAX_FILES];
static uint32_t file_count;

// A little-endian u32 at any byte address: the bundle packs its records with no padding, and the hart traps a
// misaligned load.
static uint32_t read_u32(const uint8_t *p) {
  uint32_t v;
  memcpy(&v, p, 4);
  return v;
}

// A bundle: u32 count, then per file u32 name length, the name, u64 data length, the data.
static void index_bundle(const uint8_t *p, uint64_t length) {
  const uint8_t *end = p + length;
  uint32_t count = read_u32(p);
  p += 4;
  if (count > MAX_FILES) fatal("the bundle holds too many files", count, MAX_FILES, 0);
  for (uint32_t i = 0; i < count; i++) {
    uint32_t nl = read_u32(p);
    p += 4;
    files[i].name = (const char *)p;
    files[i].name_length = nl;
    p += nl;
    uint64_t dl;
    memcpy(&dl, p, 8);
    p += 8;
    files[i].data = p;
    files[i].length = dl;
    p += dl;
    if (p > end) fatal("the bundle is truncated", i, count, 0);
  }
  file_count = count;
}

enum fd_kind { FD_FREE, FD_CONSOLE_IN, FD_CONSOLE_OUT, FD_FILE, FD_LINK };
struct fd {
  enum fd_kind kind;
  uint32_t file;
  uint64_t pos;
};
static struct fd fds[MAX_FDS];

static int name_eq(const char *a, uint32_t alen, const char *b) {
  uint32_t i = 0;
  for (; i < alen && b[i]; i++)
    if (a[i] != b[i]) return 0;
  return i == alen && b[i] == 0;
}

// ── memory the compiler asks for ─────────────────────────────────────────────────────────────────────────────
#define PAGE 4096ull
static uint64_t brk_start, brk_now;
static uint8_t mmap_used[MMAP_SIZE / PAGE / 8];

static int page_used(uint64_t i) { return (mmap_used[i / 8] >> (i % 8)) & 1; }
static void page_mark(uint64_t i, int used) {
  if (used) mmap_used[i / 8] |= (uint8_t)(1u << (i % 8));
  else mmap_used[i / 8] &= (uint8_t)~(1u << (i % 8));
}

static uint64_t mmap_alloc(uint64_t length) {
  uint64_t pages = (length + PAGE - 1) / PAGE, total = MMAP_SIZE / PAGE, run = 0;
  for (uint64_t i = 0; i < total; i++) {
    run = page_used(i) ? 0 : run + 1;
    if (run == pages) {
      uint64_t first = i + 1 - pages;
      for (uint64_t k = first; k <= i; k++) page_mark(k, 1);
      uint64_t addr = MMAP_BASE + first * PAGE;
      memset((void *)addr, 0, pages * PAGE);
      return addr;
    }
  }
  return 0;
}

static void mmap_free(uint64_t addr, uint64_t length) {
  if (addr < MMAP_BASE || addr >= MMAP_BASE + MMAP_SIZE) return;
  uint64_t first = (addr - MMAP_BASE) / PAGE, pages = (length + PAGE - 1) / PAGE;
  for (uint64_t k = first; k < first + pages && k < MMAP_SIZE / PAGE; k++) page_mark(k, 0);
}

// ── system calls ─────────────────────────────────────────────────────────────────────────────────────────────
#define E_BADF 9
#define E_NOMEM 12
#define E_FAULT 14
#define E_INVAL 22
#define E_NOENT 2
#define E_AGAIN 11
#define E_NFILE 23

static uint64_t rng_state = 0x9e3779b97f4a7c15ull;
static uint64_t next_random(void) {
  uint64_t z = (rng_state += 0x9e3779b97f4a7c15ull);
  z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull;
  z = (z ^ (z >> 27)) * 0x94d049bb133111ebull;
  return z ^ (z >> 31);
}

static int64_t sys_openat(uint64_t path_addr, uint64_t flags) {
  const char *path = (const char *)path_addr;
  if ((flags & 3) != 0 && !name_eq("/dev/link", 9, path)) return -E_INVAL;
  int slot = -1;
  for (int i = 3; i < MAX_FDS; i++)
    if (fds[i].kind == FD_FREE) {
      slot = i;
      break;
    }
  if (slot < 0) return -E_NFILE;
  if (name_eq("/dev/link", 9, path)) {
    fds[slot].kind = FD_LINK;
    return slot;
  }
  for (uint32_t f = 0; f < file_count; f++) {
    if (name_eq(files[f].name, files[f].name_length, path)) {
      fds[slot].kind = FD_FILE;
      fds[slot].file = f;
      fds[slot].pos = 0;
      return slot;
    }
  }
  return -E_NOENT;
}

static int64_t sys_read(uint64_t fd, uint64_t buf, uint64_t count) {
  if (fd >= MAX_FDS) return -E_BADF;
  struct fd *d = &fds[fd];
  switch (d->kind) {
    case FD_CONSOLE_IN:
      return 0;
    case FD_FILE: {
      const struct file *f = &files[d->file];
      uint64_t left = f->length - d->pos, n = count < left ? count : left;
      memcpy((void *)buf, f->data + d->pos, n);
      d->pos += n;
      return (int64_t)n;
    }
    case FD_LINK: {
      uint64_t n = 0;
      while (n < count) {
        uint64_t v = mmio_read(LINK_RX);
        if (!(v & 0x100)) break;
        ((uint8_t *)buf)[n++] = (uint8_t)v;
      }
      return n ? (int64_t)n : -E_AGAIN;
    }
    default:
      return -E_BADF;
  }
}

static int64_t sys_write(uint64_t fd, uint64_t buf, uint64_t count) {
  if (fd >= MAX_FDS) return -E_BADF;
  const uint8_t *p = (const uint8_t *)buf;
  switch (fds[fd].kind) {
    case FD_CONSOLE_OUT:
      for (uint64_t i = 0; i < count; i++) log_byte((char)p[i]);
      return (int64_t)count;
    case FD_LINK:
      for (uint64_t i = 0; i < count; i++) {
        while (mmio_read(LINK_STATUS) & 1) {
        }
        mmio_write(LINK_TX, p[i]);
      }
      return (int64_t)count;
    default:
      return -E_BADF;
  }
}

// struct stat for riscv64 Linux.
struct kstat {
  uint64_t dev, ino;
  uint32_t mode, nlink, uid, gid;
  uint64_t rdev, pad1;
  int64_t size;
  int32_t blksize, pad2;
  int64_t blocks;
  int64_t atime, atime_ns, mtime, mtime_ns, ctime, ctime_ns;
  uint32_t unused[2];
};

static int64_t sys_fstat(uint64_t fd, uint64_t out) {
  if (fd >= MAX_FDS || fds[fd].kind == FD_FREE) return -E_BADF;
  struct kstat st;
  memset(&st, 0, sizeof st);
  st.nlink = 1;
  st.blksize = 4096;
  if (fds[fd].kind == FD_FILE) {
    st.mode = 0100444;
    st.size = (int64_t)files[fds[fd].file].length;
    st.ino = fds[fd].file + 1;
  } else {
    st.mode = 0020666;
  }
  memcpy((void *)out, &st, sizeof st);
  return 0;
}

static int64_t sys_brk(uint64_t addr) {
  if (addr >= brk_start && addr <= MMAP_BASE) brk_now = addr;
  return (int64_t)brk_now;
}

static int64_t sys_mmap(uint64_t addr, uint64_t length, uint64_t prot, uint64_t flags, uint64_t fd) {
  (void)prot;
  const uint64_t MAP_FIXED = 0x10, MAP_ANONYMOUS = 0x20;
  if (!(flags & MAP_ANONYMOUS) || (int64_t)fd != -1) fatal("mmap of a file", addr, length, flags);
  if (length == 0) return -E_INVAL;
  if (flags & MAP_FIXED) {
    // A fixed mapping inside the region the shim hands out is reserved there; one elsewhere (musl's guard page
    // after the break) needs nothing: without paging, every address already exists.
    if (addr >= MMAP_BASE && addr + length <= MMAP_BASE + MMAP_SIZE) {
      uint64_t first = (addr - MMAP_BASE) / PAGE, pages = (length + PAGE - 1) / PAGE;
      for (uint64_t k = first; k < first + pages; k++) page_mark(k, 1);
      memset((void *)addr, 0, pages * PAGE);
    }
    return (int64_t)addr;
  }
  uint64_t got = mmap_alloc(length);
  return got ? (int64_t)got : -E_NOMEM;
}

static int64_t sys_mremap(uint64_t old, uint64_t old_len, uint64_t new_len, uint64_t flags) {
  const uint64_t MREMAP_MAYMOVE = 1;
  if (!(flags & MREMAP_MAYMOVE)) return -E_NOMEM;
  uint64_t got = mmap_alloc(new_len);
  if (!got) return -E_NOMEM;
  memcpy((void *)got, (const void *)old, old_len < new_len ? old_len : new_len);
  mmap_free(old, old_len);
  return (int64_t)got;
}

static int64_t sys_ppoll(uint64_t fds_addr, uint64_t n) {
  // struct pollfd { int fd; short events; short revents; }: every descriptor the compiler polls is open.
  for (uint64_t i = 0; i < n; i++) {
    uint8_t *p = (uint8_t *)fds_addr + 8 * i;
    *(int16_t *)(p + 6) = 0;
  }
  return 0;
}

static int64_t sys_clock_gettime(uint64_t out) {
  uint64_t cycles = mmio_read(SYS_CYCLES);
  int64_t ts[2] = {(int64_t)(cycles / HART_HZ), (int64_t)((cycles % HART_HZ) * 1000000000ull / HART_HZ)};
  memcpy((void *)out, ts, sizeof ts);
  return 0;
}

static int64_t syscall_dispatch(struct frame *f) {
  uint64_t n = f->x[17], a0 = f->x[10], a1 = f->x[11], a2 = f->x[12], a3 = f->x[13], a4 = f->x[14];
  switch (n) {
    case 25:  // fcntl
      return 0;
    case 56:  // openat
      return sys_openat(a1, a2);
    case 57:  // close
      if (a0 < 3 || a0 >= MAX_FDS || fds[a0].kind == FD_FREE) return -E_BADF;
      fds[a0].kind = FD_FREE;
      return 0;
    case 63:  // read
      return sys_read(a0, a1, a2);
    case 64:  // write
      return sys_write(a0, a1, a2);
    case 73:  // ppoll
      return sys_ppoll(a0, a1);
    case 80:  // fstat
      return sys_fstat(a0, a1);
    case 93:  // exit
    case 94:  // exit_group
      log_str("\nshim: the compiler exited with status ");
      log_dec(a0);
      log_str("\n");
      shim_exit(a0);
      return 0;
    case 96:  // set_tid_address
      return 1;
    case 113:  // clock_gettime
      return sys_clock_gettime(a1);
    case 132:  // sigaltstack
    case 134:  // rt_sigaction
    case 135:  // rt_sigprocmask
      if (n == 132 && a1) memset((void *)a1, 0, 24);
      if (n == 134 && a2) memset((void *)a2, 0, 32);
      if (n == 135 && a2) memset((void *)a2, 0, 8);
      return 0;
    case 214:  // brk
      return sys_brk(a0);
    case 215:  // munmap
      mmap_free(a0, a1);
      return 0;
    case 216:  // mremap
      return sys_mremap(a0, a1, a2, a3);
    case 222:  // mmap
      return sys_mmap(a0, a1, a2, a3, a4);
    case 226:  // mprotect
    case 233:  // madvise
      return 0;
    case 278: {  // getrandom
      uint8_t *p = (uint8_t *)a0;
      for (uint64_t i = 0; i < a1; i++) {
        if (i % 8 == 0) next_random();
        p[i] = (uint8_t)(rng_state >> (8 * (i % 8)));
      }
      return (int64_t)a1;
    }
    default:
      fatal("an unimplemented system call", n, f->x[0], CSR_READ(mepc));
      return 0;
  }
}

// ── misaligned loads and stores, done a byte at a time ───────────────────────────────────────────────────────
static uint32_t fetch_insn(uint64_t pc, uint64_t *len) {
  uint16_t lo = *(const uint16_t *)pc;
  if ((lo & 3) != 3) {
    *len = 2;
    return lo;
  }
  *len = 4;
  return lo | ((uint32_t) * (const uint16_t *)(pc + 2) << 16);
}

static void misaligned(struct frame *f, uint64_t pc, uint64_t addr, int store) {
  uint64_t len;
  uint32_t raw = fetch_insn(pc, &len);
  uint32_t inst = len == 2 ? rvc_expand((uint16_t)raw) : raw;
  uint32_t op = inst & 0x7f, f3 = (inst >> 12) & 7, rd = (inst >> 7) & 31, rs2 = (inst >> 20) & 31;
  uint64_t size = 1ull << (f3 & 3);
  if (!store && op == 0x03) {
    uint64_t v = 0;
    for (uint64_t i = 0; i < size; i++) v |= (uint64_t)((const uint8_t *)addr)[i] << (8 * i);
    if (!(f3 & 4) && size < 8) {
      uint64_t shift = 64 - 8 * size;
      v = (uint64_t)((int64_t)(v << shift) >> shift);
    }
    if (rd) f->x[rd] = v;
  } else if (store && op == 0x23) {
    uint64_t v = f->x[rs2];
    for (uint64_t i = 0; i < size; i++) ((uint8_t *)addr)[i] = (uint8_t)(v >> (8 * i));
  } else {
    fatal(store ? "a misaligned store the shim cannot complete" : "a misaligned load the shim cannot complete",
          pc, addr, inst);
  }
  CSR_WRITE(mepc, pc + len);
}

// ── traps ────────────────────────────────────────────────────────────────────────────────────────────────────
void trap(struct frame *f) {
  uint64_t cause = CSR_READ(mcause), pc = CSR_READ(mepc), tval = CSR_READ(mtval);
  if (((CSR_READ(mstatus) >> 11) & 3) == 3) fatal("the shim itself trapped", cause, pc, tval);
  switch (cause) {
    case 8:
      f->x[10] = (uint64_t)syscall_dispatch(f);
      CSR_WRITE(mepc, pc + 4);
      return;
    case 2: {
      uint64_t len = ((tval & 3) == 3) ? 4 : 2;
      if (fpu_emulate(f, (uint32_t)tval, len == 2)) {
        CSR_WRITE(mepc, pc + len);
        return;
      }
      fatal("an illegal instruction", pc, tval, 0);
      return;
    }
    case 4:
      misaligned(f, pc, tval, 0);
      return;
    case 6:
      misaligned(f, pc, tval, 1);
      return;
    case 1:
      fatal("an instruction fetch the compiler may not make", pc, tval, 0);
      return;
    case 5:
    case 7:
      if (tval >= STACK_BASE && tval < STACK_BASE + PAGE) fatal("the compiler overflowed its stack", pc, tval, 0);
      fatal(cause == 5 ? "a load the compiler may not make" : "a store the compiler may not make", pc, tval, 0);
      return;
    default:
      fatal("an unexpected trap", cause, pc, tval);
  }
}

// ── loading the compiler ─────────────────────────────────────────────────────────────────────────────────────
static uint64_t load_elf(const uint8_t *elf, uint64_t length, uint64_t *phdr, uint64_t *phnum, uint64_t *end) {
  if (length < 64 || elf[0] != 0x7f || elf[1] != 'E' || elf[2] != 'L' || elf[3] != 'F' || elf[4] != 2)
    fatal("the compiler is not a 64-bit ELF", length, 0, 0);
  uint64_t entry, phoff;
  uint16_t phentsize, count;
  memcpy(&entry, elf + 24, 8);
  memcpy(&phoff, elf + 32, 8);
  memcpy(&phentsize, elf + 54, 2);
  memcpy(&count, elf + 56, 2);
  uint64_t top = 0, first_vaddr = 0;
  int first = 1;
  for (uint16_t i = 0; i < count; i++) {
    const uint8_t *p = elf + phoff + (uint64_t)i * phentsize;
    uint32_t type;
    uint64_t off, vaddr, filesz, memsz;
    memcpy(&type, p, 4);
    if (type != 1) continue;
    memcpy(&off, p + 8, 8);
    memcpy(&vaddr, p + 16, 8);
    memcpy(&filesz, p + 32, 8);
    memcpy(&memsz, p + 40, 8);
    if (vaddr < FIRMWARE_BASE || vaddr + memsz > MMAP_BASE) fatal("a compiler segment outside its memory", vaddr, memsz, 0);
    if (off + filesz > length) fatal("a compiler segment past the end of its file", off, filesz, length);
    memcpy((void *)vaddr, elf + off, filesz);
    memset((void *)(vaddr + filesz), 0, memsz - filesz);
    if (first) {
      first_vaddr = vaddr - off;
      first = 0;
    }
    if (vaddr + memsz > top) top = vaddr + memsz;
  }
  *phdr = first_vaddr + phoff;
  *phnum = count;
  *end = top;
  return entry;
}

// The process stack Linux builds: argc, argv, envp, the auxiliary vector, and the strings they point at. The
// compiler's name is "logos"; its other arguments are the image's ARGS section, each NUL-terminated.
#define MAX_ARGS 32
static uint64_t build_stack(uint64_t entry, uint64_t phdr, uint64_t phnum, const uint8_t *args, uint64_t args_len) {
  uint64_t top = STACK_BASE + STACK_SIZE;
  uint64_t sp = top - 16 - ((args_len + 15) & ~15ull);
  uint64_t strings = sp;
  memcpy((void *)strings, args, args_len);
  uint64_t argv[MAX_ARGS + 1];
  uint64_t argc = 0;
  sp -= 16;
  memcpy((void *)sp, "logos", 6);
  argv[argc++] = sp;
  for (uint64_t i = 0; i < args_len; argc++) {
    if (argc == MAX_ARGS) fatal("the image gives the compiler too many arguments", argc, MAX_ARGS, 0);
    argv[argc] = strings + i;
    while (i < args_len && args[i]) i++;
    i++;
  }
  sp -= 16;
  uint64_t random = sp;
  for (int i = 0; i < 2; i++) ((uint64_t *)random)[i] = next_random();
  uint64_t aux[][2] = {
      {3, phdr},  {4, 56},  {5, phnum}, {6, PAGE},       {9, entry},  {11, 0},     {12, 0},
      {13, 0},    {14, 0},  {16, HART_HWCAP},           {23, 0},     {25, random}, {0, 0},
  };
  uint64_t words = 1 + (argc + 1) + 1 + sizeof aux / sizeof(uint64_t);
  sp = (sp - words * 8) & ~15ull;
  uint64_t *w = (uint64_t *)sp;
  *w++ = argc;
  for (uint64_t i = 0; i < argc; i++) *w++ = argv[i];
  *w++ = 0;
  *w++ = 0;
  for (uint64_t i = 0; i < sizeof aux / sizeof aux[0]; i++) {
    *w++ = aux[i][0];
    *w++ = aux[i][1];
  }
  return sp;
}

// Protection: the shim's own 2 MB and a guard page under the compiler's stack are closed to user mode; the rest of
// the DDR2 is the compiler's. Nothing else — the boot ROM, the scratch RAM, the devices — is reachable from it.
static void protect(void) {
  uint64_t shim = (SHIM_BASE >> 2) | ((SHIM_SIZE >> 3) - 1);
  uint64_t guard = (STACK_BASE >> 2) | ((PAGE >> 3) - 1);
  uint64_t ram = (RAM_BASE >> 2) | ((RAM_SIZE >> 3) - 1);
  CSR_WRITE(pmpaddr0, shim);
  CSR_WRITE(pmpaddr1, guard);
  CSR_WRITE(pmpaddr2, ram);
  const uint64_t NAPOT = 3 << 3, R = 1, W = 2, X = 4;
  CSR_WRITE(pmpcfg0, NAPOT | (NAPOT << 8) | ((NAPOT | R | W | X) << 16));
}

void shim_main(const uint8_t *body) {
  log_str("shim: loading the compiler\n");
  uint64_t elf_len, bundle_len;
  const uint8_t *elf = section_of(body, SECTION_FIRMWARE, &elf_len);
  const uint8_t *bundle = section_of(body, SECTION_BUNDLE, &bundle_len);
  index_bundle(bundle, bundle_len);
  uint64_t phdr, phnum, end;
  uint64_t entry = load_elf(elf, elf_len, &phdr, &phnum, &end);
  brk_start = brk_now = (end + PAGE - 1) & ~(PAGE - 1);
  for (int i = 0; i < MAX_FDS; i++) fds[i].kind = FD_FREE;
  fds[0].kind = FD_CONSOLE_IN;
  fds[1].kind = FD_CONSOLE_OUT;
  fds[2].kind = FD_CONSOLE_OUT;
  fpu_reset();
  uint64_t args_len = 0;
  const uint8_t *args = find_section(body, SECTION_ARGS, &args_len);
  uint64_t sp = build_stack(entry, phdr, phnum, args ? args : (const uint8_t *)"", args ? args_len : 0);
  protect();
  __asm__ volatile("fence.i");
  log_str("shim: entering the compiler at ");
  log_hex(entry);
  log_str("\n");
  enter_user(entry, sp);
}
