use std::collections::HashMap;
use std::fmt::Write;

use crate::analysis::TypeRegistry;
use crate::ast::stmt::*;
use crate::intern::{Interner, Symbol};

// =============================================================================
// C Runtime — embedded as a string constant
// =============================================================================

const C_RUNTIME: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <inttypes.h>

/* ========== Dynamic Array: Seq_i64 ========== */

typedef struct { int64_t *data; size_t len; size_t cap; } Seq_i64;

static Seq_i64 seq_i64_new(void) { return (Seq_i64){NULL, 0, 0}; }

static void seq_i64_push(Seq_i64 *s, int64_t val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (int64_t *)realloc(s->data, s->cap * sizeof(int64_t));
    }
    s->data[s->len++] = val;
}

static int64_t seq_i64_get(Seq_i64 *s, int64_t idx) { return s->data[idx - 1]; }
static void seq_i64_set(Seq_i64 *s, int64_t idx, int64_t val) { s->data[idx - 1] = val; }
static int64_t seq_i64_len(Seq_i64 *s) { return (int64_t)s->len; }

/* ========== Dynamic Array: Seq_bool ========== */

typedef struct { bool *data; size_t len; size_t cap; } Seq_bool;

static Seq_bool seq_bool_new(void) { return (Seq_bool){NULL, 0, 0}; }

static void seq_bool_push(Seq_bool *s, bool val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (bool *)realloc(s->data, s->cap * sizeof(bool));
    }
    s->data[s->len++] = val;
}

static bool seq_bool_get(Seq_bool *s, int64_t idx) { return s->data[idx - 1]; }
static void seq_bool_set(Seq_bool *s, int64_t idx, bool val) { s->data[idx - 1] = val; }
static int64_t seq_bool_len(Seq_bool *s) { return (int64_t)s->len; }

/* ========== Dynamic Array: Seq_str ========== */

typedef struct { char **data; size_t len; size_t cap; } Seq_str;

static Seq_str seq_str_new(void) { return (Seq_str){NULL, 0, 0}; }

static void seq_str_push(Seq_str *s, char *val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (char **)realloc(s->data, s->cap * sizeof(char *));
    }
    s->data[s->len++] = val;
}

static char *seq_str_get(Seq_str *s, int64_t idx) { return s->data[idx - 1]; }
static int64_t seq_str_len(Seq_str *s) { return (int64_t)s->len; }

/* ========== Hash Map: Map_i64_i64 (open addressing) ========== */

typedef struct {
    int64_t *keys;
    int64_t *vals;
    uint8_t *state;
    size_t cap;
    size_t len;
} Map_i64_i64;

static size_t _map_hash(int64_t key, size_t cap) {
    uint64_t h = (uint64_t)key;
    h = (h ^ (h >> 30)) * 0xbf58476d1ce4e5b9ULL;
    h = (h ^ (h >> 27)) * 0x94d049bb133111ebULL;
    h = h ^ (h >> 31);
    return (size_t)(h % cap);
}

static Map_i64_i64 map_i64_i64_new(void) {
    size_t cap = 16;
    Map_i64_i64 m;
    m.keys = (int64_t *)calloc(cap, sizeof(int64_t));
    m.vals = (int64_t *)calloc(cap, sizeof(int64_t));
    m.state = (uint8_t *)calloc(cap, sizeof(uint8_t));
    m.cap = cap;
    m.len = 0;
    return m;
}

static Map_i64_i64 map_i64_i64_with_capacity(int64_t cap_hint) {
    size_t cap = 16;
    while (cap < (size_t)(cap_hint * 2)) cap *= 2;
    Map_i64_i64 m;
    m.keys = (int64_t *)calloc(cap, sizeof(int64_t));
    m.vals = (int64_t *)calloc(cap, sizeof(int64_t));
    m.state = (uint8_t *)calloc(cap, sizeof(uint8_t));
    m.cap = cap;
    m.len = 0;
    return m;
}

static void _map_resize(Map_i64_i64 *m) {
    size_t old_cap = m->cap;
    int64_t *old_keys = m->keys;
    int64_t *old_vals = m->vals;
    uint8_t *old_state = m->state;
    size_t new_cap = old_cap * 2;
    m->keys = (int64_t *)calloc(new_cap, sizeof(int64_t));
    m->vals = (int64_t *)calloc(new_cap, sizeof(int64_t));
    m->state = (uint8_t *)calloc(new_cap, sizeof(uint8_t));
    m->cap = new_cap;
    m->len = 0;
    for (size_t i = 0; i < old_cap; i++) {
        if (old_state[i]) {
            size_t idx = _map_hash(old_keys[i], new_cap);
            while (m->state[idx]) idx = (idx + 1) % new_cap;
            m->keys[idx] = old_keys[i];
            m->vals[idx] = old_vals[i];
            m->state[idx] = 1;
            m->len++;
        }
    }
    free(old_keys); free(old_vals); free(old_state);
}

static void map_i64_i64_set(Map_i64_i64 *m, int64_t key, int64_t val) {
    if (m->len * 4 >= m->cap * 3) _map_resize(m);
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) { m->vals[idx] = val; return; }
        idx = (idx + 1) % m->cap;
    }
    m->keys[idx] = key;
    m->vals[idx] = val;
    m->state[idx] = 1;
    m->len++;
}

static int64_t map_i64_i64_get(Map_i64_i64 *m, int64_t key) {
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) return m->vals[idx];
        idx = (idx + 1) % m->cap;
    }
    return 0;
}

/* ========== String Hash Function ========== */

static size_t _map_hash_str(const char *key, size_t cap) {
    uint64_t h = 5381;
    while (*key) { h = ((h << 5) + h) + (uint8_t)*key; key++; }
    return (size_t)(h % cap);
}

/* ========== Hash Map: Map_str_i64 ========== */

typedef struct {
    char **keys; int64_t *vals; uint8_t *state; size_t cap; size_t len;
} Map_str_i64;

static Map_str_i64 map_str_i64_new(void) {
    size_t cap = 16;
    Map_str_i64 m = {(char **)calloc(cap, sizeof(char *)),
        (int64_t *)calloc(cap, sizeof(int64_t)),
        (uint8_t *)calloc(cap, 1), cap, 0};
    return m;
}

static void _map_str_i64_resize(Map_str_i64 *m) {
    size_t old_cap = m->cap;
    char **ok = m->keys; int64_t *ov = m->vals; uint8_t *os = m->state;
    size_t nc = old_cap * 2;
    m->keys = (char **)calloc(nc, sizeof(char *));
    m->vals = (int64_t *)calloc(nc, sizeof(int64_t));
    m->state = (uint8_t *)calloc(nc, 1); m->cap = nc; m->len = 0;
    for (size_t i = 0; i < old_cap; i++) {
        if (os[i]) {
            size_t idx = _map_hash_str(ok[i], nc);
            while (m->state[idx]) idx = (idx + 1) % nc;
            m->keys[idx] = ok[i]; m->vals[idx] = ov[i]; m->state[idx] = 1; m->len++;
        }
    }
    free(ok); free(ov); free(os);
}

static void map_str_i64_set(Map_str_i64 *m, const char *key, int64_t val) {
    if (m->len * 4 >= m->cap * 3) _map_str_i64_resize(m);
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) { m->vals[idx] = val; return; }
        idx = (idx + 1) % m->cap;
    }
    m->keys[idx] = strdup(key); m->vals[idx] = val; m->state[idx] = 1; m->len++;
}

static int64_t map_str_i64_get(Map_str_i64 *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return m->vals[idx];
        idx = (idx + 1) % m->cap;
    }
    return 0;
}

static bool map_str_i64_contains(Map_str_i64 *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return true;
        idx = (idx + 1) % m->cap;
    }
    return false;
}

/* ========== Hash Map: Map_str_str ========== */

typedef struct {
    char **keys; char **vals; uint8_t *state; size_t cap; size_t len;
} Map_str_str;

static Map_str_str map_str_str_new(void) {
    size_t cap = 16;
    Map_str_str m = {(char **)calloc(cap, sizeof(char *)),
        (char **)calloc(cap, sizeof(char *)),
        (uint8_t *)calloc(cap, 1), cap, 0};
    return m;
}

static void _map_str_str_resize(Map_str_str *m) {
    size_t old_cap = m->cap;
    char **ok = m->keys; char **ov = m->vals; uint8_t *os = m->state;
    size_t nc = old_cap * 2;
    m->keys = (char **)calloc(nc, sizeof(char *));
    m->vals = (char **)calloc(nc, sizeof(char *));
    m->state = (uint8_t *)calloc(nc, 1); m->cap = nc; m->len = 0;
    for (size_t i = 0; i < old_cap; i++) {
        if (os[i]) {
            size_t idx = _map_hash_str(ok[i], nc);
            while (m->state[idx]) idx = (idx + 1) % nc;
            m->keys[idx] = ok[i]; m->vals[idx] = ov[i]; m->state[idx] = 1; m->len++;
        }
    }
    free(ok); free(ov); free(os);
}

static void map_str_str_set(Map_str_str *m, const char *key, char *val) {
    if (m->len * 4 >= m->cap * 3) _map_str_str_resize(m);
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) { m->vals[idx] = val; return; }
        idx = (idx + 1) % m->cap;
    }
    m->keys[idx] = strdup(key); m->vals[idx] = val; m->state[idx] = 1; m->len++;
}

static char *map_str_str_get(Map_str_str *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return m->vals[idx];
        idx = (idx + 1) % m->cap;
    }
    return "";
}

static bool map_str_str_contains(Map_str_str *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return true;
        idx = (idx + 1) % m->cap;
    }
    return false;
}

/* ========== Hash Map: Map_i64_str ========== */

typedef struct {
    int64_t *keys; char **vals; uint8_t *state; size_t cap; size_t len;
} Map_i64_str;

static Map_i64_str map_i64_str_new(void) {
    size_t cap = 16;
    Map_i64_str m = {(int64_t *)calloc(cap, sizeof(int64_t)),
        (char **)calloc(cap, sizeof(char *)),
        (uint8_t *)calloc(cap, 1), cap, 0};
    return m;
}

static void _map_i64_str_resize(Map_i64_str *m) {
    size_t old_cap = m->cap;
    int64_t *ok = m->keys; char **ov = m->vals; uint8_t *os = m->state;
    size_t nc = old_cap * 2;
    m->keys = (int64_t *)calloc(nc, sizeof(int64_t));
    m->vals = (char **)calloc(nc, sizeof(char *));
    m->state = (uint8_t *)calloc(nc, 1); m->cap = nc; m->len = 0;
    for (size_t i = 0; i < old_cap; i++) {
        if (os[i]) {
            size_t idx = _map_hash(ok[i], nc);
            while (m->state[idx]) idx = (idx + 1) % nc;
            m->keys[idx] = ok[i]; m->vals[idx] = ov[i]; m->state[idx] = 1; m->len++;
        }
    }
    free(ok); free(ov); free(os);
}

static void map_i64_str_set(Map_i64_str *m, int64_t key, char *val) {
    if (m->len * 4 >= m->cap * 3) _map_i64_str_resize(m);
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) { m->vals[idx] = val; return; }
        idx = (idx + 1) % m->cap;
    }
    m->keys[idx] = key; m->vals[idx] = val; m->state[idx] = 1; m->len++;
}

static char *map_i64_str_get(Map_i64_str *m, int64_t key) {
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) return m->vals[idx];
        idx = (idx + 1) % m->cap;
    }
    return "";
}

static bool map_i64_str_contains(Map_i64_str *m, int64_t key) {
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) return true;
        idx = (idx + 1) % m->cap;
    }
    return false;
}

/* ========== String Helpers ========== */

static char *str_concat(const char *a, const char *b) {
    size_t la = strlen(a), lb = strlen(b);
    char *r = (char *)malloc(la + lb + 1);
    memcpy(r, a, la);
    memcpy(r + la, b, lb + 1);
    return r;
}

static char *i64_to_str(int64_t n) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%" PRId64, n);
    return strdup(buf);
}

/* ========== Dynamic Array: Seq_f64 ========== */

typedef struct { double *data; size_t len; size_t cap; } Seq_f64;

static Seq_f64 seq_f64_new(void) { return (Seq_f64){NULL, 0, 0}; }

static void seq_f64_push(Seq_f64 *s, double val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (double *)realloc(s->data, s->cap * sizeof(double));
    }
    s->data[s->len++] = val;
}

static double seq_f64_get(Seq_f64 *s, int64_t idx) { return s->data[idx - 1]; }
static void seq_f64_set(Seq_f64 *s, int64_t idx, double val) { s->data[idx - 1] = val; }
static int64_t seq_f64_len(Seq_f64 *s) { return (int64_t)s->len; }

/* ========== Pop Operations ========== */

static int64_t seq_i64_pop(Seq_i64 *s) { return s->data[--s->len]; }
static bool seq_bool_pop(Seq_bool *s) { return s->data[--s->len]; }
static char *seq_str_pop(Seq_str *s) { return s->data[--s->len]; }
static double seq_f64_pop(Seq_f64 *s) { return s->data[--s->len]; }

/* ========== Contains Operations ========== */

static bool seq_i64_contains(Seq_i64 *s, int64_t val) {
    for (size_t i = 0; i < s->len; i++) if (s->data[i] == val) return true;
    return false;
}
static bool seq_bool_contains(Seq_bool *s, bool val) {
    for (size_t i = 0; i < s->len; i++) if (s->data[i] == val) return true;
    return false;
}
static bool seq_str_contains(Seq_str *s, const char *val) {
    for (size_t i = 0; i < s->len; i++) if (strcmp(s->data[i], val) == 0) return true;
    return false;
}
static bool seq_f64_contains(Seq_f64 *s, double val) {
    for (size_t i = 0; i < s->len; i++) if (s->data[i] == val) return true;
    return false;
}
static bool map_i64_i64_contains(Map_i64_i64 *m, int64_t key) {
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) return true;
        idx = (idx + 1) % m->cap;
    }
    return false;
}

/* ========== Copy Operations ========== */

static Seq_i64 seq_i64_copy(Seq_i64 *s) {
    Seq_i64 r = {(int64_t *)malloc(s->len * sizeof(int64_t)), s->len, s->len};
    memcpy(r.data, s->data, s->len * sizeof(int64_t));
    return r;
}
static Seq_bool seq_bool_copy(Seq_bool *s) {
    Seq_bool r = {(bool *)malloc(s->len * sizeof(bool)), s->len, s->len};
    memcpy(r.data, s->data, s->len * sizeof(bool));
    return r;
}
static Seq_str seq_str_copy(Seq_str *s) {
    Seq_str r = {(char **)malloc(s->len * sizeof(char *)), s->len, s->len};
    for (size_t i = 0; i < s->len; i++) r.data[i] = strdup(s->data[i]);
    return r;
}
static Seq_f64 seq_f64_copy(Seq_f64 *s) {
    Seq_f64 r = {(double *)malloc(s->len * sizeof(double)), s->len, s->len};
    memcpy(r.data, s->data, s->len * sizeof(double));
    return r;
}

/* ========== String Operations ========== */

static bool str_equals(const char *a, const char *b) { return strcmp(a, b) == 0; }
static int64_t str_len(const char *s) { return (int64_t)strlen(s); }

/* ========== IO ========== */

static void show_i64(int64_t x) { printf("%" PRId64 "\n", x); }
static void show_f64(double x) { printf("%g\n", x); }
static void show_bool(bool x) { printf("%s\n", x ? "true" : "false"); }
static void show_str(const char *s) { printf("%s\n", s); }

static void show_seq_i64(Seq_i64 *s) {
    printf("[");
    for (size_t i = 0; i < s->len; i++) {
        if (i > 0) printf(", ");
        printf("%" PRId64, s->data[i]);
    }
    printf("]\n");
}

static void show_seq_bool(Seq_bool *s) {
    printf("[");
    for (size_t i = 0; i < s->len; i++) {
        if (i > 0) printf(", ");
        printf("%s", s->data[i] ? "true" : "false");
    }
    printf("]\n");
}

static void show_seq_str(Seq_str *s) {
    printf("[");
    for (size_t i = 0; i < s->len; i++) {
        if (i > 0) printf(", ");
        printf("%s", s->data[i]);
    }
    printf("]\n");
}

static void show_seq_f64(Seq_f64 *s) {
    printf("[");
    for (size_t i = 0; i < s->len; i++) {
        if (i > 0) printf(", ");
        printf("%g", s->data[i]);
    }
    printf("]\n");
}

/* ========== WithCapacity ========== */

static Seq_i64 seq_i64_with_capacity(int64_t cap) {
    Seq_i64 s = {(int64_t *)malloc(cap * sizeof(int64_t)), 0, (size_t)cap};
    return s;
}
static Seq_bool seq_bool_with_capacity(int64_t cap) {
    Seq_bool s = {(bool *)malloc(cap * sizeof(bool)), 0, (size_t)cap};
    return s;
}
static Seq_str seq_str_with_capacity(int64_t cap) {
    Seq_str s = {(char **)malloc(cap * sizeof(char *)), 0, (size_t)cap};
    return s;
}
static Seq_f64 seq_f64_with_capacity(int64_t cap) {
    Seq_f64 s = {(double *)malloc(cap * sizeof(double)), 0, (size_t)cap};
    return s;
}

/* ========== Slice Operations ========== */

static Seq_i64 seq_i64_slice(Seq_i64 *s, int64_t start, int64_t end) {
    Seq_i64 r = seq_i64_new();
    for (int64_t i = start; i <= end && i <= (int64_t)s->len; i++) {
        seq_i64_push(&r, s->data[i - 1]);
    }
    return r;
}

static Seq_bool seq_bool_slice(Seq_bool *s, int64_t start, int64_t end) {
    Seq_bool r = seq_bool_new();
    for (int64_t i = start; i <= end && i <= (int64_t)s->len; i++) {
        seq_bool_push(&r, s->data[i - 1]);
    }
    return r;
}

static Seq_str seq_str_slice(Seq_str *s, int64_t start, int64_t end) {
    Seq_str r = seq_str_new();
    for (int64_t i = start; i <= end && i <= (int64_t)s->len; i++) {
        seq_str_push(&r, s->data[i - 1]);
    }
    return r;
}

static Seq_f64 seq_f64_slice(Seq_f64 *s, int64_t start, int64_t end) {
    Seq_f64 r = seq_f64_new();
    for (int64_t i = start; i <= end && i <= (int64_t)s->len; i++) {
        seq_f64_push(&r, s->data[i - 1]);
    }
    return r;
}

/* ========== Set: Set_i64 (open addressing) ========== */

typedef struct {
    int64_t *keys; uint8_t *state; size_t cap; size_t len;
} Set_i64;

static Set_i64 set_i64_new(void) {
    size_t cap = 16;
    Set_i64 s = {(int64_t *)calloc(cap, sizeof(int64_t)),
        (uint8_t *)calloc(cap, 1), cap, 0};
    return s;
}

static void _set_i64_resize(Set_i64 *s) {
    size_t old_cap = s->cap;
    int64_t *ok = s->keys; uint8_t *os = s->state;
    size_t nc = old_cap * 2;
    s->keys = (int64_t *)calloc(nc, sizeof(int64_t));
    s->state = (uint8_t *)calloc(nc, 1); s->cap = nc; s->len = 0;
    for (size_t i = 0; i < old_cap; i++) {
        if (os[i]) {
            size_t idx = _map_hash(ok[i], nc);
            while (s->state[idx]) idx = (idx + 1) % nc;
            s->keys[idx] = ok[i]; s->state[idx] = 1; s->len++;
        }
    }
    free(ok); free(os);
}

static void set_i64_add(Set_i64 *s, int64_t val) {
    if (s->len * 4 >= s->cap * 3) _set_i64_resize(s);
    size_t idx = _map_hash(val, s->cap);
    while (s->state[idx]) {
        if (s->keys[idx] == val) return;
        idx = (idx + 1) % s->cap;
    }
    s->keys[idx] = val; s->state[idx] = 1; s->len++;
}

static bool set_i64_contains(Set_i64 *s, int64_t val) {
    size_t idx = _map_hash(val, s->cap);
    while (s->state[idx]) {
        if (s->keys[idx] == val) return true;
        idx = (idx + 1) % s->cap;
    }
    return false;
}

static void set_i64_remove(Set_i64 *s, int64_t val) {
    size_t idx = _map_hash(val, s->cap);
    while (s->state[idx]) {
        if (s->keys[idx] == val) { s->state[idx] = 0; s->len--; return; }
        idx = (idx + 1) % s->cap;
    }
}

static int64_t set_i64_len(Set_i64 *s) { return (int64_t)s->len; }

/* ========== Set: Set_str (open addressing) ========== */

typedef struct {
    char **keys; uint8_t *state; size_t cap; size_t len;
} Set_str;

static Set_str set_str_new(void) {
    size_t cap = 16;
    Set_str s = {(char **)calloc(cap, sizeof(char *)),
        (uint8_t *)calloc(cap, 1), cap, 0};
    return s;
}

static void _set_str_resize(Set_str *s) {
    size_t old_cap = s->cap;
    char **ok = s->keys; uint8_t *os = s->state;
    size_t nc = old_cap * 2;
    s->keys = (char **)calloc(nc, sizeof(char *));
    s->state = (uint8_t *)calloc(nc, 1); s->cap = nc; s->len = 0;
    for (size_t i = 0; i < old_cap; i++) {
        if (os[i]) {
            size_t idx = _map_hash_str(ok[i], nc);
            while (s->state[idx]) idx = (idx + 1) % nc;
            s->keys[idx] = ok[i]; s->state[idx] = 1; s->len++;
        }
    }
    free(ok); free(os);
}

static void set_str_add(Set_str *s, const char *val) {
    if (s->len * 4 >= s->cap * 3) _set_str_resize(s);
    size_t idx = _map_hash_str(val, s->cap);
    while (s->state[idx]) {
        if (strcmp(s->keys[idx], val) == 0) return;
        idx = (idx + 1) % s->cap;
    }
    s->keys[idx] = strdup(val); s->state[idx] = 1; s->len++;
}

static bool set_str_contains(Set_str *s, const char *val) {
    size_t idx = _map_hash_str(val, s->cap);
    while (s->state[idx]) {
        if (strcmp(s->keys[idx], val) == 0) return true;
        idx = (idx + 1) % s->cap;
    }
    return false;
}

static void set_str_remove(Set_str *s, const char *val) {
    size_t idx = _map_hash_str(val, s->cap);
    while (s->state[idx]) {
        if (strcmp(s->keys[idx], val) == 0) { s->state[idx] = 0; s->len--; return; }
        idx = (idx + 1) % s->cap;
    }
}

static int64_t set_str_len(Set_str *s) { return (int64_t)s->len; }

/* ========== Native Function Support ========== */

static int64_t logos_parseInt(const char *s) { return atoll(s); }

static int _logos_argc;
static char **_logos_argv;

static Seq_str logos_args(void) {
    Seq_str s = seq_str_new();
    for (int i = 0; i < _logos_argc; i++) {
        seq_str_push(&s, _logos_argv[i]);
    }
    return s;
}

"#;

// =============================================================================
// C Identifier Escaping
// =============================================================================

fn is_c_reserved(name: &str) -> bool {
    matches!(name,
        // C keywords
        "auto" | "break" | "case" | "char" | "const" | "continue" | "default" |
        "do" | "double" | "else" | "enum" | "extern" | "float" | "for" | "goto" |
        "if" | "int" | "long" | "register" | "return" | "short" | "signed" |
        "sizeof" | "static" | "struct" | "switch" | "typedef" | "union" |
        "unsigned" | "void" | "volatile" | "while" |
        // C99
        "inline" | "restrict" | "_Bool" | "_Complex" | "_Imaginary" |
        // C11
        "_Alignas" | "_Alignof" | "_Atomic" | "_Generic" | "_Noreturn" |
        "_Static_assert" | "_Thread_local" |
        // C23
        "bool" | "true" | "false" | "nullptr" | "alignas" | "alignof" |
        "constexpr" | "static_assert" | "thread_local" | "typeof" |
        // Standard library types/functions that we use in the runtime
        "printf" | "malloc" | "calloc" | "realloc" | "free" | "memcpy" |
        "strlen" | "strdup" | "snprintf" | "atoll" | "atof" |
        // POSIX common
        "size_t" | "ssize_t" | "ptrdiff_t" | "intptr_t" |
        // Our runtime identifiers (prevent user collision)
        "main" | "argc" | "argv"
    )
}

fn escape_c_ident(name: &str) -> String {
    if is_c_reserved(name) {
        format!("logos_{}", name)
    } else {
        name.to_string()
    }
}

// =============================================================================
// C Type System
// =============================================================================

#[derive(Clone, Debug, PartialEq)]
enum CType {
    Int64,
    Float64,
    Bool,
    String,
    SeqI64,
    SeqBool,
    SeqStr,
    SeqF64,
    MapI64I64,
    MapStrI64,
    MapStrStr,
    MapI64Str,
    SetI64,
    SetStr,
    Struct(Symbol),
    Enum(Symbol),
    Void,
}

fn c_type_str(ty: &CType) -> &'static str {
    match ty {
        CType::Int64 => "int64_t",
        CType::Float64 => "double",
        CType::Bool => "bool",
        CType::String => "char *",
        CType::SeqI64 => "Seq_i64",
        CType::SeqBool => "Seq_bool",
        CType::SeqStr => "Seq_str",
        CType::SeqF64 => "Seq_f64",
        CType::MapI64I64 => "Map_i64_i64",
        CType::MapStrI64 => "Map_str_i64",
        CType::MapStrStr => "Map_str_str",
        CType::MapI64Str => "Map_i64_str",
        CType::SetI64 => "Set_i64",
        CType::SetStr => "Set_str",
        CType::Struct(_) | CType::Enum(_) => "/* user type */",
        CType::Void => "void",
    }
}

fn c_type_str_resolved(ty: &CType, interner: &Interner) -> String {
    match ty {
        CType::Struct(sym) | CType::Enum(sym) => escape_c_ident(interner.resolve(*sym)),
        _ => c_type_str(ty).to_string(),
    }
}

fn field_type_to_ctype(ft: &crate::analysis::FieldType, interner: &Interner, registry: &TypeRegistry) -> CType {
    match ft {
        crate::analysis::FieldType::Primitive(sym) | crate::analysis::FieldType::Named(sym) => {
            match interner.resolve(*sym) {
                "Int" | "Nat" => CType::Int64,
                "Float" => CType::Float64,
                "Bool" => CType::Bool,
                "Text" => CType::String,
                _ => {
                    match registry.get(*sym) {
                        Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*sym),
                        Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*sym),
                        _ => CType::Int64,
                    }
                }
            }
        }
        crate::analysis::FieldType::Generic { .. } => CType::Int64,
        crate::analysis::FieldType::TypeParam(_) => CType::Int64,
    }
}

fn resolve_type_expr(ty: &TypeExpr, interner: &Interner) -> CType {
    resolve_type_expr_with_registry(ty, interner, None)
}

fn resolve_type_expr_with_registry(ty: &TypeExpr, interner: &Interner, registry: Option<&TypeRegistry>) -> CType {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            match interner.resolve(*sym) {
                "Int" | "Nat" => CType::Int64,
                "Float" => CType::Float64,
                "Bool" => CType::Bool,
                "Text" => CType::String,
                _ => {
                    if let Some(reg) = registry {
                        match reg.get(*sym) {
                            Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*sym),
                            Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*sym),
                            _ => CType::Int64,
                        }
                    } else {
                        CType::Int64
                    }
                }
            }
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            match base_name {
                "Seq" | "List" => {
                    if let Some(elem) = params.first() {
                        match resolve_type_expr(elem, interner) {
                            CType::Bool => CType::SeqBool,
                            CType::String => CType::SeqStr,
                            CType::Float64 => CType::SeqF64,
                            _ => CType::SeqI64,
                        }
                    } else {
                        CType::SeqI64
                    }
                }
                "Map" => {
                    let key_type = params.first().map(|p| resolve_type_expr(p, interner)).unwrap_or(CType::Int64);
                    let val_type = params.get(1).map(|p| resolve_type_expr(p, interner)).unwrap_or(CType::Int64);
                    match (&key_type, &val_type) {
                        (CType::String, CType::Int64) => CType::MapStrI64,
                        (CType::String, CType::String) => CType::MapStrStr,
                        (CType::Int64, CType::String) => CType::MapI64Str,
                        _ => CType::MapI64I64,
                    }
                }
                "Set" => {
                    if let Some(elem) = params.first() {
                        match resolve_type_expr(elem, interner) {
                            CType::String => CType::SetStr,
                            _ => CType::SetI64,
                        }
                    } else {
                        CType::SetI64
                    }
                }
                _ => CType::Int64,
            }
        }
        _ => CType::Int64,
    }
}

// =============================================================================
// Codegen Context
// =============================================================================

struct CContext<'a> {
    vars: HashMap<Symbol, CType>,
    funcs: HashMap<Symbol, CType>,
    interner: &'a Interner,
    registry: &'a TypeRegistry,
}

impl<'a> CContext<'a> {
    fn new(interner: &'a Interner, registry: &'a TypeRegistry) -> Self {
        Self {
            vars: HashMap::new(),
            funcs: HashMap::new(),
            interner,
            registry,
        }
    }

    fn resolve(&self, sym: Symbol) -> String {
        escape_c_ident(self.interner.resolve(sym))
    }
}

// =============================================================================
// Type Inference
// =============================================================================

fn infer_expr_type(expr: &Expr, ctx: &CContext) -> CType {
    match expr {
        Expr::Literal(Literal::Number(_)) => CType::Int64,
        Expr::Literal(Literal::Float(_)) => CType::Float64,
        Expr::Literal(Literal::Boolean(_)) => CType::Bool,
        Expr::Literal(Literal::Text(_)) => CType::String,
        Expr::Literal(Literal::Nothing) => CType::Void,
        Expr::Literal(_) => CType::Int64,
        Expr::Identifier(sym) => ctx.vars.get(sym).cloned().unwrap_or(CType::Int64),
        Expr::BinaryOp { op, left, right } => {
            match op {
                BinaryOpKind::Eq | BinaryOpKind::NotEq
                | BinaryOpKind::Lt | BinaryOpKind::LtEq
                | BinaryOpKind::Gt | BinaryOpKind::GtEq
                | BinaryOpKind::And | BinaryOpKind::Or => CType::Bool,
                BinaryOpKind::Concat => CType::String,
                BinaryOpKind::Add | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply | BinaryOpKind::Divide
                | BinaryOpKind::Modulo => {
                    let lt = infer_expr_type(left, ctx);
                    let rt = infer_expr_type(right, ctx);
                    if lt == CType::String || rt == CType::String {
                        CType::String
                    } else if lt == CType::Float64 || rt == CType::Float64 {
                        CType::Float64
                    } else {
                        CType::Int64
                    }
                }
            }
        }
        Expr::Call { function, .. } => ctx.funcs.get(function).cloned().unwrap_or(CType::Int64),
        Expr::CallExpr { .. } => CType::Int64,
        Expr::Length { .. } => CType::Int64,
        Expr::Index { collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => CType::Int64,
                    Some(CType::SeqBool) => CType::Bool,
                    Some(CType::SeqStr) => CType::String,
                    Some(CType::SeqF64) => CType::Float64,
                    Some(CType::MapI64I64) => CType::Int64,
                    Some(CType::MapStrI64) => CType::Int64,
                    Some(CType::MapStrStr) => CType::String,
                    Some(CType::MapI64Str) => CType::String,
                    _ => CType::Int64,
                }
            } else {
                CType::Int64
            }
        }
        Expr::New { type_name, type_args, .. } => {
            let name = ctx.interner.resolve(*type_name);
            match name {
                "Seq" | "List" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::Bool => CType::SeqBool,
                            CType::String => CType::SeqStr,
                            CType::Float64 => CType::SeqF64,
                            _ => CType::SeqI64,
                        }
                    } else {
                        CType::SeqI64
                    }
                }
                "Map" => {
                    let key_type = type_args.first().map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    let val_type = type_args.get(1).map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    match (&key_type, &val_type) {
                        (CType::String, CType::Int64) => CType::MapStrI64,
                        (CType::String, CType::String) => CType::MapStrStr,
                        (CType::Int64, CType::String) => CType::MapI64Str,
                        _ => CType::MapI64I64,
                    }
                }
                "Set" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::String => CType::SetStr,
                            _ => CType::SetI64,
                        }
                    } else {
                        CType::SetI64
                    }
                }
                _ => {
                    match ctx.registry.get(*type_name) {
                        Some(crate::analysis::TypeDef::Struct { .. }) => CType::Struct(*type_name),
                        Some(crate::analysis::TypeDef::Enum { .. }) => CType::Enum(*type_name),
                        _ => CType::Int64,
                    }
                }
            }
        }
        Expr::WithCapacity { value, .. } => infer_expr_type(value, ctx),
        Expr::Copy { expr: inner } => infer_expr_type(inner, ctx),
        Expr::List(elems) => {
            if let Some(first) = elems.first() {
                match infer_expr_type(first, ctx) {
                    CType::Bool => CType::SeqBool,
                    CType::String => CType::SeqStr,
                    CType::Float64 => CType::SeqF64,
                    _ => CType::SeqI64,
                }
            } else {
                CType::SeqI64
            }
        }
        Expr::Contains { .. } => CType::Bool,
        Expr::Give { value } => infer_expr_type(value, ctx),
        Expr::Slice { collection, .. } => infer_expr_type(collection, ctx),
        Expr::FieldAccess { object, field } => {
            let obj_type = infer_expr_type(object, ctx);
            if let CType::Struct(sym) = obj_type {
                if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(sym) {
                    for f in fields {
                        if f.name == *field {
                            return field_type_to_ctype(&f.ty, ctx.interner, ctx.registry);
                        }
                    }
                }
            }
            CType::Int64
        }
        Expr::NewVariant { enum_name, .. } => CType::Enum(*enum_name),
        Expr::OptionSome { .. } | Expr::OptionNone => CType::Int64,
        _ => CType::Int64,
    }
}

// =============================================================================
// Expression Codegen
// =============================================================================

fn codegen_expr(expr: &Expr, ctx: &CContext) -> String {
    match expr {
        Expr::Literal(lit) => codegen_literal(lit, ctx),
        Expr::Identifier(sym) => ctx.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            let lt = infer_expr_type(left, ctx);
            let rt = infer_expr_type(right, ctx);

            // String concatenation via + operator
            if *op == BinaryOpKind::Add && (lt == CType::String || rt == CType::String) {
                let ls = if lt == CType::String {
                    codegen_expr(left, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(left, ctx))
                };
                let rs = if rt == CType::String {
                    codegen_expr(right, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(right, ctx))
                };
                return format!("str_concat({}, {})", ls, rs);
            }

            if *op == BinaryOpKind::Concat {
                let ls = if lt == CType::String {
                    codegen_expr(left, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(left, ctx))
                };
                let rs = if rt == CType::String {
                    codegen_expr(right, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(right, ctx))
                };
                return format!("str_concat({}, {})", ls, rs);
            }

            // String comparison
            if (*op == BinaryOpKind::Eq || *op == BinaryOpKind::NotEq) && (lt == CType::String || rt == CType::String) {
                let l = codegen_expr(left, ctx);
                let r = codegen_expr(right, ctx);
                if *op == BinaryOpKind::Eq {
                    return format!("str_equals({}, {})", l, r);
                } else {
                    return format!("(!str_equals({}, {}))", l, r);
                }
            }

            let l = codegen_expr(left, ctx);
            let r = codegen_expr(right, ctx);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And => "&&",
                BinaryOpKind::Or => "||",
                BinaryOpKind::Concat => "+",
            };
            format!("({} {} {})", l, op_str, r)
        }
        Expr::Call { function, args } => {
            let raw_name = ctx.interner.resolve(*function);
            match raw_name {
                "args" => "logos_args()".to_string(),
                "parseInt" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "\"0\"".to_string()
                    };
                    format!("logos_parseInt({})", arg)
                }
                "parseFloat" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "\"0\"".to_string()
                    };
                    format!("atof({})", arg)
                }
                _ => {
                    let fname = escape_c_ident(raw_name);
                    let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, ctx)).collect();
                    format!("{}({})", fname, args_str.join(", "))
                }
            }
        }
        Expr::Index { collection, index } => {
            let idx = codegen_expr(index, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => format!("seq_i64_get(&{}, {})", coll, idx),
                    Some(CType::SeqBool) => format!("seq_bool_get(&{}, {})", coll, idx),
                    Some(CType::SeqStr) => format!("seq_str_get(&{}, {})", coll, idx),
                    Some(CType::SeqF64) => format!("seq_f64_get(&{}, {})", coll, idx),
                    Some(CType::MapI64I64) => format!("map_i64_i64_get(&{}, {})", coll, idx),
                    Some(CType::MapStrI64) => format!("map_str_i64_get(&{}, {})", coll, idx),
                    Some(CType::MapStrStr) => format!("map_str_str_get(&{}, {})", coll, idx),
                    Some(CType::MapI64Str) => format!("map_i64_str_get(&{}, {})", coll, idx),
                    _ => format!("seq_i64_get(&{}, {})", coll, idx),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_get(&{}, {})", coll, idx)
            }
        }
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => format!("seq_i64_len(&{})", coll),
                    Some(CType::SeqBool) => format!("seq_bool_len(&{})", coll),
                    Some(CType::SeqStr) => format!("seq_str_len(&{})", coll),
                    Some(CType::SeqF64) => format!("seq_f64_len(&{})", coll),
                    Some(CType::String) => format!("str_len({})", coll),
                    Some(CType::SetI64) => format!("set_i64_len(&{})", coll),
                    Some(CType::SetStr) => format!("set_str_len(&{})", coll),
                    _ => format!("seq_i64_len(&{})", coll),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_len(&{})", coll)
            }
        }
        Expr::Contains { collection, value } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => format!("seq_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::SeqBool) => format!("seq_bool_contains(&{}, {})", coll, val_str),
                    Some(CType::SeqStr) => format!("seq_str_contains(&{}, {})", coll, val_str),
                    Some(CType::SeqF64) => format!("seq_f64_contains(&{}, {})", coll, val_str),
                    Some(CType::MapI64I64) => format!("map_i64_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::MapStrI64) => format!("map_str_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::MapStrStr) => format!("map_str_str_contains(&{}, {})", coll, val_str),
                    Some(CType::MapI64Str) => format!("map_i64_str_contains(&{}, {})", coll, val_str),
                    Some(CType::SetI64) => format!("set_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::SetStr) => format!("set_str_contains(&{}, {})", coll, val_str),
                    _ => format!("seq_i64_contains(&{}, {})", coll, val_str),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_contains(&{}, {})", coll, val_str)
            }
        }
        Expr::New { type_name, type_args, init_fields, .. } => {
            let name = ctx.interner.resolve(*type_name);
            match name {
                "Seq" | "List" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::Bool => "seq_bool_new()".to_string(),
                            CType::String => "seq_str_new()".to_string(),
                            CType::Float64 => "seq_f64_new()".to_string(),
                            _ => "seq_i64_new()".to_string(),
                        }
                    } else {
                        "seq_i64_new()".to_string()
                    }
                }
                "Map" => {
                    let key_type = type_args.first().map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    let val_type = type_args.get(1).map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    match (&key_type, &val_type) {
                        (CType::String, CType::Int64) => "map_str_i64_new()".to_string(),
                        (CType::String, CType::String) => "map_str_str_new()".to_string(),
                        (CType::Int64, CType::String) => "map_i64_str_new()".to_string(),
                        _ => "map_i64_i64_new()".to_string(),
                    }
                }
                "Set" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::String => "set_str_new()".to_string(),
                            _ => "set_i64_new()".to_string(),
                        }
                    } else {
                        "set_i64_new()".to_string()
                    }
                }
                _ => {
                    // Check if this is a user-defined struct
                    if let Some(crate::analysis::TypeDef::Struct { .. }) = ctx.registry.get(*type_name) {
                        let escaped = escape_c_ident(name);
                        if init_fields.is_empty() {
                            format!("({}){{0}}", escaped)
                        } else {
                            let fields_str: Vec<String> = init_fields.iter().map(|(fname, fexpr)| {
                                let fn_str = escape_c_ident(ctx.interner.resolve(*fname));
                                let val = codegen_expr(fexpr, ctx);
                                format!(".{} = {}", fn_str, val)
                            }).collect();
                            format!("({}){{{}}}", escaped, fields_str.join(", "))
                        }
                    } else {
                        format!("/* unsupported new {} */0", name)
                    }
                }
            }
        }
        Expr::WithCapacity { value, capacity } => {
            let cap = codegen_expr(capacity, ctx);
            let ty = infer_expr_type(value, ctx);
            match ty {
                CType::MapI64I64 => format!("map_i64_i64_with_capacity({})", cap),
                CType::SeqI64 => format!("seq_i64_with_capacity({})", cap),
                CType::SeqBool => format!("seq_bool_with_capacity({})", cap),
                CType::SeqStr => format!("seq_str_with_capacity({})", cap),
                CType::SeqF64 => format!("seq_f64_with_capacity({})", cap),
                CType::String => "strdup(\"\")".to_string(),
                _ => codegen_expr(value, ctx),
            }
        }
        Expr::List(elems) => {
            if elems.is_empty() {
                return "seq_i64_new()".to_string();
            }
            let elem_type = infer_expr_type(elems.first().unwrap(), ctx);
            let (new_fn, _push_fn) = match elem_type {
                CType::Bool => ("seq_bool_new", "seq_bool_push"),
                CType::String => ("seq_str_new", "seq_str_push"),
                CType::Float64 => ("seq_f64_new", "seq_f64_push"),
                _ => ("seq_i64_new", "seq_i64_push"),
            };
            let mut parts = Vec::new();
            for e in elems {
                parts.push(codegen_expr(e, ctx));
            }
            format!("{}() /* list literal: {} */", new_fn, parts.join(", "))
        }
        Expr::Copy { expr: inner } => {
            let ty = infer_expr_type(inner, ctx);
            let inner_str = codegen_expr(inner, ctx);
            match ty {
                CType::SeqI64 => format!("seq_i64_copy(&{})", inner_str),
                CType::SeqBool => format!("seq_bool_copy(&{})", inner_str),
                CType::SeqStr => format!("seq_str_copy(&{})", inner_str),
                CType::SeqF64 => format!("seq_f64_copy(&{})", inner_str),
                _ => inner_str,
            }
        }
        Expr::Give { value } => codegen_expr(value, ctx),
        Expr::FieldAccess { object, field } => {
            let obj = codegen_expr(object, ctx);
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            format!("{}.{}", obj, fname)
        }
        Expr::NewVariant { enum_name, variant, fields } => {
            let ename = escape_c_ident(ctx.interner.resolve(*enum_name));
            let vname = escape_c_ident(ctx.interner.resolve(*variant));
            if fields.is_empty() {
                format!("({}){{{}.tag = {}_{}}}", ename, "", ename, vname)
            } else {
                // Check which fields are recursive (self-referencing pointer)
                let variant_def_fields = ctx.registry.get(*enum_name)
                    .and_then(|td| {
                        if let crate::analysis::TypeDef::Enum { variants, .. } = td {
                            variants.iter().find(|v| v.name == *variant).map(|v| &v.fields)
                        } else {
                            None
                        }
                    });

                let field_inits: Vec<String> = fields.iter().map(|(fname, fexpr)| {
                    let fn_str = escape_c_ident(ctx.interner.resolve(*fname));
                    let val = codegen_expr(fexpr, ctx);

                    // Check if this field is a self-reference (pointer type)
                    let is_recursive = variant_def_fields
                        .and_then(|vfields| vfields.iter().find(|f| f.name == *fname))
                        .map(|f| {
                            if let crate::analysis::FieldType::Named(fsym) = &f.ty {
                                *fsym == *enum_name
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);

                    if is_recursive {
                        // Heap-allocate recursive field: ({Type *__p = malloc(sizeof(Type)); *__p = val; __p;})
                        format!(".{} = ({{{}* __p = ({}*)malloc(sizeof({})); *__p = {}; __p;}})", fn_str, ename, ename, ename, val)
                    } else {
                        format!(".{} = {}", fn_str, val)
                    }
                }).collect();
                format!("({}){{{}.tag = {}_{}, .data.{} = {{{}}}}}", ename, "", ename, vname, vname, field_inits.join(", "))
            }
        }
        Expr::Slice { collection, start, end } => {
            let start_str = codegen_expr(start, ctx);
            let end_str = codegen_expr(end, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqBool) => format!("seq_bool_slice(&{}, {}, {})", coll, start_str, end_str),
                    Some(CType::SeqStr) => format!("seq_str_slice(&{}, {}, {})", coll, start_str, end_str),
                    Some(CType::SeqF64) => format!("seq_f64_slice(&{}, {}, {})", coll, start_str, end_str),
                    _ => format!("seq_i64_slice(&{}, {}, {})", coll, start_str, end_str),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_slice(&{}, {}, {})", coll, start_str, end_str)
            }
        }
        Expr::Escape { code, .. } => {
            format!("/* Escape: {} */0", ctx.interner.resolve(*code).replace("*/", "* /"))
        }
        _ => "0".to_string(),
    }
}

fn codegen_literal(lit: &Literal, ctx: &CContext) -> String {
    match lit {
        Literal::Number(n) => format!("{}LL", n),
        Literal::Float(f) => format!("{}", f),
        Literal::Boolean(b) => if *b { "true".to_string() } else { "false".to_string() },
        Literal::Text(sym) => format!("\"{}\"", ctx.interner.resolve(*sym).replace('\\', "\\\\").replace('"', "\\\"")),
        Literal::Nothing => "0".to_string(),
        _ => "0".to_string(),
    }
}

// =============================================================================
// Statement Codegen
// =============================================================================

fn codegen_stmt(stmt: &Stmt, ctx: &mut CContext, output: &mut String, indent: usize) {
    let pad = "    ".repeat(indent);
    match stmt {
        Stmt::Let { var, value, ty, .. } => {
            let var_name = ctx.resolve(*var).to_string();

            // Determine C type
            let c_type = if let Some(ty_expr) = ty {
                resolve_type_expr(ty_expr, ctx.interner)
            } else {
                infer_expr_type(value, ctx)
            };

            // Handle list literals specially — need to build inline
            if let Expr::List(elems) = value {
                if !elems.is_empty() {
                    let (new_fn, push_fn) = match &c_type {
                        CType::SeqBool => ("seq_bool_new", "seq_bool_push"),
                        CType::SeqStr => ("seq_str_new", "seq_str_push"),
                        CType::SeqF64 => ("seq_f64_new", "seq_f64_push"),
                        _ => ("seq_i64_new", "seq_i64_push"),
                    };
                    writeln!(output, "{}{} {} = {}();", pad, c_type_str(&c_type), var_name, new_fn).unwrap();
                    for e in elems {
                        let v = codegen_expr(e, ctx);
                        writeln!(output, "{}{}(&{}, {});", pad, push_fn, var_name, v).unwrap();
                    }
                    ctx.vars.insert(*var, c_type);
                    return;
                }
            }

            let val_str = codegen_expr(value, ctx);
            let type_str = c_type_str_resolved(&c_type, ctx.interner);
            writeln!(output, "{}{} {} = {};", pad, type_str, var_name, val_str).unwrap();
            ctx.vars.insert(*var, c_type);
        }
        Stmt::Set { target, value } => {
            let val_str = codegen_expr(value, ctx);
            let var_name = ctx.resolve(*target);
            writeln!(output, "{}{} = {};", pad, var_name, val_str).unwrap();
        }
        Stmt::Show { object, .. } => {
            let ty = infer_expr_type(object, ctx);
            let val_str = codegen_expr(object, ctx);
            match ty {
                CType::Int64 => writeln!(output, "{}show_i64({});", pad, val_str).unwrap(),
                CType::Float64 => writeln!(output, "{}show_f64({});", pad, val_str).unwrap(),
                CType::Bool => writeln!(output, "{}show_bool({});", pad, val_str).unwrap(),
                CType::String => writeln!(output, "{}show_str({});", pad, val_str).unwrap(),
                CType::SeqI64 => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_i64(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_i64 __tmp = {}; show_seq_i64(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::SeqBool => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_bool(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_bool __tmp = {}; show_seq_bool(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::SeqStr => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_str(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_str __tmp = {}; show_seq_str(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::SeqF64 => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_f64(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_f64 __tmp = {}; show_seq_f64(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::Struct(sym) => {
                    if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(sym) {
                        let struct_name = escape_c_ident(ctx.interner.resolve(sym));
                        // Print struct as: StructName(field1: val1, field2: val2)
                        let mut fmt_parts = Vec::new();
                        let mut arg_parts = Vec::new();
                        fmt_parts.push(format!("{}(", struct_name));
                        for (i, f) in fields.iter().enumerate() {
                            let fname = escape_c_ident(ctx.interner.resolve(f.name));
                            let ctype = field_type_to_ctype(&f.ty, ctx.interner, ctx.registry);
                            if i > 0 { fmt_parts.push(", ".to_string()); }
                            let fmt_spec = match ctype {
                                CType::Int64 => "%\" PRId64 \"",
                                CType::Float64 => "%g",
                                CType::Bool => "%s",
                                CType::String => "%s",
                                _ => "%\" PRId64 \"",
                            };
                            fmt_parts.push(format!("{}: {}", fname, fmt_spec));
                            match ctype {
                                CType::Bool => arg_parts.push(format!("{}.{} ? \"true\" : \"false\"", val_str, fname)),
                                _ => arg_parts.push(format!("{}.{}", val_str, fname)),
                            }
                        }
                        fmt_parts.push(")".to_string());
                        let fmt_string = fmt_parts.join("");
                        if arg_parts.is_empty() {
                            writeln!(output, "{}printf(\"{}\\n\");", pad, fmt_string).unwrap();
                        } else {
                            writeln!(output, "{}printf(\"{}\\n\", {});", pad, fmt_string, arg_parts.join(", ")).unwrap();
                        }
                    } else {
                        writeln!(output, "{}show_i64({});", pad, val_str).unwrap();
                    }
                }
                _ => writeln!(output, "{}show_i64({});", pad, val_str).unwrap(),
            }
        }
        Stmt::Return { value } => {
            if let Some(val) = value {
                let val_str = codegen_expr(val, ctx);
                writeln!(output, "{}return {};", pad, val_str).unwrap();
            } else {
                writeln!(output, "{}return;", pad).unwrap();
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr(cond, ctx);
            writeln!(output, "{}if ({}) {{", pad, cond_str).unwrap();
            for s in *then_block {
                codegen_stmt(s, ctx, output, indent + 1);
            }
            if let Some(eb) = else_block {
                writeln!(output, "{}}} else {{", pad).unwrap();
                for s in *eb {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
            }
            writeln!(output, "{}}}", pad).unwrap();
        }
        Stmt::While { cond, body, .. } => {
            let cond_str = codegen_expr(cond, ctx);
            writeln!(output, "{}while ({}) {{", pad, cond_str).unwrap();
            for s in *body {
                codegen_stmt(s, ctx, output, indent + 1);
            }
            writeln!(output, "{}}}", pad).unwrap();
        }
        Stmt::Push { collection, value } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => writeln!(output, "{}seq_i64_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SeqBool) => writeln!(output, "{}seq_bool_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SeqStr) => writeln!(output, "{}seq_str_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SeqF64) => writeln!(output, "{}seq_f64_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    _ => writeln!(output, "{}seq_i64_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                }
            }
        }
        Stmt::Pop { collection, into } => {
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                let (pop_fn, elem_type) = match ctx.vars.get(sym) {
                    Some(CType::SeqBool) => ("seq_bool_pop", CType::Bool),
                    Some(CType::SeqStr) => ("seq_str_pop", CType::String),
                    Some(CType::SeqF64) => ("seq_f64_pop", CType::Float64),
                    _ => ("seq_i64_pop", CType::Int64),
                };
                if let Some(var) = into {
                    let var_name = ctx.resolve(*var).to_string();
                    writeln!(output, "{}{} {} = {}(&{});", pad, c_type_str(&elem_type), var_name, pop_fn, coll_name).unwrap();
                    ctx.vars.insert(*var, elem_type);
                } else {
                    writeln!(output, "{}{}(&{});", pad, pop_fn, coll_name).unwrap();
                }
            }
        }
        Stmt::Call { function, args } => {
            let raw_name = ctx.interner.resolve(*function);
            let fname = escape_c_ident(raw_name);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, ctx)).collect();
            writeln!(output, "{}{}({});", pad, fname, args_str.join(", ")).unwrap();
        }
        Stmt::SetIndex { collection, index, value } => {
            let idx_str = codegen_expr(index, ctx);
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => writeln!(output, "{}seq_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::SeqBool) => writeln!(output, "{}seq_bool_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::SeqF64) => writeln!(output, "{}seq_f64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapI64I64) => writeln!(output, "{}map_i64_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapStrI64) => writeln!(output, "{}map_str_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapStrStr) => writeln!(output, "{}map_str_str_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapI64Str) => writeln!(output, "{}map_i64_str_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    _ => writeln!(output, "{}seq_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                }
            }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            // Check for Range-based iteration (optimized)
            if let Expr::Range { start, end } = iterable {
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => if let Some(s) = syms.first() { *s } else { return },
                };
                let var_name = ctx.resolve(var_sym).to_string();
                let start_str = codegen_expr(start, ctx);
                let end_str = codegen_expr(end, ctx);
                writeln!(output, "{}for (int64_t {} = {}; {} <= {}; {}++) {{", pad, var_name, start_str, var_name, end_str, var_name).unwrap();
                ctx.vars.insert(var_sym, CType::Int64);
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                writeln!(output, "{}}}", pad).unwrap();
                return;
            }

            let coll_type = infer_expr_type(iterable, ctx);

            // Map iteration — direct bucket scan
            let map_types = match &coll_type {
                CType::MapI64I64 => Some((CType::Int64, CType::Int64)),
                CType::MapStrI64 => Some((CType::String, CType::Int64)),
                CType::MapStrStr => Some((CType::String, CType::String)),
                CType::MapI64Str => Some((CType::Int64, CType::String)),
                _ => None,
            };

            if let Some((key_type, val_type)) = map_types {
                let iter_str = codegen_expr(iterable, ctx);
                writeln!(output, "{}for (size_t __mi = 0; __mi < {}.cap; __mi++) {{", pad, iter_str).unwrap();
                writeln!(output, "{}    if (!{}.state[__mi]) continue;", pad, iter_str).unwrap();

                match pattern {
                    Pattern::Tuple(syms) if syms.len() >= 2 => {
                        let k_sym = syms[0];
                        let v_sym = syms[1];
                        let k_name = ctx.resolve(k_sym).to_string();
                        let v_name = ctx.resolve(v_sym).to_string();
                        writeln!(output, "{}    {} {} = {}.keys[__mi];", pad, c_type_str(&key_type), k_name, iter_str).unwrap();
                        writeln!(output, "{}    {} {} = {}.vals[__mi];", pad, c_type_str(&val_type), v_name, iter_str).unwrap();
                        ctx.vars.insert(k_sym, key_type);
                        ctx.vars.insert(v_sym, val_type);
                    }
                    _ => {
                        let var_sym = match pattern {
                            Pattern::Identifier(sym) => *sym,
                            Pattern::Tuple(syms) => if let Some(s) = syms.first() { *s } else { return },
                        };
                        let var_name = ctx.resolve(var_sym).to_string();
                        writeln!(output, "{}    {} {} = {}.keys[__mi];", pad, c_type_str(&key_type), var_name, iter_str).unwrap();
                        ctx.vars.insert(var_sym, key_type);
                    }
                }

                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                writeln!(output, "{}}}", pad).unwrap();
            } else {
                // Seq iteration
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => if let Some(s) = syms.first() { *s } else { return },
                };
                let var_name = ctx.resolve(var_sym).to_string();
                let iter_str = codegen_expr(iterable, ctx);
                let (len_fn, get_fn, elem_type) = match coll_type {
                    CType::SeqBool => ("seq_bool_len", "seq_bool_get", CType::Bool),
                    CType::SeqStr => ("seq_str_len", "seq_str_get", CType::String),
                    CType::SeqF64 => ("seq_f64_len", "seq_f64_get", CType::Float64),
                    _ => ("seq_i64_len", "seq_i64_get", CType::Int64),
                };
                writeln!(output, "{}for (int64_t __idx = 1; __idx <= {}(&{}); __idx++) {{", pad, len_fn, iter_str).unwrap();
                writeln!(output, "{}    {} {} = {}(&{}, __idx);", pad, c_type_str(&elem_type), var_name, get_fn, iter_str).unwrap();
                ctx.vars.insert(var_sym, elem_type);
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                writeln!(output, "{}}}", pad).unwrap();
            }
        }
        Stmt::Add { value, collection } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SetI64) => writeln!(output, "{}set_i64_add(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SetStr) => writeln!(output, "{}set_str_add(&{}, {});", pad, coll_name, val_str).unwrap(),
                    _ => writeln!(output, "{}set_i64_add(&{}, {});", pad, coll_name, val_str).unwrap(),
                }
            }
        }
        Stmt::Remove { value, collection } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SetI64) => writeln!(output, "{}set_i64_remove(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SetStr) => writeln!(output, "{}set_str_remove(&{}, {});", pad, coll_name, val_str).unwrap(),
                    _ => writeln!(output, "{}set_i64_remove(&{}, {});", pad, coll_name, val_str).unwrap(),
                }
            }
        }
        Stmt::SetField { object, field, value } => {
            let obj = codegen_expr(object, ctx);
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            let val = codegen_expr(value, ctx);
            writeln!(output, "{}{}.{} = {};", pad, obj, fname, val).unwrap();
        }
        Stmt::Inspect { target, arms, .. } => {
            let target_str = codegen_expr(target, ctx);
            let target_type = infer_expr_type(target, ctx);
            let enum_sym = if let CType::Enum(sym) = target_type { Some(sym) } else { None };

            let mut first = true;
            for arm in arms {
                if let Some(variant_sym) = arm.variant {
                    let ename = arm.enum_name
                        .map(|s| escape_c_ident(ctx.interner.resolve(s)))
                        .or_else(|| enum_sym.map(|s| escape_c_ident(ctx.interner.resolve(s))))
                        .unwrap_or_else(|| "Unknown".to_string());
                    let vname = escape_c_ident(ctx.interner.resolve(variant_sym));

                    if first {
                        writeln!(output, "{}if ({}.tag == {}_{}) {{", pad, target_str, ename, vname).unwrap();
                        first = false;
                    } else {
                        writeln!(output, "{}}} else if ({}.tag == {}_{}) {{", pad, target_str, ename, vname).unwrap();
                    }

                    // Extract bindings from variant fields
                    for (field_name, binding_name) in &arm.bindings {
                        let fname = escape_c_ident(ctx.interner.resolve(*field_name));
                        let bname = escape_c_ident(ctx.interner.resolve(*binding_name));
                        // Infer the field type from the registry, detecting recursive fields
                        let (field_ctype, is_recursive_field) = if let Some(esym) = enum_sym {
                            if let Some(crate::analysis::TypeDef::Enum { variants, .. }) = ctx.registry.get(esym) {
                                variants.iter()
                                    .find(|v| v.name == variant_sym)
                                    .and_then(|v| v.fields.iter().find(|f| f.name == *field_name))
                                    .map(|f| {
                                        let is_self = matches!(&f.ty, crate::analysis::FieldType::Named(fsym) if *fsym == esym);
                                        (field_type_to_ctype(&f.ty, ctx.interner, ctx.registry), is_self)
                                    })
                                    .unwrap_or((CType::Int64, false))
                            } else {
                                (CType::Int64, false)
                            }
                        } else {
                            (CType::Int64, false)
                        };
                        let type_str = c_type_str_resolved(&field_ctype, ctx.interner);
                        if is_recursive_field {
                            // Recursive field is a pointer — dereference to get value copy
                            writeln!(output, "{}    {} {} = *{}.data.{}.{};", pad, type_str, bname, target_str, vname, fname).unwrap();
                        } else {
                            writeln!(output, "{}    {} {} = {}.data.{}.{};", pad, type_str, bname, target_str, vname, fname).unwrap();
                        }
                        ctx.vars.insert(*binding_name, field_ctype);
                    }
                } else {
                    // Otherwise arm
                    if first {
                        writeln!(output, "{}{{", pad).unwrap();
                        first = false;
                    } else {
                        writeln!(output, "{}}} else {{", pad).unwrap();
                    }
                }

                for s in arm.body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
            }
            if !first {
                writeln!(output, "{}}}", pad).unwrap();
            }
        }
        Stmt::FunctionDef { .. } => {}
        _ => {
            writeln!(output, "{}/* unsupported stmt */", pad).unwrap();
        }
    }
}

// =============================================================================
// Function Codegen
// =============================================================================

fn codegen_function(stmt: &Stmt, ctx: &mut CContext, output: &mut String) {
    if let Stmt::FunctionDef { name, params, body, return_type, is_native, .. } = stmt {
        if *is_native {
            return;
        }

        let func_name = ctx.resolve(*name).to_string();

        let ret_type = if let Some(rt) = return_type {
            resolve_type_expr_with_registry(rt, ctx.interner, Some(ctx.registry))
        } else {
            CType::Void
        };

        ctx.funcs.insert(*name, ret_type.clone());

        let mut param_strs = Vec::new();
        let mut param_types = Vec::new();
        for (param_name, param_type) in params {
            let p_type = resolve_type_expr_with_registry(param_type, ctx.interner, Some(ctx.registry));
            param_strs.push(format!("{} {}", c_type_str_resolved(&p_type, ctx.interner), ctx.resolve(*param_name)));
            param_types.push((*param_name, p_type));
        }

        write!(output, "{} {}({})", c_type_str_resolved(&ret_type, ctx.interner), func_name, param_strs.join(", ")).unwrap();
        writeln!(output, " {{").unwrap();

        let saved_vars = ctx.vars.clone();
        for (pname, ptype) in &param_types {
            ctx.vars.insert(*pname, ptype.clone());
        }

        for s in *body {
            codegen_stmt(s, ctx, output, 1);
        }

        ctx.vars = saved_vars;
        writeln!(output, "}}\n").unwrap();
    }
}

// =============================================================================
// Entry Point
// =============================================================================

fn codegen_c_struct_defs(registry: &TypeRegistry, interner: &Interner, output: &mut String) {
    use std::fmt::Write;
    use std::collections::HashSet;

    // Collect all struct symbols
    let struct_syms: Vec<Symbol> = registry.iter_types()
        .filter_map(|(sym, td)| {
            if matches!(td, crate::analysis::TypeDef::Struct { .. }) { Some(*sym) } else { None }
        })
        .collect();

    // Topological sort: emit structs whose field types are already emitted first
    let mut emitted: HashSet<Symbol> = HashSet::new();
    let mut ordered: Vec<Symbol> = Vec::new();

    fn field_deps(fields: &[crate::analysis::FieldDef], registry: &TypeRegistry) -> Vec<Symbol> {
        fields.iter().filter_map(|f| {
            if let crate::analysis::FieldType::Named(sym) = &f.ty {
                if matches!(registry.get(*sym), Some(crate::analysis::TypeDef::Struct { .. })) {
                    return Some(*sym);
                }
            }
            None
        }).collect()
    }

    // Simple iterative topological sort (O(n^2) but n is small)
    let mut remaining = struct_syms;
    while !remaining.is_empty() {
        let prev_len = remaining.len();
        remaining.retain(|sym| {
            if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(*sym) {
                let deps = field_deps(fields, registry);
                if deps.iter().all(|d| emitted.contains(d)) {
                    emitted.insert(*sym);
                    ordered.push(*sym);
                    return false; // remove from remaining
                }
            }
            true
        });
        if remaining.len() == prev_len {
            // Circular dependency or missing type — emit remaining as-is
            for sym in &remaining {
                ordered.push(*sym);
            }
            break;
        }
    }

    for sym in &ordered {
        if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(*sym) {
            let name = escape_c_ident(interner.resolve(*sym));
            writeln!(output, "typedef struct {{").unwrap();
            for field in fields {
                let field_name = escape_c_ident(interner.resolve(field.name));
                let ctype = field_type_to_ctype(&field.ty, interner, registry);
                let type_str = c_type_str_resolved(&ctype, interner);
                writeln!(output, "    {} {};", type_str, field_name).unwrap();
            }
            writeln!(output, "}} {};\n", name).unwrap();
        }
    }
}

fn codegen_c_enum_defs(registry: &TypeRegistry, interner: &Interner, output: &mut String) {
    use std::fmt::Write;
    for (sym, typedef) in registry.iter_types() {
        if let crate::analysis::TypeDef::Enum { variants, .. } = typedef {
            let name = escape_c_ident(interner.resolve(*sym));

            // Tag enum
            write!(output, "typedef enum {{ ").unwrap();
            for (i, v) in variants.iter().enumerate() {
                let vname = escape_c_ident(interner.resolve(v.name));
                if i > 0 { write!(output, ", ").unwrap(); }
                write!(output, "{}_{}", name, vname).unwrap();
            }
            writeln!(output, " }} {}_tag;\n", name).unwrap();

            // Check if any variant has fields
            let has_data = variants.iter().any(|v| !v.fields.is_empty());

            // Check if any variant is recursive (contains pointer to self)
            let is_recursive = variants.iter().any(|v| {
                v.fields.iter().any(|f| {
                    if let crate::analysis::FieldType::Named(fsym) = &f.ty {
                        *fsym == *sym
                    } else {
                        false
                    }
                })
            });

            if is_recursive {
                writeln!(output, "typedef struct {} {};", name, name).unwrap();
            }

            if is_recursive {
                writeln!(output, "struct {} {{", name).unwrap();
            } else {
                writeln!(output, "typedef struct {{").unwrap();
            }
            writeln!(output, "    {}_tag tag;", name).unwrap();
            if has_data {
                writeln!(output, "    union {{").unwrap();
                for v in variants {
                    if v.fields.is_empty() { continue; }
                    let vname = escape_c_ident(interner.resolve(v.name));
                    writeln!(output, "        struct {{").unwrap();
                    for f in &v.fields {
                        let fname = escape_c_ident(interner.resolve(f.name));
                        let is_self_ref = if let crate::analysis::FieldType::Named(fsym) = &f.ty {
                            *fsym == *sym
                        } else {
                            false
                        };
                        if is_self_ref {
                            writeln!(output, "            {} *{};", name, fname).unwrap();
                        } else {
                            let ctype = field_type_to_ctype(&f.ty, interner, registry);
                            let type_str = c_type_str_resolved(&ctype, interner);
                            writeln!(output, "            {} {};", type_str, fname).unwrap();
                        }
                    }
                    writeln!(output, "        }} {};", vname).unwrap();
                }
                writeln!(output, "    }} data;").unwrap();
            }
            if is_recursive {
                writeln!(output, "}};\n").unwrap();
            } else {
                writeln!(output, "}} {};\n", name).unwrap();
            }
        }
    }
}

pub fn codegen_program_c(stmts: &[Stmt], _registry: &TypeRegistry, interner: &Interner) -> String {
    let mut output = String::with_capacity(4096);
    let mut ctx = CContext::new(interner, _registry);

    output.push_str(C_RUNTIME);

    // Emit struct and enum type definitions
    codegen_c_struct_defs(_registry, interner, &mut output);
    codegen_c_enum_defs(_registry, interner, &mut output);

    // First pass: register all function return types (for forward references)
    for stmt in stmts {
        if let Stmt::FunctionDef { name, return_type, is_native, .. } = stmt {
            if *is_native {
                let fname = interner.resolve(*name);
                let ret_type = match fname {
                    "args" => CType::SeqStr,
                    "parseInt" => CType::Int64,
                    "parseFloat" => CType::Float64,
                    _ => {
                        if let Some(rt) = return_type {
                            resolve_type_expr_with_registry(rt, interner, Some(_registry))
                        } else {
                            CType::Void
                        }
                    }
                };
                ctx.funcs.insert(*name, ret_type);
            } else {
                let ret_type = if let Some(rt) = return_type {
                    resolve_type_expr_with_registry(rt, interner, Some(_registry))
                } else {
                    CType::Void
                };
                ctx.funcs.insert(*name, ret_type);
            }
        }
    }

    // Forward declarations
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, return_type, is_native, .. } = stmt {
            if *is_native {
                continue;
            }
            let func_name = ctx.resolve(*name).to_string();
            let ret_type = if let Some(rt) = return_type {
                resolve_type_expr_with_registry(rt, interner, Some(_registry))
            } else {
                CType::Void
            };
            let param_strs: Vec<String> = params.iter().map(|(pname, ptype)| {
                let p_type = resolve_type_expr_with_registry(ptype, interner, Some(_registry));
                format!("{} {}", c_type_str_resolved(&p_type, interner), ctx.resolve(*pname))
            }).collect();
            writeln!(output, "{} {}({});", c_type_str_resolved(&ret_type, interner), func_name, param_strs.join(", ")).unwrap();
        }
    }
    output.push('\n');

    // Function definitions
    for stmt in stmts {
        if let Stmt::FunctionDef { is_native: false, .. } = stmt {
            codegen_function(stmt, &mut ctx, &mut output);
        }
    }

    // Main function
    writeln!(output, "int main(int argc, char **argv) {{").unwrap();
    writeln!(output, "    _logos_argc = argc;").unwrap();
    writeln!(output, "    _logos_argv = argv;").unwrap();

    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef { .. } => continue,
            _ => codegen_stmt(stmt, &mut ctx, &mut output, 1),
        }
    }

    writeln!(output, "    return 0;").unwrap();
    writeln!(output, "}}").unwrap();

    output
}
