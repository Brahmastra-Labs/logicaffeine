/* ========== The exact extension on RISC-V: what the emitted assembly calls ==========
   A register holding a value of the extension holds its LTower cell (the core above, over the shim's collector).
   Every operation is one call: an operand that is a cell comes with the cell's shape (form 22), an operand of any
   other kind with its slot's shape, and is made the runtime's value on the way in, so the assembly converts nothing
   and keeps nothing across a call. A change to a list or a field goes to the register's own value, copied first
   unless its owned flag says the register holds it alone, as every `_own` helper of the shim does. */

/* The assembly that calls these declares itself (`logos_links_value_runtime`), so a shim with this adapter never
   links a program that does not ask for it, and a program that asks never links without it. */
extern const unsigned char logos_links_value_runtime[];

#define LOGOS_TOWER_FORM 22

static LTower logos_tower_cell(long long v, const logos_shape *s);

/* The runtime's list (or tuple) of the cells of a sequence of the shim, each by its part's shape. */
static LTower logos_tower_parts(const logos_seq *a, const logos_shape *s, int tuple) {
    LTower *items = (LTower *)lc_gc_alloc((size_t)(a->len ? a->len : 1) * sizeof *items);
    for (long long i = 0; i < a->len; i++) {
        items[i] = logos_tower_cell(a->data[i], (const logos_shape *)s->parts[s->n == 1 ? 0 : i]);
    }
    LTower v = tuple ? ltower_tuple((size_t)a->len, items) : ltower_list((size_t)a->len, items);
    lc_gc_free(items);
    return v;
}

/* The cell of the slot value `v` of shape `s`: the cell itself, or the runtime's value of what the slot holds. */
static LTower logos_tower_cell(long long v, const logos_shape *s) {
    switch (s->form) {
        case LOGOS_TOWER_FORM: return (LTower)v;
        case 0: return ltower_int(v);
        case 16: return ltower_bool(v != 0);
        case 8: { double f; memcpy(&f, &v, sizeof f); return ltower_float(f); }
        case 7: return ltower_text((const char *)v);
        case 9: {
            const LDec *d = (const LDec *)v;
            char *digits = ldec_to_string(d);
            LTower x = (d->is_rat || d->scale == 0) ? ltower_exact(digits) : ltower_decimal(digits);
            lc_gc_free(digits);
            return x;
        }
        case 17: return ltower_date(v);
        case 18: return ltower_moment(v);
        case 23: return ltower_nothing();
        case 24: return ltower_span((int32_t)(v >> 32), (int32_t)v);
        case 6: return ltower_int(logos_crdt_sum((const logos_crdt *)v));
        case 1: return logos_tower_parts((const logos_seq *)v, s, 0);
        case 11: return logos_tower_parts((const logos_seq *)v, s, 1);
    }
    logos_fatalf("the RISC-V runtime holds no exact value of a value of shape form %lld", s->form);
}

long long logos_tower_of(long long v, const logos_shape *s) { return (long long)logos_tower_cell(v, s); }

/* The cells of `n` slot values, each by its own shape, for an operation that takes a run of values. */
static LTower *logos_tower_run(long long n, const long long *values, const logos_shape *const *shapes) {
    LTower *cells = (LTower *)lc_gc_alloc((size_t)(n ? n : 1) * sizeof *cells);
    for (long long i = 0; i < n; i++) cells[i] = logos_tower_cell(values[i], shapes[i]);
    return cells;
}

/* The list of the `n` values in consecutive registers from `values`. */
long long logos_tower_list(long long n, const long long *values, const logos_shape *const *shapes) {
    LTower *cells = logos_tower_run(n, values, shapes);
    LTower v = ltower_list((size_t)n, cells);
    lc_gc_free(cells);
    return (long long)v;
}

/* The builtin of the extension named `name` applied to the `argc` values in consecutive registers from `args`. */
long long logos_tower_call(const char *name, long long argc, const long long *args, const logos_shape *const *shapes) {
    LTower *cells = logos_tower_run(argc, args, shapes);
    LTower v = ltower_call(name, (size_t)argc, cells);
    lc_gc_free(cells);
    return (long long)v;
}

/* A cell crossing to a task or through a channel: a real number a function of the program defines stays with the
   program, which stops in the interpreter's words. */
void logos_tower_crossing(long long cell) { ltower_crossing((LTower)cell); }

/* A function the program defines a real number by, read at the place `n` the value runtime asks for: the closure `env`
   called through its body's entry in `logos_fn_dispatch`, the closure after its argument as a `CallValue` passes it,
   and its answer made the runtime's value by the shape the table `code` holds for that body. */
static LogosRtValue *logos_tower_defines(void *code, void *env, int64_t n) {
    const long long *clo = (const long long *)env;
    const logos_shape *answers = ((const logos_shape *const *)code)[clo[0]];
    if (!answers) logos_fatal("a real number is defined by a function of one Int that answers a number");
    long long (*body)(long long, long long) = (long long (*)(long long, long long))logos_fn_dispatch[clo[0]];
    return ltower_hand_out(logos_tower_cell(body(n, (long long)clo), answers));
}

/* `definedReal(kind, base, name, clo)`: the real number the closure `clo` defines, its answers read by the shapes
   `answers` holds, one per function of the program. */
long long logos_tower_define(const char *kind, long long base, const char *name, long long clo,
                             const logos_shape *const *answers) {
    return (long long)ltower_define(kind, base, name, logos_tower_defines, (void *)answers, (void *)(intptr_t)clo);
}

/* `items start through end of a`. */
long long logos_tower_slice(long long a, const logos_shape *as, long long start, const logos_shape *ss, long long end,
                            const logos_shape *es) {
    LTower list = logos_tower_cell(a, as);
    LTower from = logos_tower_cell(start, ss);
    return (long long)ltower_slice(list, from, logos_tower_cell(end, es));
}

#define LOGOS_TOWER_BINARY(op) \
    long long logos_tower_##op(long long a, const logos_shape *as, long long b, const logos_shape *bs) { \
        LTower x = logos_tower_cell(a, as); \
        return (long long)ltower_##op(x, logos_tower_cell(b, bs)); \
    }
LOGOS_TOWER_BINARY(add)
LOGOS_TOWER_BINARY(sub)
LOGOS_TOWER_BINARY(mul)
LOGOS_TOWER_BINARY(div)
LOGOS_TOWER_BINARY(exact_div)
LOGOS_TOWER_BINARY(floor_div)
LOGOS_TOWER_BINARY(rem)
LOGOS_TOWER_BINARY(pow)
LOGOS_TOWER_BINARY(approx_eq)
LOGOS_TOWER_BINARY(seq_concat)
LOGOS_TOWER_BINARY(bit_and)
LOGOS_TOWER_BINARY(bit_or)
LOGOS_TOWER_BINARY(bit_xor)
LOGOS_TOWER_BINARY(shl)
LOGOS_TOWER_BINARY(shr)
LOGOS_TOWER_BINARY(min)
LOGOS_TOWER_BINARY(max)
LOGOS_TOWER_BINARY(power)
LOGOS_TOWER_BINARY(item)
LOGOS_TOWER_BINARY(eq)
LOGOS_TOWER_BINARY(ne)
LOGOS_TOWER_BINARY(lt)
LOGOS_TOWER_BINARY(gt)
LOGOS_TOWER_BINARY(le)
LOGOS_TOWER_BINARY(ge)
#undef LOGOS_TOWER_BINARY

#define LOGOS_TOWER_UNARY(op) \
    long long logos_tower_##op(long long a, const logos_shape *as) { return (long long)ltower_##op(logos_tower_cell(a, as)); }
LOGOS_TOWER_UNARY(not)
LOGOS_TOWER_UNARY(abs)
LOGOS_TOWER_UNARY(sqrt)
LOGOS_TOWER_UNARY(floor)
LOGOS_TOWER_UNARY(ceil)
LOGOS_TOWER_UNARY(round)
#undef LOGOS_TOWER_UNARY

long long logos_tower_length(long long a) { return ltower_length((LTower)a); }

long long logos_tower_contains(long long a, long long v, const logos_shape *vs) {
    return ltower_contains((LTower)a, logos_tower_cell(v, vs));
}

/* The truth a condition reads: a residue-dependent or provisional answer stops the program. */
long long logos_tower_truth(long long a) { return ltower_truth((LTower)a); }

/* The `Int` a place holding a machine integer reads. */
long long logos_tower_int_at(long long a, const char *past_word, const char *fraction) {
    return ltower_int_at((LTower)a, past_word, fraction);
}

/* The register's own list, in `*slot`: a copy of its cell when `*owned` is clear. */
static LTower logos_tower_own(long long *slot, long long *owned) {
    if (!*owned) {
        *slot = (long long)ltower_own((LTower)*slot);
        *owned = 1;
    }
    return (LTower)*slot;
}

/* `Push v to xs`. `v` arrives by value and is made a cell before the register's list is made its own. */
void logos_tower_push_own(long long *slot, long long *owned, long long v, const logos_shape *vs) {
    LTower item = logos_tower_cell(v, vs);
    ltower_push(logos_tower_own(slot, owned), item);
}

/* `Set item i of xs to v`. */
void logos_tower_set_item_own(long long *slot, long long *owned, long long i, const logos_shape *is, long long v,
    const logos_shape *vs) {
    LTower index = logos_tower_cell(i, is);
    LTower item = logos_tower_cell(v, vs);
    ltower_set_item(logos_tower_own(slot, owned), index, item);
}

/* `Pop from xs into v`. */
long long logos_tower_pop_own(long long *slot, long long *owned) {
    return (long long)ltower_pop(logos_tower_own(slot, owned));
}

static long long logos_tower_copy(long long cell) { return (long long)ltower_own((LTower)cell); }

/* `Push v to r's field`, the field a list of the extension: the register's own record's own cell, in place while
   the field flag is set. */
void logos_tower_field_push_own(logos_seq **slot, long long *owned, long long *field_owned, long long cell, long long v,
    const logos_shape *vs) {
    LTower item = logos_tower_cell(v, vs);
    ltower_push((LTower)logos_field_own(slot, owned, field_owned, cell, logos_tower_copy), item);
}

/* The items a `Repeat` over the value binds, taken once before the loop runs, as a sequence of the shim whose cells
   are the items' cells, which the shim's iterator walks as any sequence. */
logos_seq *logos_tower_items(long long a) {
    LTower items = ltower_items((LTower)a);
    int64_t n = ltower_length(items);
    logos_seq *s = logos_seq_new_cap(n);
    for (int64_t k = 1; k <= n; k++) logos_seq_push_inplace(s, (long long)ltower_item_at(items, k));
    return s;
}

char *logos_tower_str(long long a) { return ltower_to_string((LTower)a); }

void logos_tower_show(long long a) { show_ltower((LTower)a); }

/* Two values of which at least one is a cell, equal as the runtime compares them. */
static int logos_tower_equal(long long a, const logos_shape *as, long long b, const logos_shape *bs) {
    LTower x = logos_tower_cell(a, as);
    return ltower_truth(ltower_eq(x, logos_tower_cell(b, bs)));
}

__attribute__((constructor)) static void logos_tower_start(void) {
    if (logos_links_value_runtime[0] != 1) logos_fatal("the program does not declare the value runtime it links");
    logos_tower_text_hook = logos_tower_str;
    logos_tower_copy_hook = logos_tower_copy;
    logos_tower_equal_hook = logos_tower_equal;
}
