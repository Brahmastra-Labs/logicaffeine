#define LOGOS_FN static __attribute__((unused))
#define LOGOS_VAR static
#define LOGOS_TASK_TIER
#define logos_alloc(n) lc_gc_calloc(1,(n))
#define logos_release(p) lc_gc_free(p)
