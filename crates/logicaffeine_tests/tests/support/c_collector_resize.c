
int main(void) {
    unsigned char *p = lc_gc_alloc(8192);
    memset(p, 0x5a, 8192);
    p = lc_gc_realloc(p, 4096);
    p = lc_gc_realloc(p, 8192);
    size_t changed = 0, dirty = 0;
    for (size_t i = 0; i < 4096; i++) changed += p[i] != 0x5a;
    for (size_t i = 4096; i < 8192; i++) dirty += p[i] != 0;
    printf("changed retained bytes=%zu uninitialized grown bytes=%zu\n", changed, dirty);
    lc_gc_free(p);
    return changed || dirty ? 7 : 0;
}
