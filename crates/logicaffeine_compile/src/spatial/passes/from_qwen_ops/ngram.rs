//! The n-gram ids: a token window hashed on the integer datapath.

use super::*;

/// The integers a fill holds at `len` elements: a constant, a pattern, or literal writes over either — `None` for data
/// the compiler cannot see.
pub(super) fn fill_ints(fill: &Fill, len: usize) -> Option<Vec<i64>> {
    match fill {
        Fill::Const(c) => Some(vec![*c; len]),
        Fill::Pattern { modulus, shift } => Some((0..len as i64).map(|p| p % modulus + shift).collect()),
        Fill::Overrides { base, sets } => {
            let mut v = fill_ints(base, len)?;
            for &(i, x) in sets {
                *v.get_mut(i)? = x;
            }
            Some(v)
        }
        _ => None,
    }
}

/// `cell` appended to `cells`: its value.
pub(super) fn put(cells: &mut Vec<LoopCell>, cell: LoopCell) -> Src {
    cells.push(cell);
    Src::Val(cells.len() - 1)
}

/// One head's table row for a token, as a scalar body over `places` inputs — `In(k)` the token row `k` places back
/// (`In(0)` the token's own; `0` where no token was): each place's id (`E` where none was, and once an `E` lies between,
/// less one) times its multiplier `m[k]`, the products exclusive-ored, modulo the head's `modulus`, plus `offset` and
/// one. The products, the exclusive-ors and the remainder are KERNEL-6's lane programs as cells
/// ([`crate::spatial::wide_int::LaneProgram::cells`]); the end of document is compares and selects.
pub(super) fn ngram_head_body(places: usize, eos: i64, m: &[i64], modulus: u32, offset: i64) -> ScalarBody {
    let mut cells: Vec<LoopCell> = Vec::new();
    let less_one = |cells: &mut Vec<LoopCell>, x: Src| put(cells, LoopCell::Arith { op: AluOp::Sub, a: x, b: Src::Const(1) });
    // A page's elements past the last token read row 0: taken as row 1, so every lane stays in its 32 bits.
    let own = put(&mut cells, LoopCell::Select { cmp: CmpKind::Eq, ca: Src::In(0), cb: Src::Const(0), t: Src::Const(1), e: Src::In(0) });
    let mut ids = vec![less_one(&mut cells, own)];
    let mut seen: Option<Src> = None;
    for k in 1..places {
        let back = put(&mut cells, LoopCell::Select { cmp: CmpKind::Eq, ca: Src::In(k), cb: Src::Const(0), t: Src::Const(eos), e: Src::In(k) });
        let ends = put(&mut cells, LoopCell::Select { cmp: CmpKind::Eq, ca: back, cb: Src::Const(eos), t: Src::Const(1), e: Src::Const(0) });
        let now = match seen {
            None => ends,
            Some(before) => put(&mut cells, LoopCell::Arith { op: AluOp::Or, a: before, b: ends }),
        };
        seen = Some(now);
        let read = put(&mut cells, LoopCell::Select { cmp: CmpKind::Eq, ca: now, cb: Src::Const(1), t: Src::Const(eos), e: back });
        ids.push(less_one(&mut cells, read));
    }
    let (product, xor, rem) = (lane_program(Op64::Mul), lane_program(Op64::Xor), lane_program(Op64::Rem(modulus)));
    let mut hash: Option<[Src; 2]> = None;
    for (k, &id) in ids.iter().enumerate() {
        let mk = m[k] as u64;
        let lanes = [id, Src::Const(0), Src::Const((mk & 0xFFFF_FFFF) as i64), Src::Const((mk >> 32) as i64)];
        let term = product.cells(&lanes, &mut cells);
        hash = Some(match hash {
            None => term,
            Some(x) => xor.cells(&[x[0], x[1], term[0], term[1]], &mut cells),
        });
    }
    let x = hash.expect("an order of at least one place");
    let r = rem.cells(&[x[0], x[1]], &mut cells);
    let row = put(&mut cells, LoopCell::Arith { op: AluOp::Add, a: r[0], b: Src::Const(offset + 1) });
    ScalarBody { n_in: places, cells, result: row }
}

/// `I = the ngram ids of T after R by M modulo V from O ending at E` on a chip whose integers are 32 bits (QWEN4-3): the
/// hash's 64-bit products, exclusive-ors and remainders as KERNEL-6's lane programs, two lanes a value.
///   · A section a chunk of tokens ([`ngram_rows_section`]), as many as an accumulator page holds the rows of: the
///     chunk's window of the history and the tokens, the token rows each place back read out of it, one scalar stage a
///     head (its order's places, its multipliers, its modulus and its first row written into its cells — the hash's
///     constants are the program's own), the heads joined a token's row at a time. Several chunks' rows are joined in a
///     section of their own, a block of columns an item where they are wider than a page.
///   · The history: its last `N − 1` rows of the history and the tokens together, written back.
///
/// Refused by name where the target has no scalar datapath, or the hash's constants are not the program's.
pub fn from_ngram_ids(spec: &NgramIdsSpec, names: &StageNames, target: &SpatialTarget) -> Result<Vec<SpatialProgram>, SpatialReject> {
    if target.scalar_target().is_none() {
        return Err(SpatialReject::MissingCompute { needed: "a scalar datapath for an n-gram hash", target: target.name });
    }
    let (tokens, n, heads) = (spec.tokens, spec.ngram, spec.heads());
    let constants = |k: usize, len: usize| fill_ints(names.operand(k).1, len);
    let (Some(m), Some(v), Some(o)) = (constants(2, n), constants(3, heads), constants(4, heads)) else {
        return Err(SpatialReject::NotLowered("an n-gram hash whose multipliers, moduli and offsets are not the program's constants"));
    };
    let lane = 1i64 << 31;
    if m.iter().any(|x| *x < 1) || v.iter().any(|x| !(1..=u32::MAX as i64).contains(x)) || o.iter().zip(&v).any(|(o, v)| !(0..lane).contains(&(o + v))) {
        return Err(SpatialReject::NotLowered("an n-gram hash whose multipliers are not positive or whose table rows pass a 32-bit integer"));
    }
    let chunk = (row_block_tiles(target) * target.tile.cols / heads).max(1);
    let chunks: Vec<(usize, usize)> = (0..tokens).step_by(chunk).map(|first| (first, chunk.min(tokens - first))).collect();
    let part = |j: usize| if chunks.len() == 1 { names.out.clone() } else { format!("{}__ngram{j}", names.out) };
    let mut sections = Vec::with_capacity(chunks.len() + 2);
    for (j, &(first, len)) in chunks.iter().enumerate() {
        sections.push(ngram_rows_section(spec, names, (&m, &v, &o), (first, len), &part(j), target)?);
    }
    if chunks.len() > 1 {
        // The chunks' rows end to end, chunk `j`'s at `first·H`: a block of result columns an item past a page.
        let out_n = tokens * heads;
        let mut b = FlatBuild::on(Datapath::Int, target)?;
        if row_needs_blocking(out_n, target) {
            let cols_t = tiles_of(out_n, target);
            let blk = row_block(cols_t, target);
            b.blocks = Some(ColumnBlocks { blk, items: cols_t / blk });
        }
        let p = b.precision;
        let out_buf = b.result_page("rows", out_n, p, target);
        let (mut slots, mut bands) = (Vec::with_capacity(chunks.len()), Vec::with_capacity(chunks.len()));
        for (j, &(first, len)) in chunks.iter().enumerate() {
            let buf = b.input_paged(&part(j), len * heads, &Fill::Unknown, p, 1, target);
            slots.push(Slot { buf, map: at(-((first * heads) as i64)) });
            bands.push(((first + len) * heads, TensorBody::Operand(j)));
        }
        let chain = b.pointwise("ngram_join", out_n, slots, banded(bands), out_buf, target)?;
        sections.push(b.finish_as(&names.out, out_n, chain, out_buf, format!("ngram_ids_join_{tokens}x{heads}"), NumericsClaim::Exact, p, target)?);
    }
    let (t_name, t_fill) = names.operand(0);
    let (r_name, r_fill) = names.operand(1);
    let keep = n - 1;
    let history = {
        let mut b = FlatBuild::on(Datapath::Int, target)?;
        let p = b.precision;
        let tok = b.input_paged(t_name, tokens, t_fill, p, 1, target);
        let hist = b.input_paged(r_name, keep, r_fill, p, 1, target);
        // Entry `j` of the new history is entry `j + T` of the history and the tokens together.
        let out_t = tiles_of(keep, target);
        let out_buf = b.t.buffer("history", out_t, p);
        let slots = vec![Slot { buf: hist, map: at(tokens as i64) }, Slot { buf: tok, map: at(tokens as i64 - keep as i64) }];
        let body = match keep.saturating_sub(tokens) {
            0 => TensorBody::Operand(1),
            from_history => banded(vec![(from_history, TensorBody::Operand(0)), (keep, TensorBody::Operand(1))]),
        };
        let s = b.compute("ngram_history", flat_space(keep), slots, ident_map(wide(out_t, target)), body, out_buf);
        b.finish_fanned(r_name, keep, vec![s], out_buf, format!("ngram_ids_history_{keep}"), NumericsClaim::Exact, p, target)?
    };
    sections.push(history);
    Ok(sections)
}

/// The table rows of tokens `first .. first + len` of an n-gram hash, as `out`: the chunk's window of the history and the
/// tokens end to end, rows `first .. first + N − 1 + len` of them (`ngram_window`), the chunk's token rows each place back
/// read out of it (`ngram_back{k}`), one scalar stage a head ([`ngram_head_body`]), and the heads joined a token's row at
/// a time. Every load and scratch a page deep: the section computes each once.
pub(super) fn ngram_rows_section(
    spec: &NgramIdsSpec,
    names: &StageNames,
    (m, v, o): (&[i64], &[i64], &[i64]),
    (first, len): (usize, usize),
    out: &str,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (tokens, n, per, heads) = (spec.tokens, spec.ngram, spec.per_order, spec.heads());
    let keep = n - 1;
    let (t_name, t_fill) = names.operand(0);
    let (r_name, r_fill) = names.operand(1);
    let mut b = FlatBuild::on(Datapath::Int, target)?;
    let p = b.precision;
    let tok = b.input_paged(t_name, tokens, t_fill, p, 1, target);
    let hist = b.input_paged(r_name, keep, r_fill, p, 1, target);
    // Entry `e` of the window is row `first + e` of the history and the tokens together.
    let span = keep + len;
    let all_t = tiles_of(span, target);
    let all = b.t.scratch_deep("ngram_window", 1, all_t, p);
    let slots = vec![Slot { buf: hist, map: at(first as i64) }, Slot { buf: tok, map: at(first as i64 - keep as i64) }];
    let body = match keep.saturating_sub(first) {
        0 => TensorBody::Operand(1),
        from_history => banded(vec![(from_history, TensorBody::Operand(0)), (span, TensorBody::Operand(1))]),
    };
    let mut chain = vec![b.compute("ngram_window", flat_space(span), slots, ident_map(wide(all_t, target)), body, all)];
    let (t_t, w_t) = (tiles_of(len, target), wide(tiles_of(len, target), target));
    let mut back = Vec::with_capacity(n);
    for k in 0..n {
        // The chunk's token `i`'s place `k` back: entry `N − 1 + i − k` of the window.
        let buf = b.t.scratch_deep(&format!("ngram_back{k}"), 1, t_t, p);
        let slots = vec![Slot { buf: all, map: at((keep - k) as i64) }];
        chain.push(b.compute(&format!("ngram_back{k}"), flat_space(len), slots, ident_map(w_t), TensorBody::Operand(0), buf));
        back.push(buf);
    }
    let mut head_rows = Vec::with_capacity(heads);
    for h in 0..heads {
        let places = 2 + h / per;
        let body = ngram_head_body(places, spec.eos, &m[..places], v[h] as u32, o[h]);
        let buf = b.t.scratch_deep(&format!("ngram_head{h}"), 1, t_t, p);
        chain.push(b.t.stage(&format!("ngram_head{h}"), StageKind::Compute(ComputeOp::Scalar(body)), back[..places].to_vec(), vec![buf]));
        head_rows.push(buf);
    }
    // Token `i`'s row of the chunk's result: head `h` at `i·H + h`, a band each reading the head's row at the token.
    let out_n = len * heads;
    let out_t = tiles_of(out_n, target);
    let out_buf = b.t.buffer("rows", out_t, p);
    let (mut slots, mut bands) = (Vec::with_capacity(out_n), Vec::with_capacity(out_n));
    for i in 0..len {
        for (h, &buf) in head_rows.iter().enumerate() {
            slots.push(Slot { buf, map: fixed(i as i64) });
            bands.push((i * heads + h + 1, TensorBody::Operand(slots.len() - 1)));
        }
    }
    chain.push(b.compute("ngram_rows", flat_space(out_n), slots, ident_map(wide(out_t, target)), banded(bands), out_buf));
    b.finish_fanned(out, out_n, chain, out_buf, format!("ngram_ids_rows_{first}+{len}x{heads}"), NumericsClaim::Exact, p, target)
}
