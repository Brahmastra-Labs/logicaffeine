//! `## Store S shared on "topic"` — a store persisted to its TOPIC, which every replica of the
//! store on that topic shares, in this page and in every other page on the relay.
//!
//! A shared store's record is the module binding a local store has. Each field lives in the topic
//! area (`"shared"`) under `"<topic>.<field>"` as a FRAME, and every write to that area is a JOIN:
//! the frame held and the frame written merge into their least upper bound (`__sh_join`), so the
//! area only ever moves up, whatever order frames reach it in, however often they repeat. What the
//! module adds to persistence:
//!
//! - ★ A REPLICA JOINS ON FIRST ACCESS. `__join_<S>` runs before the first statement that reads
//!   one of the store's fields: it adopts every field the topic holds, and publishes every plain
//!   field it does not. A replica nobody reads never joins, and so never publishes its initial
//!   values over the state a reader adopts.
//! - ★ A PLAIN FIELD IS A LAST-WRITER-WINS REGISTER. Its frame is `<lamport>:<value>`. Publishing
//!   ticks this page's Lamport clock, and only a field whose value moved is published, so an
//!   untouched field never overwrites a concurrent write to it. Frames order by Lamport time, then
//!   by the value's bytes — a total order, so the held frame is the greatest a page has seen, and
//!   no page needs an identity of its own to break a tie.
//! - ★ A CONVERGENT FIELD IS A CRDT, AND ITS AUTHOR SEES A PLAIN VALUE. A `Tally`/`ConvergentCount`
//!   field is an `Int`, a `SharedSet of T` a `Set of T`, a `SharedSequence of T` a `Seq of T`, and
//!   every sentence that works on those works on it. The CRDT is the frame
//!   (`~<kind><key><value>…`, length-prefixed atoms): a map from key to a value that only grows,
//!   which joins by union, keeping the greater value where both hold a key. After an action the
//!   operation is read off how the plain value moved from the one last adopted — a counter's
//!   difference becomes this page's increments or decrements, a set's added and removed elements
//!   become tagged adds and tombstones of the adds it had seen (add-wins), a sequence's edit
//!   becomes insert-after nodes and tombstones (RGA) — and the value is rebuilt from the merged
//!   frame, in the one order every page derives. Tags and node ids are `<lamport>.<replica>`: the
//!   replica is drawn once per page load, and only by a page with a convergent field.
//! - ★ ONE STORE PER TOPIC IN A PAGE. After a publish, every other replica of the topic that has
//!   joined is refreshed from it, so a write through one replica is what every replica reads.
//! - ★ THE MERGE LIVES HERE, ONCE. A relay host only moves frames: it hands each frame that
//!   arrives to `__lgx_shared(topic, key, frame)`, which joins it, refreshes the joined replicas
//!   and repaints the page.
//!
//! The frame algebra is LOGOS ([`SHARED_HELPERS`]), parsed with the page and lowered with it on
//! every tier — the lowering writes these functions, so no standard-library demand stands between
//! a page and them.

use super::*;
use crate::analysis::TypeRegistry;
use crate::ast::component::{ComponentDef, ComponentKind, StateField, StoreMode};
use crate::ast::stmt::{
    BinaryOpKind, ClosureBody, Expr, Pattern, SelectBranch, Stmt, StringPart, TypeExpr, UiStmt,
};
use logicaffeine_base::{Interner, Symbol};

/// The frame algebra every shared store runs on, in LOGOS.
///
/// A plain field's frame is `<lamport>:<value>`; a convergent field's is `~` and then
/// length-prefixed atoms (`<chars>:<text>`) — its kind (`C` counter, `S` set, `Q` sequence), then
/// its entries as key/value pairs sorted by key. Counter keys are `i<replica>`/`d<replica>` over a
/// replica's running increments/decrements; set keys `a<tag>` (the element added under that tag)
/// and `r<tag>` (that add removed); sequence keys `n<id>` (`<parent id>:<element>`, the root
/// parent `0`) and `d<id>` (that node removed).
pub(crate) const SHARED_HELPERS: &str = r#"## To __sh_atom (__sh_t: Text) -> Text:
    Let mutable __sh_n be 0.
    Repeat for __sh_c in __sh_t:
        Set __sh_n to __sh_n + 1.
    Return "{__sh_n}:{__sh_t}".

## To __sh_first (__sh_t: Text) -> Text:
    Repeat for __sh_c in __sh_t:
        Return __sh_c.
    Return "".

## To __sh_tail (__sh_t: Text) -> Text:
    Let mutable __sh_out be "".
    Let mutable __sh_skip be true.
    Repeat for __sh_c in __sh_t:
        If __sh_skip:
            Set __sh_skip to false.
        Otherwise:
            Set __sh_out to __sh_out + __sh_c.
    Return __sh_out.

## To __sh_before_colon (__sh_t: Text) -> Text:
    Let mutable __sh_out be "".
    Repeat for __sh_c in __sh_t:
        If __sh_c is equal to ":":
            Return __sh_out.
        Set __sh_out to __sh_out + __sh_c.
    Return __sh_out.

## To __sh_value (__sh_t: Text) -> Text:
    Let mutable __sh_out be "".
    Let mutable __sh_past be false.
    Repeat for __sh_c in __sh_t:
        If __sh_past:
            Set __sh_out to __sh_out + __sh_c.
        Otherwise If __sh_c is equal to ":":
            Set __sh_past to true.
    Return __sh_out.

## To __sh_lamport (__sh_frame: Text) -> Int:
    Return parseInt(__sh_before_colon(__sh_frame)).

## To __sh_after (__sh_a: Text) and (__sh_b: Text) -> Bool:
    Let __sh_x be text_bytes(__sh_a).
    Let __sh_y be text_bytes(__sh_b).
    Let mutable __sh_n be length of __sh_x.
    If length of __sh_y is less than __sh_n:
        Set __sh_n to length of __sh_y.
    Let mutable __sh_i be 1.
    While __sh_i is at most __sh_n:
        If item __sh_i of __sh_x is greater than item __sh_i of __sh_y:
            Return true.
        If item __sh_i of __sh_x is less than item __sh_i of __sh_y:
            Return false.
        Set __sh_i to __sh_i + 1.
    Return length of __sh_x is greater than length of __sh_y.

## To __sh_bigger (__sh_a: Text) and (__sh_b: Text) -> Bool:
    Let __sh_la be length of text_bytes(__sh_a).
    Let __sh_lb be length of text_bytes(__sh_b).
    If __sh_la is not equal to __sh_lb:
        Return __sh_la is greater than __sh_lb.
    Return __sh_after(__sh_a, __sh_b).

## To __sh_newer (__sh_frame: Text) and (__sh_held: Text) -> Bool:
    If __sh_held is equal to "":
        Return true.
    Let __sh_a be __sh_lamport(__sh_frame).
    Let __sh_h be __sh_lamport(__sh_held).
    If __sh_a is greater than __sh_h:
        Return true.
    If __sh_a is less than __sh_h:
        Return false.
    Return __sh_after(__sh_value(__sh_frame), __sh_value(__sh_held)).

## To __sh_tick -> Int:
    Set __SH_CLOCK to __SH_CLOCK + 1.
    Return __SH_CLOCK.

## To __sh_witness (__sh_time: Int):
    If __sh_time is greater than __SH_CLOCK:
        Set __SH_CLOCK to __sh_time.

## To __sh_stamp (__sh_v: Text) -> Text:
    Return "{__sh_tick()}:{__sh_v}".

## To __sh_atoms (__sh_s: Text) -> Seq of Text:
    Let mutable __sh_out be a new Seq of Text.
    Let mutable __sh_len be 0.
    Let mutable __sh_left be 0 - 1.
    Let mutable __sh_cur be "".
    Let mutable __sh_skip be true.
    Repeat for __sh_c in __sh_s:
        If __sh_skip:
            Set __sh_skip to false.
        Otherwise:
            If __sh_left is less than 0:
                If __sh_c is equal to ":":
                    If __sh_len is equal to 0:
                        Push "" to __sh_out.
                    Otherwise:
                        Set __sh_left to __sh_len.
                        Set __sh_cur to "".
                    Set __sh_len to 0.
                Otherwise:
                    Set __sh_len to __sh_len * 10 + parseInt(__sh_c).
            Otherwise:
                Set __sh_cur to __sh_cur + __sh_c.
                Set __sh_left to __sh_left - 1.
                If __sh_left is equal to 0:
                    Push __sh_cur to __sh_out.
                    Set __sh_left to 0 - 1.
    Return __sh_out.

## To __sh_kind (__sh_frame: Text) -> Text:
    If __sh_first(__sh_frame) is not equal to "~":
        Return "".
    Let __sh_all be __sh_atoms(__sh_frame).
    If length of __sh_all is equal to 0:
        Return "".
    Return item 1 of __sh_all.

## To __sh_entries (__sh_frame: Text) -> Seq of Text:
    Let mutable __sh_out be a new Seq of Text.
    If __sh_first(__sh_frame) is not equal to "~":
        Return __sh_out.
    Let __sh_all be __sh_atoms(__sh_frame).
    Let mutable __sh_i be 2.
    While __sh_i is at most length of __sh_all:
        Push item __sh_i of __sh_all to __sh_out.
        Set __sh_i to __sh_i + 1.
    Return __sh_out.

## To __sh_entries_of (__sh_frame: Text) and (__sh_k: Text) -> Seq of Text:
    If __sh_kind(__sh_frame) is equal to __sh_k:
        Return __sh_entries(__sh_frame).
    Return a new Seq of Text.

## To __sh_state (__sh_k: Text) and (__sh_entries: Seq of Text) -> Text:
    Let mutable __sh_out be "~" + __sh_atom(__sh_k).
    Repeat for __sh_e in __sh_entries:
        Set __sh_out to __sh_out + __sh_atom(__sh_e).
    Return __sh_out.

## To __sh_union (__sh_a: Seq of Text) and (__sh_b: Seq of Text) -> Seq of Text:
    Let mutable __sh_out be a new Seq of Text.
    Let mutable __sh_i be 1.
    Let mutable __sh_j be 1.
    Let __sh_na be length of __sh_a.
    Let __sh_nb be length of __sh_b.
    While __sh_i is less than __sh_na or __sh_j is less than __sh_nb:
        If __sh_j is greater than __sh_nb:
            Push item __sh_i of __sh_a to __sh_out.
            Push item (__sh_i + 1) of __sh_a to __sh_out.
            Set __sh_i to __sh_i + 2.
        Otherwise If __sh_i is greater than __sh_na:
            Push item __sh_j of __sh_b to __sh_out.
            Push item (__sh_j + 1) of __sh_b to __sh_out.
            Set __sh_j to __sh_j + 2.
        Otherwise:
            Let __sh_ka be item __sh_i of __sh_a.
            Let __sh_kb be item __sh_j of __sh_b.
            If __sh_ka is equal to __sh_kb:
                Let __sh_va be item (__sh_i + 1) of __sh_a.
                Let __sh_vb be item (__sh_j + 1) of __sh_b.
                Push __sh_ka to __sh_out.
                If __sh_bigger(__sh_vb, __sh_va):
                    Push __sh_vb to __sh_out.
                Otherwise:
                    Push __sh_va to __sh_out.
                Set __sh_i to __sh_i + 2.
                Set __sh_j to __sh_j + 2.
            Otherwise If __sh_after(__sh_kb, __sh_ka):
                Push __sh_ka to __sh_out.
                Push item (__sh_i + 1) of __sh_a to __sh_out.
                Set __sh_i to __sh_i + 2.
            Otherwise:
                Push __sh_kb to __sh_out.
                Push item (__sh_j + 1) of __sh_b to __sh_out.
                Set __sh_j to __sh_j + 2.
    Return __sh_out.

## To __sh_put (__sh_entries: Seq of Text) and (__sh_key: Text) and (__sh_v: Text) -> Seq of Text:
    Let mutable __sh_one be a new Seq of Text.
    Push __sh_key to __sh_one.
    Push __sh_v to __sh_one.
    Return __sh_union(__sh_entries, __sh_one).

## To __sh_get (__sh_entries: Seq of Text) and (__sh_key: Text) -> Text:
    Let mutable __sh_i be 1.
    While __sh_i is less than length of __sh_entries:
        If item __sh_i of __sh_entries is equal to __sh_key:
            Return item (__sh_i + 1) of __sh_entries.
        Set __sh_i to __sh_i + 2.
    Return "".

## To __sh_has (__sh_entries: Seq of Text) and (__sh_key: Text) -> Bool:
    Let mutable __sh_i be 1.
    While __sh_i is less than length of __sh_entries:
        If item __sh_i of __sh_entries is equal to __sh_key:
            Return true.
        Set __sh_i to __sh_i + 2.
    Return false.

## To __sh_join (__sh_held: Text) and (__sh_frame: Text) -> Text:
    If __sh_held is equal to "":
        Return __sh_frame.
    If __sh_frame is equal to "":
        Return __sh_held.
    Let __sh_hk be __sh_kind(__sh_held).
    Let __sh_fk be __sh_kind(__sh_frame).
    If __sh_hk is equal to "" and __sh_fk is equal to "":
        If __sh_newer(__sh_frame, __sh_held):
            Return __sh_frame.
        Return __sh_held.
    If __sh_hk is not equal to __sh_fk:
        If __sh_after(__sh_fk, __sh_hk):
            Return __sh_frame.
        Return __sh_held.
    Return __sh_state(__sh_hk, __sh_union(__sh_entries(__sh_held), __sh_entries(__sh_frame))).

## To __sh_id_part (__sh_id: Text) and (__sh_second: Bool) -> Int:
    Let mutable __sh_out be "".
    Let mutable __sh_past be false.
    Repeat for __sh_c in __sh_id:
        If __sh_c is equal to ".":
            If __sh_second is equal to false:
                Return parseInt(__sh_out).
            Set __sh_past to true.
        Otherwise If __sh_past is equal to __sh_second:
            Set __sh_out to __sh_out + __sh_c.
    Return parseInt(__sh_out).

## To __sh_id_after (__sh_a: Text) and (__sh_b: Text) -> Bool:
    Let __sh_la be __sh_id_part(__sh_a, false).
    Let __sh_lb be __sh_id_part(__sh_b, false).
    If __sh_la is not equal to __sh_lb:
        Return __sh_la is greater than __sh_lb.
    Return __sh_id_part(__sh_a, true) is greater than __sh_id_part(__sh_b, true).

## To __sh_witness_entries (__sh_entries: Seq of Text):
    Let mutable __sh_i be 1.
    While __sh_i is less than length of __sh_entries:
        Let __sh_k be item __sh_i of __sh_entries.
        Let __sh_c be __sh_first(__sh_k).
        If __sh_c is equal to "a" or __sh_c is equal to "n":
            __sh_witness(__sh_id_part(__sh_tail(__sh_k), false)).
        Set __sh_i to __sh_i + 2.

## To __sh_counter (__sh_entries: Seq of Text) -> Int:
    Let mutable __sh_total be 0.
    Let mutable __sh_i be 1.
    While __sh_i is less than length of __sh_entries:
        Let __sh_k be __sh_first(item __sh_i of __sh_entries).
        Let __sh_v be parseInt(item (__sh_i + 1) of __sh_entries).
        If __sh_k is equal to "i":
            Set __sh_total to __sh_total + __sh_v.
        If __sh_k is equal to "d":
            Set __sh_total to __sh_total - __sh_v.
        Set __sh_i to __sh_i + 2.
    Return __sh_total.

## To __sh_count (__sh_entries: Seq of Text) and (__sh_replica: Int) and (__sh_delta: Int) -> Seq of Text:
    If __sh_delta is equal to 0:
        Return __sh_entries.
    Let mutable __sh_key be "i{__sh_replica}".
    Let mutable __sh_by be __sh_delta.
    If __sh_delta is less than 0:
        Set __sh_key to "d{__sh_replica}".
        Set __sh_by to 0 - __sh_delta.
    Let mutable __sh_now be 0.
    If __sh_has(__sh_entries, __sh_key):
        Set __sh_now to parseInt(__sh_get(__sh_entries, __sh_key)).
    Return __sh_put(__sh_entries, __sh_key, "{__sh_now + __sh_by}").

## To __sh_sorted (__sh_items: Seq of Text) and (__sh_numeric: Bool) -> Seq of Text:
    Let mutable __sh_out be a new Seq of Text.
    Repeat for __sh_x in __sh_items:
        If not (__sh_out contains __sh_x):
            Push __sh_x to __sh_out.
            Let mutable __sh_i be length of __sh_out.
            Let mutable __sh_moving be true.
            While __sh_moving:
                If __sh_i is at most 1:
                    Set __sh_moving to false.
                Otherwise:
                    Let __sh_prev be item (__sh_i - 1) of __sh_out.
                    Let mutable __sh_swap be false.
                    If __sh_numeric:
                        Set __sh_swap to parseInt(__sh_prev) is greater than parseInt(__sh_x).
                    Otherwise:
                        Set __sh_swap to __sh_after(__sh_prev, __sh_x).
                    If __sh_swap:
                        Set item __sh_i of __sh_out to __sh_prev.
                        Set item (__sh_i - 1) of __sh_out to __sh_x.
                        Set __sh_i to __sh_i - 1.
                    Otherwise:
                        Set __sh_moving to false.
    Return __sh_out.

## To __sh_members (__sh_entries: Seq of Text) and (__sh_numeric: Bool) -> Seq of Text:
    Let mutable __sh_found be a new Seq of Text.
    Let mutable __sh_i be 1.
    While __sh_i is less than length of __sh_entries:
        Let __sh_k be item __sh_i of __sh_entries.
        If __sh_first(__sh_k) is equal to "a":
            If __sh_has(__sh_entries, "r" + __sh_tail(__sh_k)) is equal to false:
                Push item (__sh_i + 1) of __sh_entries to __sh_found.
        Set __sh_i to __sh_i + 2.
    Return __sh_sorted(__sh_found, __sh_numeric).

## To __sh_set_moved (__sh_entries: Seq of Text) and (__sh_base: Seq of Text) and (__sh_now: Seq of Text) and (__sh_replica: Int) -> Seq of Text:
    Let mutable __sh_out be __sh_entries.
    Repeat for __sh_e in __sh_base:
        If not (__sh_now contains __sh_e):
            Let mutable __sh_i be 1.
            While __sh_i is less than length of __sh_entries:
                Let __sh_k be item __sh_i of __sh_entries.
                If __sh_first(__sh_k) is equal to "a" and item (__sh_i + 1) of __sh_entries is equal to __sh_e:
                    Set __sh_out to __sh_put(__sh_out, "r" + __sh_tail(__sh_k), "").
                Set __sh_i to __sh_i + 2.
    Repeat for __sh_e in __sh_now:
        If not (__sh_base contains __sh_e):
            Set __sh_out to __sh_put(__sh_out, "a{__sh_tick()}.{__sh_replica}", __sh_e).
    Return __sh_out.

## To __sh_children (__sh_ids: Seq of Text) and (__sh_parents: Seq of Text) and (__sh_of: Text) -> Seq of Text:
    Let mutable __sh_out be a new Seq of Text.
    Let mutable __sh_i be 1.
    While __sh_i is at most length of __sh_ids:
        If item __sh_i of __sh_parents is equal to __sh_of:
            Let __sh_x be item __sh_i of __sh_ids.
            Push __sh_x to __sh_out.
            Let mutable __sh_j be length of __sh_out.
            Let mutable __sh_moving be true.
            While __sh_moving:
                If __sh_j is at most 1:
                    Set __sh_moving to false.
                Otherwise If __sh_id_after(item (__sh_j - 1) of __sh_out, __sh_x):
                    Set item __sh_j of __sh_out to item (__sh_j - 1) of __sh_out.
                    Set item (__sh_j - 1) of __sh_out to __sh_x.
                    Set __sh_j to __sh_j - 1.
                Otherwise:
                    Set __sh_moving to false.
        Set __sh_i to __sh_i + 1.
    Return __sh_out.

## To __sh_sequence (__sh_entries: Seq of Text) -> Seq of Text:
    Let mutable __sh_ids be a new Seq of Text.
    Let mutable __sh_parents be a new Seq of Text.
    Let mutable __sh_elems be a new Seq of Text.
    Let mutable __sh_i be 1.
    While __sh_i is less than length of __sh_entries:
        Let __sh_k be item __sh_i of __sh_entries.
        If __sh_first(__sh_k) is equal to "n":
            Let __sh_v be item (__sh_i + 1) of __sh_entries.
            Push __sh_tail(__sh_k) to __sh_ids.
            Push __sh_before_colon(__sh_v) to __sh_parents.
            Push __sh_value(__sh_v) to __sh_elems.
        Set __sh_i to __sh_i + 2.
    Let mutable __sh_out be a new Seq of Text.
    Let mutable __sh_stack be __sh_children(__sh_ids, __sh_parents, "0").
    Let mutable __sh_top be length of __sh_stack.
    While __sh_top is at least 1:
        Let __sh_id be item __sh_top of __sh_stack.
        Set __sh_top to __sh_top - 1.
        If __sh_has(__sh_entries, "d" + __sh_id) is equal to false:
            Let mutable __sh_at be 1.
            While item __sh_at of __sh_ids is not equal to __sh_id:
                Set __sh_at to __sh_at + 1.
            Push __sh_id to __sh_out.
            Push item __sh_at of __sh_elems to __sh_out.
        Repeat for __sh_child in __sh_children(__sh_ids, __sh_parents, __sh_id):
            Set __sh_top to __sh_top + 1.
            If __sh_top is greater than length of __sh_stack:
                Push __sh_child to __sh_stack.
            Otherwise:
                Set item __sh_top of __sh_stack to __sh_child.
    Return __sh_out.

## To __sh_seq_moved (__sh_entries: Seq of Text) and (__sh_ids: Seq of Text) and (__sh_base: Seq of Text) and (__sh_now: Seq of Text) and (__sh_replica: Int) -> Seq of Text:
    Let __sh_nb be length of __sh_base.
    Let __sh_nn be length of __sh_now.
    Let mutable __sh_p be 0.
    While __sh_p is less than __sh_nb and __sh_p is less than __sh_nn and item (__sh_p + 1) of __sh_base is equal to item (__sh_p + 1) of __sh_now:
        Set __sh_p to __sh_p + 1.
    Let mutable __sh_s be 0.
    While __sh_s is less than __sh_nb - __sh_p and __sh_s is less than __sh_nn - __sh_p and item (__sh_nb - __sh_s) of __sh_base is equal to item (__sh_nn - __sh_s) of __sh_now:
        Set __sh_s to __sh_s + 1.
    Let mutable __sh_out be __sh_entries.
    Let mutable __sh_i be __sh_p + 1.
    While __sh_i is at most __sh_nb - __sh_s:
        Set __sh_out to __sh_put(__sh_out, "d" + item __sh_i of __sh_ids, "").
        Set __sh_i to __sh_i + 1.
    Let mutable __sh_parent be "0".
    If __sh_p is greater than 0:
        Set __sh_parent to item __sh_p of __sh_ids.
    Set __sh_i to __sh_p + 1.
    While __sh_i is at most __sh_nn - __sh_s:
        Let __sh_id be "{__sh_tick()}.{__sh_replica}".
        Set __sh_out to __sh_put(__sh_out, "n" + __sh_id, __sh_parent + ":" + item __sh_i of __sh_now).
        Set __sh_parent to __sh_id.
        Set __sh_i to __sh_i + 1.
    Return __sh_out.
"#;

/// What a shared store's field converges as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SharedFieldKind {
    /// A plain field: a last-writer-wins register.
    Register,
    /// `Tally`/`PNCounter` (`grows_only` false) or `ConvergentCount`/`GCounter` (true): the count
    /// every page's increments and decrements add up to.
    Counter { grows_only: bool },
    /// `SharedSet of T`: every page's adds and removes, an add winning over a concurrent remove.
    Set { elem: Symbol },
    /// `SharedSequence of T`: every page's inserts and removals, in one order every page derives.
    Sequence { elem: Symbol },
}

/// How a declared state-field type stands to a shared store: `None` for a type that is not a
/// convergent one, the kind it converges as, or — for a convergent type a shared store's field
/// cannot be — why not.
pub(crate) fn shared_field_kind(
    ty: &TypeExpr<'_>,
    interner: &Interner,
) -> Option<Result<SharedFieldKind, String>> {
    match ty {
        TypeExpr::Primitive(t) | TypeExpr::Named(t) => match interner.resolve(*t) {
            "Tally" | "PNCounter" => Some(Ok(SharedFieldKind::Counter { grows_only: false })),
            "ConvergentCount" | "GCounter" => {
                Some(Ok(SharedFieldKind::Counter { grows_only: true }))
            }
            name @ ("SharedSet" | "ORSet" | "SharedSet_AddWins" | "SharedSequence" | "RGA") => {
                Some(Err(format!(
                    "a {name} holds elements of one type: `{name} of Text`"
                )))
            }
            name @ ("SharedSet_RemoveWins"
            | "SharedSequence_YATA"
            | "CollaborativeSequence"
            | "SharedMap"
            | "ORMap"
            | "Divergent"
            | "MVRegister") => Some(Err(unsupported_shared_kind(name))),
            _ => None,
        },
        TypeExpr::Generic { base, params } => {
            let name = interner.resolve(*base);
            let element = |kind: fn(Symbol) -> SharedFieldKind| -> Result<SharedFieldKind, String> {
                match params {
                    [TypeExpr::Primitive(e) | TypeExpr::Named(e)]
                        if matches!(interner.resolve(*e), "Text" | "Int" | "Bool") =>
                    {
                        Ok(kind(*e))
                    }
                    _ => Err(format!(
                        "a {name} in a shared store holds Text, Int or Bool elements"
                    )),
                }
            };
            match name {
                "SharedSet" | "ORSet" | "SharedSet_AddWins" => {
                    Some(element(|elem| SharedFieldKind::Set { elem }))
                }
                "SharedSequence" | "RGA" => {
                    Some(element(|elem| SharedFieldKind::Sequence { elem }))
                }
                "SharedSet_RemoveWins"
                | "SharedSequence_YATA"
                | "CollaborativeSequence"
                | "SharedMap"
                | "ORMap"
                | "Divergent"
                | "MVRegister" => Some(Err(unsupported_shared_kind(name))),
                _ => None,
            }
        }
        _ => None,
    }
}

fn unsupported_shared_kind(name: &str) -> String {
    format!(
        "a {name} is not a field a shared store converges: a shared store's field is a plain value \
         (the latest write wins), a `Tally` or `ConvergentCount`, a `SharedSet of T`, or a \
         `SharedSequence of T`"
    )
}

/// The plain type a convergent field's author works with: `Int` for a counter, `Set of T` for a
/// set, `Seq of T` for a sequence — spelled the way a store's field types are (`"Seq of T"` is one
/// Symbol; see `Parser::seq_state_type`).
pub(crate) fn shared_carrier_name(kind: SharedFieldKind, interner: &Interner) -> Option<String> {
    match kind {
        SharedFieldKind::Register => None,
        SharedFieldKind::Counter { .. } => Some("Int".to_string()),
        SharedFieldKind::Set { elem } => Some(format!("Set of {}", interner.resolve(elem))),
        SharedFieldKind::Sequence { elem } => Some(format!("Seq of {}", interner.resolve(elem))),
    }
}

/// The convergent kind of a shared store's field, from its declaration — `Register` for a plain
/// field (and for one a declaration error already refused).
fn declared_kind(field: &StateField<'_>, interner: &Interner) -> SharedFieldKind {
    field
        .ty
        .and_then(|ty| shared_field_kind(ty, interner))
        .and_then(Result::ok)
        .unwrap_or(SharedFieldKind::Register)
}

/// One field of a shared store the lowering gave a record: its name, its record type, how it
/// converges, and its initializer (a counter's starting total).
#[derive(Clone)]
struct SharedField<'a> {
    name: Symbol,
    ty: Symbol,
    kind: SharedFieldKind,
    init: &'a Expr<'a>,
}

/// One shared store the lowering gave a record: its name, its topic, and its fields.
struct SharedStore<'a> {
    name: Symbol,
    topic: &'a Expr<'a>,
    fields: Vec<SharedField<'a>>,
}

impl SharedStore<'_> {
    fn converges(&self) -> bool {
        self.fields
            .iter()
            .any(|f| f.kind != SharedFieldKind::Register)
    }
}

impl<'a, 'ctx, 'int> super::super::Parser<'a, 'ctx, 'int> {
    /// What a shared store's prelude adds beside its record: the flag that says whether this
    /// replica has joined its topic, and for each convergent field the plain value it last adopted
    /// — what the next action's change is read against — and, for a sequence, the node id behind
    /// each of its elements.
    pub(crate) fn shared_store_prelude(
        &mut self,
        def: &ComponentDef<'a>,
        _fields: &[(Symbol, Symbol, bool)],
    ) -> Option<Vec<Stmt<'a>>> {
        let store = self.interner.resolve(def.name).to_string();
        let joined = self.interner.intern(&shared_joined_binding(&store));
        let mut out = vec![Stmt::Let {
            var: joined,
            ty: None,
            value: self.te_bool(false),
            mutable: true,
        }];
        for field in def.state.iter().filter(|f| !f.derived) {
            let field_name = self.interner.resolve(field.name).to_string();
            let base = self
                .interner
                .intern(&shared_base_binding(&store, &field_name));
            match declared_kind(field, self.interner) {
                SharedFieldKind::Register => {}
                SharedFieldKind::Counter { .. } => {
                    out.push(Stmt::Let {
                        var: base,
                        ty: None,
                        value: field.init,
                        mutable: true,
                    });
                }
                SharedFieldKind::Set { .. } => {
                    let texts = self.new_texts();
                    out.push(Stmt::Let {
                        var: base,
                        ty: None,
                        value: texts,
                        mutable: true,
                    });
                }
                SharedFieldKind::Sequence { .. } => {
                    let texts = self.new_texts();
                    out.push(Stmt::Let {
                        var: base,
                        ty: None,
                        value: texts,
                        mutable: true,
                    });
                    let ids = self
                        .interner
                        .intern(&shared_ids_binding(&store, &field_name));
                    let id_texts = self.new_texts();
                    out.push(Stmt::Let {
                        var: ids,
                        ty: None,
                        value: id_texts,
                        mutable: true,
                    });
                }
            }
        }
        Some(out)
    }

    /// What ends a shared store's action, once its write-back is done: every field the action moved
    /// published to the topic, and every other joined replica refreshed from it.
    pub(crate) fn shared_store_publish(
        &mut self,
        def: &ComponentDef<'a>,
        _fields: &[(Symbol, Symbol, bool)],
    ) -> Option<Vec<Stmt<'a>>> {
        let store = self.interner.resolve(def.name).to_string();
        let publish = self.interner.intern(&shared_publish_fn_name(&store));
        Some(vec![Stmt::Call {
            function: publish,
            args: Vec::new(),
        }])
    }

    /// Everything a program's shared stores need beyond their records: the Lamport clock (and, when
    /// a field converges, this page's replica id), the frame algebra, each store's
    /// join/publish/refresh, the relay-arrival entry, and a join before every statement that reads a
    /// shared store's field. `statements` is the program the author wrote; the lowered functions
    /// are in `deferred_functions`.
    pub(crate) fn lower_shared_stores(&mut self, statements: &mut Vec<Stmt<'a>>) {
        let stores = self.lowered_shared_stores(statements);
        if stores.is_empty() {
            return;
        }
        let mut emitted: Vec<Stmt<'a>> = Vec::new();
        let clock = self.interner.intern(shared_clock_binding());
        emitted.push(Stmt::Let {
            var: clock,
            ty: None,
            value: self.te_int(0),
            mutable: true,
        });
        if stores.iter().any(SharedStore::converges) {
            let replica = self.interner.intern(shared_replica_binding());
            let random = self.interner.intern("randomInt");
            let draw = self.te_call(
                random,
                vec![self.te_int(1), self.te_int(SHARED_REPLICA_MAX)],
            );
            emitted.push(Stmt::Let {
                var: replica,
                ty: None,
                value: draw,
                mutable: false,
            });
        }
        emitted.extend(self.shared_helper_functions());
        for store in &stores {
            emitted.push(self.shared_join_fn(store));
            emitted.push(self.shared_publish_fn(store, &stores));
            emitted.push(self.shared_refresh_fn(store));
        }
        emitted.push(self.lower_shared_arrival(&stores));
        let names: Vec<Symbol> = stores.iter().map(|s| s.name).collect();
        let machinery: Vec<Symbol> = emitted
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        let deferred = std::mem::take(&mut self.deferred_functions);
        self.deferred_functions = deferred
            .into_iter()
            .map(|stmt| self.join_before_reads_in_def(stmt, &names, &machinery))
            .collect();
        self.deferred_functions.extend(emitted);
        let author = std::mem::take(statements);
        *statements = self.join_before_reads_in_program(&author, &names, &machinery);
    }

    /// [`SHARED_HELPERS`], parsed into this program's arena by a parser of its own over the same
    /// interner and the program's type registry.
    fn shared_helper_functions(&mut self) -> Vec<Stmt<'a>> {
        let tokens = crate::lexer::Lexer::new(SHARED_HELPERS, self.interner).tokenize();
        let trie = crate::mwe::build_mwe_trie();
        let tokens = crate::mwe::apply_mwe_pipeline(tokens, &trie, self.interner);
        let types = match self.type_registry.clone() {
            Some(types) => types,
            None => TypeRegistry::with_primitives(self.interner),
        };
        let mut helpers = super::super::Parser::new(
            tokens,
            &mut *self.world_state,
            &mut *self.interner,
            self.ctx,
            types,
        );
        helpers.parse_program().unwrap_or_else(|e| {
            panic!(
                "the shared-store frame algebra is LOGOS the compiler ships, and it parses: {e:?}"
            )
        })
    }

    /// The `shared on` stores `lower_store` gave a record to, in declaration order.
    fn lowered_shared_stores(&mut self, statements: &[Stmt<'a>]) -> Vec<SharedStore<'a>> {
        let mut out = Vec::new();
        for stmt in statements {
            let Stmt::ComponentDef(def) = stmt else {
                continue;
            };
            let ComponentKind::Store {
                mode: StoreMode::Shared { topic },
            } = &def.kind
            else {
                continue;
            };
            let has_record = self.deferred_functions.iter().any(
                |s| matches!(s, Stmt::Let { var, value: Expr::New { .. }, .. } if *var == def.name),
            );
            if !has_record {
                continue;
            }
            let Some(types) = self.store_field_types(def) else {
                continue;
            };
            let fields = def
                .state
                .iter()
                .filter(|f| !f.derived)
                .zip(types)
                .map(|(field, (name, ty, _))| SharedField {
                    name,
                    ty,
                    kind: declared_kind(field, self.interner),
                    init: field.init,
                })
                .collect();
            out.push(SharedStore {
                name: def.name,
                topic,
                fields,
            });
        }
        out
    }

    /// `__join_<S>()`: once per page, adopt every field the topic holds and publish every plain
    /// field it does not.
    fn shared_join_fn(&mut self, store: &SharedStore<'a>) -> Stmt<'a> {
        let name = self.interner.resolve(store.name).to_string();
        let joined = self.interner.intern(&shared_joined_binding(&name));
        let mut body = vec![
            Stmt::If {
                cond: self.te_id(joined),
                then_block: self.alloc_block(vec![Stmt::Return { value: None }]),
                else_block: None,
            },
            Stmt::Set {
                target: joined,
                value: self.te_bool(true),
            },
        ];
        for field in store.fields.clone() {
            let raw = self.interner.intern("__sh_raw");
            body.push(Stmt::Let {
                var: raw,
                ty: None,
                value: self.area_read(store, field.name),
                mutable: false,
            });
            if field.kind == SharedFieldKind::Register {
                let empty = self.text_lit("");
                let text = self.field_text(store.name, field.name, field.ty);
                let stamp = self.helper_call("__sh_stamp", vec![text]);
                let publish = self.area_join_write(store, field.name, raw, stamp);
                let adopt = self.adopt_field(store, &field, raw);
                body.push(Stmt::If {
                    cond: self.te_bin(BinaryOpKind::Eq, self.te_id(raw), empty),
                    then_block: self.alloc_block(publish),
                    else_block: Some(self.alloc_block(adopt)),
                });
            } else {
                body.extend(self.adopt_field(store, &field, raw));
            }
        }
        self.shared_fn(&shared_join_fn_name(&name), Vec::new(), None, body)
    }

    /// `__publish_<S>()`: every field's change since it was last adopted, joined into the topic —
    /// a plain field that moved under a fresh stamp, a convergent field as the operations its
    /// change amounts to — and each convergent field rebuilt from the joined frame; then every
    /// other joined replica on the topic is refreshed. A `ConvergentCount` an action took DOWN is
    /// refused before anything is published: the store is put back to what its topic holds, and
    /// the action fails.
    fn shared_publish_fn(&mut self, store: &SharedStore<'a>, all: &[SharedStore<'a>]) -> Stmt<'a> {
        let name = self.interner.resolve(store.name).to_string();
        let mut body = Vec::new();
        let shrunk: Vec<SharedField<'a>> = store
            .fields
            .iter()
            .filter(|f| matches!(f.kind, SharedFieldKind::Counter { grows_only: true }))
            .cloned()
            .collect();
        for field in &shrunk {
            let field_name = self.interner.resolve(field.name).to_string();
            let base = self
                .interner
                .intern(&shared_base_binding(&name, &field_name));
            let now = self.field_read(store.name, field.name);
            let cond = self.te_bin(BinaryOpKind::Lt, now, self.te_id(base));
            let mut restore = Vec::new();
            for f in store.fields.clone() {
                let raw = self.interner.intern("__sh_raw");
                restore.push(Stmt::Let {
                    var: raw,
                    ty: None,
                    value: self.area_read(store, f.name),
                    mutable: false,
                });
                restore.extend(self.adopt_field(store, &f, raw));
                if f.kind != SharedFieldKind::Register {
                    restore.extend(self.reset_to_base(store, &f));
                }
            }
            let reason = self.ctx.alloc_imperative_expr(Expr::InterpolatedString(vec![StringPart::Literal(
                self.interner.intern(&format!(
                    "`{name}'s {field_name}` is a ConvergentCount, which only ever grows, and this action took it \
                     down; the store is back to what its topic holds. A count that goes both ways is a Tally."
                )),
            )]));
            restore.push(Stmt::Fail { reason });
            body.push(Stmt::If {
                cond,
                then_block: self.alloc_block(restore),
                else_block: None,
            });
        }
        let replica = self.interner.intern(shared_replica_binding());
        for field in store.fields.clone() {
            let field_name = self.interner.resolve(field.name).to_string();
            let raw = self.interner.intern("__sh_raw");
            body.push(Stmt::Let {
                var: raw,
                ty: None,
                value: self.area_read(store, field.name),
                mutable: false,
            });
            match field.kind {
                SharedFieldKind::Register => {
                    let now = self.interner.intern("__sh_now");
                    let text = self.field_text(store.name, field.name, field.ty);
                    body.push(Stmt::Let {
                        var: now,
                        ty: None,
                        value: text,
                        mutable: false,
                    });
                    let held_value = self.helper_call("__sh_value", vec![self.te_id(raw)]);
                    let moved = self.te_bin(BinaryOpKind::NotEq, held_value, self.te_id(now));
                    let empty = self.text_lit("");
                    let absent = self.te_bin(BinaryOpKind::Eq, self.te_id(raw), empty);
                    let stamp = self.helper_call("__sh_stamp", vec![self.te_id(now)]);
                    let write = self.area_join_write(store, field.name, raw, stamp);
                    body.push(Stmt::If {
                        cond: self.te_bin(BinaryOpKind::Or, absent, moved),
                        then_block: self.alloc_block(write),
                        else_block: None,
                    });
                }
                kind => {
                    let base = self
                        .interner
                        .intern(&shared_base_binding(&name, &field_name));
                    let (tag, entries) = match kind {
                        SharedFieldKind::Counter { .. } => {
                            let held = self.entries_of(raw, "C");
                            let now = self.field_read(store.name, field.name);
                            let delta = self.te_bin(BinaryOpKind::Subtract, now, self.te_id(base));
                            (
                                "C",
                                self.helper_call(
                                    "__sh_count",
                                    vec![held, self.te_id(replica), delta],
                                ),
                            )
                        }
                        SharedFieldKind::Set { .. } => {
                            let now = self.interner.intern("__sh_now");
                            body.extend(self.carrier_texts(store.name, field.name, now));
                            let held = self.entries_of(raw, "S");
                            let moved = self.helper_call(
                                "__sh_set_moved",
                                vec![held, self.te_id(base), self.te_id(now), self.te_id(replica)],
                            );
                            ("S", moved)
                        }
                        SharedFieldKind::Sequence { .. } => {
                            let now = self.interner.intern("__sh_now");
                            body.extend(self.carrier_texts(store.name, field.name, now));
                            let ids = self
                                .interner
                                .intern(&shared_ids_binding(&name, &field_name));
                            let held = self.entries_of(raw, "Q");
                            let moved = self.helper_call(
                                "__sh_seq_moved",
                                vec![
                                    held,
                                    self.te_id(ids),
                                    self.te_id(base),
                                    self.te_id(now),
                                    self.te_id(replica),
                                ],
                            );
                            ("Q", moved)
                        }
                        SharedFieldKind::Register => {
                            unreachable!("a plain field is published above")
                        }
                    };
                    let tag = self.text_lit(tag);
                    let frame = self.helper_call("__sh_state", vec![tag, entries]);
                    body.extend(self.area_join_write(store, field.name, raw, frame));
                    let joined = self.interner.intern("__sh_raw");
                    body.push(Stmt::Let {
                        var: joined,
                        ty: None,
                        value: self.area_read(store, field.name),
                        mutable: false,
                    });
                    body.extend(self.adopt_field(store, &field, joined));
                }
            }
        }
        for other in all.iter().filter(|o| o.name != store.name) {
            let other_topic = self.topic_text(other);
            let this_topic = self.topic_text(store);
            let same_topic = self.te_bin(BinaryOpKind::Eq, other_topic, this_topic);
            let refresh = self
                .interner
                .intern(&shared_refresh_fn_name(self.interner.resolve(other.name)));
            body.push(Stmt::If {
                cond: same_topic,
                then_block: self.alloc_block(vec![Stmt::Call {
                    function: refresh,
                    args: Vec::new(),
                }]),
                else_block: None,
            });
        }
        self.shared_fn(&shared_publish_fn_name(&name), Vec::new(), None, body)
    }

    /// `__refresh_<S>()`: a joined replica adopts every field the topic holds.
    fn shared_refresh_fn(&mut self, store: &SharedStore<'a>) -> Stmt<'a> {
        let name = self.interner.resolve(store.name).to_string();
        let joined = self.interner.intern(&shared_joined_binding(&name));
        let mut body = vec![Stmt::If {
            cond: self.te_not(self.te_id(joined)),
            then_block: self.alloc_block(vec![Stmt::Return { value: None }]),
            else_block: None,
        }];
        for field in store.fields.clone() {
            let raw = self.interner.intern("__sh_raw");
            body.push(Stmt::Let {
                var: raw,
                ty: None,
                value: self.area_read(store, field.name),
                mutable: false,
            });
            body.extend(self.adopt_field(store, &field, raw));
        }
        self.shared_fn(&shared_refresh_fn_name(&name), Vec::new(), None, body)
    }

    /// `__lgx_shared(topic, key, frame)` — a frame another page published on `topic` for the field
    /// stored under `key`. The frame is joined into the one held; when that moved it, every joined
    /// replica on the topic adopts the result and the page repaints.
    fn lower_shared_arrival(&mut self, stores: &[SharedStore<'a>]) -> Stmt<'a> {
        let text = self.text_type();
        let topic = self.interner.intern("__sh_topic");
        let key = self.interner.intern("__sh_key");
        let frame = self.interner.intern("__sh_frame_text");
        let held = self.interner.intern("__sh_held");
        let merged = self.interner.intern("__sh_merged");
        let get = self.interner.intern("storageGetOr");
        let set = self.interner.intern("storageSet");
        let area = self.text_lit(SHARED_AREA);
        let empty = self.text_lit("");
        let mut moved = vec![Stmt::Call {
            function: set,
            args: vec![area, self.te_id(key), self.te_id(merged)],
        }];
        for store in stores {
            let refresh = self
                .interner
                .intern(&shared_refresh_fn_name(self.interner.resolve(store.name)));
            let store_topic = self.topic_text(store);
            moved.push(Stmt::If {
                cond: self.te_bin(BinaryOpKind::Eq, store_topic, self.te_id(topic)),
                then_block: self.alloc_block(vec![Stmt::Call {
                    function: refresh,
                    args: Vec::new(),
                }]),
                else_block: None,
            });
        }
        let repaint = self.interner.intern(repaint_root_fn_name());
        moved.push(Stmt::Call {
            function: repaint,
            args: Vec::new(),
        });
        let area = self.text_lit(SHARED_AREA);
        let join = self.helper_call("__sh_join", vec![self.te_id(held), self.te_id(frame)]);
        let body = vec![
            Stmt::Let {
                var: held,
                ty: None,
                value: self.te_call(get, vec![area, self.te_id(key), empty]),
                mutable: false,
            },
            Stmt::Let {
                var: merged,
                ty: None,
                value: join,
                mutable: false,
            },
            Stmt::If {
                cond: self.te_bin(BinaryOpKind::NotEq, self.te_id(merged), self.te_id(held)),
                then_block: self.alloc_block(moved),
                else_block: None,
            },
        ];
        self.shared_fn(
            shared_arrival_fn_name(),
            vec![(topic, text), (key, text), (frame, text)],
            None,
            body,
        )
    }

    /// Set the store's `field` from the frame in `raw`, when it holds one of the kind the field
    /// converges as: a plain field decodes the frame's value (and moves the clock past its stamp);
    /// a counter is its starting total plus the frame's count; a set is the frame's members; a
    /// sequence the frame's elements, in order. What was adopted is what the next action's change
    /// is read against.
    fn adopt_field(
        &mut self,
        store: &SharedStore<'a>,
        field: &SharedField<'a>,
        raw: Symbol,
    ) -> Vec<Stmt<'a>> {
        let name = self.interner.resolve(store.name).to_string();
        let field_name = self.interner.resolve(field.name).to_string();
        let base_store = self.te_id(store.name);
        let tag = match field.kind {
            SharedFieldKind::Register => "",
            SharedFieldKind::Counter { .. } => "C",
            SharedFieldKind::Set { .. } => "S",
            SharedFieldKind::Sequence { .. } => "Q",
        };
        let kind = self.helper_call("__sh_kind", vec![self.te_id(raw)]);
        let tag_text = self.text_lit(tag);
        let mut adopt = Vec::new();
        match field.kind {
            SharedFieldKind::Register => {
                let empty = self.text_lit("");
                let lamport = self.helper_call("__sh_lamport", vec![self.te_id(raw)]);
                adopt.push(Stmt::Call {
                    function: self.interner.intern("__sh_witness"),
                    args: vec![lamport],
                });
                let held = self.interner.intern("__sh_held_value");
                let value = self.helper_call("__sh_value", vec![self.te_id(raw)]);
                adopt.push(Stmt::Let {
                    var: held,
                    ty: None,
                    value,
                    mutable: false,
                });
                if self.needs_json_codec(field.ty) {
                    if let Some((_, dec)) = self.json_codec_for(field.ty) {
                        let decoded = self.te_call(dec, vec![self.te_id(held)]);
                        let err = self.interner.intern("__sh_read_err");
                        let ok = self.interner.intern("__sh_read_v");
                        let no_op = Stmt::Splice {
                            body: self.alloc_block(Vec::new()),
                        };
                        let set = Stmt::SetField {
                            object: base_store,
                            field: field.name,
                            value: self.te_id(ok),
                        };
                        adopt.push(self.inspect_result(
                            decoded,
                            super::json::ResultShape::Result,
                            err,
                            no_op,
                            ok,
                            vec![set],
                        ));
                    }
                } else {
                    let value = self.storage_parse(held, field.ty);
                    adopt.push(Stmt::SetField {
                        object: base_store,
                        field: field.name,
                        value,
                    });
                }
                let present = self.te_bin(BinaryOpKind::NotEq, self.te_id(raw), empty);
                let plain = self.te_bin(BinaryOpKind::Eq, kind, tag_text);
                return vec![Stmt::If {
                    cond: self.te_bin(BinaryOpKind::And, present, plain),
                    then_block: self.alloc_block(adopt),
                    else_block: None,
                }];
            }
            SharedFieldKind::Counter { .. } => {
                let base = self
                    .interner
                    .intern(&shared_base_binding(&name, &field_name));
                let entries = self.helper_call("__sh_entries", vec![self.te_id(raw)]);
                let count = self.helper_call("__sh_counter", vec![entries]);
                let value = self.te_bin(BinaryOpKind::Add, field.init, count);
                adopt.push(Stmt::SetField {
                    object: base_store,
                    field: field.name,
                    value,
                });
                let now = self.field_read(store.name, field.name);
                adopt.push(Stmt::Set {
                    target: base,
                    value: now,
                });
            }
            SharedFieldKind::Set { elem } | SharedFieldKind::Sequence { elem } => {
                let is_set = matches!(field.kind, SharedFieldKind::Set { .. });
                let base = self
                    .interner
                    .intern(&shared_base_binding(&name, &field_name));
                let entries = self.interner.intern("__sh_ent");
                let value = self.helper_call("__sh_entries", vec![self.te_id(raw)]);
                adopt.push(Stmt::Let {
                    var: entries,
                    ty: None,
                    value,
                    mutable: false,
                });
                adopt.push(Stmt::Call {
                    function: self.interner.intern("__sh_witness_entries"),
                    args: vec![self.te_id(entries)],
                });
                let texts = self.interner.intern("__sh_texts");
                let built = self.interner.intern("__sh_built");
                let x = self.interner.intern("__sh_x");
                let parsed = self.storage_parse(x, elem);
                if is_set {
                    let numeric = self.te_bool(self.interner.resolve(elem) == "Int");
                    let members =
                        self.helper_call("__sh_members", vec![self.te_id(entries), numeric]);
                    adopt.push(Stmt::Let {
                        var: texts,
                        ty: None,
                        value: members,
                        mutable: false,
                    });
                    let set = self.new_collection("Set", elem);
                    adopt.push(Stmt::Let {
                        var: built,
                        ty: None,
                        value: set,
                        mutable: true,
                    });
                    adopt.push(Stmt::Repeat {
                        pattern: Pattern::Identifier(x),
                        iterable: self.te_id(texts),
                        body: self.alloc_block(vec![Stmt::Add {
                            value: parsed,
                            collection: self.te_id(built),
                        }]),
                    });
                } else {
                    let ids = self
                        .interner
                        .intern(&shared_ids_binding(&name, &field_name));
                    let pairs = self.interner.intern("__sh_pairs");
                    let at = self.interner.intern("__sh_at");
                    let value = self.helper_call("__sh_sequence", vec![self.te_id(entries)]);
                    adopt.push(Stmt::Let {
                        var: pairs,
                        ty: None,
                        value,
                        mutable: false,
                    });
                    let fresh_ids = self.new_texts();
                    adopt.push(Stmt::Set {
                        target: ids,
                        value: fresh_ids,
                    });
                    let fresh_texts = self.new_texts();
                    adopt.push(Stmt::Let {
                        var: texts,
                        ty: None,
                        value: fresh_texts,
                        mutable: true,
                    });
                    let seq = self.new_collection("Seq", elem);
                    adopt.push(Stmt::Let {
                        var: built,
                        ty: None,
                        value: seq,
                        mutable: true,
                    });
                    adopt.push(Stmt::Let {
                        var: at,
                        ty: None,
                        value: self.te_int(1),
                        mutable: true,
                    });
                    let id_at = self.ctx.alloc_imperative_expr(Expr::Index {
                        collection: self.te_id(pairs),
                        index: self.te_id(at),
                    });
                    let next = self.te_bin(BinaryOpKind::Add, self.te_id(at), self.te_int(1));
                    let text_at = self.ctx.alloc_imperative_expr(Expr::Index {
                        collection: self.te_id(pairs),
                        index: next,
                    });
                    let len = self.ctx.alloc_imperative_expr(Expr::Length {
                        collection: self.te_id(pairs),
                    });
                    adopt.push(Stmt::While {
                        cond: self.te_bin(BinaryOpKind::Lt, self.te_id(at), len),
                        body: self.alloc_block(vec![
                            Stmt::Push {
                                value: id_at,
                                collection: self.te_id(ids),
                            },
                            Stmt::Let {
                                var: x,
                                ty: None,
                                value: text_at,
                                mutable: false,
                            },
                            Stmt::Push {
                                value: self.te_id(x),
                                collection: self.te_id(texts),
                            },
                            Stmt::Push {
                                value: parsed,
                                collection: self.te_id(built),
                            },
                            Stmt::Set {
                                target: at,
                                value: self.te_bin(
                                    BinaryOpKind::Add,
                                    self.te_id(at),
                                    self.te_int(2),
                                ),
                            },
                        ]),
                        decreasing: None,
                    });
                }
                adopt.push(Stmt::SetField {
                    object: base_store,
                    field: field.name,
                    value: self.te_id(built),
                });
                adopt.push(Stmt::Set {
                    target: base,
                    value: self.te_id(texts),
                });
            }
        }
        vec![Stmt::If {
            cond: self.te_bin(BinaryOpKind::Eq, kind, tag_text),
            then_block: self.alloc_block(adopt),
            else_block: None,
        }]
    }

    /// A convergent field put back to the value it last adopted when its topic holds nothing of its
    /// kind — the case [`Self::adopt_field`] leaves alone.
    fn reset_to_base(&mut self, store: &SharedStore<'a>, field: &SharedField<'a>) -> Vec<Stmt<'a>> {
        let name = self.interner.resolve(store.name).to_string();
        let field_name = self.interner.resolve(field.name).to_string();
        let base = self
            .interner
            .intern(&shared_base_binding(&name, &field_name));
        let raw = self.interner.intern("__sh_raw");
        let tag = match field.kind {
            SharedFieldKind::Counter { .. } => "C",
            SharedFieldKind::Set { .. } => "S",
            SharedFieldKind::Sequence { .. } => "Q",
            SharedFieldKind::Register => return Vec::new(),
        };
        let kind = self.helper_call("__sh_kind", vec![self.te_id(raw)]);
        let tag_text = self.text_lit(tag);
        let value = match field.kind {
            SharedFieldKind::Counter { .. } => self.te_id(base),
            SharedFieldKind::Set { elem } | SharedFieldKind::Sequence { elem } => {
                let built = self.interner.intern("__sh_built");
                let x = self.interner.intern("__sh_x");
                let parsed = self.storage_parse(x, elem);
                let (collection, grow) = match field.kind {
                    SharedFieldKind::Set { .. } => (
                        "Set",
                        Stmt::Add {
                            value: parsed,
                            collection: self.te_id(built),
                        },
                    ),
                    _ => (
                        "Seq",
                        Stmt::Push {
                            value: parsed,
                            collection: self.te_id(built),
                        },
                    ),
                };
                let fresh = self.new_collection(collection, elem);
                let rebuild = vec![
                    Stmt::Let {
                        var: built,
                        ty: None,
                        value: fresh,
                        mutable: true,
                    },
                    Stmt::Repeat {
                        pattern: Pattern::Identifier(x),
                        iterable: self.te_id(base),
                        body: self.alloc_block(vec![grow]),
                    },
                    Stmt::SetField {
                        object: self.te_id(store.name),
                        field: field.name,
                        value: self.te_id(built),
                    },
                ];
                return vec![Stmt::If {
                    cond: self.te_bin(BinaryOpKind::NotEq, kind, tag_text),
                    then_block: self.alloc_block(rebuild),
                    else_block: None,
                }];
            }
            SharedFieldKind::Register => unreachable!("returned above"),
        };
        vec![Stmt::If {
            cond: self.te_bin(BinaryOpKind::NotEq, kind, tag_text),
            then_block: self.alloc_block(vec![Stmt::SetField {
                object: self.te_id(store.name),
                field: field.name,
                value,
            }]),
            else_block: None,
        }]
    }

    /// `storageGetOr("shared", "<topic>.<field>", "")`.
    fn area_read(&mut self, store: &SharedStore<'a>, field: Symbol) -> &'a Expr<'a> {
        let get = self.interner.intern("storageGetOr");
        let area = self.text_lit(SHARED_AREA);
        let key = self.area_key(store, field);
        let empty = self.text_lit("");
        self.te_call(get, vec![area, key, empty])
    }

    /// `frame` joined into the frame held in `raw`, written back when that moved it.
    fn area_join_write(
        &mut self,
        store: &SharedStore<'a>,
        field: Symbol,
        raw: Symbol,
        frame: &'a Expr<'a>,
    ) -> Vec<Stmt<'a>> {
        let set = self.interner.intern("storageSet");
        let merged = self.interner.intern("__sh_merged");
        let join = self.helper_call("__sh_join", vec![self.te_id(raw), frame]);
        let area = self.text_lit(SHARED_AREA);
        let key = self.area_key(store, field);
        vec![
            Stmt::Let {
                var: merged,
                ty: None,
                value: join,
                mutable: false,
            },
            Stmt::If {
                cond: self.te_bin(BinaryOpKind::NotEq, self.te_id(merged), self.te_id(raw)),
                then_block: self.alloc_block(vec![Stmt::Call {
                    function: set,
                    args: vec![area, key, self.te_id(merged)],
                }]),
                else_block: None,
            },
        ]
    }

    /// `"<topic>.<field>"` — the key a field is shared under, whatever store on the topic holds it.
    fn area_key(&mut self, store: &SharedStore<'a>, field: Symbol) -> &'a Expr<'a> {
        let suffix = self
            .interner
            .intern(&format!(".{}", self.interner.resolve(field)));
        self.ctx
            .alloc_imperative_expr(Expr::InterpolatedString(vec![
                StringPart::Expr {
                    value: store.topic,
                    format_spec: None,
                    debug: false,
                },
                StringPart::Literal(suffix),
            ]))
    }

    /// The store's topic as Text.
    fn topic_text(&mut self, store: &SharedStore<'a>) -> &'a Expr<'a> {
        self.as_text(store.topic)
    }

    /// `<store>'s <field>`.
    fn field_read(&mut self, store: Symbol, field: Symbol) -> &'a Expr<'a> {
        self.ctx.alloc_imperative_expr(Expr::FieldAccess {
            object: self.te_id(store),
            field,
        })
    }

    /// A plain field's value as the text the topic carries: its own text for a scalar, its JSON
    /// for a `Seq` or record — the persisted store's encoding.
    fn field_text(&mut self, store: Symbol, field: Symbol, ty: Symbol) -> &'a Expr<'a> {
        let read = self.field_read(store, field);
        if self.needs_json_codec(ty) {
            if let Some((enc, _)) = self.json_codec_for(ty) {
                return self.te_call(enc, vec![read]);
            }
        }
        self.as_text(read)
    }

    /// `Let <into> be` the text of every element of a set or sequence field, in its order.
    fn carrier_texts(&mut self, store: Symbol, field: Symbol, into: Symbol) -> Vec<Stmt<'a>> {
        let x = self.interner.intern("__sh_x");
        let texts = self.new_texts();
        let each = self.as_text(self.te_id(x));
        vec![
            Stmt::Let {
                var: into,
                ty: None,
                value: texts,
                mutable: true,
            },
            Stmt::Repeat {
                pattern: Pattern::Identifier(x),
                iterable: self.field_read(store, field),
                body: self.alloc_block(vec![Stmt::Push {
                    value: each,
                    collection: self.te_id(into),
                }]),
            },
        ]
    }

    /// `__sh_entries_of(raw, "<kind>")` — the entries of the frame in `raw` when it is of that kind,
    /// none otherwise.
    fn entries_of(&mut self, raw: Symbol, kind: &str) -> &'a Expr<'a> {
        let kind = self.text_lit(kind);
        self.helper_call("__sh_entries_of", vec![self.te_id(raw), kind])
    }

    /// A call of one of [`SHARED_HELPERS`]' functions.
    fn helper_call(&mut self, name: &str, args: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
        let f = self.interner.intern(name);
        self.te_call(f, args)
    }

    /// `a new Seq of Text`.
    fn new_texts(&mut self) -> &'a Expr<'a> {
        let text = self.interner.intern("Text");
        self.new_seq_expr(text)
    }

    /// `a new <collection> of <elem>`.
    fn new_collection(&mut self, collection: &str, elem: Symbol) -> &'a Expr<'a> {
        let type_name = self.interner.intern(collection);
        self.ctx.alloc_imperative_expr(Expr::New {
            type_name,
            type_args: vec![TypeExpr::Primitive(elem)],
            init_fields: Vec::new(),
        })
    }

    /// The Text literal `s`.
    fn text_lit(&mut self, s: &str) -> &'a Expr<'a> {
        let sym = self.interner.intern(s);
        self.te_text(sym)
    }

    fn shared_fn(
        &mut self,
        name: &str,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        return_type: Option<&'a TypeExpr<'a>>,
        body: Vec<Stmt<'a>>,
    ) -> Stmt<'a> {
        Stmt::FunctionDef {
            name: self.interner.intern(name),
            generics: Vec::new(),
            params,
            body: self.alloc_block(body),
            return_type,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: crate::optimization::OptimizationConfig::all_on(),
        }
    }

    /// A lowered definition with a join before every statement in its body that reads a shared
    /// store's field — every function that runs in the page, but the shared-store machinery itself.
    /// A server render (`__html_<C>`) never joins: the server holds no topic, and draws a shared
    /// store from its initializers as it draws every store.
    fn join_before_reads_in_def(
        &mut self,
        stmt: Stmt<'a>,
        stores: &[Symbol],
        machinery: &[Symbol],
    ) -> Stmt<'a> {
        match stmt {
            Stmt::FunctionDef { name, .. } if machinery.contains(&name) => stmt,
            Stmt::FunctionDef { name, .. }
                if !is_browser_only_lowering(self.interner.resolve(name)) =>
            {
                stmt
            }
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } => Stmt::FunctionDef {
                name,
                generics,
                params,
                body: {
                    let joined = self.join_before_reads(body, stores, machinery);
                    self.alloc_block(joined)
                },
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            },
            other => other,
        }
    }

    /// The author's program with joins before its reads: its top-level statements, its tests, and the
    /// bodies of the functions it defines, which run in the page.
    fn join_before_reads_in_program(
        &mut self,
        program: &[Stmt<'a>],
        stores: &[Symbol],
        machinery: &[Symbol],
    ) -> Vec<Stmt<'a>> {
        program
            .iter()
            .map(|stmt| match stmt {
                Stmt::FunctionDef {
                    name,
                    generics,
                    params,
                    body,
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                } => {
                    let joined = self.join_before_reads(body, stores, machinery);
                    Stmt::FunctionDef {
                        name: *name,
                        generics: generics.clone(),
                        params: params.clone(),
                        body: self.alloc_block(joined),
                        return_type: *return_type,
                        is_native: *is_native,
                        native_path: *native_path,
                        is_exported: *is_exported,
                        export_target: *export_target,
                        opt_flags: opt_flags.clone(),
                    }
                }
                Stmt::TestDef {
                    name,
                    skip,
                    capabilities,
                    budget,
                    cases,
                    body,
                } => {
                    let mut read = Vec::new();
                    stmt_reads(stmt, stores, &mut read);
                    let mut joined: Vec<Stmt<'a>> = read
                        .into_iter()
                        .map(|store| {
                            let join = self
                                .interner
                                .intern(&shared_join_fn_name(self.interner.resolve(store)));
                            Stmt::Call {
                                function: join,
                                args: Vec::new(),
                            }
                        })
                        .collect();
                    joined.extend(self.join_before_reads(body, stores, machinery));
                    Stmt::TestDef {
                        name: *name,
                        skip: *skip,
                        capabilities: capabilities.clone(),
                        budget: *budget,
                        cases: cases.clone(),
                        body: self.alloc_block(joined),
                    }
                }
                other => {
                    let mut joined =
                        self.join_before_reads(std::slice::from_ref(other), stores, machinery);
                    if joined.len() == 1 {
                        joined.pop().expect("one statement")
                    } else {
                        Stmt::Splice {
                            body: self.alloc_block(joined),
                        }
                    }
                }
            })
            .collect()
    }

    /// `block` with `Call __join_<S>.` before each statement that reads a field of a shared store
    /// `S` in its own expressions — a condition, an operand, a closure it creates — descending into
    /// every nested block, so a store is joined no earlier than the first statement that could
    /// read it. Joins are idempotent, so a read already joined costs a flag test.
    fn join_before_reads(
        &mut self,
        block: &[Stmt<'a>],
        stores: &[Symbol],
        machinery: &[Symbol],
    ) -> Vec<Stmt<'a>> {
        let mut out = Vec::with_capacity(block.len());
        for stmt in block {
            let mut read = Vec::new();
            stmt_reads(stmt, stores, &mut read);
            for store in read {
                let join = self
                    .interner
                    .intern(&shared_join_fn_name(self.interner.resolve(store)));
                out.push(Stmt::Call {
                    function: join,
                    args: Vec::new(),
                });
            }
            out.push(self.with_nested_joins(stmt, stores, machinery));
        }
        out
    }

    /// `stmt` with [`Self::join_before_reads`] applied to every block it holds.
    fn with_nested_joins(
        &mut self,
        stmt: &Stmt<'a>,
        stores: &[Symbol],
        machinery: &[Symbol],
    ) -> Stmt<'a> {
        let nested =
            |me: &mut Self, b: crate::ast::stmt::Block<'a>| -> crate::ast::stmt::Block<'a> {
                let rewritten = me.join_before_reads(b, stores, machinery);
                me.alloc_block(rewritten)
            };
        match stmt {
            Stmt::FunctionDef { name, .. } if machinery.contains(name) => stmt.clone(),
            Stmt::FunctionDef { .. } => {
                self.join_before_reads_in_def(stmt.clone(), stores, machinery)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => Stmt::If {
                cond,
                then_block: nested(self, then_block),
                else_block: else_block.map(|b| nested(self, b)),
            },
            Stmt::While {
                cond,
                body,
                decreasing,
            } => Stmt::While {
                cond,
                body: nested(self, body),
                decreasing: *decreasing,
            },
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => Stmt::Repeat {
                pattern: pattern.clone(),
                iterable,
                body: nested(self, body),
            },
            Stmt::Inspect {
                target,
                arms,
                has_otherwise,
            } => Stmt::Inspect {
                target,
                arms: arms
                    .iter()
                    .map(|arm| crate::ast::stmt::MatchArm {
                        enum_name: arm.enum_name,
                        variant: arm.variant,
                        bindings: arm.bindings.clone(),
                        body: nested(self, arm.body),
                    })
                    .collect(),
                has_otherwise: *has_otherwise,
            },
            Stmt::TestDef {
                name,
                skip,
                capabilities,
                budget,
                cases,
                body,
            } => Stmt::TestDef {
                name: *name,
                skip: *skip,
                capabilities: capabilities.clone(),
                budget: *budget,
                cases: cases.clone(),
                body: nested(self, body),
            },
            Stmt::ExpectFail { message, body } => Stmt::ExpectFail {
                message: *message,
                body: nested(self, body),
            },
            Stmt::During { label, body } => Stmt::During {
                label: *label,
                body: nested(self, body),
            },
            Stmt::ForAll {
                vars,
                domain,
                constraint,
                body,
            } => Stmt::ForAll {
                vars: vars.clone(),
                domain: *domain,
                constraint: *constraint,
                body: nested(self, body),
            },
            Stmt::Splice { body } => Stmt::Splice {
                body: nested(self, body),
            },
            Stmt::Reversibly { body } => Stmt::Reversibly {
                body: nested(self, body),
            },
            Stmt::Zone {
                name,
                capacity,
                source_file,
                body,
            } => Stmt::Zone {
                name: *name,
                capacity: *capacity,
                source_file: source_file.clone(),
                body: nested(self, body),
            },
            Stmt::Concurrent { tasks } => Stmt::Concurrent {
                tasks: nested(self, tasks),
            },
            Stmt::Parallel { tasks } => Stmt::Parallel {
                tasks: nested(self, tasks),
            },
            Stmt::Select { branches } => Stmt::Select {
                branches: branches
                    .iter()
                    .map(|branch| match branch {
                        SelectBranch::Receive { var, pipe, body } => SelectBranch::Receive {
                            var: *var,
                            pipe,
                            body: nested(self, body),
                        },
                        SelectBranch::Timeout { milliseconds, body } => SelectBranch::Timeout {
                            milliseconds,
                            body: nested(self, body),
                        },
                    })
                    .collect(),
            },
            Stmt::DirectiveDef {
                name,
                params,
                body,
                removed,
            } => Stmt::DirectiveDef {
                name: *name,
                params: params.clone(),
                body: nested(self, body),
                removed: removed.map(|b| nested(self, b)),
            },
            Stmt::MiddlewareDef { param, body } => Stmt::MiddlewareDef {
                param: *param,
                body: nested(self, body),
            },
            other => other.clone(),
        }
    }
}

/// The largest replica id a page draws — `2^53 - 1`, exact on every tier's number type.
const SHARED_REPLICA_MAX: i64 = 9_007_199_254_740_991;

/// The stores of `stores` whose fields `stmt` reads in its OWN expressions — not in the blocks it
/// holds, which [`join_before_reads`] reaches on its own — in first-mention order.
fn stmt_reads(stmt: &Stmt<'_>, stores: &[Symbol], out: &mut Vec<Symbol>) {
    let e = |x: &Expr<'_>, out: &mut Vec<Symbol>| expr_reads(x, stores, out);
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => e(value, out),
        Stmt::Call { args, .. }
        | Stmt::LaunchTask { args, .. }
        | Stmt::LaunchTaskWithHandle { args, .. } => args.iter().for_each(|a| e(a, out)),
        Stmt::If { cond, .. } => e(cond, out),
        Stmt::While {
            cond, decreasing, ..
        } => {
            e(cond, out);
            if let Some(d) = decreasing {
                e(d, out);
            }
        }
        Stmt::Repeat { iterable, .. } => e(iterable, out),
        Stmt::Return { value } => {
            if let Some(v) = value {
                e(v, out);
            }
        }
        Stmt::RuntimeAssert {
            condition, reason, ..
        } => {
            e(condition, out);
            if let Some(r) = reason {
                e(r, out);
            }
        }
        Stmt::Fail { reason } => e(reason, out),
        Stmt::Give { object, recipient } | Stmt::Show { object, recipient } => {
            e(object, out);
            e(recipient, out);
        }
        Stmt::SetField { object, value, .. } => {
            e(object, out);
            e(value, out);
        }
        Stmt::Expect { condition } => e(condition, out),
        Stmt::AdvanceTime { millis } => e(millis, out),
        Stmt::Observe { payloads, .. } => payloads.iter().for_each(|(_, p)| e(p, out)),
        Stmt::ForAll { constraint, .. } => {
            if let Some(c) = constraint {
                e(c, out);
            }
        }
        Stmt::Inspect { target, .. } => e(target, out),
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            e(value, out);
            e(collection, out);
        }
        Stmt::Pop { collection, .. } => e(collection, out),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            e(collection, out);
            e(index, out);
            e(value, out);
        }
        Stmt::ReadFrom { source, .. } => {
            if let crate::ast::stmt::ReadSource::File(path) = source {
                e(path, out);
            }
        }
        Stmt::WriteFile { content, path } => {
            e(content, out);
            e(path, out);
        }
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => {
            e(message, out);
            e(destination, out);
        }
        Stmt::AwaitMessage { source, .. } => e(source, out),
        Stmt::StreamMessage {
            values,
            destination,
        } => {
            e(values, out);
            e(destination, out);
        }
        Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
            e(source, out);
            e(target, out);
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            e(object, out);
            e(amount, out);
        }
        Stmt::AppendToSequence { sequence, value } => {
            e(sequence, out);
            e(value, out);
        }
        Stmt::ResolveConflict { object, value, .. } => {
            e(object, out);
            e(value, out);
        }
        Stmt::Listen { address, secure } | Stmt::ConnectTo { address, secure } => {
            e(address, out);
            if let Some(pad) = secure {
                e(pad.pad, out);
            }
        }
        Stmt::Serve { port, handler } => {
            e(port, out);
            e(handler, out);
        }
        Stmt::LetPeerAgent { address, .. } => e(address, out),
        Stmt::Sleep { milliseconds } => e(milliseconds, out),
        Stmt::Sync { topic, .. } => e(topic, out),
        Stmt::Ui(ui) => ui_reads(ui, stores, out),
        Stmt::Fetch { url, body, .. } => {
            e(url, out);
            if let Some(b) = body {
                e(b, out);
            }
        }
        Stmt::GivenFetch {
            url, status, body, ..
        } => {
            e(url, out);
            e(status, out);
            e(body, out);
        }
        Stmt::Mount { path, .. } => e(path, out),
        Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
            e(value, out);
            e(pipe, out);
        }
        Stmt::ReceivePipe { pipe, .. } | Stmt::TryReceivePipe { pipe, .. } => e(pipe, out),
        Stmt::StopTask { handle } | Stmt::AwaitTask { handle, .. } => e(handle, out),
        Stmt::CloseChannel { channel } => e(channel, out),
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { pipe, .. } => e(pipe, out),
                    SelectBranch::Timeout { milliseconds, .. } => e(milliseconds, out),
                }
            }
        }
        Stmt::TestDef { cases, .. } => {
            if let Some(table) = cases {
                table.rows.iter().flatten().for_each(|c| e(c, out));
            }
        }
        Stmt::Swap { .. }
        | Stmt::Break
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::StructDef { .. }
        | Stmt::FunctionDef { .. }
        | Stmt::ExpectFail { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::GivenFile { .. }
        | Stmt::During { .. }
        | Stmt::Splice { .. }
        | Stmt::Reversibly { .. }
        | Stmt::Zone { .. }
        | Stmt::Concurrent { .. }
        | Stmt::Parallel { .. }
        | Stmt::Spawn { .. }
        | Stmt::Check { .. }
        | Stmt::Replay { .. }
        | Stmt::ServeFetches
        | Stmt::ComponentDef(_)
        | Stmt::DirectiveDef { .. }
        | Stmt::RouteDef { .. }
        | Stmt::MiddlewareDef { .. }
        | Stmt::RedirectDef { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_)
        | Stmt::Escape { .. }
        | Stmt::Require { .. }
        | Stmt::Site(_) => {}
    }
}

fn ui_reads(ui: &UiStmt<'_>, stores: &[Symbol], out: &mut Vec<Symbol>) {
    let e = |x: &Expr<'_>, out: &mut Vec<Symbol>| expr_reads(x, stores, out);
    match ui {
        UiStmt::Focus { selector }
        | UiStmt::Blur { selector }
        | UiStmt::Hover { selector }
        | UiStmt::ScrollTo { selector }
        | UiStmt::SelectText { selector }
        | UiStmt::Click { selector }
        | UiStmt::Submit { selector } => e(selector, out),
        UiStmt::AddClass { class, selector } | UiStmt::RemoveClass { class, selector } => {
            e(class, out);
            e(selector, out);
        }
        UiStmt::CopyToClipboard { text } => e(text, out),
        UiStmt::Download { bytes, name } => {
            e(bytes, out);
            e(name, out);
        }
        UiStmt::Navigate { url } | UiStmt::ReplaceUrl { url } => e(url, out),
        UiStmt::RaiseWindowEvent {
            event,
            target,
            value,
        } => {
            e(event, out);
            e(target, out);
            e(value, out);
        }
        UiStmt::SetTitle { title } => e(title, out),
        UiStmt::Save { value, key } => {
            e(value, out);
            e(key, out);
        }
        UiStmt::Load { key, .. } | UiStmt::Forget { key } => e(key, out),
        UiStmt::Notify { message, .. } => e(message, out),
        UiStmt::Publish { message, topic } => {
            e(message, out);
            e(topic, out);
        }
        UiStmt::CallStore { store, args, .. } => {
            args.iter().for_each(|a| e(a, out));
            if stores.contains(store) && !out.contains(store) {
                out.push(*store);
            }
        }
        UiStmt::Choose { value, selector } => {
            e(value, out);
            e(selector, out);
        }
        UiStmt::Press { key, selector } => {
            e(key, out);
            if let Some(s) = selector {
                e(s, out);
            }
        }
        UiStmt::TypeInto { text, selector } => {
            e(text, out);
            e(selector, out);
        }
        UiStmt::Mount { selector, .. } => {
            if let Some(s) = selector {
                e(s, out);
            }
        }
        UiStmt::GoBack
        | UiStmt::Reload
        | UiStmt::Toggle { .. }
        | UiStmt::StopTimer { .. }
        | UiStmt::StartTimer { .. } => {}
    }
}

/// Every store of `stores` whose field `expr` reads (`<S>'s <field>`), in first-mention order. A
/// closure's body counts as the expression's own: the closure is created here, and a join here is
/// before anything it could read. A view value reads whatever its markup reads, which this walk
/// does not look into, so it is taken to read every shared store.
fn expr_reads(expr: &Expr<'_>, stores: &[Symbol], out: &mut Vec<Symbol>) {
    let note = |s: Symbol, out: &mut Vec<Symbol>| {
        if stores.contains(&s) && !out.contains(&s) {
            out.push(s);
        }
    };
    match expr {
        Expr::FieldAccess {
            object: Expr::Identifier(s),
            ..
        } => note(*s, out),
        Expr::FieldAccess { object, .. } => expr_reads(object, stores, out),
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone | Expr::Escape { .. } => {}
        Expr::BinaryOp { left, right, .. }
        | Expr::Contains {
            collection: left,
            value: right,
        }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        }
        | Expr::Index {
            collection: left,
            index: right,
        }
        | Expr::WithCapacity {
            value: left,
            capacity: right,
        }
        | Expr::ChunkAt {
            index: left,
            zone: right,
        } => {
            expr_reads(left, stores, out);
            expr_reads(right, stores, out);
        }
        Expr::Not { operand }
        | Expr::Copy { expr: operand }
        | Expr::Give { value: operand }
        | Expr::Length {
            collection: operand,
        }
        | Expr::ManifestOf { zone: operand }
        | Expr::OptionSome { value: operand } => expr_reads(operand, stores, out),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            expr_reads(collection, stores, out);
            expr_reads(start, stores, out);
            expr_reads(end, stores, out);
        }
        Expr::Call { args, .. } => args.iter().for_each(|a| expr_reads(a, stores, out)),
        Expr::CallExpr { callee, args } => {
            expr_reads(callee, stores, out);
            args.iter().for_each(|a| expr_reads(a, stores, out));
        }
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().for_each(|i| expr_reads(i, stores, out))
        }
        Expr::New { init_fields, .. } => init_fields
            .iter()
            .for_each(|(_, v)| expr_reads(v, stores, out)),
        Expr::NewVariant { fields, .. } => {
            fields.iter().for_each(|(_, v)| expr_reads(v, stores, out))
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => expr_reads(e, stores, out),
            ClosureBody::Block(block) => {
                for stmt in block.iter() {
                    stmt_reads(stmt, stores, out);
                    nested_block_reads(stmt, stores, out);
                }
            }
        },
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    expr_reads(value, stores, out);
                }
            }
        }
        Expr::View(_) => stores.iter().for_each(|s| note(*s, out)),
        Expr::Dom(q) => {
            if let Some(sel) = q.selector() {
                expr_reads(sel, stores, out);
            }
        }
    }
}

/// The reads in every block `stmt` holds — for a closure body, where no join can be inserted
/// inside and the statement that creates the closure joins for all of it.
fn nested_block_reads(stmt: &Stmt<'_>, stores: &[Symbol], out: &mut Vec<Symbol>) {
    let block = |b: &[Stmt<'_>], out: &mut Vec<Symbol>| {
        for s in b {
            stmt_reads(s, stores, out);
            nested_block_reads(s, stores, out);
        }
    };
    match stmt {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            block(then_block, out);
            if let Some(b) = else_block {
                block(b, out);
            }
        }
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::ExpectFail { body, .. }
        | Stmt::During { body, .. }
        | Stmt::ForAll { body, .. }
        | Stmt::Splice { body }
        | Stmt::Reversibly { body }
        | Stmt::Zone { body, .. } => block(body, out),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => block(tasks, out),
        Stmt::Inspect { arms, .. } => arms.iter().for_each(|arm| block(arm.body, out)),
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => {
                        block(body, out)
                    }
                }
            }
        }
        _ => {}
    }
}
