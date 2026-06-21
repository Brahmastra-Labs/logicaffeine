//! Append-only-worklist → pre-sized-buffer + register-tail transform.
//!
//! A BFS/DFS frontier `q` whose every enqueue sits behind a monotone visited-set
//! guard (`if dist[x]==-1 { dist[x]=…; push x to q }`) is bounded by
//! `length(dist)` pushes, so codegen pre-sizes the buffer and writes at a
//! register tail (`q[tail]=x; tail+=1`) — C's frontier code — instead of
//! `Vec::push` (capacity check + length-to-memory + a `grow_one` call that
//! blocks unrolling). Anything outside that exact shape stays an ordinary `Vec`.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::compile_to_rust;

const BFS: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable dist be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 0 - 1 to dist.
    Set i to i + 1.
Let mutable queue be a new Seq of Int.
Push 0 to queue.
Set item 1 of dist to 0.
Let mutable front be 1.
While front is at most length of queue:
    Let v be item front of queue.
    Let nb be (v + 1) % n.
    If item (nb + 1) of dist equals 0 - 1:
        Set item (nb + 1) of dist to item (v + 1) of dist + 1.
        Push nb to queue.
    Set front to front + 1.
Show length of queue.
"#;

/// The BFS worklist converts: a pre-sized buffer + a register tail, with the
/// enqueues lowered to indexed writes and `length of queue` reading the tail.
#[test]
fn bfs_worklist_becomes_presized_buffer_with_register_tail() {
    let rust = compile_to_rust(BFS).unwrap();
    assert!(
        rust.contains("__queue_tail") && rust.contains("queue[__queue_tail]"),
        "the BFS frontier must lower to a register tail + indexed write. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("vec![0i64;"),
        "the worklist buffer must be pre-sized (`vec![0i64; cap]`). Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("queue.push"),
        "every `queue` enqueue must be an indexed write, not `Vec::push`. Got:\n{}",
        rust
    );
}

/// A worklist that is also POPPED is not append-only — the tail-only model is
/// unsound (the logical length shrinks), so it must stay an ordinary `Vec`
/// with `push`/`pop`.
#[test]
fn popped_worklist_is_not_converted() {
    let src = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable dist be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push 0 - 1 to dist.
    Set i to i + 1.
Let mutable stack be a new Seq of Int.
Push 0 to stack.
Set item 1 of dist to 0.
While length of stack is greater than 0:
    Pop from stack into v.
    Let nb be (v + 1) % n.
    If item (nb + 1) of dist equals 0 - 1:
        Set item (nb + 1) of dist to item (v + 1) of dist + 1.
        Push nb to stack.
Show length of stack.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("stack.push") || rust.contains("stack.pop"),
        "a popped worklist must keep `Vec` push/pop semantics, not convert. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("__stack_tail"),
        "a popped worklist must NOT get the register-tail conversion. Got:\n{}",
        rust
    );
}

/// An UNCONDITIONAL push inside a loop is unbounded (it is not gated by a
/// monotone visited set), so the capacity cannot be bounded and the worklist
/// must NOT convert — guarding against a buffer overflow.
#[test]
fn unbounded_in_loop_push_is_not_converted() {
    let src = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable queue be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to queue.
    Set i to i + 1.
Let mutable total be 0.
Let mutable front be 1.
While front is at most length of queue:
    Let v be item front of queue.
    Set total to total + v.
    Set front to front + 1.
Show total.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("__queue_tail"),
        "a worklist with an unconditional in-loop push has no bound and must \
         NOT get the register-tail conversion. Got:\n{}",
        rust
    );
}
