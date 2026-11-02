/* The C family's coroutine tier: the scheduler core (`sched_core.c`) under `ucontext`. Every task runs
 * on a stack of its own; an operation performs the core's step for it, then swaps back to the scheduler
 * loop, which resumes the next ready task — so every operation is a scheduling point, as it is in the
 * runtime. Shared by host C and RISC-V, which differ in `LOGOS_API` — the linkage of the entry points the
 * emitted code calls (static for C, external for RISC-V's assembly) — and in `LOGOS_TASK_CALL`, how a task
 * body is called (RISC-V's take their arguments in registers).
 *
 * A task body is an entry `long long entry(const long long *args)` the tier calls by address with the
 * task's own copy of its launch arguments, however many there are. A run ends when nothing is ready and no
 * timer is pending: it fails with the first task failure, or — when some task never finished — as a
 * deadlock, the interpreter's words for both. */

#ifndef LOGOS_STACK_ENTER
#define LOGOS_STACK_ENTER(lo, hi) ((void)0)
#define LOGOS_STACK_LEAVE() ((void)0)
#endif

#define LOGOS_STACK 262144

static ucontext_t logos_sched_ctx;
static logos_task *logos_current = 0;

static void logos_yield(void) { swapcontext(&logos_current->ctx, &logos_sched_ctx); }

/* A failure inside a task fails the task — the run keeps the first, and reports it once every other task
   is quiescent — and the task never runs again. Installed as the runtime's failure hook for the run. */
static void logos_task_failed(const char *message) {
    if (!logos_current) return;
    logos_op_fail(logos_current, message);
    logos_yield();
}

/* How the tier calls a task body: by default an entry taking the task's argument array. A tier whose
   bodies take their arguments otherwise defines `LOGOS_TASK_CALL(t)` before this file. */
#ifndef LOGOS_TASK_CALL
#define LOGOS_TASK_CALL(t) ((long long (*)(const long long *))(t)->fn)((t)->args)
#endif

static void logos_task_trampoline(void) {
    logos_task *t = logos_current;
    LOGOS_TASK_CALL(t);
    logos_op_exit(t);
}

static logos_task *logos_make_task(void *fn, long long nargs, const long long *args) {
    logos_task *t = (logos_task *)logos_alloc(sizeof(logos_task));
    t->stack = logos_alloc(LOGOS_STACK);
    t->fn = fn;
    t->nargs = nargs;
    t->args = (long long *)logos_alloc((nargs > 0 ? (size_t)nargs : 1) * sizeof(long long));
    for (long long i = 0; i < nargs; i++) t->args[i] = args[i];
    getcontext(&t->ctx);
    t->ctx.uc_stack.ss_sp = t->stack;
    t->ctx.uc_stack.ss_size = LOGOS_STACK;
    t->ctx.uc_link = &logos_sched_ctx;
    makecontext(&t->ctx, logos_task_trampoline, 0);
    return t;
}

/* `Launch a task to f [with …]`: the child runs before its parent resumes; answers its handle, for
   `Stop` and `Await`. */
LOGOS_API long long logos_spawn(void *fn, long long nargs, const long long *args) {
    logos_task *child = logos_make_task(fn, nargs, args);
    logos_op_spawn(logos_current, child);
    logos_yield();
    return (long long)(intptr_t)child;
}

/* `Await h`: resumes once `h` has finished; a task's result is `nothing`. */
LOGOS_API void logos_task_await(logos_task *t) {
    logos_op_await(logos_current, t);
    logos_yield();
}

/* `Stop h`. */
LOGOS_API void logos_task_abort(logos_task *t) {
    logos_op_abort(logos_current, t);
    logos_yield();
}

/* `Let ch be a Pipe of …` (capacity < 0: the default). */
LOGOS_API logos_chan *logos_chan_new(long long capacity) {
    logos_chan *c = logos_op_new_chan(logos_current, capacity);
    logos_yield();
    return c;
}

LOGOS_API void logos_chan_send(logos_chan *c, long long v) {
    logos_op_send(logos_current, c, v);
    logos_yield();
}

/* What the running task's last operation resumed with. */
static logos_opt logos_resumed(void) {
    logos_opt o = { !logos_current->resume_nothing, logos_current->resume_nothing ? 0 : logos_current->resume };
    return o;
}

/* `Receive x from c`: the value, or `nothing` once the pipe is closed and drained. */
LOGOS_API logos_opt logos_chan_recv(logos_chan *c) {
    logos_op_recv(logos_current, c);
    logos_yield();
    return logos_resumed();
}

LOGOS_API void logos_chan_close(logos_chan *c) {
    logos_op_close(logos_current, c);
    logos_yield();
}

/* `Try to send v into c`: 1 when it was taken, else 0. */
LOGOS_API long long logos_chan_trysend(logos_chan *c, long long v) {
    logos_op_try_send(logos_current, c, v);
    logos_yield();
    return logos_current->resume;
}

/* `Try to receive x from c`: a value, or `nothing` when none is buffered. */
LOGOS_API logos_opt logos_chan_tryrecv(logos_chan *c) {
    logos_op_try_recv(logos_current, c);
    logos_yield();
    return logos_resumed();
}

/* `Sleep n`: parks on the logical clock; a sleep of no time is no scheduling point, as the interpreter
   makes it none. */
LOGOS_API void logos_sleep(long long ticks) {
    if (ticks <= 0) return;
    logos_op_sleep(logos_current, ticks);
    logos_yield();
}

/* `Await the first of:` — the arms are staged in declaration order (reset, then each arm), then
   `logos_select_wait` answers the winning arm's index, having written what a receive arm read — a value,
   or `nothing` from a closed pipe — into its slot. The staging is copied to the waiting task's own stack before it yields, so another task's select
   can stage in between. */
static int logos_sel_n = 0;
static int logos_sel_kind[LOGOS_SEL_MAX];
static logos_chan *logos_sel_chan[LOGOS_SEL_MAX];
static long long logos_sel_ticks[LOGOS_SEL_MAX];
static logos_opt *logos_sel_slot[LOGOS_SEL_MAX];

LOGOS_API void logos_select_reset(void) { logos_sel_n = 0; }

LOGOS_API void logos_select_arm_recv(logos_chan *c, logos_opt *slot) {
    logos_sel_kind[logos_sel_n] = LOGOS_ARM_RECV;
    logos_sel_chan[logos_sel_n] = c;
    logos_sel_ticks[logos_sel_n] = 0;
    logos_sel_slot[logos_sel_n] = slot;
    logos_sel_n++;
}

LOGOS_API void logos_select_arm_timeout(long long ticks) {
    logos_sel_kind[logos_sel_n] = LOGOS_ARM_AFTER;
    logos_sel_chan[logos_sel_n] = 0;
    logos_sel_ticks[logos_sel_n] = ticks;
    logos_sel_slot[logos_sel_n] = 0;
    logos_sel_n++;
}

LOGOS_API long long logos_select_wait(void) {
    int n = logos_sel_n;
    int kind[LOGOS_SEL_MAX];
    logos_chan *chan[LOGOS_SEL_MAX];
    long long ticks[LOGOS_SEL_MAX];
    logos_opt *slot[LOGOS_SEL_MAX];
    for (int i = 0; i < n; i++) {
        kind[i] = logos_sel_kind[i];
        chan[i] = logos_sel_chan[i];
        ticks[i] = logos_sel_ticks[i];
        slot[i] = logos_sel_slot[i];
    }
    logos_sel_n = 0;
    logos_op_select(logos_current, n, kind, chan, ticks);
    logos_yield();
    int arm = logos_current->selected_arm;
    if (kind[arm] == LOGOS_ARM_RECV && slot[arm]) *slot[arm] = logos_resumed();
    return arm;
}

/* The scheduler stack is active here; a finished task's stack can now be released. */
static void logos_retire_completed(void) {
    logos_task *t;
    while ((t = logos_queue_pop(&logos_retired_q))) {
        logos_release(t->stack);
        logos_release(t->args);
        t->stack = 0;
        t->args = 0;
        t->fn = 0;
        t->nargs = 0;
        memset(&t->ctx, 0, sizeof(t->ctx));
    }
}

/* Run `main_fn` as the first task and drive the scheduler to quiescence. */
LOGOS_API void logos_sched_bootstrap(void *main_fn) {
    logos_seed(logos_env_seed(0));
    logos_fail_hook = logos_task_failed;
    logos_task_begin(logos_make_task(main_fn, 0, 0));
    for (;;) {
        logos_task *t = logos_pick_ready();
        if (t) {
            logos_current = t;
            LOGOS_STACK_ENTER(t->stack, (char *)t->stack + LOGOS_STACK);
            swapcontext(&logos_sched_ctx, &t->ctx);
            LOGOS_STACK_LEAVE();
            logos_retire_completed();
            continue;
        }
        if (!logos_advance_timers()) break;
    }
    logos_current = 0;
    logos_fail_hook = 0;
    fflush(stdout);
    if (logos_failure) {
        fprintf(stderr, "%s\n", logos_failure);
        exit(1);
    }
    if (logos_deadlocked()) {
        fprintf(stderr, "deadlock: every task is blocked waiting\n");
        exit(1);
    }
}
