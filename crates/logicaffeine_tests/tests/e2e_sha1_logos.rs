//! SHA-1 compress written IN LOGOS over the `Lanes4Word32` SHA-NI lane type. The algorithm is the
//! full 80-round Intel SHA-NI schedule (`sha1rnds4`/`sha1msg1`/`sha1msg2`/`sha1nexte` + lane-wise add
//! and xor) expressed as Logos statements — no native hash kernel. On the tree-walker and bytecode VM
//! the lane ops run the byte-identical software spec (`logicaffeine_base::sha_ops`); the AOT lowers the
//! same source to the `sha1rnds4` hardware instruction sequence. All three tiers must equal the digest
//! Python `hashlib.sha1` produces, so the schedule + the exact ABCD/E lane placement are proven correct.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
const SHA1_LOGOS: &str = r####"## To mkLane4 (a: Word32) and (b: Word32) and (c: Word32) and (d: Word32) -> Lanes4Word32:
    Let s be a new Seq of Word32.
    Push a to s.
    Push b to s.
    Push c to s.
    Push d to s.
    Return lanes4Word32(s).

## To sha1Compress (h: Seq of Word32) and (w: Seq of Word32) -> Seq of Word32:
    Let zero be word32(0).
    Let mutable st be mkLane4(item 4 of h, item 3 of h, item 2 of h, item 1 of h).
    Let mutable e0 be mkLane4(zero, zero, zero, item 5 of h).
    Let abcdSave be st.
    Let e0Save be e0.
    Let mutable m0 be mkLane4(item 4 of w, item 3 of w, item 2 of w, item 1 of w).
    Let mutable m1 be mkLane4(item 8 of w, item 7 of w, item 6 of w, item 5 of w).
    Let mutable m2 be mkLane4(item 12 of w, item 11 of w, item 10 of w, item 9 of w).
    Let mutable m3 be mkLane4(item 16 of w, item 15 of w, item 14 of w, item 13 of w).
    Let mutable e1 be st.
    Set e0 to e0 + m0.
    Set e1 to st.
    Set st to sha1rnds4(st, e0, 0).
    Set e1 to sha1nexte(e1, m1).
    Set e0 to st.
    Set st to sha1rnds4(st, e1, 0).
    Set m0 to sha1msg1(m0, m1).
    Set e0 to sha1nexte(e0, m2).
    Set e1 to st.
    Set st to sha1rnds4(st, e0, 0).
    Set m1 to sha1msg1(m1, m2).
    Set m0 to m0 xor m2.
    Set e1 to sha1nexte(e1, m3).
    Set e0 to st.
    Set m0 to sha1msg2(m0, m3).
    Set st to sha1rnds4(st, e1, 0).
    Set m2 to sha1msg1(m2, m3).
    Set m1 to m1 xor m3.
    Set e0 to sha1nexte(e0, m0).
    Set e1 to st.
    Set m1 to sha1msg2(m1, m0).
    Set st to sha1rnds4(st, e0, 0).
    Set m3 to sha1msg1(m3, m0).
    Set m2 to m2 xor m0.
    Set e1 to sha1nexte(e1, m1).
    Set e0 to st.
    Set m2 to sha1msg2(m2, m1).
    Set st to sha1rnds4(st, e1, 1).
    Set m0 to sha1msg1(m0, m1).
    Set m3 to m3 xor m1.
    Set e0 to sha1nexte(e0, m2).
    Set e1 to st.
    Set m3 to sha1msg2(m3, m2).
    Set st to sha1rnds4(st, e0, 1).
    Set m1 to sha1msg1(m1, m2).
    Set m0 to m0 xor m2.
    Set e1 to sha1nexte(e1, m3).
    Set e0 to st.
    Set m0 to sha1msg2(m0, m3).
    Set st to sha1rnds4(st, e1, 1).
    Set m2 to sha1msg1(m2, m3).
    Set m1 to m1 xor m3.
    Set e0 to sha1nexte(e0, m0).
    Set e1 to st.
    Set m1 to sha1msg2(m1, m0).
    Set st to sha1rnds4(st, e0, 1).
    Set m3 to sha1msg1(m3, m0).
    Set m2 to m2 xor m0.
    Set e1 to sha1nexte(e1, m1).
    Set e0 to st.
    Set m2 to sha1msg2(m2, m1).
    Set st to sha1rnds4(st, e1, 1).
    Set m0 to sha1msg1(m0, m1).
    Set m3 to m3 xor m1.
    Set e0 to sha1nexte(e0, m2).
    Set e1 to st.
    Set m3 to sha1msg2(m3, m2).
    Set st to sha1rnds4(st, e0, 2).
    Set m1 to sha1msg1(m1, m2).
    Set m0 to m0 xor m2.
    Set e1 to sha1nexte(e1, m3).
    Set e0 to st.
    Set m0 to sha1msg2(m0, m3).
    Set st to sha1rnds4(st, e1, 2).
    Set m2 to sha1msg1(m2, m3).
    Set m1 to m1 xor m3.
    Set e0 to sha1nexte(e0, m0).
    Set e1 to st.
    Set m1 to sha1msg2(m1, m0).
    Set st to sha1rnds4(st, e0, 2).
    Set m3 to sha1msg1(m3, m0).
    Set m2 to m2 xor m0.
    Set e1 to sha1nexte(e1, m1).
    Set e0 to st.
    Set m2 to sha1msg2(m2, m1).
    Set st to sha1rnds4(st, e1, 2).
    Set m0 to sha1msg1(m0, m1).
    Set m3 to m3 xor m1.
    Set e0 to sha1nexte(e0, m2).
    Set e1 to st.
    Set m3 to sha1msg2(m3, m2).
    Set st to sha1rnds4(st, e0, 2).
    Set m1 to sha1msg1(m1, m2).
    Set m0 to m0 xor m2.
    Set e1 to sha1nexte(e1, m3).
    Set e0 to st.
    Set m0 to sha1msg2(m0, m3).
    Set st to sha1rnds4(st, e1, 3).
    Set m2 to sha1msg1(m2, m3).
    Set m1 to m1 xor m3.
    Set e0 to sha1nexte(e0, m0).
    Set e1 to st.
    Set m1 to sha1msg2(m1, m0).
    Set st to sha1rnds4(st, e0, 3).
    Set m3 to sha1msg1(m3, m0).
    Set m2 to m2 xor m0.
    Set e1 to sha1nexte(e1, m1).
    Set e0 to st.
    Set m2 to sha1msg2(m2, m1).
    Set st to sha1rnds4(st, e1, 3).
    Set m3 to m3 xor m1.
    Set e0 to sha1nexte(e0, m2).
    Set e1 to st.
    Set m3 to sha1msg2(m3, m2).
    Set st to sha1rnds4(st, e0, 3).
    Set e1 to sha1nexte(e1, m3).
    Set e0 to st.
    Set st to sha1rnds4(st, e1, 3).
    Set e0 to sha1nexte(e0, e0Save).
    Set st to st + abcdSave.
    Let sl be seqOfLanes4W32(st).
    Let el be seqOfLanes4W32(e0).
    Let out be a new Seq of Word32.
    Push item 4 of sl to out.
    Push item 3 of sl to out.
    Push item 2 of sl to out.
    Push item 1 of sl to out.
    Push item 4 of el to out.
    Return out.

## Main
    Let mutable h be a new Seq of Word32.
    Push word32(1732584193) to h.
    Push word32(4023233417) to h.
    Push word32(2562383102) to h.
    Push word32(271733878) to h.
    Push word32(3285377520) to h.
    Let mutable w be a new Seq of Word32.
    Push word32(1633837952) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(0) to w.
    Push word32(24) to w.
    Let result be sha1Compress(h, w).
    Repeat for x in result:
        Show intOfWord32(x)."####;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sha1_compress_written_in_logos_matches_reference() {
    // tree-walker == bytecode VM: the software SHA lane spec (logicaffeine_base::sha_ops).
    common::assert_interpreter_output(
        SHA1_LOGOS,
        "2845392438\n1191608682\n3124634993\n2018558572\n2630932637",
    );
    // AOT: the identical Logos source compiles to the `sha1rnds4` hardware sequence and produces the
    // same digest — SHA-1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d.
    common::assert_output_lines(
        SHA1_LOGOS,
        &[
            "2845392438",
            "1191608682",
            "3124634993",
            "2018558572",
            "2630932637",
        ],
    );
}
