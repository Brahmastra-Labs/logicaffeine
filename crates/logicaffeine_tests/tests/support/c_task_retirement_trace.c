
static size_t trace_one(const void *root, size_t size) {
    lc_gc_prepare(); lc_gc_pending_n = 0;
    lc_gc_scan((uintptr_t)root, (uintptr_t)root + size);
    while (lc_gc_pending_n) {
        lc_gc_range r = lc_gc_pending[--lc_gc_pending_n];
        lc_gc_scan(r.lo, r.hi);
    }
    size_t unused = 0;
    return lc_gc_live = lc_gc_sweep(&unused);
}
