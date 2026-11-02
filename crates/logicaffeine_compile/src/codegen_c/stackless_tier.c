/* The C family's stackless tier: the scheduler core (`sched_core.c`) under resumable step functions, for a
 * target with no coroutine stacks (a GPU thread) and for the host bytecode walker. A task is a chain of heap
 * frames; a frame's step function runs from its saved `pc` to its next operation, performs the core's step
 * for it, saves the `pc` to resume at and returns, and the loop runs the next ready task — so every
 * operation is a scheduling point, as it is in the runtime. On resume the step reads what the operation
 * delivered from its task (`resume`, `resume_nothing`, `selected_arm`).
 *
 * A call to a blocking helper pushes a callee frame onto the task and returns LOGOS_STEP_CALL, and the loop
 * steps the new top frame at once; the callee's return pops back to its caller the same way. A run ends when
 * nothing is ready and no timer is pending: when some task never finished it is a deadlock, the
 * interpreter's words, reported through `LOGOS_RUN_FAILS` (the host's stderr and exit status; a trap on a
 * device). */

#define LOGOS_STEP_DONE 0
#define LOGOS_STEP_YIELD 1
#define LOGOS_STEP_CALL 2

#ifndef LOGOS_RUN_FAILS
#define LOGOS_RUN_FAILS(message) asm("trap;")
#endif

/* The header every frame begins with. A task's parameters follow it as the frame's first cells, then its
   locals, each a `long long`. */
typedef struct logos_frame_hdr {
    int pc;
    logos_step_fn step;
    struct logos_frame_hdr *caller;
    long long *result_slot;
} logos_frame_hdr;

LOGOS_VAR logos_task *logos_current = 0;

/* A zeroed frame of `size` bytes running `step` from its start, its first `nargs` cells the arguments. */
LOGOS_FN logos_frame_hdr *logos_new_frame(logos_step_fn step, long long size, long long nargs, const long long *args) {
    char *raw = (char *)logos_malloc((unsigned long)size);
    for (long long i = 0; i < size; i++) raw[i] = 0;
    logos_frame_hdr *h = (logos_frame_hdr *)raw;
    h->step = step;
    long long *cells = (long long *)(raw + sizeof(logos_frame_hdr));
    for (long long i = 0; i < nargs; i++) cells[i] = args[i];
    return h;
}

LOGOS_FN logos_task *logos_new_task(logos_frame_hdr *root) {
    logos_task *t = (logos_task *)logos_malloc(sizeof(logos_task));
    t->step = root->step;
    t->frame = (void *)root;
    t->priority = 0;
    return t;
}

/* `Launch a task to f [with …]`: the child runs before its parent resumes; answers its handle. */
LOGOS_FN long long logos_spawn(logos_step_fn step, long long frame_size, long long nargs, const long long *args) {
    logos_task *child = logos_new_task(logos_new_frame(step, frame_size, nargs, args));
    logos_op_spawn(logos_current, child);
    return (long long)(intptr_t)child;
}

/* A call to a blocking helper: the callee's frame becomes the task's top, its return value bound for
   `result_slot` (0 for none). The step returns LOGOS_STEP_CALL. */
LOGOS_FN void logos_call_push(logos_step_fn step, long long frame_size, long long *result_slot, long long nargs,
                              const long long *args) {
    logos_frame_hdr *h = logos_new_frame(step, frame_size, nargs, args);
    h->caller = (logos_frame_hdr *)logos_current->frame;
    h->result_slot = result_slot;
    logos_current->frame = (void *)h;
    logos_current->step = step;
}

/* A helper's `Return v`: the value into its caller's slot, and the caller is the task's top again. The step
   returns LOGOS_STEP_CALL. */
LOGOS_FN void logos_call_return(long long v) {
    logos_frame_hdr *callee = (logos_frame_hdr *)logos_current->frame;
    logos_frame_hdr *caller = callee->caller;
    if (callee->result_slot) *callee->result_slot = v;
    logos_current->frame = (void *)caller;
    logos_current->step = caller->step;
}

/* Called only after the step returned: no retired frame is executing. */
LOGOS_FN void logos_retire_completed(void) {
    logos_task *t;
    while ((t = logos_queue_pop(&logos_retired_q))) {
        logos_frame_hdr *h = (logos_frame_hdr *)t->frame;
        t->frame = 0;
        t->step = 0;
        while (h) {
            logos_frame_hdr *caller = h->caller;
            logos_release(h);
            h = caller;
        }
    }
}

/* Step one ready task to its next operation, or to its end. */
LOGOS_FN void logos_step_task(logos_task *t) {
    logos_current = t;
    for (;;) {
        int r = t->step(t->frame);
        if (r == LOGOS_STEP_CALL) continue;
        if (r == LOGOS_STEP_DONE) logos_op_exit(t);
        break;
    }
    logos_retire_completed();
}

#ifndef LOGOS_SCHED_WORKERS
#define LOGOS_SCHED_WORKERS 1
#endif

#ifndef LOGOS_SCHED_ROOTS_ENTER
#define LOGOS_SCHED_ROOTS_ENTER(top) ((void)0)
#define LOGOS_SCHED_ROOTS_LEAVE(top) ((void)0)
#define LOGOS_WORKER_STACK_ENTER(top) ((void)0)
#define LOGOS_WORKER_STACK_LEAVE() ((void)0)
#endif

#if LOGOS_SCHED_WORKERS > 1
/* Work-stealing: LOGOS_SCHED_WORKERS threads take from the one ready queue under a lock held across a whole
   step, so the steps happen in the order the single loop takes them and the output does not depend on the
   worker count (host only). */
#include <pthread.h>
static pthread_mutex_t logos_sched_mtx = PTHREAD_MUTEX_INITIALIZER;
static int logos_sched_quit = 0;
static void *logos_worker_main(void *arg) {
    (void)arg;
    volatile uintptr_t stack_top = 0;
    for (;;) {
        pthread_mutex_lock(&logos_sched_mtx);
        if (logos_sched_quit) {
            pthread_mutex_unlock(&logos_sched_mtx);
            return 0;
        }
        LOGOS_WORKER_STACK_ENTER((const void *)&stack_top);
        logos_task *t = logos_pick_ready();
        if (t) {
            logos_step_task(t);
        } else if (!logos_advance_timers()) {
            logos_sched_quit = 1;
        }
        LOGOS_WORKER_STACK_LEAVE();
        pthread_mutex_unlock(&logos_sched_mtx);
    }
}
static void logos_sched_loop(void) {
    volatile uintptr_t stack_top = 0;
    LOGOS_SCHED_ROOTS_ENTER((const void *)&stack_top);
    pthread_t logos_th[LOGOS_SCHED_WORKERS];
    for (int i = 0; i < LOGOS_SCHED_WORKERS; i++) pthread_create(&logos_th[i], 0, logos_worker_main, 0);
    for (int i = 0; i < LOGOS_SCHED_WORKERS; i++) pthread_join(logos_th[i], 0);
    LOGOS_SCHED_ROOTS_LEAVE((const void *)&stack_top);
}
#else
LOGOS_FN void logos_sched_loop(void) {
    for (;;) {
        logos_task *t = logos_pick_ready();
        if (t) {
            logos_step_task(t);
            continue;
        }
        if (!logos_advance_timers()) break;
    }
}
#endif

/* Run `main_step`'s frame as the first task, seeded with `seed`, to quiescence. */
LOGOS_FN void logos_sched_bootstrap(logos_step_fn main_step, long long frame_size, unsigned long long seed) {
    logos_seed(seed);
    logos_task_begin(logos_new_task(logos_new_frame(main_step, frame_size, 0, 0)));
    logos_sched_loop();
    logos_current = 0;
    if (logos_failure) LOGOS_RUN_FAILS(logos_failure);
    if (logos_deadlocked()) LOGOS_RUN_FAILS("deadlock: every task is blocked waiting");
}
