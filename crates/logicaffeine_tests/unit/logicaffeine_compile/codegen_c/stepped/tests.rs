use super::*;
use crate::compile::compile_to_c_stepped;
use crate::ui_bridge::with_parsed_program;

const RECV_BEFORE_SEND: &str = "## To consume (ch: Int):\n    Receive x from ch.\n    Show x.\n\n\
        ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 5 into ch.\n";
const TWO_BLOCKING_RECVS: &str =
    "## To consume (ch: Int):\n    Receive a from ch.\n    Show a.\n    \
        Receive b from ch.\n    Show b.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
        Launch a task to consume with ch.\n    Send 1 into ch.\n    Send 2 into ch.\n";
const SLEEP_RESUME: &str = "## Main\n    Show 1.\n    Sleep 5.\n    Show 2.\n";
const FIRE_AND_FORGET: &str = "## To worker:\n    Show \"work\".\n\n## Main\n    Launch a task to worker.\n    Show \"main\".\n";
const PRODUCER_CONSUMER: &str = "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    \
        Send 3 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    \
        Receive a from jobs.\n    Receive b from jobs.\n    Receive c from jobs.\n    Show a + b + c.\n";

/// The machinery of a coroutine tier, none of which a GPU thread can run.
const COROUTINE_MACHINERY: &[&str] = &[
    "ucontext",
    "swapcontext",
    "getcontext",
    "makecontext",
    "LOGOS_STACK",
];

/// The program's C on the stepped tier.
fn stepped_c(src: &str) -> String {
    compile_to_c_stepped(src)
        .unwrap_or_else(|e| panic!("the program lowers to the stepped tier: {e:?}\n{src}"))
}

fn assert_no_coroutine_machinery(c: &str) {
    for banned in COROUTINE_MACHINERY {
        assert!(
            !c.contains(banned),
            "the stepped tier's C must not carry `{banned}`"
        );
    }
}

/// The names of the program's functions that wait, and of those launched as tasks, each sorted.
fn waits_and_launched(src: &str) -> (Vec<String>, Vec<String>) {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, ..) = parsed.expect("test program parses");
        let stepped = stepped_functions(stmts);
        let names = |set: &HashSet<Symbol>| {
            let mut v: Vec<String> = set
                .iter()
                .map(|s| interner.resolve(*s).to_string())
                .collect();
            v.sort();
            v
        };
        (names(&stepped.waits), names(&stepped.launched))
    })
}

/// One step function's definition, from its opening line to its closing brace at the margin.
fn step_body<'a>(c: &'a str, cname: &str) -> &'a str {
    let marker = format!("static int logos_step_{cname}(void *fp) {{");
    let start = c
        .find(&marker)
        .unwrap_or_else(|| panic!("no definition for step `{cname}`"));
    let rest = &c[start..];
    let end = rest.find("\n}\n").map(|e| e + 2).unwrap_or(rest.len());
    &rest[..end]
}

/// One frame's fields, between its `typedef struct … {` and its closing name.
fn frame<'a>(c: &'a str, cname: &str) -> &'a str {
    let open = format!("typedef struct logos_frame_{cname} {{");
    let body = c
        .split(&open)
        .nth(1)
        .unwrap_or_else(|| panic!("no frame for `{cname}`"));
    &body[..body
        .find(&format!("}} logos_frame_{cname};"))
        .expect("the frame closes")]
}

/// The byte index of the `}` matching the first `{` at or after `from` — a brace-depth walk, so a nested
/// block does not close the outer one.
fn matching_close_brace(s: &str, from: usize) -> usize {
    let bytes = s.as_bytes();
    let open = from + s[from..].find('{').expect("an open brace at/after `from`");
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    panic!("no matching close brace for the block opened at {open}");
}

#[test]
fn every_concurrent_program_is_stepped_and_the_waiters_are_exactly_the_functions_that_wait() {
    let expect: &[(&str, &[&str], &[&str])] = &[
        (RECV_BEFORE_SEND, &["consume"], &["consume"]),
        (TWO_BLOCKING_RECVS, &["consume"], &["consume"]),
        (SLEEP_RESUME, &[], &[]),
        (FIRE_AND_FORGET, &[], &["worker"]),
        (PRODUCER_CONSUMER, &["produce"], &["produce"]),
    ];
    for (src, waits, launched) in expect {
        let (w, l) = waits_and_launched(src);
        assert_eq!(w, *waits, "the functions that wait:\n{src}");
        assert_eq!(l, *launched, "the functions launched:\n{src}");
        let c = stepped_c(src);
        assert!(
            c.contains("static int logos_step___main(void *fp) {"),
            "Main of a concurrent program is a step function:\n{src}"
        );
        for f in launched.iter() {
            assert!(
                c.contains(&format!("static int logos_step_{f}(void *fp) {{")),
                "a launched `{f}` is a step function:\n{src}"
            );
        }
        assert_no_coroutine_machinery(&c);
    }
}

#[test]
fn recv_before_send_emits_the_expected_state_machine() {
    let c = stepped_c(RECV_BEFORE_SEND);
    assert!(c.contains("typedef struct logos_frame___main {"));
    assert!(c.contains("typedef struct logos_frame_consume {"));
    let consume = step_body(&c, "consume");
    let recv = consume
        .find("logos_op_recv(logos_current, ")
        .expect("the receive is the core's, which parks on an empty pipe");
    let saved = consume
        .find("fr->__hdr.pc = 1; return LOGOS_STEP_YIELD;")
        .expect("the receive saves its resume point and yields");
    let resume = consume
        .find("case 1: ;")
        .expect("the receive's resume point");
    let read = consume
        .find("fr->x = logos_resumed();")
        .expect("the resume reads what the receive delivered");
    assert!(
        recv < saved && saved < resume && resume < read,
        "receive, yield, resume, read — in that order:\n{consume}"
    );
    assert!(c.contains(
        "logos_sched_bootstrap(logos_step___main, sizeof(logos_frame___main), LOGOS_RUN_SEED);"
    ));
    assert_no_coroutine_machinery(&c);
}

#[test]
fn sleep_lowers_to_a_timer_park() {
    let c = stepped_c(SLEEP_RESUME);
    let main = step_body(&c, "__main");
    assert!(
        main.contains("logos_op_sleep(logos_current, "),
        "Sleep arms a timer on the logical clock:\n{main}"
    );
    assert!(
        main.contains("if (fr->__ticks0 > 0) {"),
        "a sleep of no time is no scheduling point:\n{main}"
    );
    assert!(
        c.contains("logos_advance_timers"),
        "the scheduler advances the clock"
    );
}

const LOOP_RECV: &str =
    "## To consume (ch: Int):\n    Let i be 0.\n    While i is less than 3:\n        \
        Receive x from ch.\n        Show x.\n        Set i to i + 1.\n\n## Main\n    \
        Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    \
        Send 10 into ch.\n    Send 20 into ch.\n    Send 30 into ch.\n";
const LOOP_SUM: &str = "## To sum_until (ch: Int):\n    Let total be 0.\n    Let n be 0.\n    \
        While n is less than 3:\n        Receive v from ch.\n        Set total to total + v.\n        \
        Set n to n + 1.\n    Show total.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
        Launch a task to sum_until with ch.\n    Send 5 into ch.\n    Send 7 into ch.\n    Send 9 into ch.\n";
const COND_RECV: &str =
    "## To consume (ch: Int):\n    Receive x from ch.\n    If x is greater than 5:\n        \
        Receive y from ch.\n        Show y.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
        Launch a task to consume with ch.\n    Send 10 into ch.\n    Send 20 into ch.\n";

#[test]
fn a_blocking_recv_in_a_while_places_the_resume_case_inside_the_loop() {
    let c = stepped_c(LOOP_RECV);
    let step = step_body(&c, "consume");
    let while_at = step.find("while (").expect("the While is a C while");
    let case1_at = step.find("case 1:").expect("the receive's resume point");
    let close_at = matching_close_brace(step, while_at);
    assert!(
        while_at < case1_at && case1_at < close_at,
        "the receive's `case 1:` sits inside the while body:\n{step}"
    );
}

#[test]
fn a_blocking_loop_keeps_its_counter_and_recv_var_in_the_frame() {
    let c = stepped_c(LOOP_RECV);
    let fields = frame(&c, "consume");
    assert!(
        fields.contains("int64_t i;"),
        "the loop counter is a frame field:\n{fields}"
    );
    assert!(
        fields.contains("logos_opt x;"),
        "the received value is a frame field:\n{fields}"
    );
    let step = step_body(&c, "consume");
    assert!(
        step.contains("fr->i = 0LL;"),
        "the counter is assigned in the frame:\n{step}"
    );
    for local in ["int64_t i", "logos_opt x"] {
        assert!(
            !step.contains(local),
            "no declaration may stand between the switch and a case (`{local}`):\n{step}"
        );
    }
}

#[test]
fn a_blocking_recv_in_an_if_places_the_resume_case_inside_the_branch() {
    let c = stepped_c(COND_RECV);
    let step = step_body(&c, "consume");
    let if_line = "if ((({ if (!(fr->x).present) { LOGOS_STEP_FAIL(\"Cannot compare Nothing and Int\"); } (((int64_t)(fr->x).value) > 5LL); }))) {";
    let if_at = step.find(if_line).expect("the If is a C if");
    let case2_at = step
        .find("case 2:")
        .expect("the branch receive's resume point");
    let close_at = matching_close_brace(step, if_at + if_line.len() - 1);
    assert!(
        if_at < case2_at && case2_at < close_at,
        "the branch receive's `case 2:` sits inside the if:\n{step}"
    );
}

#[test]
fn an_accumulating_loop_lowers_its_set_mutations() {
    let c = stepped_c(LOOP_SUM);
    let step = step_body(&c, "sum_until");
    assert!(
        step.contains("fr->total = ({ if (!(fr->v).present) { LOGOS_STEP_FAIL(\"Cannot add Int and Nothing\"); } logos_lint_add(fr->total, lint_of((int64_t)(((int64_t)(fr->v).value)))); });"),
        "the accumulator is a frame-field assignment:\n{step}"
    );
    let fields = frame(&c, "sum_until");
    assert!(
        fields.contains("LInt total;") && fields.contains("int64_t n;"),
        "accumulators are frame fields:\n{fields}"
    );
}

const NESTED_CALL: &str = "## To get (ch: Int) -> Int:\n    Receive x from ch.\n    Return x.\n\n\
        ## To consume (ch: Int):\n    Let y be get(ch).\n    Show y.\n\n## Main\n    \
        Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 42 into ch.\n";
const NESTED_TWICE: &str = "## To get (ch: Int) -> Int:\n    Receive x from ch.\n    Return x.\n\n\
        ## To consume (ch: Int):\n    Let a be get(ch).\n    Let b be get(ch).\n    Show a + b.\n\n\
        ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    \
        Send 3 into ch.\n    Send 4 into ch.\n";

#[test]
fn a_blocking_helper_call_lowers_to_a_frame_stack_push_and_pop() {
    let c = stepped_c(NESTED_CALL);
    assert!(
        c.contains("typedef struct logos_frame_consume {")
            && c.contains("typedef struct logos_frame_get {")
    );
    let consume = step_body(&c, "consume");
    assert!(
        consume.contains("logos_call_push(logos_step_get, sizeof(logos_frame_get), (long long *)&fr->__returned0, 1, __args);"),
        "the call pushes the callee's frame, its result bound for a cell of the caller's:\n{consume}"
    );
    assert!(
        consume.contains("return LOGOS_STEP_CALL;"),
        "the callee runs at once, with no trip through the ready queue:\n{consume}"
    );
    assert!(
        consume.contains("fr->y = (int64_t)fr->__returned0;"),
        "the caller resumes reading the callee's result:\n{consume}"
    );
    let get = step_body(&c, "get");
    assert!(
        get.contains("logos_call_return(__result);"),
        "the helper's Return pops to its caller with the value:\n{get}"
    );
    assert!(
        !c.contains("int64_t get("),
        "a function that waits runs only as a step function"
    );
    assert_no_coroutine_machinery(&c);
}

#[test]
fn a_blocking_helper_call_makes_its_caller_wait() {
    for src in [NESTED_CALL, NESTED_TWICE] {
        let (waits, _) = waits_and_launched(src);
        assert_eq!(
            waits,
            ["consume", "get"],
            "the caller of a function that waits waits too:\n{src}"
        );
    }
}

#[test]
fn a_blocking_helper_called_twice_lowers_two_push_pop_cycles() {
    let c = stepped_c(NESTED_TWICE);
    let fields = frame(&c, "consume");
    assert!(
        fields.contains("int64_t a;") && fields.contains("int64_t b;"),
        "both results are frame fields:\n{fields}"
    );
    let step = step_body(&c, "consume");
    assert_eq!(
        step.matches("logos_call_push(logos_step_get, ").count(),
        2,
        "each call site pushes its own callee frame:\n{step}"
    );
}

#[test]
fn a_pure_helper_call_stays_an_ordinary_c_call() {
    let src = "## To double (n: Int) -> Int:\n    Return n + n.\n\n\
            ## To consume (ch: Int):\n    Receive x from ch.\n    Let y be double(x).\n    Show y.\n\n## Main\n    \
            Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 5 into ch.\n";
    let (waits, _) = waits_and_launched(src);
    assert_eq!(
        waits,
        ["consume"],
        "a helper that never waits is no step function"
    );
    let c = stepped_c(src);
    assert!(
        c.contains("LInt logos_double(int64_t n) {"),
        "the helper is an ordinary C function:\n{c}"
    );
    let step = step_body(&c, "consume");
    assert!(
        step.contains("fr->y = logos_double(((int64_t)logos_opt_int(fr->x)));"),
        "the step calls it in place:\n{step}"
    );
    assert!(
        !step.contains("logos_call_push"),
        "an ordinary call pushes no frame:\n{step}"
    );
    assert!(
        !c.contains("logos_step_double"),
        "a helper that never waits has no step function"
    );
}

const DIRECT_RECURSION: &str =
    "## To drain (ch: Int):\n    Receive x from ch.\n    If x is greater than 0:\n        \
        Show x.\n        drain(ch).\n\n## Main\n    Let ch be a Pipe of Int.\n    \
        Launch a task to drain with ch.\n    Send 2 into ch.\n    Send 1 into ch.\n    Send 0 into ch.\n";

#[test]
fn direct_self_recursion_lowers_to_the_same_frame_stack_as_any_nested_call() {
    let c = stepped_c(DIRECT_RECURSION);
    assert_eq!(
        c.matches("static int logos_step_drain(void *fp) {").count(),
        1,
        "one step definition serves every depth"
    );
    let step = step_body(&c, "drain");
    assert!(
        step.contains("logos_call_push(logos_step_drain, sizeof(logos_frame_drain), (long long *)0, 1, __args);"),
        "each recursive call pushes a fresh frame:\n{step}"
    );
    assert!(
        step.contains("return LOGOS_STEP_CALL;"),
        "the callee runs in place:\n{step}"
    );
    assert!(
        step.contains("if (fr->__hdr.caller == 0) return LOGOS_STEP_DONE;"),
        "a root frame ends its task:\n{step}"
    );
    assert!(
        step.contains("logos_call_return(0LL);"),
        "a helper falling off its end pops to its caller:\n{step}"
    );
    assert_no_coroutine_machinery(&c);
}

#[test]
fn a_mutually_recursive_blocking_cycle_lowers_to_the_frame_stack() {
    let src = "## To ping (ch: Int):\n    Receive x from ch.\n    If x is greater than 0:\n        \
            Show x.\n        pong(ch).\n\n## To pong (ch: Int):\n    Receive y from ch.\n    If y is greater than 0:\n        \
            Show y.\n        ping(ch).\n\n## Main\n    Let ch be a Pipe of Int.\n    \
            Launch a task to ping with ch.\n    Send 2 into ch.\n    Send 1 into ch.\n    Send 0 into ch.\n";
    let c = stepped_c(src);
    assert!(
        step_body(&c, "ping")
            .contains("logos_call_push(logos_step_pong, sizeof(logos_frame_pong), "),
        "ping pushes pong"
    );
    assert!(
        step_body(&c, "pong")
            .contains("logos_call_push(logos_step_ping, sizeof(logos_frame_ping), "),
        "pong pushes ping"
    );
}

const SELECT_RECV_READY: &str = "## To produce (ch: Int):\n    Send 9 into ch.\n\n## Main\n    \
        Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Await the first of:\n        \
        Receive v from ch:\n            Show v.\n        After 100 seconds:\n            Show 0.\n";
const SELECT_TIMEOUT_WINS: &str =
    "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        \
        Receive v from ch:\n            Show v.\n        After 5 seconds:\n            Show 42.\n";

#[test]
fn a_top_level_select_makes_main_a_step() {
    for src in [SELECT_RECV_READY, SELECT_TIMEOUT_WINS] {
        let c = stepped_c(src);
        assert!(
            step_body(&c, "__main").contains("logos_op_select(logos_current, "),
            "Main selects as a step:\n{src}"
        );
    }
}

#[test]
fn a_select_lowers_to_one_core_select_over_every_arm() {
    let c = stepped_c(SELECT_RECV_READY);
    let main = step_body(&c, "__main");
    assert!(
        main.contains("const int __k[] = {LOGOS_ARM_RECV, LOGOS_ARM_AFTER};"),
        "the arms go to the core in declaration order:\n{main}"
    );
    assert!(
        main.contains("logos_op_select(logos_current, 2, __k, __c, __t);"),
        "one core operation over both arms:\n{main}"
    );
    assert!(
        main.contains("logos_current->selected_arm"),
        "the resume dispatches on the selected arm:\n{main}"
    );
    assert!(
        frame(&c, "__main").contains("logos_opt v;"),
        "the receive arm's value is a frame field"
    );
    assert_no_coroutine_machinery(&c);
}

#[test]
fn a_select_dispatches_each_arm_body_under_its_index() {
    let c = stepped_c(SELECT_RECV_READY);
    let main = step_body(&c, "__main");
    let arm0 = main
        .find("if (logos_current->selected_arm == 0) {")
        .expect("the receive arm is guarded by its index");
    let arm1 = main
        .find("} else if (logos_current->selected_arm == 1) {")
        .expect("the timeout arm is guarded by its index");
    let bind = main
        .find("fr->v = logos_resumed();")
        .expect("the receive arm binds what the core delivered");
    assert!(
        arm0 < bind && bind < arm1,
        "the receive arm binds its value inside its own block:\n{main}"
    );
    assert!(
        main[arm1..].contains("show_i64(0LL);"),
        "the timeout arm runs its body:\n{main}"
    );
}

#[test]
fn a_timeout_arm_ticks_match_the_vm_virtual_clock() {
    let c = stepped_c(SELECT_TIMEOUT_WINS);
    assert!(c.contains("const long long __t[] = {0LL, (long long)(5000000000LL)};"),
        "`After 5 seconds` arms 5000000000 ticks (Duration nanos), as the VM's clock counts them:\n{c}");
}

#[test]
fn two_ready_recv_arms_lower_to_one_seeded_select() {
    let src = "## To p (a: Int):\n    Send 1 into a.\n\n## To q (b: Int):\n    Send 2 into b.\n\n## Main\n    \
            Let a be a Pipe of Int.\n    Let b be a Pipe of Int.\n    Launch a task to p with a.\n    \
            Launch a task to q with b.\n    Await the first of:\n        Receive x from a:\n            Show x.\n        \
            Receive y from b:\n            Show y.\n";
    let c = stepped_c(src);
    assert!(
        c.contains("const int __k[] = {LOGOS_ARM_RECV, LOGOS_ARM_RECV};"),
        "both arms receive"
    );
    assert!(
        c.contains("logos_op_select(logos_current, 2, __k, __c, __t);"),
        "one core select, resolved by its seeded chooser"
    );
}

#[test]
fn a_select_arm_can_show_text() {
    let src = "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        \
            Receive v from ch:\n            Show v.\n        After 5 seconds:\n            Show \"gone\".\n";
    let main = stepped_c(src);
    assert!(
        step_body(&main, "__main").contains("show_str(\"gone\");"),
        "the arm shows its text"
    );
}

#[test]
fn a_task_body_can_show_text_after_a_blocking_receive() {
    let src = "## To consume (ch: Int):\n    Receive x from ch.\n    Show \"hi\".\n\n## Main\n    \
            Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 5 into ch.\n";
    let c = stepped_c(src);
    let step = step_body(&c, "consume");
    let resume = step.find("case 1:").expect("the receive's resume point");
    assert!(
        step[resume..].contains("show_str(\"hi\");"),
        "the text shows after the receive resumes:\n{step}"
    );
}

#[test]
fn a_real_channel_carries_the_doubles_bits() {
    let src = "## To consume (ch: Real):\n    Receive x from ch.\n    Show x.\n\n## Main\n    \
            Let ch be a Pipe of Real.\n    Launch a task to consume with ch.\n    Send 1.5 into ch.\n";
    let c = stepped_c(src);
    assert!(c.contains("logos_op_send(logos_current, (logos_chan *)(intptr_t)(fr->ch), logos_real_bits((double)(1.5)));"),
        "the sent Real rides as its bits, never truncated:\n{c}");
    assert!(
        c.contains("show_f64(logos_bits_real((fr->x).value))"),
        "the received Real shows as a Real"
    );
    assert_no_coroutine_machinery(&c);
}

#[test]
fn a_text_channel_carries_the_char_pointer_across_the_yield() {
    let src = "## To consume (ch: Text):\n    Receive x from ch.\n    Show x.\n\n## Main\n    \
            Let ch be a Pipe of Text.\n    Launch a task to consume with ch.\n    Send \"hello\" into ch.\n";
    let c = stepped_c(src);
    assert!(c.contains("logos_op_send(logos_current, (logos_chan *)(intptr_t)(fr->ch), (long long)(\"hello\"));"),
        "the sent text rides as its pointer:\n{c}");
    assert!(
        c.contains("show_str((char *)(fr->x).value)"),
        "the received text shows as text"
    );
}

#[test]
fn a_bool_channel_shows_with_the_bool_renderer() {
    let src = "## To consume (ch: Bool):\n    Receive x from ch.\n    Show x.\n\n## Main\n    \
            Let ch be a Pipe of Bool.\n    Launch a task to consume with ch.\n    Send true into ch.\n";
    let c = stepped_c(src);
    assert!(
        c.contains("show_bool((bool)(fr->x).value)"),
        "the received Bool shows as true/false, never 1/0:\n{c}"
    );
}

#[test]
fn a_record_channel_payload_is_refused_not_miscompiled() {
    let src = "## A Point has:\n    an x, which is Int.\n\n## To consume (ch: Point):\n    Receive p from ch.\n    \
            Show p's x.\n\n## Main\n    Let ch be a Pipe of Point.\n    Launch a task to consume with ch.\n    \
            Let q be a new Point with x 1.\n    Send q into ch.\n";
    let c = stepped_c(src);
    assert!(c.contains("#error \"the C backend does not lower a pipe of Point, which a pipe's scalar cell cannot carry\""),
        "a record crossing a pipe is refused in the C tier's words, never carried wrong:\n{c}");
}

#[test]
fn a_received_real_does_real_arithmetic() {
    let src = "## To consume (ch: Real):\n    Receive x from ch.\n    Show x + 1.\n\n## Main\n    \
            Let ch be a Pipe of Real.\n    Launch a task to consume with ch.\n    Send 1.5 into ch.\n";
    let c = stepped_c(src);
    assert!(c.contains("show_f64(({ if (!(fr->x).present) { LOGOS_STEP_FAIL(\"Cannot add Nothing and Int\"); } (logos_bits_real((fr->x).value) + 1LL); }));"),
        "the received Real is read back from its bits before the arithmetic:\n{c}");
}

#[test]
fn a_binding_named_like_c_is_a_c_safe_frame_field() {
    let src = "## To consume (ch: Int):\n    Receive register from ch.\n    Show register.\n\n## Main\n    \
            Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 5 into ch.\n";
    let c = stepped_c(src);
    assert!(
        frame(&c, "consume").contains("logos_opt logos_register;"),
        "a C keyword binding is a renamed field"
    );
    assert!(step_body(&c, "consume").contains("fr->logos_register = logos_resumed();"));
}

#[test]
fn a_step_does_its_arithmetic_through_the_c_tiers_own_lowering() {
    let src = "## To consume (ch: Int):\n    Receive x from ch.\n    Show x + 10.\n    Show x - 1.\n    Show x * 2.\n    \
            Show x / 2.\n    Show x % 3.\n    If x is at least 5:\n        Show \"at least\".\n\n## Main\n    \
            Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 5 into ch.\n";
    let c = stepped_c(src);
    let step = step_body(&c, "consume");
    let x = "((int64_t)(fr->x).value)";
    let fails = |words: &str| format!("if (!(fr->x).present) {{ LOGOS_STEP_FAIL(\"{words}\"); }}");
    for line in [
        format!(
            "show_i64(({{ {} ({x} + 10LL); }}));",
            fails("Cannot add Nothing and Int")
        ),
        format!(
            "show_i64(({{ {} ({x} - 1LL); }}));",
            fails("Cannot subtract Int from Nothing")
        ),
        format!(
            "show_i64(({{ {} ({x} * 2LL); }}));",
            fails("Cannot multiply Nothing and Int")
        ),
        format!(
            "show_i64(({{ {} logos_idiv((int64_t)({x}), (int64_t)(2LL)); }}));",
            fails("Cannot divide Nothing by Int")
        ),
        format!(
            "show_i64(({{ {} logos_imod((int64_t)({x}), (int64_t)(3LL)); }}));",
            fails("Cannot compute modulo of Nothing and Int")
        ),
        format!(
            "if ((({{ {} ({x} >= 5LL); }}))) {{",
            fails("Cannot compare Nothing and Int")
        ),
    ] {
        assert!(step.contains(&line), "`{line}` in the step:\n{step}");
    }
}
