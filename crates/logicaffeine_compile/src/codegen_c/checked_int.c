/* ========== Checked machine-word arithmetic ========== */

/* A sum, difference or product of machine words that records in `*over` when its exact value leaves the word
   (the wrapped value it returns is then discarded, and the expression is computed exactly instead). */
__attribute__((unused)) static inline int64_t logos_ck_add(int64_t a, int64_t b, int *over) {
    int64_t r;
    if (__builtin_add_overflow(a, b, &r)) *over = 1;
    return r;
}
__attribute__((unused)) static inline int64_t logos_ck_sub(int64_t a, int64_t b, int *over) {
    int64_t r;
    if (__builtin_sub_overflow(a, b, &r)) *over = 1;
    return r;
}
__attribute__((unused)) static inline int64_t logos_ck_mul(int64_t a, int64_t b, int *over) {
    int64_t r;
    if (__builtin_mul_overflow(a, b, &r)) *over = 1;
    return r;
}

/* `a & m` for a non-negative `m`, as the VM computes it on an exact integer: a machine word's bits, and a big
   integer's low bits when `m` is a run of low ones and the integer is not negative. The VM refuses any other
   big integer, in the words `refusal` carries. */
__attribute__((unused)) static inline int64_t logos_exact_band(LInt a, int64_t m, const char *refusal) {
    if (!a.big) return a.small & m;
    uint64_t runs = (uint64_t)m;
    if (m >= 0 && ((runs + 1) & runs) == 0 && lint_cmp(a, lint_small(0)) >= 0) {
        return lint_expect_i64(lint_mod(a, lint_big(lbig_from_u64(runs + 1))));
    }
    logos_fatal(refusal);
}

/* Power-of-two reductions under the shared whole-body non-negativity proof.
   Keep runtime guards too: even if a future analysis regresses, a negative or
   promoted value takes the original exact operation rather than changing `/`
   rounding or `%` sign semantics. */
__attribute__((unused)) static inline int64_t logos_lint_mod_pow2_nonnegative(LInt a, int64_t divisor) {
    if (__builtin_expect(!a.big && a.small >= 0, 1)) return a.small & (divisor - 1);
    LInt remainder = lint_mod(a, lint_small(divisor));
    int64_t result = lint_expect_i64(remainder);
    if(remainder.big){ lbig_free(remainder.big); lc_gc_free(remainder.big); }
    return result;
}

__attribute__((unused)) static inline LInt logos_lint_div_pow2_nonnegative(
    LInt a,
    int64_t divisor,
    unsigned shift
) {
    if (__builtin_expect(!a.big && a.small >= 0, 1)) return lint_small(a.small >> shift);
    return lint_div(a, lint_small(divisor));
}

/* An exact integer's arithmetic on its machine-word path, inline: two words whose result is a word stay in the
   code computing them, and anything else takes the runtime's exact path. A zero divisor fails the run in the VM's
   words, which `refusal` carries. */
__attribute__((unused)) static inline LInt logos_lint_add(LInt a, LInt b) {
    int64_t r;
    if (__builtin_expect(!a.big && !b.big, 1) && !__builtin_add_overflow(a.small, b.small, &r)) return lint_small(r);
    return lint_add(a, b);
}
__attribute__((unused)) static inline LInt logos_lint_sub(LInt a, LInt b) {
    int64_t r;
    if (__builtin_expect(!a.big && !b.big, 1) && !__builtin_sub_overflow(a.small, b.small, &r)) return lint_small(r);
    return lint_sub(a, b);
}
__attribute__((unused)) static inline LInt logos_lint_mul(LInt a, LInt b) {
    int64_t r;
    if (__builtin_expect(!a.big && !b.big, 1) && !__builtin_mul_overflow(a.small, b.small, &r)) return lint_small(r);
    return lint_mul(a, b);
}
__attribute__((unused)) static inline int logos_lint_cmp(LInt a, LInt b) {
    if (__builtin_expect(!a.big && !b.big, 1)) return (a.small > b.small) - (a.small < b.small);
    return lint_cmp(a, b);
}
__attribute__((unused)) static inline LInt logos_lint_div(LInt a, LInt b, const char *refusal) {
    if (!b.big && b.small == 0) logos_fatal(refusal);
    if (__builtin_expect(!a.big && !b.big, 1) && !(a.small == INT64_MIN && b.small == -1)) return lint_small(a.small / b.small);
    return lint_div(a, b);
}
__attribute__((unused)) static inline LInt logos_lint_mod(LInt a, LInt b, const char *refusal) {
    if (!b.big && b.small == 0) logos_fatal(refusal);
    if (__builtin_expect(!a.big && !b.big, 1) && !(a.small == INT64_MIN && b.small == -1)) return lint_small(a.small % b.small);
    return lint_mod(a, b);
}
/* `a // b` — the quotient floored toward negative infinity: the truncated one, less one where the remainder is nonzero
   and its sign differs from the divisor's. */
__attribute__((unused)) static inline LInt logos_lint_floordiv(LInt a, LInt b, const char *refusal) {
    LInt q = logos_lint_div(a, b, refusal), r = logos_lint_mod(a, b, refusal);
    int rs = logos_lint_cmp(r, lint_small(0)), bs = logos_lint_cmp(b, lint_small(0));
    return (rs != 0 && (rs < 0) != (bs < 0)) ? logos_lint_sub(q, lint_small(1)) : q;
}

/* ========== Sizing a list before a loop fills it ========== */

/* The passes of a loop counting from `from` up to `to` (`to` itself excluded when `exclusive`), times `per_pass`:
   the elements the loop pushes. A count past a word, or a loop that never runs, reserves nothing. */
__attribute__((unused)) static inline int64_t logos_counted_fill(int64_t from, int64_t to, int exclusive, int64_t per_pass) {
    int64_t span, passes, total;
    if (__builtin_sub_overflow(to, from, &span)) return 0;
    if (!exclusive && __builtin_add_overflow(span, 1, &span)) return 0;
    passes = span;
    if (passes <= 0 || __builtin_mul_overflow(passes, per_pass, &total)) return 0;
    return total;
}

/* Room for `more` elements past the length of the list `s` (any `Seq_*`), in one allocation — at least doubling
   the capacity, so a reservation made again and again (an inner loop's, once per outer pass) still grows the list
   geometrically. */
#define LOGOS_SEQ_RESERVE(s, more)                                                                    \
    do {                                                                                              \
        __typeof__(s) __list = (s);                                                                   \
        int64_t __more = (more);                                                                      \
        if (__more > 0 && (uint64_t)__more > (uint64_t)(__list->cap - __list->len)) {                  \
            size_t __cap = __list->len + (size_t)__more;                                              \
            if (__cap < 2 * __list->cap) __cap = 2 * __list->cap;                                     \
            __list->data = (__typeof__(__list->data))lc_gc_realloc(__list->data, __cap * sizeof(*__list->data)); \
            __list->cap = __cap;                                                                      \
        }                                                                                             \
    } while (0)
