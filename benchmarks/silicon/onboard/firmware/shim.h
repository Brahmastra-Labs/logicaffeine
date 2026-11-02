// The shim's shared declarations: the saved user registers, and the pieces the files of the shim call across.
#pragma once

#include <stddef.h>
#include <stdint.h>

// The user registers as trap_entry saves them: x[0] is zero, x[2] is the user's stack pointer.
struct frame {
  uint64_t x[32];
};

void *memcpy(void *dst, const void *src, size_t n);
void *memset(void *dst, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);

uint32_t rvc_expand(uint16_t c);

void log_byte(char c);
void log_str(const char *s);
void log_hex(uint64_t v);
void log_dec(uint64_t v);
void fatal(const char *what, uint64_t a, uint64_t b, uint64_t c) __attribute__((noreturn));
void shim_exit(uint64_t status) __attribute__((noreturn));

void fpu_reset(void);
int fpu_emulate(struct frame *f, uint32_t raw, int compressed);

void enter_user(uint64_t entry, uint64_t sp) __attribute__((noreturn));
