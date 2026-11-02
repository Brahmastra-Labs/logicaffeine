
#define LOGOS_FN static __attribute__((unused))
#define LOGOS_VAR static
#define LOGOS_API static __attribute__((unused))
#define logos_alloc(n) lc_gc_calloc(1,(n))
#define logos_malloc(n) lc_gc_alloc(n)
#define logos_release(p) lc_gc_free(p)
#define LOGOS_RUN_FAILS(msg) do { fprintf(stderr, "%s\n", (msg)); exit(1); } while (0)

#include <ucontext.h>
#define LOGOS_TASK_TIER ucontext_t ctx; void *stack; void *fn; long long nargs; long long *args;
#define LOGOS_STACK_ENTER(lo, hi) lc_gc_stack_enter((lo),(hi))
#define LOGOS_STACK_LEAVE() lc_gc_stack_leave()
static void (*logos_fail_hook)(const char *);
static unsigned long long logos_env_seed(unsigned long long fallback) { return fallback; }
typedef struct { int present; long long value; } logos_opt;
