
static void *test_root;
static size_t collect_explicit_root(void) {
    lc_gc_prepare();
    lc_gc_pending_n = 0;
    lc_gc_consider((uintptr_t)test_root);
    while (lc_gc_pending_n) {
        lc_gc_range r=lc_gc_pending[--lc_gc_pending_n];
        lc_gc_scan(r.lo,r.hi);
    }
    size_t freed=0;
    size_t live=lc_gc_sweep(&freed);
    lc_gc_live=live;
    return live;
}
int main(int argc,char **argv) {
    int mode=atoi(argv[1]);
    void *a=lc_gc_alloc(8);
    memset(a,0,8);
    void *b=lc_gc_alloc(8);
    memset(b,0,8);
    size_t one=lc_gc_live/2;
    test_root=a;
    size_t expected=one;
    if(mode==0) { lc_gc_free(a); expected=0; }
    if(mode==1) { /* The allocator's pad link is not an edge to b. */ }
    if(mode==2) { *(void **)a=b; expected=2*one; }
    if(mode==3) { *(void **)a=b; lc_gc_free(a); test_root=lc_gc_alloc(8); }
    if(mode==4) { test_root=lc_gc_alloc_atomic(8); *(void **)test_root=b; }
    size_t actual=collect_explicit_root();
    printf("mode=%d expected=%zu actual=%zu\n",mode,expected,actual);
    return actual==expected?0:7;
}
