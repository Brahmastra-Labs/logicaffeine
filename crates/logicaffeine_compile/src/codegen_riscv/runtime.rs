//! The C runtime shim compiled alongside the emitted `main.s`: the heap (text, sequences, maps, records, closures), the
//! formatters and the task scheduler, kept in C (not hand-rolled asm) for the same reason the AOT-C tier calls libc:
//! correctness over ceremony. Every block comes from the C tier's collector (`codegen_c::runtime::C_GC_COLLECTOR`),
//! emitted first, so a program holds only the memory it can still reach, as on every other tier.

/// `logos_show_f64` renders an f64 EXACTLY like the VM's `Show` (Rust's `{}` Display: shortest decimal
/// that round-trips, NEVER scientific notation, bare integral floats, `NaN`/`inf`/`-inf`/`-0`). It finds
/// the shortest significant-digit count whose `%e` form round-trips via `strtod`, then reformats that to
/// fixed decimal notation so any magnitude prints without an exponent — matching Rust for every finite
/// value. Called with the double in `fa0` (a named param, so no variadic-FP marshalling needed).
pub const RISCV_RUNTIME_C: &str = concat!(
    crate::codegen_c::runtime::lc_gc_collector_source!(),
    crate::codegen_c::runtime::lc_gc_api_source!(),
r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <stdint.h>
#include <math.h>
#include <time.h>
#include <ucontext.h>
#include <unistd.h>

/* Nothing in a program unwinds: a failure prints its sentence and exits, and a task switches stacks by its
   context. So the runtime's call-frame information goes to `.debug_frame`, which nothing reads at run time,
   rather than `.eh_frame`, every entry of which a static binary's startup registers before `main`. */
__asm__(".cfi_sections .debug_frame");

/* A failure: the program stops with its reason alone on stderr, exit status 1 — the report every tier
   gives (`FailWith`, `Fail with …`, a broken contract's reason, a runtime error) — or, inside a task of
   a concurrent program, the task stops and never returns here, and the run fails with its first
   failure once every other task is quiescent (the scheduler installs `logos_fail_hook` for the run). */
static void (*logos_fail_hook)(const char *) = 0;

__attribute__((noreturn)) void logos_fatal(const char *reason) {
    fflush(stdout);
    if (logos_fail_hook) logos_fail_hook(reason);
    fprintf(stderr, "%s\n", reason);
    exit(1);
}

__attribute__((noreturn)) static void logos_fatalf(const char *fmt, long long arg) {
    char buf[256];
    snprintf(buf, sizeof buf, fmt, arg);
    logos_fatal(lc_gc_strdup(buf));
}

/* Rust's `char::is_whitespace`. */
static int logos_is_space(unsigned cp) {
    return (cp >= 0x09 && cp <= 0x0D) || cp == 0x20 || cp == 0x85 || cp == 0xA0 || cp == 0x1680
        || (cp >= 0x2000 && cp <= 0x200A) || cp == 0x2028 || cp == 0x2029 || cp == 0x202F || cp == 0x205F
        || cp == 0x3000;
}

/* The code point UTF-8 encodes at `s` into `cp`, answering its length in bytes. */
static int logos_utf8_at(const unsigned char *s, unsigned *cp) {
    if (s[0] < 0x80) { *cp = s[0]; return 1; }
    if (s[0] < 0xE0) { *cp = ((s[0] & 0x1Fu) << 6) | (s[1] & 0x3Fu); return 2; }
    if (s[0] < 0xF0) { *cp = ((s[0] & 0x0Fu) << 12) | ((s[1] & 0x3Fu) << 6) | (s[2] & 0x3Fu); return 3; }
    *cp = ((s[0] & 0x07u) << 18) | ((s[1] & 0x3Fu) << 12) | ((s[2] & 0x3Fu) << 6) | (s[3] & 0x3Fu);
    return 4;
}

/* `str::trim` — a copy of `s` without its leading and trailing whitespace. */
static char *logos_trim(const char *s) {
    const unsigned char *b = (const unsigned char *)s, *e = b + strlen(s);
    unsigned cp;
    while (b < e) {
        int n = logos_utf8_at(b, &cp);
        if (!logos_is_space(cp)) break;
        b += n;
    }
    while (e > b) {
        const unsigned char *p = e - 1;
        while (p > b && (*p & 0xC0) == 0x80) p--;
        logos_utf8_at(p, &cp);
        if (!logos_is_space(cp)) break;
        e = p;
    }
    size_t n = (size_t)(e - b);
    char *out = (char *)lc_gc_alloc_atomic(n + 1);
    memcpy(out, b, n);
    out[n] = '\0';
    return out;
}

/* The sentences an operation that fails stops the program with — the VM's own, which the emitter takes from
   the VM and emits beside the program, and which a helper reads only on its way to stopping:
   `logos_index_refusal` is a format of the index (`%lld`), `logos_missing_key_refusal`, `logos_parse_int_refusal`
   and `logos_parse_float_refusal` of a text (`%s`), `logos_random_bounds_refusal` of the two bounds
   (`%1$lld`, `%2$lld`), `logos_utf8_invalid_refusal` of a sequence's length and start (`%1$d`, `%2$lld`) and
   `logos_utf8_incomplete_refusal` of a start (`%lld`). */
extern const char logos_index_refusal[], logos_missing_key_refusal[], logos_exact_division_refusal[],
    logos_float_division_refusal[],
    logos_negative_exponent_refusal[], logos_exponent_too_large_refusal[], logos_parse_int_refusal[],
    logos_parse_float_refusal[], logos_random_bounds_refusal[], logos_utf8_invalid_refusal[],
    logos_utf8_incomplete_refusal[], logos_await_without_listen_refusal[];

/* The 0-based cell of index `i` among `len` that is not a 1-based one in range — the rest of the VM's
   `resolve_index`: `-i` counts from the back, and any other index stops the program. Every accessor tests the
   1-based index in range itself, inline, and calls this out of line for everything else. */
__attribute__((noinline, cold)) static long long logos_index_from_back(long long i, long long len) {
    if (i <= -1 && (unsigned long long)0 - (unsigned long long)i <= (unsigned long long)len) return len + i;
    logos_fatalf(logos_index_refusal, i);
}

/* The 0-based cell of 1-based index `i` among `len` (the VM's `resolve_index`). */
static inline long long logos_resolve_index(long long i, long long len) {
    if (__builtin_expect(i < 1 || i > len, 0)) return logos_index_from_back(i, len);
    return i - 1;
}

/* Render an f64 into a fresh string EXACTLY like Rust's `{}` Display (see logos_show_f64). */
char *logos_fmt_f64(double x) {
    if (isnan(x)) return lc_gc_strdup("NaN");
    if (isinf(x)) return lc_gc_strdup(x < 0 ? "-inf" : "inf");

    /* Shortest significant-digit count whose scientific form round-trips. */
    char sci[64];
    int prec;
    for (prec = 0; prec <= 17; prec++) {
        snprintf(sci, sizeof sci, "%.*e", prec, x);
        if (strtod(sci, NULL) == x) break;
    }

    /* Parse sci = [-]D[.FFF]e[+-]EE into a sign, a digit string, and a decimal exponent. */
    const char *p = sci;
    char sign = 0;
    if (*p == '-') { sign = '-'; p++; }
    char digits[40];
    int nd = 0;
    digits[nd++] = *p++;                 /* leading digit D */
    if (*p == '.') { p++; while (*p != 'e' && *p != 'E') digits[nd++] = *p++; }
    p++;                                  /* skip 'e' */
    int exp = atoi(p);                    /* decimal point sits after (exp+1) digits from the start */
    int pointpos = exp + 1;

    /* Reformat as fixed decimal (no exponent), matching Rust's Display. */
    char *out = (char *)lc_gc_alloc_atomic(600);
    int oi = 0;
    if (sign) out[oi++] = '-';
    if (pointpos <= 0) {
        out[oi++] = '0';
        out[oi++] = '.';
        for (int i = 0; i < -pointpos; i++) out[oi++] = '0';
        for (int i = 0; i < nd; i++) out[oi++] = digits[i];
    } else if (pointpos >= nd) {
        for (int i = 0; i < nd; i++) out[oi++] = digits[i];
        for (int i = 0; i < pointpos - nd; i++) out[oi++] = '0';
    } else {
        for (int i = 0; i < nd; i++) {
            if (i == pointpos) out[oi++] = '.';
            out[oi++] = digits[i];
        }
    }
    out[oi] = '\0';
    return out;
}

void logos_show_f64(double x) {
    char *s = logos_fmt_f64(x);
    printf("%s\n", s);
    lc_gc_free(s);
}

/* ---- Text (heap char*) ---- */

/* Stringify one slot value to a fresh string, by its static kind (0=Int, 1=Float, 2=Bool,
 * 3=Text) — the AOT analog of `to_display_string`. `bits` is the raw 64-bit slot content (an i64, an
 * f64 bit pattern, 0/1, or a `char*`). */
"#,
    crate::codegen_c::runtime::exact_numbers_source!(),
    r#"
/* ---- Rational: a handle to an exact number of the section above (an `LDec`), which follows the VM's rules for
   two exact numbers — a whole rational answers `/`, `%`, `//`, `**` and the bitwise operators as the integer it
   equals. An operand is a rational handle (`flag` 1) or a machine Int (`flag` 0). ---- */

static long long logos_ldec_box(LDec d) {
    LDec *p = (LDec *)lc_gc_alloc(sizeof(LDec));
    *p = d;
    return (long long)p;
}

static LDec logos_ldec_of(long long v, int flag) { return flag ? *(const LDec *)v : ldec_from_i64(v); }

/* An Int as the rational it equals. */
long long logos_ldec_of_int(long long v) { return logos_ldec_box(ldec_from_i64(v)); }

/* `a op b`, a rational: op 0 `+`, 1 `-`, 2 `*`, 3 `/`, 4 the exact `/`, 5 `//`, 6 `%`, 7 `**`. */
long long logos_ldec_op(long long a, int af, long long b, int bf, int op) {
    LDec x = logos_ldec_of(a, af), y = logos_ldec_of(b, bf);
    switch (op) {
        case 0: return logos_ldec_box(ldec_add(x, y));
        case 1: return logos_ldec_box(ldec_sub(x, y));
        case 2: return logos_ldec_box(ldec_mul(x, y));
        case 3: return logos_ldec_box(ldec_quotient(x, y));
        case 4:
            if (ldec_is_zero(&y)) logos_fatal(logos_exact_division_refusal);
            return logos_ldec_box(ldec_div(x, y));
        case 5: return logos_ldec_box(ldec_floor_quotient(x, y));
        case 6: return logos_ldec_box(ldec_remainder(x, y));
        default: return logos_ldec_box(ldec_power(x, y));
    }
}

/* `a & b`, `a | b`, `a ^ b`, `a << b`, `a >> b` (`op` '&', '|', '^', '<', '>') over two whole numbers, an Int. */
long long logos_ldec_bitwise(long long a, int af, long long b, int bf, int op) {
    return ldec_bitwise(logos_ldec_of(a, af), logos_ldec_of(b, bf), (char)op);
}

/* The order of two exact numbers: -1, 0 or 1. */
int logos_ldec_cmp(long long a, int af, long long b, int bf) { return ldec_cmp(logos_ldec_of(a, af), logos_ldec_of(b, bf)); }

/* The order of a rational against a real by the real's exact value: -1, 0 or 1, and 2 when the real is NaN. */
int logos_ldec_cmp_f64(long long a, double f) { return ldec_cmp_f64(*(const LDec *)a, f); }

/* The double the VM converts a rational to beside a real. */
double logos_ldec_to_f64(long long a) { return ldec_to_f64(*(const LDec *)a); }

/* `a op f`, or `f op a` when the rational is on the right (`rational_left` 0), a rational beside a real, as the VM
   computes it on the rational's double: op 0 `+`, 1 `-`, 2 `*`, 3 `/`, 4 `**`. */
double logos_ldec_real_op(long long a, double f, int rational_left, int op) {
    double x = ldec_to_f64(*(const LDec *)a);
    double l = rational_left ? x : f, r = rational_left ? f : x;
    switch (op) {
        case 0: return l + r;
        case 1: return l - r;
        case 2: return l * r;
        case 3:
            if (r == 0.0) logos_fatal(logos_float_division_refusal);
            return l / r;
        default: return pow(l, r);
    }
}

/* A rational where the VM takes a machine integer: the integer a whole one is, else the place's refusal. */
long long logos_ldec_as_i64(long long a, const char *past_word, const char *fraction) {
    return ldec_as_i64(*(const LDec *)a, past_word, fraction);
}

/* `num/den`, or the integer a whole rational equals. */
char *logos_ldec_str(long long handle) { return ldec_to_string((const LDec *)handle); }

/* ---- Temporal: Date (days since epoch) / Moment (nanos since epoch), ≡ the VM's Show ---- */

/* Floored (Euclidean) division — a pre-epoch Moment yields the correct date + a 0..86399 time-of-day. */
static long long logos_fdiv(long long a, long long b) {
    long long q = a / b;
    if ((a % b != 0) && ((a < 0) != (b < 0))) q--;
    return q;
}

/* Civil date from days-since-epoch (Howard Hinnant's algorithm, matching `RuntimeValue::Date`). */
static void logos_civil(long long days, long long *year, long long *mon, long long *day) {
    long long z = days + 719468;
    long long era = (z >= 0 ? z : z - 146096) / 146097;
    long long doe = z - era * 146097;
    long long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    long long y = yoe + era * 400;
    long long doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    long long mp = (5 * doy + 2) / 153;
    long long d = doy - (153 * mp + 2) / 5 + 1;
    long long m = mp + (mp < 10 ? 3 : -9);
    *year = y + (m <= 2 ? 1 : 0);
    *mon = m;
    *day = d;
}

char *logos_date_str(long long days) {
    long long y, m, d;
    logos_civil(days, &y, &m, &d);
    char *b = (char *)lc_gc_alloc_atomic(32);
    snprintf(b, 32, "%04lld-%02lld-%02lld", y, m, d);
    return b;
}

char *logos_moment_str(long long nanos) {
    long long total_seconds = logos_fdiv(nanos, 1000000000LL);
    long long days = logos_fdiv(total_seconds, 86400LL);
    long long day_seconds = total_seconds - days * 86400LL;
    long long hours = day_seconds / 3600;
    long long minutes = (day_seconds % 3600) / 60;
    long long y, m, d;
    logos_civil(days, &y, &m, &d);
    char *b = (char *)lc_gc_alloc_atomic(48);
    snprintf(b, 48, "%04lld-%02lld-%02lld %02lld:%02lld", y, m, d, hours, minutes);
    return b;
}

/* `today` / `now` — the injected fixed clock (env `LOGOS_FIXED_DAYS`/`LOGOS_FIXED_NANOS`, set by the
   differential harness so the AOT binary and the VM oracle read the SAME instant) else the real clock. */
long long logos_today(void) {
    const char *e = getenv("LOGOS_FIXED_DAYS");
    if (e) return atoll(e);
    return (long long)(time(NULL) / 86400);
}

long long logos_now(void) {
    const char *e = getenv("LOGOS_FIXED_NANOS");
    if (e) return atoll(e);
    return (long long)time(NULL) * 1000000000LL;
}

/* time.lg's `now()` — the Unix time in milliseconds from the real clock, as the interpreter reads it. */
long long logos_time_now(void) {
    struct timespec ts;
    timespec_get(&ts, TIME_UTC);
    return (long long)ts.tv_sec * 1000 + (long long)(ts.tv_nsec / 1000000);
}

/* env.lg's `get(name)`, in the halves the VM expands it into: whether the process environment sets the
   variable, and its text (`""` when it is unset). */
long long logos_env_has(const char *name) {
    return getenv(name) != NULL;
}

char *logos_env_value(const char *name) {
    const char *v = getenv(name);
    return lc_gc_strdup(v ? v : "");
}

/* How a cell of the exact extension displays, set by the extension's adapter when the shim embeds it
   (`codegen_riscv::runtime`); a program that holds no cell never reads it. */
static char *(*logos_tower_text_hook)(long long);

char *logos_val_to_str(long long bits, int kind) {
    switch (kind) {
        case 0: { char *b = (char *)lc_gc_alloc_atomic(24); snprintf(b, 24, "%lld", bits); return b; }
        case 1: { double f; memcpy(&f, &bits, 8); return logos_fmt_f64(f); }
        case 2: return lc_gc_strdup(bits ? "true" : "false");
        case 4: return logos_ldec_str(bits); /* Rational */
        case 5: return logos_date_str(bits); /* Date */
        case 6: return logos_moment_str(bits); /* Moment */
        case 7: return lc_gc_strdup("nothing"); /* a value-or-nothing holding nothing */
        case 8: { /* a PeerAgent: `<peer topic>` */
            size_t n = strlen((const char *)bits) + 8;
            char *b = (char *)lc_gc_alloc_atomic(n);
            snprintf(b, n, "<peer %s>", (const char *)bits);
            return b;
        }
        case 9: return logos_tower_text_hook(bits); /* a cell of the exact extension */
        default: return lc_gc_strdup((const char *)bits); /* Text: copy so the result is independently owned */
    }
}

/* ---- A Text a register owns: `t = t + v` in place ----
   While a register's owned flag is set, its Text is a block of its own — its length and capacity just before
   the characters — which an append grows in place, amortized. Any other Text is copied into such a block
   first, so an alias never sees the change. The characters stay an ordinary C string. */
typedef struct { size_t len, cap; } logos_text_block;

/* `t = t + v` into the register at `slot`: `v`'s text by its kind tag. `v` is read before anything moves. */
void logos_text_append_own(char **slot, long long *owned, long long v, int tag) {
    char *made = tag == 3 ? NULL : logos_val_to_str(v, tag);
    const char *add = made ? made : (const char *)v;
    size_t n = strlen(add);
    logos_text_block *b;
    if (*owned) {
        b = (logos_text_block *)*slot - 1;
    } else {
        size_t len = strlen(*slot);
        b = (logos_text_block *)lc_gc_alloc_atomic(sizeof *b + len + n + 1);
        b->len = len;
        b->cap = len + n;
        memcpy(b + 1, *slot, len + 1);
        *owned = 1;
    }
    if (b->len + n > b->cap) {
        size_t cap = b->cap * 2 > b->len + n ? b->cap * 2 : b->len + n;
        b = (logos_text_block *)lc_gc_realloc_atomic(b, sizeof *b + cap + 1);
        b->cap = cap;
    }
    memcpy((char *)(b + 1) + b->len, add, n + 1);
    b->len += n;
    *slot = (char *)(b + 1);
    lc_gc_free(made);
}

/* `Op::Concat` — stringify both operands (by kind) and join, matching the VM's `arith::concat`. */
char *logos_concat_vals(long long a, int ak, long long b, int bk) {
    char *sa = logos_val_to_str(a, ak);
    char *sb = logos_val_to_str(b, bk);
    size_t la = strlen(sa), lb = strlen(sb);
    char *out = (char *)lc_gc_alloc_atomic(la + lb + 1);
    memcpy(out, sa, la);
    memcpy(out + la, sb, lb);
    out[la + lb] = '\0';
    lc_gc_free(sa);
    lc_gc_free(sb);
    return out;
}

/* `Show` of a Text value — raw contents + newline, exactly like the VM. */
void logos_show_text(const char *s) { printf("%s\n", s); }

/* `Fail with <value>.` — the value exactly as `Show` renders it. */
void logos_fail_val(long long bits, int kind) { logos_fatal(logos_val_to_str(bits, kind)); }

/* ---- Program arguments (`args()`) + `parseInt` ---- */
static int logos_argc = 0;
static char **logos_argv = NULL;
void logos_set_args(int argc, char **argv) { logos_argc = argc; logos_argv = argv; }

/* A failure naming a Text: `refusal` is a format of it (`%s`). */
__attribute__((noreturn)) static void logos_fatals(const char *refusal, const char *text) {
    size_t n = strlen(refusal) + strlen(text) + 1;
    char *why = (char *)lc_gc_alloc_atomic(n);
    snprintf(why, n, refusal, text);
    logos_fatal(why);
}

/* `parseInt(t)` — the VM's: `t` trimmed (`str::trim`), then the whole of it an `i64` in base 10 as
   `str::parse` reads one (an optional sign, then digits, in range). Anything else stops the program with
   `logos_parse_int_refusal`. */
long long logos_parse_int(const char *s) {
    const char *refusal = logos_parse_int_refusal;
    const char *p = logos_trim(s);
    int negative = *p == '-';
    if (*p == '+' || *p == '-') p++;
    if (!*p) logos_fatals(refusal, s);
    unsigned long long v = 0, limit = negative ? 9223372036854775808ULL : 9223372036854775807ULL;
    for (; *p; p++) {
        if (*p < '0' || *p > '9') logos_fatals(refusal, s);
        unsigned long long d = (unsigned long long)(*p - '0');
        if (v > (limit - d) / 10) logos_fatals(refusal, s);
        v = v * 10 + d;
    }
    return negative ? (long long)(0 - v) : (long long)v;
}

/* Whether `s` spells `word`, ASCII case folded. */
static int logos_ascii_is(const char *s, const char *word) {
    for (; *word; s++, word++) {
        char c = *s >= 'A' && *s <= 'Z' ? (char)(*s - 'A' + 'a') : *s;
        if (c != *word) return 0;
    }
    return !*s;
}

/* `parseFloat(t)` — the VM's: `t` trimmed, then the whole of it an `f64` as `str::parse` reads one: an
   optional sign, then `inf`, `infinity` or `nan` in any case, or digits with at most one point among them
   and at least one digit, then an optional exponent (`e`, an optional sign, digits). Anything else stops the
   program with `logos_parse_float_refusal`. The value is the correctly rounded one (`strtod`). */
double logos_parse_float(const char *s) {
    const char *refusal = logos_parse_float_refusal;
    const char *t = logos_trim(s), *p = t;
    int negative = *p == '-';
    if (*p == '+' || *p == '-') p++;
    if (logos_ascii_is(p, "inf") || logos_ascii_is(p, "infinity")) return negative ? -INFINITY : INFINITY;
    if (logos_ascii_is(p, "nan")) return negative ? -NAN : NAN;
    int digits = 0;
    while (*p >= '0' && *p <= '9') { p++; digits++; }
    if (*p == '.') {
        p++;
        while (*p >= '0' && *p <= '9') { p++; digits++; }
    }
    if (!digits) logos_fatals(refusal, s);
    if (*p == 'e' || *p == 'E') {
        p++;
        if (*p == '+' || *p == '-') p++;
        if (!(*p >= '0' && *p <= '9')) logos_fatals(refusal, s);
        while (*p >= '0' && *p <= '9') p++;
    }
    if (*p) logos_fatals(refusal, s);
    return strtod(t, NULL);
}

/* `chr(codepoint)` — a single-character UTF-8 Text handle, matching the VM's `char::from_u32`. */
char *logos_chr(long long cp) {
    char *out = (char *)lc_gc_alloc_atomic(5);
    unsigned int c = (unsigned int)cp;
    int n = 0;
    if (c < 0x80) { out[n++] = (char)c; }
    else if (c < 0x800) { out[n++] = (char)(0xC0 | (c >> 6)); out[n++] = (char)(0x80 | (c & 0x3F)); }
    else if (c < 0x10000) {
        out[n++] = (char)(0xE0 | (c >> 12));
        out[n++] = (char)(0x80 | ((c >> 6) & 0x3F));
        out[n++] = (char)(0x80 | (c & 0x3F));
    } else {
        out[n++] = (char)(0xF0 | (c >> 18));
        out[n++] = (char)(0x80 | ((c >> 12) & 0x3F));
        out[n++] = (char)(0x80 | ((c >> 6) & 0x3F));
        out[n++] = (char)(0x80 | (c & 0x3F));
    }
    out[n] = '\0';
    return out;
}

/* ---- CRDT counters (G-Counter / PN-Counter) — a per-replica count map ---- */
/* The observable is the SUM; merge/join take the per-replica MAX (IDEMPOTENT — a redelivered state
   never double-counts), so it converges regardless of order/duplication. The replica ids are drawn
   from a local monotone sequence: the differential observable (the sum) depends only on the ids being
   DISTINCT and STABLE within a run, not their absolute values, so it matches the VM whatever the VM's
   own (process-global) replica sequence happens to be. */

typedef struct { long long my_rid; long long len, cap; long long *rids; long long *cnts; } logos_crdt;

static long long logos_replica_seq = 0;

logos_crdt *logos_crdt_new(void) {
    logos_crdt *c = (logos_crdt *)lc_gc_alloc(sizeof(logos_crdt));
    c->my_rid = logos_replica_seq++;
    c->len = 1; c->cap = 4;
    c->rids = (long long *)lc_gc_alloc_atomic((size_t)c->cap * 8);
    c->cnts = (long long *)lc_gc_alloc_atomic((size_t)c->cap * 8);
    c->rids[0] = c->my_rid; c->cnts[0] = 0;
    return c;
}

/* Increment (or decrement) THIS counter's own replica entry, in place. */
void logos_crdt_bump(logos_crdt *c, long long amount, int negate) {
    long long d = negate ? -amount : amount;
    for (long long i = 0; i < c->len; i++) {
        if (c->rids[i] == c->my_rid) { c->cnts[i] += d; return; }
    }
    /* my_rid always present (seeded in _new); defensive. */
    c->cnts[0] += d;
}

long long logos_crdt_sum(const logos_crdt *c) {
    long long s = 0;
    for (long long i = 0; i < c->len; i++) s += c->cnts[i];
    return s;
}

static void logos_crdt_put_max(logos_crdt *d, long long rid, long long cnt) {
    for (long long i = 0; i < d->len; i++) {
        if (d->rids[i] == rid) { if (cnt > d->cnts[i]) d->cnts[i] = cnt; return; }
    }
    if (d->len == d->cap) {
        d->cap *= 2;
        d->rids = (long long *)lc_gc_realloc_atomic(d->rids, (size_t)d->cap * 8);
        d->cnts = (long long *)lc_gc_realloc_atomic(d->cnts, (size_t)d->cap * 8);
    }
    d->rids[d->len] = rid; d->cnts[d->len] = cnt; d->len++;
}

/* Lattice join: per-replica MAX of `s` into `d`, in place (idempotent, commutative, associative). */
void logos_crdt_merge(logos_crdt *d, const logos_crdt *s) {
    for (long long i = 0; i < s->len; i++) logos_crdt_put_max(d, s->rids[i], s->cnts[i]);
}

/* `copy` of a counter: the same state, the same replica. */
logos_crdt *logos_crdt_clone(const logos_crdt *c) {
    logos_crdt *d = (logos_crdt *)lc_gc_alloc(sizeof(logos_crdt));
    *d = *c;
    d->rids = (long long *)lc_gc_alloc_atomic((size_t)c->cap * 8);
    d->cnts = (long long *)lc_gc_alloc_atomic((size_t)c->cap * 8);
    memcpy(d->rids, c->rids, (size_t)c->len * 8);
    memcpy(d->cnts, c->cnts, (size_t)c->len * 8);
    return d;
}

/* ---- Closures — a heap `[body_index, captured values…, present flags…]` cell array ---- */

long long *logos_closure_new(long long body, long long ncap) {
    long long *c = (long long *)lc_gc_alloc((size_t)(1 + 2 * ncap) * 8);
    c[0] = body;
    return c;
}

void logos_closure_set(long long *c, long long idx, long long val) { c[idx] = val; }

/* ---- `FormatValue` spec helpers (the spec is a compile-time constant, dispatched in codegen) ---- */

/* Currency `$` — `${:.2}` (two decimals). Matches `apply_format_spec`'s float path. */
char *logos_fmt_currency(double f) {
    char *b = (char *)lc_gc_alloc_atomic(64);
    snprintf(b, 64, "$%.2f", f);
    return b;
}

/* Precision `.N` — `{:.N}` fixed-decimal float (round-to-nearest-even, matching Rust's Display). */
char *logos_fmt_fixed(double f, int prec) {
    char *b = (char *)lc_gc_alloc_atomic(64 + (size_t)prec);
    snprintf(b, 64 + (size_t)prec, "%.*f", prec, f);
    return b;
}

/* Alignment/width — pad `s` to `width` columns: dir '>' right, '<' left, '^' center (extra column on
   the RIGHT, matching Rust's `{:^w}`). No truncation when `s` is already wide enough. */
char *logos_pad(const char *s, long long width, char dir) {
    size_t len = strlen(s);
    if ((long long)len >= width) return lc_gc_strdup(s);
    size_t pad = (size_t)width - len;
    char *out = (char *)lc_gc_alloc_atomic((size_t)width + 1);
    size_t left = dir == '>' ? pad : dir == '^' ? pad / 2 : 0;
    size_t right = pad - left;
    memset(out, ' ', left);
    memcpy(out + left, s, len);
    memset(out + left + len, ' ', right);
    out[width] = '\0';
    return out;
}

/* EXACT i64-vs-f64 comparison for a mixed Int/Float relational op — replicates the VM's
 * `cmp_i64_f64_exact` + `rel` (crates/logicaffeine_base/src/numeric.rs, semantics/compare.rs). `i`/`f`
 * are the numeric operands (the Int first regardless of source order); `reversed` = 1 when the SOURCE
 * order was (Float, Int) so the comparison is (f vs i); `op` selects lt=0/gt=1/le=2/ge=3/eq=4/ne=5.
 * Returns 0/1. A NaN operand is unordered: every ordering/eq is false, `!=` is true — matching `rel`. */
int logos_cmp_i64_f64_rel(long long i, double f, int reversed, int op) {
    int ord; /* -2 unordered, else -1 (i<f) / 0 (i==f) / 1 (i>f) */
    if (isnan(f)) {
        ord = -2;
    } else if (isinf(f)) {
        ord = (f > 0) ? -1 : 1;
    } else if (i <= (1LL << 53) && i >= -(1LL << 53)) {
        double di = (double)i;                 /* |i| <= 2^53 is exact */
        ord = di < f ? -1 : (di > f ? 1 : 0);
    } else if (f >= 9223372036854775808.0) {   /* f >= 2^63 > any i64 */
        ord = -1;
    } else if (f < -9223372036854775808.0) {   /* f < -2^63 < any i64 */
        ord = 1;
    } else {
        long long t = (long long)f;            /* trunc toward zero, exact for f in i64 range */
        ord = i < t ? -1 : (i > t ? 1 : 0);
    }
    if (reversed && ord != -2) ord = -ord;
    switch (op) {
        case 0: return ord == -1;              /* lt */
        case 1: return ord == 1;               /* gt */
        case 2: return ord == -1 || ord == 0;  /* le  (None -> false) */
        case 3: return ord == 0 || ord == 1;   /* ge  (None -> false) */
        case 4: return ord == 0;               /* eq  (None -> false) */
        default: return ord != 0;              /* ne  (None -> true)  */
    }
}


/* ---- Seq (heap growable array of uniform 8-byte cells) ----
 * Every scalar element (i64 / f64-bits / char*) is one 8-byte cell; the element KIND is tracked by the
 * emitter and only needed for Show. VALUE SEMANTICS BY OWNERSHIP: `Move` copies the handle (share), and a
 * register's collection is mutated only through a `_own` helper handed the register's SLOT and its OWNED
 * flag. A clear flag means the handle may be reachable from somewhere else (an alias, a global, a field, an
 * iterator, a closure), so the helper first copies the collection into the slot and sets the flag; then it
 * mutates in place. An alias or an in-flight iterator keeps its own list — the VM's copy-on-write and
 * snapshot — and a register that owns its collection mutates it in amortized O(1). The emitter keeps the
 * flag: an op that makes a fresh collection sets it, any op that could leave the handle elsewhere clears it. */
typedef struct { long long len, cap; long long *data; } logos_seq;

/* An empty sequence with room for `capacity` cells, in one block: the cells follow the header until the sequence
   first outgrows them (`logos_seq_room`). A record, a tuple and an enum value never grow, so each is one block. */
logos_seq *logos_seq_new_cap(long long capacity) {
    if (capacity < 0) capacity = 0;
    logos_seq *s = (logos_seq *)lc_gc_alloc(sizeof(logos_seq) + (size_t)capacity * 8);
    s->len = 0;
    s->cap = capacity;
    s->data = capacity ? (long long *)(s + 1) : NULL;
    return s;
}

/* Room for `cap` cells: cells that still follow the header move to a block of their own, any other block resizes. */
static void logos_seq_room(logos_seq *a, long long cap) {
    if (a->data == (long long *)(a + 1)) {
        long long *cells = (long long *)lc_gc_alloc((size_t)cap * 8);
        memcpy(cells, a->data, (size_t)a->len * 8);
        a->data = cells;
    } else {
        a->data = (long long *)lc_gc_realloc(a->data, (size_t)cap * 8);
    }
    a->cap = cap;
}

/* A fresh sequence, with room for the cells its first pushes add: an enum value's tag and payload, a tuple's items,
   a list's first items all go in its one block. */
logos_seq *logos_seq_new(void) { return logos_seq_new_cap(4); }

static logos_seq *logos_seq_dup(const logos_seq *a) {
    logos_seq *s = logos_seq_new_cap(a->len);
    s->len = a->len;
    if (a->len) memcpy(s->data, a->data, (size_t)a->len * 8);
    return s;
}

/* In-place append, into a collection its caller has made its own; the header pointer identity stays stable
   (only the cells move), and growth is taken out of line. */
__attribute__((noinline)) static void logos_seq_push_growing(logos_seq *a, long long val) {
    logos_seq_room(a, a->cap ? a->cap * 2 : 4);
    a->data[a->len++] = val;
}

void logos_seq_push_inplace(logos_seq *a, long long val) {
    if (__builtin_expect(a->len == a->cap, 0)) {
        logos_seq_push_growing(a, val);
        return;
    }
    a->data[a->len++] = val;
}

/* The collection in `*slot`, made the register's own: a copy when `*owned` is clear. */
static logos_seq *logos_seq_own(logos_seq **slot, long long *owned) {
    if (!*owned) { *slot = logos_seq_dup(*slot); *owned = 1; }
    return *slot;
}

/* The register's own collection, for an op that changes several of its cells in place (`Merge`): the
   emitter calls it only when the register's owned flag is clear. */
logos_seq *logos_seq_make_own(logos_seq **slot, long long *owned) {
    return logos_seq_own(slot, owned);
}

__attribute__((noinline)) static void logos_seq_push_copying(logos_seq **slot, long long *owned, long long val) {
    logos_seq_push_inplace(logos_seq_own(slot, owned), val);
}

/* `Push val to xs`. `val` arrives by value, read before the register is made its own. */
void logos_seq_push_own(logos_seq **slot, long long *owned, long long val) {
    if (__builtin_expect(!*owned, 0)) {
        logos_seq_push_copying(slot, owned, val);
        return;
    }
    logos_seq_push_inplace(*slot, val);
}

/* `args()` → a `Seq of Text` of the process argv (index 1 is the program name, exactly as the VM's
   `program_args`), so `item 2 of args()` is the first real argument. */
logos_seq *logos_args(void) {
    logos_seq *s = logos_seq_new();
    for (int i = 0; i < logos_argc; i++) logos_seq_push_inplace(s, (long long)logos_argv[i]);
    return s;
}

long long logos_seq_len(const logos_seq *a) { return a->len; }

/* `Pop from xs into v` — the last element, removed (`present` 1), or `nothing` from an empty list (`present`
   0). An empty pop changes nothing, so it leaves the register's ownership as it found it. */
/* The call sites whose callee stops the program itself rather than answer nothing: pairs of a return
   address and the sentence to stop with, ended by a zero pair. The emitter writes the table; a helper
   searches it only on its way to stopping. */
extern const struct { const void *site; const char *reason; } logos_fail_sites[];

__attribute__((noreturn, noinline)) static void logos_fail_at(const void *site) {
    for (long long i = 0; logos_fail_sites[i].site; i++) {
        if (logos_fail_sites[i].site == site) logos_fatal(logos_fail_sites[i].reason);
    }
    abort();
}

typedef struct { long long present, value; } logos_popped;
logos_popped logos_seq_pop_own(logos_seq **slot, long long *owned) {
    logos_popped r = {0, 0};
    if (!(*slot)->len) return r;
    logos_seq *s = logos_seq_own(slot, owned);
    r.present = 1;
    r.value = s->data[--s->len];
    return r;
}

/* `Pop from xs into v` where the op after it fails on `nothing`: the last element, or, from an empty list,
   the program stopped with the sentence registered for the call site. */
__attribute__((noinline)) long long logos_seq_pop_value(logos_seq **slot, long long *owned) {
    if (!(*slot)->len) logos_fail_at(__builtin_return_address(0));
    logos_seq *s = logos_seq_own(slot, owned);
    return s->data[--s->len];
}

/* In-place indexed write (1-based) — a struct-field mutation that must be visible through every alias
   of the struct (`CrdtBump`'s counter update; the seq header pointer stays stable). */
__attribute__((noinline, cold)) static void logos_seq_set_from_back(logos_seq *a, long long i, long long val) {
    a->data[logos_index_from_back(i, a->len)] = val;
}

void logos_seq_set_inplace(logos_seq *a, long long i, long long val) {
    if (__builtin_expect(i < 1 || i > a->len, 0)) {
        logos_seq_set_from_back(a, i, val);
        return;
    }
    a->data[i - 1] = val;
}

/* `Set item i of xs to val` (1-based, or from the back when negative), and a struct's field write (its cells
   are a seq). An index out of range stops the program as the VM does, before anything is copied. */
__attribute__((noinline)) static void logos_seq_set_own_copying(logos_seq **slot, long long *owned, long long i, long long val) {
    long long at = logos_resolve_index(i, (*slot)->len);
    logos_seq_own(slot, owned)->data[at] = val;
}

void logos_seq_set_own(logos_seq **slot, long long *owned, long long i, long long val) {
    logos_seq *s = *slot;
    if (__builtin_expect(*owned && i >= 1 && i <= s->len, 1)) {
        s->data[i - 1] = val;
        return;
    }
    logos_seq_set_own_copying(slot, owned, i, val);
}

/* The value in field `cell` (1-based) of the record in `*slot`, made the register's own record's own: the record
   copied unless `*owned` says the register is its only holder, the value copied with `copy` unless
   `*field_owned` says the record is its only holder, and both flags set. */
__attribute__((noinline)) static long long logos_field_own(logos_seq **slot, long long *owned, long long *field_owned,
    long long cell, long long (*copy)(long long)) {
    logos_seq *record = logos_seq_own(slot, owned);
    long long value = record->data[cell - 1];
    if (!*field_owned) {
        value = copy(value);
        record->data[cell - 1] = value;
        *field_owned = 1;
    }
    return value;
}

static long long logos_field_seq_copy(long long list) { return (long long)logos_seq_dup((const logos_seq *)list); }

__attribute__((noinline)) static void logos_field_push_copying(logos_seq **slot, long long *owned, long long *field_owned,
    long long cell, long long val) {
    logos_seq_push_inplace((logos_seq *)logos_field_own(slot, owned, field_owned, cell, logos_field_seq_copy), val);
}

/* `Push val to r's field` — into the register's own record's own list: in place while the field flag is set
   (the emitter keeps a field flag set only while its register's owned flag is), so a loop of pushes copies at
   most once. */
void logos_field_push_own(logos_seq **slot, long long *owned, long long *field_owned, long long cell, long long val) {
    if (__builtin_expect(!*field_owned, 0)) {
        logos_field_push_copying(slot, owned, field_owned, cell, val);
        return;
    }
    logos_seq_push_inplace((logos_seq *)(*slot)->data[cell - 1], val);
}

static long long logos_field_crdt_copy(long long c) { return (long long)logos_crdt_clone((const logos_crdt *)c); }

__attribute__((noinline)) static void logos_field_bump_copying(logos_seq **slot, long long *owned, long long *field_owned,
    long long cell, long long amount, int negate) {
    logos_crdt_bump((logos_crdt *)logos_field_own(slot, owned, field_owned, cell, logos_field_crdt_copy), amount, negate);
}

/* `Increase/Decrease r's counter by amount` — the register's own record's own counter: in place while the
   field flag is set. */
void logos_field_bump_own(logos_seq **slot, long long *owned, long long *field_owned, long long cell, long long amount,
    int negate) {
    if (__builtin_expect(!*field_owned, 0)) {
        logos_field_bump_copying(slot, owned, field_owned, cell, amount, negate);
        return;
    }
    logos_crdt_bump((logos_crdt *)(*slot)->data[cell - 1], amount, negate);
}

/* `a followed by b` — fresh seq with a's elements then b's. */
logos_seq *logos_seq_concat(const logos_seq *a, const logos_seq *b) {
    logos_seq *s = logos_seq_new_cap(a->len + b->len);
    s->len = a->len + b->len;
    if (a->len) memcpy(s->data, a->data, (size_t)a->len * 8);
    if (b->len) memcpy(s->data + a->len, b->data, (size_t)b->len * 8);
    return s;
}

/* `items start through end of a` — the VM's `slice`: the 1-based inclusive range, and empty unless it lies
   within the list (`slice.get(start - 1 .. end)`, the bounds read as unsigned). */
logos_seq *logos_seq_slice(const logos_seq *a, long long start, long long end) {
    unsigned long long from = (unsigned long long)start ? (unsigned long long)start - 1 : 0, to = (unsigned long long)end;
    logos_seq *s = logos_seq_new();
    if (from < to && to <= (unsigned long long)a->len) {
        for (unsigned long long i = from; i < to; i++) logos_seq_push_inplace(s, a->data[i]);
    }
    return s;
}

/* Element equality by kind (0=Int, 1=Float, 3=Text) — the VM's `values_equal` for scalar cells. */
static int logos_cell_eq(long long a, long long b, int kind) {
    if (kind == 3) return strcmp((const char *)a, (const char *)b) == 0;
    if (kind == 1) { double x, y; memcpy(&x, &a, 8); memcpy(&y, &b, 8); return x == y; }
    return a == b;
}

/* Two struct fields compared by kind (a policy `ObjectFieldEquals` — the VM's `values_equal`). The
   cells are 1-based; struct field indices are always in range. */
int logos_struct_field_eq(logos_seq *a, long long ai, logos_seq *b, long long bi, int kind) {
    return logos_cell_eq(a->data[ai - 1], b->data[bi - 1], kind);
}

/* `a contains val` — membership by element kind. */
int logos_seq_contains(const logos_seq *a, long long val, int kind) {
    for (long long i = 0; i < a->len; i++) {
        if (logos_cell_eq(a->data[i], val, kind)) return 1;
    }
    return 0;
}

/* `a contains val` where the items or the needle are rationals (`items_rational`, `val_rational`): by value, an Int
   by the rational it equals. */
int logos_seq_contains_exact(const logos_seq *a, int items_rational, long long val, int val_rational) {
    for (long long i = 0; i < a->len; i++) {
        if (logos_ldec_cmp(a->data[i], items_rational, val, val_rational) == 0) return 1;
    }
    return 0;
}

/* An Int put among rationals goes in as the rational it equals: a literal's item, a pushed one, one stored. */
void logos_seq_push_inplace_ldec(logos_seq *a, long long v) { logos_seq_push_inplace(a, logos_ldec_of_int(v)); }
void logos_seq_push_own_ldec(logos_seq **slot, long long *owned, long long v) {
    logos_seq_push_own(slot, owned, logos_ldec_of_int(v));
}
void logos_seq_set_own_ldec(logos_seq **slot, long long *owned, long long i, long long v) {
    logos_seq_set_own(slot, owned, i, logos_ldec_of_int(v));
}

/* A list of Ints as the list of the rationals they equal. */
logos_seq *logos_seq_as_ldec(const logos_seq *a) {
    logos_seq *s = logos_seq_new();
    for (long long i = 0; i < a->len; i++) logos_seq_push_inplace_ldec(s, a->data[i]);
    return s;
}

/* ---- Set (insertion-ordered, dedup'd — shares the logos_seq storage) ---- */

/* `Add val to s` — append if not already present (dedup). A present element changes nothing. */
void logos_set_add_own(logos_seq **slot, long long *owned, long long val, int kind) {
    if (logos_seq_contains(*slot, val, kind)) return;
    logos_seq_push_inplace(logos_seq_own(slot, owned), val);
}

/* `Remove val from s` — drop every matching element, keeping the order of the rest. An absent element
   changes nothing. */
void logos_set_remove_own(logos_seq **slot, long long *owned, long long val, int kind) {
    if (!logos_seq_contains(*slot, val, kind)) return;
    logos_seq *s = logos_seq_own(slot, owned);
    long long kept = 0;
    for (long long i = 0; i < s->len; i++) {
        if (!logos_cell_eq(s->data[i], val, kind)) s->data[kept++] = s->data[i];
    }
    s->len = kept;
}

/* `a union b` — a's elements, then b's novel elements (insertion-ordered). */
logos_seq *logos_set_union(const logos_seq *a, const logos_seq *b, int kind) {
    logos_seq *s = logos_seq_dup(a);
    for (long long i = 0; i < b->len; i++) {
        if (!logos_seq_contains(s, b->data[i], kind)) {
            if (s->len == s->cap) logos_seq_room(s, s->cap ? s->cap * 2 : 4);
            s->data[s->len++] = b->data[i];
        }
    }
    return s;
}

/* `a intersect b` — a's elements that are also in b (a's order). */
logos_seq *logos_set_intersect(const logos_seq *a, const logos_seq *b, int kind) {
    logos_seq *s = logos_seq_new();
    for (long long i = 0; i < a->len; i++) {
        if (logos_seq_contains(b, a->data[i], kind)) {
            if (s->len == s->cap) logos_seq_room(s, s->cap ? s->cap * 2 : 4);
            s->data[s->len++] = a->data[i];
        }
    }
    return s;
}

/* `Show` of a set — `{1, 2, 3}` (braces; insertion order; text elements unquoted). */
void logos_set_show_int(const logos_seq *a) {
    putchar('{');
    for (long long i = 0; i < a->len; i++) { if (i) fputs(", ", stdout); printf("%lld", a->data[i]); }
    fputs("}\n", stdout);
}
void logos_set_show_text(const logos_seq *a) {
    putchar('{');
    for (long long i = 0; i < a->len; i++) { if (i) fputs(", ", stdout); fputs((const char *)a->data[i], stdout); }
    fputs("}\n", stdout);
}

/* `copy of a` — deep clone. */
logos_seq *logos_seq_clone(const logos_seq *a) { return logos_seq_dup(a); }

/* A struct is a fixed array of `n` field cells (declaration order), zero-initialized. */
logos_seq *logos_struct_new(long long n) {
    logos_seq *s = logos_seq_new_cap(n);
    if (n > 0) memset(s->data, 0, (size_t)n * 8);
    s->len = n > 0 ? n : 0;
    return s;
}

/* Print without a trailing newline (for building `Name { f: v, ... }` piece by piece). */
void logos_print_raw(const char *s) { fputs(s, stdout); }
void logos_print_nl(void) { putchar('\n'); }

/* Granlund–Montgomery / libdivide unsigned magic division — a byte-for-byte port of the VM's
 * `magic_eval` (vm/compiler.rs). Emitted only for a constant non-power-of-two divisor with an
 * Oracle-proven non-negative dividend; `mul_back == 0` is `/`, `mul_back == c` is `%`. */
long long logos_magic_div(long long x, unsigned long long magic, int more, long long mul_back) {
    unsigned long long n = (unsigned long long)x;
    unsigned long long q;
    if (more & 0x80) {
        q = n >> (more & 0x3F);
    } else {
        unsigned long long hi = (unsigned long long)(((unsigned __int128)magic * (unsigned __int128)n) >> 64);
        if (more & 0x40) {
            unsigned long long t = ((n - hi) >> 1) + hi;
            q = t >> (more & 0x3F);
        } else {
            q = hi >> (more & 0x3F);
        }
    }
    if (mul_back == 0) return (long long)q;
    return x - (long long)q * mul_back;
}

/* Integer exponentiation by squaring (wrapping, matching the VM's in-i64 result). A negative exponent, or
 * one past 32 bits, stops the program with the VM's sentence. */
long long logos_ipow(long long base, long long exp) {
    if (exp < 0) logos_fatal(logos_negative_exponent_refusal);
    if (exp > 4294967295LL) logos_fatal(logos_exponent_too_large_refusal);
    unsigned long long r = 1, b = (unsigned long long)base, e = (unsigned long long)exp;
    while (e > 0) { if (e & 1) r *= b; b *= b; e >>= 1; }
    return (long long)r;
}

/* `lo to hi` — a materialized inclusive integer range (empty if lo > hi). */
logos_seq *logos_seq_range(long long lo, long long hi) {
    logos_seq *s = logos_seq_new();
    for (long long v = lo; v <= hi; v++) {
        if (s->len == s->cap) logos_seq_room(s, s->cap ? s->cap * 2 : 8);
        s->data[s->len++] = v;
    }
    return s;
}

/* `start to end`, and `items start through end of a`, with a bound that is a rational (`lf`, `hf`): each the
   integer a whole one is, a fraction or one past i64 refused in the VM's words for the place. */
logos_seq *logos_seq_range_ldec(long long lo, int lf, long long hi, int hf, const char *past_word, const char *fraction) {
    return logos_seq_range(lf ? logos_ldec_as_i64(lo, past_word, fraction) : lo,
                           hf ? logos_ldec_as_i64(hi, past_word, fraction) : hi);
}
logos_seq *logos_seq_slice_ldec(const logos_seq *a, long long start, int sf, long long end, int ef,
                                const char *past_word, const char *fraction) {
    return logos_seq_slice(a, sf ? logos_ldec_as_i64(start, past_word, fraction) : start,
                           ef ? logos_ldec_as_i64(end, past_word, fraction) : end);
}

/* `item i of xs` — 1-based, or from the back when negative; an index out of range stops the program as the
   VM does. */
__attribute__((noinline, cold)) static long long logos_seq_index_from_back(const logos_seq *a, long long i) {
    return a->data[logos_index_from_back(i, a->len)];
}

long long logos_seq_index(const logos_seq *a, long long i) {
    if (__builtin_expect(i < 1 || i > a->len, 0)) return logos_seq_index_from_back(a, i);
    return a->data[i - 1];
}

/* `length of text` — BYTE length (the VM's `Text` length is `s.len()`, not the char count). */
long long logos_text_len(const char *s) { return (long long)strlen(s); }

/* `item i of text` — 1-based character access returning a single-character Text, matching the VM's
   `index_get`: an ASCII string indexes by byte; otherwise the i-th Unicode scalar (negative i counts
   from the end); an out-of-range index errors like the VM (exit nonzero). */
char *logos_text_index(const char *s, long long i) {
    long long blen = (long long)strlen(s);
    int ascii = 1;
    for (long long k = 0; k < blen; k++) {
        if ((unsigned char)s[k] >= 0x80) { ascii = 0; break; }
    }
    if (ascii && i >= 1 && i <= blen) {
        char *out = (char *)lc_gc_alloc_atomic(2);
        out[0] = s[i - 1];
        out[1] = '\0';
        return out;
    }
    long long char_len = blen;
    if (!ascii) {
        char_len = 0;
        for (long long k = 0; k < blen;) {
            unsigned char c = (unsigned char)s[k];
            int n = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
            k += n;
            char_len++;
        }
    }
    long long idx;
    if (i >= 1) {
        idx = i - 1;
        if (idx >= char_len) goto oob;
    } else if (i <= -1) {
        long long back = -i;
        if (back > char_len) goto oob;
        idx = char_len - back;
    } else {
        goto oob;
    }
    {
        long long k = 0, ci = 0;
        while (k < blen) {
            unsigned char c = (unsigned char)s[k];
            int n = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
            if (ci == idx) {
                char *out = (char *)lc_gc_alloc_atomic(n + 1);
                for (int t = 0; t < n; t++) out[t] = s[k + t];
                out[n] = '\0';
                return out;
            }
            k += n;
            ci++;
        }
    }
oob:
    logos_fatalf(logos_index_refusal, i);
}

/* ---- Files: the VM's sandboxed in-memory VFS, which keeps files only inside a `## Test`'s sealed world ----
   A program keeps none, so every read finds nothing and stops the program with `missing` (a format of the
   path) — the VM's sentence, taken from the VM by the emitter. */
__attribute__((noreturn)) void logos_vfs_read(const char *path, const char *missing) {
    size_t n = strlen(missing) + strlen(path) + 1;
    char *why = (char *)lc_gc_alloc_atomic(n);
    snprintf(why, n, missing, path);
    logos_fatal(why);
}

/* `Repeat for c in t` — the Text's characters, each a Text of its own. */
logos_seq *logos_text_chars(const char *s) {
    logos_seq *out = logos_seq_new();
    const unsigned char *p = (const unsigned char *)s;
    while (*p) {
        unsigned cp;
        int n = logos_utf8_at(p, &cp);
        char *c = (char *)lc_gc_alloc_atomic((size_t)n + 1);
        memcpy(c, p, (size_t)n);
        c[n] = '\0';
        logos_seq_push_inplace(out, (long long)c);
        p += n;
    }
    return out;
}

/* Eight bytes from the system's entropy; `getentropy` fails only for a request over 256 bytes. */
static unsigned long long logos_entropy(void) {
    unsigned long long draw;
    if (getentropy(&draw, sizeof draw) != 0) abort();
    return draw;
}

/* `randomInt(lo, hi)` — uniform over `lo..=hi` from the system's entropy (rejection sampling, so no bound is
   favored); crossed bounds stop the program with `logos_random_bounds_refusal`. */
long long logos_random_int(long long lo, long long hi) {
    if (lo > hi) {
        char why[256];
        snprintf(why, sizeof why, logos_random_bounds_refusal, lo, hi);
        logos_fatal(lc_gc_strdup(why));
    }
    unsigned long long span = (unsigned long long)hi - (unsigned long long)lo, draw, limit;
    if (span == ~0ULL) return (long long)logos_entropy();
    limit = ~0ULL - (~0ULL % (span + 1) + 1) % (span + 1);
    do draw = logos_entropy(); while (draw > limit);
    return (long long)((unsigned long long)lo + draw % (span + 1));
}

/* `text_bytes(t)` — each UTF-8 byte of the Text, as an Int 0..255. */
logos_seq *logos_text_bytes(const char *s) {
    logos_seq *out = logos_seq_new();
    for (const unsigned char *p = (const unsigned char *)s; *p; p++) logos_seq_push_inplace(out, *p);
    return out;
}

/* `readReals(path)` — the file's little-endian binary64 values, eight bytes a value in order, each slot of the
   Seq of Real the value's bits; refused as `logicaffeine_system::file::read_reals` refuses: a file it cannot read,
   or one that is not a whole number of values. */
logos_seq *logos_read_reals(const char *path) {
    FILE *f = fopen(path, "rb");
    char *why;
    size_t n;
    if (!f) {
        const char *reason = strerror(errno);
        n = strlen(path) + strlen(reason) + 64;
        why = (char *)lc_gc_alloc_atomic(n);
        snprintf(why, n, "readReals: cannot read '%s': %s", path, reason);
        logos_fatal(why);
    }
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size < 0 || size % 8 != 0) {
        fclose(f);
        n = strlen(path) + 96;
        why = (char *)lc_gc_alloc_atomic(n);
        snprintf(why, n, "readReals: '%s' holds %ld bytes, not a whole number of 8-byte Reals", path, size);
        logos_fatal(why);
    }
    logos_seq *out = logos_seq_new_cap(size / 8 > 0 ? size / 8 : 1);
    unsigned char b[8];
    for (long i = 0; i < size / 8; i++) {
        if (fread(b, 1, 8, f) != 8) logos_fatals("readReals: cannot read '%s'", path);
        unsigned long long bits = 0;
        for (int k = 7; k >= 0; k--) bits = (bits << 8) | b[k];
        logos_seq_push_inplace(out, (long long)bits);
    }
    fclose(f);
    return out;
}

/* The length a UTF-8 sequence led by `b` claims, 0 for a byte no sequence starts with — Rust's
   `utf8_char_width`. */
static int logos_utf8_width(unsigned b) {
    if (b < 0x80) return 1;
    if (b < 0xC2) return 0;
    if (b < 0xE0) return 2;
    if (b < 0xF0) return 3;
    if (b < 0xF5) return 4;
    return 0;
}

/* Rust's UTF-8 validation (`core::str::from_utf8`), step for step: 1 when `v[0..n)` is UTF-8, else 0 with
   where the valid prefix ends and the length of the invalid sequence there — 0 when the bytes end inside a
   sequence — the two facts `Utf8Error` reports. */
static int logos_utf8_valid(const unsigned char *v, long long n, long long *valid_up_to, int *error_len) {
    long long i = 0;
    while (i < n) {
        long long start = i;
        unsigned first = v[i], b;
#define LOGOS_UTF8_FAIL(len) do { *valid_up_to = start; *error_len = (len); return 0; } while (0)
#define LOGOS_UTF8_NEXT() do { if (++i >= n) LOGOS_UTF8_FAIL(0); b = v[i]; } while (0)
#define LOGOS_UTF8_CONT(len) do { LOGOS_UTF8_NEXT(); if (b < 0x80 || b >= 0xC0) LOGOS_UTF8_FAIL(len); } while (0)
        if (first >= 0x80) {
            switch (logos_utf8_width(first)) {
            case 2:
                LOGOS_UTF8_CONT(1);
                break;
            case 3:
                LOGOS_UTF8_NEXT();
                if (!((first == 0xE0 && b >= 0xA0 && b <= 0xBF) || (first >= 0xE1 && first <= 0xEC && b >= 0x80 && b <= 0xBF)
                      || (first == 0xED && b >= 0x80 && b <= 0x9F) || (first >= 0xEE && b >= 0x80 && b <= 0xBF)))
                    LOGOS_UTF8_FAIL(1);
                LOGOS_UTF8_CONT(2);
                break;
            case 4:
                LOGOS_UTF8_NEXT();
                if (!((first == 0xF0 && b >= 0x90 && b <= 0xBF) || (first >= 0xF1 && first <= 0xF3 && b >= 0x80 && b <= 0xBF)
                      || (first == 0xF4 && b >= 0x80 && b <= 0x8F)))
                    LOGOS_UTF8_FAIL(1);
                LOGOS_UTF8_CONT(2);
                LOGOS_UTF8_CONT(3);
                break;
            default:
                LOGOS_UTF8_FAIL(1);
            }
        }
#undef LOGOS_UTF8_CONT
#undef LOGOS_UTF8_NEXT
#undef LOGOS_UTF8_FAIL
        i++;
    }
    return 1;
}

/* `text_from_bytes(bytes)` — the low byte of each element, as a Text when they are UTF-8. Otherwise the
   program stops with `logos_utf8_invalid_refusal` (of the invalid sequence's length and where it starts) or
   `logos_utf8_incomplete_refusal` (of where the unfinished sequence starts). */
char *logos_text_from_bytes(const logos_seq *b) {
    unsigned char *bytes = (unsigned char *)lc_gc_alloc_atomic((size_t)b->len + 1);
    for (long long i = 0; i < b->len; i++) bytes[i] = (unsigned char)(b->data[i] & 0xff);
    bytes[b->len] = '\0';
    long long at;
    int len;
    if (!logos_utf8_valid(bytes, b->len, &at, &len)) {
        char why[512];
        if (len) snprintf(why, sizeof why, logos_utf8_invalid_refusal, len, at);
        else snprintf(why, sizeof why, logos_utf8_incomplete_refusal, at);
        logos_fatal(lc_gc_strdup(why));
    }
    return (char *)bytes;
}

void logos_seq_show_int(const logos_seq *a) {
    putchar('[');
    for (long long i = 0; i < a->len; i++) { if (i) fputs(", ", stdout); printf("%lld", a->data[i]); }
    fputs("]\n", stdout);
}

void logos_seq_show_bool(const logos_seq *a) {
    putchar('[');
    for (long long i = 0; i < a->len; i++) { if (i) fputs(", ", stdout); fputs(a->data[i] ? "true" : "false", stdout); }
    fputs("]\n", stdout);
}

void logos_seq_show_float(const logos_seq *a) {
    putchar('[');
    for (long long i = 0; i < a->len; i++) {
        if (i) fputs(", ", stdout);
        double f; memcpy(&f, &a->data[i], 8);
        char *s = logos_fmt_f64(f); fputs(s, stdout); lc_gc_free(s);
    }
    fputs("]\n", stdout);
}

/* Text elements render UNQUOTED inside a list, like the VM's `[foo, bar]`. */
void logos_seq_show_text(const logos_seq *a) {
    putchar('[');
    for (long long i = 0; i < a->len; i++) { if (i) fputs(", ", stdout); fputs((const char *)a->data[i], stdout); }
    fputs("]\n", stdout);
}

/* ---- Map (insertion-ordered key→value; parallel key/value arrays of uniform 8-byte cells) ---- */
typedef struct { long long len, cap; long long *keys; long long *vals; } logos_map;

logos_map *logos_map_new(void) {
    logos_map *m = (logos_map *)lc_gc_alloc(sizeof(logos_map));
    m->len = 0; m->cap = 0; m->keys = NULL; m->vals = NULL;
    return m;
}

logos_map *logos_map_new_cap(long long capacity) {
    if (capacity < 0) capacity = 0;
    logos_map *m = (logos_map *)lc_gc_alloc(sizeof(logos_map));
    m->len = 0; m->cap = capacity;
    m->keys = capacity ? (long long *)lc_gc_alloc((size_t)capacity * 8) : NULL;
    m->vals = capacity ? (long long *)lc_gc_alloc((size_t)capacity * 8) : NULL;
    return m;
}

static logos_map *logos_map_dup(const logos_map *a) {
    logos_map *m = (logos_map *)lc_gc_alloc(sizeof(logos_map));
    m->len = a->len; m->cap = a->len;
    m->keys = a->len ? (long long *)lc_gc_alloc((size_t)a->len * 8) : NULL;
    m->vals = a->len ? (long long *)lc_gc_alloc((size_t)a->len * 8) : NULL;
    if (a->len) { memcpy(m->keys, a->keys, (size_t)a->len * 8); memcpy(m->vals, a->vals, (size_t)a->len * 8); }
    return m;
}

/* `Set item key of m to val` — insert or update (insertion-ordered), in the register's own map (a copy
   when its owned flag is clear; see the Seq section). `key_kind` selects key compare. */
void logos_map_set_own(logos_map **slot, long long *owned, long long key, long long val, int key_kind) {
    if (!*owned) { *slot = logos_map_dup(*slot); *owned = 1; }
    logos_map *m = *slot;
    for (long long i = 0; i < m->len; i++) {
        if (logos_cell_eq(m->keys[i], key, key_kind)) { m->vals[i] = val; return; }
    }
    if (m->len == m->cap) {
        m->cap = m->cap ? m->cap * 2 : 4;
        m->keys = (long long *)lc_gc_realloc(m->keys, (size_t)m->cap * 8);
        m->vals = (long long *)lc_gc_realloc(m->vals, (size_t)m->cap * 8);
    }
    m->keys[m->len] = key; m->vals[m->len] = val; m->len++;
}

/* The key-wise merge of two maps (the VM's `crdt_merge_field` on maps): `a`'s entries in its order, each key
   `b` also holds taking the merge of the two values — Ints summed (`value_rule` 0) or `b`'s value (1) — then
   `b`'s other keys in its order. A fresh map. */
logos_map *logos_map_merge(const logos_map *a, const logos_map *b, int key_kind, int value_rule) {
    logos_map *out = logos_map_dup(a);
    for (long long i = 0; i < b->len; i++) {
        long long j = 0;
        while (j < out->len && !logos_cell_eq(out->keys[j], b->keys[i], key_kind)) j++;
        if (j < out->len) {
            out->vals[j] = value_rule == 0
                ? (long long)((unsigned long long)out->vals[j] + (unsigned long long)b->vals[i])
                : b->vals[i];
            continue;
        }
        if (out->len == out->cap) {
            out->cap = out->cap ? out->cap * 2 : 4;
            out->keys = (long long *)lc_gc_realloc(out->keys, (size_t)out->cap * 8);
            out->vals = (long long *)lc_gc_realloc(out->vals, (size_t)out->cap * 8);
        }
        out->keys[out->len] = b->keys[i];
        out->vals[out->len] = b->vals[i];
        out->len++;
    }
    return out;
}

/* `Merge`/`Converge` of cell `cell` (1-based) of two records of one type, for the fields the emitter does
   not merge inline: what the VM's `crdt_merge_field` makes of the target's value and the source's, by the
   emitter's rule — 3 a fresh counter holding the per-replica maximum, 4 the union of sets of element kind
   `a`, 5 the key-wise merge of maps keyed by kind `a` under value rule `b`. */
long long logos_merge_cell(const logos_seq *target, const logos_seq *source, long long cell, int how, int a, int b) {
    long long mine = target->data[cell - 1], theirs = source->data[cell - 1];
    switch (how) {
        case 3: {
            logos_crdt *merged = logos_crdt_clone((const logos_crdt *)mine);
            logos_crdt_merge(merged, (const logos_crdt *)theirs);
            return (long long)merged;
        }
        case 4: return (long long)logos_set_union((const logos_seq *)mine, (const logos_seq *)theirs, a);
        case 5: return (long long)logos_map_merge((const logos_map *)mine, (const logos_map *)theirs, a, b);
        default: abort();
    }
}

/* `item key of m` — lookup; a missing key stops the program with the VM's sentence, naming the key. */
long long logos_map_get(const logos_map *a, long long key, int key_kind) {
    for (long long i = 0; i < a->len; i++) {
        if (logos_cell_eq(a->keys[i], key, key_kind)) return a->vals[i];
    }
    char *shown = logos_val_to_str(key, key_kind);
    size_t n = strlen(logos_missing_key_refusal) + strlen(shown) + 1;
    char *why = (char *)lc_gc_alloc_atomic(n);
    snprintf(why, n, logos_missing_key_refusal, shown);
    logos_fatal(why);
}

/* `length of map` — the entry count. */
long long logos_map_len(const logos_map *a) { return a->len; }

/* `map contains key` — key membership (never errors on a missing key), matching the VM's `Contains`. */
long long logos_map_has(const logos_map *a, long long key, int key_kind) {
    for (long long i = 0; i < a->len; i++) {
        if (logos_cell_eq(a->keys[i], key, key_kind)) return 1;
    }
    return 0;
}

/* `Repeat for (k, v) in m` — the map's entries in order, each a `(key, value)` tuple. */
logos_seq *logos_map_pairs(const logos_map *m) {
    logos_seq *out = logos_seq_new();
    for (long long i = 0; i < m->len; i++) {
        logos_seq *pair = logos_seq_new();
        logos_seq_push_inplace(pair, m->keys[i]);
        logos_seq_push_inplace(pair, m->vals[i]);
        logos_seq_push_inplace(out, (long long)pair);
    }
    return out;
}

/* `Show` of a map — `{k: v, ...}` (insertion order; keys/values formatted by kind, text unquoted). */
void logos_map_show(const logos_map *a, int key_kind, int val_kind) {
    putchar('{');
    for (long long i = 0; i < a->len; i++) {
        if (i) fputs(", ", stdout);
        char *k = logos_val_to_str(a->keys[i], key_kind);
        char *v = logos_val_to_str(a->vals[i], val_kind);
        printf("%s: %s", k, v);
        lc_gc_free(k); lc_gc_free(v);
    }
    fputs("}\n", stdout);
}

/* ---- Deep copy and structural equality — the VM's `deep_clone` and `values_equal`, by the shape the emitter
   gives each kind. A shape is `[form, n, parts]`: forms 0 (Int), 7 (Text), 8 (Float) and 9 (Rational)
   are copied as values; 1 is a sequence with one element shape; 10 a set; 2 a map (key and value shapes);
   3 a declared enum with tagged constructor payload shapes; 6 a counter. */
/* Forms 11 (tuple) and 12 (record) use the sequence layout without collection identity.
   Forms 13/14 are Option/Result constructors. Forms 16..20 distinguish Bool, Date, Moment,
   Word32 and opaque identities from Int; 21 is a peer; 22 a cell of the exact extension; 23 `nothing`; 24 a span,
   its months and days packed into one word. */
typedef struct { long long form, n; const void *const *parts; } logos_shape;
typedef struct { long long tag, n; const logos_shape *const *parts; } logos_variant;

/* Form 22 is a cell of the exact extension, which the extension's adapter copies and compares (two values of which
   one is a cell, each by its own shape), set when the shim embeds it. */
static long long (*logos_tower_copy_hook)(long long);
static int (*logos_tower_equal_hook)(long long, const logos_shape *, long long, const logos_shape *);

long long logos_deep_copy(long long v, const logos_shape *s) {
    if (s->form == 22) return logos_tower_copy_hook(v);
    if (s->form == 0 || (s->form >= 7 && s->form <= 9) || s->form >= 16 || !v) return v;
    switch (s->form) {
        case 1:
        case 11:
        case 12:
        case 10: {
            const logos_seq *a = (const logos_seq *)v;
            logos_seq *c = logos_seq_new();
            for (long long i = 0; i < a->len; i++) {
                const logos_shape *cell = (const logos_shape *)s->parts[s->n == 1 ? 0 : i];
                logos_seq_push_inplace(c, logos_deep_copy(a->data[i], cell));
            }
            return (long long)c;
        }
        case 2: {
            const logos_map *a = (const logos_map *)v;
            logos_map *c = logos_map_dup(a);
            for (long long i = 0; i < a->len; i++) {
                c->keys[i] = logos_deep_copy(a->keys[i], (const logos_shape *)s->parts[0]);
                c->vals[i] = logos_deep_copy(a->vals[i], (const logos_shape *)s->parts[1]);
            }
            return (long long)c;
        }
        case 3:
        case 13:
        case 14: {
            const logos_seq *a = (const logos_seq *)v;
            const logos_variant *ctor = 0;
            for (long long i = 0; i < s->n; i++) {
                const logos_variant *r = (const logos_variant *)s->parts[i];
                if (r->tag == a->data[0]) ctor = r;
            }
            logos_seq *c = logos_seq_new();
            for (long long i = 0; i < a->len; i++) {
                long long cell = a->data[i];
                if (i > 0 && ctor && i - 1 < ctor->n) cell = logos_deep_copy(cell, ctor->parts[i - 1]);
                logos_seq_push_inplace(c, cell);
            }
            return (long long)c;
        }
        default: return (long long)logos_crdt_clone((const logos_crdt *)v);
    }
}

/* Whether `a` and `b`, two values of shape `s`, are equal by structure. */
int logos_deep_eq(long long a, long long b, const logos_shape *s) {
    switch (s->form) {
        case 0: case 16: case 17: case 18: case 19: case 20: case 23: case 24: return a == b;
        case 22: return logos_tower_equal_hook(a, s, b, s);
        case 7: case 21: return strcmp((const char *)a, (const char *)b) == 0;
        case 8: { double x, y; memcpy(&x, &a, 8); memcpy(&y, &b, 8); return x == y; }
        case 9: return logos_ldec_cmp(a, 1, b, 1) == 0;
    }
    /* Only the VM's identity-bearing collections short-circuit on identity. A record, tuple or
       enum still compares its fields, so an aliased NaN payload is unequal to itself. */
    if (a == b && (s->form == 1 || s->form == 2 || s->form == 10)) return 1;
    if (!a || !b) return 0;
    switch (s->form) {
        case 1:
        case 11:
        case 12: {
            const logos_seq *x = (const logos_seq *)a, *y = (const logos_seq *)b;
            if (x->len != y->len) return 0;
            for (long long i = 0; i < x->len; i++) {
                if (!logos_deep_eq(x->data[i], y->data[i], (const logos_shape *)s->parts[s->n == 1 ? 0 : i])) return 0;
            }
            return 1;
        }
        case 10: {
            const logos_seq *x = (const logos_seq *)a, *y = (const logos_seq *)b;
            if (x->len != y->len) return 0;
            for (long long i = 0; i < x->len; i++) {
                long long j = 0;
                while (j < y->len && !logos_deep_eq(x->data[i], y->data[j], (const logos_shape *)s->parts[0])) j++;
                if (j == y->len) return 0;
            }
            return 1;
        }
        case 2: {
            const logos_map *x = (const logos_map *)a, *y = (const logos_map *)b;
            if (x->len != y->len) return 0;
            for (long long i = 0; i < x->len; i++) {
                long long j = 0;
                while (j < y->len && !logos_deep_eq(x->keys[i], y->keys[j], (const logos_shape *)s->parts[0])) j++;
                if (j == y->len || !logos_deep_eq(x->vals[i], y->vals[j], (const logos_shape *)s->parts[1])) return 0;
            }
            return 1;
        }
        case 3:
        case 13:
        case 14: {
            const logos_seq *x = (const logos_seq *)a, *y = (const logos_seq *)b;
            if (x->data[0] != y->data[0] || x->len != y->len) return 0;
            const logos_variant *ctor = 0;
            for (long long i = 0; i < s->n; i++) {
                const logos_variant *r = (const logos_variant *)s->parts[i];
                if (r->tag == x->data[0]) ctor = r;
            }
            for (long long i = 1; i < x->len; i++) {
                if (ctor && i - 1 < ctor->n ? !logos_deep_eq(x->data[i], y->data[i], ctor->parts[i - 1]) : x->data[i] != y->data[i]) return 0;
            }
            return 1;
        }
        default: return logos_crdt_sum((const logos_crdt *)a) == logos_crdt_sum((const logos_crdt *)b);
    }
}

/* Cross-kind structural equality retains both layouts. Int and Float cells use exact numeric
   comparison, never an interpretation of one kind's bits as another kind's value. */
int logos_deep_eq_pair(long long a, long long b, const logos_shape *s, const logos_shape *t) {
    if (s == t) return logos_deep_eq(a, b, s);
    if (s->form == 22 || t->form == 22) return logos_tower_equal_hook(a, s, b, t);
    if (s->form == 6) return logos_deep_eq_pair(logos_crdt_sum((const logos_crdt *)a), b, &(logos_shape){0, 0, 0}, t);
    if (t->form == 6) return logos_deep_eq_pair(a, logos_crdt_sum((const logos_crdt *)b), s, &(logos_shape){0, 0, 0});
    if (s->form == 0 && t->form == 0) return a == b;
    if ((s->form == 0 && t->form == 8) || (s->form == 8 && t->form == 0)) {
        long long bits = s->form == 8 ? a : b;
        double f; memcpy(&f, &bits, 8);
        return logos_cmp_i64_f64_rel(s->form == 0 ? a : b, f, 0, 4);
    }
    if ((s->form == 9 && t->form == 0) || (s->form == 0 && t->form == 9)) {
        return logos_ldec_cmp(a, s->form == 9, b, t->form == 9) == 0;
    }
    if ((s->form == 9 && t->form == 8) || (s->form == 8 && t->form == 9)) {
        long long bits = s->form == 8 ? a : b;
        double f; memcpy(&f, &bits, 8);
        return logos_ldec_cmp_f64(s->form == 9 ? a : b, f) == 0;
    }
    if (s->form != t->form || !a || !b) return 0;
    switch (s->form) {
        case 1:
        case 11: {
            const logos_seq *x = (const logos_seq *)a, *y = (const logos_seq *)b;
            if (x->len != y->len) return 0;
            for (long long i = 0; i < x->len; i++) {
                const logos_shape *u = (const logos_shape *)s->parts[s->n == 1 ? 0 : i];
                const logos_shape *v = (const logos_shape *)t->parts[t->n == 1 ? 0 : i];
                if (!logos_deep_eq_pair(x->data[i], y->data[i], u, v)) return 0;
            }
            return 1;
        }
        case 2: {
            const logos_map *x = (const logos_map *)a, *y = (const logos_map *)b;
            if (x->len != y->len) return 0;
            for (long long i = 0; i < x->len; i++) {
                long long j = 0;
                while (j < y->len && !logos_deep_eq_pair(x->keys[i], y->keys[j],
                    (const logos_shape *)s->parts[0], (const logos_shape *)t->parts[0])) j++;
                if (j == y->len || !logos_deep_eq_pair(x->vals[i], y->vals[j],
                    (const logos_shape *)s->parts[1], (const logos_shape *)t->parts[1])) return 0;
            }
            return 1;
        }
        case 13:
        case 14: {
            const logos_seq *x = (const logos_seq *)a, *y = (const logos_seq *)b;
            if (x->len != y->len || x->data[0] != y->data[0]) return 0;
            const logos_variant *u = 0, *v = 0;
            for (long long i = 0; i < s->n; i++) {
                const logos_variant *r = (const logos_variant *)s->parts[i];
                if (r->tag == x->data[0]) u = r;
            }
            for (long long i = 0; i < t->n; i++) {
                const logos_variant *r = (const logos_variant *)t->parts[i];
                if (r->tag == y->data[0]) v = r;
            }
            if (!u || !v || u->n != v->n || x->len != u->n + 1) return 0;
            for (long long i = 0; i < u->n; i++) {
                if (!logos_deep_eq_pair(x->data[i+1], y->data[i+1], u->parts[i], v->parts[i])) return 0;
            }
            return 1;
        }
        /* Different nominal records/enums or different scalar kinds cannot compare equal. */
        default: return 0;
    }
}

/* How a value of shape `s` displays, the VM's `to_display_string`: a scalar as `Show` prints it, a sequence as
   `[a, b]`, a set as `{a, b}`, a tuple as `(a, b)` and a map as `{k: v}`, each part by its own shape. */
typedef struct { char *text; size_t len, cap; } logos_text_buf;

static void logos_buf_put(logos_text_buf *b, const char *s) {
    size_t n = strlen(s);
    if (b->len + n + 1 > b->cap) {
        size_t cap = b->cap * 2 > b->len + n + 1 ? b->cap * 2 : b->len + n + 1;
        b->text = (char *)lc_gc_realloc_atomic(b->text, cap);
        b->cap = cap;
    }
    memcpy(b->text + b->len, s, n + 1);
    b->len += n;
}

static void logos_shaped_into(logos_text_buf *b, long long v, const logos_shape *s);

static void logos_parts_into(logos_text_buf *b, const logos_seq *a, const logos_shape *s, const char *open, const char *close) {
    logos_buf_put(b, open);
    for (long long i = 0; i < a->len; i++) {
        if (i) logos_buf_put(b, ", ");
        logos_shaped_into(b, a->data[i], (const logos_shape *)s->parts[s->n == 1 ? 0 : i]);
    }
    logos_buf_put(b, close);
}

static void logos_shaped_into(logos_text_buf *b, long long v, const logos_shape *s) {
    char *made = 0;
    switch (s->form) {
        case 0: case 19: made = logos_val_to_str(v, 0); break;
        case 16: made = logos_val_to_str(v, 2); break;
        case 7: logos_buf_put(b, (const char *)v); return;
        case 8: made = logos_val_to_str(v, 1); break;
        case 9: made = logos_val_to_str(v, 4); break;
        case 17: made = logos_val_to_str(v, 5); break;
        case 18: made = logos_val_to_str(v, 6); break;
        case 21: made = logos_val_to_str(v, 8); break;
        case 6: made = logos_val_to_str(logos_crdt_sum((const logos_crdt *)v), 0); break;
        case 22: made = logos_tower_text_hook(v); break;
        case 23: logos_buf_put(b, "nothing"); return;
        case 1: logos_parts_into(b, (const logos_seq *)v, s, "[", "]"); return;
        case 10: logos_parts_into(b, (const logos_seq *)v, s, "{", "}"); return;
        case 11: logos_parts_into(b, (const logos_seq *)v, s, "(", ")"); return;
        case 2: {
            const logos_map *m = (const logos_map *)v;
            logos_buf_put(b, "{");
            for (long long i = 0; i < m->len; i++) {
                if (i) logos_buf_put(b, ", ");
                logos_shaped_into(b, m->keys[i], (const logos_shape *)s->parts[0]);
                logos_buf_put(b, ": ");
                logos_shaped_into(b, m->vals[i], (const logos_shape *)s->parts[1]);
            }
            logos_buf_put(b, "}");
            return;
        }
        default: logos_fatalf("the RISC-V runtime has no display for a value of shape form %lld", s->form);
    }
    logos_buf_put(b, made);
    lc_gc_free(made);
}

char *logos_shaped_str(long long v, const logos_shape *s) {
    logos_text_buf b = {0, 0, 0};
    logos_buf_put(&b, "");
    logos_shaped_into(&b, v, s);
    return b.text;
}

void logos_show_shaped(long long v, const logos_shape *s) {
    char *text = logos_shaped_str(v, s);
    printf("%s\n", text);
    lc_gc_free(text);
}

/* Snapshot iterator stack (for `Repeat for x in xs`). A snapshot is just the handle: the iterated
 * register's handle escapes into the iterator, which clears the register's owned flag, so a mutation of it
 * inside the loop copies first and the iterated list never changes under the iterator. Each task iterates
 * on a stack of its own, so a task that yields inside a loop resumes it where it left off (`logos_iters`). */
typedef struct { logos_seq *seq[256]; long long idx[256]; int top; } logos_iter_stack;
static logos_iter_stack *logos_iters(void);
void logos_iter_begin(logos_seq *s) {
    logos_iter_stack *it = logos_iters();
    it->seq[it->top] = s;
    it->idx[it->top] = 0;
    it->top++;
}
int logos_iter_has(void) {
    logos_iter_stack *it = logos_iters();
    int t = it->top - 1;
    return it->idx[t] < it->seq[t]->len;
}
long long logos_iter_take(void) {
    logos_iter_stack *it = logos_iters();
    int t = it->top - 1;
    return it->seq[t]->data[it->idx[t]++];
}
void logos_iter_end(void) { logos_iters()->top--; }

/* ---- Deterministic cooperative concurrency: the C family's scheduler core under `ucontext` ---- */
"#,
    "#define LOGOS_FN static __attribute__((unused))\n",
    "#define LOGOS_VAR static\n",
    "#define LOGOS_API\n",
    "#define logos_alloc(n) lc_gc_calloc(1, (n))\n",
    "#define logos_release(p) lc_gc_free(p)\n",
    "#define LOGOS_STACK_ENTER(lo, hi) lc_gc_stack_enter((lo), (hi))\n",
    "#define LOGOS_STACK_LEAVE() lc_gc_stack_leave()\n",
    "#define LOGOS_TASK_TIER ucontext_t ctx; void *stack; void *fn; long long nargs; long long *args; logos_iter_stack *iters;\n",
    "static long long logos_riscv_call(void *fn, long long n, const long long *a);\n",
    "#define LOGOS_TASK_CALL(t) logos_riscv_call((t)->fn, (t)->nargs, (t)->args)\n",
    include_str!("../codegen_c/sched_core.c"),
    include_str!("../codegen_c/host_seed.c"),
    include_str!("../codegen_c/opt.c"),
    include_str!("../codegen_c/ucontext_tier.c"),
    r#"
/* A task body is an emitted `logos_fn_N`, which takes its arguments in a0…a7, as every call does. */
static long long logos_riscv_call(void *fn, long long n, const long long *a) {
    switch (n) {
        case 0: return ((long long (*)(void))fn)();
        case 1: return ((long long (*)(long long))fn)(a[0]);
        case 2: return ((long long (*)(long long, long long))fn)(a[0], a[1]);
        case 3: return ((long long (*)(long long, long long, long long))fn)(a[0], a[1], a[2]);
        case 4: return ((long long (*)(long long, long long, long long, long long))fn)(a[0], a[1], a[2], a[3]);
        case 5: return ((long long (*)(long long, long long, long long, long long, long long))fn)(a[0], a[1], a[2], a[3], a[4]);
        case 6: return ((long long (*)(long long, long long, long long, long long, long long, long long))fn)(a[0], a[1], a[2], a[3], a[4], a[5]);
        case 7: return ((long long (*)(long long, long long, long long, long long, long long, long long, long long))fn)(a[0], a[1], a[2], a[3], a[4], a[5], a[6]);
        default: return ((long long (*)(long long, long long, long long, long long, long long, long long, long long, long long))fn)(a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
    }
}

/* The running task's iterator stack; the program's own when no task runs. */
static logos_iter_stack logos_iters_outside;
static logos_iter_stack *logos_iters(void) {
    if (!logos_current) return &logos_iters_outside;
    if (!logos_current->iters) logos_current->iters = (logos_iter_stack *)lc_gc_calloc(1, sizeof(logos_iter_stack));
    return logos_current->iters;
}

extern void *logos_fn_dispatch[];

/* `Launch a task to f …` from the emitted code: the body by its index in `logos_fn_dispatch`, the
   arguments as the launching frame's contiguous slots. */
long long logos_spawn_fn(long long func_idx, long long nargs, const long long *args) {
    return logos_spawn(logos_fn_dispatch[func_idx], nargs, args);
}

/* The box a select's receive arm resolves into: the emitted code reads it back at the wait. */
logos_opt *logos_opt_nothing(void) { return (logos_opt *)lc_gc_calloc(1, sizeof(logos_opt)); }

/* ---- Networking — the VM's offline `NetInbox`: one node, no relay ----
   `Listen` names this node's inbox. `Send`/`Stream` put the message, stamped with the sender's inbox (empty
   before any `Listen`), on the loopback outbox under the topic it is addressed to. `Await` needs an inbox: it
   moves each outboxed message into the received queue when it is addressed to the inbox, dropping it
   otherwise, then takes the first received message (or batch) from the peer it names. A lone node sends
   nothing while it waits, so when that peer's message is not there the program waits for ever, as the VM's
   `Await` polls for ever. A topic is canonical: `logos_net_topic`, the relay's `canonical_topic`. */

/* Whether `s` begins with `prefix`, ASCII case folded (`to_ascii_lowercase().starts_with`). */
static int logos_ascii_prefix(const char *s, const char *prefix) {
    for (; *prefix; s++, prefix++) {
        char c = *s >= 'A' && *s <= 'Z' ? (char)(*s - 'A' + 'a') : *s;
        if (c != *prefix) return 0;
    }
    return 1;
}

/* Whether the `n` bytes at `s` parse as a `u16` (`str::parse::<u16>`: an optional `+`, then digits). */
static int logos_is_u16(const char *s, size_t n) {
    if (n && s[0] == '+') { s++; n--; }
    if (!n) return 0;
    unsigned long v = 0;
    for (size_t i = 0; i < n; i++) {
        if (s[i] < '0' || s[i] > '9') return 0;
        v = v * 10 + (unsigned long)(s[i] - '0');
        if (v > 65535) return 0;
    }
    return 1;
}

/* `multiaddr_to_ws_url` — the `ws://` URL a libp2p multiaddr (or a ws/wss URL) names, or NULL when it names
   none. */
static char *logos_multiaddr_url(const char *addr) {
    char *t = logos_trim(addr);
    if (!*t) return NULL;
    if (logos_ascii_prefix(t, "ws://") || logos_ascii_prefix(t, "wss://")) return t;
    if (t[0] != '/') return NULL;
    long long ns = 1;
    for (const char *p = t + 1; *p; p++) ns += *p == '/';
    const char **seg = (const char **)lc_gc_alloc((size_t)ns * sizeof *seg);
    size_t *len = (size_t *)lc_gc_alloc_atomic((size_t)ns * sizeof *len);
    const char *p = t + 1;
    for (long long k = 0; k < ns; k++) {
        const char *q = strchr(p, '/');
        seg[k] = p;
        len[k] = q ? (size_t)(q - p) : strlen(p);
        p = q ? q + 1 : p + len[k];
    }
#define LOGOS_SEG(k, name) (len[k] == strlen(name) && strncmp(seg[k], name, len[k]) == 0)
    const char *host = NULL, *port = NULL, *scheme = "ws";
    size_t host_len = 0, port_len = 0;
    int v6 = 0;
    long long i = 0;
    while (i < ns) {
        if (LOGOS_SEG(i, "ip4") || LOGOS_SEG(i, "dns") || LOGOS_SEG(i, "dns4") || LOGOS_SEG(i, "dns6")
            || LOGOS_SEG(i, "dnsaddr") || LOGOS_SEG(i, "ip6")) {
            if (i + 1 >= ns || len[i + 1] == 0) return NULL;
            v6 = LOGOS_SEG(i, "ip6");
            host = seg[i + 1];
            host_len = len[i + 1];
            i += 2;
        } else if (LOGOS_SEG(i, "tcp")) {
            if (i + 1 >= ns || len[i + 1] == 0 || !logos_is_u16(seg[i + 1], len[i + 1])) return NULL;
            port = seg[i + 1];
            port_len = len[i + 1];
            i += 2;
        } else if (LOGOS_SEG(i, "ws")) {
            scheme = "ws";
            i += 1;
        } else if (LOGOS_SEG(i, "wss") || LOGOS_SEG(i, "tls")) {
            scheme = "wss";
            i += 1;
        } else if (LOGOS_SEG(i, "p2p")) {
            i += 2;
        } else if (LOGOS_SEG(i, "p2p-circuit")) {
            i += 1;
        } else {
            return NULL;
        }
    }
#undef LOGOS_SEG
    if (!host || !port) return NULL;
    size_t n = strlen(scheme) + host_len + port_len + 8;
    char *url = (char *)lc_gc_alloc_atomic(n);
    snprintf(url, n, v6 ? "%s://[%.*s]:%.*s" : "%s://%.*s:%.*s", scheme, (int)host_len, host, (int)port_len, port);
    return url;
}

/* `canonical_topic` — the relay topic an address names: its `ws://` URL, else the address trimmed. */
char *logos_net_topic(const char *addr) {
    char *url = logos_multiaddr_url(addr);
    return url ? url : logos_trim(addr);
}

typedef struct { char *topic; char *from; long long value; int batch; } logos_net_msg;
typedef struct { logos_net_msg *at; long long len, cap; } logos_net_queue;
static logos_net_queue logos_net_outbox, logos_net_received;
static char *logos_net_inbox = NULL;

static void logos_net_put(logos_net_queue *q, logos_net_msg m) {
    if (q->len == q->cap) {
        q->cap = q->cap ? q->cap * 2 : 8;
        q->at = (logos_net_msg *)lc_gc_realloc(q->at, (size_t)q->cap * sizeof *q->at);
    }
    q->at[q->len++] = m;
}

/* `Listen on <topic>` — `topic` (canonical) becomes this node's inbox. */
void logos_net_listen(char *topic) { logos_net_inbox = topic; }

/* `Send`/`Stream` to `topic` (canonical): `how` is 0 for a message, 1 for a batch that is the sequence
   `value`, 2 for a batch of the one value `value`. */
void logos_net_send(const char *topic, long long value, int how) {
    if (how == 2) {
        logos_seq *one = logos_seq_new();
        logos_seq_push_inplace(one, value);
        value = (long long)one;
    }
    logos_net_msg m = { lc_gc_strdup(topic), lc_gc_strdup(logos_net_inbox ? logos_net_inbox : ""), value, how != 0 };
    logos_net_put(&logos_net_outbox, m);
}

/* `Await [stream] from <want>` (canonical) — the first message (`batch` 0) or batch (`batch` 1) `want` sent to
   this node's inbox. Without an inbox the program stops with `logos_await_without_listen_refusal`. */
long long logos_net_await(const char *want, int batch) {
    if (!logos_net_inbox) logos_fatal(logos_await_without_listen_refusal);
    for (long long i = 0; i < logos_net_outbox.len; i++) {
        if (strcmp(logos_net_outbox.at[i].topic, logos_net_inbox) == 0) logos_net_put(&logos_net_received, logos_net_outbox.at[i]);
    }
    logos_net_outbox.len = 0;
    for (long long i = 0; i < logos_net_received.len; i++) {
        logos_net_msg m = logos_net_received.at[i];
        if (m.batch == batch && strcmp(m.from, want) == 0) {
            memmove(&logos_net_received.at[i], &logos_net_received.at[i + 1], (size_t)(logos_net_received.len - i - 1) * sizeof m);
            logos_net_received.len--;
            return m.value;
        }
    }
    fflush(stdout);
    for (;;) pause();
}
"#,
);
