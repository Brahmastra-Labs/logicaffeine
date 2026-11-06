/// The C runtime both C generators emit, its failure sentences filled in with the VM's own words, taken at
/// generation time from the functions the VM calls ([`super::words`]).
pub(super) fn c_runtime() -> String {
    let sentences: String = super::words::exact_number_sentences()
        .into_iter()
        .map(|(global, sentence)| format!("const char {global}[] = {};\n", super::emit::c_string_literal(&sentence)))
        .collect();
    format!("{C_RUNTIME}\n{sentences}{}", include_str!("random.c"))
        .replace("@RANDOM_BOUNDS@", &super::words::random_bounds_refused())
        .replace(
            "@INDEX_OUT_OF_BOUNDS@",
            &super::words::index_out_of_bounds(),
        )
        .replace("@DIVISION_BY_ZERO@", &super::words::division_by_zero())
        .replace(
            "@REAL_DIVISION_BY_ZERO@",
            &super::words::real_division_by_zero(),
        )
        .replace("@MODULO_BY_ZERO@", &super::words::modulo_by_zero())
}

/// [`C_GC_COLLECTOR`]'s source, as a literal a runtime written as one `concat!` can embed (the RISC-V shim's).
macro_rules! lc_gc_collector_source {
    () => {
        r#"
/* LC_GC_COLLECTOR_BEGIN */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#if defined(__GLIBC__) || defined(__APPLE__)
#define LC_GC_COLLECTS 1
#include <setjmp.h>
#include <sys/mman.h>
#if defined(__SANITIZE_ADDRESS__)
#define LC_GC_CHECKED_HEAP 1
#elif defined(__has_feature)
#if __has_feature(address_sanitizer)
#define LC_GC_CHECKED_HEAP 1
#endif
#endif
#if defined(__APPLE__)
#include <pthread.h>
#include <mach-o/dyld.h>
#include <mach-o/getsect.h>
#endif

/* The collector: mark from the roots, sweep what was not reached. Two heaps share its marking and its policy:
   - the paged heap (shipped builds): blocks of 64 KiB, each cut into slots of one size class, found from any
     address through a two-level page map, so an allocation pops a free list and a sweep is one pass over a block's
     mark bits; an object larger than the largest class is its own malloc'd block behind a header, found through a
     table of them sorted by address at each collection, and resized in place by the C library where it can be;
   - the checked heap (builds with AddressSanitizer): every object is its own malloc'd block, found through a table
     sorted by address at each collection, so the sanitizer sees each object's bounds and LeakSanitizer each one
     the sweep fails to free.
   A slot holds a 16-byte pad, then the object, then one byte: a word anywhere in it — the object's start, inside
   it, one past its end, or a little before its start — holds it. */

typedef struct { uintptr_t lo, hi; } lc_gc_range;

static lc_gc_range *lc_gc_roots; static size_t lc_gc_roots_n, lc_gc_roots_cap;
static lc_gc_range lc_gc_left[64]; static int lc_gc_left_n;
static uintptr_t lc_gc_stack_hi;
static size_t lc_gc_since, lc_gc_trigger = (size_t)8 << 20;
static int lc_gc_started, lc_gc_stress, lc_gc_collecting;
/* The heap's ledger, in the bytes each object takes in its heap (a small object's slot; a large object's header and
   payload; in the checked heap, every object's header and payload): what is held now, the most ever held at once
   (taken at each allocation, before the collection it may trigger), what was held after the last collection, the
   collections run, and what was allocated and freed in all. Deterministic but for conservative retention: a block a
   non-pointer word happens to point into stays held. */
static size_t lc_gc_live, lc_gc_high, lc_gc_after, lc_gc_collections, lc_gc_allocated, lc_gc_freed;
/* The marking stack: the objects reached and not yet scanned, as ranges. */
static lc_gc_range *lc_gc_pending; static size_t lc_gc_pending_n, lc_gc_pending_cap;
/* The objects that hold memory outside the heap (a linked runtime's), each with what gives that memory back: a
   collection calls it for each one it did not reach, before the sweep frees the object, and the teardown for each
   one left. The runtime that holds the memory may count its bytes, which the next collection's trigger counts as
   the heap's own, and add to the heap report. */
typedef struct { void *object; void (*release)(void *); } lc_gc_hold;
static lc_gc_hold *lc_gc_holds; static size_t lc_gc_holds_n, lc_gc_holds_cap;
static size_t (*lc_gc_outside)(void);
static void (*lc_gc_outside_report)(FILE *);

__attribute__((noreturn)) static void lc_gc_exhausted(void) {
    fflush(stdout);
    fputs("memory allocation failed\n", stderr);
    exit(1);
}

/* The C library's allocator, for the collector's own tables and pages, which it never scans as objects. */
static void *lc_gc_table(void *p, size_t n) {
    void *q = realloc(p, n);
    if (!q) lc_gc_exhausted();
    return q;
}

static void lc_gc_push(uintptr_t lo, uintptr_t hi) {
    if (lc_gc_pending_n == lc_gc_pending_cap) {
        lc_gc_pending_cap = lc_gc_pending_cap ? lc_gc_pending_cap * 2 : 1024;
        lc_gc_pending = (lc_gc_range *)lc_gc_table(lc_gc_pending, lc_gc_pending_cap * sizeof *lc_gc_pending);
    }
    lc_gc_pending[lc_gc_pending_n++] = (lc_gc_range){lo, hi};
}

#define LC_GC_PAD 16

#ifndef LC_GC_CHECKED_HEAP
/* ---- The paged heap ---- */
#define LC_GC_BLOCK ((uintptr_t)1 << 16)
#define LC_GC_CLASSES 21
#define LC_GC_SMALL 2048
/* The slot sizes: pad + object + one byte, rounded up. */
static const uint32_t lc_gc_class_size[LC_GC_CLASSES] = {
    32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 1024, 1280, 1536, 2048};
/* A block of slots of one class. */
typedef struct lc_gc_blk {
    uintptr_t start;
    uint32_t size;          /* the slot size */
    uint32_t nslots;
    uint8_t atomic;
    uint8_t cls;
    uint64_t *marks;        /* one bit per slot */
    uint64_t *allocated;    /* only allocated slots may enter tracing */
    struct lc_gc_blk *next; /* the blocks of one class and kind */
} lc_gc_blk;
/* A large object's header: its size, its place in the table, whether it is atomic, its mark. */
typedef struct { size_t size; uint32_t index; uint8_t atomic; uint8_t marked; } lc_gc_hdr;
static lc_gc_blk *lc_gc_classes[2][LC_GC_CLASSES];
static lc_gc_hdr **lc_gc_large; static size_t lc_gc_large_n, lc_gc_large_cap;
static uint8_t lc_gc_class_of_16[(LC_GC_SMALL / 16) + 1];
/* Blocks come from the system LC_GC_CHUNK at a time, each mapping cut to a block boundary, so nothing pads a block to
   its alignment and the address space a heap takes is its blocks'; a block a sweep empties waits among the spares for
   reuse, and goes back to the system past them. */
#define LC_GC_CHUNK 64
/* The collector's own state that points into the heap or indexes it: the free lists, the heap's bounds, the page
   map (address bits 47..32, then 31..16, to the block that owns those 64 KiB), the chunk blocks are cut from and the
   spare blocks. The globals scan skips it, or every free slot would hold the next one. */
static struct {
    void *free_list[2][LC_GC_CLASSES];
    uintptr_t heap_lo, heap_hi, large_lo, large_hi;
    uintptr_t chunk_next, chunk_end;
    uintptr_t spare[LC_GC_CHUNK];
    int spare_n;
    lc_gc_blk **map[1 << 16];
} lc_gc_own = {.heap_lo = UINTPTR_MAX};

/* A block's memory: a spare, else the next block of the chunk, mapping a fresh chunk when that one is used up. */
static uintptr_t lc_gc_block_take(void) {
    if (lc_gc_own.spare_n) return lc_gc_own.spare[--lc_gc_own.spare_n];
    if (lc_gc_own.chunk_next == lc_gc_own.chunk_end) {
        size_t want = (size_t)LC_GC_BLOCK * LC_GC_CHUNK, span = want + LC_GC_BLOCK;
        char *m = (char *)mmap(0, span, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (m == MAP_FAILED) lc_gc_exhausted();
        uintptr_t lo = ((uintptr_t)m + LC_GC_BLOCK - 1) & ~(LC_GC_BLOCK - 1), hi = lo + want;
        if (lo > (uintptr_t)m) munmap(m, lo - (uintptr_t)m);
        if ((uintptr_t)m + span > hi) munmap((void *)hi, (uintptr_t)m + span - hi);
        lc_gc_own.chunk_next = lo;
        lc_gc_own.chunk_end = hi;
    }
    uintptr_t b = lc_gc_own.chunk_next;
    lc_gc_own.chunk_next += LC_GC_BLOCK;
    return b;
}

/* A block's memory no block holds any more. */
static void lc_gc_block_give(uintptr_t b) {
    if (lc_gc_own.spare_n < LC_GC_CHUNK) lc_gc_own.spare[lc_gc_own.spare_n++] = b;
    else munmap((void *)b, LC_GC_BLOCK);
}

static void lc_gc_heap_init(void) {
    int c = 0;
    for (size_t k = 0; k <= LC_GC_SMALL / 16; k++) {
        while (lc_gc_class_size[c] < k * 16) c++;
        lc_gc_class_of_16[k] = (uint8_t)c;
    }
}

static void lc_gc_map_set(uintptr_t at, uintptr_t end, lc_gc_blk *b) {
    for (uintptr_t a = at & ~(LC_GC_BLOCK - 1); a < end; a += LC_GC_BLOCK) {
        size_t top = (a >> 32) & 0xFFFF;
        if (!lc_gc_own.map[top]) {
            lc_gc_own.map[top] = (lc_gc_blk **)calloc((size_t)1 << 16, sizeof(lc_gc_blk *));
            if (!lc_gc_own.map[top]) lc_gc_exhausted();
        }
        lc_gc_own.map[top][(a >> 16) & 0xFFFF] = b;
    }
    if (at < lc_gc_own.heap_lo) lc_gc_own.heap_lo = at;
    if (end > lc_gc_own.heap_hi) lc_gc_own.heap_hi = end;
}

static inline lc_gc_blk *lc_gc_owner(uintptr_t w) {
    lc_gc_blk **second = lc_gc_own.map[(w >> 32) & 0xFFFF];
    return second ? second[(w >> 16) & 0xFFFF] : 0;
}

/* A fresh block of class `c`, its slots on the free list. */
static void lc_gc_new_block(int atomic, int c) {
    lc_gc_blk *b = (lc_gc_blk *)calloc(1, sizeof *b);
    if (!b) lc_gc_exhausted();
    b->start = lc_gc_block_take();
    b->size = lc_gc_class_size[c];
    b->nslots = (uint32_t)(LC_GC_BLOCK / b->size);
    b->atomic = (uint8_t)atomic;
    b->cls = (uint8_t)c;
    b->marks = (uint64_t *)calloc((b->nslots + 63) / 64, sizeof(uint64_t));
    b->allocated = (uint64_t *)calloc((b->nslots + 63) / 64, sizeof(uint64_t));
    if (!b->marks || !b->allocated) lc_gc_exhausted();
    b->next = lc_gc_classes[atomic][c];
    lc_gc_classes[atomic][c] = b;
    lc_gc_map_set(b->start, b->start + LC_GC_BLOCK, b);
    void *head = lc_gc_own.free_list[atomic][c];
    for (uint32_t i = b->nslots; i-- > 0;) {
        void *slot = (void *)(b->start + (uintptr_t)i * b->size);
        *(void **)slot = head;
        head = slot;
    }
    lc_gc_own.free_list[atomic][c] = head;
}

/* The bytes an object of `n` takes in the heap: its slot, or a large object's header and payload. */
static inline size_t lc_gc_takes(size_t n) {
    size_t need = n + LC_GC_PAD + 1;
    return need <= LC_GC_SMALL ? lc_gc_class_size[lc_gc_class_of_16[(need + 15) / 16]] : sizeof(lc_gc_hdr) + n;
}

static void *lc_gc_heap_alloc(size_t n, int atomic) {
    size_t need = n + LC_GC_PAD + 1;
    if (need <= LC_GC_SMALL) {
        int c = lc_gc_class_of_16[(need + 15) / 16];
        void *slot = lc_gc_own.free_list[atomic][c];
        if (!slot) {
            lc_gc_new_block(atomic, c);
            slot = lc_gc_own.free_list[atomic][c];
        }
        lc_gc_own.free_list[atomic][c] = *(void **)slot;
        lc_gc_blk *b = lc_gc_owner((uintptr_t)slot);
        uint32_t i = (uint32_t)(((uintptr_t)slot - b->start) / b->size);
        b->allocated[i >> 6] |= (uint64_t)1 << (i & 63);
        /* Unwritten pointer-bearing capacity must not retain a prior occupant. */
        if (!atomic) memset((char *)slot + LC_GC_PAD, 0, b->size - LC_GC_PAD - 1);
        lc_gc_live += lc_gc_class_size[c];
        lc_gc_allocated += lc_gc_class_size[c];
        return (char *)slot + LC_GC_PAD;
    }
    lc_gc_live += sizeof(lc_gc_hdr) + n;
    lc_gc_allocated += sizeof(lc_gc_hdr) + n;
    lc_gc_hdr *h = (lc_gc_hdr *)malloc(sizeof *h + n + 1);
    if (!h) lc_gc_exhausted();
    if (!atomic) memset(h + 1, 0, n);
    h->size = n;
    h->atomic = (uint8_t)atomic;
    h->marked = 0;
    if (lc_gc_large_n == (size_t)UINT32_MAX) lc_gc_exhausted();
    if (lc_gc_large_n == lc_gc_large_cap) {
        lc_gc_large_cap = lc_gc_large_cap ? lc_gc_large_cap * 2 : 256;
        lc_gc_large = (lc_gc_hdr **)lc_gc_table(lc_gc_large, lc_gc_large_cap * sizeof *lc_gc_large);
    }
    h->index = (uint32_t)lc_gc_large_n;
    lc_gc_large[lc_gc_large_n++] = h;
    return h + 1;
}

/* Whether object `p` is a small one, in a block of slots. */
static inline int lc_gc_is_small(void *p) { return lc_gc_owner((uintptr_t)p) != 0; }

/* The bytes object `p` can hold. */
static size_t lc_gc_capacity(void *p) {
    lc_gc_blk *b = lc_gc_owner((uintptr_t)p);
    return b ? b->size - LC_GC_PAD - 1 : ((lc_gc_hdr *)p - 1)->size;
}

static void lc_gc_unmap(uintptr_t at, uintptr_t end) {
    for (uintptr_t a = at & ~(LC_GC_BLOCK - 1); a < end; a += LC_GC_BLOCK) lc_gc_own.map[(a >> 32) & 0xFFFF][(a >> 16) & 0xFFFF] = 0;
}

static void lc_gc_heap_free(void *p) {
    lc_gc_blk *b = lc_gc_owner((uintptr_t)p);
    if (!b) {
        lc_gc_hdr *h = (lc_gc_hdr *)p - 1;
        size_t i = h->index;
        lc_gc_live -= sizeof *h + h->size;
        lc_gc_freed += sizeof *h + h->size;
        lc_gc_large[i] = lc_gc_large[--lc_gc_large_n];
        lc_gc_large[i]->index = (uint32_t)i;
        free(h);
        return;
    }
    lc_gc_live -= b->size;
    lc_gc_freed += b->size;
    void *slot = (char *)p - LC_GC_PAD;
    uint32_t i = (uint32_t)(((uintptr_t)slot - b->start) / b->size);
    b->allocated[i >> 6] &= ~((uint64_t)1 << (i & 63));
    *(void **)slot = lc_gc_own.free_list[b->atomic][b->cls];
    lc_gc_own.free_list[b->atomic][b->cls] = slot;
}

/* A large object resized by the C library, in place where it can be. */
static void *lc_gc_large_resize(void *p, size_t n) {
    lc_gc_hdr *h = (lc_gc_hdr *)p - 1;
    size_t i = h->index, was = h->size;
    lc_gc_hdr *g = (lc_gc_hdr *)realloc(h, sizeof *g + n + 1);
    if (!g) lc_gc_exhausted();
    if (!g->atomic && n > was) memset((char *)(g + 1) + was, 0, n - was);
    g->size = n;
    lc_gc_large[i] = g;
    if (n > was) { lc_gc_live += n - was; lc_gc_allocated += n - was; }
    else { lc_gc_live -= was - n; lc_gc_freed += was - n; }
    return g + 1;
}

static int lc_gc_by_address(const void *a, const void *b) {
    uintptr_t x = (uintptr_t)*(lc_gc_hdr *const *)a, y = (uintptr_t)*(lc_gc_hdr *const *)b;
    return x < y ? -1 : x > y;
}

static void lc_gc_prepare(void) {
    for (int a = 0; a < 2; a++)
        for (int c = 0; c < LC_GC_CLASSES; c++)
            for (lc_gc_blk *b = lc_gc_classes[a][c]; b; b = b->next) memset(b->marks, 0, ((b->nslots + 63) / 64) * sizeof(uint64_t));
    qsort(lc_gc_large, lc_gc_large_n, sizeof *lc_gc_large, lc_gc_by_address);
    for (size_t i = 0; i < lc_gc_large_n; i++) {
        lc_gc_large[i]->index = (uint32_t)i;
        lc_gc_large[i]->marked = 0;
    }
    lc_gc_own.large_lo = lc_gc_large_n ? (uintptr_t)lc_gc_large[0] : 0;
    lc_gc_own.large_hi = lc_gc_large_n ? (uintptr_t)(lc_gc_large[lc_gc_large_n - 1] + 1) + lc_gc_large[lc_gc_large_n - 1]->size : 0;
}

/* A word into a large object: mark it and queue it to be scanned. */
static void lc_gc_consider_large(uintptr_t w) {
    size_t lo = 0, hi = lc_gc_large_n;
    while (hi - lo > 1) {
        size_t mid = lo + (hi - lo) / 2;
        if ((uintptr_t)lc_gc_large[mid] <= w) lo = mid; else hi = mid;
    }
    lc_gc_hdr *h = lc_gc_large[lo];
    if (w < (uintptr_t)h || w > (uintptr_t)(h + 1) + h->size || h->marked) return;
    h->marked = 1;
    if (!h->atomic) lc_gc_push((uintptr_t)(h + 1), (uintptr_t)(h + 1) + h->size);
}

/* The word `w`, if it points into an object not yet marked: mark it and queue it to be scanned. */
static inline void lc_gc_consider(uintptr_t w) {
    w &= ~(uintptr_t)1;
    if (lc_gc_large_n && w >= lc_gc_own.large_lo && w <= lc_gc_own.large_hi && !lc_gc_owner(w)) {
        lc_gc_consider_large(w);
        return;
    }
    if (w < lc_gc_own.heap_lo || w >= lc_gc_own.heap_hi) return;
    lc_gc_blk *b = lc_gc_owner(w);
    if (!b) return;
    uint32_t i = (uint32_t)((w - b->start) / b->size);
    if (i >= b->nslots) return;
    uint64_t bit = (uint64_t)1 << (i & 63);
    if (!(b->allocated[i >> 6] & bit) || (b->marks[i >> 6] & bit)) return;
    b->marks[i >> 6] |= bit;
    if (!b->atomic) {
        uintptr_t s = b->start + (uintptr_t)i * b->size;
        lc_gc_push(s + LC_GC_PAD, s + b->size - 1);
    }
}

/* Whether the marking reached object `p`. */
static int lc_gc_reached(void *p) {
    lc_gc_blk *b = lc_gc_owner((uintptr_t)p);
    if (!b) return ((lc_gc_hdr *)p - 1)->marked;
    uint32_t i = (uint32_t)(((uintptr_t)p - b->start) / b->size);
    return (int)((b->marks[i >> 6] >> (i & 63)) & 1);
}

/* Rebuild every free list from the unmarked slots; free every unmarked large object and every block left empty.
   Answers the bytes left live and adds the bytes freed to `*freed`. */
static size_t lc_gc_sweep(size_t *freed) {
    size_t live = 0;
    for (int a = 0; a < 2; a++) {
        for (int c = 0; c < LC_GC_CLASSES; c++) {
            void *head = 0;
            lc_gc_blk **at = &lc_gc_classes[a][c];
            while (*at) {
                lc_gc_blk *b = *at;
                uint32_t used = 0;
                for (uint32_t w = 0; w < (b->nslots + 63) / 64; w++) used += (uint32_t)__builtin_popcountll(b->marks[w]);
                if (used == 0) {
                    *at = b->next;
                    lc_gc_unmap(b->start, b->start + LC_GC_BLOCK);
                    lc_gc_block_give(b->start);
                    free(b->marks);
                    free(b->allocated);
                    free(b);
                    *freed += LC_GC_BLOCK;
                    continue;
                }
                for (uint32_t i = b->nslots; i-- > 0;) {
                    if (!(b->marks[i >> 6] & ((uint64_t)1 << (i & 63)))) {
                        void *slot = (void *)(b->start + (uintptr_t)i * b->size);
                        *(void **)slot = head;
                        head = slot;
                    }
                }
                memcpy(b->allocated, b->marks, ((b->nslots + 63) / 64) * sizeof(uint64_t));
                live += (size_t)used * b->size;
                *freed += (size_t)(b->nslots - used) * b->size;
                at = &b->next;
            }
            lc_gc_own.free_list[a][c] = head;
        }
    }
    size_t kept = 0;
    for (size_t i = 0; i < lc_gc_large_n; i++) {
        lc_gc_hdr *h = lc_gc_large[i];
        if (h->marked) {
            h->index = (uint32_t)kept;
            lc_gc_large[kept++] = h;
            live += h->size + sizeof *h;
        } else {
            *freed += h->size + sizeof *h;
            free(h);
        }
    }
    lc_gc_large_n = kept;
    return live;
}

static void lc_gc_heap_teardown(void) {
    for (int a = 0; a < 2; a++) {
        for (int c = 0; c < LC_GC_CLASSES; c++) {
            lc_gc_blk *b = lc_gc_classes[a][c];
            while (b) {
                lc_gc_blk *next = b->next;
                munmap((void *)b->start, LC_GC_BLOCK);
                free(b->marks);
                free(b->allocated);
                free(b);
                b = next;
            }
            lc_gc_classes[a][c] = 0;
            lc_gc_own.free_list[a][c] = 0;
        }
    }
    for (int k = 0; k < lc_gc_own.spare_n; k++) munmap((void *)lc_gc_own.spare[k], LC_GC_BLOCK);
    lc_gc_own.spare_n = 0;
    if (lc_gc_own.chunk_next < lc_gc_own.chunk_end)
        munmap((void *)lc_gc_own.chunk_next, lc_gc_own.chunk_end - lc_gc_own.chunk_next);
    lc_gc_own.chunk_next = lc_gc_own.chunk_end = 0;
    for (size_t i = 0; i < lc_gc_large_n; i++) free(lc_gc_large[i]);
    free(lc_gc_large);
    lc_gc_large = 0;
    lc_gc_large_n = lc_gc_large_cap = 0;
    for (size_t top = 0; top < ((size_t)1 << 16); top++) {
        free(lc_gc_own.map[top]);
        lc_gc_own.map[top] = 0;
    }
}
#else
/* ---- The checked heap ---- */
/* An object's block: its header (the object's size, its place in the table, whether it is atomic), then the
   object, then one byte. */
typedef struct { size_t size; uint32_t index; uint32_t atomic; } lc_gc_hdr;
static lc_gc_hdr **lc_gc_blocks; static size_t lc_gc_count, lc_gc_cap;
static unsigned char *lc_gc_marks; static size_t lc_gc_marks_cap;
static lc_gc_hdr **lc_gc_sorting; static size_t lc_gc_sorting_cap;
/* The collector's own state that points into the heap or sorts it: the heap's bounds and the sort's counts. The
   globals scan skips it. */
static struct { uintptr_t heap_lo, heap_hi; size_t counts[1 << 16]; } lc_gc_own;

static void lc_gc_heap_init(void) {}

static inline size_t lc_gc_takes(size_t n) { return sizeof(lc_gc_hdr) + n; }

static void *lc_gc_heap_alloc(size_t n, int atomic) {
    if (lc_gc_count == (size_t)UINT32_MAX) lc_gc_exhausted();
    lc_gc_hdr *h = (lc_gc_hdr *)malloc(sizeof *h + n + 1);
    if (!h) lc_gc_exhausted();
    if (!atomic) memset(h + 1, 0, n);
    h->size = n;
    h->atomic = (uint32_t)atomic;
    lc_gc_live += sizeof *h + n;
    lc_gc_allocated += sizeof *h + n;
    if (lc_gc_count == lc_gc_cap) {
        lc_gc_cap = lc_gc_cap ? lc_gc_cap * 2 : 1024;
        lc_gc_blocks = (lc_gc_hdr **)lc_gc_table(lc_gc_blocks, lc_gc_cap * sizeof *lc_gc_blocks);
    }
    h->index = (uint32_t)lc_gc_count;
    lc_gc_blocks[lc_gc_count++] = h;
    return h + 1;
}

static size_t lc_gc_capacity(void *p) { return ((lc_gc_hdr *)p - 1)->size; }

static void lc_gc_heap_free(void *p) {
    lc_gc_hdr *h = (lc_gc_hdr *)p - 1;
    size_t i = h->index;
    lc_gc_live -= sizeof *h + h->size;
    lc_gc_freed += sizeof *h + h->size;
    lc_gc_blocks[i] = lc_gc_blocks[--lc_gc_count];
    lc_gc_blocks[i]->index = (uint32_t)i;
    free(h);
}

/* The table in address order: a radix sort over the addresses' low 48 bits, sixteen at a time. */
static void lc_gc_prepare(void) {
    if (lc_gc_sorting_cap < lc_gc_count) {
        lc_gc_sorting_cap = lc_gc_cap;
        lc_gc_sorting = (lc_gc_hdr **)lc_gc_table(lc_gc_sorting, lc_gc_sorting_cap * sizeof *lc_gc_sorting);
    }
    size_t *counts = lc_gc_own.counts;
    lc_gc_hdr **from = lc_gc_blocks, **to = lc_gc_sorting;
    for (int shift = 0; shift < 48; shift += 16) {
        memset(counts, 0, sizeof lc_gc_own.counts);
        for (size_t i = 0; i < lc_gc_count; i++) counts[((uintptr_t)from[i] >> shift) & 0xFFFF]++;
        size_t at = 0;
        for (size_t d = 0; d < (1 << 16); d++) { size_t c = counts[d]; counts[d] = at; at += c; }
        for (size_t i = 0; i < lc_gc_count; i++) to[counts[((uintptr_t)from[i] >> shift) & 0xFFFF]++] = from[i];
        lc_gc_hdr **t = from; from = to; to = t;
    }
    if (from != lc_gc_blocks) memcpy(lc_gc_blocks, from, lc_gc_count * sizeof *lc_gc_blocks);
    for (size_t i = 0; i < lc_gc_count; i++) lc_gc_blocks[i]->index = (uint32_t)i;
    lc_gc_own.heap_lo = lc_gc_count ? (uintptr_t)lc_gc_blocks[0] : 0;
    lc_gc_own.heap_hi = lc_gc_count ? (uintptr_t)(lc_gc_blocks[lc_gc_count - 1] + 1) + lc_gc_blocks[lc_gc_count - 1]->size : 0;
    if (lc_gc_marks_cap < lc_gc_count) {
        lc_gc_marks_cap = lc_gc_count * 2;
        lc_gc_marks = (unsigned char *)lc_gc_table(lc_gc_marks, lc_gc_marks_cap);
    }
    if (lc_gc_count) memset(lc_gc_marks, 0, lc_gc_count);
}

static inline void lc_gc_consider(uintptr_t w) {
    w &= ~(uintptr_t)1;
    if (!lc_gc_count || w < lc_gc_own.heap_lo || w > lc_gc_own.heap_hi) return;
    size_t lo = 0, hi = lc_gc_count;
    while (hi - lo > 1) {
        size_t mid = lo + (hi - lo) / 2;
        if ((uintptr_t)lc_gc_blocks[mid] <= w) lo = mid; else hi = mid;
    }
    lc_gc_hdr *h = lc_gc_blocks[lo];
    if (w < (uintptr_t)h || w > (uintptr_t)(h + 1) + h->size || lc_gc_marks[lo]) return;
    lc_gc_marks[lo] = 1;
    if (!h->atomic) lc_gc_push((uintptr_t)(h + 1), (uintptr_t)(h + 1) + h->size);
}

static int lc_gc_reached(void *p) { return lc_gc_marks[((lc_gc_hdr *)p - 1)->index]; }

static size_t lc_gc_sweep(size_t *freed) {
    size_t kept = 0, live = 0;
    for (size_t i = 0; i < lc_gc_count; i++) {
        lc_gc_hdr *h = lc_gc_blocks[i];
        if (lc_gc_marks[i]) {
            h->index = (uint32_t)kept;
            lc_gc_blocks[kept++] = h;
            live += h->size + sizeof *h;
        } else {
            *freed += h->size + sizeof *h;
            free(h);
        }
    }
    lc_gc_count = kept;
    return live;
}

static void lc_gc_heap_teardown(void) {
    for (size_t i = 0; i < lc_gc_count; i++) free(lc_gc_blocks[i]);
    free(lc_gc_blocks);
    free(lc_gc_marks);
    free(lc_gc_sorting);
    lc_gc_blocks = 0; lc_gc_marks = 0; lc_gc_sorting = 0;
    lc_gc_count = lc_gc_cap = lc_gc_marks_cap = lc_gc_sorting_cap = 0;
}
#endif

static void lc_gc_teardown(void);

/* The base of the stack the program starts on. */
static uintptr_t lc_gc_main_stack_base(void) {
#if defined(__APPLE__)
    return (uintptr_t)pthread_get_stackaddr_np(pthread_self());
#elif defined(__GLIBC__)
    extern void *__libc_stack_end;
    return (uintptr_t)__libc_stack_end;
#endif
}

/* The collector starts at the program's first allocation, its marking stack reserved then: a collection, which runs
   when memory is short, allocates nothing until marking outgrows it. */
static void lc_gc_start(void) {
    lc_gc_started = 1;
    const char *s = getenv("LOGOS_GC_STRESS");
    lc_gc_stress = s && s[0] == '1' && s[1] == 0;
    lc_gc_stack_hi = lc_gc_main_stack_base();
    lc_gc_heap_init();
    lc_gc_pending_cap = 1024;
    lc_gc_pending = (lc_gc_range *)lc_gc_table(lc_gc_pending, lc_gc_pending_cap * sizeof *lc_gc_pending);
    atexit(lc_gc_teardown);
}

/* Every aligned word of [lo, hi). It reads the stack's and the globals' padding too, which address sanitizing
   would report, so it is not instrumented. */
__attribute__((no_sanitize_address, noinline)) static void lc_gc_scan(uintptr_t lo, uintptr_t hi) {
    for (uintptr_t a = (lo + 7) & ~(uintptr_t)7; a + 8 <= hi; a += 8) lc_gc_consider(*(const uintptr_t *)a);
}

/* The program's data and bss segments: its globals and this runtime's statics. */
static void lc_gc_scan_globals(void) {
#if defined(__APPLE__)
    const struct mach_header_64 *mh = (const struct mach_header_64 *)_dyld_get_image_header(0);
    const char *segments[] = {"__DATA", "__DATA_DIRTY"};
    for (int k = 0; k < 2; k++) {
        unsigned long size = 0;
        uint8_t *at = getsegmentdata(mh, segments[k], &size);
        if (!at) continue;
        uintptr_t lo = (uintptr_t)at, hi = lo + size, own = (uintptr_t)&lc_gc_own, own_end = (uintptr_t)(&lc_gc_own + 1);
        if (own >= lo && own_end <= hi) {
            lc_gc_scan(lo, own);
            lc_gc_scan(own_end, hi);
        } else {
            lc_gc_scan(lo, hi);
        }
    }
#else
    extern char __data_start[], _end[];
    lc_gc_scan((uintptr_t)__data_start, (uintptr_t)&lc_gc_own);
    lc_gc_scan((uintptr_t)(&lc_gc_own + 1), (uintptr_t)_end);
#endif
}

/* The running stack, from below this frame to its base: the caller spills its registers into it first. */
__attribute__((noinline)) static void lc_gc_scan_stack(void) {
    volatile uintptr_t top = 0;
    lc_gc_scan((uintptr_t)&top, lc_gc_stack_hi);
}

static void lc_gc_collect(void) {
    if (lc_gc_collecting) return;
    lc_gc_collecting = 1;
    lc_gc_prepare();
    lc_gc_pending_n = 0;
    /* The registers, spilled into this frame, which the stack scan below covers. */
    jmp_buf regs;
    setjmp(regs);
    lc_gc_scan_stack();
    for (int k = 0; k < lc_gc_left_n; k++) lc_gc_scan(lc_gc_left[k].lo, lc_gc_left[k].hi);
    for (size_t k = 0; k < lc_gc_roots_n; k++) lc_gc_scan(lc_gc_roots[k].lo, lc_gc_roots[k].hi);
    lc_gc_scan_globals();
    while (lc_gc_pending_n) {
        lc_gc_range r = lc_gc_pending[--lc_gc_pending_n];
        lc_gc_scan(r.lo, r.hi);
    }
    if (lc_gc_holds_n) {
        size_t kept = 0;
        for (size_t k = 0; k < lc_gc_holds_n; k++) {
            lc_gc_hold h = lc_gc_holds[k];
            if (lc_gc_reached(h.object)) lc_gc_holds[kept++] = h;
            else h.release(h.object);
        }
        lc_gc_holds_n = kept;
    }
    size_t before = lc_gc_live, unused = 0, live = lc_gc_sweep(&unused);
    size_t freed = before > live ? before - live : 0;
    lc_gc_freed += freed;
    lc_gc_live = live;
    lc_gc_after = live;
    lc_gc_collections++;
    lc_gc_since = 0;
    /* The next collection waits for twice the bytes held (the heap's live bytes and those its objects hold outside
       it), four times when this one found little garbage. */
    size_t held = live + (lc_gc_outside ? lc_gc_outside() : 0);
    size_t next = freed * 4 < live + freed ? held * 4 : held * 2;
    lc_gc_trigger = next > ((size_t)8 << 20) ? next : ((size_t)8 << 20);
    lc_gc_collecting = 0;
}

static void lc_gc_teardown(void) {
    const char *report = getenv("LOGOS_HEAP_REPORT");
    if (report && report[0]) {
        fprintf(stderr, "logos heap: live=%zu high=%zu collections=%zu allocated=%zu freed=%zu",
                lc_gc_collections ? lc_gc_after : lc_gc_live, lc_gc_high, lc_gc_collections, lc_gc_allocated, lc_gc_freed);
        if (lc_gc_outside_report) lc_gc_outside_report(stderr);
        fputc('\n', stderr);
    }
    for (size_t k = 0; k < lc_gc_holds_n; k++) lc_gc_holds[k].release(lc_gc_holds[k].object);
    lc_gc_heap_teardown();
    free(lc_gc_pending);
    free(lc_gc_roots);
    free(lc_gc_holds);
    lc_gc_pending = 0; lc_gc_roots = 0; lc_gc_holds = 0;
    lc_gc_pending_n = lc_gc_pending_cap = lc_gc_roots_n = lc_gc_roots_cap = lc_gc_holds_n = lc_gc_holds_cap = 0;
}

/* A report asked for is printed whether or not the program allocated: one that never did started no collector, so
   no teardown prints it. Registered before main, this runs after every handler registered later, the teardown of a
   collector that started among them. */
static void lc_gc_report_unstarted(void) {
    const char *report = getenv("LOGOS_HEAP_REPORT");
    if (!lc_gc_started && report && report[0])
        fprintf(stderr, "logos heap: live=0 high=0 collections=0 allocated=0 freed=0\n");
}
__attribute__((constructor)) static void lc_gc_arm_report(void) { atexit(lc_gc_report_unstarted); }

/* Before allocating `n` bytes: collect when due. */
static inline void lc_gc_due(size_t n) {
    if (__builtin_expect(!lc_gc_started, 0)) lc_gc_start();
    lc_gc_since += n;
    if (__builtin_expect(lc_gc_stress || lc_gc_since >= lc_gc_trigger, 0)) lc_gc_collect();
}

/* Both heap layouts reserve a header and a trailing byte. Check before
   size-class arithmetic, ledger updates, collection, or libc allocation. */
static inline void lc_gc_check_size(size_t n) {
    if (n > SIZE_MAX - sizeof(lc_gc_hdr) - LC_GC_PAD - 1) lc_gc_exhausted();
}

static inline void *lc_gc_block(size_t n, int atomic) {
    lc_gc_check_size(n);
    if (__builtin_expect(!lc_gc_started, 0)) lc_gc_start();
    size_t takes = lc_gc_takes(n);
    if (lc_gc_live + takes > lc_gc_high) lc_gc_high = lc_gc_live + takes;
    lc_gc_due(takes);
    return lc_gc_heap_alloc(n, atomic);
}

__attribute__((unused, malloc)) static void *lc_gc_alloc(size_t n) { return lc_gc_block(n, 0); }
__attribute__((unused, malloc)) static void *lc_gc_alloc_atomic(size_t n) { return lc_gc_block(n, 1); }

__attribute__((unused, malloc)) static void *lc_gc_calloc(size_t count, size_t n) {
    if (n && count > (size_t)-1 / n) lc_gc_exhausted();
    void *p = lc_gc_alloc(count * n);
    memset(p, 0, count * n);
    return p;
}

__attribute__((unused, malloc)) static void *lc_gc_calloc_atomic(size_t count, size_t n) {
    if (n && count > (size_t)-1 / n) lc_gc_exhausted();
    void *p = lc_gc_alloc_atomic(count * n);
    memset(p, 0, count * n);
    return p;
}

__attribute__((unused)) static void lc_gc_free(void *p) {
    if (p) lc_gc_heap_free(p);
}

/* A resize keeps a small object in its slot when it fits and lets the C library resize a large one in place where
   it can; otherwise the object moves into a new one of the same kind. It counts only the bytes it adds. */
static void *lc_gc_resize(void *p, size_t n, int atomic) {
    lc_gc_check_size(n);
    if (!p) return lc_gc_block(n, atomic);
    size_t have = lc_gc_capacity(p);
#ifndef LC_GC_CHECKED_HEAP
    if (lc_gc_is_small(p)) {
        if (n <= have) return p;
    } else if (n + LC_GC_PAD + 1 > LC_GC_SMALL) {
        lc_gc_due(n > have ? n - have : 0);
        return lc_gc_large_resize(p, n);
    }
#endif
    void *q = lc_gc_block(n, atomic);
    memcpy(q, p, have < n ? have : n);
    lc_gc_heap_free(p);
    return q;
}

__attribute__((unused)) static void *lc_gc_realloc(void *p, size_t n) { return lc_gc_resize(p, n, 0); }
__attribute__((unused)) static void *lc_gc_realloc_atomic(void *p, size_t n) { return lc_gc_resize(p, n, 1); }

__attribute__((unused)) static void lc_gc_add_root(const void *lo, const void *hi) {
    if (lc_gc_roots_n == lc_gc_roots_cap) {
        lc_gc_roots_cap = lc_gc_roots_cap ? lc_gc_roots_cap * 2 : 16;
        lc_gc_roots = (lc_gc_range *)lc_gc_table(lc_gc_roots, lc_gc_roots_cap * sizeof *lc_gc_roots);
    }
    lc_gc_roots[lc_gc_roots_n++] = (lc_gc_range){(uintptr_t)lo, (uintptr_t)hi};
}

__attribute__((unused)) static void lc_gc_remove_root(const void *lo) {
    for (size_t k = 0; k < lc_gc_roots_n; k++) {
        if (lc_gc_roots[k].lo == (uintptr_t)lo) {
            lc_gc_roots[k] = lc_gc_roots[--lc_gc_roots_n];
            return;
        }
    }
}

__attribute__((unused)) static void lc_gc_on_release(void *object, void (*release)(void *)) {
    if (lc_gc_holds_n == lc_gc_holds_cap) {
        lc_gc_holds_cap = lc_gc_holds_cap ? lc_gc_holds_cap * 2 : 256;
        lc_gc_holds = (lc_gc_hold *)lc_gc_table(lc_gc_holds, lc_gc_holds_cap * sizeof *lc_gc_holds);
    }
    lc_gc_holds[lc_gc_holds_n++] = (lc_gc_hold){object, release};
}

__attribute__((unused)) static void lc_gc_count_outside(size_t (*bytes)(void), void (*report)(FILE *)) {
    lc_gc_outside = bytes;
    lc_gc_outside_report = report;
}

__attribute__((unused)) static void lc_gc_note_outside(size_t grown) { lc_gc_due(grown); }

/* A scheduler moving onto the stack [lo, hi): the stack it leaves stays a root from its current top to its base. */
__attribute__((unused, noinline)) static void lc_gc_stack_enter(const void *lo, const void *hi) {
    if (!lc_gc_started) lc_gc_start();
    volatile uintptr_t top = 0;
    if (lc_gc_left_n == 64) lc_gc_exhausted();
    lc_gc_left[lc_gc_left_n++] = (lc_gc_range){(uintptr_t)&top, lc_gc_stack_hi};
    (void)lo;
    lc_gc_stack_hi = (uintptr_t)hi;
}

__attribute__((unused)) static void lc_gc_stack_leave(void) {
    lc_gc_range r = lc_gc_left[--lc_gc_left_n];
    lc_gc_stack_hi = r.hi;
}

/* Worker steps are serialized by the scheduler mutex. The scheduler's original
   stack stays registered until every worker joins; only the running worker's
   stack is scanned as the current stack. */
__attribute__((unused)) static void lc_gc_scheduler_stack_enter(const void *top) {
    if (!lc_gc_started) lc_gc_start();
    lc_gc_add_root(top, (const void *)lc_gc_stack_hi);
}
__attribute__((unused)) static uintptr_t lc_gc_worker_stack_enter(const void *top) {
    uintptr_t saved = lc_gc_stack_hi;
    lc_gc_stack_hi = (uintptr_t)top + sizeof(uintptr_t);
    return saved;
}
__attribute__((unused)) static void lc_gc_worker_stack_leave(uintptr_t saved) {
    lc_gc_stack_hi = saved;
}

#endif
/* LC_GC_COLLECTOR_END */
"#
    };
}
pub(crate) use lc_gc_collector_source;

/// The collector ([`C_RUNTIME`]'s memory section): emitted ahead of the runtime by a generator whose every allocation
/// goes through `lc_gc_*`, which defines `LC_GC_COLLECTS` on a platform whose segments it can find (glibc, Apple). A
/// generator that leaves it out, and every device target, keeps the C library's calls under the same names.
pub(crate) const C_GC_COLLECTOR: &str = lc_gc_collector_source!();

/// The C library's allocator under the collector's names, for a runtime built where the collector is not
/// (`LC_GC_COLLECTS` undefined: another generator, a device target, a platform whose segments it cannot find). The C
/// runtime emits it ahead of [`lc_gc_api_source`]; a runtime that always carries the collector (the RISC-V shim)
/// leaves it out, so nothing in it can reach the C library's allocator.
macro_rules! lc_gc_libc_source {
    () => {
        r#"/* LC_GC_BEGIN — the C library's allocator under the collector's names, where the collector is not built. */
#ifndef LC_GC_COLLECTS
__attribute__((unused)) static inline void *lc_gc_alloc(size_t n) { return malloc(n); }
__attribute__((unused)) static inline void *lc_gc_alloc_atomic(size_t n) { return malloc(n); }
__attribute__((unused)) static inline void *lc_gc_calloc(size_t count, size_t n) { return calloc(count, n); }
__attribute__((unused)) static inline void *lc_gc_calloc_atomic(size_t count, size_t n) { return calloc(count, n); }
__attribute__((unused)) static inline void *lc_gc_realloc(void *p, size_t n) { return realloc(p, n); }
__attribute__((unused)) static inline void *lc_gc_realloc_atomic(void *p, size_t n) { return realloc(p, n); }
__attribute__((unused)) static inline void lc_gc_free(void *p) { free(p); }
__attribute__((unused)) static inline void lc_gc_add_root(const void *lo, const void *hi) { (void)lo; (void)hi; }
__attribute__((unused)) static inline void lc_gc_remove_root(const void *lo) { (void)lo; }
__attribute__((unused)) static inline void lc_gc_on_release(void *p, void (*release)(void *)) { (void)p; (void)release; }
__attribute__((unused)) static inline void lc_gc_count_outside(size_t (*bytes)(void), void (*report)(FILE *)) { (void)bytes; (void)report; }
__attribute__((unused)) static inline void lc_gc_note_outside(size_t grown) { (void)grown; }
__attribute__((unused)) static inline void lc_gc_stack_enter(const void *lo, const void *hi) { (void)lo; (void)hi; }
__attribute__((unused)) static inline void lc_gc_stack_leave(void) {}
__attribute__((unused)) static inline void lc_gc_scheduler_stack_enter(const void *top) { (void)top; }
__attribute__((unused)) static inline uintptr_t lc_gc_worker_stack_enter(const void *top) { (void)top; return 0; }
__attribute__((unused)) static inline void lc_gc_worker_stack_leave(uintptr_t saved) { (void)saved; }
#endif
/* LC_GC_END */
"#
    };
}
pub(crate) use lc_gc_libc_source;

/// The allocator interface every runtime calls (its comment, `LC_GC_BEGIN`, is the contract): the collector's when
/// [`C_GC_COLLECTOR`] is emitted ahead of it, the C library's ([`lc_gc_libc_source`]) otherwise.
macro_rules! lc_gc_api_source {
    () => {
        r#"/* LC_GC_BEGIN — the one allocator. Every block the program and this runtime allocate comes from it, and nothing else
   calls the C library's allocator:
     void *lc_gc_alloc(size_t n);                     malloc's contract
     void *lc_gc_calloc(size_t count, size_t n);      zeroed
     void *lc_gc_realloc(void *p, size_t n);          p == NULL allocates
     void *lc_gc_alloc_atomic(size_t n);              a block that will never hold a pointer (text, bytes,
     void *lc_gc_calloc_atomic(size_t count, size_t n);  digits, floats, flags, indices): never scanned,
     void *lc_gc_realloc_atomic(void *p, size_t n);      and atomic across every resize
     void  lc_gc_free(void *p);                       releases a block known dead now; NULL does nothing
     char *lc_gc_strdup(const char *s);
     char *lc_gc_strndup(const char *s, size_t n);
     void  lc_gc_add_root(const void *lo, const void *hi);   [lo, hi) is scanned while registered
     void  lc_gc_remove_root(const void *lo);
     void  lc_gc_on_release(void *p, void (*release)(void *));   p holds memory outside the heap: release(p)
                                                     gives it back once p is unreached, or at exit
     void  lc_gc_count_outside(size_t (*bytes)(void), void (*report)(FILE *));   that memory's size, and what
                                                     the heap report adds for it
     void  lc_gc_note_outside(size_t grown);         that memory grew by `grown` bytes
     void  lc_gc_stack_enter(const void *lo, const void *hi); a scheduler moves onto the stack [lo, hi)
     void  lc_gc_stack_leave(void);                           and back onto the stack it left

   A program built with LC_GC_COLLECTS also reclaims every block it can no longer reach, so its memory is bounded by
   what it holds, and frees the rest at exit. The collector marks from its roots and sweeps what it did not reach:
   - the roots: the running stack from its current top to its base (the registers spilled into it first), every
     stack a scheduler has left (lc_gc_stack_enter / lc_gc_stack_leave), every range registered with
     lc_gc_add_root, and the program's whole data and bss segments (its globals and this runtime's statics);
   - every reached block is scanned word by word, so a block is held by any word that points into it: at its
     start, inside it, one past its end, or into its header just before it, and with the low bit set (an exact
     integer's tagged word `p | 1` points at its box);
   - it runs once the bytes allocated since the last collection, and those its objects came to hold outside it,
     pass the larger of 8 MiB and the bytes then held, and before every allocation when LOGOS_GC_STRESS=1 is in the
     environment at the first allocation;
   - at exit, after every handler registered later, it gives back what every object still holds outside the heap,
     then frees every block, then its own tables.
   The collector is C_GC_COLLECTOR, emitted ahead of this runtime; without it (LC_GC_COLLECTS undefined: another
   generator, a device target, a platform whose segments it cannot find) the calls are the C library's, defined just
   before this contract. */
__attribute__((unused)) static char *lc_gc_strdup(const char *s) {
    size_t n = strlen(s);
    char *t = (char *)lc_gc_alloc_atomic(n + 1);
    memcpy(t, s, n + 1);
    return t;
}
__attribute__((unused)) static char *lc_gc_strndup(const char *s, size_t n) {
    size_t k = 0;
    while (k < n && s[k]) k++;
    char *t = (char *)lc_gc_alloc_atomic(k + 1);
    memcpy(t, s, k);
    t[k] = 0;
    return t;
}
/* LC_GC_END */"#
    };
}
pub(crate) use lc_gc_api_source;

/// The exact numbers both C generators and the RISC-V shim compute with: the arbitrary-precision integer `LBig`, the
/// exact `LDec` (a ratio, or a decimal's scaled integer), the tagged integer `LInt`, and the VM's rules over them
/// (`ldec_quotient` and kin). Its failure sentences are globals each runtime defines from the VM's own words
/// ([`super::words::exact_number_sentences`]).
macro_rules! exact_numbers_source {
    () => {
        r#"
/* The sentences this section stops a program with, the VM's own, which the generator emits beside the program. */
extern const char logos_says_big_exponent[], logos_says_bitand_fraction[], logos_says_bitand_refused[], logos_says_bitor_fraction[], logos_says_bitor_refused[], logos_says_bitxor_fraction[], logos_says_bitxor_refused[], logos_says_division_by_zero[], logos_says_exact_floor_division_by_zero[], logos_says_exponent_too_large[], logos_says_modulo_by_zero[], logos_says_modulo_int_rational[], logos_says_modulo_rational_int[], logos_says_modulo_rational_rational[], logos_says_negative_exponent[], logos_says_power_int_big[], logos_says_power_int_rational[], logos_says_power_rational_big[], logos_says_power_rational_rational[], logos_says_rational_division_by_zero[], logos_says_rational_exponent_too_large[], logos_says_shl_fraction[], logos_says_shl_refused[], logos_says_shr_fraction[], logos_says_shr_refused[];

/* ========== Arbitrary-precision integer (LBig): sign-magnitude, little-endian base-1e9 limbs.
   The substrate for LOGOS `Int` beyond int64 on the C backend — byte-identical decimal I/O with the
   interpreter's BigInt. Schoolbook add/sub/mul + exponentiation-by-squaring; O(n^2) mul is fine
   (correctness over speed — the C backend picks its fast path elsewhere). ========== */

#define LBIG_BASE 1000000000u   /* 1e9 — nine decimal digits per limb */

typedef struct { int sign; uint32_t *limb; size_t len; } LBig; /* sign in {-1,0,1}; zero => len==0 */

static LBig lbig_make(size_t cap) {
    LBig b; b.sign = 0; b.len = 0;
    b.limb = (uint32_t *)lc_gc_calloc_atomic(cap ? cap : 1, sizeof(uint32_t));
    return b;
}
static void lbig_free(LBig *b) { if (b->limb) lc_gc_free(b->limb); b->limb = NULL; b->len = 0; b->sign = 0; }
static void lbig_trim(LBig *b) {
    while (b->len > 0 && b->limb[b->len - 1] == 0) b->len--;
    if (b->len == 0) b->sign = 0;
}
static LBig lbig_copy(const LBig *a) {
    LBig r = lbig_make(a->len ? a->len : 1);
    for (size_t i = 0; i < a->len; i++) r.limb[i] = a->limb[i];
    r.len = a->len; r.sign = a->sign; return r;
}
static LBig lbig_from_u64(uint64_t v) {
    LBig b = lbig_make(3);
    size_t i = 0;
    while (v > 0) { b.limb[i++] = (uint32_t)(v % LBIG_BASE); v /= LBIG_BASE; }
    b.len = i; b.sign = (i > 0) ? 1 : 0;
    return b;
}
static LBig lbig_from_i64(int64_t v) {
    int neg = v < 0;
    uint64_t u = neg ? (uint64_t)(-(v + 1)) + 1u : (uint64_t)v; /* safe for INT64_MIN */
    LBig b = lbig_from_u64(u);
    if (neg && b.len > 0) b.sign = -1;
    return b;
}
static int lbig_cmp_mag(const LBig *a, const LBig *b) {
    if (a->len != b->len) return a->len < b->len ? -1 : 1;
    for (size_t i = a->len; i-- > 0; )
        if (a->limb[i] != b->limb[i]) return a->limb[i] < b->limb[i] ? -1 : 1;
    return 0;
}
static LBig lbig_add_mag(const LBig *a, const LBig *b) {
    size_t n = (a->len > b->len ? a->len : b->len) + 1;
    LBig r = lbig_make(n);
    uint64_t carry = 0;
    for (size_t i = 0; i < n; i++) {
        uint64_t s = carry;
        if (i < a->len) s += a->limb[i];
        if (i < b->len) s += b->limb[i];
        r.limb[i] = (uint32_t)(s % LBIG_BASE);
        carry = s / LBIG_BASE;
    }
    r.len = n; r.sign = 1; lbig_trim(&r);
    return r;
}
static LBig lbig_sub_mag(const LBig *a, const LBig *b) { /* requires |a| >= |b| */
    LBig r = lbig_make(a->len);
    int64_t borrow = 0;
    for (size_t i = 0; i < a->len; i++) {
        int64_t s = (int64_t)a->limb[i] - borrow - (i < b->len ? (int64_t)b->limb[i] : 0);
        if (s < 0) { s += LBIG_BASE; borrow = 1; } else borrow = 0;
        r.limb[i] = (uint32_t)s;
    }
    r.len = a->len; r.sign = 1; lbig_trim(&r);
    return r;
}
static LBig lbig_add(const LBig *a, const LBig *b) {
    if (a->sign == 0) return lbig_copy(b);
    if (b->sign == 0) return lbig_copy(a);
    if (a->sign == b->sign) { LBig r = lbig_add_mag(a, b); if (r.len) r.sign = a->sign; return r; }
    int c = lbig_cmp_mag(a, b);
    if (c == 0) return lbig_make(1);
    if (c > 0) { LBig r = lbig_sub_mag(a, b); if (r.len) r.sign = a->sign; return r; }
    LBig r = lbig_sub_mag(b, a); if (r.len) r.sign = b->sign; return r;
}
static LBig lbig_neg(const LBig *a) { LBig r = lbig_copy(a); if (r.len) r.sign = -a->sign; return r; }
static LBig lbig_sub(const LBig *a, const LBig *b) { LBig nb = lbig_neg(b); LBig r = lbig_add(a, &nb); lbig_free(&nb); return r; }
static LBig lbig_mul(const LBig *a, const LBig *b) {
    if (a->sign == 0 || b->sign == 0) return lbig_make(1);
    LBig r = lbig_make(a->len + b->len);
    for (size_t i = 0; i < a->len; i++) {
        uint64_t carry = 0;
        for (size_t j = 0; j < b->len; j++) {
            uint64_t cur = (uint64_t)r.limb[i + j] + (uint64_t)a->limb[i] * b->limb[j] + carry;
            r.limb[i + j] = (uint32_t)(cur % LBIG_BASE);
            carry = cur / LBIG_BASE;
        }
        r.limb[i + b->len] += (uint32_t)carry;
    }
    r.len = a->len + b->len; r.sign = a->sign * b->sign; lbig_trim(&r);
    return r;
}
static LBig lbig_pow(const LBig *base, uint64_t exp) {
    LBig result = lbig_from_i64(1);
    LBig b = lbig_copy(base);
    while (exp > 0) {
        if (exp & 1) { LBig t = lbig_mul(&result, &b); lbig_free(&result); result = t; }
        exp >>= 1;
        if (exp) { LBig t = lbig_mul(&b, &b); lbig_free(&b); b = t; }
    }
    lbig_free(&b);
    return result;
}
static int lbig_cmp(const LBig *a, const LBig *b) {
    if (a->sign != b->sign) return a->sign < b->sign ? -1 : 1;
    int m = lbig_cmp_mag(a, b);
    return a->sign < 0 ? -m : m; /* for negatives, larger magnitude = smaller value */
}
static char *lbig_to_string(const LBig *a) {
    if (a->len == 0) { char *z = (char *)lc_gc_alloc_atomic(2); z[0] = '0'; z[1] = '\0'; return z; }
    char *buf = (char *)lc_gc_alloc_atomic(a->len * 9 + 2);
    char *p = buf;
    if (a->sign < 0) *p++ = '-';
    p += sprintf(p, "%u", a->limb[a->len - 1]);
    for (size_t i = a->len - 1; i-- > 0; ) p += sprintf(p, "%09u", a->limb[i]);
    *p = '\0';
    return buf;
}
static void show_bignum(LBig a) { char *s = lbig_to_string(&a); printf("%s\n", s); lc_gc_free(s); }

/* ================= Exact rational + complex tower over LBig ==================
   The AOT-Rust backend keeps exact LogosRational / LogosComplex; the C backend now
   mirrors them so `decimal`/`complex` are byte-identical on every tier. Built on LBig:
   binary GCD (no long division needed) + a per-limb binary-search divmod for reduction. */
__attribute__((unused)) static int lbig_is_even(const LBig *a) { return a->len == 0 ? 1 : (a->limb[0] % 2 == 0); }
__attribute__((unused)) static LBig lbig_halve(const LBig *a) { /* |a|/2, sign kept */
    LBig r = lbig_make(a->len ? a->len : 1); uint64_t rem = 0;
    for (size_t i = a->len; i-- > 0; ) { uint64_t cur = rem * LBIG_BASE + a->limb[i]; r.limb[i] = (uint32_t)(cur / 2); rem = cur % 2; }
    r.len = a->len; r.sign = a->sign; lbig_trim(&r); return r;
}
__attribute__((unused)) static LBig lbig_double(const LBig *a) { /* |a|*2, sign kept */
    LBig r = lbig_make(a->len + 1); uint64_t carry = 0;
    for (size_t i = 0; i < a->len; i++) { uint64_t cur = (uint64_t)a->limb[i] * 2 + carry; r.limb[i] = (uint32_t)(cur % LBIG_BASE); carry = cur / LBIG_BASE; }
    r.limb[a->len] = (uint32_t)carry; r.len = a->len + 1; r.sign = a->sign; lbig_trim(&r); return r;
}
/* gcd of magnitudes (result sign 0/1), binary/Stein — shifts+subtraction, no division. */
__attribute__((unused)) static LBig lbig_gcd(const LBig *x, const LBig *y) {
    LBig a = lbig_copy(x); if (a.len) a.sign = 1; LBig b = lbig_copy(y); if (b.len) b.sign = 1;
    if (a.len == 0) { lbig_free(&a); return b; } if (b.len == 0) { lbig_free(&b); return a; }
    size_t shift = 0;
    while (lbig_is_even(&a) && lbig_is_even(&b)) { LBig t = lbig_halve(&a); lbig_free(&a); a = t; t = lbig_halve(&b); lbig_free(&b); b = t; shift++; }
    while (lbig_is_even(&a)) { LBig t = lbig_halve(&a); lbig_free(&a); a = t; }
    do {
        while (lbig_is_even(&b)) { LBig t = lbig_halve(&b); lbig_free(&b); b = t; }
        if (lbig_cmp_mag(&a, &b) > 0) { LBig t = a; a = b; b = t; }
        LBig nb = lbig_sub_mag(&b, &a); lbig_free(&b); b = nb;
    } while (b.len != 0);
    lbig_free(&b);
    for (size_t i = 0; i < shift; i++) { LBig t = lbig_double(&a); lbig_free(&a); a = t; }
    return a;
}
__attribute__((unused)) static LBig lbig_mul_small(const LBig *a, uint32_t m) {
    if (a->sign == 0 || m == 0) return lbig_make(1);
    LBig r = lbig_make(a->len + 2); uint64_t carry = 0; size_t i;
    for (i = 0; i < a->len; i++) { uint64_t cur = (uint64_t)a->limb[i] * m + carry; r.limb[i] = (uint32_t)(cur % LBIG_BASE); carry = cur / LBIG_BASE; }
    while (carry) { r.limb[i++] = (uint32_t)(carry % LBIG_BASE); carry /= LBIG_BASE; }
    r.len = i; r.sign = a->sign; lbig_trim(&r); return r;
}
/* magnitude long division A/D via per-limb binary search: q outputs floor(|A|/|D|), *rem gets |A|%|D|. */
__attribute__((unused)) static LBig lbig_divmod_mag(const LBig *A, const LBig *D, LBig *rem) {
    LBig q = lbig_make(A->len ? A->len : 1); q.len = A->len; q.sign = 1;
    LBig r = lbig_make(1); /* running remainder, magnitude */
    for (size_t i = A->len; i-- > 0; ) {
        LBig sh = lbig_mul_small(&r, (uint32_t)LBIG_BASE); lbig_free(&r); r = sh; /* r *= BASE */
        if (A->limb[i]) { LBig dig = lbig_from_u64(A->limb[i]); LBig s = lbig_add_mag(&r, &dig); lbig_free(&dig); lbig_free(&r); r = s; }
        uint32_t lo = 0, hi = (uint32_t)LBIG_BASE - 1, qd = 0;
        while (lo <= hi) { uint32_t mid = lo + (hi - lo) / 2; LBig t = lbig_mul_small(D, mid);
            int c = lbig_cmp_mag(&t, &r); lbig_free(&t);
            if (c <= 0) { qd = mid; if (mid == (uint32_t)LBIG_BASE - 1) break; lo = mid + 1; } else { if (mid == 0) break; hi = mid - 1; } }
        q.limb[i] = qd;
        if (qd) { LBig t = lbig_mul_small(D, qd); LBig nr = lbig_sub_mag(&r, &t); lbig_free(&t); lbig_free(&r); r = nr; }
    }
    lbig_trim(&q); if (q.len == 0) q.sign = 0;
    if (rem) { lbig_trim(&r); if (r.len == 0) r.sign = 0; *rem = r; } else lbig_free(&r);
    return q;
}
/* exact quotient |a|/|g| (g divides a); sign 0/1. */
__attribute__((unused)) static LBig lbig_divexact(const LBig *a, const LBig *g) { return lbig_divmod_mag(a, g, NULL); }
/* The double nearest to `a`, ties to even, infinite once the rounding reaches 2^1024 — the definition is
   `logicaffeine_base::BigInt::to_f64`, and `c_bigint_to_f64_lock` holds this twin to it. The magnitude is
   re-based to base 2^32 exactly; its top 64 significant bits are taken with every lower bit folded into
   bit 0 as a sticky bit (eleven places under the 53-bit rounding position), converted once, and scaled
   back by a power of two, which is exact until it overflows. */
__attribute__((unused)) static double lbig_to_f64(const LBig *a) {
    if (a->len == 0) return 0.0;
    uint32_t *w = (uint32_t *)lc_gc_calloc_atomic(a->len + 1, sizeof(uint32_t));
    size_t n = 0;
    for (size_t i = a->len; i-- > 0; ) {
        uint64_t carry = a->limb[i];
        for (size_t j = 0; j < n; j++) {
            uint64_t t = (uint64_t)w[j] * LBIG_BASE + carry;
            w[j] = (uint32_t)t;
            carry = t >> 32;
        }
        if (carry) w[n++] = (uint32_t)carry; /* < 10^9 + 1: one word */
    }
    uint64_t bits = (uint64_t)(n - 1) * 32 + (uint64_t)(32 - __builtin_clz(w[n - 1]));
    double r;
    if (bits <= 64) {
        r = (double)((uint64_t)w[0] | (n > 1 ? (uint64_t)w[1] << 32 : 0));
    } else {
        uint64_t dropped = bits - 64, top = 0;
        for (uint64_t k = 0; k < 64; k++) {
            uint64_t bit = dropped + k;
            top |= (uint64_t)((w[bit / 32] >> (bit % 32)) & 1u) << k;
        }
        int sticky = 0;
        for (uint64_t word = 0; word < dropped / 32 && !sticky; word++) sticky = w[word] != 0;
        if (!sticky && dropped % 32) sticky = (w[dropped / 32] & ((1u << (dropped % 32)) - 1u)) != 0;
        r = ldexp((double)(top | (uint64_t)sticky), (int)dropped);
    }
    lc_gc_free(w);
    return a->sign < 0 ? -r : r;
}

/* ===== Decimal (scale-aware) — the C mirror of the interpreter's Decimal|Rational union. A value is
   either a FIXED-POINT decimal (`a / 10^scale`, is_rat=0) or, once a non-terminating division occurs,
   a REDUCED FRACTION (`a / b`, b>0, is_rat=1) that stays a fraction through further arithmetic.
   `decimal("0.10")`→"0.10"; `decimal(1)/decimal(3)`→"1/3"; `1/3 + 1/6`→"1/2". By-value, built on LBig. */
typedef struct { int is_rat; LBig a; LBig b; int scale; } LDec;
__attribute__((unused)) static LBig lbig_pow10(int k) {
    LBig r = lbig_from_i64(1), ten = lbig_from_i64(10);
    for (int i = 0; i < k; i++) { LBig t = lbig_mul(&r, &ten); lbig_free(&r); r = t; }
    lbig_free(&ten); return r;
}
/* exact division `n / d` (d divides n); the quotient carries n's sign. */
__attribute__((unused)) static LBig lbig_exact_div(const LBig *n, const LBig *d) {
    LBig rem; LBig q = lbig_divmod_mag(n, d, &rem); lbig_free(&rem); q.sign = q.len ? n->sign : 0; return q;
}
__attribute__((unused)) static LDec ldec_from_i64(int64_t v) {
    LDec d; d.is_rat = 0; d.a = lbig_from_i64(v); d.b = lbig_from_i64(0); d.scale = 0; return d;
}
/* (num, den) view of a decimal — the caller owns the returned copies. */
__attribute__((unused)) static void ldec_as_rat(const LDec *d, LBig *num, LBig *den) {
    *num = lbig_copy(&d->a);
    *den = d->is_rat ? lbig_copy(&d->b) : lbig_pow10(d->scale);
}
/* reduce `num/den` to lowest terms with den>0; CONSUMES num & den → a rational-form LDec. */
__attribute__((unused)) static LDec ldec_rat(LBig num, LBig den) {
    if (den.len == 0) logos_fatal("decimal divide by zero");
    LBig g = lbig_gcd(&num, &den);
    LBig rn = lbig_exact_div(&num, &g), rd = lbig_exact_div(&den, &g);
    lbig_free(&num); lbig_free(&den); lbig_free(&g);
    if (rd.sign < 0) { rd.sign = 1; if (rn.len) rn.sign = -rn.sign; }
    LDec d; d.is_rat = 1; d.a = rn; d.b = rd; d.scale = 0; return d;
}
__attribute__((unused)) static LDec ldec_add(LDec x, LDec y) {
    if (!x.is_rat && !y.is_rat) {
        int s = x.scale > y.scale ? x.scale : y.scale;
        LBig px = lbig_pow10(s - x.scale), py = lbig_pow10(s - y.scale);
        LBig xa = lbig_mul(&x.a, &px), ya = lbig_mul(&y.a, &py), sum = lbig_add(&xa, &ya);
        lbig_free(&px); lbig_free(&py); lbig_free(&xa); lbig_free(&ya);
        LDec d; d.is_rat = 0; d.a = sum; d.b = lbig_from_i64(0); d.scale = s; return d;
    }
    LBig xn, xd, yn, yd; ldec_as_rat(&x, &xn, &xd); ldec_as_rat(&y, &yn, &yd);
    LBig t1 = lbig_mul(&xn, &yd), t2 = lbig_mul(&yn, &xd), num = lbig_add(&t1, &t2), den = lbig_mul(&xd, &yd);
    lbig_free(&xn); lbig_free(&xd); lbig_free(&yn); lbig_free(&yd); lbig_free(&t1); lbig_free(&t2);
    return ldec_rat(num, den);
}
__attribute__((unused)) static LDec ldec_sub(LDec x, LDec y) {
    if (!x.is_rat && !y.is_rat) {
        int s = x.scale > y.scale ? x.scale : y.scale;
        LBig px = lbig_pow10(s - x.scale), py = lbig_pow10(s - y.scale);
        LBig xa = lbig_mul(&x.a, &px), ya = lbig_mul(&y.a, &py), diff = lbig_sub(&xa, &ya);
        lbig_free(&px); lbig_free(&py); lbig_free(&xa); lbig_free(&ya);
        LDec d; d.is_rat = 0; d.a = diff; d.b = lbig_from_i64(0); d.scale = s; return d;
    }
    LBig xn, xd, yn, yd; ldec_as_rat(&x, &xn, &xd); ldec_as_rat(&y, &yn, &yd);
    LBig t1 = lbig_mul(&xn, &yd), t2 = lbig_mul(&yn, &xd), num = lbig_sub(&t1, &t2), den = lbig_mul(&xd, &yd);
    lbig_free(&xn); lbig_free(&xd); lbig_free(&yn); lbig_free(&yd); lbig_free(&t1); lbig_free(&t2);
    return ldec_rat(num, den);
}
__attribute__((unused)) static LDec ldec_mul(LDec x, LDec y) {
    if (!x.is_rat && !y.is_rat) {
        LBig prod = lbig_mul(&x.a, &y.a);
        LDec d; d.is_rat = 0; d.a = prod; d.b = lbig_from_i64(0); d.scale = x.scale + y.scale; return d;
    }
    LBig xn, xd, yn, yd; ldec_as_rat(&x, &xn, &xd); ldec_as_rat(&y, &yn, &yd);
    LBig num = lbig_mul(&xn, &yn), den = lbig_mul(&xd, &yd);
    lbig_free(&xn); lbig_free(&xd); lbig_free(&yn); lbig_free(&yd);
    return ldec_rat(num, den);
}
__attribute__((unused)) static LDec ldec_div(LDec x, LDec y) {
    LBig xn, xd, yn, yd; ldec_as_rat(&x, &xn, &xd); ldec_as_rat(&y, &yn, &yd);
    LBig num = lbig_mul(&xn, &yd), den = lbig_mul(&xd, &yn);
    lbig_free(&xn); lbig_free(&xd); lbig_free(&yn); lbig_free(&yd);
    return ldec_rat(num, den);
}
__attribute__((unused)) static char *ldec_to_string(const LDec *d) {
    if (d->is_rat) {
        char *n = lbig_to_string(&d->a);
        LBig one = lbig_from_i64(1);
        int den_is_one = (lbig_cmp_mag(&d->b, &one) == 0);
        lbig_free(&one);
        if (den_is_one) return n;
        char *den = lbig_to_string(&d->b);
        char *out = (char *) lc_gc_alloc_atomic(strlen(n) + strlen(den) + 2);
        sprintf(out, "%s/%s", n, den); lc_gc_free(n); lc_gc_free(den); return out;
    }
    int neg = (d->a.sign < 0);
    LBig abs = lbig_copy(&d->a); if (abs.len) abs.sign = 1;
    char *mag = lbig_to_string(&abs); lbig_free(&abs);
    int mlen = (int) strlen(mag);
    if (d->scale == 0) {
        char *out = (char *) lc_gc_alloc_atomic(mlen + 2); int p = 0;
        if (neg) out[p++] = '-'; strcpy(out + p, mag); lc_gc_free(mag); return out;
    }
    int total = mlen > d->scale ? mlen : d->scale + 1;   /* ensure >= 1 integer digit */
    int intlen = total - d->scale, pad = total - mlen;
    char *out = (char *) lc_gc_alloc_atomic(total + 3); int p = 0;
    if (neg) out[p++] = '-';
    for (int i = 0; i < intlen; i++) { int di = i - pad; out[p++] = (di < 0) ? '0' : mag[di]; }
    out[p++] = '.';
    for (int i = intlen; i < total; i++) { int di = i - pad; out[p++] = (di < 0) ? '0' : mag[di]; }
    out[p] = 0; lc_gc_free(mag); return out;
}
__attribute__((unused)) static void show_ldec(LDec d) { char *s = ldec_to_string(&d); printf("%s\n", s); lc_gc_free(s); }
/* Sign-correct three-way compare of two exact values (rational OR decimal). Cross-multiply over the
   (num, den>0) view: x <=> y  ⟺  xn·yd <=> yn·xd. Mirrors the interpreter's `Rational`/`Decimal` Ord. */
__attribute__((unused)) static int ldec_cmp(LDec x, LDec y) {
    LBig xn, xd, yn, yd; ldec_as_rat(&x, &xn, &xd); ldec_as_rat(&y, &yn, &yd);
    LBig l = lbig_mul(&xn, &yd), r = lbig_mul(&yn, &xd);
    int c = lbig_cmp(&l, &r);
    lbig_free(&xn); lbig_free(&xd); lbig_free(&yn); lbig_free(&yd); lbig_free(&l); lbig_free(&r);
    return c;
}
/* |value| — the sign of the numerator/mantissa cleared, the exact form (rational vs decimal) kept. */
__attribute__((unused)) static LDec ldec_abs(LDec x) {
    LDec r; r.is_rat = x.is_rat; r.a = lbig_copy(&x.a); if (r.a.len) r.a.sign = 1;
    r.b = lbig_copy(&x.b); r.scale = x.scale; return r;
}
/* floor: the greatest integer ≤ x, toward −∞ (a negative non-integer drops one below truncation). */
__attribute__((unused)) static LBig ldec_floor(LDec x) {
    LBig num, den; ldec_as_rat(&x, &num, &den);
    LBig rem; LBig q = lbig_divmod_mag(&num, &den, &rem);
    if (q.len) q.sign = num.sign;
    if (num.sign < 0 && rem.len) { LBig one = lbig_from_i64(1); LBig q2 = lbig_sub(&q, &one); lbig_free(&q); lbig_free(&one); q = q2; }
    lbig_free(&num); lbig_free(&den); lbig_free(&rem); return q;
}
/* ceil: the least integer ≥ x, toward +∞ (a positive non-integer adds one above truncation). */
__attribute__((unused)) static LBig ldec_ceil(LDec x) {
    LBig num, den; ldec_as_rat(&x, &num, &den);
    LBig rem; LBig q = lbig_divmod_mag(&num, &den, &rem);
    if (q.len) q.sign = num.sign;
    if (num.sign > 0 && rem.len) { LBig one = lbig_from_i64(1); LBig q2 = lbig_add(&q, &one); lbig_free(&q); lbig_free(&one); q = q2; }
    lbig_free(&num); lbig_free(&den); lbig_free(&rem); return q;
}
/* round: nearest integer, HALF AWAY FROM ZERO (`7/2 → 4`, `-7/2 → -4`) — the interpreter's `Rational::round`
   (HalfUp), which matches Rust's `f64::round`. Rounds up in magnitude when 2·|rem| ≥ den. */
__attribute__((unused)) static LBig ldec_round(LDec x) {
    LBig num, den; ldec_as_rat(&x, &num, &den);
    LBig rem; LBig q = lbig_divmod_mag(&num, &den, &rem);
    LBig two = lbig_from_i64(2), rem2 = lbig_mul(&rem, &two);
    if (lbig_cmp_mag(&rem2, &den) >= 0) { LBig one = lbig_from_i64(1); LBig q2 = lbig_add(&q, &one); lbig_free(&q); lbig_free(&one); q = q2; }
    if (q.len) q.sign = num.sign;
    lbig_free(&num); lbig_free(&den); lbig_free(&rem); lbig_free(&two); lbig_free(&rem2); return q;
}
/* rescale a decimal to exactly `target` fractional digits, banker's rounding (round-half-even). */
__attribute__((unused)) static LDec ldec_rescale(LDec d, int target) {
    LBig num, den; ldec_as_rat(&d, &num, &den);
    LBig p = lbig_pow10(target), n2 = lbig_mul(&num, &p);
    lbig_free(&num); lbig_free(&p);
    int neg = (n2.sign < 0); LBig nabs = n2; if (nabs.len) nabs.sign = 1;
    LBig r; LBig q = lbig_divmod_mag(&nabs, &den, &r);
    LBig two = lbig_from_i64(2), r2 = lbig_mul(&r, &two); lbig_free(&two);
    int cmp = lbig_cmp_mag(&r2, &den), roundup = 0;
    if (cmp > 0) roundup = 1;
    else if (cmp == 0) roundup = (q.len && (q.limb[0] & 1u));   /* tie → round to even */
    lbig_free(&r2); lbig_free(&r); lbig_free(&nabs); lbig_free(&den);
    if (roundup) { LBig one = lbig_from_i64(1), q2 = lbig_add(&q, &one); lbig_free(&one); lbig_free(&q); q = q2; }
    if (neg && q.len) q.sign = -1;
    LDec out; out.is_rat = 0; out.a = q; out.b = lbig_from_i64(0); out.scale = target; return out;
}

/* ===== Tagged big integer for LOGOS `Int` — int64 fast path, LBig on overflow =====
   The C mirror of the Rust backend's `LogosInt` (enum Small(i64)|Big). `big == NULL` means the value
   IS `small` (the common, register-cheap case); otherwise the value is `*big`. Arithmetic stays raw
   i64 while it provably fits (via `__builtin_*_overflow`) and promotes to LBig exactly on overflow, so
   `10000000000 * 10000000000` is decimal-exact where raw `int64_t` wraps. Codegen-facing, BY VALUE. */
typedef struct { int64_t small; LBig *big; } LInt;
__attribute__((unused)) static LInt lint_small(int64_t v) { LInt r; r.small = v; r.big = NULL; return r; }
__attribute__((unused)) static LInt lint_big(LBig b) { LInt r; r.small = 0; r.big = (LBig *)lc_gc_alloc(sizeof(LBig)); *r.big = b; return r; }
__attribute__((unused)) static LBig lint_as_big(LInt a) { return a.big ? lbig_copy(a.big) : lbig_from_i64(a.small); }
/* An exact integer as the `LDec` it is, for exact arithmetic with a fraction. */
__attribute__((unused)) static LDec ldec_from_lint(LInt v) { LDec d; d.is_rat = 0; d.a = lint_as_big(v); d.b = lbig_from_i64(0); d.scale = 0; return d; }
/* normalize an LBig result back to a small when it fits i64 (keeps the fast path sticky). */
__attribute__((unused)) static LInt lint_norm(LBig b) {
    if (b.len <= 2) { /* < 10^18 in magnitude → surely fits i64 */
        int64_t v = 0; for (size_t i = b.len; i-- > 0; ) v = v * (int64_t)LBIG_BASE + (int64_t)b.limb[i];
        if (b.sign < 0) v = -v; lbig_free(&b); return lint_small(v);
    }
    if (b.len == 3) { /* 10^18..10^27: may or may not fit — compare against i64 bounds via f64 guard then exact */
        double approx = lbig_to_f64(&b);
        if (approx >= -9.2e18 && approx <= 9.2e18) {
            /* borderline — rebuild and range-check exactly is overkill; fall through to big form when unsure */
        }
    }
    return lint_big(b);
}
__attribute__((unused)) static LInt lint_add(LInt a, LInt b) {
    if (!a.big && !b.big) { int64_t r; if (!__builtin_add_overflow(a.small, b.small, &r)) return lint_small(r); }
    LBig x = lint_as_big(a), y = lint_as_big(b), s = lbig_add(&x, &y); lbig_free(&x); lbig_free(&y); return lint_norm(s);
}
__attribute__((unused)) static LInt lint_sub(LInt a, LInt b) {
    if (!a.big && !b.big) { int64_t r; if (!__builtin_sub_overflow(a.small, b.small, &r)) return lint_small(r); }
    LBig x = lint_as_big(a), y = lint_as_big(b), s = lbig_sub(&x, &y); lbig_free(&x); lbig_free(&y); return lint_norm(s);
}
__attribute__((unused)) static LInt lint_mul(LInt a, LInt b) {
    if (!a.big && !b.big) { int64_t r; if (!__builtin_mul_overflow(a.small, b.small, &r)) return lint_small(r); }
    LBig x = lint_as_big(a), y = lint_as_big(b), s = lbig_mul(&x, &y); lbig_free(&x); lbig_free(&y); return lint_norm(s);
}
__attribute__((unused)) static int lint_cmp(LInt a, LInt b) {
    if (!a.big && !b.big) return a.small < b.small ? -1 : (a.small > b.small ? 1 : 0);
    LBig x = lint_as_big(a), y = lint_as_big(b); int c = lbig_cmp(&x, &y); lbig_free(&x); lbig_free(&y); return c;
}
/* Exact integer exponentiation — the C mirror of the interpreter's promote-on-overflow `**`/`pow`.
   Computes `base^exp` through the arbitrary-precision `LBig`, then `lint_norm`s the result so a value
   that still fits i64 stays a fast small (`2**10 -> 1024`) and one that overflows promotes
   (`2**100 -> the exact 31-digit BigInt`). A negative exponent has no exact integer value, so it
   aborts LOUDLY — byte-for-byte the tree-walker/VM, which reject it rather than truncate. */
__attribute__((unused)) static LInt lint_pow(LInt base, int64_t exp) {
    if (exp < 0) logos_fatal("LOGOS runtime error: a negative Int exponent has no exact integer value");
    LBig b = lint_as_big(base), p = lbig_pow(&b, (uint64_t)exp); lbig_free(&b); return lint_norm(p);
}
/* truncating division / C-remainder, sign-correct, with an LBig fallback beyond i64. */
__attribute__((unused)) static LInt lint_div(LInt a, LInt b) {
    if (!a.big && !b.big && !(a.small == INT64_MIN && b.small == -1)) return lint_small(a.small / b.small);
    LBig x = lint_as_big(a), y = lint_as_big(b); int sign = ((x.sign < 0) != (y.sign < 0)) ? -1 : 1;
    LBig q = lbig_divmod_mag(&x, &y, NULL); if (q.len) q.sign = sign; lbig_free(&x); lbig_free(&y); return lint_norm(q);
}
__attribute__((unused)) static LInt lint_mod(LInt a, LInt b) {
    if (!a.big && !b.big && !(a.small == INT64_MIN && b.small == -1)) return lint_small(a.small % b.small);
    LBig x = lint_as_big(a), y = lint_as_big(b), rem; LBig q = lbig_divmod_mag(&x, &y, &rem); lbig_free(&q);
    if (rem.len) rem.sign = x.sign; /* C remainder takes the dividend's sign */
    lbig_free(&x); lbig_free(&y); return lint_norm(rem);
}
__attribute__((unused)) static char *lint_to_string(LInt a) { if (!a.big) { char *s = (char *)lc_gc_alloc_atomic(24); sprintf(s, "%lld", (long long)a.small); return s; } return lbig_to_string(a.big); }
__attribute__((unused)) static void show_lint(LInt a) { char *s = lint_to_string(a); printf("%s\n", s); lc_gc_free(s); }
__attribute__((unused)) static double lint_to_f64(LInt a) { return a.big ? lbig_to_f64(a.big) : (double)a.small; }
/* An exact integer in one `long long` cell (a task's argument, a stepped call's argument or result): a value within 62
   bits as 2v + 1, any other as the address of its collector-held big integer, whose low bit is clear. A zeroed cell
   reads as 0. The cell holds the big integer for as long as the frame holding the cell is reached. */
__attribute__((unused)) static inline long long lint_to_cell(LInt a) {
    if (!a.big && a.small >= -((int64_t)1 << 62) && a.small < ((int64_t)1 << 62)) return (long long)(a.small * 2 + 1);
    LBig *big = a.big ? a.big : lint_big(lbig_from_i64(a.small)).big;
    return (long long)(intptr_t)big;
}
__attribute__((unused)) static inline LInt lint_of_cell(long long c) {
    if (c & 1) return lint_small((int64_t)((c - 1) / 2));
    LInt r; r.small = 0; r.big = (LBig *)(intptr_t)c; return r;
}
/* Word constructors reduce exact integers modulo 2^64. Unsigned multiply
   and subtraction are defined modulo 2^64 for every magnitude and sign. */
__attribute__((unused)) static inline uint64_t lint_word_bits(LInt a) {
    if (!a.big) return (uint64_t)a.small;
    uint64_t bits = 0;
    for (size_t i = a.big->len; i-- > 0; )
        bits = bits * (uint64_t)LBIG_BASE + (uint64_t)a.big->limb[i];
    return a.big->sign < 0 ? (uint64_t)0 - bits : bits;
}
/* Narrow to i64 at an int64-only boundary (a `Seq_i64` slot, an int64 param, an index). LOUD — a value
   genuinely beyond i64 aborts rather than silently truncating (matches the Rust `expect_i64`). */
__attribute__((unused, noinline)) static int64_t lint_expect_i64_big(LInt a);
/* An exact integer as a machine word: inline while it is one, out of line when it holds a big integer. */
__attribute__((unused)) static inline int64_t lint_expect_i64(LInt a) {
    if (__builtin_expect(!a.big, 1)) return a.small;
    return lint_expect_i64_big(a);
}
__attribute__((unused, noinline)) static int64_t lint_expect_i64_big(LInt a) {
    if (a.big->len <= 2) { int64_t v = 0; for (size_t i = a.big->len; i-- > 0; ) v = v * (int64_t)LBIG_BASE + (int64_t)a.big->limb[i]; return a.big->sign < 0 ? -v : v; }
    if (a.big->len == 3) { /* 10^18..10^27 — range-check exactly against i64 bounds */
        LBig lo = lbig_from_i64(INT64_MIN), hi = lbig_from_i64(INT64_MAX);
        int in = lbig_cmp(a.big, &lo) >= 0 && lbig_cmp(a.big, &hi) <= 0; lbig_free(&lo); lbig_free(&hi);
        if (in) { int64_t v = 0; for (size_t i = a.big->len; i-- > 0; ) v = v * (int64_t)LBIG_BASE + (int64_t)a.big->limb[i]; return a.big->sign < 0 ? -v : v; }
    }
    logos_fatal("LOGOS runtime error: integer value out of range for this int64 context");
}
/* An exact integer as a collection's index: the machine word it fits, or the reference tier's refusal of an index past
   one (`refusal`, the generator's copy of its sentence). */
__attribute__((unused, noinline)) static int64_t lint_index_i64_big(LInt a, const char *refusal);
__attribute__((unused)) static inline int64_t lint_index_i64(LInt a, const char *refusal) {
    if (__builtin_expect(!a.big, 1)) return a.small;
    return lint_index_i64_big(a, refusal);
}
__attribute__((unused, noinline)) static int64_t lint_index_i64_big(LInt a, const char *refusal) {
    LBig lo = lbig_from_i64(INT64_MIN), hi = lbig_from_i64(INT64_MAX);
    int in = lbig_cmp(a.big, &lo) >= 0 && lbig_cmp(a.big, &hi) <= 0; lbig_free(&lo); lbig_free(&hi);
    if (!in) logos_fatal(refusal);
    return lint_expect_i64(a);
}
/* promote any int64 value into an LInt for mixed BigInt arithmetic. */
__attribute__((unused)) static LInt lint_of(int64_t v) { return lint_small(v); }
__attribute__((unused)) static inline int lint_is_zero(LInt a) { return !a.big && a.small == 0; }
/* Whether the exact integer is within i64 — what the VM holds as an `Int` — and its value when it is. */
__attribute__((unused)) static int lint_i64(LInt a, int64_t *v) {
    if (!a.big) { *v = a.small; return 1; }
    LBig lo = lbig_from_i64(INT64_MIN), hi = lbig_from_i64(INT64_MAX);
    int in = lbig_cmp(a.big, &lo) >= 0 && lbig_cmp(a.big, &hi) <= 0; lbig_free(&lo); lbig_free(&hi);
    if (in) *v = lint_expect_i64(a);
    return in;
}
/* The bitwise operations over exact integers: within i64 they are the machine's; past it the VM refuses them —
   save a low-bit mask `2^k - 1` over a non-negative integer, which is its remainder by `2^k`. */
__attribute__((unused)) static int64_t lint_bitand(LInt a, LInt b) {
    int64_t x, y; int fx = lint_i64(a, &x), fy = lint_i64(b, &y);
    if (fx && fy) return x & y;
    if (fx != fy) {
        int64_t m = fx ? x : y; LInt big = fx ? b : a;
        if (m >= 0 && ((((uint64_t)m) + 1u) & (uint64_t)m) == 0 && lint_cmp(big, lint_small(0)) >= 0)
            return lint_expect_i64(lint_mod(big, lint_add(lint_small(m), lint_small(1))));
    }
    logos_fatal(logos_says_bitand_refused);
}
__attribute__((unused)) static int64_t lint_bitor(LInt a, LInt b) { int64_t x, y; if (lint_i64(a, &x) && lint_i64(b, &y)) return x | y; logos_fatal(logos_says_bitor_refused); }
__attribute__((unused)) static int64_t lint_bitxor(LInt a, LInt b) { int64_t x, y; if (lint_i64(a, &x) && lint_i64(b, &y)) return x ^ y; logos_fatal(logos_says_bitxor_refused); }
__attribute__((unused)) static int64_t lint_shl(LInt a, LInt b) { int64_t x, y; if (lint_i64(a, &x) && lint_i64(b, &y)) return (int64_t)((uint64_t)x << ((uint32_t)y & 63u)); logos_fatal(logos_says_shl_refused); }
__attribute__((unused)) static int64_t lint_shr(LInt a, LInt b) { int64_t x, y; if (lint_i64(a, &x) && lint_i64(b, &y)) return x >> ((uint32_t)y & 63u); logos_fatal(logos_says_shr_refused); }

/* ===== An exact number as the VM holds it. The VM keeps a whole rational as the integer it equals
   (`RuntimeValue::from_rational`), so a rational here answers `/`, `//`, `%`, `**`, the bitwise operators, an index
   and a range bound by its value: a whole one as that integer does, a fraction by the rational rules, each refusal
   in the VM's words. Operands are `LDec`s of an integer or a ratio; an integer operand comes in through
   `ldec_from_lint`. ===== */
__attribute__((unused)) static int ldec_whole(const LDec *d) {
    if (d->is_rat) return d->b.len == 1 && d->b.limb[0] == 1;
    if (d->scale == 0) return 1;
    LBig p = lbig_pow10(d->scale), rem; LBig q = lbig_divmod_mag(&d->a, &p, &rem);
    int whole = rem.len == 0; lbig_free(&p); lbig_free(&q); lbig_free(&rem); return whole;
}
/* The integer a whole exact number equals. */
__attribute__((unused)) static LInt ldec_integer(const LDec *d) {
    LBig num, den; ldec_as_rat(d, &num, &den);
    LBig q = lbig_exact_div(&num, &den);
    lbig_free(&num); lbig_free(&den); return lint_norm(q);
}
__attribute__((unused)) static inline int ldec_is_zero(const LDec *d) { return d->a.len == 0; }
/* The double the VM converts an exact number to: numerator over denominator, each rounded once. */
__attribute__((unused)) static double ldec_to_f64(LDec d) {
    LBig num, den; ldec_as_rat(&d, &num, &den);
    double v = lbig_to_f64(&num) / lbig_to_f64(&den);
    lbig_free(&num); lbig_free(&den); return v;
}
/* `x / y`: two whole numbers divide as integers, toward zero; a fraction divides exactly. */
__attribute__((unused)) static LDec ldec_quotient(LDec x, LDec y) {
    if (ldec_whole(&x) && ldec_whole(&y)) {
        LInt b = ldec_integer(&y);
        if (lint_is_zero(b)) logos_fatal(logos_says_division_by_zero);
        return ldec_from_lint(lint_div(ldec_integer(&x), b));
    }
    if (ldec_is_zero(&y)) logos_fatal(logos_says_rational_division_by_zero);
    return ldec_div(x, y);
}
/* `x // y`: the floor of the exact quotient. */
__attribute__((unused)) static LDec ldec_floor_quotient(LDec x, LDec y) {
    if (ldec_is_zero(&y)) logos_fatal(logos_says_exact_floor_division_by_zero);
    return ldec_from_lint(lint_norm(ldec_floor(ldec_div(x, y))));
}
/* `x % y`: two whole numbers take the integer remainder, carrying the dividend's sign; a fraction has none. */
__attribute__((unused)) static LDec ldec_remainder(LDec x, LDec y) {
    int wx = ldec_whole(&x), wy = ldec_whole(&y);
    if (wx && wy) {
        LInt b = ldec_integer(&y);
        if (lint_is_zero(b)) logos_fatal(logos_says_modulo_by_zero);
        return ldec_from_lint(lint_mod(ldec_integer(&x), b));
    }
    if (wx) logos_fatal(logos_says_modulo_int_rational);
    if (wy) logos_fatal(logos_says_modulo_rational_int);
    logos_fatal(logos_says_modulo_rational_rational);
}
/* `base ** exponent`: a whole base takes an integer's power, which has no negative exponent; a fraction takes any
   exponent within i32; an exponent that is a fraction is refused. */
__attribute__((unused)) static LDec ldec_power(LDec base, LDec exponent) {
    int wb = ldec_whole(&base);
    if (!ldec_whole(&exponent)) logos_fatal(wb ? logos_says_power_int_rational : logos_says_power_rational_rational);
    LInt e = ldec_integer(&exponent);
    if (wb) {
        LInt b = ldec_integer(&base);
        if (e.big) logos_fatal(logos_says_power_int_big);
        if (!b.big) {
            if (e.small < 0) logos_fatal(logos_says_negative_exponent);
            if (e.small > (int64_t)UINT32_MAX) logos_fatal(logos_says_exponent_too_large);
        } else if (e.small < 0 || e.small > (int64_t)UINT32_MAX) {
            logos_fatal(logos_says_big_exponent);
        }
        return ldec_from_lint(lint_pow(b, e.small));
    }
    if (e.big) logos_fatal(logos_says_power_rational_big);
    if (e.small < INT32_MIN || e.small > INT32_MAX) logos_fatal(logos_says_rational_exponent_too_large);
    LBig num, den; ldec_as_rat(&base, &num, &den);
    uint64_t k = e.small < 0 ? (uint64_t)(-e.small) : (uint64_t)e.small;
    LBig pn = lbig_pow(&num, k), pd = lbig_pow(&den, k);
    lbig_free(&num); lbig_free(&den);
    return e.small < 0 ? ldec_rat(pd, pn) : ldec_rat(pn, pd);
}
/* `x & y`, `x | y`, `x ^ y`, `x << y`, `x >> y` (`op` '&', '|', '^', '<', '>'): two whole numbers as the integers
   they are, a fraction refused. */
__attribute__((unused)) static int64_t ldec_bitwise(LDec x, LDec y, char op) {
    if (!ldec_whole(&x) || !ldec_whole(&y)) {
        switch (op) {
        case '&': logos_fatal(logos_says_bitand_fraction);
        case '|': logos_fatal(logos_says_bitor_fraction);
        case '^': logos_fatal(logos_says_bitxor_fraction);
        case '<': logos_fatal(logos_says_shl_fraction);
        default: logos_fatal(logos_says_shr_fraction);
        }
    }
    LInt a = ldec_integer(&x), b = ldec_integer(&y);
    switch (op) {
    case '&': return lint_bitand(a, b);
    case '|': return lint_bitor(a, b);
    case '^': return lint_bitxor(a, b);
    case '<': return lint_shl(a, b);
    default: return lint_shr(a, b);
    }
}
/* An exact number where the VM takes a machine integer — an index, a range bound, a slice bound: a whole one within
   i64 is that integer; one past it, or a fraction, is refused in the VM's words for the place (`past_word`,
   `fraction`). */
__attribute__((unused)) static int64_t ldec_as_i64(LDec d, const char *past_word, const char *fraction) {
    if (!ldec_whole(&d)) logos_fatal(fraction);
    int64_t i;
    if (!lint_i64(ldec_integer(&d), &i)) logos_fatal(past_word);
    return i;
}
/* The double `f`, exactly, as a ratio: its significand over a power of two. Finite `f` only. */
__attribute__((unused)) static LDec ldec_of_f64(double f) {
    int e; double m = frexp(f, &e);
    int64_t sig = (int64_t)ldexp(m, 53); e -= 53;
    LBig two = lbig_from_i64(2), num = lbig_from_i64(sig), den = lbig_from_i64(1);
    LBig p = lbig_pow(&two, (uint64_t)(e < 0 ? -e : e)); lbig_free(&two);
    if (e >= 0) { LBig t = lbig_mul(&num, &p); lbig_free(&num); num = t; } else { lbig_free(&den); den = lbig_copy(&p); }
    lbig_free(&p);
    return ldec_rat(num, den);
}
/* The order of an exact number against a double, exactly (`cmp_rational_f64_exact`): -1, 0 or 1, and 2 when `f` is
   NaN, which orders against nothing. */
__attribute__((unused)) static int ldec_cmp_f64(LDec x, double f) {
    if (f != f) return 2;
    if (f == INFINITY) return -1;
    if (f == -INFINITY) return 1;
    return ldec_cmp(x, ldec_of_f64(f));
}

"#
    };
}
pub(crate) use exact_numbers_source;

const C_RUNTIME: &str = concat!(
r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <inttypes.h>
#include <stdarg.h>
#include <math.h>
#include <time.h>


/* ========== Memory ========== */

"#,
    lc_gc_libc_source!(),
    lc_gc_api_source!(),
    r#"

/* ========== Failure ========== */

/* A runtime failure: the program stops with its message on stderr — or, inside a task of a concurrent
   program, the task stops and never returns here, and the run fails with its first failure once every
   other task is quiescent (the scheduler installs `logos_fail_hook` for the run). */
static void (*logos_fail_hook)(const char *) = 0;

__attribute__((unused, noreturn)) static void logos_fatal(const char *message) {
    fflush(stdout);
    if (logos_fail_hook) logos_fail_hook(message);
    fprintf(stderr, "%s\n", message);
    exit(1);
}

__attribute__((unused, noreturn)) static void logos_fatalf(const char *fmt, ...) {
    char buf[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    logos_fatal(lc_gc_strdup(buf));
}

/* Division and remainder as the interpreter takes them: truncating, and a zero divisor fails the run (a task,
   under the scheduler) with the interpreter's words. INT64_MIN / -1, the one quotient past int64, wraps
   modulo 2^64 as every Int operation of this tier does (by unsigned negation, which C defines); its
   remainder is 0. */
__attribute__((unused)) static int64_t logos_idiv(int64_t a, int64_t b) {
    if (b == 0) logos_fatalf(@DIVISION_BY_ZERO@);
    if (b == -1) return (int64_t)(0 - (uint64_t)a);
    return a / b;
}

__attribute__((unused)) static int64_t logos_imod(int64_t a, int64_t b) {
    if (b == 0) logos_fatalf(@MODULO_BY_ZERO@);
    if (b == -1) return 0;
    return a % b;
}

__attribute__((unused)) static double logos_rdiv(double a, double b) {
    if (b == 0.0) logos_fatalf(@REAL_DIVISION_BY_ZERO@);
    return a / b;
}

/* The language's `min` and `max` of two reals (base's `float_min`/`float_max`): a NaN gives way to the
   other operand, and of two equal values `min` answers the one with the negative sign, `max` the one with
   the positive sign, so `min(0, -0.0)` is `-0.0` on every tier and under every optimizer. */
__attribute__((unused)) static double logos_min_f64(double a, double b) {
    if (a != a) return b;
    if (b != b || a < b || (a == b && signbit(a))) return a;
    return b;
}

__attribute__((unused)) static double logos_max_f64(double a, double b) {
    if (a != a) return b;
    if (b != b || a > b || (a == b && !signbit(a))) return a;
    return b;
}

/* The byte length of the Unicode whitespace character (Rust's `char::is_whitespace`) that the UTF-8 text `s`
   of `n` bytes starts with, or 0 when it starts with anything else. */
__attribute__((unused)) static size_t logos_space_len(const unsigned char *s, size_t n) {
    if (n >= 1 && ((s[0] >= 0x09 && s[0] <= 0x0D) || s[0] == 0x20)) return 1;
    if (n >= 2 && s[0] == 0xC2 && (s[1] == 0x85 || s[1] == 0xA0)) return 2;
    if (n >= 3 && s[0] == 0xE1 && s[1] == 0x9A && s[2] == 0x80) return 3;
    if (n >= 3 && s[0] == 0xE2 && s[1] == 0x80 && ((s[2] >= 0x80 && s[2] <= 0x8A) || s[2] == 0xA8 || s[2] == 0xA9 || s[2] == 0xAF)) return 3;
    if (n >= 3 && s[0] == 0xE2 && s[1] == 0x81 && s[2] == 0x9F) return 3;
    if (n >= 3 && s[0] == 0xE3 && s[1] == 0x80 && s[2] == 0x80) return 3;
    return 0;
}

/* `s` without its leading and trailing whitespace — exactly what Rust's `str::trim` removes — as a fresh string. */
__attribute__((unused)) static char *logos_trim(const char *s) {
    const unsigned char *u = (const unsigned char *)s;
    size_t n = strlen(s), lo = 0, hi = n, k;
    while (lo < hi && (k = logos_space_len(u + lo, hi - lo))) lo += k;
    for (;;) {
        size_t back = 0;
        for (size_t w = 1; w <= 3 && w <= hi - lo; w++) {
            if ((w == 1 || (u[hi - w] & 0xC0) != 0x80) && logos_space_len(u + hi - w, w) == w) { back = w; break; }
        }
        if (!back) break;
        hi -= back;
    }
    char *t = (char *)lc_gc_alloc_atomic(hi - lo + 1);
    memcpy(t, s + lo, hi - lo);
    t[hi - lo] = 0;
    return t;
}

/* ========== String Formatting Helpers ========== */

static char *logos_center_str(const char *s, int width) {
    int len = (int)strlen(s);
    if (len >= width) return lc_gc_strdup(s);
    int total_pad = width - len;
    int left = total_pad / 2;
    int right = total_pad - left;
    char *result = (char *)lc_gc_alloc_atomic(width + 1);
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
    char *buf = (char *)lc_gc_alloc_atomic(len + 1);
    vsnprintf(buf, len + 1, fmt, args2);
    va_end(args2);
    return buf;
}

/* Forward decl: `str_concat` is defined further down but is used by the map/set `*_to_string`
 * builders above that point (insertion-ordered `Show`). */
static char *str_concat(const char *a, const char *b);

/* ========== Dynamic Array: Seq_i64 ========== */

typedef struct { int64_t *data; size_t len; size_t cap; } Seq_i64;

static Seq_i64 seq_i64_new(void) { return (Seq_i64){NULL, 0, 0}; }

static void seq_i64_push(Seq_i64 *s, int64_t val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (int64_t *)lc_gc_realloc(s->data, s->cap * sizeof(int64_t));
    }
    s->data[s->len++] = val;
}

/* LOGOS `item k of xs`: 1-based from the front (k>=1 -> k-1), or from the END for a negative k
   (k<0 -> len+k, so -1 is the last element) — matching the VM/tree-walker's Python-style negative index.
   An index naming no element (incl. `item 0`) fails the run with the VM's sentence, which names the index
   asked for (flush first so any already-printed output matches the VM's partial stream). */
static inline size_t logos_idx0(int64_t idx, size_t len) {
    if (idx >= 1) { if ((uint64_t)(idx - 1) < (uint64_t)len) return (size_t)(idx - 1); }
    else if (idx <= -1) { uint64_t back = (uint64_t)0 - (uint64_t)idx; if (back <= (uint64_t)len) return (size_t)((uint64_t)len - back); }
    logos_fatalf(@INDEX_OUT_OF_BOUNDS@, idx);
}
static int64_t seq_i64_get(Seq_i64 *s, int64_t idx) { return s->data[logos_idx0(idx, s->len)]; }
static void seq_i64_set(Seq_i64 *s, int64_t idx, int64_t val) { s->data[logos_idx0(idx, s->len)] = val; }
static int64_t seq_i64_len(Seq_i64 *s) { return (int64_t)s->len; }

/* text <-> byte sequence — the foundation the whole `strings.lg` stdlib builds on.
   `text_bytes` yields the raw bytes (0..255) as a Seq of Int; `text_from_bytes` reassembles them.
   Matches the interpreter/Rust builtins (Text is UTF-8, bytes are the encoding's octets). */
static Seq_i64 logos_text_bytes(const char *s) {
    Seq_i64 r = seq_i64_new();
    if (s) {
        for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
            seq_i64_push(&r, (int64_t)*p);
        }
    }
    return r;
}
static char *logos_text_from_bytes(Seq_i64 s) {
    char *buf = (char *)lc_gc_alloc_atomic(s.len + 1);
    for (size_t i = 0; i < s.len; i++) {
        buf[i] = (char)(unsigned char)s.data[i];
    }
    buf[s.len] = '\0';
    return buf;
}

/* `chr(code)` = the single character with the given Unicode scalar value, UTF-8 encoded —
   byte-identical to the interpreter's `char::from_u32(code).to_string()`. */
static char *logos_chr(int64_t code) {
    uint32_t c = (uint32_t)code;
    unsigned char buf[4];
    int n = 0;
    if (c < 0x80) {
        buf[n++] = (unsigned char)c;
    } else if (c < 0x800) {
        buf[n++] = (unsigned char)(0xC0 | (c >> 6));
        buf[n++] = (unsigned char)(0x80 | (c & 0x3F));
    } else if (c < 0x10000) {
        buf[n++] = (unsigned char)(0xE0 | (c >> 12));
        buf[n++] = (unsigned char)(0x80 | ((c >> 6) & 0x3F));
        buf[n++] = (unsigned char)(0x80 | (c & 0x3F));
    } else {
        buf[n++] = (unsigned char)(0xF0 | (c >> 18));
        buf[n++] = (unsigned char)(0x80 | ((c >> 12) & 0x3F));
        buf[n++] = (unsigned char)(0x80 | ((c >> 6) & 0x3F));
        buf[n++] = (unsigned char)(0x80 | (c & 0x3F));
    }
    char *out = (char *)lc_gc_alloc_atomic(n + 1);
    for (int i = 0; i < n; i++) out[i] = (char)buf[i];
    out[n] = '\0';
    return out;
}

/* ========== Dynamic Array: Seq_bool ========== */

typedef struct { bool *data; size_t len; size_t cap; } Seq_bool;

static Seq_bool seq_bool_new(void) { return (Seq_bool){NULL, 0, 0}; }

static void seq_bool_push(Seq_bool *s, bool val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (bool *)lc_gc_realloc_atomic(s->data, s->cap * sizeof(bool));
    }
    s->data[s->len++] = val;
}

static bool seq_bool_get(Seq_bool *s, int64_t idx) { return s->data[logos_idx0(idx, s->len)]; }
static void seq_bool_set(Seq_bool *s, int64_t idx, bool val) { s->data[logos_idx0(idx, s->len)] = val; }
static int64_t seq_bool_len(Seq_bool *s) { return (int64_t)s->len; }

/* ========== Dynamic Array: Seq_str ========== */

typedef struct { char **data; size_t len; size_t cap; } Seq_str;

static Seq_str seq_str_new(void) { return (Seq_str){NULL, 0, 0}; }

static void seq_str_push(Seq_str *s, char *val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (char **)lc_gc_realloc(s->data, s->cap * sizeof(char *));
    }
    s->data[s->len++] = val;
}

static char *seq_str_get(Seq_str *s, int64_t idx) { return s->data[logos_idx0(idx, s->len)]; }
static void seq_str_set(Seq_str *s, int64_t idx, char *val) { s->data[logos_idx0(idx, s->len)] = val; }
static int64_t seq_str_len(Seq_str *s) { return (int64_t)s->len; }

/* ========== Hash Map: Map_i64_i64 (open addressing) ========== */

/* Every map's and set's capacity is a power of two — 16 to start, doubled as it grows, a capacity hint rounded
   up to one — so a hash or a probe step reduces to a slot by masking with `cap - 1`.

   Removing an entry leaves no hole a lookup could stop at: the entries after it in its probe run move back
   over it (each only as far as its home slot allows), so every slot a probe crosses is still full, and a slot
   is either full or empty. The insertion order drops the removed slot and follows every moved entry, so the
   rest keep their order and a value added again comes last, as the VM's ordered maps and sets give it. */
__attribute__((unused)) static void _logos_order_drop(size_t *order, size_t *n, size_t slot) {
    for (size_t k = 0; k < *n; k++) {
        if (order[k] == slot) {
            memmove(order + k, order + k + 1, (*n - k - 1) * sizeof(size_t));
            (*n)--;
            return;
        }
    }
}
__attribute__((unused)) static void _logos_order_move(size_t *order, size_t n, size_t from, size_t to) {
    for (size_t k = 0; k < n; k++) {
        if (order[k] == from) {
            order[k] = to;
            return;
        }
    }
}
/* Whether the entry at slot `j`, whose home slot is `home`, may move back to the hole at `i`: `i` lies on its
   probe path from `home` to `j`. */
__attribute__((unused)) static int _logos_slot_backs(size_t home, size_t i, size_t j, size_t mask) {
    return ((j - home) & mask) >= ((j - i) & mask);
}

typedef struct {
    int64_t *keys;
    int64_t *vals;
    uint8_t *state;
    size_t cap;
    size_t len;
    size_t *order;
    size_t ord_len;
} Map_i64_i64;

static size_t _map_hash(int64_t key, size_t cap) {
    uint64_t h = (uint64_t)key;
    h = (h ^ (h >> 30)) * 0xbf58476d1ce4e5b9ULL;
    h = (h ^ (h >> 27)) * 0x94d049bb133111ebULL;
    h = h ^ (h >> 31);
    return (size_t)(h & (cap - 1));
}

static size_t logos_map_capacity(int64_t cap_hint) {
    if (cap_hint <= 0) return 16;
    uint64_t need = (uint64_t)cap_hint;
    if (need > (uint64_t)SIZE_MAX / 2) logos_fatal("map capacity overflow");
    need *= 2;
    size_t cap = 16;
    while ((uint64_t)cap < need) {
        if (cap > SIZE_MAX / 2) logos_fatal("map capacity overflow");
        cap *= 2;
    }
    return cap;
}

static Map_i64_i64 map_i64_i64_new(void) {
    size_t cap = 16;
    Map_i64_i64 m;
    m.keys = (int64_t *)lc_gc_calloc(cap, sizeof(int64_t));
    m.vals = (int64_t *)lc_gc_calloc(cap, sizeof(int64_t));
    m.state = (uint8_t *)lc_gc_calloc_atomic(cap, sizeof(uint8_t));
    m.cap = cap;
    m.len = 0;
    m.order = (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t));
    m.ord_len = 0;
    return m;
}

static Map_i64_i64 map_i64_i64_with_capacity(int64_t cap_hint) {
    size_t cap = logos_map_capacity(cap_hint);
    Map_i64_i64 m;
    m.keys = (int64_t *)lc_gc_calloc(cap, sizeof(int64_t));
    m.vals = (int64_t *)lc_gc_calloc(cap, sizeof(int64_t));
    m.state = (uint8_t *)lc_gc_calloc_atomic(cap, sizeof(uint8_t));
    m.cap = cap;
    m.len = 0;
    m.order = (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t));
    m.ord_len = 0;
    return m;
}

static void _map_resize(Map_i64_i64 *m) {
    int64_t *ok = m->keys; int64_t *ov = m->vals; uint8_t *os = m->state; size_t *oo = m->order; size_t ool = m->ord_len;
    size_t nc = m->cap * 2;
    m->keys = (int64_t *)lc_gc_calloc(nc, sizeof(int64_t));
    m->vals = (int64_t *)lc_gc_calloc(nc, sizeof(int64_t));
    m->state = (uint8_t *)lc_gc_calloc_atomic(nc, sizeof(uint8_t));
    m->order = (size_t *)lc_gc_calloc_atomic(nc, sizeof(size_t));
    m->cap = nc; m->len = 0; m->ord_len = 0;
    for (size_t oi = 0; oi < ool; oi++) {
        size_t si = oo[oi];
        if (!os[si]) continue;
        size_t idx = _map_hash(ok[si], nc);
        while (m->state[idx]) idx = (idx + 1) & (nc - 1);
        m->keys[idx] = ok[si]; m->vals[idx] = ov[si]; m->state[idx] = 1; m->len++;
        m->order[m->ord_len++] = idx;
    }
    lc_gc_free(ok); lc_gc_free(ov); lc_gc_free(os); lc_gc_free(oo);
}

/* Reserve for a proven counted insertion loop. The ordered-map representation
   remains unchanged; this only pays its allocations and rehashing once before
   the hot loop instead of repeatedly inside it. */
static void map_i64_i64_reserve(Map_i64_i64 *m, int64_t additional) {
    if (additional <= 0) return;
    size_t add = (size_t)additional;
    if (add > SIZE_MAX - m->ord_len) logos_fatal("map capacity overflow");
    size_t needed = m->ord_len + add;
    while (needed > m->cap - (m->cap / 4)) {
        if (m->cap > SIZE_MAX / 2) logos_fatal("map capacity overflow");
        _map_resize(m);
    }
}

static void map_i64_i64_set(Map_i64_i64 *m, int64_t key, int64_t val) {
    if (m->ord_len * 4 >= m->cap * 3) _map_resize(m);
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) { m->vals[idx] = val; return; }
        idx = (idx + 1) & (m->cap - 1);
    }
    m->keys[idx] = key;
    m->vals[idx] = val;
    m->state[idx] = 1;
    m->len++;
    m->order[m->ord_len++] = idx;
}

/* The text of an integer a collection holds: an Int signed, a machine word (`word`) by its unsigned value. */
__attribute__((unused)) static char *logos_int_text(int64_t v, int word) {
    return word ? logos_dyn_sprintf("%" PRIu64, (uint64_t)v) : logos_dyn_sprintf("%" PRId64, v);
}
__attribute__((unused)) static char *map_i64_i64_to_string(Map_i64_i64 *m) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < m->ord_len; oi++) {
        size_t si = m->order[oi]; if (!m->state[si]) continue;
        char *e = logos_dyn_sprintf("%s%" PRId64 ": %" PRId64, first ? "" : ", ", m->keys[si], m->vals[si]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}
__attribute__((unused)) static void show_map_i64_i64(Map_i64_i64 *m) { char *t = map_i64_i64_to_string(m); printf("%s\n", t); lc_gc_free(t); }
__attribute__((unused)) static char *map_i64_i64_to_string_as(Map_i64_i64 *m, int key_words, int val_words) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < m->ord_len; oi++) {
        size_t si = m->order[oi]; if (!m->state[si]) continue;
        char *k = logos_int_text(m->keys[si], key_words), *v = logos_int_text(m->vals[si], val_words);
        char *e = logos_dyn_sprintf("%s%s: %s", first ? "" : ", ", k, v); lc_gc_free(k); lc_gc_free(v);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}

/* Reading an absent key is a TRAP on the VM (`item k of m` requires k present), so abort ≡ VM rather than
   returning a silent default. */
static void logos_map_miss(void) { logos_fatal("missing map key"); }
static int64_t map_i64_i64_get(Map_i64_i64 *m, int64_t key) {
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) return m->vals[idx];
        idx = (idx + 1) & (m->cap - 1);
    }
    logos_map_miss();
    return 0;
}

/* ========== String Hash Function ========== */

static size_t _map_hash_str(const char *key, size_t cap) {
    uint64_t h = 5381;
    while (*key) { h = ((h << 5) + h) + (uint8_t)*key; key++; }
    return (size_t)(h & (cap - 1));
}

/* ========== Hash Map: Map_str_i64 ========== */

/* Maps carry an `order` array (slots in insertion order) + `ord_len` so iteration/`Show` is
 * insertion-ordered — byte-identical to the interpreter oracle. Hash lookup (get/contains) is
 * unchanged; only set/resize maintain `order`, and resize compacts tombstones while preserving order. */
typedef struct {
    char **keys; int64_t *vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len;
} Map_str_i64;

static Map_str_i64 map_str_i64_new(void) {
    size_t cap = 16;
    Map_str_i64 m = {(char **)lc_gc_calloc(cap, sizeof(char *)),
        (int64_t *)lc_gc_calloc(cap, sizeof(int64_t)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return m;
}

static Map_str_i64 map_str_i64_with_capacity(int64_t cap_hint) {
    size_t cap = logos_map_capacity(cap_hint);
    Map_str_i64 m = {(char **)lc_gc_calloc(cap, sizeof(char *)),
        (int64_t *)lc_gc_calloc(cap, sizeof(int64_t)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return m;
}

static void _map_str_i64_resize(Map_str_i64 *m) {
    char **ok = m->keys; int64_t *ov = m->vals; uint8_t *os = m->state; size_t *oo = m->order; size_t ool = m->ord_len;
    size_t nc = m->cap * 2;
    m->keys = (char **)lc_gc_calloc(nc, sizeof(char *));
    m->vals = (int64_t *)lc_gc_calloc(nc, sizeof(int64_t));
    m->state = (uint8_t *)lc_gc_calloc_atomic(nc, 1);
    m->order = (size_t *)lc_gc_calloc_atomic(nc, sizeof(size_t));
    m->cap = nc; m->len = 0; m->ord_len = 0;
    for (size_t oi = 0; oi < ool; oi++) {
        size_t si = oo[oi];
        if (!os[si]) continue;
        size_t idx = _map_hash_str(ok[si], nc);
        while (m->state[idx]) idx = (idx + 1) & (nc - 1);
        m->keys[idx] = ok[si]; m->vals[idx] = ov[si]; m->state[idx] = 1; m->len++;
        m->order[m->ord_len++] = idx;
    }
    lc_gc_free(ok); lc_gc_free(ov); lc_gc_free(os); lc_gc_free(oo);
}

static void map_str_i64_set(Map_str_i64 *m, const char *key, int64_t val) {
    if (m->ord_len * 4 >= m->cap * 3) _map_str_i64_resize(m);
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) { m->vals[idx] = val; return; }
        idx = (idx + 1) & (m->cap - 1);
    }
    m->keys[idx] = lc_gc_strdup(key); m->vals[idx] = val; m->state[idx] = 1; m->len++;
    m->order[m->ord_len++] = idx;
}

__attribute__((unused)) static char *map_str_i64_to_string(Map_str_i64 *m) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < m->ord_len; oi++) {
        size_t si = m->order[oi]; if (!m->state[si]) continue;
        char *e = logos_dyn_sprintf("%s%s: %" PRId64, first ? "" : ", ", m->keys[si], m->vals[si]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}
__attribute__((unused)) static void show_map_str_i64(Map_str_i64 *m) { char *t = map_str_i64_to_string(m); printf("%s\n", t); lc_gc_free(t); }
__attribute__((unused)) static char *map_str_i64_to_string_as(Map_str_i64 *m, int val_words) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < m->ord_len; oi++) {
        size_t si = m->order[oi]; if (!m->state[si]) continue;
        char *v = logos_int_text(m->vals[si], val_words);
        char *e = logos_dyn_sprintf("%s%s: %s", first ? "" : ", ", m->keys[si], v); lc_gc_free(v);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}

static int64_t map_str_i64_get(Map_str_i64 *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return m->vals[idx];
        idx = (idx + 1) & (m->cap - 1);
    }
    logos_map_miss();
    return 0;
}

static bool map_str_i64_contains(Map_str_i64 *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return true;
        idx = (idx + 1) & (m->cap - 1);
    }
    return false;
}

/* ========== Hash Map: Map_str_str ========== */

typedef struct {
    char **keys; char **vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len;
} Map_str_str;

static Map_str_str map_str_str_new(void) {
    size_t cap = 16;
    Map_str_str m = {(char **)lc_gc_calloc(cap, sizeof(char *)),
        (char **)lc_gc_calloc(cap, sizeof(char *)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return m;
}

static Map_str_str map_str_str_with_capacity(int64_t cap_hint) {
    size_t cap = logos_map_capacity(cap_hint);
    Map_str_str m = {(char **)lc_gc_calloc(cap, sizeof(char *)),
        (char **)lc_gc_calloc(cap, sizeof(char *)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return m;
}

static void _map_str_str_resize(Map_str_str *m) {
    char **ok = m->keys; char **ov = m->vals; uint8_t *os = m->state; size_t *oo = m->order; size_t ool = m->ord_len;
    size_t nc = m->cap * 2;
    m->keys = (char **)lc_gc_calloc(nc, sizeof(char *));
    m->vals = (char **)lc_gc_calloc(nc, sizeof(char *));
    m->state = (uint8_t *)lc_gc_calloc_atomic(nc, 1);
    m->order = (size_t *)lc_gc_calloc_atomic(nc, sizeof(size_t));
    m->cap = nc; m->len = 0; m->ord_len = 0;
    for (size_t oi = 0; oi < ool; oi++) {
        size_t si = oo[oi];
        if (!os[si]) continue;
        size_t idx = _map_hash_str(ok[si], nc);
        while (m->state[idx]) idx = (idx + 1) & (nc - 1);
        m->keys[idx] = ok[si]; m->vals[idx] = ov[si]; m->state[idx] = 1; m->len++;
        m->order[m->ord_len++] = idx;
    }
    lc_gc_free(ok); lc_gc_free(ov); lc_gc_free(os); lc_gc_free(oo);
}

static void map_str_str_set(Map_str_str *m, const char *key, char *val) {
    if (m->ord_len * 4 >= m->cap * 3) _map_str_str_resize(m);
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) { m->vals[idx] = val; return; }
        idx = (idx + 1) & (m->cap - 1);
    }
    m->keys[idx] = lc_gc_strdup(key); m->vals[idx] = val; m->state[idx] = 1; m->len++;
    m->order[m->ord_len++] = idx;
}

__attribute__((unused)) static char *map_str_str_to_string(Map_str_str *m) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < m->ord_len; oi++) {
        size_t si = m->order[oi]; if (!m->state[si]) continue;
        char *e = logos_dyn_sprintf("%s%s: %s", first ? "" : ", ", m->keys[si], m->vals[si]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}
__attribute__((unused)) static void show_map_str_str(Map_str_str *m) { char *t = map_str_str_to_string(m); printf("%s\n", t); lc_gc_free(t); }

static char *map_str_str_get(Map_str_str *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return m->vals[idx];
        idx = (idx + 1) & (m->cap - 1);
    }
    logos_map_miss();
    return "";
}

static bool map_str_str_contains(Map_str_str *m, const char *key) {
    size_t idx = _map_hash_str(key, m->cap);
    while (m->state[idx]) {
        if (strcmp(m->keys[idx], key) == 0) return true;
        idx = (idx + 1) & (m->cap - 1);
    }
    return false;
}

/* ========== Hash Map: Map_i64_str ========== */

typedef struct {
    int64_t *keys; char **vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len;
} Map_i64_str;

static Map_i64_str map_i64_str_new(void) {
    size_t cap = 16;
    Map_i64_str m = {(int64_t *)lc_gc_calloc(cap, sizeof(int64_t)),
        (char **)lc_gc_calloc(cap, sizeof(char *)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return m;
}

static Map_i64_str map_i64_str_with_capacity(int64_t cap_hint) {
    size_t cap = logos_map_capacity(cap_hint);
    Map_i64_str m = {(int64_t *)lc_gc_calloc(cap, sizeof(int64_t)),
        (char **)lc_gc_calloc(cap, sizeof(char *)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return m;
}

static void _map_i64_str_resize(Map_i64_str *m) {
    int64_t *ok = m->keys; char **ov = m->vals; uint8_t *os = m->state; size_t *oo = m->order; size_t ool = m->ord_len;
    size_t nc = m->cap * 2;
    m->keys = (int64_t *)lc_gc_calloc(nc, sizeof(int64_t));
    m->vals = (char **)lc_gc_calloc(nc, sizeof(char *));
    m->state = (uint8_t *)lc_gc_calloc_atomic(nc, 1);
    m->order = (size_t *)lc_gc_calloc_atomic(nc, sizeof(size_t));
    m->cap = nc; m->len = 0; m->ord_len = 0;
    for (size_t oi = 0; oi < ool; oi++) {
        size_t si = oo[oi];
        if (!os[si]) continue;
        size_t idx = _map_hash(ok[si], nc);
        while (m->state[idx]) idx = (idx + 1) & (nc - 1);
        m->keys[idx] = ok[si]; m->vals[idx] = ov[si]; m->state[idx] = 1; m->len++;
        m->order[m->ord_len++] = idx;
    }
    lc_gc_free(ok); lc_gc_free(ov); lc_gc_free(os); lc_gc_free(oo);
}

static void map_i64_str_set(Map_i64_str *m, int64_t key, char *val) {
    if (m->ord_len * 4 >= m->cap * 3) _map_i64_str_resize(m);
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) { m->vals[idx] = val; return; }
        idx = (idx + 1) & (m->cap - 1);
    }
    m->keys[idx] = key; m->vals[idx] = val; m->state[idx] = 1; m->len++;
    m->order[m->ord_len++] = idx;
}

__attribute__((unused)) static char *map_i64_str_to_string(Map_i64_str *m) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < m->ord_len; oi++) {
        size_t si = m->order[oi]; if (!m->state[si]) continue;
        char *e = logos_dyn_sprintf("%s%" PRId64 ": %s", first ? "" : ", ", m->keys[si], m->vals[si]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}
__attribute__((unused)) static void show_map_i64_str(Map_i64_str *m) { char *t = map_i64_str_to_string(m); printf("%s\n", t); lc_gc_free(t); }
__attribute__((unused)) static char *map_i64_str_to_string_as(Map_i64_str *m, int key_words) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < m->ord_len; oi++) {
        size_t si = m->order[oi]; if (!m->state[si]) continue;
        char *k = logos_int_text(m->keys[si], key_words);
        char *e = logos_dyn_sprintf("%s%s: %s", first ? "" : ", ", k, m->vals[si]); lc_gc_free(k);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}

static char *map_i64_str_get(Map_i64_str *m, int64_t key) {
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) return m->vals[idx];
        idx = (idx + 1) & (m->cap - 1);
    }
    logos_map_miss();
    return "";
}

static bool map_i64_str_contains(Map_i64_str *m, int64_t key) {
    size_t idx = _map_hash(key, m->cap);
    while (m->state[idx]) {
        if (m->keys[idx] == key) return true;
        idx = (idx + 1) & (m->cap - 1);
    }
    return false;
}

/* ========== String Helpers ========== */

static char *str_concat(const char *a, const char *b) {
    size_t la = strlen(a), lb = strlen(b);
    char *r = (char *)lc_gc_alloc_atomic(la + lb + 1);
    memcpy(r, a, la);
    memcpy(r + la, b, lb + 1);
    return r;
}

static char *str_append(char *s, const char *suffix) {
    size_t slen = strlen(s);
    size_t suffix_len = strlen(suffix);
    s = (char *)lc_gc_realloc_atomic(s, slen + suffix_len + 1);
    memcpy(s + slen, suffix, suffix_len + 1);
    return s;
}

static char *i64_to_str(int64_t n) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%" PRId64, n);
    return lc_gc_strdup(buf);
}

/* ========== Dynamic Array: Seq_f64 ========== */

typedef struct { double *data; size_t len; size_t cap; } Seq_f64;

static Seq_f64 seq_f64_new(void) { return (Seq_f64){NULL, 0, 0}; }

static void seq_f64_push(Seq_f64 *s, double val) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->data = (double *)lc_gc_realloc_atomic(s->data, s->cap * sizeof(double));
    }
    s->data[s->len++] = val;
}

static double seq_f64_get(Seq_f64 *s, int64_t idx) { return s->data[logos_idx0(idx, s->len)]; }
static void seq_f64_set(Seq_f64 *s, int64_t idx, double val) { s->data[logos_idx0(idx, s->len)] = val; }
static int64_t seq_f64_len(Seq_f64 *s) { return (int64_t)s->len; }

/* ========== Pop Operations ========== */

static int64_t seq_i64_pop(Seq_i64 *s) { int64_t value = s->data[--s->len]; s->data[s->len] = 0; return value; }
static bool seq_bool_pop(Seq_bool *s) { return s->data[--s->len]; }
static char *seq_str_pop(Seq_str *s) { char *value = s->data[--s->len]; s->data[s->len] = 0; return value; }
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
        idx = (idx + 1) & (m->cap - 1);
    }
    return false;
}

/* `Remove key from m`: see the removal note at `_logos_order_drop`. */
static void map_i64_i64_remove(Map_i64_i64 *t, int64_t key) {
    size_t mask = t->cap - 1, i = _map_hash(key, t->cap);
    while (t->state[i] && !(t->keys[i] == key)) i = (i + 1) & mask;
    if (!t->state[i]) return;
    _logos_order_drop(t->order, &t->ord_len, i);
    t->len--;
    for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {
        if (_logos_slot_backs(_map_hash(t->keys[j], t->cap), i, j, mask)) {
            t->keys[i] = t->keys[j];
            t->vals[i] = t->vals[j];
            _logos_order_move(t->order, t->ord_len, j, i);
            i = j;
        }
    }
    t->keys[i] = 0;
    t->vals[i] = 0;
    t->state[i] = 0;
}
static void map_str_i64_remove(Map_str_i64 *t, const char * key) {
    size_t mask = t->cap - 1, i = _map_hash_str(key, t->cap);
    while (t->state[i] && !(strcmp(t->keys[i], key) == 0)) i = (i + 1) & mask;
    if (!t->state[i]) return;
    _logos_order_drop(t->order, &t->ord_len, i);
    t->len--;
    for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {
        if (_logos_slot_backs(_map_hash_str(t->keys[j], t->cap), i, j, mask)) {
            t->keys[i] = t->keys[j];
            t->vals[i] = t->vals[j];
            _logos_order_move(t->order, t->ord_len, j, i);
            i = j;
        }
    }
    t->keys[i] = 0;
    t->vals[i] = 0;
    t->state[i] = 0;
}
static void map_str_str_remove(Map_str_str *t, const char * key) {
    size_t mask = t->cap - 1, i = _map_hash_str(key, t->cap);
    while (t->state[i] && !(strcmp(t->keys[i], key) == 0)) i = (i + 1) & mask;
    if (!t->state[i]) return;
    _logos_order_drop(t->order, &t->ord_len, i);
    t->len--;
    for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {
        if (_logos_slot_backs(_map_hash_str(t->keys[j], t->cap), i, j, mask)) {
            t->keys[i] = t->keys[j];
            t->vals[i] = t->vals[j];
            _logos_order_move(t->order, t->ord_len, j, i);
            i = j;
        }
    }
    t->keys[i] = 0;
    t->vals[i] = 0;
    t->state[i] = 0;
}
static void map_i64_str_remove(Map_i64_str *t, int64_t key) {
    size_t mask = t->cap - 1, i = _map_hash(key, t->cap);
    while (t->state[i] && !(t->keys[i] == key)) i = (i + 1) & mask;
    if (!t->state[i]) return;
    _logos_order_drop(t->order, &t->ord_len, i);
    t->len--;
    for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {
        if (_logos_slot_backs(_map_hash(t->keys[j], t->cap), i, j, mask)) {
            t->keys[i] = t->keys[j];
            t->vals[i] = t->vals[j];
            _logos_order_move(t->order, t->ord_len, j, i);
            i = j;
        }
    }
    t->keys[i] = 0;
    t->vals[i] = 0;
    t->state[i] = 0;
}

/* ========== Copy Operations ========== */

static Seq_i64 seq_i64_copy(Seq_i64 *s) {
    Seq_i64 r = {(int64_t *)lc_gc_alloc(s->len * sizeof(int64_t)), s->len, s->len};
    memcpy(r.data, s->data, s->len * sizeof(int64_t));
    return r;
}
static Seq_bool seq_bool_copy(Seq_bool *s) {
    Seq_bool r = {(bool *)lc_gc_alloc_atomic(s->len * sizeof(bool)), s->len, s->len};
    memcpy(r.data, s->data, s->len * sizeof(bool));
    return r;
}
static Seq_str seq_str_copy(Seq_str *s) {
    Seq_str r = {(char **)lc_gc_alloc(s->len * sizeof(char *)), s->len, s->len};
    for (size_t i = 0; i < s->len; i++) r.data[i] = lc_gc_strdup(s->data[i]);
    return r;
}
static Seq_f64 seq_f64_copy(Seq_f64 *s) {
    Seq_f64 r = {(double *)lc_gc_alloc_atomic(s->len * sizeof(double)), s->len, s->len};
    memcpy(r.data, s->data, s->len * sizeof(double));
    return r;
}

/* ========== String Operations ========== */

static bool str_equals(const char *a, const char *b) { return strcmp(a, b) == 0; }
static int64_t str_len(const char *s) { return (int64_t)strlen(s); }
/* `item k of <Text>` (1-based) → the k-th character as a fresh 1-char string, matching the VM's Show
 * and Text equality. Byte-indexed (correct for ASCII; a multi-byte codepoint is a documented limit). */
static char *logos_str_char_at(const char *s, int64_t k) {
    char *r = (char *)lc_gc_alloc_atomic(2);
    r[0] = (k >= 1 && (size_t)k <= strlen(s)) ? s[k - 1] : '\0';
    r[1] = '\0';
    return r;
}

/* ========== IO ========== */

/* Render an f64 into a fresh malloc'd string EXACTLY like the VM's `Show` (Rust's `{}` Display:
 * shortest decimal that round-trips, NEVER scientific notation, bare integral floats, NaN/inf/-inf).
 * Finds the shortest significant-digit count whose `%e` form round-trips via strtod, then reformats to
 * fixed decimal so any magnitude prints without an exponent — matching the VM for every finite value. */
static char *logos_fmt_f64(double x) {
    if (isnan(x)) return lc_gc_strdup("NaN");
    if (isinf(x)) return lc_gc_strdup(x < 0 ? "-inf" : "inf");
    char sci[64];
    int prec;
    for (prec = 0; prec <= 17; prec++) {
        snprintf(sci, sizeof sci, "%.*e", prec, x);
        if (strtod(sci, NULL) == x) break;
    }
    const char *p = sci;
    char sign = 0;
    if (*p == '-') { sign = '-'; p++; }
    char digits[40];
    int nd = 0;
    digits[nd++] = *p++;
    if (*p == '.') { p++; while (*p != 'e' && *p != 'E') digits[nd++] = *p++; }
    p++;
    int exp = atoi(p);
    int pointpos = exp + 1;
    char *out = (char *)lc_gc_alloc_atomic(600);
    int oi = 0;
    if (sign) out[oi++] = '-';
    if (pointpos <= 0) {
        out[oi++] = '0'; out[oi++] = '.';
        for (int i = 0; i < -pointpos; i++) out[oi++] = '0';
        for (int i = 0; i < nd; i++) out[oi++] = digits[i];
    } else if (pointpos >= nd) {
        for (int i = 0; i < nd; i++) out[oi++] = digits[i];
        for (int i = 0; i < pointpos - nd; i++) out[oi++] = '0';
    } else {
        for (int i = 0; i < nd; i++) { if (i == pointpos) out[oi++] = '.'; out[oi++] = digits[i]; }
    }
    out[oi] = '\0';
    return out;
}

static void show_i64(int64_t x) { printf("%" PRId64 "\n", x); }
static void show_f64(double x) { char *s = logos_fmt_f64(x); printf("%s\n", s); lc_gc_free(s); }
static void show_bool(bool x) { printf("%s\n", x ? "true" : "false"); }
static void show_str(const char *s) { printf("%s\n", s); }

/* Each scalar Seq has a `*_to_string` that returns the `[a, b, c]` display string (no newline) so it
 * can be embedded inside a struct's `<Name>_to_string`; `show_seq_*` is the thin print+newline wrapper. */
__attribute__((unused)) static char *seq_i64_to_string(Seq_i64 *s) {
    char *out = lc_gc_strdup("[");
    for (size_t i = 0; i < s->len; i++) {
        char *e = logos_dyn_sprintf("%s%" PRId64, i ? ", " : "", s->data[i]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw;
    }
    char *nw = str_concat(out, "]"); lc_gc_free(out); return nw;
}
static void show_seq_i64(Seq_i64 *s) { char *t = seq_i64_to_string(s); printf("%s\n", t); lc_gc_free(t); }
__attribute__((unused)) static char *seq_i64_to_string_as(Seq_i64 *s, int words) {
    char *out = lc_gc_strdup("[");
    for (size_t i = 0; i < s->len; i++) {
        char *v = logos_int_text(s->data[i], words);
        char *e = logos_dyn_sprintf("%s%s", i ? ", " : "", v); lc_gc_free(v);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw;
    }
    char *nw = str_concat(out, "]"); lc_gc_free(out); return nw;
}

__attribute__((unused)) static char *seq_bool_to_string(Seq_bool *s) {
    char *out = lc_gc_strdup("[");
    for (size_t i = 0; i < s->len; i++) {
        char *e = logos_dyn_sprintf("%s%s", i ? ", " : "", s->data[i] ? "true" : "false");
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw;
    }
    char *nw = str_concat(out, "]"); lc_gc_free(out); return nw;
}
static void show_seq_bool(Seq_bool *s) { char *t = seq_bool_to_string(s); printf("%s\n", t); lc_gc_free(t); }

__attribute__((unused)) static char *seq_str_to_string(Seq_str *s) {
    char *out = lc_gc_strdup("[");
    for (size_t i = 0; i < s->len; i++) {
        char *e = logos_dyn_sprintf("%s%s", i ? ", " : "", s->data[i]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw;
    }
    char *nw = str_concat(out, "]"); lc_gc_free(out); return nw;
}
static void show_seq_str(Seq_str *s) { char *t = seq_str_to_string(s); printf("%s\n", t); lc_gc_free(t); }

__attribute__((unused)) static char *seq_f64_to_string(Seq_f64 *s) {
    char *out = lc_gc_strdup("[");
    for (size_t i = 0; i < s->len; i++) {
        char *fs = logos_fmt_f64(s->data[i]);
        char *e = logos_dyn_sprintf("%s%s", i ? ", " : "", fs); lc_gc_free(fs);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw;
    }
    char *nw = str_concat(out, "]"); lc_gc_free(out); return nw;
}
static void show_seq_f64(Seq_f64 *s) { char *t = seq_f64_to_string(s); printf("%s\n", t); lc_gc_free(t); }

/* ========== WithCapacity ========== */

static Seq_i64 seq_i64_with_capacity(int64_t cap) {
    size_t n = cap > 0 ? (size_t)cap : 0;
    Seq_i64 s = {(int64_t *)lc_gc_alloc(n * sizeof(int64_t)), 0, n};
    return s;
}
static Seq_bool seq_bool_with_capacity(int64_t cap) {
    size_t n = cap > 0 ? (size_t)cap : 0;
    Seq_bool s = {(bool *)lc_gc_alloc_atomic(n * sizeof(bool)), 0, n};
    return s;
}
static Seq_str seq_str_with_capacity(int64_t cap) {
    size_t n = cap > 0 ? (size_t)cap : 0;
    Seq_str s = {(char **)lc_gc_alloc(n * sizeof(char *)), 0, n};
    return s;
}
static Seq_f64 seq_f64_with_capacity(int64_t cap) {
    size_t n = cap > 0 ? (size_t)cap : 0;
    Seq_f64 s = {(double *)lc_gc_alloc_atomic(n * sizeof(double)), 0, n};
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
    int64_t *keys; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len;
} Set_i64;

static Set_i64 set_i64_new(void) {
    size_t cap = 16;
    Set_i64 s = {(int64_t *)lc_gc_calloc(cap, sizeof(int64_t)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return s;
}

static void _set_i64_resize(Set_i64 *s) {
    int64_t *ok = s->keys; uint8_t *os = s->state; size_t *oo = s->order; size_t ool = s->ord_len;
    size_t nc = s->cap * 2;
    s->keys = (int64_t *)lc_gc_calloc(nc, sizeof(int64_t));
    s->state = (uint8_t *)lc_gc_calloc_atomic(nc, 1);
    s->order = (size_t *)lc_gc_calloc_atomic(nc, sizeof(size_t));
    s->cap = nc; s->len = 0; s->ord_len = 0;
    for (size_t oi = 0; oi < ool; oi++) {
        size_t si = oo[oi];
        if (!os[si]) continue;
        size_t idx = _map_hash(ok[si], nc);
        while (s->state[idx]) idx = (idx + 1) & (nc - 1);
        s->keys[idx] = ok[si]; s->state[idx] = 1; s->len++;
        s->order[s->ord_len++] = idx;
    }
    lc_gc_free(ok); lc_gc_free(os); lc_gc_free(oo);
}

static void set_i64_add(Set_i64 *s, int64_t val) {
    if (s->ord_len * 4 >= s->cap * 3) _set_i64_resize(s);
    size_t idx = _map_hash(val, s->cap);
    while (s->state[idx]) {
        if (s->keys[idx] == val) return;
        idx = (idx + 1) & (s->cap - 1);
    }
    s->keys[idx] = val; s->state[idx] = 1; s->len++;
    s->order[s->ord_len++] = idx;
}

__attribute__((unused)) static char *set_i64_to_string(Set_i64 *s) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < s->ord_len; oi++) {
        size_t si = s->order[oi]; if (!s->state[si]) continue;
        char *e = logos_dyn_sprintf("%s%" PRId64, first ? "" : ", ", s->keys[si]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}
__attribute__((unused)) static void show_set_i64(Set_i64 *s) { char *t = set_i64_to_string(s); printf("%s\n", t); lc_gc_free(t); }
__attribute__((unused)) static char *set_i64_to_string_as(Set_i64 *s, int words) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < s->ord_len; oi++) {
        size_t si = s->order[oi]; if (!s->state[si]) continue;
        char *v = logos_int_text(s->keys[si], words);
        char *e = logos_dyn_sprintf("%s%s", first ? "" : ", ", v); lc_gc_free(v);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}

static bool set_i64_contains(Set_i64 *s, int64_t val) {
    size_t idx = _map_hash(val, s->cap);
    while (s->state[idx]) {
        if (s->keys[idx] == val) return true;
        idx = (idx + 1) & (s->cap - 1);
    }
    return false;
}

static void set_i64_remove(Set_i64 *t, int64_t key) {
    size_t mask = t->cap - 1, i = _map_hash(key, t->cap);
    while (t->state[i] && !(t->keys[i] == key)) i = (i + 1) & mask;
    if (!t->state[i]) return;
    _logos_order_drop(t->order, &t->ord_len, i);
    t->len--;
    for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {
        if (_logos_slot_backs(_map_hash(t->keys[j], t->cap), i, j, mask)) {
            t->keys[i] = t->keys[j];
            _logos_order_move(t->order, t->ord_len, j, i);
            i = j;
        }
    }
    t->keys[i] = 0;
    t->state[i] = 0;
}

static int64_t set_i64_len(Set_i64 *s) { return (int64_t)s->len; }

/* ========== Set: Set_str (open addressing) ========== */

typedef struct {
    char **keys; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len;
} Set_str;

static Set_str set_str_new(void) {
    size_t cap = 16;
    Set_str s = {(char **)lc_gc_calloc(cap, sizeof(char *)),
        (uint8_t *)lc_gc_calloc_atomic(cap, 1), cap, 0,
        (size_t *)lc_gc_calloc_atomic(cap, sizeof(size_t)), 0};
    return s;
}

static void _set_str_resize(Set_str *s) {
    char **ok = s->keys; uint8_t *os = s->state; size_t *oo = s->order; size_t ool = s->ord_len;
    size_t nc = s->cap * 2;
    s->keys = (char **)lc_gc_calloc(nc, sizeof(char *));
    s->state = (uint8_t *)lc_gc_calloc_atomic(nc, 1);
    s->order = (size_t *)lc_gc_calloc_atomic(nc, sizeof(size_t));
    s->cap = nc; s->len = 0; s->ord_len = 0;
    for (size_t oi = 0; oi < ool; oi++) {
        size_t si = oo[oi];
        if (!os[si]) continue;
        size_t idx = _map_hash_str(ok[si], nc);
        while (s->state[idx]) idx = (idx + 1) & (nc - 1);
        s->keys[idx] = ok[si]; s->state[idx] = 1; s->len++;
        s->order[s->ord_len++] = idx;
    }
    lc_gc_free(ok); lc_gc_free(os); lc_gc_free(oo);
}

static void set_str_add(Set_str *s, const char *val) {
    if (s->ord_len * 4 >= s->cap * 3) _set_str_resize(s);
    size_t idx = _map_hash_str(val, s->cap);
    while (s->state[idx]) {
        if (strcmp(s->keys[idx], val) == 0) return;
        idx = (idx + 1) & (s->cap - 1);
    }
    s->keys[idx] = lc_gc_strdup(val); s->state[idx] = 1; s->len++;
    s->order[s->ord_len++] = idx;
}

__attribute__((unused)) static char *set_str_to_string(Set_str *s) {
    char *out = lc_gc_strdup("{"); int first = 1;
    for (size_t oi = 0; oi < s->ord_len; oi++) {
        size_t si = s->order[oi]; if (!s->state[si]) continue;
        char *e = logos_dyn_sprintf("%s%s", first ? "" : ", ", s->keys[si]);
        char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0;
    }
    char *nw = str_concat(out, "}"); lc_gc_free(out); return nw;
}
__attribute__((unused)) static void show_set_str(Set_str *s) { char *t = set_str_to_string(s); printf("%s\n", t); lc_gc_free(t); }

static bool set_str_contains(Set_str *s, const char *val) {
    size_t idx = _map_hash_str(val, s->cap);
    while (s->state[idx]) {
        if (strcmp(s->keys[idx], val) == 0) return true;
        idx = (idx + 1) & (s->cap - 1);
    }
    return false;
}

static void set_str_remove(Set_str *t, const char * key) {
    size_t mask = t->cap - 1, i = _map_hash_str(key, t->cap);
    while (t->state[i] && !(strcmp(t->keys[i], key) == 0)) i = (i + 1) & mask;
    if (!t->state[i]) return;
    _logos_order_drop(t->order, &t->ord_len, i);
    t->len--;
    for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {
        if (_logos_slot_backs(_map_hash_str(t->keys[j], t->cap), i, j, mask)) {
            t->keys[i] = t->keys[j];
            _logos_order_move(t->order, t->ord_len, j, i);
            i = j;
        }
    }
    t->keys[i] = 0;
    t->state[i] = 0;
}

static int64_t set_str_len(Set_str *s) { return (int64_t)s->len; }

/* Set union / intersection — insertion-order-preserving (`add` dedups): union keeps the left set's order then
   the right set's new elements; intersection keeps the left set's order of elements also in the right. ≡ VM. */
static Set_i64 set_i64_union(Set_i64 *a, Set_i64 *b) {
    Set_i64 r = set_i64_new();
    for (size_t oi = 0; oi < a->ord_len; oi++) { size_t si = a->order[oi]; if (a->state[si]) set_i64_add(&r, a->keys[si]); }
    for (size_t oi = 0; oi < b->ord_len; oi++) { size_t si = b->order[oi]; if (b->state[si]) set_i64_add(&r, b->keys[si]); }
    return r;
}
static Set_i64 set_i64_intersect(Set_i64 *a, Set_i64 *b) {
    Set_i64 r = set_i64_new();
    for (size_t oi = 0; oi < a->ord_len; oi++) { size_t si = a->order[oi]; if (a->state[si] && set_i64_contains(b, a->keys[si])) set_i64_add(&r, a->keys[si]); }
    return r;
}
static Set_str set_str_union(Set_str *a, Set_str *b) {
    Set_str r = set_str_new();
    for (size_t oi = 0; oi < a->ord_len; oi++) { size_t si = a->order[oi]; if (a->state[si]) set_str_add(&r, a->keys[si]); }
    for (size_t oi = 0; oi < b->ord_len; oi++) { size_t si = b->order[oi]; if (b->state[si]) set_str_add(&r, b->keys[si]); }
    return r;
}
static Set_str set_str_intersect(Set_str *a, Set_str *b) {
    Set_str r = set_str_new();
    for (size_t oi = 0; oi < a->ord_len; oi++) { size_t si = a->order[oi]; if (a->state[si] && set_str_contains(b, a->keys[si])) set_str_add(&r, a->keys[si]); }
    return r;
}

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

#include <errno.h>
/* `readReals(path)` — the file's little-endian binary64 values, eight bytes a value in order, refused as
   `logicaffeine_system::file::read_reals` refuses: a file it cannot read, or one that is not a whole number of
   values. Each value is assembled from its bytes, so the read is the same on a big-endian machine. */
__attribute__((unused)) static Seq_f64 logos_read_reals(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) logos_fatalf("readReals: cannot read '%s': %s", path, strerror(errno));
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size < 0) logos_fatalf("readReals: cannot read '%s': %s", path, strerror(errno));
    if (size % 8 != 0) {
        fclose(f);
        logos_fatalf("readReals: '%s' holds %ld bytes, not a whole number of 8-byte Reals", path, size);
    }
    Seq_f64 out = seq_f64_new();
    unsigned char b[8];
    for (long i = 0; i < size / 8; i++) {
        if (fread(b, 1, 8, f) != 8) logos_fatalf("readReals: cannot read '%s': %s", path, strerror(errno));
        uint64_t bits = 0;
        for (int k = 7; k >= 0; k--) bits = (bits << 8) | b[k];
        double v;
        memcpy(&v, &bits, sizeof v);
        seq_f64_push(&out, v);
    }
    fclose(f);
    return out;
}

/* ========== Temporal tower: Moment/Date/Time (all int64_t), byte-identical to base::temporal.
   Moment = Unix NANOSECONDS; Date = days since 1970-01-01; Time = nanoseconds-of-day. The calendar
   is Howard Hinnant's branch-light days_from_civil/civil_from_days (proleptic Gregorian, no leap
   seconds for calendar projection) — the exact algorithms the interpreter/Rust backend use. ========== */

#define LOGOS_NANOS_PER_SEC   1000000000LL
#define LOGOS_SECS_PER_DAY    86400LL
#define LOGOS_NANOS_PER_DAY   86400000000000LL

/* Floored (Euclidean) division/modulo — match Rust div_euclid/rem_euclid so pre-epoch (negative)
   Moments yield the correct date and a 0..86399 time-of-day, not a negative clock. */
static int64_t logos_div_euclid(int64_t a, int64_t b) {
    int64_t q = a / b;
    if (a % b != 0 && ((a % b < 0) != (b < 0))) q -= 1;
    return q;
}
static int64_t logos_rem_euclid(int64_t a, int64_t b) {
    int64_t m = a % b;
    if (m < 0) m += (b < 0 ? -b : b);
    return m;
}

static bool logos_is_greg_leap(int64_t y) {
    return (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
}
static int64_t logos_last_day_of_month(int64_t y, int64_t mo) {
    switch (mo) {
        case 1: case 3: case 5: case 7: case 8: case 10: case 12: return 31;
        case 4: case 6: case 9: case 11: return 30;
        case 2: return logos_is_greg_leap(y) ? 29 : 28;
        default: return 0;
    }
}
static int64_t logos_days_from_civil(int64_t year, int64_t month, int64_t day) {
    int64_t y = (month <= 2) ? year - 1 : year;
    int64_t era = (y >= 0 ? y : y - 399) / 400;
    int64_t yoe = y - era * 400;
    int64_t doy = (153 * (month > 2 ? month - 3 : month + 9) + 2) / 5 + day - 1;
    int64_t doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}
static void logos_civil_from_days(int64_t z, int64_t *year, int64_t *month, int64_t *day) {
    z += 719468;
    int64_t era = (z >= 0 ? z : z - 146096) / 146097;
    int64_t doe = z - era * 146097;
    int64_t yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int64_t y = yoe + era * 400;
    int64_t doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    int64_t mp = (5 * doy + 2) / 153;
    int64_t d = doy - (153 * mp + 2) / 5 + 1;
    int64_t mo = (mp < 10) ? mp + 3 : mp - 9;
    *year = (mo <= 2) ? y + 1 : y;
    *month = mo;
    *day = d;
}
static int64_t logos_iso_weekday_from_days(int64_t z) { return logos_rem_euclid(z + 3, 7) + 1; }
static int64_t logos_iso_week_from_days(int64_t z) {
    int64_t iso_dow = logos_iso_weekday_from_days(z);
    int64_t thursday = z + (4 - iso_dow);
    int64_t iy, im, id; logos_civil_from_days(thursday, &iy, &im, &id);
    int64_t jan4 = logos_days_from_civil(iy, 1, 4);
    int64_t week1_monday = jan4 - (logos_iso_weekday_from_days(jan4) - 1);
    return (z - week1_monday) / 7 + 1;
}

/* civil components (UTC) of a Moment (Unix ns). */
static void logos_civil_from_nanos(int64_t nanos, int64_t *y, int64_t *mo, int64_t *d,
                                   int64_t *h, int64_t *mi, int64_t *se, int64_t *ns) {
    int64_t total_secs = logos_div_euclid(nanos, LOGOS_NANOS_PER_SEC);
    int64_t days = logos_div_euclid(total_secs, LOGOS_SECS_PER_DAY);
    int64_t day_secs = logos_rem_euclid(total_secs, LOGOS_SECS_PER_DAY);
    *h = day_secs / 3600;
    *mi = (day_secs % 3600) / 60;
    *se = day_secs % 60;
    *ns = logos_rem_euclid(nanos, LOGOS_NANOS_PER_SEC);
    logos_civil_from_days(days, y, mo, d);
}

/* parse_rfc3339: "YYYY-MM-DDTHH:MM:SS[.frac][Z|+-HH:MM]" -> Unix ns. Returns 1 on success. */
static int logos_parse_rfc3339(const char *s0, int64_t *out) {
    const char *s = s0;
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r') s++;
    size_t n = strlen(s);
    while (n > 0 && (s[n-1]==' '||s[n-1]=='\t'||s[n-1]=='\n'||s[n-1]=='\r')) n--;
    if (n < 20) return 0;
    const unsigned char *b = (const unsigned char *)s;
    if (b[4]!='-'||b[7]!='-'||b[10]!='T'||b[13]!=':'||b[16]!=':') return 0;
    int64_t year=0,month=0,day=0,hour=0,minute=0,second=0;
    #define LP_DIG(st,ln,dst) do { int64_t v=0; for (size_t i=(st); i<(st)+(ln); i++) { if (i>=n||b[i]<'0'||b[i]>'9') return 0; v=v*10+(b[i]-'0'); } dst=v; } while(0)
    LP_DIG(0,4,year); LP_DIG(5,2,month); LP_DIG(8,2,day);
    LP_DIG(11,2,hour); LP_DIG(14,2,minute); LP_DIG(17,2,second);
    if (month<1||month>12) return 0;
    int64_t ldm = logos_last_day_of_month(year, month);
    if (day<1||day>ldm) return 0;
    if (hour>23||minute>59||second>59) return 0;
    size_t pos = 19; int64_t nanosecond = 0;
    if (pos<n && b[pos]=='.') {
        pos++; int64_t frac=0; int count=0;
        while (pos<n && b[pos]>='0'&&b[pos]<='9') { if (count<9){frac=frac*10+(b[pos]-'0');count++;} pos++; }
        if (count==0) return 0;
        for (int i=count;i<9;i++) frac*=10;
        nanosecond = frac;
    }
    int64_t offset_seconds;
    if (pos<n && b[pos]=='Z') { pos++; offset_seconds=0; }
    else if (pos<n && (b[pos]=='+'||b[pos]=='-')) {
        int64_t sign=(b[pos]=='+')?1:-1, oh=0, om=0;
        LP_DIG(pos+1,2,oh);
        if (pos+3>=n || b[pos+3]!=':') return 0;
        LP_DIG(pos+4,2,om);
        if (oh>23||om>59) return 0;
        pos+=6; offset_seconds=sign*(oh*3600+om*60);
    } else return 0;
    if (pos != n) return 0;
    int64_t days = logos_days_from_civil(year, month, day);
    int64_t tod = hour*3600 + minute*60 + second;
    *out = days*LOGOS_NANOS_PER_DAY + tod*LOGOS_NANOS_PER_SEC + nanosecond - offset_seconds*LOGOS_NANOS_PER_SEC;
    #undef LP_DIG
    return 1;
}
static int64_t logos_parse_timestamp(const char *s) {
    int64_t out;
    if (!logos_parse_rfc3339(s, &out)) {
        logos_fatalf("Cannot parse '%s' as an RFC 3339 timestamp", s);
    }
    return out;
}

/* Moment component extractors. */
static int64_t logos_year_of(int64_t m)   { int64_t y,mo,d,h,mi,s,n; logos_civil_from_nanos(m,&y,&mo,&d,&h,&mi,&s,&n); return y; }
static int64_t logos_month_of(int64_t m)  { int64_t y,mo,d,h,mi,s,n; logos_civil_from_nanos(m,&y,&mo,&d,&h,&mi,&s,&n); return mo; }
static int64_t logos_day_of(int64_t m)    { int64_t y,mo,d,h,mi,s,n; logos_civil_from_nanos(m,&y,&mo,&d,&h,&mi,&s,&n); return d; }
static int64_t logos_hour_of(int64_t m)   { int64_t y,mo,d,h,mi,s,n; logos_civil_from_nanos(m,&y,&mo,&d,&h,&mi,&s,&n); return h; }
static int64_t logos_minute_of(int64_t m) { int64_t y,mo,d,h,mi,s,n; logos_civil_from_nanos(m,&y,&mo,&d,&h,&mi,&s,&n); return mi; }
static int64_t logos_second_of(int64_t m) { int64_t y,mo,d,h,mi,s,n; logos_civil_from_nanos(m,&y,&mo,&d,&h,&mi,&s,&n); return s; }
static int64_t logos_weekday_of(int64_t m){ return logos_iso_weekday_from_days(logos_div_euclid(m, LOGOS_NANOS_PER_DAY)); }
static int64_t logos_week_of(int64_t m)   { return logos_iso_week_from_days(logos_div_euclid(m, LOGOS_NANOS_PER_DAY)); }
static int64_t logos_quarter_of(int64_t m){ return (logos_month_of(m) - 1) / 3 + 1; }
static int64_t logos_date_of(int64_t m)   { return logos_div_euclid(m, LOGOS_NANOS_PER_DAY); }
static int64_t logos_time_of(int64_t m)   { return logos_rem_euclid(m, LOGOS_NANOS_PER_DAY); }
static int64_t logos_seconds_between(int64_t a, int64_t b) { return (b - a) / LOGOS_NANOS_PER_SEC; }
static int64_t logos_add_seconds(int64_t m, int64_t s)     { return m + s * LOGOS_NANOS_PER_SEC; }
/* Civil-component constructors — the C mirrors of makeDate/makeMoment (same days_from_civil core as timegm). */
static int64_t logos_make_date(int64_t y, int64_t mo, int64_t d) { return logos_days_from_civil(y, mo, d); }
static int64_t logos_make_moment(int64_t y, int64_t mo, int64_t d, int64_t h, int64_t mi, int64_t s) {
    return logos_days_from_civil(y, mo, d) * LOGOS_NANOS_PER_DAY + (h * 3600 + mi * 60 + s) * LOGOS_NANOS_PER_SEC;
}

/* RFC-3339 string of a Moment (for format_timestamp). */
static char *logos_format_rfc3339(int64_t nanos) {
    int64_t y,mo,d,h,mi,s,ns; logos_civil_from_nanos(nanos,&y,&mo,&d,&h,&mi,&s,&ns);
    char base[48];
    snprintf(base, 48, "%04" PRId64 "-%02" PRId64 "-%02" PRId64 "T%02" PRId64 ":%02" PRId64 ":%02" PRId64, y,mo,d,h,mi,s);
    if (ns != 0) {
        char frac[16]; snprintf(frac, 16, "%09" PRId64, ns);
        int len=(int)strlen(frac); while (len>0 && frac[len-1]=='0') frac[--len]='\0';
        return logos_dyn_sprintf("%s.%sZ", base, frac);
    }
    return logos_dyn_sprintf("%sZ", base);
}
static char *logos_format_time_of_day(int64_t nanos_from_midnight) {
    int64_t rem = logos_rem_euclid(nanos_from_midnight, LOGOS_NANOS_PER_DAY);
    int64_t secs = rem / LOGOS_NANOS_PER_SEC;
    char base[16]; snprintf(base, 16, "%02" PRId64 ":%02" PRId64 ":%02" PRId64, secs/3600, (secs%3600)/60, secs%60);
    int64_t nanos = rem % LOGOS_NANOS_PER_SEC;
    if (nanos != 0) {
        char frac[16]; snprintf(frac, 16, "%09" PRId64, nanos);
        int len=(int)strlen(frac); while (len>0 && frac[len-1]=='0') frac[--len]='\0';
        return logos_dyn_sprintf("%s.%s", base, frac);
    }
    return lc_gc_strdup(base);
}

/* ========== Calendar spans and durations. A Span is (months, days); a Duration is signed nanoseconds.
   `Date + Span` and `Moment + Span` apply the months first, clamped to the end of the month, then the days
   — base::temporal::{date_add_span, add_span}, the arithmetic every tier shares. ========== */
typedef struct { int32_t months; int32_t days; } LogosSpan;
static LogosSpan logos_span(int32_t months, int32_t days) { LogosSpan s; s.months = months; s.days = days; return s; }

/* Two's-complement wrapping, as Int span and Duration arithmetic wrap on every tier. */
static int32_t logos_wrap_add32(int32_t a, int32_t b) { return (int32_t)((uint32_t)a + (uint32_t)b); }
static int32_t logos_wrap_mul32(int32_t a, int32_t b) { return (int32_t)((uint32_t)a * (uint32_t)b); }
static int32_t logos_wrap_neg32(int32_t a) { return (int32_t)(0u - (uint32_t)a); }
static int64_t logos_wrap_add64(int64_t a, int64_t b) { return (int64_t)((uint64_t)a + (uint64_t)b); }
static int64_t logos_wrap_sub64(int64_t a, int64_t b) { return (int64_t)((uint64_t)a - (uint64_t)b); }

/* base::temporal::date_add_span, step for step (a Date is days since 1970-01-01). */
static int64_t logos_date_add_span(int64_t date, int32_t months, int32_t days) {
    int32_t z = (int32_t)date + 719468;
    int32_t era = z >= 0 ? z / 146097 : (z - 146096) / 146097;
    uint32_t doe = (uint32_t)(z - era * 146097);
    uint32_t yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int32_t y = (int32_t)yoe + era * 400;
    uint32_t doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    uint32_t mp = (5 * doy + 2) / 153;
    uint32_t d = doy - (153 * mp + 2) / 5 + 1;
    uint32_t m = mp < 10 ? mp + 3 : mp - 9;
    int32_t year = y + (m <= 2 ? 1 : 0);
    int32_t month = (int32_t)m;
    int32_t day = (int32_t)d;
    int32_t total_months = logos_wrap_add32(logos_wrap_add32(logos_wrap_mul32(year, 12), month - 1), months);
    year = total_months / 12;
    month = total_months % 12 + 1;
    if (month <= 0) { month += 12; year -= 1; }
    int32_t last = (int32_t)logos_last_day_of_month(year, month);
    if (day > last) day = last;
    int32_t yp = year - (month <= 2 ? 1 : 0);
    int32_t era2 = yp >= 0 ? yp / 400 : (yp - 399) / 400;
    uint32_t yoe2 = (uint32_t)(yp - era2 * 400);
    uint32_t mp2 = month > 2 ? (uint32_t)month - 3 : (uint32_t)month + 9;
    uint32_t doy2 = (153 * mp2 + 2) / 5 + (uint32_t)day - 1;
    uint32_t doe2 = yoe2 * 365 + yoe2 / 4 - yoe2 / 100 + doy2;
    int32_t result = era2 * 146097 + (int32_t)doe2 - 719468;
    return (int64_t)logos_wrap_add32(result, days);
}

/* base::temporal::add_span on the Moment's civil UTC date-time: the time of day rides along. */
static int64_t logos_moment_add_span(int64_t nanos, int32_t months, int32_t days) {
    int64_t y, mo, d, h, mi, s, ns;
    logos_civil_from_nanos(nanos, &y, &mo, &d, &h, &mi, &s, &ns);
    int64_t total = y * 12 + (mo - 1) + (int64_t)months;
    int64_t ny = logos_div_euclid(total, 12);
    int64_t nmo = logos_rem_euclid(total, 12) + 1;
    int64_t last = logos_last_day_of_month(ny, nmo);
    int64_t z = logos_days_from_civil(ny, nmo, d < last ? d : last) + (int64_t)days;
    return z * LOGOS_NANOS_PER_DAY + (h * 3600 + mi * 60 + s) * LOGOS_NANOS_PER_SEC + ns;
}

/* Text of each temporal value, byte-identical to the interpreter's display: Date "YYYY-MM-DD", Moment
   "YYYY-MM-DD HH:MM", Time "HH:MM:SS[.frac]", Duration in its largest whole unit (`90s` is `1min`),
   Span "N years and N months and N days" (zero parts omitted, `0 days` when all are zero). */
static char *logos_date_to_string(int64_t days) {
    int64_t y,mo,d; logos_civil_from_days(days,&y,&mo,&d);
    return logos_dyn_sprintf("%04" PRId64 "-%02" PRId64 "-%02" PRId64, y, mo, d);
}
static char *logos_moment_to_string(int64_t nanos) {
    int64_t total_secs = logos_div_euclid(nanos, LOGOS_NANOS_PER_SEC);
    int64_t days = logos_div_euclid(total_secs, LOGOS_SECS_PER_DAY);
    int64_t day_secs = logos_rem_euclid(total_secs, LOGOS_SECS_PER_DAY);
    int64_t y,mo,d; logos_civil_from_days(days,&y,&mo,&d);
    return logos_dyn_sprintf("%04" PRId64 "-%02" PRId64 "-%02" PRId64 " %02" PRId64 ":%02" PRId64,
                             y, mo, d, day_secs / 3600, (day_secs % 3600) / 60);
}
static char *logos_duration_to_string(int64_t nanos) {
    uint64_t a = nanos < 0 ? (uint64_t)0 - (uint64_t)nanos : (uint64_t)nanos;
    const char *sign = nanos < 0 ? "-" : "";
    if (a >= 3600000000000ULL) return logos_dyn_sprintf("%s%" PRIu64 "h", sign, a / 3600000000000ULL);
    if (a >= 60000000000ULL) return logos_dyn_sprintf("%s%" PRIu64 "min", sign, a / 60000000000ULL);
    if (a >= 1000000000ULL) return logos_dyn_sprintf("%s%" PRIu64 "s", sign, a / 1000000000ULL);
    if (a >= 1000000ULL) return logos_dyn_sprintf("%s%" PRIu64 "ms", sign, a / 1000000ULL);
    if (a >= 1000ULL) return logos_dyn_sprintf("%s%" PRIu64 "\xce\xbcs", sign, a / 1000ULL);
    return logos_dyn_sprintf("%s%" PRIu64 "ns", sign, a);
}
static char *logos_span_to_string(LogosSpan sp) {
    char *parts[3];
    int n = 0;
    int32_t years = sp.months / 12, months = sp.months % 12;
    if (years != 0) parts[n++] = logos_dyn_sprintf("%d %s", years, (years == 1 || years == -1) ? "year" : "years");
    if (months != 0) parts[n++] = logos_dyn_sprintf("%d %s", months, (months == 1 || months == -1) ? "month" : "months");
    if (sp.days != 0 || n == 0) parts[n++] = logos_dyn_sprintf("%d %s", sp.days, (sp.days == 1 || sp.days == -1) ? "day" : "days");
    size_t len = 1;
    for (int i = 0; i < n; i++) len += strlen(parts[i]) + 5;
    char *out = (char *)lc_gc_alloc_atomic(len);
    out[0] = '\0';
    for (int i = 0; i < n; i++) {
        if (i > 0) strcat(out, " and ");
        strcat(out, parts[i]);
        lc_gc_free(parts[i]);
    }
    return out;
}
static bool logos_span_equals(LogosSpan a, LogosSpan b) { return a.months == b.months && a.days == b.days; }

static void logos_show_text(char *s) { printf("%s\n", s); lc_gc_free(s); }
static void show_moment(int64_t nanos) { logos_show_text(logos_moment_to_string(nanos)); }
static void show_date(int64_t days) { logos_show_text(logos_date_to_string(days)); }
static void show_time(int64_t nanos_of_day) { logos_show_text(logos_format_time_of_day(nanos_of_day)); }
static void show_duration(int64_t nanos) { logos_show_text(logos_duration_to_string(nanos)); }
static void show_span(LogosSpan sp) { logos_show_text(logos_span_to_string(sp)); }

/* `today` / `now` — the injected fixed clock (env `LOGOS_FIXED_DAYS`/`LOGOS_FIXED_NANOS`, set by the
   differential harness so the AOT binary and the VM oracle read the SAME instant) else the real clock.
   Byte-for-byte the RISC-V shim (`codegen_riscv/runtime.rs`), so the two AOT tiers cannot diverge. */
static int64_t logos_today(void) {
    const char *e = getenv("LOGOS_FIXED_DAYS");
    if (e) return (int64_t)atoll(e);
    return (int64_t)(time(NULL) / 86400);
}
static int64_t logos_now(void) {
    const char *e = getenv("LOGOS_FIXED_NANOS");
    if (e) return (int64_t)atoll(e);
    return (int64_t)time(NULL) * 1000000000LL;
}
/* time.lg's `now()` — the Unix time in milliseconds from the real clock, as the interpreter reads it. */
__attribute__((unused)) static int64_t logos_time_now(void) {
    struct timespec ts;
    timespec_get(&ts, TIME_UTC);
    return (int64_t)ts.tv_sec * 1000 + (int64_t)(ts.tv_nsec / 1000000);
}

"#,
    exact_numbers_source!(),
    r#"
typedef struct { LBig num, den; } LRat; /* den > 0, gcd(|num|,den)==1; zero == 0/1 */
__attribute__((unused)) static LRat rat_reduce(LBig num, LBig den) {
    if (den.sign < 0) { num.sign = -num.sign; den.sign = 1; }
    if (num.len == 0) { lbig_free(&num); lbig_free(&den); LRat z = { lbig_from_i64(0), lbig_from_i64(1) }; return z; }
    LBig g = lbig_gcd(&num, &den);
    LBig one = lbig_from_i64(1);
    if (lbig_cmp_mag(&g, &one) != 0) { LBig nn = lbig_divexact(&num, &g); nn.sign = num.sign; LBig nd = lbig_divexact(&den, &g); nd.sign = 1;
        lbig_free(&num); lbig_free(&den); num = nn; den = nd; }
    lbig_free(&one); lbig_free(&g);
    LRat r = { num, den }; return r;
}
__attribute__((unused)) static LRat rat_from_i64(int64_t v) { LRat r = { lbig_from_i64(v), lbig_from_i64(1) }; return r; }
/* Exact dyadic rational for an f64: d = mant·2^(e-53) with mant an exact 53-bit integer, so the
   rational round-trips (rat_to_f64) back to the identical double. This is the C mirror of the
   AOT-Rust LogosComplexF path — `complex(realExpr, realExpr)` on every native tier. */
__attribute__((unused)) static LRat rat_from_f64(double d) {
    if (d == 0.0) { LRat z = { lbig_from_i64(0), lbig_from_i64(1) }; return z; }
    int neg = d < 0.0; double a = neg ? -d : d;
    int e; double m = frexp(a, &e);                       /* a = m·2^e, m in [0.5,1) */
    int64_t mant = (int64_t)(m * 9007199254740992.0);     /* m·2^53 — an exact integer < 2^53 */
    int shift = e - 53;
    LBig num = lbig_from_i64(mant), den = lbig_from_i64(1), two = lbig_from_i64(2);
    if (shift >= 0) {
        for (int i = 0; i < shift; i++) { LBig t = lbig_mul(&num, &two); lbig_free(&num); num = t; }
    } else {
        for (int i = 0; i < -shift; i++) { LBig t = lbig_mul(&den, &two); lbig_free(&den); den = t; }
    }
    lbig_free(&two);
    if (neg) num.sign = -num.sign;
    return rat_reduce(num, den);
}
__attribute__((unused)) static LRat rat_add(const LRat *a, const LRat *b) { /* (an*bd + bn*ad)/(ad*bd) */
    LBig t1 = lbig_mul(&a->num, &b->den), t2 = lbig_mul(&b->num, &a->den), n = lbig_add(&t1, &t2), d = lbig_mul(&a->den, &b->den);
    lbig_free(&t1); lbig_free(&t2); return rat_reduce(n, d);
}
__attribute__((unused)) static LRat rat_neg(const LRat *a) { LBig n = lbig_copy(&a->num); n.sign = -n.sign; LBig d = lbig_copy(&a->den); LRat r = { n, d }; return r; }
__attribute__((unused)) static LRat rat_sub(const LRat *a, const LRat *b) { LRat nb = rat_neg(b); LRat r = rat_add(a, &nb); lbig_free(&nb.num); lbig_free(&nb.den); return r; }
__attribute__((unused)) static LRat rat_mul(const LRat *a, const LRat *b) { LBig n = lbig_mul(&a->num, &b->num), d = lbig_mul(&a->den, &b->den); return rat_reduce(n, d); }
__attribute__((unused)) static LRat rat_div(const LRat *a, const LRat *b) { LBig n = lbig_mul(&a->num, &b->den), d = lbig_mul(&a->den, &b->num); return rat_reduce(n, d); }
__attribute__((unused)) static int rat_is_zero(const LRat *a) { return a->num.len == 0; }
__attribute__((unused)) static int rat_is_neg(const LRat *a) { return a->num.sign < 0; }
__attribute__((unused)) static int rat_is_one(const LRat *a) { LBig o = lbig_from_i64(1); int r = (lbig_cmp(&a->num, &o) == 0) && (lbig_cmp(&a->den, &o) == 0); lbig_free(&o); return r; }
__attribute__((unused)) static double rat_to_f64(const LRat *a) { return lbig_to_f64(&a->num) / lbig_to_f64(&a->den); }
__attribute__((unused)) static char *rat_to_string(const LRat *a) { /* "n" if integer else "n/d" */
    LBig one = lbig_from_i64(1); int integer = (lbig_cmp_mag(&a->den, &one) == 0); lbig_free(&one);
    char *ns = lbig_to_string(&a->num);
    if (integer) return ns;
    char *ds = lbig_to_string(&a->den); char *buf = (char *)lc_gc_alloc_atomic(strlen(ns) + strlen(ds) + 2);
    sprintf(buf, "%s/%s", ns, ds); lc_gc_free(ns); lc_gc_free(ds); return buf;
}
__attribute__((unused)) static LRat rat_abs(const LRat *a) { LBig n = lbig_copy(&a->num); if (n.len) n.sign = 1; LBig d = lbig_copy(&a->den); LRat r = { n, d }; return r; }

typedef struct { LRat re, im; } LComplex;
/* The codegen-facing complex ops take/return BY VALUE so generated code composes rvalues naturally
   (`cplx_add(cplx_ints(1,2), cplx_ints(3,4))`), where taking `&` of a temporary would be illegal C. */
__attribute__((unused)) static char *lstrdup(const char *s) { char *r = (char *)lc_gc_alloc_atomic(strlen(s) + 1); strcpy(r, s); return r; }
__attribute__((unused)) static LComplex cplx_ints(int64_t re, int64_t im) { LComplex z = { rat_from_i64(re), rat_from_i64(im) }; return z; }
/* Floating-point complex constructor — `complex(realExpr, realExpr)`; parts held as exact dyadic
   rationals so re/im round-trip and arithmetic composes with the exact tower. */
__attribute__((unused)) static LComplex cplx_reals(double re, double im) { LComplex z = { rat_from_f64(re), rat_from_f64(im) }; return z; }
/* Real / imaginary part of a complex BY VALUE (taking `&temp.re` on an rvalue would be illegal C). */
__attribute__((unused)) static double cplx_re_f64(LComplex a) { return rat_to_f64(&a.re); }
__attribute__((unused)) static double cplx_im_f64(LComplex a) { return rat_to_f64(&a.im); }
__attribute__((unused)) static LComplex cplx_add(LComplex a, LComplex b) { LComplex z = { rat_add(&a.re, &b.re), rat_add(&a.im, &b.im) }; return z; }
__attribute__((unused)) static LComplex cplx_sub(LComplex a, LComplex b) { LComplex z = { rat_sub(&a.re, &b.re), rat_sub(&a.im, &b.im) }; return z; }
__attribute__((unused)) static LComplex cplx_mul(LComplex a, LComplex b) { /* (ac-bd) + (ad+bc)i */
    LRat ac = rat_mul(&a.re, &b.re), bd = rat_mul(&a.im, &b.im), ad = rat_mul(&a.re, &b.im), bc = rat_mul(&a.im, &b.re);
    LComplex z = { rat_sub(&ac, &bd), rat_add(&ad, &bc) };
    lbig_free(&ac.num); lbig_free(&ac.den); lbig_free(&bd.num); lbig_free(&bd.den); lbig_free(&ad.num); lbig_free(&ad.den); lbig_free(&bc.num); lbig_free(&bc.den);
    return z;
}
__attribute__((unused)) static LComplex cplx_div(LComplex a, LComplex b) { /* a*conj(b) / |b|^2 */
    LRat cc = rat_mul(&b.re, &b.re), dd = rat_mul(&b.im, &b.im), den = rat_add(&cc, &dd);
    LRat ac = rat_mul(&a.re, &b.re), bd = rat_mul(&a.im, &b.im), nre = rat_add(&ac, &bd);
    LRat bc = rat_mul(&a.im, &b.re), ad = rat_mul(&a.re, &b.im), nim = rat_sub(&bc, &ad);
    LComplex z = { rat_div(&nre, &den), rat_div(&nim, &den) };
    LRat tmp[] = { cc, dd, den, ac, bd, nre, bc, ad, nim };
    for (size_t i = 0; i < sizeof(tmp)/sizeof(tmp[0]); i++) { lbig_free(&tmp[i].num); lbig_free(&tmp[i].den); }
    return z;
}
__attribute__((unused)) static LComplex cplx_conj(LComplex a) { /* re unchanged, im negated */
    LRat re = { lbig_copy(&a.re.num), lbig_copy(&a.re.den) };
    LComplex z = { re, rat_neg(&a.im) }; return z;
}
__attribute__((unused)) static double cplx_abs_f64(LComplex a) { double re = rat_to_f64(&a.re), im = rat_to_f64(&a.im); return sqrt(re * re + im * im); }
__attribute__((unused)) static char *cplx_to_string(LComplex a) { /* matches interp Complex Display */
    if (rat_is_zero(&a.im)) return rat_to_string(&a.re);
    LRat mag = rat_abs(&a.im); int unit = rat_is_one(&mag);
    char *out;
    if (rat_is_zero(&a.re)) {
        if (rat_is_neg(&a.im)) { if (unit) { out = lstrdup("-i"); } else { char *m = rat_to_string(&mag); out = (char *)lc_gc_alloc_atomic(strlen(m) + 3); sprintf(out, "-%si", m); lc_gc_free(m); } }
        else if (unit) { out = lstrdup("i"); }
        else { char *m = rat_to_string(&mag); out = (char *)lc_gc_alloc_atomic(strlen(m) + 2); sprintf(out, "%si", m); lc_gc_free(m); }
    } else {
        char *rs = rat_to_string(&a.re); const char *sign = rat_is_neg(&a.im) ? "-" : "+";
        if (unit) { out = (char *)lc_gc_alloc_atomic(strlen(rs) + 3); sprintf(out, "%s%si", rs, sign); }
        else { char *m = rat_to_string(&mag); out = (char *)lc_gc_alloc_atomic(strlen(rs) + strlen(m) + 3); sprintf(out, "%s%s%si", rs, sign, m); lc_gc_free(m); }
        lc_gc_free(rs);
    }
    lbig_free(&mag.num); lbig_free(&mag.den); return out;
}
__attribute__((unused)) static void show_complex(LComplex a) { char *s = cplx_to_string(a); printf("%s\n", s); lc_gc_free(s); }

/* `decimal` is LOGOS's exact rational (Display "12" / "1/3") — the C mirror is LRat. These by-value
   wrappers let generated code compose rvalues (`dec_add(dec_from_i64(1), dec_from_i64(3))`). */
__attribute__((unused)) static LRat dec_from_i64(int64_t v) { return rat_from_i64(v); }
__attribute__((unused)) static LRat dec_add(LRat a, LRat b) { return rat_add(&a, &b); }
__attribute__((unused)) static LRat dec_sub(LRat a, LRat b) { return rat_sub(&a, &b); }
__attribute__((unused)) static LRat dec_mul(LRat a, LRat b) { return rat_mul(&a, &b); }
__attribute__((unused)) static LRat dec_div(LRat a, LRat b) { return rat_div(&a, &b); }
__attribute__((unused)) static double dec_to_f64(LRat a) { return rat_to_f64(&a); }
__attribute__((unused)) static void show_dec(LRat a) { char *s = rat_to_string(&a); printf("%s\n", s); lc_gc_free(s); }

__attribute__((unused)) static LDec ldec_from_text(const char *s) {
    const char *p = s; int neg = 0;
    while (*p == ' ' || *p == '\t') p++;
    if (*p == '-') { neg = 1; p++; } else if (*p == '+') p++;
    LBig a = lbig_from_i64(0), ten = lbig_from_i64(10);
    int scale = 0, seen_dot = 0, any = 0;
    for (; *p; p++) {
        if (*p == '.') { if (seen_dot) goto bad; seen_dot = 1; continue; }
        if (*p < '0' || *p > '9') goto bad;
        any = 1;
        LBig t = lbig_mul(&a, &ten); lbig_free(&a);
        LBig dg = lbig_from_i64(*p - '0'); a = lbig_add(&t, &dg); lbig_free(&t); lbig_free(&dg);
        if (seen_dot) scale++;
    }
    lbig_free(&ten);
    if (!any) { lbig_free(&a); logos_fatalf("invalid decimal: %s", s); }
    if (neg && a.len) a.sign = -1;
    { LDec d; d.is_rat = 0; d.a = a; d.b = lbig_from_i64(0); d.scale = scale; return d; }
bad: lbig_free(&ten); lbig_free(&a); logos_fatalf("invalid decimal: %s", s);
}
/* ===== Money — the C mirror of `logicaffeine_base::Money`: an exact `LDec` amount quantised to its
   ISO-4217 currency's minor unit. `+ −` combine same-currency amounts; `× k` scales; `÷` of two
   same-currency amounts is a dimensionless ratio. Display is `"{amount at scale} {CODE}"`. */
typedef struct { LDec amount; char code[8]; int scale; } LMoney;
/* Uppercase+trim `raw` into `code_out`, verify it is a known ISO-4217 code, return its minor-unit scale. */
__attribute__((unused)) static int currency_lookup(const char *raw, char *code_out) {
    int n = 0; const char *p = raw; while (*p == ' ' || *p == '\t') p++;
    for (; *p && n < 7; p++) { char ch = *p; if (ch == ' ' || ch == '\t') break; code_out[n++] = (ch >= 'a' && ch <= 'z') ? (char)(ch - 32) : ch; }
    code_out[n] = 0;
    static const char *S0[] = {"JPY","KRW","ISK","VND","CLP",0};
    static const char *S3[] = {"BHD","KWD","OMR","JOD","TND",0};
    static const char *S2[] = {"USD","EUR","GBP","CHF","CAD","AUD","NZD","SGD","HKD","CNY","INR","BRL","MXN","ZAR","SEK","NOK","DKK","RUB","TRY","PLN",0};
    for (int i = 0; S0[i]; i++) if (!strcmp(code_out, S0[i])) return 0;
    for (int i = 0; S3[i]; i++) if (!strcmp(code_out, S3[i])) return 3;
    for (int i = 0; S2[i]; i++) if (!strcmp(code_out, S2[i])) return 2;
    logos_fatalf("unknown currency: %s", raw);
}
__attribute__((unused)) static LMoney money_of(LDec amount, const char *code) {
    LMoney m; m.scale = currency_lookup(code, m.code); m.amount = ldec_rescale(amount, m.scale); return m;
}
__attribute__((unused)) static LMoney money_add(LMoney a, LMoney b) {
    LMoney m; m.amount = ldec_add(a.amount, b.amount); m.scale = a.scale; strcpy(m.code, a.code); return m;
}
__attribute__((unused)) static LMoney money_sub(LMoney a, LMoney b) {
    LMoney m; m.amount = ldec_sub(a.amount, b.amount); m.scale = a.scale; strcpy(m.code, a.code); return m;
}
/* `money × k` — scale the amount by a plain integer, re-quantised to the currency's minor unit. */
__attribute__((unused)) static LMoney money_scale(LMoney a, int64_t k) {
    LDec kd = ldec_from_i64(k), prod = ldec_mul(a.amount, kd);
    LMoney m; m.scale = a.scale; strcpy(m.code, a.code); m.amount = ldec_rescale(prod, a.scale); return m;
}
/* `money ÷ money` (same currency) — the exact dimensionless ratio, an `LDec`. */
__attribute__((unused)) static LDec money_ratio(LMoney a, LMoney b) { return ldec_div(a.amount, b.amount); }
__attribute__((unused)) static char *money_to_string(LMoney m) {
    char *a = ldec_to_string(&m.amount); char *out = (char *) lc_gc_alloc_atomic(strlen(a) + strlen(m.code) + 2);
    sprintf(out, "%s %s", a, m.code); lc_gc_free(a); return out;
}
__attribute__((unused)) static void show_money(LMoney m) { char *s = money_to_string(m); printf("%s\n", s); lc_gc_free(s); }

/* ===== Quantity — the C mirror of `logicaffeine_base::Quantity` + its display `Unit`. An exact
   `LRat` magnitude IN THE SI BASE UNIT, a 10-axis dimension vector of rational exponents, and a
   display unit (`scale`, `offset`, `symbol`). Units resolve at CODEGEN time (the base `units::by_name`),
   so the runtime carries only the dimensional ALGEBRA. Display: the magnitude re-expressed in the
   carried unit + its symbol (`5 m`, `2 in`, `6250/381 ft`); a synthetic SI unit (empty symbol, from a
   dimension-combining `× ÷`) shows the dimension SIGNATURE instead (`6 L^2`, dimensionless `1`). */
typedef struct { LRat mag; int dnum[10]; int dden[10]; LRat uscale; LRat uoffset; char usym[24]; } LQuant;
/* the base-dimension symbols, in canonical order (L M T I Θ N J rad sr bit). */
static const char *QDIM_SYM[10] = {"L", "M", "T", "I", "\xce\x98", "N", "J", "rad", "sr", "bit"};
__attribute__((unused)) static LRat rat_ratio(int64_t n, int64_t d) { return rat_reduce(lbig_from_i64(n), lbig_from_i64(d)); }
__attribute__((unused)) static int qexp_igcd(int a, int b) { a = a < 0 ? -a : a; b = b < 0 ? -b : b; while (b) { int t = b; b = a % b; a = t; } return a ? a : 1; }
__attribute__((unused)) static void qexp_reduce(int *n, int *d) { if (*d < 0) { *n = -*n; *d = -*d; } int g = qexp_igcd(*n, *d); if (g) { *n /= g; *d /= g; } if (*d == 0) *d = 1; }
/* build a Quantity from a magnitude-in-`unit`, the unit's exact scale/offset, its dimension, symbol. */
__attribute__((unused)) static LQuant quant_of(LRat value, LRat scale, LRat offset, const int *dnum, const int *dden, const char *sym) {
    LQuant q; LRat vs = rat_mul(&value, &scale); q.mag = rat_add(&vs, &offset); lbig_free(&vs.num); lbig_free(&vs.den);
    for (int i = 0; i < 10; i++) { q.dnum[i] = dnum[i]; q.dden[i] = dden[i]; }
    q.uscale = scale; q.uoffset = offset; strncpy(q.usym, sym, 23); q.usym[23] = 0; return q;
}
/* the magnitude expressed IN the carried display unit: `(si − offset) / scale`. */
__attribute__((unused)) static LRat quant_in_unit(const LQuant *q) {
    LRat d = rat_sub(&q->mag, &q->uoffset), r = rat_div(&d, &q->uscale); lbig_free(&d.num); lbig_free(&d.den); return r;
}
__attribute__((unused)) static int quant_same_dim(const LQuant *a, const int *dnum, const int *dden) {
    for (int i = 0; i < 10; i++) if (a->dnum[i] != dnum[i] || a->dden[i] != dden[i]) return 0; return 1;
}
__attribute__((unused)) static LQuant quant_add(LQuant a, LQuant b) {
    if (!quant_same_dim(&a, b.dnum, b.dden)) logos_fatal("cannot add quantities of different dimensions");
    LQuant r = a; r.mag = rat_add(&a.mag, &b.mag); return r;
}
__attribute__((unused)) static LQuant quant_sub(LQuant a, LQuant b) {
    if (!quant_same_dim(&a, b.dnum, b.dden)) logos_fatal("cannot subtract quantities of different dimensions");
    LQuant r = a; r.mag = rat_sub(&a.mag, &b.mag); return r;
}
/* `× ÷` of two quantities combine dimensions (axiswise exponent add/sub) and reset to a SYNTHETIC
   SI unit (empty symbol, scale 1, offset 0) → Display shows the dimension signature. */
__attribute__((unused)) static LQuant quant_mul(LQuant a, LQuant b) {
    LQuant r; r.mag = rat_mul(&a.mag, &b.mag);
    for (int i = 0; i < 10; i++) { int nn = a.dnum[i] * b.dden[i] + b.dnum[i] * a.dden[i], dd = a.dden[i] * b.dden[i]; qexp_reduce(&nn, &dd); r.dnum[i] = nn; r.dden[i] = dd; }
    r.uscale = rat_ratio(1, 1); r.uoffset = rat_ratio(0, 1); r.usym[0] = 0; return r;
}
__attribute__((unused)) static LQuant quant_div(LQuant a, LQuant b) {
    LQuant r; r.mag = rat_div(&a.mag, &b.mag);
    for (int i = 0; i < 10; i++) { int nn = a.dnum[i] * b.dden[i] - b.dnum[i] * a.dden[i], dd = a.dden[i] * b.dden[i]; qexp_reduce(&nn, &dd); r.dnum[i] = nn; r.dden[i] = dd; }
    r.uscale = rat_ratio(1, 1); r.uoffset = rat_ratio(0, 1); r.usym[0] = 0; return r;
}
/* scale a quantity by a dimensionless scalar, preserving its unit. */
__attribute__((unused)) static LQuant quant_scale_mul(LQuant a, LRat k) { LQuant r = a; r.mag = rat_mul(&a.mag, &k); return r; }
__attribute__((unused)) static LQuant quant_scale_div(LQuant a, LRat k) { LQuant r = a; r.mag = rat_div(&a.mag, &k); return r; }
/* re-express a quantity in another unit of the SAME dimension (`convert`/`in`). */
__attribute__((unused)) static LQuant quant_convert(LQuant q, LRat scale, LRat offset, const int *dnum, const int *dden, const char *sym) {
    if (!quant_same_dim(&q, dnum, dden)) logos_fatal("cannot convert between different dimensions");
    LQuant r = q; r.uscale = scale; r.uoffset = offset; strncpy(r.usym, sym, 23); r.usym[23] = 0; return r;
}
static void quant_dim_string(const LQuant *q, char *out) {
    out[0] = 0; int any = 0;
    for (int i = 0; i < 10; i++) {
        if (q->dnum[i] == 0) continue;
        if (any) strcat(out, "\xc2\xb7");   /* · */
        any = 1; strcat(out, QDIM_SYM[i]);
        if (!(q->dnum[i] == 1 && q->dden[i] == 1)) {
            char e[24]; if (q->dden[i] == 1) sprintf(e, "^%d", q->dnum[i]); else sprintf(e, "^%d/%d", q->dnum[i], q->dden[i]);
            strcat(out, e);
        }
    }
    if (!any) strcpy(out, "1");
}
__attribute__((unused)) static char *quant_to_string(LQuant q) {
    LRat m = quant_in_unit(&q); char *ms = rat_to_string(&m); lbig_free(&m.num); lbig_free(&m.den);
    char dim[160]; const char *tail = q.usym[0] == 0 ? (quant_dim_string(&q, dim), dim) : q.usym;
    char *out = (char *) lc_gc_alloc_atomic(strlen(ms) + strlen(tail) + 2); sprintf(out, "%s %s", ms, tail); lc_gc_free(ms); return out;
}
__attribute__((unused)) static void show_quant(LQuant q) { char *s = quant_to_string(q); printf("%s\n", s); lc_gc_free(s); }

/* ===== Modular ℤ/nℤ — the C mirror of `logicaffeine_base::Modular` (BigInt value + modulus) =====
   `modular(v, n)` reduces on construction; `+ − ×` wrap in the ring (moduli match, the VM guarantees).
   `value` is Euclidean-reduced to `[0, modulus)`. Display: `{value} (mod {modulus})`. By-value (rvalues). */
typedef struct { LBig value, modulus; } LMod;
/* reduce a SIGNED `value` into [0, modulus); CONSUMES value, BORROWS modulus (positive). */
__attribute__((unused)) static LBig mod_reduce(LBig value, const LBig *modulus) {
    LBig rem; LBig q = lbig_divmod_mag(&value, modulus, &rem); lbig_free(&q);
    if (value.sign < 0 && rem.len != 0) { LBig nr = lbig_sub_mag(modulus, &rem); lbig_free(&rem); rem = nr; }
    rem.sign = rem.len ? 1 : 0; lbig_free(&value); return rem;
}
__attribute__((unused)) static LMod mod_ints(int64_t v, int64_t n) {
    LBig modulus = lbig_from_i64(n < 0 ? -n : n); modulus.sign = modulus.len ? 1 : 0;
    LMod m = { mod_reduce(lbig_from_i64(v), &modulus), modulus }; return m;
}
/* Combining two ℤ/nℤ values requires the SAME modulus — the interpreter rejects a cross-ring op
   ("cannot combine values in different modular rings"). A compiled program that reaches one is not a
   valid LOGOS program on any tier, so we fail loud rather than silently pick one ring's modulus. */
__attribute__((unused)) static void mod_check_same(const LBig *m1, const LBig *m2) {
    if (lbig_cmp_mag(m1, m2) != 0) logos_fatal("cannot combine values in different modular rings");
}
/* Modular inverse of `a` mod `n` (n>0, gcd(a,n)=1) by the iterative extended Euclidean algorithm;
   the r-sequence stays non-negative (magnitude divmod), the s-sequence carries sign. Result in [0,n). */
__attribute__((unused)) static LBig lbig_mod_inverse(const LBig *a, const LBig *n) {
    LBig old_r = mod_reduce(lbig_copy(a), n), r = lbig_copy(n);
    LBig old_s = lbig_from_i64(1), s = lbig_from_i64(0);
    while (r.len != 0) {
        LBig rem; LBig q = lbig_divmod_mag(&old_r, &r, &rem);
        lbig_free(&old_r); old_r = r; r = rem;
        LBig qs = lbig_mul(&q, &s), ns = lbig_sub(&old_s, &qs);
        lbig_free(&qs); lbig_free(&q); lbig_free(&old_s); old_s = s; s = ns;
    }
    lbig_free(&r); lbig_free(&s); lbig_free(&old_r);
    return mod_reduce(old_s, n);
}
__attribute__((unused)) static LMod mod_add(LMod a, LMod b) {
    mod_check_same(&a.modulus, &b.modulus);
    LBig s = lbig_add(&a.value, &b.value), mod = lbig_copy(&a.modulus); LMod r = { mod_reduce(s, &mod), mod }; return r;
}
__attribute__((unused)) static LMod mod_sub(LMod a, LMod b) {
    mod_check_same(&a.modulus, &b.modulus);
    LBig s = lbig_sub(&a.value, &b.value), mod = lbig_copy(&a.modulus); LMod r = { mod_reduce(s, &mod), mod }; return r;
}
__attribute__((unused)) static LMod mod_mul(LMod a, LMod b) {
    mod_check_same(&a.modulus, &b.modulus);
    LBig s = lbig_mul(&a.value, &b.value), mod = lbig_copy(&a.modulus); LMod r = { mod_reduce(s, &mod), mod }; return r;
}
/* Division in the ring = multiply by the modular inverse of the divisor (a field only when the
   modulus is prime; the interpreter computes the inverse and errors if it does not exist). */
__attribute__((unused)) static LMod mod_div(LMod a, LMod b) {
    mod_check_same(&a.modulus, &b.modulus);
    LBig inv = lbig_mod_inverse(&b.value, &b.modulus), prod = lbig_mul(&a.value, &inv), mod = lbig_copy(&a.modulus);
    lbig_free(&inv); LMod r = { mod_reduce(prod, &mod), mod }; return r;
}
/* `pow(m, e)` — modular exponentiation by square-and-multiply, each step reduced into [0, modulus). */
__attribute__((unused)) static LMod mod_pow(LMod base, int64_t e) {
    LBig result = mod_reduce(lbig_from_i64(1), &base.modulus), b = lbig_copy(&base.value);
    uint64_t ee = (uint64_t) e;
    while (ee > 0) {
        if (ee & 1u) { LBig t = lbig_mul(&result, &b); lbig_free(&result); result = mod_reduce(t, &base.modulus); }
        ee >>= 1;
        if (ee > 0) { LBig t = lbig_mul(&b, &b); lbig_free(&b); b = mod_reduce(t, &base.modulus); }
    }
    lbig_free(&b); LMod r = { result, lbig_copy(&base.modulus) }; return r;
}
__attribute__((unused)) static char *mod_to_string(LMod a) {
    char *v = lbig_to_string(&a.value), *m = lbig_to_string(&a.modulus);
    char *out = (char *) lc_gc_alloc_atomic(strlen(v) + strlen(m) + 8); sprintf(out, "%s (mod %s)", v, m); lc_gc_free(v); lc_gc_free(m); return out;
}
__attribute__((unused)) static void show_mod(LMod a) { char *s = mod_to_string(a); printf("%s\n", s); lc_gc_free(s); }

/* ===== UUID (RFC 9562) — 16 big-endian bytes, the C mirror of `logicaffeine_base::Uuid`. `uuid(text)`
   parses the canonical `8-4-4-4-12` (or 32-hex, `urn:uuid:`/`{…}`-wrapped) form; Show renders the
   canonical LOWERCASE hyphenated string; `==` is a 16-byte compare; `uuid_bytes`/`uuid_from_bytes`
   round-trip the raw bytes through a `Seq_i64`. Fail-loud on a malformed literal (the VM errors too). */
typedef struct { unsigned char b[16]; } LUuid;
__attribute__((unused)) static int uuid_hexval(int c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}
__attribute__((unused)) static LUuid uuid_parse(const char *in) {
    const char *s = in; size_t len = strlen(s);
    while (len && (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')) { s++; len--; }
    while (len && (s[len-1] == ' ' || s[len-1] == '\t' || s[len-1] == '\n' || s[len-1] == '\r')) len--;
    if (len >= 9 && (strncmp(s, "urn:uuid:", 9) == 0 || strncmp(s, "URN:UUID:", 9) == 0)) { s += 9; len -= 9; }
    if (len >= 2 && s[0] == '{' && s[len-1] == '}') { s++; len -= 2; }
    unsigned char hex[32]; int n = 0;
    if (len == 36) {
        for (size_t i = 0; i < 36; i++) {
            if (i == 8 || i == 13 || i == 18 || i == 23) { if (s[i] != '-') goto bad; }
            else { if (n >= 32) goto bad; hex[n++] = (unsigned char) s[i]; }
        }
    } else if (len == 32) { for (int i = 0; i < 32; i++) hex[i] = (unsigned char) s[i]; n = 32; }
    else goto bad;
    if (n != 32) goto bad;
    { LUuid u;
      for (int i = 0; i < 16; i++) {
          int hi = uuid_hexval(hex[2*i]), lo = uuid_hexval(hex[2*i+1]);
          if (hi < 0 || lo < 0) goto bad;
          u.b[i] = (unsigned char)((hi << 4) | lo);
      }
      return u; }
bad: logos_fatalf("invalid UUID: %s", in);
}
__attribute__((unused)) static char *uuid_to_string(LUuid u) {
    static const char H[] = "0123456789abcdef"; char *out = (char *) lc_gc_alloc_atomic(37); int p = 0;
    for (int i = 0; i < 16; i++) { if (i == 4 || i == 6 || i == 8 || i == 10) out[p++] = '-'; out[p++] = H[u.b[i] >> 4]; out[p++] = H[u.b[i] & 0xF]; }
    out[p] = 0; return out;
}
__attribute__((unused)) static void show_uuid(LUuid u) { char *s = uuid_to_string(u); printf("%s\n", s); lc_gc_free(s); }
__attribute__((unused)) static LUuid uuid_nil(void) { LUuid u; memset(u.b, 0, 16); return u; }
__attribute__((unused)) static LUuid uuid_max(void) { LUuid u; memset(u.b, 0xFF, 16); return u; }
/* The four RFC 9562 Appendix-A namespaces differ only in byte 3 (0x10/0x11/0x12/0x14). */
__attribute__((unused)) static LUuid uuid_ns(unsigned char b3) {
    LUuid u = {{0x6b, 0xa7, 0xb8, b3, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8}}; return u;
}
__attribute__((unused)) static LUuid uuid_dns(void) { return uuid_ns(0x10); }
__attribute__((unused)) static LUuid uuid_url(void) { return uuid_ns(0x11); }
__attribute__((unused)) static LUuid uuid_oid(void) { return uuid_ns(0x12); }
__attribute__((unused)) static LUuid uuid_x500(void) { return uuid_ns(0x14); }
__attribute__((unused)) static int64_t uuid_version(LUuid u) { return (int64_t)(u.b[6] >> 4); }
__attribute__((unused)) static bool uuid_eq(LUuid a, LUuid b) { return memcmp(a.b, b.b, 16) == 0; }
__attribute__((unused)) static Seq_i64 uuid_bytes(LUuid u) {
    Seq_i64 r = seq_i64_new(); for (int i = 0; i < 16; i++) seq_i64_push(&r, (int64_t) u.b[i]); return r;
}
__attribute__((unused)) static LUuid uuid_from_bytes(Seq_i64 s) {
    LUuid u; for (int i = 0; i < 16; i++) u.b[i] = (unsigned char)(seq_i64_get(&s, i + 1) & 0xFF); return u;
}

"#
);

/// The **deterministic cooperative-concurrency runtime** for host C: the C family's scheduler core
/// (`sched_core.c`, `logicaffeine_runtime::Scheduler::process_step` ported) under the shared `ucontext`
/// tier (`ucontext_tier.c`). Every operation performs the core's step, then yields to the scheduler loop,
/// which runs the next ready task by the policy and advances the logical clock when none is ready — so a
/// program's output, at every `LOGOS_SEED`, is the interpreter's. Emitted ONLY when the program uses
/// concurrency (see `program_uses_concurrency`), so a non-concurrency program's C stays byte-identical.
pub(crate) const C_CONCURRENCY_RUNTIME: &str = concat!(
    "\n#include <ucontext.h>\n",
    "#define LOGOS_FN static __attribute__((unused))\n",
    "#define LOGOS_VAR static\n",
    "#define LOGOS_API static __attribute__((unused))\n",
    "#define logos_alloc(n) lc_gc_calloc(1, (n))\n",
    "#define logos_release(p) lc_gc_free(p)\n",
    "#define LOGOS_STACK_ENTER(lo, hi) lc_gc_stack_enter((lo), (hi))\n",
    "#define LOGOS_STACK_LEAVE() lc_gc_stack_leave()\n",
    "#define LOGOS_TASK_TIER ucontext_t ctx; void *stack; void *fn; long long nargs; long long *args;\n",
    include_str!("sched_core.c"),
    include_str!("host_seed.c"),
    include_str!("opt.c"),
    include_str!("ucontext_tier.c"),
);

/// The **stepped scheduler**: the same scheduler core under resumable step functions over frames
/// (`stackless_tier.c`) instead of coroutines, with the value packing and the glue the lowered code calls
/// (`stepped_tier.c`). The C tier lowers each task body into a step function (`stepped.rs`) that performs the
/// core's operation, saves the point to resume at and returns. Each target that runs it puts its own prologue
/// first — [`C_STEPPED_HOST`] on the host, the CUDA tier's device prologue on a GPU thread — so one scheduler
/// runs unchanged on both.
pub(crate) const STEPPED_SCHEDULER: &str = concat!(
    include_str!("sched_core.c"),
    include_str!("opt.c"),
    include_str!("stackless_tier.c"),
    include_str!("stepped_tier.c"),
);

/// The host's prologue to [`STEPPED_SCHEDULER`]: static linkage, the libc allocator, a failed run's message on
/// stderr and exit status 1, and the run's seed read from `LOGOS_SEED` (else the `LOGOS_SCHED_SEED` knob's).
pub(crate) const C_STEPPED_HOST: &str = concat!(
    "\n#include <setjmp.h>\n",
    "#define LOGOS_FN static __attribute__((unused))\n",
    "#define LOGOS_VAR static\n",
    "#define LOGOS_API static __attribute__((unused))\n",
    "#define logos_alloc(n) lc_gc_calloc(1, (n))\n",
    "#define logos_release(p) lc_gc_free(p)\n",
    "#define logos_malloc(n) lc_gc_alloc(n)\n",
    "#define LOGOS_SCHED_ROOTS_ENTER(top) lc_gc_scheduler_stack_enter(top)\n",
    "#define LOGOS_SCHED_ROOTS_LEAVE(top) lc_gc_remove_root(top)\n",
    "#define LOGOS_WORKER_STACK_ENTER(top) uintptr_t logos_saved_stack = lc_gc_worker_stack_enter(top)\n",
    "#define LOGOS_WORKER_STACK_LEAVE() lc_gc_worker_stack_leave(logos_saved_stack)\n",
    "typedef int (*logos_step_fn)(void *);\n",
    "#define LOGOS_TASK_TIER logos_step_fn step; void *frame;\n",
    "#define LOGOS_RUN_FAILS(message) do { fflush(stdout); fprintf(stderr, \"%s\\n\", (message)); exit(1); } while (0)\n",
    "#ifndef LOGOS_SCHED_SEED\n#define LOGOS_SCHED_SEED 0ULL\n#endif\n",
    "#define LOGOS_RUN_SEED logos_env_seed(LOGOS_SCHED_SEED)\n",
    include_str!("host_seed.c"),
);

/// The **single-replica CRDT-counter runtime** — a `logos_crdt` per-replica count map, a straight port of
/// `codegen_riscv`'s `logos_crdt_*` (itself matching the tree-walker's `GCounter`/`PNCounter`). Emitted
/// ONLY when the program uses a counter CRDT (see `program_uses_crdt`), so a non-CRDT program's C stays
/// byte-identical.
///
/// The counter lives behind a `logos_crdt*` handle stored in the struct field (whose declared type is
/// `Int`, so the field cell is an `int64_t` holding the cast pointer). Its OBSERVABLE is the SUM of the
/// per-replica slots; `Increase`/`Decrease` bump THIS replica's own slot; `Merge`/`Join` take the
/// per-replica MAX (idempotent — a redelivered state never double-counts). A single replica therefore
/// behaves like an ordinary monotone counter, matching the VM/tree-walker byte-for-byte.
///
/// DEVICE-PORTABILITY: this uses ONLY the heap + integer ops (malloc/realloc + element-copy loops, no
/// `memcpy`/`getenv`/`time`/`snprintf`), so the CUDA tier — which tree-shakes + lifts `codegen_c` — can
/// lift it verbatim onto the device (malloc/realloc already have device shims).
pub(super) const C_CRDT_RUNTIME: &str = r#"
/* ---- CRDT counters (G-Counter / PN-Counter) — a per-replica count map ---- */
/* The observable is the SUM; merge/join take the per-replica MAX (IDEMPOTENT — a redelivered state
   never double-counts), so it converges regardless of order/duplication. The replica ids are drawn
   from a local monotone sequence: the observable (the sum) depends only on the ids being DISTINCT
   and STABLE within a run, not their absolute values, so it matches the VM whatever the VM's own
   (process-global) replica sequence happens to be. */

typedef struct { long long my_rid; long long len, cap; long long *rids; long long *cnts; } logos_crdt;

static long long logos_replica_seq = 0;

__attribute__((unused)) static logos_crdt *logos_crdt_new(void) {
    logos_crdt *c = (logos_crdt *)lc_gc_alloc(sizeof(logos_crdt));
    c->my_rid = logos_replica_seq++;
    c->len = 1; c->cap = 4;
    c->rids = (long long *)lc_gc_alloc((size_t)c->cap * sizeof(long long));
    c->cnts = (long long *)lc_gc_alloc((size_t)c->cap * sizeof(long long));
    c->rids[0] = c->my_rid; c->cnts[0] = 0;
    return c;
}

/* Increment (or decrement) THIS counter's own replica entry, in place. */
__attribute__((unused)) static void logos_crdt_bump(logos_crdt *c, long long amount, int negate) {
    long long d = negate ? -amount : amount;
    for (long long i = 0; i < c->len; i++) {
        if (c->rids[i] == c->my_rid) { c->cnts[i] += d; return; }
    }
    /* my_rid always present (seeded in _new); defensive. */
    c->cnts[0] += d;
}

/* The observable: the sum of every per-replica slot. */
__attribute__((unused)) static long long logos_crdt_sum(const logos_crdt *c) {
    long long s = 0;
    for (long long i = 0; i < c->len; i++) s += c->cnts[i];
    return s;
}

static void logos_crdt_put_max(logos_crdt *d, long long rid, long long cnt) {
    for (long long i = 0; i < d->len; i++) {
        if (d->rids[i] == rid) { if (cnt > d->cnts[i]) d->cnts[i] = cnt; return; }
    }
    if (d->len == d->cap) {
        long long nc = d->cap * 2;
        long long *nr = (long long *)lc_gc_alloc((size_t)nc * sizeof(long long));
        long long *nn = (long long *)lc_gc_alloc((size_t)nc * sizeof(long long));
        for (long long i = 0; i < d->len; i++) { nr[i] = d->rids[i]; nn[i] = d->cnts[i]; }
        lc_gc_free(d->rids); lc_gc_free(d->cnts);
        d->rids = nr; d->cnts = nn; d->cap = nc;
    }
    d->rids[d->len] = rid; d->cnts[d->len] = cnt; d->len++;
}

/* Lattice join: per-replica MAX of `s` into `d`, in place (idempotent, commutative, associative). */
__attribute__((unused)) static void logos_crdt_merge(logos_crdt *d, const logos_crdt *s) {
    for (long long i = 0; i < s->len; i++) logos_crdt_put_max(d, s->rids[i], s->cnts[i]);
}
"#;

/// The call of `logos_dyn_sprintf` that renders one of the VM's sentences, `words` (a C format and the names of
/// the arguments it takes, in order), with each name bound to the C expression `vars` gives it.
fn c_sentence(words: &(String, Vec<&'static str>), vars: &[(&str, &str)]) -> String {
    let args: Vec<&str> = words
        .1
        .iter()
        .map(|name| {
            vars.iter()
                .find(|(n, _)| n == name)
                .map(|(_, e)| *e)
                .unwrap_or_else(|| panic!("no C value for {name}"))
        })
        .collect();
    format!("logos_dyn_sprintf({}, {})", words.0, args.join(", "))
}

/// Peer messaging for both C generators — the C mirror of the VM's `NetInbox` in its offline, single-node
/// mode, which is how every tier runs a program that never connects to a relay:
///
/// - An address is its canonical relay topic: a `ws://` URL as written, a libp2p multiaddr as its `ws://`
///   form, any other text trimmed (`logos_net_topic`, a port of `addr::canonical_topic` /
///   `multiaddr_to_ws_url`, trimming exactly the characters Rust's `trim` does — the runtime's `logos_trim`). A peer handle is its topic.
/// - `Connect` validates its address and dials nothing; an address that is neither fails the run in the
///   VM's words.
/// - `Listen` sets this node's inbox topic.
/// - `Send` / `Stream` post a frame — the destination topic, the sender (this node's inbox, or nothing
///   before a `Listen`), whether it is a stream, and the message's 64-bit register encoding (the value
///   itself, or its heap handle) — to the loopback outbox.
/// - `Await` fails the run without an inbox; otherwise it takes the first received frame from the awaited
///   peer of its own sort (a plain `Await` never takes a stream), draining the outbox into the received queue
///   first — keeping only the frames addressed to this node's inbox, exactly as the relay would deliver. A
///   message nothing will ever send is awaited forever, as the VM awaits it.
///
/// The functions take and return C strings and `long long` payloads, so the AST generator's calls
/// (`logos_net_send(to, msg)`, `logos_net_await(from)`) and the bytecode walker's share one transport. The
/// slice uses only `malloc`/`realloc`/`memcpy`/`strlen`/`strcmp`/`strdup` and the runtime's formatter, all of
/// which the CUDA tier shims, so it lifts onto a device unchanged. Emitted only when the program networks.
pub(super) fn c_net_runtime() -> String {
    let w = super::words::address_words();
    let t = [("trimmed", "trimmed"), ("seg", "seg")];
    NET_RUNTIME
        .replace("@EMPTY@", &format!("lc_gc_strdup({})", w.empty))
        .replace("@NOT_AN_ADDRESS@", &c_sentence(&w.not_an_address, &t))
        .replace("@NO_HOST_AFTER@", &c_sentence(&w.no_host_after, &t))
        .replace("@NO_PORT_AFTER_TCP@", &c_sentence(&w.no_port_after_tcp, &t))
        .replace("@NON_NUMERIC_PORT@", &c_sentence(&w.non_numeric_port, &t))
        .replace(
            "@UNSUPPORTED_PROTOCOL@",
            &c_sentence(&w.unsupported_protocol, &t),
        )
        .replace("@NO_HOST@", &c_sentence(&w.no_host, &t))
        .replace("@NO_PORT@", &c_sentence(&w.no_port, &t))
        .replace(
            "@CONNECT_REFUSED@",
            &c_sentence(
                &super::words::connect_refused(),
                &[("raw", "url"), ("why", "why")],
            ),
        )
        .replace(
            "@AWAIT_WITHOUT_LISTEN@",
            &super::words::await_without_listen(),
        )
}

const NET_RUNTIME: &str = r#"
// --- Peer messaging: the VM's offline NetInbox (loopback outbox, received queue, inbox topic) ----------------
typedef struct { char *topic; char *from; int stream; long long payload; } logos_net_frame;
typedef struct { logos_net_frame *at; size_t len, cap; } logos_net_queue;
static logos_net_queue logos_net_outbox, logos_net_received;
static char *logos_net_inbox = 0;
__attribute__((unused)) static void logos_net_push(logos_net_queue *q, logos_net_frame f) {
    if (q->len == q->cap) {
        q->cap = q->cap ? q->cap * 2 : 8;
        q->at = (logos_net_frame *)lc_gc_realloc(q->at, q->cap * sizeof(logos_net_frame));
    }
    q->at[q->len++] = f;
}
__attribute__((unused)) static int logos_net_prefix_ci(const char *s, const char *p) {
    for (; *p; s++, p++) {
        char c = *s;
        if (c >= 'A' && c <= 'Z') c = (char)(c - 'A' + 'a');
        if (c != *p) return 0;
    }
    return 1;
}
/* Whether `v` parses as Rust parses a `u16`: an optional `+`, at least one digit, nothing else, at most 65535. */
__attribute__((unused)) static int logos_net_is_port(const char *v) {
    if (*v == '+') v++;
    if (!*v) return 0;
    long value = 0;
    for (; *v; v++) {
        if (*v < '0' || *v > '9') return 0;
        value = value * 10 + (*v - '0');
        if (value > 65535) return 0;
    }
    return 1;
}
/* `addr` as the `ws://` URL a relay is dialed at — a `ws://`/`wss://` URL as written, a multiaddr as its URL —
   or NULL with `*why` the reason it is neither, in `multiaddr_to_ws_url`'s words. */
__attribute__((unused)) static char *logos_net_ws_url(const char *addr, char **why) {
    char *trimmed = logos_trim(addr);
    if (!*trimmed) { *why = @EMPTY@; return 0; }
    if (logos_net_prefix_ci(trimmed, "ws://") || logos_net_prefix_ci(trimmed, "wss://")) return trimmed;
    if (trimmed[0] != '/') { *why = @NOT_AN_ADDRESS@; return 0; }
    size_t nseg = 0, cap = 8;
    char **segs = (char **)lc_gc_alloc(cap * sizeof(char *));
    for (const char *p = trimmed + 1;;) {
        const char *e = p;
        while (*e && *e != '/') e++;
        if (nseg == cap) { cap *= 2; segs = (char **)lc_gc_realloc(segs, cap * sizeof(char *)); }
        char *seg = (char *)lc_gc_alloc_atomic((size_t)(e - p) + 1);
        memcpy(seg, p, (size_t)(e - p));
        seg[e - p] = 0;
        segs[nseg++] = seg;
        if (!*e) break;
        p = e + 1;
    }
    const char *host = 0, *port = 0, *scheme = "ws";
    int v6 = 0;
    for (size_t i = 0; i < nseg;) {
        const char *seg = segs[i];
        if (!strcmp(seg, "ip4") || !strcmp(seg, "dns") || !strcmp(seg, "dns4") || !strcmp(seg, "dns6") || !strcmp(seg, "dnsaddr") || !strcmp(seg, "ip6")) {
            if (i + 1 >= nseg || !*segs[i + 1]) { *why = @NO_HOST_AFTER@; return 0; }
            host = segs[i + 1];
            v6 = !strcmp(seg, "ip6");
            i += 2;
        } else if (!strcmp(seg, "tcp")) {
            if (i + 1 >= nseg || !*segs[i + 1]) { *why = @NO_PORT_AFTER_TCP@; return 0; }
            if (!logos_net_is_port(segs[i + 1])) { seg = segs[i + 1]; *why = @NON_NUMERIC_PORT@; return 0; }
            port = segs[i + 1];
            i += 2;
        } else if (!strcmp(seg, "ws")) {
            scheme = "ws";
            i += 1;
        } else if (!strcmp(seg, "wss") || !strcmp(seg, "tls")) {
            scheme = "wss";
            i += 1;
        } else if (!strcmp(seg, "p2p")) {
            i += 2;
        } else if (!strcmp(seg, "p2p-circuit")) {
            i += 1;
        } else {
            *why = @UNSUPPORTED_PROTOCOL@;
            return 0;
        }
    }
    if (!host) { *why = @NO_HOST@; return 0; }
    if (!port) { *why = @NO_PORT@; return 0; }
    return logos_dyn_sprintf("%s://%s%s%s:%s", scheme, v6 ? "[" : "", host, v6 ? "]" : "", port);
}
/* The relay topic an address names: its URL, or the address itself, trimmed. */
__attribute__((unused)) static char *logos_net_topic(const char *addr) {
    char *why = 0;
    char *url = logos_net_ws_url(addr, &why);
    return url ? url : logos_trim(addr);
}
__attribute__((unused)) static void logos_net_connect(const char *url) {
    char *why = 0;
    if (!logos_net_ws_url(url, &why)) logos_fatal(@CONNECT_REFUSED@);
}
__attribute__((unused)) static void logos_net_listen(const char *topic) { logos_net_inbox = logos_net_topic(topic); }
__attribute__((unused)) static long long logos_net_makepeer(long long addr) {
    return (long long)(intptr_t)logos_net_topic((const char *)(intptr_t)addr);
}
__attribute__((unused)) static void logos_net_post(const char *to, int stream, long long payload) {
    logos_net_frame f = { logos_net_topic(to), logos_net_inbox ? logos_net_inbox : (char *)"", stream, payload };
    logos_net_push(&logos_net_outbox, f);
}
__attribute__((unused)) static void logos_net_send(const char *to, long long msg) { logos_net_post(to, 0, msg); }
__attribute__((unused)) static void logos_net_stream(const char *to, long long values) { logos_net_post(to, 1, values); }
__attribute__((unused)) static int logos_net_take(const char *want, int stream, long long *payload) {
    for (size_t i = 0; i < logos_net_received.len; i++) {
        logos_net_frame f = logos_net_received.at[i];
        if (f.stream == stream && !strcmp(f.from, want)) {
            for (size_t j = i + 1; j < logos_net_received.len; j++) logos_net_received.at[j - 1] = logos_net_received.at[j];
            logos_net_received.len--;
            *payload = f.payload;
            return 1;
        }
    }
    return 0;
}
__attribute__((unused)) static long long logos_net_await_sort(const char *from, int stream) {
    char *want = logos_net_topic(from);
    if (!logos_net_inbox) logos_fatalf(@AWAIT_WITHOUT_LISTEN@);
    long long payload = 0;
    if (logos_net_take(want, stream, &payload)) return payload;
    for (size_t i = 0; i < logos_net_outbox.len; i++) {
        if (!strcmp(logos_net_outbox.at[i].topic, logos_net_inbox)) logos_net_push(&logos_net_received, logos_net_outbox.at[i]);
    }
    logos_net_outbox.len = 0;
    if (logos_net_take(want, stream, &payload)) return payload;
    fflush(stdout);
    for (;;) {}
}
__attribute__((unused)) static long long logos_net_await(const char *from) { return logos_net_await_sort(from, 0); }
__attribute__((unused)) static long long logos_net_await_stream(const char *from) { return logos_net_await_sort(from, 1); }
"#;

/// The sandboxed file system the file statements run against — the C mirror of the VM's `MemVfs`, shared by
/// the C-family backends. Files live only in a sealed test world (`logos_vfs_seal`, which a test runner calls
/// before seeding its `Given a file …` fixtures with `logos_vfs_write`); outside one — any ordinary program run —
/// a write lands nowhere and a read finds nothing, failing in the VM's words, so no run ever touches the real
/// file system. `Mount` reads a journal's text, or nothing when it is not there. Emitted only when the program
/// reads or writes a file.
pub(super) fn c_vfs_runtime() -> String {
    VFS_RUNTIME.replace("@FILE_NOT_FOUND@", &super::words::file_not_found())
}

const VFS_RUNTIME: &str = r#"
// --- Files: the VM's sandboxed MemVfs — contents only inside a sealed test world ---------------------------
typedef struct { char *path; char *content; } logos_vfs_file;
static logos_vfs_file *logos_vfs_files = 0;
static size_t logos_vfs_len = 0, logos_vfs_cap = 0;
static int logos_vfs_sealed = 0;
__attribute__((unused)) static void logos_vfs_seal(void) { logos_vfs_sealed = 1; }
__attribute__((unused)) static logos_vfs_file *logos_vfs_find(const char *path) {
    for (size_t i = 0; i < logos_vfs_len; i++) if (!strcmp(logos_vfs_files[i].path, path)) return &logos_vfs_files[i];
    return 0;
}
__attribute__((unused)) static void logos_vfs_write(const char *path, const char *content) {
    if (!logos_vfs_sealed) return;
    logos_vfs_file *f = logos_vfs_find(path);
    if (f) { f->content = lc_gc_strdup(content); return; }
    if (logos_vfs_len == logos_vfs_cap) {
        logos_vfs_cap = logos_vfs_cap ? logos_vfs_cap * 2 : 8;
        logos_vfs_files = (logos_vfs_file *)lc_gc_realloc(logos_vfs_files, logos_vfs_cap * sizeof(logos_vfs_file));
    }
    logos_vfs_files[logos_vfs_len].path = lc_gc_strdup(path);
    logos_vfs_files[logos_vfs_len].content = lc_gc_strdup(content);
    logos_vfs_len++;
}
__attribute__((unused)) static char *logos_vfs_read(const char *path) {
    logos_vfs_file *f = logos_vfs_find(path);
    if (!f) logos_fatalf(@FILE_NOT_FOUND@, path);
    return lc_gc_strdup(f->content);
}
__attribute__((unused)) static char *logos_vfs_mount(const char *path) {
    logos_vfs_file *f = logos_vfs_find(path);
    return lc_gc_strdup(f ? f->content : "");
}
"#;

/// The real-socket TCP client runtime — the C mirror of `TcpRegistry` (the async tiers) and
/// `logicaffeine_system::tcp` (the sync AOT tier). POSIX sockets; connections/listeners are opaque
/// `long long` handles in a process-global table, exactly like the Rust registries. Each helper returns
/// the RAW value (a handle / byte count / -1 on failure) and, on failure, writes the shared `Err` text
/// into `*err` — the Op-walker boxes the raw value or the text into the LOGOS `Result`. The wording is
/// byte-identical to the Rust tiers: an OS failure renders `"<strerror> (os error <errno>)"`, matching
/// `std::io::Error`'s Display, so the `Err` value cannot drift between the VM and this tier.
pub(super) const C_TCP_RUNTIME: &str = r#"
// --- TCP client: real POSIX sockets, the C mirror of TcpRegistry / logicaffeine_system::tcp ----------
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <unistd.h>
#include <errno.h>

#define LOGOS_TCP_MAX 256
#define LOGOS_TCP_MAX_RECV (64LL * 1024 * 1024)
static int logos_tcp_fds[LOGOS_TCP_MAX];   /* fd for a live handle, -1 for a free/closed slot */
static int logos_tcp_is_listener[LOGOS_TCP_MAX];
static int logos_tcp_next = 1;             /* next handle id (1-based, matching the Rust `next`) */
static int logos_tcp_inited = 0;
__attribute__((unused)) static void logos_tcp_init(void) {
    if (logos_tcp_inited) return;
    for (int i = 0; i < LOGOS_TCP_MAX; i++) { logos_tcp_fds[i] = -1; logos_tcp_is_listener[i] = 0; }
    logos_tcp_inited = 1;
}
/* The shared OS-error rendering: `"<strerror> (os error <errno>)"` — byte-identical to Rust's
   `std::io::Error` Display, so an `Err` produced here equals the one the VM (tokio) produces. */
__attribute__((unused)) static char *logos_tcp_oserr(const char *prefix, int e) {
    return logos_dyn_sprintf("%s%s (os error %d)", prefix, strerror(e), e);
}
/* Register an accepted/connected fd, returning its LOGOS handle (or -1 on table exhaustion). */
__attribute__((unused)) static long long logos_tcp_register(int fd, int is_listener) {
    logos_tcp_init();
    long long id = logos_tcp_next++;
    if (id >= LOGOS_TCP_MAX) { close(fd); return -1; }
    logos_tcp_fds[id] = fd;
    logos_tcp_is_listener[id] = is_listener;
    return id;
}
/* tcpConnect: fd handle on success, -1 on failure (`*err` = the shared `Err` text). */
__attribute__((unused)) static long long logos_tcp_connect(const char *host, long long port, char **err) {
    logos_tcp_init();
    if (port < 1 || port > 65535) { *err = logos_dyn_sprintf("tcp: invalid port %lld (1..=65535)", port); return -1; }
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC; hints.ai_socktype = SOCK_STREAM;
    char portstr[16]; snprintf(portstr, sizeof(portstr), "%lld", port);
    int gai = getaddrinfo(host, portstr, &hints, &res);
    if (gai != 0) { *err = logos_dyn_sprintf("tcp: connect to %s:%lld failed: %s", host, port, gai_strerror(gai)); return -1; }
    int fd = -1; int last = ECONNREFUSED;
    for (struct addrinfo *ai = res; ai; ai = ai->ai_next) {
        fd = socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
        if (fd < 0) { last = errno; continue; }
        if (connect(fd, ai->ai_addr, ai->ai_addrlen) == 0) break;
        last = errno; close(fd); fd = -1;
    }
    freeaddrinfo(res);
    if (fd < 0) {
        char pfx[128]; snprintf(pfx, sizeof(pfx), "tcp: connect to %s:%lld failed: ", host, port);
        *err = logos_tcp_oserr(pfx, last); return -1;
    }
    int one = 1; setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one)); /* disable Nagle (libpq/tokio parity) */
    return logos_tcp_register(fd, 0);
}
/* tcpSend: byte count sent on success, -1 on failure. `data` is a `Seq of Int` (each element masked to a byte). */
__attribute__((unused)) static long long logos_tcp_send(long long conn, Seq_i64 *data, char **err) {
    logos_tcp_init();
    if (conn < 0 || conn >= LOGOS_TCP_MAX || logos_tcp_fds[conn] < 0) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", conn); return -1; }
    int fd = logos_tcp_fds[conn];
    size_t n = data->len;
    unsigned char *bytes = (unsigned char *)lc_gc_alloc_atomic(n ? n : 1);
    for (size_t i = 0; i < n; i++) bytes[i] = (unsigned char)(data->data[i] & 0xff);
    size_t off = 0;
    while (off < n) {
        ssize_t w = send(fd, bytes + off, n - off, 0);
        if (w < 0) { int e = errno; lc_gc_free(bytes); *err = logos_tcp_oserr("tcp: send failed: ", e); return -1; }
        off += (size_t)w;
    }
    lc_gc_free(bytes);
    return (long long)n;
}
/* tcpRecv: a `Seq of Int` (up to `max_len` bytes; a clean EOF is the empty Seq) on success, NULL on failure. */
__attribute__((unused)) static Seq_i64 *logos_tcp_recv(long long conn, long long max_len, char **err) {
    logos_tcp_init();
    if (max_len < 0 || max_len > LOGOS_TCP_MAX_RECV) { *err = logos_dyn_sprintf("tcp: invalid length %lld (0..=%lld)", max_len, LOGOS_TCP_MAX_RECV); return NULL; }
    if (conn < 0 || conn >= LOGOS_TCP_MAX || logos_tcp_fds[conn] < 0) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", conn); return NULL; }
    int fd = logos_tcp_fds[conn];
    unsigned char *buf = (unsigned char *)lc_gc_alloc_atomic(max_len ? (size_t)max_len : 1);
    ssize_t got = recv(fd, buf, (size_t)max_len, 0);
    if (got < 0) { int e = errno; lc_gc_free(buf); *err = logos_tcp_oserr("tcp: recv failed: ", e); return NULL; }
    Seq_i64 *out = (Seq_i64 *)lc_gc_alloc(sizeof(Seq_i64));
    *out = seq_i64_new();
    for (ssize_t i = 0; i < got; i++) seq_i64_push(out, (int64_t)buf[i]);
    lc_gc_free(buf);
    return out;
}
/* tcpRecvExact: a `Seq of Int` of EXACTLY `len` bytes on success; a peer closing early is a torn-frame Err. */
__attribute__((unused)) static Seq_i64 *logos_tcp_recv_exact(long long conn, long long len, char **err) {
    logos_tcp_init();
    if (len < 0 || len > LOGOS_TCP_MAX_RECV) { *err = logos_dyn_sprintf("tcp: invalid length %lld (0..=%lld)", len, LOGOS_TCP_MAX_RECV); return NULL; }
    if (conn < 0 || conn >= LOGOS_TCP_MAX || logos_tcp_fds[conn] < 0) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", conn); return NULL; }
    int fd = logos_tcp_fds[conn];
    unsigned char *buf = (unsigned char *)lc_gc_alloc_atomic(len ? (size_t)len : 1);
    size_t off = 0;
    while (off < (size_t)len) {
        ssize_t r = recv(fd, buf + off, (size_t)len - off, 0);
        if (r == 0) { lc_gc_free(buf); *err = logos_dyn_sprintf("tcp: connection closed after %zu of %lld bytes", off, len); return NULL; }
        if (r < 0) { int e = errno; lc_gc_free(buf); *err = logos_tcp_oserr("tcp: recv failed: ", e); return NULL; }
        off += (size_t)r;
    }
    Seq_i64 *out = (Seq_i64 *)lc_gc_alloc(sizeof(Seq_i64));
    *out = seq_i64_new();
    for (size_t i = 0; i < (size_t)len; i++) seq_i64_push(out, (int64_t)buf[i]);
    lc_gc_free(buf);
    return out;
}
/* tcpClose: 0 on success, -1 on an unknown handle. */
__attribute__((unused)) static long long logos_tcp_close(long long conn, char **err) {
    logos_tcp_init();
    if (conn < 0 || conn >= LOGOS_TCP_MAX || logos_tcp_fds[conn] < 0) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", conn); return -1; }
    close(logos_tcp_fds[conn]);
    logos_tcp_fds[conn] = -1; logos_tcp_is_listener[conn] = 0;
    return 0;
}
/* tcpSetTimeout: per-connection recv/send deadline via SO_RCVTIMEO/SO_SNDTIMEO. 0 on success, -1 + *err. */
__attribute__((unused)) static long long logos_tcp_set_timeout(long long conn, long long millis, char **err) {
    logos_tcp_init();
    if (conn < 0 || conn >= LOGOS_TCP_MAX || logos_tcp_fds[conn] < 0) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", conn); return -1; }
    int fd = logos_tcp_fds[conn];
    struct timeval tv; tv.tv_sec = millis / 1000; tv.tv_usec = (millis % 1000) * 1000;
    if (setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)) < 0) { int e = errno; *err = logos_tcp_oserr("tcp: set timeout failed: ", e); return -1; }
    if (setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)) < 0) { int e = errno; *err = logos_tcp_oserr("tcp: set timeout failed: ", e); return -1; }
    return 0;
}
/* tcpShutdownWrite: half-close the write side (peer sees EOF). 0 on success, -1 + *err. */
__attribute__((unused)) static long long logos_tcp_shutdown_write(long long conn, char **err) {
    logos_tcp_init();
    if (conn < 0 || conn >= LOGOS_TCP_MAX || logos_tcp_fds[conn] < 0) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", conn); return -1; }
    if (shutdown(logos_tcp_fds[conn], SHUT_WR) < 0) { int e = errno; *err = logos_tcp_oserr("tcp: shutdown write failed: ", e); return -1; }
    return 0;
}
/* tcpListen: listener handle on success, -1 on failure (`0` picks an ephemeral port). */
__attribute__((unused)) static long long logos_tcp_listen(long long port, char **err) {
    logos_tcp_init();
    if (port < 0 || port > 65535) { *err = logos_dyn_sprintf("tcp: invalid port %lld (1..=65535)", port); return -1; }
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) { *err = logos_tcp_oserr("tcp: listen on port ", errno); return -1; }
    int one = 1; setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in addr; memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET; addr.sin_addr.s_addr = htonl(INADDR_ANY); addr.sin_port = htons((uint16_t)port);
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) { int e = errno; close(fd); char pfx[64]; snprintf(pfx, sizeof(pfx), "tcp: listen on port %lld failed: ", port); *err = logos_tcp_oserr(pfx, e); return -1; }
    if (listen(fd, 128) < 0) { int e = errno; close(fd); char pfx[64]; snprintf(pfx, sizeof(pfx), "tcp: listen on port %lld failed: ", port); *err = logos_tcp_oserr(pfx, e); return -1; }
    return logos_tcp_register(fd, 1);
}
/* tcpAccept: a new connection handle on success, -1 on failure. */
__attribute__((unused)) static long long logos_tcp_accept(long long listener, char **err) {
    logos_tcp_init();
    if (listener < 0 || listener >= LOGOS_TCP_MAX || logos_tcp_fds[listener] < 0 || !logos_tcp_is_listener[listener]) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", listener); return -1; }
    int fd = accept(logos_tcp_fds[listener], NULL, NULL);
    if (fd < 0) { *err = logos_tcp_oserr("tcp: accept failed: ", errno); return -1; }
    int one = 1; setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
    return logos_tcp_register(fd, 0);
}
/* tcpListenerPort: the local port a listener bound to (how an ephemeral `0` bind is learned). */
__attribute__((unused)) static long long logos_tcp_listener_port(long long listener, char **err) {
    logos_tcp_init();
    if (listener < 0 || listener >= LOGOS_TCP_MAX || logos_tcp_fds[listener] < 0 || !logos_tcp_is_listener[listener]) { *err = logos_dyn_sprintf("tcp: closed or unknown connection %lld", listener); return -1; }
    struct sockaddr_in addr; socklen_t len = sizeof(addr);
    if (getsockname(logos_tcp_fds[listener], (struct sockaddr *)&addr, &len) < 0) { *err = logos_tcp_oserr("tcp: accept failed: ", errno); return -1; }
    return (long long)ntohs(addr.sin_port);
}
"#;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/runtime/bignum_lib_tests.rs"]
mod bignum_lib_tests;
