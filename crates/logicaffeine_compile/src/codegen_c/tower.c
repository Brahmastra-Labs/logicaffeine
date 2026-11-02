/* ========== The exact extension: values held in the linked value runtime ==========
   A value the exact extension reaches lives in logicaffeine-data's compiled value model, linked as a static
   library behind the `logos_rt_value_*` C ABI. The program holds one as an LTower, a collected cell around the
   runtime's handle; when a collection finds a cell unreached it gives the handle back, and the bytes the runtime
   holds count toward when the next collection runs. Every operation borrows its operands and answers a fresh
   cell, so a cell is never shared by two places that change it: `Push`, `Pop` and `Set item` change the cell the
   binding owns, and a value stored into a second place is taken through ltower_own. A refusal stops the program
   with the runtime's own sentence, as every failure of this runtime does. The core reads nothing of the runtime
   around it but the collector and `logos_fatal`, so every runtime built on that collector embeds it: the C
   backends', and the RISC-V shim's. */
#define LOGOS_LINKS_VALUE_RUNTIME 1
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef struct LogosRtValue LogosRtValue;
extern void logos_rt_value_init(void);
extern const uint8_t *logos_rt_value_error(size_t *len);
extern uint64_t logos_rt_value_heap_bytes(void);
extern uint64_t logos_rt_value_live(void);
extern void logos_rt_value_drop(LogosRtValue *h);
extern LogosRtValue *logos_rt_value_clone(const LogosRtValue *h);
extern LogosRtValue *logos_rt_value_int(int64_t n);
extern LogosRtValue *logos_rt_value_float(double x);
extern LogosRtValue *logos_rt_value_bool(int32_t b);
extern LogosRtValue *logos_rt_value_nothing(void);
extern LogosRtValue *logos_rt_value_date(int32_t days);
extern LogosRtValue *logos_rt_value_moment(int64_t nanos);
extern LogosRtValue *logos_rt_value_time(int64_t nanos);
extern LogosRtValue *logos_rt_value_span(int32_t months, int32_t days);
extern LogosRtValue *logos_rt_value_text(const uint8_t *ptr, size_t len);
extern LogosRtValue *logos_rt_value_exact(const uint8_t *ptr, size_t len);
extern LogosRtValue *logos_rt_value_decimal(const uint8_t *ptr, size_t len);
extern LogosRtValue *logos_rt_value_list(const LogosRtValue *const *items, size_t n);
extern LogosRtValue *logos_rt_value_tuple(const LogosRtValue *const *items, size_t n);
extern LogosRtValue *logos_rt_value_call(const uint8_t *name, size_t name_len, const LogosRtValue *const *args, size_t argc);
extern LogosRtValue *logos_rt_value_apply(size_t index, const LogosRtValue *args);
extern LogosRtValue *logos_rt_value_tuple_of(const LogosRtValue *list);
extern LogosRtValue *logos_rt_value_of_bigint(const void *b);
extern LogosRtValue *logos_rt_value_of_rational(const void *r);
extern LogosRtValue *logos_rt_value_of_decimal(const void *d);
extern LogosRtValue *logos_rt_value_add(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_sub(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_mul(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_div(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_exact_div(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_floor_div(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_rem(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_pow(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_approx_eq(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_seq_concat(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_bit_and(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_bit_or(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_bit_xor(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_shl(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_shr(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_min(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_max(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_power(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_item(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_eq(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_ne(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_lt(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_gt(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_le(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_ge(const LogosRtValue *a, const LogosRtValue *b);
extern LogosRtValue *logos_rt_value_not(const LogosRtValue *a);
extern LogosRtValue *logos_rt_value_abs(const LogosRtValue *a);
extern LogosRtValue *logos_rt_value_sqrt(const LogosRtValue *a);
extern LogosRtValue *logos_rt_value_floor(const LogosRtValue *a);
extern LogosRtValue *logos_rt_value_ceil(const LogosRtValue *a);
extern LogosRtValue *logos_rt_value_round(const LogosRtValue *a);
extern LogosRtValue *logos_rt_value_item_at(const LogosRtValue *a, int64_t index);
extern LogosRtValue *logos_rt_value_slice(const LogosRtValue *a, const LogosRtValue *start, const LogosRtValue *end);
extern int32_t logos_rt_value_length(const LogosRtValue *a, int64_t *out);
extern int32_t logos_rt_value_contains(const LogosRtValue *a, const LogosRtValue *v, int32_t *out);
extern int32_t logos_rt_value_truth(const LogosRtValue *a, int32_t *out);
extern int32_t logos_rt_value_int_of(const LogosRtValue *a, int64_t *out);
extern int32_t logos_rt_value_int_at(const LogosRtValue *a, const uint8_t *past_word, size_t past_word_len, const uint8_t *fraction, size_t fraction_len, int64_t *out);
extern int32_t logos_rt_value_push(LogosRtValue *a, const LogosRtValue *v);
extern int32_t logos_rt_value_set_item(LogosRtValue *a, const LogosRtValue *index, const LogosRtValue *v);
extern LogosRtValue *logos_rt_value_pop(LogosRtValue *a);
extern LogosRtValue *logos_rt_value_items(const LogosRtValue *a);
extern uint8_t *logos_rt_value_show(const LogosRtValue *a, size_t *len);
typedef LogosRtValue *(*LTowerDefiner)(void *code, void *env, int64_t n);
extern int32_t logos_rt_value_sendable(const LogosRtValue *a);
extern LogosRtValue *logos_rt_value_defined_real(const uint8_t *kind, size_t kind_len, int64_t base, const uint8_t *name, size_t name_len, LTowerDefiner call, void *code, void *env);
extern void logos_rt_value_text_free(uint8_t *ptr, size_t len);

typedef struct { LogosRtValue *h; } LTowerCell;
typedef LTowerCell *LTower;

/* The cells alive now and the most alive at once, and the runtime's bytes when a cell was last made. */
static size_t ltower_cells, ltower_cells_high;
static uint64_t ltower_bytes_seen;

static void ltower_release(void *cell) {
    logos_rt_value_drop(((LTowerCell *)cell)->h);
    ltower_cells--;
}

static size_t ltower_outside(void) { return (size_t)logos_rt_value_heap_bytes(); }

static void ltower_report(FILE *out) {
    fprintf(out, " values=%zu values_high=%zu", ltower_cells, ltower_cells_high);
}

__attribute__((noreturn)) static void ltower_refused(void) {
    size_t len = 0;
    const uint8_t *why = logos_rt_value_error(&len);
    char *text = (char *)lc_gc_alloc_atomic(len + 1);
    memcpy(text, why, len);
    text[len] = 0;
    logos_fatal(text);
}

__attribute__((constructor)) static void ltower_start(void) {
    logos_rt_value_init();
    lc_gc_count_outside(ltower_outside, ltower_report);
}

/* The cell around a fresh handle, or the program stops with the runtime's refusal. */
static LTower ltower_wrap(LogosRtValue *h) {
    if (!h) ltower_refused();
    LTowerCell *cell = (LTowerCell *)lc_gc_alloc_atomic(sizeof *cell);
    cell->h = h;
    lc_gc_on_release(cell, ltower_release);
    if (++ltower_cells > ltower_cells_high) ltower_cells_high = ltower_cells;
    uint64_t bytes = logos_rt_value_heap_bytes();
    if (bytes > ltower_bytes_seen) lc_gc_note_outside((size_t)(bytes - ltower_bytes_seen));
    ltower_bytes_seen = bytes;
    return cell;
}

__attribute__((unused)) static LTower ltower_own(LTower v) { return ltower_wrap(logos_rt_value_clone(v->h)); }
__attribute__((unused)) static LTower ltower_int(int64_t n) { return ltower_wrap(logos_rt_value_int(n)); }
__attribute__((unused)) static LTower ltower_float(double x) { return ltower_wrap(logos_rt_value_float(x)); }
__attribute__((unused)) static LTower ltower_bool(int b) { return ltower_wrap(logos_rt_value_bool(b != 0)); }
__attribute__((unused)) static LTower ltower_nothing(void) { return ltower_wrap(logos_rt_value_nothing()); }
__attribute__((unused)) static LTower ltower_date(int64_t days) { return ltower_wrap(logos_rt_value_date((int32_t)days)); }
__attribute__((unused)) static LTower ltower_moment(int64_t nanos) { return ltower_wrap(logos_rt_value_moment(nanos)); }
__attribute__((unused)) static LTower ltower_time(int64_t nanos) { return ltower_wrap(logos_rt_value_time(nanos)); }
__attribute__((unused)) static LTower ltower_span(int32_t months, int32_t days) { return ltower_wrap(logos_rt_value_span(months, days)); }
__attribute__((unused)) static LTower ltower_text(const char *s) {
    return ltower_wrap(logos_rt_value_text((const uint8_t *)s, strlen(s)));
}
__attribute__((unused)) static LTower ltower_exact(const char *digits) {
    return ltower_wrap(logos_rt_value_exact((const uint8_t *)digits, strlen(digits)));
}
__attribute__((unused)) static LTower ltower_decimal(const char *digits) {
    return ltower_wrap(logos_rt_value_decimal((const uint8_t *)digits, strlen(digits)));
}

/* The handles of `n` cells, for an operation that takes a run of values. */
static const LogosRtValue **ltower_handles(size_t n, LTower *cells) {
    const LogosRtValue **hs = (const LogosRtValue **)lc_gc_alloc_atomic((n ? n : 1) * sizeof *hs);
    for (size_t i = 0; i < n; i++) hs[i] = cells[i]->h;
    return hs;
}

__attribute__((unused)) static LTower ltower_list(size_t n, LTower *items) {
    const LogosRtValue **hs = ltower_handles(n, items);
    LogosRtValue *h = logos_rt_value_list(hs, n);
    lc_gc_free(hs);
    return ltower_wrap(h);
}

__attribute__((unused)) static LTower ltower_tuple(size_t n, LTower *items) {
    const LogosRtValue **hs = ltower_handles(n, items);
    LogosRtValue *h = logos_rt_value_tuple(hs, n);
    lc_gc_free(hs);
    return ltower_wrap(h);
}

/* The builtin of the extension named `name` applied to `argc` values. */
__attribute__((unused)) static LTower ltower_call(const char *name, size_t argc, LTower *args) {
    const LogosRtValue **hs = ltower_handles(argc, args);
    LogosRtValue *h = logos_rt_value_call((const uint8_t *)name, strlen(name), hs, argc);
    lc_gc_free(hs);
    return ltower_wrap(h);
}

#define LTOWER_BINARY(op) \
    __attribute__((unused)) static LTower ltower_##op(LTower a, LTower b) { return ltower_wrap(logos_rt_value_##op(a->h, b->h)); }
LTOWER_BINARY(add)
LTOWER_BINARY(sub)
LTOWER_BINARY(mul)
LTOWER_BINARY(div)
LTOWER_BINARY(exact_div)
LTOWER_BINARY(floor_div)
LTOWER_BINARY(rem)
LTOWER_BINARY(pow)
LTOWER_BINARY(approx_eq)
LTOWER_BINARY(seq_concat)
LTOWER_BINARY(bit_and)
LTOWER_BINARY(bit_or)
LTOWER_BINARY(bit_xor)
LTOWER_BINARY(shl)
LTOWER_BINARY(shr)
LTOWER_BINARY(min)
LTOWER_BINARY(max)
LTOWER_BINARY(power)
LTOWER_BINARY(item)
LTOWER_BINARY(eq)
LTOWER_BINARY(ne)
LTOWER_BINARY(lt)
LTOWER_BINARY(gt)
LTOWER_BINARY(le)
LTOWER_BINARY(ge)
#undef LTOWER_BINARY

#define LTOWER_UNARY(op) \
    __attribute__((unused)) static LTower ltower_##op(LTower a) { return ltower_wrap(logos_rt_value_##op(a->h)); }
LTOWER_UNARY(not)
LTOWER_UNARY(abs)
LTOWER_UNARY(sqrt)
LTOWER_UNARY(floor)
LTOWER_UNARY(ceil)
LTOWER_UNARY(round)
#undef LTOWER_UNARY

/* The environments of the functions the program defines real numbers by, reached from here for as long as the program
   runs: the runtime reads a function whenever a refinement of its number asks. */
static void **ltower_definers;
static size_t ltower_definers_n, ltower_definers_cap;

/* The real number a function of the program defines by its digits in `base` or its approximations (`kind`), `name`
   being how the program spells it: the runtime calls `call(code, env, n)` for the function's answer at `n`, as a
   fresh handle. */
__attribute__((unused)) static LTower ltower_define(const char *kind, int64_t base, const char *name, LTowerDefiner call,
                                                   void *code, void *env) {
    if (env) {
        if (ltower_definers_n == ltower_definers_cap) {
            ltower_definers_cap = ltower_definers_cap ? 2 * ltower_definers_cap : 8;
            ltower_definers = (void **)lc_gc_realloc(ltower_definers, ltower_definers_cap * sizeof *ltower_definers);
        }
        ltower_definers[ltower_definers_n++] = env;
    }
    return ltower_wrap(logos_rt_value_defined_real((const uint8_t *)kind, strlen(kind), base, (const uint8_t *)name,
                                                   strlen(name), call, code, env));
}

/* A value crossing to a task or through a channel: one holding a real number a function of the program defines stays
   with the program, which stops in the interpreter's words. */
__attribute__((unused)) static void ltower_crossing(LTower v) {
    if (!logos_rt_value_sendable(v->h)) ltower_refused();
}

/* A fresh handle of the cell's value, for the runtime to take: a function's answer handed back to it. */
__attribute__((unused)) static LogosRtValue *ltower_hand_out(LTower v) { return logos_rt_value_clone(v->h); }

__attribute__((unused)) static LTower ltower_item_at(LTower a, int64_t index) {
    return ltower_wrap(logos_rt_value_item_at(a->h, index));
}

/* `items start through end of a`. */
__attribute__((unused)) static LTower ltower_slice(LTower a, LTower start, LTower end) {
    return ltower_wrap(logos_rt_value_slice(a->h, start->h, end->h));
}

__attribute__((unused)) static int64_t ltower_length(LTower a) {
    int64_t n = 0;
    if (!logos_rt_value_length(a->h, &n)) ltower_refused();
    return n;
}

__attribute__((unused)) static bool ltower_contains(LTower a, LTower v) {
    int32_t found = 0;
    if (!logos_rt_value_contains(a->h, v->h, &found)) ltower_refused();
    return found != 0;
}

/* The truth a condition reads: a residue-dependent or provisional answer stops the program. */
__attribute__((unused)) static bool ltower_truth(LTower a) {
    int32_t truth = 0;
    if (!logos_rt_value_truth(a->h, &truth)) ltower_refused();
    return truth != 0;
}

/* The `Int` a place holding a machine integer reads. */
__attribute__((unused)) static int64_t ltower_int_of(LTower a) {
    int64_t n = 0;
    if (!logos_rt_value_int_of(a->h, &n)) ltower_refused();
    return n;
}

/* The `Int` a place holding a machine integer reads, refused in that place's own sentences: `past_word` for an
   integer past a word, `fraction` for a fraction. */
__attribute__((unused)) static int64_t ltower_int_at(LTower a, const char *past_word, const char *fraction) {
    int64_t n = 0;
    if (!logos_rt_value_int_at(a->h, (const uint8_t *)past_word, strlen(past_word), (const uint8_t *)fraction,
                               strlen(fraction), &n))
        ltower_refused();
    return n;
}

__attribute__((unused)) static void ltower_push(LTower list, LTower v) {
    if (!logos_rt_value_push(list->h, v->h)) ltower_refused();
}

__attribute__((unused)) static void ltower_set_item(LTower list, LTower index, LTower v) {
    if (!logos_rt_value_set_item(list->h, index->h, v->h)) ltower_refused();
}

__attribute__((unused)) static LTower ltower_pop(LTower list) { return ltower_wrap(logos_rt_value_pop(list->h)); }

/* How the value prints, as the runtime's text. */
__attribute__((unused)) static char *ltower_to_string(LTower a) {
    size_t len = 0;
    uint8_t *bytes = logos_rt_value_show(a->h, &len);
    if (!bytes) ltower_refused();
    char *text = (char *)lc_gc_alloc_atomic(len + 1);
    memcpy(text, bytes, len);
    text[len] = 0;
    logos_rt_value_text_free(bytes, len);
    return text;
}

__attribute__((unused)) static void show_ltower(LTower a) {
    char *s = ltower_to_string(a);
    printf("%s\n", s);
    lc_gc_free(s);
}

/* The items a `Repeat` over the value binds, taken once before the loop runs: a list's items, a text's
   characters. */
__attribute__((unused)) static LTower ltower_items(LTower a) { return ltower_wrap(logos_rt_value_items(a->h)); }
