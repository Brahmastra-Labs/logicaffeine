/* Convergent collections — the data crate's observed-remove set, add-wins (`ORSet<T, AddWins>`) and
 * remove-wins (`ORSet<T, RemoveWins>`), and its observed-remove map of last-writer values
 * (`ORMap<K, LatestWrite<V>>`), ported step for step: what `SharedSet of T`, `SharedSet (RemoveWins) of T`
 * and `SharedMap from K to V` hold on the tree-walker and the VM, their state merge (`Merge … into …`)
 * included.
 *
 * ONE core for every C-family tier and RISC-V. A value is one 8-byte cell of the collection's single element
 * kind — an Int as itself, a Bool as 0/1, a Text as its `char *` — ordered as the interpreter's `CrdtScalar`
 * is: variants Int < Text < Bool, numbers by value, Text by its bytes, false before true. Entries are kept
 * sorted by element (a map's by key), the order both interpreters show and iterate them in. Every dot a
 * context holds arrives with a whole state, never out of order, so a context is exactly its vector clock.
 *
 * Before this text a tier defines LOGOS_FN, LOGOS_VAR and logos_alloc(n) / logos_release(p), as for
 * sched_core.c, and includes <string.h> and <stdint.h>.
 */

#define LOGOS_CRDT_INT 0
#define LOGOS_CRDT_BOOL 2
#define LOGOS_CRDT_TEXT 3

/* The variant rank of an element kind in `CrdtScalar`'s order. */
LOGOS_FN int logos_crdt_rank(int kind) {
    return kind == LOGOS_CRDT_TEXT ? 1 : kind == LOGOS_CRDT_BOOL ? 2 : 0;
}

/* -1, 0 or 1 as `a` orders before, with, or after `b`, both of `kind`. */
LOGOS_FN int logos_crdt_cmp(long long a, long long b, int kind) {
    if (kind == LOGOS_CRDT_TEXT) {
        int c = strcmp((const char *)(intptr_t)a, (const char *)(intptr_t)b);
        return (c > 0) - (c < 0);
    }
    return (a > b) - (a < b);
}

/* The last replica id handed out. Ids are 1, 2, 3, … in creation order, as the interpreter's are; only their
   distinctness reaches what a program can observe. */
LOGOS_VAR long long logos_crdt_replica_seq = 0;

LOGOS_FN long long logos_crdt_next_replica(void) {
#if defined(__CUDA_ARCH__)
    return ++logos_crdt_replica_seq;
#else
    return __atomic_add_fetch(&logos_crdt_replica_seq, 1, __ATOMIC_RELAXED);
#endif
}

/* ---- dots and vector clocks ---- */

typedef struct { long long replica, counter; } logos_dot;
typedef struct { long long n, cap; logos_dot *at; } logos_dots;
typedef struct { long long n, cap; long long *replica, *counter; } logos_vclock;

LOGOS_FN int logos_dots_has(const logos_dots *s, logos_dot d) {
    for (long long i = 0; i < s->n; i++) {
        if (s->at[i].replica == d.replica && s->at[i].counter == d.counter) return 1;
    }
    return 0;
}

LOGOS_FN void logos_dots_put(logos_dots *s, logos_dot d) {
    if (logos_dots_has(s, d)) return;
    if (s->n == s->cap) {
        long long cap = s->cap ? s->cap * 2 : 4;
        logos_dot *at = (logos_dot *)logos_alloc((size_t)cap * sizeof(logos_dot));
        if (s->n) memcpy(at, s->at, (size_t)s->n * sizeof(logos_dot));
        if (s->at) logos_release(s->at);
        s->at = at;
        s->cap = cap;
    }
    s->at[s->n++] = d;
}

LOGOS_FN logos_dots logos_dots_clone(const logos_dots *s) {
    logos_dots c = {0, 0, 0};
    for (long long i = 0; i < s->n; i++) logos_dots_put(&c, s->at[i]);
    return c;
}

/* A collection owns the cells it keeps: a Text is copied when it is stored and freed when it is dropped, so
   a caller's cell is only ever borrowed. Int and Bool cells are values. */
LOGOS_FN long long logos_crdt_own(long long cell, int kind) {
    if (kind != LOGOS_CRDT_TEXT || !cell) return cell;
    const char *t = (const char *)(intptr_t)cell;
    size_t n = strlen(t) + 1;
    char *c = (char *)logos_alloc(n);
    memcpy(c, t, n);
    return (long long)(intptr_t)c;
}

LOGOS_FN void logos_crdt_drop(long long cell, int kind) {
    if (kind == LOGOS_CRDT_TEXT && cell) logos_release((void *)(intptr_t)cell);
}

LOGOS_FN void logos_dots_free(logos_dots *s) {
    if (s->at) logos_release(s->at);
    s->n = s->cap = 0;
    s->at = 0;
}

LOGOS_FN long long logos_vclock_get(const logos_vclock *c, long long replica) {
    for (long long i = 0; i < c->n; i++) {
        if (c->replica[i] == replica) return c->counter[i];
    }
    return 0;
}

LOGOS_FN void logos_vclock_set(logos_vclock *c, long long replica, long long counter) {
    for (long long i = 0; i < c->n; i++) {
        if (c->replica[i] == replica) { c->counter[i] = counter; return; }
    }
    if (c->n == c->cap) {
        long long cap = c->cap ? c->cap * 2 : 4;
        long long *r = (long long *)logos_alloc((size_t)cap * sizeof(long long));
        long long *k = (long long *)logos_alloc((size_t)cap * sizeof(long long));
        if (c->n) { memcpy(r, c->replica, (size_t)c->n * sizeof(long long)); memcpy(k, c->counter, (size_t)c->n * sizeof(long long)); }
        if (c->replica) { logos_release(c->replica); logos_release(c->counter); }
        c->replica = r;
        c->counter = k;
        c->cap = cap;
    }
    c->replica[c->n] = replica;
    c->counter[c->n] = counter;
    c->n++;
}

/* `DotContext::next`: this replica's next event. */
LOGOS_FN logos_dot logos_vclock_next(logos_vclock *c, long long replica) {
    logos_dot d = {replica, logos_vclock_get(c, replica) + 1};
    logos_vclock_set(c, replica, d.counter);
    return d;
}

/* `DotContext::has_seen`. */
LOGOS_FN int logos_vclock_seen(const logos_vclock *c, logos_dot d) {
    return logos_vclock_get(c, d.replica) >= d.counter;
}

/* `DotContext::observed`: every event the context has seen, the Lamport time a map's write is stamped with. */
LOGOS_FN long long logos_vclock_total(const logos_vclock *c) {
    long long t = 0;
    for (long long i = 0; i < c->n; i++) t += c->counter[i];
    return t;
}

/* `DotContext::merge`: the per-replica maximum. */
LOGOS_FN void logos_vclock_join(logos_vclock *c, const logos_vclock *o) {
    for (long long i = 0; i < o->n; i++) {
        if (o->counter[i] > logos_vclock_get(c, o->replica[i])) logos_vclock_set(c, o->replica[i], o->counter[i]);
    }
}

LOGOS_FN logos_vclock logos_vclock_clone(const logos_vclock *c) {
    logos_vclock k = {0, 0, 0, 0};
    for (long long i = 0; i < c->n; i++) logos_vclock_set(&k, c->replica[i], c->counter[i]);
    return k;
}

LOGOS_FN void logos_vclock_free(logos_vclock *c) {
    if (c->replica) { logos_release(c->replica); logos_release(c->counter); }
    c->replica = c->counter = 0;
    c->n = c->cap = 0;
}

/* ---- the observed-remove set ---- */

typedef struct { long long elem; logos_dots dots; } logos_crdt_entry;

typedef struct {
    long long n, cap;
    logos_crdt_entry *e;  /* sorted by element; every entry holds at least one dot */
    logos_vclock ctx;
    long long replica;
    int kind;
    int remove_wins;
} logos_crdt_set;

/* The index of `elem` in the sorted entries, or of where it would go, with *found set. */
LOGOS_FN long long logos_crdt_set_find(const logos_crdt_set *s, long long elem, int *found) {
    long long lo = 0, hi = s->n;
    while (lo < hi) {
        long long mid = lo + (hi - lo) / 2;
        int c = logos_crdt_cmp(s->e[mid].elem, elem, s->kind);
        if (c == 0) { *found = 1; return mid; }
        if (c < 0) lo = mid + 1; else hi = mid;
    }
    *found = 0;
    return lo;
}

LOGOS_FN void logos_crdt_set_reserve(logos_crdt_set *s, long long need) {
    if (need <= s->cap) return;
    long long cap = s->cap ? s->cap : 4;
    while (cap < need) cap *= 2;
    logos_crdt_entry *e = (logos_crdt_entry *)logos_alloc((size_t)cap * sizeof(logos_crdt_entry));
    if (s->n) memcpy(e, s->e, (size_t)s->n * sizeof(logos_crdt_entry));
    if (s->e) logos_release(s->e);
    s->e = e;
    s->cap = cap;
}

/* A new, empty set of `kind` elements with a replica of its own. */
LOGOS_FN logos_crdt_set *logos_crdt_set_new(int kind, int remove_wins) {
    logos_crdt_set *s = (logos_crdt_set *)logos_alloc(sizeof(logos_crdt_set));
    s->kind = kind;
    s->remove_wins = remove_wins;
    s->replica = logos_crdt_next_replica();
    return s;
}

/* `Add v to s`: a new dot for the element. */
LOGOS_FN void logos_crdt_set_add(logos_crdt_set *s, long long elem) {
    logos_dot d = logos_vclock_next(&s->ctx, s->replica);
    int found;
    long long i = logos_crdt_set_find(s, elem, &found);
    if (!found) {
        logos_crdt_set_reserve(s, s->n + 1);
        memmove(&s->e[i + 1], &s->e[i], (size_t)(s->n - i) * sizeof(logos_crdt_entry));
        s->e[i].elem = logos_crdt_own(elem, s->kind);
        s->e[i].dots.n = s->e[i].dots.cap = 0;
        s->e[i].dots.at = 0;
        s->n++;
    }
    logos_dots_put(&s->e[i].dots, d);
}

/* `Remove v from s`: the element's dots go; the context keeps having seen them. */
LOGOS_FN void logos_crdt_set_remove(logos_crdt_set *s, long long elem) {
    int found;
    long long i = logos_crdt_set_find(s, elem, &found);
    if (!found) return;
    logos_dots_free(&s->e[i].dots);
    logos_crdt_drop(s->e[i].elem, s->kind);
    memmove(&s->e[i], &s->e[i + 1], (size_t)(s->n - i - 1) * sizeof(logos_crdt_entry));
    s->n--;
}

LOGOS_FN int logos_crdt_set_contains(const logos_crdt_set *s, long long elem) {
    int found;
    logos_crdt_set_find(s, elem, &found);
    return found;
}

LOGOS_FN long long logos_crdt_set_len(const logos_crdt_set *s) { return s->n; }

/* The `i`-th element (0-based) in the set's order. */
LOGOS_FN long long logos_crdt_set_nth(const logos_crdt_set *s, long long i) { return s->e[i].elem; }

/* `copy of s`: the same state, the same replica. */
LOGOS_FN logos_crdt_set *logos_crdt_set_clone(const logos_crdt_set *s) {
    logos_crdt_set *c = (logos_crdt_set *)logos_alloc(sizeof(logos_crdt_set));
    *c = *s;
    c->e = 0;
    c->n = c->cap = 0;
    logos_crdt_set_reserve(c, s->n);
    for (long long i = 0; i < s->n; i++) {
        c->e[i].elem = logos_crdt_own(s->e[i].elem, s->kind);
        c->e[i].dots = logos_dots_clone(&s->e[i].dots);
    }
    c->n = s->n;
    c->ctx = logos_vclock_clone(&s->ctx);
    return c;
}

/* `ORSet::merge`: for every element either side holds, the dots neither side observed removed survive, and
   the bias decides an element one side holds and the other removed. Both contexts are read as they stood
   before the merge; the target's then joins the source's. */
LOGOS_FN void logos_crdt_set_merge(logos_crdt_set *s, const logos_crdt_set *o) {
    static const logos_dots none = {0, 0, 0};
    long long cap = s->n + o->n, n = 0, i = 0, j = 0;
    logos_crdt_entry *out = cap ? (logos_crdt_entry *)logos_alloc((size_t)cap * sizeof(logos_crdt_entry)) : 0;
    while (i < s->n || j < o->n) {
        int c = i == s->n ? 1 : j == o->n ? -1 : logos_crdt_cmp(s->e[i].elem, o->e[j].elem, s->kind);
        long long elem = c <= 0 ? s->e[i].elem : o->e[j].elem;
        const logos_dots *mine = c <= 0 ? &s->e[i].dots : &none;
        const logos_dots *theirs = c >= 0 ? &o->e[j].dots : &none;
        int my_removed = 0, their_removed = 0;
        if (mine->n == 0) {
            for (long long k = 0; k < theirs->n; k++) if (logos_vclock_seen(&s->ctx, theirs->at[k])) { my_removed = 1; break; }
        }
        if (theirs->n == 0) {
            for (long long k = 0; k < mine->n; k++) if (logos_vclock_seen(&o->ctx, mine->at[k])) { their_removed = 1; break; }
        }
        logos_dots combined = {0, 0, 0};
        for (long long k = 0; k < mine->n; k++) {
            if (!logos_vclock_seen(&o->ctx, mine->at[k]) || logos_dots_has(theirs, mine->at[k])) logos_dots_put(&combined, mine->at[k]);
        }
        for (long long k = 0; k < theirs->n; k++) {
            if (!logos_vclock_seen(&s->ctx, theirs->at[k]) || logos_dots_has(mine, theirs->at[k])) logos_dots_put(&combined, theirs->at[k]);
        }
        int present = mine->n > 0 || theirs->n > 0;
        int keep = s->remove_wins ? (!my_removed && !their_removed && present) : present;
        if (keep && combined.n > 0) {
            out[n].elem = c <= 0 ? elem : logos_crdt_own(elem, s->kind);
            out[n].dots = combined;
            n++;
        } else {
            logos_dots_free(&combined);
            if (c <= 0) logos_crdt_drop(elem, s->kind);
        }
        if (c <= 0) { logos_dots_free(&s->e[i].dots); i++; }
        if (c >= 0) j++;
    }
    if (s->e) logos_release(s->e);
    s->e = out;
    s->n = n;
    s->cap = cap;
    logos_vclock_join(&s->ctx, &o->ctx);
}

/* ---- the observed-remove map of last-writer values ---- */

typedef struct { long long key; logos_dots dots; } logos_crdt_key;
/* A key's `LatestWrite`: its stamp and value. A placeholder (`LatestWrite::default()`, stamp 0, `Int(0)`) is
   what a key a merge meets before any write to it holds. */
typedef struct { long long key, stamp, value; int placeholder; } logos_crdt_value;

typedef struct {
    long long nk, capk;
    logos_crdt_key *k;    /* sorted by key; every key holds at least one dot */
    long long nv, capv;
    logos_crdt_value *v;  /* sorted by key; a value outlives its key's removal, as `ORMap::values` does */
    logos_vclock ctx;
    long long replica;
    int key_kind, value_kind;
} logos_crdt_map;

LOGOS_FN long long logos_crdt_map_find_key(const logos_crdt_map *m, long long key, int *found) {
    long long lo = 0, hi = m->nk;
    while (lo < hi) {
        long long mid = lo + (hi - lo) / 2;
        int c = logos_crdt_cmp(m->k[mid].key, key, m->key_kind);
        if (c == 0) { *found = 1; return mid; }
        if (c < 0) lo = mid + 1; else hi = mid;
    }
    *found = 0;
    return lo;
}

LOGOS_FN long long logos_crdt_map_find_value(const logos_crdt_map *m, long long key, int *found) {
    long long lo = 0, hi = m->nv;
    while (lo < hi) {
        long long mid = lo + (hi - lo) / 2;
        int c = logos_crdt_cmp(m->v[mid].key, key, m->key_kind);
        if (c == 0) { *found = 1; return mid; }
        if (c < 0) lo = mid + 1; else hi = mid;
    }
    *found = 0;
    return lo;
}

/* The dots of `key`, the key added (with no dots) when absent. */
LOGOS_FN logos_dots *logos_crdt_map_key_entry(logos_crdt_map *m, long long key) {
    int found;
    long long i = logos_crdt_map_find_key(m, key, &found);
    if (!found) {
        if (m->nk == m->capk) {
            long long cap = m->capk ? m->capk * 2 : 4;
            logos_crdt_key *k = (logos_crdt_key *)logos_alloc((size_t)cap * sizeof(logos_crdt_key));
            if (m->nk) memcpy(k, m->k, (size_t)m->nk * sizeof(logos_crdt_key));
            if (m->k) logos_release(m->k);
            m->k = k;
            m->capk = cap;
        }
        memmove(&m->k[i + 1], &m->k[i], (size_t)(m->nk - i) * sizeof(logos_crdt_key));
        m->k[i].key = logos_crdt_own(key, m->key_kind);
        m->k[i].dots.n = m->k[i].dots.cap = 0;
        m->k[i].dots.at = 0;
        m->nk++;
    }
    return &m->k[i].dots;
}

/* The value of `key`, a placeholder added when absent. */
LOGOS_FN logos_crdt_value *logos_crdt_map_value_entry(logos_crdt_map *m, long long key) {
    int found;
    long long i = logos_crdt_map_find_value(m, key, &found);
    if (!found) {
        if (m->nv == m->capv) {
            long long cap = m->capv ? m->capv * 2 : 4;
            logos_crdt_value *v = (logos_crdt_value *)logos_alloc((size_t)cap * sizeof(logos_crdt_value));
            if (m->nv) memcpy(v, m->v, (size_t)m->nv * sizeof(logos_crdt_value));
            if (m->v) logos_release(m->v);
            m->v = v;
            m->capv = cap;
        }
        memmove(&m->v[i + 1], &m->v[i], (size_t)(m->nv - i) * sizeof(logos_crdt_value));
        m->v[i].key = logos_crdt_own(key, m->key_kind);
        m->v[i].stamp = 0;
        m->v[i].value = 0;
        m->v[i].placeholder = 1;
        m->nv++;
    }
    return &m->v[i];
}

/* A new, empty map with a replica of its own. */
LOGOS_FN logos_crdt_map *logos_crdt_map_new(int key_kind, int value_kind) {
    logos_crdt_map *m = (logos_crdt_map *)logos_alloc(sizeof(logos_crdt_map));
    m->key_kind = key_kind;
    m->value_kind = value_kind;
    m->replica = logos_crdt_next_replica();
    return m;
}

/* `Set m[key] to value`: a new dot for the key, then the value stamped with the context's Lamport time. */
LOGOS_FN void logos_crdt_map_put(logos_crdt_map *m, long long key, long long value) {
    logos_dot d = logos_vclock_next(&m->ctx, m->replica);
    logos_dots_put(logos_crdt_map_key_entry(m, key), d);
    logos_crdt_value *v = logos_crdt_map_value_entry(m, key);
    if (!v->placeholder) logos_crdt_drop(v->value, m->value_kind);
    v->stamp = logos_vclock_total(&m->ctx);
    v->value = logos_crdt_own(value, m->value_kind);
    v->placeholder = 0;
}

LOGOS_FN int logos_crdt_map_contains(const logos_crdt_map *m, long long key) {
    int found;
    logos_crdt_map_find_key(m, key, &found);
    return found;
}

/* The value under a present `key`; *present is 0 when the key is absent. */
LOGOS_FN long long logos_crdt_map_get(const logos_crdt_map *m, long long key, int *present) {
    int found;
    logos_crdt_map_find_key(m, key, &found);
    if (!found) { *present = 0; return 0; }
    long long i = logos_crdt_map_find_value(m, key, &found);
    *present = 1;
    return m->v[i].value;
}

/* `Remove key from m`: the key's dots go; its value stays, as `ORMap::remove` leaves it. */
LOGOS_FN void logos_crdt_map_remove(logos_crdt_map *m, long long key) {
    int found;
    long long i = logos_crdt_map_find_key(m, key, &found);
    if (!found) return;
    logos_dots_free(&m->k[i].dots);
    logos_crdt_drop(m->k[i].key, m->key_kind);
    memmove(&m->k[i], &m->k[i + 1], (size_t)(m->nk - i - 1) * sizeof(logos_crdt_key));
    m->nk--;
}

LOGOS_FN long long logos_crdt_map_len(const logos_crdt_map *m) { return m->nk; }

/* The `i`-th key (0-based) in key order, and its value. */
LOGOS_FN long long logos_crdt_map_key_nth(const logos_crdt_map *m, long long i) { return m->k[i].key; }

LOGOS_FN long long logos_crdt_map_value_nth(const logos_crdt_map *m, long long i) {
    int present;
    return logos_crdt_map_get(m, m->k[i].key, &present);
}

/* `copy of m`: the same state, the same replica. */
LOGOS_FN logos_crdt_map *logos_crdt_map_clone(const logos_crdt_map *m) {
    logos_crdt_map *c = (logos_crdt_map *)logos_alloc(sizeof(logos_crdt_map));
    *c = *m;
    c->k = 0;
    c->capk = c->nk = 0;
    c->v = 0;
    c->capv = c->nv = 0;
    for (long long i = 0; i < m->nk; i++) {
        logos_dots *d = logos_crdt_map_key_entry(c, m->k[i].key);
        *d = logos_dots_clone(&m->k[i].dots);
    }
    for (long long i = 0; i < m->nv; i++) {
        logos_crdt_value *v = logos_crdt_map_value_entry(c, m->v[i].key);
        v->stamp = m->v[i].stamp;
        v->placeholder = m->v[i].placeholder;
        v->value = m->v[i].placeholder ? 0 : logos_crdt_own(m->v[i].value, m->value_kind);
    }
    c->ctx = logos_vclock_clone(&m->ctx);
    return c;
}

/* `LatestWrite`'s order on (stamp, value): a placeholder's value is `Int(0)`. */
LOGOS_FN int logos_crdt_write_cmp(const logos_crdt_value *a, const logos_crdt_value *b, int kind) {
    if (a->stamp != b->stamp) return a->stamp < b->stamp ? -1 : 1;
    int ka = a->placeholder ? LOGOS_CRDT_INT : kind, kb = b->placeholder ? LOGOS_CRDT_INT : kind;
    if (ka != kb) return logos_crdt_rank(ka) < logos_crdt_rank(kb) ? -1 : 1;
    return logos_crdt_cmp(a->placeholder ? 0 : a->value, b->placeholder ? 0 : b->value, ka);
}

/* `ORMap::merge`: every value takes the later write; a key keeps the dots the source added that the target had
   not seen, and loses the dots the source observed and no longer holds. Both contexts are read as they stood
   before the merge; the target's then joins the source's. */
LOGOS_FN void logos_crdt_map_merge(logos_crdt_map *m, const logos_crdt_map *o) {
    for (long long i = 0; i < o->nv; i++) {
        logos_crdt_value *mine = logos_crdt_map_value_entry(m, o->v[i].key);
        if (logos_crdt_write_cmp(&o->v[i], mine, m->value_kind) > 0) {
            if (!mine->placeholder) logos_crdt_drop(mine->value, m->value_kind);
            mine->stamp = o->v[i].stamp;
            mine->placeholder = o->v[i].placeholder;
            mine->value = o->v[i].placeholder ? 0 : logos_crdt_own(o->v[i].value, m->value_kind);
        }
    }
    for (long long i = 0; i < o->nk; i++) {
        logos_dots *mine = logos_crdt_map_key_entry(m, o->k[i].key);
        for (long long k = 0; k < o->k[i].dots.n; k++) {
            if (!logos_vclock_seen(&m->ctx, o->k[i].dots.at[k])) logos_dots_put(mine, o->k[i].dots.at[k]);
        }
    }
    long long n = 0;
    for (long long i = 0; i < m->nk; i++) {
        int found;
        long long j = logos_crdt_map_find_key(o, m->k[i].key, &found);
        const logos_dots *theirs = found ? &o->k[j].dots : 0;
        logos_dots kept = {0, 0, 0};
        for (long long k = 0; k < m->k[i].dots.n; k++) {
            logos_dot d = m->k[i].dots.at[k];
            if (!logos_vclock_seen(&o->ctx, d) || (theirs && logos_dots_has(theirs, d))) logos_dots_put(&kept, d);
        }
        logos_dots_free(&m->k[i].dots);
        if (kept.n > 0) {
            m->k[n].key = m->k[i].key;
            m->k[n].dots = kept;
            n++;
        } else {
            logos_crdt_drop(m->k[i].key, m->key_kind);
        }
    }
    m->nk = n;
    logos_vclock_join(&m->ctx, &o->ctx);
}

/* Free a set: its entries' elements and dots, the entry array, the context, the set. */
LOGOS_FN void logos_crdt_set_free(logos_crdt_set *s) {
    if (!s) return;
    for (long long i = 0; i < s->n; i++) {
        logos_dots_free(&s->e[i].dots);
        logos_crdt_drop(s->e[i].elem, s->kind);
    }
    if (s->e) logos_release(s->e);
    logos_vclock_free(&s->ctx);
    logos_release(s);
}

/* Free a map: its keys and their dots, its values and their keys, both arrays, the context, the map. */
LOGOS_FN void logos_crdt_map_free(logos_crdt_map *m) {
    if (!m) return;
    for (long long i = 0; i < m->nk; i++) {
        logos_dots_free(&m->k[i].dots);
        logos_crdt_drop(m->k[i].key, m->key_kind);
    }
    for (long long i = 0; i < m->nv; i++) {
        logos_crdt_drop(m->v[i].key, m->key_kind);
        if (!m->v[i].placeholder) logos_crdt_drop(m->v[i].value, m->value_kind);
    }
    if (m->k) logos_release(m->k);
    if (m->v) logos_release(m->v);
    logos_vclock_free(&m->ctx);
    logos_release(m);
}
