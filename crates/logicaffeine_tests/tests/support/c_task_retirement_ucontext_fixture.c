
static logos_task *held;
static long long body(const long long *args) {
    (void)args;
    held = logos_current;
    return 0;
}
int main(void) {
    logos_sched_bootstrap((void *)body);
    if (!held || held->state != LOGOS_DONE) return 8;
    size_t expected = lc_gc_takes(sizeof(logos_task));
    size_t actual = trace_one(&held, sizeof held);
    printf("expected=%zu actual=%zu\n", expected, actual);
    return actual == expected ? 0 : 7;
}
