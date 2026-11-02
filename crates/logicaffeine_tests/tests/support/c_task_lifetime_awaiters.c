int main(int argc, char **argv) {
    int mode = atoi(argv[1]);
    logos_task *target = logos_alloc(sizeof(logos_task));
    logos_task *waiter = logos_alloc(sizeof(logos_task));
    target->state = LOGOS_READY;
    logos_op_await(waiter, target);
    if (waiter->state != LOGOS_BLOCKED_AWAIT || target->awaiters.len != 1) return 8;
    size_t expected = lc_gc_takes(sizeof(logos_task));
    if (mode == 0) {
        logos_op_exit(target);
        if (target->state != LOGOS_DONE || waiter->state != LOGOS_READY ||
            !waiter->resume_nothing || target->awaiters.len) return 8;
    } else if (mode == 1) {
        if (logos_queue_at(&target->awaiters, 0) != waiter || waiter->awaits != target) return 8;
        expected += lc_gc_takes(sizeof(logos_task)) + lc_gc_takes(8 * sizeof(logos_task *));
    } else return 9;
    size_t actual = trace_one(&target, sizeof target);
    printf("mode=%d expected=%zu actual=%zu\n", mode, expected, actual);
    return expected == actual ? 0 : 7;
}
