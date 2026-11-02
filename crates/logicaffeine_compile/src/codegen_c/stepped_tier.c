/* The stepped tier's glue for a program the C tier lowers into step functions (`stepped.rs`): what a
 * scheduling point resumed with, and a failure inside a task. A failing statement of a step (`Require`,
 * `Fail`, a `Check`, an `Inspect` with no arm) fails its task through the core and the step returns, on every
 * target. A failure deeper down, inside a runtime helper, escapes on the host to the running step's entry,
 * which has already failed the task through the core, and the step returns — so the run keeps its first
 * failure and every other task still runs, as on the interpreter. A GPU thread has no such escape
 * (`LOGOS_STEP_NO_ESCAPE`): a helper's failure there traps the run. */

/* What the running task's last operation resumed with, as a value-or-nothing. */
LOGOS_FN logos_opt logos_resumed(void) {
    logos_opt o = { !logos_current->resume_nothing, logos_current->resume_nothing ? 0 : logos_current->resume };
    return o;
}

/* A failing statement of a step: its task fails with `message`, and the step returns. */
#define LOGOS_STEP_FAIL(message) do { logos_op_fail(logos_current, (message)); return LOGOS_STEP_YIELD; } while (0)

#ifndef LOGOS_STEP_NO_ESCAPE
static jmp_buf logos_step_escape;

/* The runtime's failure hook while a stepped program runs: fail the running task and leave its step. */
static void logos_step_failed(const char *message) {
    if (!logos_current) return;
    logos_op_fail(logos_current, message);
    longjmp(logos_step_escape, 1);
}

/* The first statement of every step function: a failure below lands here, and the step returns. */
#define LOGOS_STEP_ENTRY if (setjmp(logos_step_escape)) return LOGOS_STEP_YIELD;
#define LOGOS_STEP_HOOK() (logos_fail_hook = logos_step_failed)
#else
#define LOGOS_STEP_ENTRY
#define LOGOS_STEP_HOOK() ((void)0)
#endif
