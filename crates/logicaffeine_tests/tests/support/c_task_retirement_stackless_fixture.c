
static int done(void *frame) { (void)frame; return LOGOS_STEP_DONE; }
static int parked(void *frame) { (void)frame; return LOGOS_STEP_YIELD; }
int main(int argc, char **argv) {
    int mode = atoi(argv[1]);
    logos_frame_hdr *h = logos_new_frame(mode == 2 ? parked : done,
        sizeof(logos_frame_hdr) + sizeof(void *), 0, 0);
    void *payload = logos_alloc(2048);
    *(void **)((char *)h + sizeof(logos_frame_hdr)) = payload;
    logos_task *t = logos_new_task(h);
    t->state = LOGOS_READY;
    size_t expected = lc_gc_takes(sizeof(logos_task));
    if (mode == 0) {
        logos_step_task(t);
        if (t->state != LOGOS_DONE) return 8;
    } else if (mode == 1) {
        logos_task *stopper = logos_new_task(logos_new_frame(done, sizeof(logos_frame_hdr), 0, 0));
        logos_op_abort(stopper, t);
        logos_sched_loop();
        if (t->state != LOGOS_DONE) return 8;
    } else if (mode == 2) {
        logos_step_task(t);
        if (t->state != LOGOS_READY || t->frame != h ||
            *(void **)((char *)t->frame + sizeof(logos_frame_hdr)) != payload) return 8;
        expected += lc_gc_takes(sizeof(logos_frame_hdr) + sizeof(void *)) + lc_gc_takes(2048);
    } else return 9;
    void *root = t;
    size_t actual = trace_one(&root, sizeof root);
    printf("mode=%d expected=%zu actual=%zu\n", mode, expected, actual);
    return actual == expected ? 0 : 7;
}
