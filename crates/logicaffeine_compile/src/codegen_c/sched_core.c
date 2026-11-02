/* The LOGOS scheduler — `logicaffeine_runtime::Scheduler::process_step`, ported step for step.
 *
 * ONE policy core for every C-family tier: the coroutine runtimes (host C, RISC-V) and the stackless
 * frames (CUDA, the host bytecode walker). An operation is a state transition over the task table: it
 * mutates the queues and leaves the task it was handed either READY at the back of the ready queue,
 * holding the value it resumes with, or blocked off the queue. A tier supplies only the yield — switching
 * back to its scheduler loop's context, or saving a frame's pc and returning — and reads `resume`
 * when the task runs again. Every concurrency operation is a scheduling point, as it is in the runtime.
 *
 * Before this text a tier defines:
 *   LOGOS_FN         the qualifier of a core function (`static`, or `__device__ static`)
 *   LOGOS_VAR        the qualifier of a core variable
 *   logos_alloc(n)   / logos_release(p)
 *   LOGOS_TASK_TIER  the tier's own fields of a task: its context, or its frame
 * and may define LOGOS_SCHED_POLICY and LOGOS_CHAN_DEFAULT_CAP (defaults below: the runtime's own).
 */

#ifndef LOGOS_SCHED_POLICY
#define LOGOS_SCHED_FIFO 0
#define LOGOS_SCHED_LIFO 1
#define LOGOS_SCHED_ROUNDROBIN 2
#define LOGOS_SCHED_RANDOM 3
#define LOGOS_SCHED_PRIORITY 4
#define LOGOS_SCHED_POLICY LOGOS_SCHED_FIFO
#endif
/* A pipe with no written capacity holds `SchedulerConfig::default_channel_capacity` values. */
#ifndef LOGOS_CHAN_DEFAULT_CAP
#define LOGOS_CHAN_DEFAULT_CAP 32LL
#endif
#ifndef LOGOS_SEL_MAX
#define LOGOS_SEL_MAX 16
#endif

#define LOGOS_READY 0
#define LOGOS_BLOCKED_RECV 1
#define LOGOS_BLOCKED_SEND 2
#define LOGOS_BLOCKED_TIMER 3
#define LOGOS_BLOCKED_SELECT 4
#define LOGOS_BLOCKED_AWAIT 5
#define LOGOS_DONE 6

#define LOGOS_ARM_RECV 0
#define LOGOS_ARM_AFTER 1

typedef struct logos_task logos_task;
typedef struct logos_chan logos_chan;

/* Tasks in arrival order, oldest first. */
typedef struct {
    logos_task **at;
    int head, len, cap;
} logos_queue;

struct logos_task {
    int state;
    long long resume;         /* the value the operation that woke it delivers */
    int resume_nothing;       /* it delivered `nothing`: a closed pipe, an empty try, an awaited task */
    int selected_arm;         /* the arm a select resolved to */
    long long send_value;     /* the value a blocked send holds */
    logos_chan *waits_on;     /* the pipe a blocked receive or send waits on */
    logos_task *awaits;       /* the task a blocked await waits for */
    int sel_n;                /* a waiting select's arms, in declaration order */
    int sel_kind[LOGOS_SEL_MAX];
    logos_chan *sel_chan[LOGOS_SEL_MAX];
    logos_queue awaiters;     /* the tasks awaiting this one, in the order they began to */
    long long priority;
    LOGOS_TASK_TIER
};

struct logos_chan {
    long long *buf;
    int head, len, cap_alloc;
    long long capacity;
    int closed;
    logos_queue receivers;    /* blocked receives; a waiting select once per arm naming this pipe */
    logos_queue senders;      /* blocked sends, each holding its value in `send_value` */
};

typedef struct {
    long long fire_at;
    logos_task *task;
    int arm;                  /* the select arm it resolves, or -1 for a `Sleep` */
} logos_timer;

LOGOS_VAR logos_queue logos_ready_q;
/* Finished tasks retain their storage until the scheduler has left their execution context. */
LOGOS_VAR logos_queue logos_retired_q;
LOGOS_VAR long long logos_clock = 0;
LOGOS_VAR unsigned long long logos_rng = 0;
LOGOS_VAR logos_timer *logos_timers = 0;
LOGOS_VAR int logos_timer_n = 0, logos_timer_cap = 0;
LOGOS_VAR logos_task **logos_all = 0;
LOGOS_VAR int logos_all_n = 0, logos_all_cap = 0;
/* The first failure of any task; the run fails with it once every other task is quiescent. */
LOGOS_VAR const char *logos_failure = 0;

/* ---- the chooser: `SeededRng`, the runtime's one source of scheduling choice ---- */

LOGOS_FN void logos_seed(unsigned long long seed) { logos_rng = seed; }

LOGOS_FN unsigned long long logos_rng_next(void) {
    logos_rng += 0x9E3779B97F4A7C15ULL;
    unsigned long long z = logos_rng;
    z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ULL;
    z = (z ^ (z >> 27)) * 0x94D049BB133111EBULL;
    return z ^ (z >> 31);
}

/* An index in [0, n); one option draws nothing, as `SeededRng::below` does. */
LOGOS_FN int logos_choose(int n) {
    if (n <= 1) return 0;
    return (int)(logos_rng_next() % (unsigned long long)n);
}

/* ---- queues ---- */

LOGOS_FN logos_task *logos_queue_at(logos_queue *q, int k) { return q->at[(q->head + k) % q->cap]; }

LOGOS_FN void logos_queue_push(logos_queue *q, logos_task *t) {
    if (q->len == q->cap) {
        int cap = q->cap ? q->cap * 2 : 8;
        logos_task **at = (logos_task **)logos_alloc((unsigned long)cap * sizeof(logos_task *));
        for (int i = 0; i < q->len; i++) at[i] = logos_queue_at(q, i);
        if (q->at) logos_release(q->at);
        q->at = at;
        q->head = 0;
        q->cap = cap;
    }
    q->at[(q->head + q->len) % q->cap] = t;
    q->len++;
}

/* Remove and return the k-th oldest. */
LOGOS_FN logos_task *logos_queue_take(logos_queue *q, int k) {
    logos_task *t = logos_queue_at(q, k);
    if (k == 0) {
        q->at[q->head] = 0;
        q->head = (q->head + 1) % q->cap;
    } else {
        for (int i = k; i + 1 < q->len; i++) q->at[(q->head + i) % q->cap] = logos_queue_at(q, i + 1);
        q->at[(q->head + q->len - 1) % q->cap] = 0;
    }
    q->len--;
    return t;
}

LOGOS_FN logos_task *logos_queue_pop(logos_queue *q) { return q->len ? logos_queue_take(q, 0) : 0; }

/* Drop every entry that is `t`, keeping the others in order. */
LOGOS_FN void logos_queue_remove(logos_queue *q, logos_task *t) {
    int kept = 0;
    for (int i = 0; i < q->len; i++) {
        logos_task *x = logos_queue_at(q, i);
        if (x != t) q->at[(q->head + kept++) % q->cap] = x;
    }
    for (int i = kept; i < q->len; i++) q->at[(q->head + i) % q->cap] = 0;
    q->len = kept;
}

/* ---- timers ---- */

LOGOS_FN void logos_timer_add(long long fire_at, logos_task *t, int arm) {
    if (logos_timer_n == logos_timer_cap) {
        int cap = logos_timer_cap ? logos_timer_cap * 2 : 16;
        logos_timer *at = (logos_timer *)logos_alloc((unsigned long)cap * sizeof(logos_timer));
        for (int i = 0; i < logos_timer_n; i++) at[i] = logos_timers[i];
        if (logos_timers) logos_release(logos_timers);
        logos_timers = at;
        logos_timer_cap = cap;
    }
    logos_timers[logos_timer_n].fire_at = fire_at;
    logos_timers[logos_timer_n].task = t;
    logos_timers[logos_timer_n].arm = arm;
    logos_timer_n++;
}

/* Drop every timer of `t`, keeping the others in order. */
LOGOS_FN void logos_timer_remove(logos_task *t) {
    int kept = 0;
    for (int i = 0; i < logos_timer_n; i++) {
        if (logos_timers[i].task != t) logos_timers[kept++] = logos_timers[i];
    }
    for (int i = kept; i < logos_timer_n; i++) logos_timers[i].task = 0;
    logos_timer_n = kept;
}

/* ---- tasks ---- */

LOGOS_FN void logos_mark_ready(logos_task *t, long long value, int nothing) {
    t->waits_on = 0;
    t->awaits = 0;
    t->send_value = 0;
    t->resume = value;
    t->resume_nothing = nothing;
    t->state = LOGOS_READY;
    logos_queue_push(&logos_ready_q, t);
}

/* A new task joins the table ready — the runtime's `spawn_inner`. */
LOGOS_FN void logos_task_begin(logos_task *t) {
    t->resume = 0;
    t->resume_nothing = 1;
    t->selected_arm = -1;
    t->send_value = 0;
    t->waits_on = 0;
    t->awaits = 0;
    t->sel_n = 0;
    t->awaiters.at = 0;
    t->awaiters.head = t->awaiters.len = t->awaiters.cap = 0;
    if (logos_all_n == logos_all_cap) {
        int cap = logos_all_cap ? logos_all_cap * 2 : 16;
        logos_task **at = (logos_task **)logos_alloc((unsigned long)cap * sizeof(logos_task *));
        for (int i = 0; i < logos_all_n; i++) at[i] = logos_all[i];
        if (logos_all) logos_release(logos_all);
        logos_all = at;
        logos_all_cap = cap;
    }
    logos_all[logos_all_n++] = t;
    t->state = LOGOS_READY;
    logos_queue_push(&logos_ready_q, t);
}

/* Wake everything awaiting `t`, which has finished: each resumes with `nothing`, the result every
   C-family task carries (a task exits `Nothing` on the tree-walker). */
LOGOS_FN void logos_complete(logos_task *t) {
    t->resume = 0;
    t->resume_nothing = 1;
    t->waits_on = 0;
    t->awaits = 0;
    t->send_value = 0;
    for (int i = 0; i < LOGOS_SEL_MAX; i++) t->sel_chan[i] = 0;
    t->sel_n = 0;
    logos_task *w;
    while ((w = logos_queue_pop(&t->awaiters))) logos_mark_ready(w, 0, 1);
    logos_release(t->awaiters.at);
    t->awaiters.at = 0;
    t->awaiters.cap = t->awaiters.head = t->awaiters.len = 0;
    int kept = 0;
    for (int i = 0; i < logos_all_n; i++) {
        if (logos_all[i] != t) logos_all[kept++] = logos_all[i];
    }
    for (int i = kept; i < logos_all_n; i++) logos_all[i] = 0;
    logos_all_n = kept;
    logos_queue_push(&logos_retired_q, t);
}

/* ---- pipes ---- */

LOGOS_FN int logos_chan_has_room(logos_chan *c) { return c->len < c->capacity; }

LOGOS_FN int logos_chan_can_recv(logos_chan *c) { return c->len > 0 || c->senders.len > 0 || c->closed; }

LOGOS_FN void logos_chan_buffer(logos_chan *c, long long v) {
    if (c->len == c->cap_alloc) {
        int cap = c->cap_alloc * 2;
        long long *buf = (long long *)logos_alloc((unsigned long)cap * sizeof(long long));
        for (int i = 0; i < c->len; i++) buf[i] = c->buf[(c->head + i) % c->cap_alloc];
        logos_release(c->buf);
        c->buf = buf;
        c->head = 0;
        c->cap_alloc = cap;
    }
    c->buf[(c->head + c->len) % c->cap_alloc] = v;
    c->len++;
}

LOGOS_FN int logos_chan_unbuffer(logos_chan *c, long long *v) {
    if (c->len == 0) return 0;
    *v = c->buf[c->head];
    c->buf[c->head] = 0;
    c->head = (c->head + 1) % c->cap_alloc;
    c->len--;
    return 1;
}

LOGOS_FN void logos_refill_from_blocked_sender(logos_chan *c) {
    if (!logos_chan_has_room(c)) return;
    logos_task *s = logos_queue_pop(&c->senders);
    if (s) {
        logos_chan_buffer(c, s->send_value);
        logos_mark_ready(s, 0, 1);
    }
}

/* Withdraw every registration a woken select made — on each pipe it names, the one that woke it too,
   since a select naming one pipe in two arms waits on it twice — and its timeouts. */
LOGOS_FN void logos_cancel_select(logos_task *t) {
    for (int i = 0; i < t->sel_n; i++) {
        if (t->sel_kind[i] == LOGOS_ARM_RECV) logos_queue_remove(&t->sel_chan[i]->receivers, t);
        t->sel_chan[i] = 0;
    }
    t->sel_n = 0;
    logos_timer_remove(t);
}

/* The first arm of `t`'s waiting select that receives from `c`. */
LOGOS_FN int logos_arm_on(logos_task *t, logos_chan *c) {
    for (int i = 0; i < t->sel_n; i++) {
        if (t->sel_kind[i] == LOGOS_ARM_RECV && t->sel_chan[i] == c) return i;
    }
    return 0;
}

LOGOS_FN void logos_resolve_select(logos_task *t, int arm, long long value, int nothing) {
    t->resume = value;
    t->resume_nothing = nothing;
    t->selected_arm = arm;
    t->state = LOGOS_READY;
    logos_queue_push(&logos_ready_q, t);
    logos_cancel_select(t);
}

LOGOS_FN void logos_deliver(logos_task *r, logos_chan *c, long long v) {
    if (r->state == LOGOS_BLOCKED_SELECT) {
        logos_resolve_select(r, logos_arm_on(r, c), v, 0);
    } else {
        logos_mark_ready(r, v, 0);
    }
}

/* One value from a pipe `logos_chan_can_recv` said is ready: buffered, from a blocked sender, or
   `nothing` from a closed pipe. */
LOGOS_FN void logos_take_one(logos_chan *c, long long *v, int *nothing) {
    if (logos_chan_unbuffer(c, v)) {
        *nothing = 0;
        logos_refill_from_blocked_sender(c);
        return;
    }
    logos_task *s = logos_queue_pop(&c->senders);
    if (s) {
        *v = s->send_value;
        logos_mark_ready(s, 0, 1);
        *nothing = 0;
        return;
    }
    *v = 0;
    *nothing = 1;
}

/* ---- the operations: one `process_step` arm each ---- */

LOGOS_FN void logos_op_yield(logos_task *t) { logos_mark_ready(t, 0, 1); }

LOGOS_FN logos_chan *logos_op_new_chan(logos_task *t, long long capacity) {
    logos_chan *c = (logos_chan *)logos_alloc(sizeof(logos_chan));
    c->cap_alloc = 8;
    c->buf = (long long *)logos_alloc((unsigned long)c->cap_alloc * sizeof(long long));
    c->head = 0;
    c->len = 0;
    c->capacity = capacity < 0 ? LOGOS_CHAN_DEFAULT_CAP : capacity;
    c->closed = 0;
    c->receivers.at = 0;
    c->receivers.head = c->receivers.len = c->receivers.cap = 0;
    c->senders.at = 0;
    c->senders.head = c->senders.len = c->senders.cap = 0;
    logos_mark_ready(t, 0, 1);
    return c;
}

/* The child joins the ready queue, then the parent behind it: the child runs before its parent resumes. */
LOGOS_FN void logos_op_spawn(logos_task *t, logos_task *child) {
    logos_task_begin(child);
    logos_mark_ready(t, 0, 1);
}

LOGOS_FN void logos_op_send(logos_task *t, logos_chan *c, long long v) {
    logos_task *r = logos_queue_pop(&c->receivers);
    if (r) {
        logos_deliver(r, c, v);
        logos_mark_ready(t, 0, 1);
        return;
    }
    if (logos_chan_has_room(c)) {
        logos_chan_buffer(c, v);
        logos_mark_ready(t, 0, 1);
        return;
    }
    t->send_value = v;
    t->waits_on = c;
    logos_queue_push(&c->senders, t);
    t->state = LOGOS_BLOCKED_SEND;
}

LOGOS_FN void logos_op_recv(logos_task *t, logos_chan *c) {
    long long v;
    if (logos_chan_unbuffer(c, &v)) {
        logos_mark_ready(t, v, 0);
        logos_refill_from_blocked_sender(c);
        return;
    }
    logos_task *s = logos_queue_pop(&c->senders);
    if (s) {
        logos_mark_ready(t, s->send_value, 0);
        logos_mark_ready(s, 0, 1);
        return;
    }
    if (c->closed) {
        logos_mark_ready(t, 0, 1);
        return;
    }
    t->waits_on = c;
    logos_queue_push(&c->receivers, t);
    t->state = LOGOS_BLOCKED_RECV;
}

/* Never blocks: resumes with 1 when the value was taken, else 0 (the value is dropped). */
LOGOS_FN void logos_op_try_send(logos_task *t, logos_chan *c, long long v) {
    logos_task *r = logos_queue_pop(&c->receivers);
    if (r) {
        logos_deliver(r, c, v);
        logos_mark_ready(t, 1, 0);
        return;
    }
    if (logos_chan_has_room(c)) {
        logos_chan_buffer(c, v);
        logos_mark_ready(t, 1, 0);
        return;
    }
    logos_mark_ready(t, 0, 0);
}

/* Never blocks: resumes with a value, or with `nothing` when there is none. */
LOGOS_FN void logos_op_try_recv(logos_task *t, logos_chan *c) {
    long long v;
    if (logos_chan_unbuffer(c, &v)) {
        logos_mark_ready(t, v, 0);
        logos_refill_from_blocked_sender(c);
        return;
    }
    logos_task *s = logos_queue_pop(&c->senders);
    if (s) {
        logos_mark_ready(t, s->send_value, 0);
        logos_mark_ready(s, 0, 1);
        return;
    }
    logos_mark_ready(t, 0, 1);
}

/* A select: among its receive arms ready now, one wins — the seeded chooser picks when several are —
   else it waits on every receive arm and every timeout at once. */
LOGOS_FN void logos_op_select(logos_task *t, int n, const int *kind, logos_chan *const *chan, const long long *ticks) {
    int ready[LOGOS_SEL_MAX];
    int k = 0;
    for (int i = 0; i < n; i++) {
        if (kind[i] == LOGOS_ARM_RECV && logos_chan_can_recv(chan[i])) ready[k++] = i;
    }
    if (k > 0) {
        int arm = ready[logos_choose(k)];
        long long v;
        int nothing;
        logos_take_one(chan[arm], &v, &nothing);
        t->resume = v;
        t->resume_nothing = nothing;
        t->selected_arm = arm;
        t->state = LOGOS_READY;
        logos_queue_push(&logos_ready_q, t);
        return;
    }
    t->sel_n = n;
    for (int i = 0; i < n; i++) {
        t->sel_kind[i] = kind[i];
        t->sel_chan[i] = kind[i] == LOGOS_ARM_RECV ? chan[i] : 0;
        if (kind[i] == LOGOS_ARM_RECV) {
            logos_queue_push(&chan[i]->receivers, t);
        } else {
            logos_timer_add(logos_clock + (ticks[i] < 0 ? 0 : ticks[i]), t, i);
        }
    }
    t->state = LOGOS_BLOCKED_SELECT;
}

LOGOS_FN void logos_op_sleep(logos_task *t, long long ticks) {
    logos_timer_add(logos_clock + (ticks < 0 ? 0 : ticks), t, -1);
    t->state = LOGOS_BLOCKED_TIMER;
}

LOGOS_FN void logos_op_await(logos_task *t, logos_task *target) {
    if (!target || target->state == LOGOS_DONE) {
        logos_mark_ready(t, 0, 1);
        return;
    }
    t->awaits = target;
    logos_queue_push(&target->awaiters, t);
    t->state = LOGOS_BLOCKED_AWAIT;
}

/* `Stop h`: the target leaves every queue it waits in and finishes, waking its awaiters with
   `nothing`; the stopping task goes on. A finished target is untouched. */
LOGOS_FN void logos_op_abort(logos_task *t, logos_task *target) {
    if (target && target->state != LOGOS_DONE) {
        logos_queue_remove(&logos_ready_q, target);
        switch (target->state) {
            case LOGOS_BLOCKED_RECV: logos_queue_remove(&target->waits_on->receivers, target); break;
            case LOGOS_BLOCKED_SEND: logos_queue_remove(&target->waits_on->senders, target); break;
            case LOGOS_BLOCKED_SELECT:
                for (int i = 0; i < target->sel_n; i++) {
                    if (target->sel_kind[i] == LOGOS_ARM_RECV) logos_queue_remove(&target->sel_chan[i]->receivers, target);
                }
                break;
            case LOGOS_BLOCKED_AWAIT: logos_queue_remove(&target->awaits->awaiters, target); break;
            default: break;
        }
        logos_timer_remove(target);
        target->state = LOGOS_DONE;
        logos_complete(target);
    }
    logos_mark_ready(t, 0, 1);
}

/* `Close c`: every blocked receive resumes with `nothing` — a waiting select once, on its first arm
   naming `c` — and later receives of the drained pipe answer `nothing` too. */
LOGOS_FN void logos_op_close(logos_task *t, logos_chan *c) {
    c->closed = 1;
    int n = c->receivers.len;
    logos_task **waiting = n ? (logos_task **)logos_alloc((unsigned long)n * sizeof(logos_task *)) : 0;
    for (int i = 0; i < n; i++) waiting[i] = logos_queue_at(&c->receivers, i);
    for (int i = 0; i < c->receivers.cap; i++) c->receivers.at[i] = 0;
    c->receivers.len = 0;
    c->receivers.head = 0;
    for (int i = 0; i < n; i++) {
        logos_task *r = waiting[i];
        if (r->state == LOGOS_BLOCKED_SELECT) {
            logos_resolve_select(r, logos_arm_on(r, c), 0, 1);
        } else if (r->state == LOGOS_BLOCKED_RECV) {
            logos_mark_ready(r, 0, 1);
        }
    }
    if (waiting) logos_release(waiting);
    logos_mark_ready(t, 0, 1);
}

/* The task's body returned. */
LOGOS_FN void logos_op_exit(logos_task *t) {
    t->state = LOGOS_DONE;
    logos_complete(t);
}

/* The task failed: the run keeps its first failure and every other task still runs to quiescence. */
LOGOS_FN void logos_op_fail(logos_task *t, const char *message) {
    if (!logos_failure) logos_failure = message;
    logos_op_exit(t);
}

/* ---- the scheduler loop's two moves ---- */

/* The next task to run, by the policy knob. */
LOGOS_FN logos_task *logos_pick_ready(void) {
    int n = logos_ready_q.len;
    if (n == 0) return 0;
    int k = 0;
#if LOGOS_SCHED_POLICY == LOGOS_SCHED_LIFO
    k = n - 1;
#elif LOGOS_SCHED_POLICY == LOGOS_SCHED_RANDOM
    k = logos_choose(n);
#elif LOGOS_SCHED_POLICY == LOGOS_SCHED_PRIORITY
    long long best = logos_queue_at(&logos_ready_q, 0)->priority;
    for (int i = 1; i < n; i++) {
        long long p = logos_queue_at(&logos_ready_q, i)->priority;
        if (p > best) {
            best = p;
            k = i;
        }
    }
#endif
    return logos_queue_take(&logos_ready_q, k);
}

/* Nothing is ready: the clock jumps to the earliest timer and every timer due then fires, same-tick
   ties in the order the seeded chooser draws. Answers 0 when no timer is pending. */
LOGOS_FN int logos_advance_timers(void) {
    if (logos_timer_n == 0) return 0;
    long long earliest = logos_timers[0].fire_at;
    for (int i = 1; i < logos_timer_n; i++) {
        if (logos_timers[i].fire_at < earliest) earliest = logos_timers[i].fire_at;
    }
    logos_clock = earliest;
    int due_n = 0;
    for (int i = 0; i < logos_timer_n; i++) due_n += logos_timers[i].fire_at == earliest;
    logos_timer *due = (logos_timer *)logos_alloc((unsigned long)due_n * sizeof(logos_timer));
    int d = 0, kept = 0;
    for (int i = 0; i < logos_timer_n; i++) {
        if (logos_timers[i].fire_at == earliest) {
            due[d++] = logos_timers[i];
        } else {
            logos_timers[kept++] = logos_timers[i];
        }
    }
    for (int i = kept; i < logos_timer_n; i++) logos_timers[i].task = 0;
    logos_timer_n = kept;
    int *pool = (int *)logos_alloc((unsigned long)due_n * sizeof(int));
    for (int i = 0; i < due_n; i++) pool[i] = i;
    int left = due_n;
    while (left > 0) {
        int pick = logos_choose(left);
        logos_timer fired = due[pool[pick]];
        for (int i = pick; i + 1 < left; i++) pool[i] = pool[i + 1];
        left--;
        logos_task *t = fired.task;
        if (t->state != LOGOS_BLOCKED_TIMER && t->state != LOGOS_BLOCKED_SELECT) continue;
        if (fired.arm >= 0) {
            int selecting = t->state == LOGOS_BLOCKED_SELECT;
            t->selected_arm = fired.arm;
            t->resume = 0;
            t->resume_nothing = 1;
            t->state = LOGOS_READY;
            logos_queue_push(&logos_ready_q, t);
            if (selecting) logos_cancel_select(t);
        } else {
            logos_mark_ready(t, 0, 1);
        }
    }
    logos_release(pool);
    logos_release(due);
    return 1;
}

/* At quiescence: some task never finished — the run deadlocked. */
LOGOS_FN int logos_deadlocked(void) {
    for (int i = 0; i < logos_all_n; i++) {
        if (logos_all[i]->state != LOGOS_DONE) return 1;
    }
    return 0;
}
