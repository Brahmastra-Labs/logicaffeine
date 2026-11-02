
#define LOGOS_FN static __attribute__((unused))
#define LOGOS_VAR static
#define LOGOS_API static __attribute__((unused))
#define logos_alloc(n) lc_gc_calloc(1,(n))
#define logos_malloc(n) lc_gc_alloc(n)
#define logos_release(p) lc_gc_free(p)
#define LOGOS_RUN_FAILS(msg) do { fprintf(stderr, "%s\n", (msg)); exit(1); } while (0)

typedef int (*logos_step_fn)(void *);
#define LOGOS_TASK_TIER logos_step_fn step; void *frame;
