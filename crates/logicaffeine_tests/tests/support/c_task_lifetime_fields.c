int main(int argc, char **argv) {
    int mode = atoi(argv[1]);
    void *root;
    logos_task *t = logos_alloc(sizeof(logos_task));
    size_t expected = lc_gc_takes(sizeof(logos_task)), actual;
    if (mode == 0) {
        logos_chan *c = logos_alloc(sizeof(logos_chan));
        t->waits_on = c; t->state = LOGOS_BLOCKED_RECV;
        logos_mark_ready(t, 42, 0);
        if (t->state != LOGOS_READY || t->resume != 42 || t->resume_nothing) return 8;
        root = t;
    } else if (mode == 1) {
        logos_task *target = logos_alloc(sizeof(logos_task));
        target->state = LOGOS_READY;
        logos_op_await(t, target);
        if (t->state != LOGOS_BLOCKED_AWAIT) return 8;
        logos_op_exit(target);
        if (t->state != LOGOS_READY || !t->resume_nothing || target->state != LOGOS_DONE) return 8;
        root = t;
    } else if (mode == 2) {
        logos_chan *c = logos_alloc(sizeof(logos_chan));
        void *payload = logos_alloc(2048);
        logos_op_send(t, c, (long long)(intptr_t)payload);
        if (t->state != LOGOS_BLOCKED_SEND) return 8;
        long long value; int nothing;
        logos_take_one(c, &value, &nothing);
        if (nothing || value != (long long)(intptr_t)payload || t->state != LOGOS_READY) return 8;
        root = t;
    } else if (mode == 3) {
        logos_chan *c = logos_alloc(sizeof(logos_chan));
        int kinds[1] = {LOGOS_ARM_RECV};
        logos_chan *chans[1] = {c};
        long long ticks[1] = {0};
        logos_op_select(t, 1, kinds, chans, ticks);
        if (t->state != LOGOS_BLOCKED_SELECT) return 8;
        logos_resolve_select(t, 0, 91, 0);
        if (t->selected_arm != 0 || t->resume != 91 || t->resume_nothing || c->receivers.len) return 8;
        root = t;
    } else if (mode == 4) {
        t->resume = (long long)(intptr_t)logos_alloc(2048);
        t->resume_nothing = 0;
        logos_op_exit(t);
        if (t->state != LOGOS_DONE) return 8;
        root = t;
    } else if (mode == 5) {
        logos_chan *c = logos_alloc(sizeof(logos_chan));
        logos_task closer = {0};
        logos_op_recv(t, c);
        if (t->state != LOGOS_BLOCKED_RECV) return 8;
        logos_op_close(&closer, c);
        if (!c->closed || t->state != LOGOS_READY || !t->resume_nothing || c->receivers.len) return 8;
        logos_queue_remove(&logos_ready_q, t);
        logos_op_exit(t);
        root = c;
        expected = lc_gc_takes(sizeof(logos_chan)) + lc_gc_takes(8 * sizeof(logos_task *));
    } else return 9;
    actual = trace_one(&root, sizeof root);
    printf("mode=%d expected=%zu actual=%zu\n", mode, expected, actual);
    return actual == expected ? 0 : 7;
}
