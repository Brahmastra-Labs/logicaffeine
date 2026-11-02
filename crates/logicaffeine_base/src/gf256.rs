//! GF(2^8) — the finite field with 256 elements, the shared substrate for erasure
//! coding, multivariate cryptography (Oil-and-Vinegar / Rainbow), and the algebraic
//! (XL) cryptanalysis engine.
//!
//! Elements are bytes; addition is XOR (characteristic two, so `add == sub`), and
//! multiplication is polynomial multiplication modulo the primitive polynomial
//! `x^8 + x^4 + x^3 + x^2 + 1` (`0x11d`) — the reduction polynomial used by the
//! Reed-Solomon / QR-code lineage. (Note this is NOT the AES/Rijndael polynomial
//! `0x11b`; a scheme that fixes a different field representation needs its own.)
//!
//! `mul`/`div` run off a log/exp table generated once from the generator `2`, whose
//! multiplicative order is the full 255, so `exp`/`log` are true inverses across the
//! whole nonzero field. On top of the field this module provides the linear algebra
//! that multivariate signing and kernel extraction need: matrix `invert`, `matmul`,
//! `matvec`, `rank`, `solve` (a particular solution of `A x = b`), and `nullspace`
//! (a basis of `{x : A x = 0}`) — all exact, no floating point.

use std::sync::OnceLock;

/// GF(2^8) log/exp tables under the primitive polynomial `0x11d`. `exp` is doubled to
/// 512 entries so a `log[a] + log[b]` sum never indexes out of range.
struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
}

fn gf() -> &'static Gf {
    static GF: OnceLock<Gf> = OnceLock::new();
    GF.get_or_init(|| {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11d;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        Gf { exp, log }
    })
}

/// Field addition. In characteristic two this is XOR, and it is its own inverse, so
/// subtraction is the same operation.
#[inline]
pub fn add(a: u8, b: u8) -> u8 {
    a ^ b
}

/// Field subtraction — identical to [`add`] in characteristic two. Named for call-site
/// clarity where the algebra reads as a subtraction.
#[inline]
pub fn sub(a: u8, b: u8) -> u8 {
    a ^ b
}

/// Field multiplication modulo `0x11d`. `0` absorbs.
#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let g = gf();
    g.exp[g.log[a as usize] as usize + g.log[b as usize] as usize]
}

/// Field division `a / b`. `0` has no inverse, so a zero divisor yields `0` — a safe, defined
/// sentinel (NOT a meaningful quotient) that holds identically in debug and release; callers must
/// still treat `b == 0` as a precondition violation, never as a value.
#[inline]
pub fn div(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let g = gf();
    g.exp[g.log[a as usize] as usize + 255 - g.log[b as usize] as usize]
}

/// Multiplicative inverse. `inv(a) = a^254` since the group has order 255; `inv(0) = 0` — `0`
/// has no inverse, so this is a safe sentinel rather than the silent garbage a raw table lookup
/// would return.
#[inline]
pub fn inv(a: u8) -> u8 {
    div(1, a)
}

/// `base^exp` in GF(2^8). `base^0 == 1` for every base (including `0`).
pub fn pow(base: u8, exp: usize) -> u8 {
    let mut r = 1u8;
    for _ in 0..exp {
        r = mul(r, base);
    }
    r
}

/// Invert a `k×k` matrix over GF(2^8) by Gauss-Jordan elimination; `None` if singular.
pub fn invert(m: &[Vec<u8>]) -> Option<Vec<Vec<u8>>> {
    let k = m.len();
    let mut a: Vec<Vec<u8>> = m
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = row.clone();
            r.extend((0..k).map(|j| if i == j { 1 } else { 0 }));
            r
        })
        .collect();
    for col in 0..k {
        let mut piv = col;
        while piv < k && a[piv][col] == 0 {
            piv += 1;
        }
        if piv == k {
            return None; // singular
        }
        a.swap(col, piv);
        let inv_p = div(1, a[col][col]);
        for j in 0..2 * k {
            a[col][j] = mul(a[col][j], inv_p);
        }
        for row in 0..k {
            if row == col {
                continue;
            }
            let f = a[row][col];
            if f != 0 {
                for j in 0..2 * k {
                    a[row][j] ^= mul(f, a[col][j]);
                }
            }
        }
    }
    Some(a.iter().map(|r| r[k..2 * k].to_vec()).collect())
}

/// Multiply an `n×k` matrix by a `k×c` matrix over GF(2^8).
pub fn matmul(a: &[Vec<u8>], b: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let inner = b.len();
    let cols = b[0].len();
    a.iter()
        .map(|row| {
            (0..cols)
                .map(|j| {
                    let mut acc = 0u8;
                    for t in 0..inner {
                        acc ^= mul(row[t], b[t][j]);
                    }
                    acc
                })
                .collect()
        })
        .collect()
}

/// Multiply an `m×n` matrix by an `n`-vector over GF(2^8), giving an `m`-vector.
pub fn matvec(a: &[Vec<u8>], x: &[u8]) -> Vec<u8> {
    a.iter()
        .map(|row| {
            let mut acc = 0u8;
            for (j, &c) in row.iter().enumerate() {
                acc ^= mul(c, x[j]);
            }
            acc
        })
        .collect()
}

/// Reduce `rows` to reduced row-echelon form in place, pivoting only within the first
/// `pivot_limit` columns (the variable columns; any trailing columns — e.g. an augmented
/// right-hand side — are eliminated along but never pivoted on). Returns the pivot column
/// of each pivot row, in row order.
fn rref(rows: &mut [Vec<u8>], pivot_limit: usize) -> Vec<usize> {
    let mut pivots = Vec::new();
    if rows.is_empty() {
        return pivots;
    }
    let width = rows[0].len();
    let mut r = 0usize;
    for c in 0..pivot_limit {
        let mut piv = None;
        for i in r..rows.len() {
            if rows[i][c] != 0 {
                piv = Some(i);
                break;
            }
        }
        let Some(pi) = piv else { continue };
        rows.swap(r, pi);
        let inv_p = inv(rows[r][c]);
        for j in 0..width {
            rows[r][j] = mul(rows[r][j], inv_p);
        }
        for i in 0..rows.len() {
            if i != r && rows[i][c] != 0 {
                let f = rows[i][c];
                for j in 0..width {
                    rows[i][j] ^= mul(f, rows[r][j]);
                }
            }
        }
        pivots.push(c);
        r += 1;
        if r == rows.len() {
            break;
        }
    }
    pivots
}

/// Rank of an `m×n` matrix over GF(2^8).
pub fn rank(a: &[Vec<u8>]) -> usize {
    let mut rows: Vec<Vec<u8>> = a.to_vec();
    let ncols = rows.first().map_or(0, |r| r.len());
    rref(&mut rows, ncols).len()
}

/// A particular solution of `A x = b` over GF(2^8), or `None` if the system is
/// inconsistent. `a` is `m×n` (row per equation), `b` has length `m`; the returned `x`
/// has length `n` with free variables set to zero. When the caller knows the system is
/// square and full-rank (Oil-and-Vinegar signing after fixing the vinegar variables),
/// the solution is unique.
pub fn solve(a: &[Vec<u8>], b: &[u8]) -> Option<Vec<u8>> {
    let m = a.len();
    if m == 0 {
        return Some(Vec::new());
    }
    let n = a[0].len();
    let mut rows: Vec<Vec<u8>> = a
        .iter()
        .zip(b.iter())
        .map(|(row, &rhs)| {
            let mut r = row.clone();
            r.push(rhs);
            r
        })
        .collect();
    let pivots = rref(&mut rows, n);
    // Consistency: a row that is all-zero across the n variable columns but has a
    // non-zero right-hand side is the equation 0 = c, a contradiction.
    for row in &rows {
        if row[..n].iter().all(|&v| v == 0) && row[n] != 0 {
            return None;
        }
    }
    let mut x = vec![0u8; n];
    for (row_idx, &col) in pivots.iter().enumerate() {
        x[col] = rows[row_idx][n];
    }
    Some(x)
}

/// A basis of the null space `{x : A x = 0}` over GF(2^8). The basis has `n - rank(A)`
/// vectors of length `n`; together they span every solution of the homogeneous system.
pub fn nullspace(a: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let n = a.first().map_or(0, |r| r.len());
    if n == 0 {
        return Vec::new();
    }
    let mut rows: Vec<Vec<u8>> = a.to_vec();
    let pivots = rref(&mut rows, n);
    let is_pivot = {
        let mut v = vec![false; n];
        for &c in &pivots {
            v[c] = true;
        }
        v
    };
    let mut basis = Vec::new();
    for free in 0..n {
        if is_pivot[free] {
            continue;
        }
        let mut v = vec![0u8; n];
        v[free] = 1;
        for (row_idx, &pcol) in pivots.iter().enumerate() {
            // Pivot equation: x[pcol] + Σ_{free j} rows[row_idx][j]·x[j] = 0.
            // With this free var = 1 and all other frees = 0, x[pcol] = rows[row_idx][free].
            v[pcol] = rows[row_idx][free];
        }
        basis.push(v);
    }
    basis
}

/// A 256-entry lookup table for multiplication by a fixed constant `c`: `table[b] = c · b` in
/// GF(2^8). Precompute once per coefficient; a whole region then multiplies by one table lookup
/// per byte — the SIMD-friendly hot path of Reed-Solomon coding.
pub fn mul_table(c: u8) -> [u8; 256] {
    let mut t = [0u8; 256];
    for (b, slot) in t.iter_mut().enumerate() {
        *slot = mul(c, b as u8);
    }
    t
}

/// Region multiply-accumulate over GF(2^8): `dst[i] ^= c · src[i]` for every byte — the inner loop
/// of Reed-Solomon encode/reconstruct. `src` and `dst` must be the same length. The body is a
/// per-`c` table lookup XORed into the destination, a shape the compiler AUTO-VECTORIZES to SSE/AVX
/// on native and to wasm `v128` under `simd128` — the SIMD speedup for browser FEC, in safe Rust
/// with no hand-written intrinsics to silently miscompile (its result is pinned byte-exact to the
/// scalar field by the tests).
pub fn mul_add_region(c: u8, src: &[u8], dst: &mut [u8]) {
    debug_assert_eq!(
        src.len(),
        dst.len(),
        "region multiply operands must be equal length"
    );
    let table = mul_table(c);
    for (d, &s) in dst.iter_mut().zip(src.iter()) {
        *d ^= table[s as usize];
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/gf256/tests.rs"]
mod tests;
