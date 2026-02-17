pub(super) const C_RUNTIME: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <inttypes.h>
#include <stdarg.h>
#include <math.h>

/* ========== String Formatting Helpers ========== */

static char *logos_center_str(const char *s, int width) {
    int len = (int)strlen(s);
    if (len >= width) return strdup(s);
    int total_pad = width - len;
    int left = total_pad / 2;
    int right = total_pad - left;
    char *result = (char *)malloc(width + 1);
    memset(result, ' ', left);
    memcpy(result + left, s, len);
    memset(result + left + len, ' ', right);
    result[width] = '\0';
    return result;
}

static char *logos_center_i64(int64_t val, int width) {
    char tmp[64];
    snprintf(tmp, 64, "%" PRId64, val);
    return logos_center_str(tmp, width);
}

static char *logos_dyn_sprintf(const char *fmt, ...) {
    va_list args, args2;
    va_start(args, fmt);
    va_copy(args2, args);
    int len = vsnprintf(NULL, 0, fmt, args);
    va_end(args);
    char *buf = (char *)malloc(len + 1);
    vsnprintf(buf, len + 1, fmt, args2);
    va_end(args2);
    return buf;
}

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
