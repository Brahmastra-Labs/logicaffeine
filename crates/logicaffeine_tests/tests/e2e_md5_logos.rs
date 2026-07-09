//! MD5 compress written IN LOGOS (Word32 ops + the cross-tier word bitwise builtins) over the "abc"
//! message block, on the tree-walker AND bytecode VM. Proves the hash ALGORITHM runs in-language with
//! no native kernel; output equals Python hashlib md5("abc") raw state, so the arithmetic is exact.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn md5_compress_written_in_logos_matches_reference() {
    let src = r####"## To md5Constants -> Seq of Word32:
    Let mutable kk be a new Seq of Word32.
    Push word32(3614090360) to kk.
    Push word32(3905402710) to kk.
    Push word32(606105819) to kk.
    Push word32(3250441966) to kk.
    Push word32(4118548399) to kk.
    Push word32(1200080426) to kk.
    Push word32(2821735955) to kk.
    Push word32(4249261313) to kk.
    Push word32(1770035416) to kk.
    Push word32(2336552879) to kk.
    Push word32(4294925233) to kk.
    Push word32(2304563134) to kk.
    Push word32(1804603682) to kk.
    Push word32(4254626195) to kk.
    Push word32(2792965006) to kk.
    Push word32(1236535329) to kk.
    Push word32(4129170786) to kk.
    Push word32(3225465664) to kk.
    Push word32(643717713) to kk.
    Push word32(3921069994) to kk.
    Push word32(3593408605) to kk.
    Push word32(38016083) to kk.
    Push word32(3634488961) to kk.
    Push word32(3889429448) to kk.
    Push word32(568446438) to kk.
    Push word32(3275163606) to kk.
    Push word32(4107603335) to kk.
    Push word32(1163531501) to kk.
    Push word32(2850285829) to kk.
    Push word32(4243563512) to kk.
    Push word32(1735328473) to kk.
    Push word32(2368359562) to kk.
    Push word32(4294588738) to kk.
    Push word32(2272392833) to kk.
    Push word32(1839030562) to kk.
    Push word32(4259657740) to kk.
    Push word32(2763975236) to kk.
    Push word32(1272893353) to kk.
    Push word32(4139469664) to kk.
    Push word32(3200236656) to kk.
    Push word32(681279174) to kk.
    Push word32(3936430074) to kk.
    Push word32(3572445317) to kk.
    Push word32(76029189) to kk.
    Push word32(3654602809) to kk.
    Push word32(3873151461) to kk.
    Push word32(530742520) to kk.
    Push word32(3299628645) to kk.
    Push word32(4096336452) to kk.
    Push word32(1126891415) to kk.
    Push word32(2878612391) to kk.
    Push word32(4237533241) to kk.
    Push word32(1700485571) to kk.
    Push word32(2399980690) to kk.
    Push word32(4293915773) to kk.
    Push word32(2240044497) to kk.
    Push word32(1873313359) to kk.
    Push word32(4264355552) to kk.
    Push word32(2734768916) to kk.
    Push word32(1309151649) to kk.
    Push word32(4149444226) to kk.
    Push word32(3174756917) to kk.
    Push word32(718787259) to kk.
    Push word32(3951481745) to kk.
    Return kk.

## To md5Compress (state: Seq of Word32) and (m: Seq of Word32) and (kk: Seq of Word32) -> Seq of Word32:
    Let mutable a be item 1 of state.
    Let mutable b be item 2 of state.
    Let mutable c be item 3 of state.
    Let mutable d be item 4 of state.
    Let mutable r1 be a new Seq of Int.
    Push 7 to r1.
    Push 12 to r1.
    Push 17 to r1.
    Push 22 to r1.
    Let mutable r2 be a new Seq of Int.
    Push 5 to r2.
    Push 9 to r2.
    Push 14 to r2.
    Push 20 to r2.
    Let mutable r3 be a new Seq of Int.
    Push 4 to r3.
    Push 11 to r3.
    Push 16 to r3.
    Push 23 to r3.
    Let mutable r4 be a new Seq of Int.
    Push 6 to r4.
    Push 10 to r4.
    Push 15 to r4.
    Push 21 to r4.
    Repeat for i from 0 to 15:
        Let f be word_or(word_and(b, c), word_and(word_not(b), d)).
        Let g be i.
        Let tmp be a + f + (item (i + 1) of kk) + (item (g + 1) of m).
        Set a to d.
        Set d to c.
        Set c to b.
        Set b to b + rotl(tmp, item ((i % 4) + 1) of r1).
    Repeat for i from 16 to 31:
        Let f be word_or(word_and(d, b), word_and(word_not(d), c)).
        Let g be (5 * i + 1) % 16.
        Let tmp be a + f + (item (i + 1) of kk) + (item (g + 1) of m).
        Set a to d.
        Set d to c.
        Set c to b.
        Set b to b + rotl(tmp, item ((i % 4) + 1) of r2).
    Repeat for i from 32 to 47:
        Let f be b xor c xor d.
        Let g be (3 * i + 5) % 16.
        Let tmp be a + f + (item (i + 1) of kk) + (item (g + 1) of m).
        Set a to d.
        Set d to c.
        Set c to b.
        Set b to b + rotl(tmp, item ((i % 4) + 1) of r3).
    Repeat for i from 48 to 63:
        Let f be c xor word_or(b, word_not(d)).
        Let g be (7 * i) % 16.
        Let tmp be a + f + (item (i + 1) of kk) + (item (g + 1) of m).
        Set a to d.
        Set d to c.
        Set c to b.
        Set b to b + rotl(tmp, item ((i % 4) + 1) of r4).
    Let mutable out be a new Seq of Word32.
    Push (item 1 of state) + a to out.
    Push (item 2 of state) + b to out.
    Push (item 3 of state) + c to out.
    Push (item 4 of state) + d to out.
    Return out.

## Main
    Let mutable state be a new Seq of Word32.
    Push word32(1732584193) to state.
    Push word32(4023233417) to state.
    Push word32(2562383102) to state.
    Push word32(271733878) to state.
    Let mutable m be a new Seq of Word32.
    Push word32(2153996897) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(0) to m.
    Push word32(24) to m.
    Push word32(0) to m.
    Let kk be md5Constants().
    Let result be md5Compress(state, m, kk).
    Repeat for w in result:
        Show intOfWord32(w)."####;
    common::assert_interpreter_output(src, "2555380112\n2958021180\n2101319382\n1920983336");
    // AOT: the Logos MD5 compiles to native Rust (Word32 → u32 wrapping ops + word_and/or/not) and
    // produces the identical digest — proving the algorithm is Logos AND compiles to native code.
    common::assert_output_lines(src, &["2555380112", "2958021180", "2101319382", "1920983336"]);
}

