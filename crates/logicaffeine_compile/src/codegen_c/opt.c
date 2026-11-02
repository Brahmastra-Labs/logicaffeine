/* The values a scheduler operation hands a task, in the C family's scalar packing: a Real crosses a pipe or a
 * task boundary as its bits, a Text as its pointer, anything else as itself; and a value-or-nothing — what a
 * receive, a select arm, `Try to receive` and `Await … into` read — is `logos_opt`. Shared by every tier. */

/* A Real crosses into a task as its bits, so it arrives whole. */
LOGOS_API long long logos_real_bits(double d) {
    long long bits;
    memcpy(&bits, &d, sizeof bits);
    return bits;
}

LOGOS_API double logos_bits_real(long long bits) {
    double d;
    memcpy(&d, &bits, sizeof d);
    return d;
}

/* A value, or `nothing`: what a receive reads. */
typedef struct { long long present, value; } logos_opt;

/* A value-or-nothing read as an Int: the value; `nothing` fails the task. */
LOGOS_API long long logos_opt_int(logos_opt o) {
    if (!o.present) logos_fatal("expected an Int, but the value is nothing");
    return o.value;
}

/* `a is b` between values that may be `nothing`: both nothing, or both the same value. */
LOGOS_API int logos_opt_eq(logos_opt a, logos_opt b) {
    return a.present == b.present && (!a.present || a.value == b.value);
}

/* The same, for Reals carried by their bits: equal as numbers, so `0.0` is `-0.0`. */
LOGOS_API int logos_opt_eq_real(logos_opt a, logos_opt b) {
    return a.present == b.present && (!a.present || logos_bits_real(a.value) == logos_bits_real(b.value));
}

/* The same, for Texts carried by their pointers: equal as text. */
LOGOS_API int logos_opt_eq_text(logos_opt a, logos_opt b) {
    return a.present == b.present
        && (!a.present || strcmp((const char *)(intptr_t)a.value, (const char *)(intptr_t)b.value) == 0);
}

