/* ========== The C runtime's values as exact values ==========
   What a value of the C backends' own kinds is in the value runtime: a ratio, an integer or a decimal as the
   exact number it is, a sequence as the list of its items. */

/* The value of an exact number of the C runtime: a ratio, an integer or a decimal. */
__attribute__((unused)) static LTower ltower_of_ldec(LDec d) {
    char *digits = ldec_to_string(&d);
    LTower v = (d.is_rat || d.scale == 0) ? ltower_exact(digits) : ltower_decimal(digits);
    lc_gc_free(digits);
    return v;
}

__attribute__((unused)) static LTower ltower_of_lint(LInt a) {
    char *digits = lint_to_string(a);
    LTower v = ltower_exact(digits);
    lc_gc_free(digits);
    return v;
}

/* A list of the items of a C sequence. */
#define LTOWER_OF_SEQ(name, seq, make) \
    __attribute__((unused)) static LTower ltower_of_##name(const seq *s) { \
        LTower *items = (LTower *)lc_gc_alloc((s->len ? s->len : 1) * sizeof *items); \
        for (size_t i = 0; i < s->len; i++) items[i] = make(s->data[i]); \
        LTower v = ltower_list(s->len, items); \
        lc_gc_free(items); \
        return v; \
    }
LTOWER_OF_SEQ(seq_i64, Seq_i64, ltower_int)
LTOWER_OF_SEQ(seq_f64, Seq_f64, ltower_float)
LTOWER_OF_SEQ(seq_bool, Seq_bool, ltower_bool)
LTOWER_OF_SEQ(seq_str, Seq_str, ltower_text)
#undef LTOWER_OF_SEQ
