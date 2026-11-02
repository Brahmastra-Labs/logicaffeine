
static int finishing_step(void *raw) {
    logos_frame_hdr *h = (logos_frame_hdr *)raw;
    volatile long long *cell = (volatile long long *)((char *)h + sizeof(*h));
    logos_op_exit(logos_current);
    /* The caller still owns this executing frame until this function returns. */
    *cell = 73;
    if (*cell != 73) abort();
    return LOGOS_STEP_DONE;
}
int main(void) {
    logos_task *t = logos_new_task(logos_new_frame(finishing_step,
        sizeof(logos_frame_hdr) + sizeof(long long), 0, 0));
    t->state = LOGOS_READY;
    logos_step_task(t);
    return t->state == LOGOS_DONE ? 0 : 8;
}
