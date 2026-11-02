
static size_t trace_only(const void *root, size_t size, int registry) {
    lc_gc_prepare(); lc_gc_pending_n = 0;
    lc_gc_scan((uintptr_t)root, (uintptr_t)root + size);
    if (registry) {
        lc_gc_scan((uintptr_t)&logos_ready_q, (uintptr_t)(&logos_ready_q + 1));
    }
    while (lc_gc_pending_n) {
        lc_gc_range r = lc_gc_pending[--lc_gc_pending_n];
        lc_gc_scan(r.lo, r.hi);
    }
    size_t unused = 0;
    return lc_gc_live = lc_gc_sweep(&unused);
}
int main(int argc, char **argv) {
    int mode = atoi(argv[1]);
    size_t expected, actual;
    logos_task *a = logos_alloc(sizeof(logos_task));
    logos_task *b = logos_alloc(sizeof(logos_task));
    if (mode <= 2) {
        logos_queue q = {0};
        logos_queue_push(&q, a); logos_queue_push(&q, b);
        expected = lc_gc_live - lc_gc_takes(sizeof(logos_task));
        if (mode == 0) { if (logos_queue_pop(&q) != a) return 8; }
        else if (mode == 1) { if (logos_queue_take(&q, 1) != b) return 8; }
        else logos_queue_remove(&q, b);
        if (q.len != 1 || logos_queue_at(&q, 0) != (mode == 0 ? b : a)) return 8;
        actual = trace_only(&q, sizeof q, 0);
    } else if (mode == 3) {
        logos_timer_add(1, a, -1); logos_timer_add(2, b, -1);
        expected = lc_gc_live - lc_gc_takes(sizeof(logos_task));
        logos_timer_remove(b);
        if (logos_timer_n != 1 || logos_timers[0].task != a) return 8;
        actual = trace_only(&logos_timers, sizeof logos_timers, 0);
    } else if (mode == 4) {
        logos_chan c = {0}; c.capacity = 2; c.cap_alloc = 2;
        c.buf = logos_alloc(2 * sizeof(long long));
        logos_chan_buffer(&c, (long long)(intptr_t)a);
        logos_chan_buffer(&c, (long long)(intptr_t)b);
        expected = lc_gc_live - lc_gc_takes(sizeof(logos_task));
        long long got = 0;
        if (!logos_chan_unbuffer(&c, &got) || got != (long long)(intptr_t)a || c.len != 1) return 8;
        actual = trace_only(&c, sizeof c, 0);
    } else if (mode == 5) {
        logos_task_begin(a); logos_task_begin(b);
        logos_queue_remove(&logos_ready_q, b);
        expected = lc_gc_live - lc_gc_takes(sizeof(logos_task));
        logos_op_exit(b);
        if (b->state != LOGOS_DONE || logos_deadlocked() != 1) return 8;
        actual = trace_only(&logos_all, sizeof logos_all, 1);
    } else return 9;
    printf("mode=%d expected=%zu actual=%zu\n", mode, expected, actual);
    return expected == actual ? 0 : 7;
}
