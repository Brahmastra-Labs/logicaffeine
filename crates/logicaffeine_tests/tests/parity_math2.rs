#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
/// 10 raised to n. go: math.Pow10
#[test]
fn parity_math_pow10() {
    assert_all_tiers("## Main\nShow pow10(3).\nShow pow10(6).\n", "1000\n1000000");
}
/// Permutations n!/(n-k)!. py: math.perm
#[test]
fn parity_math_perm() {
    assert_all_tiers("## Main\nShow perm(5, 2).\nShow perm(6, 3).\n", "20\n120");
}
/// Positive difference max(x-y, 0). go: math.Dim
#[test]
fn parity_math_dim() {
    assert_all_tiers(
        "## Main\nShow dimReal(5.0, 3.0).\nShow dimReal(3.0, 5.0).\n",
        "2\n0",
    );
}
/// Whether a real is negative. go: math.Signbit
#[test]
fn parity_math_signbit() {
    assert_all_tiers(
        "## Main\nShow signbitReal(0.0 - 3.0).\nShow signbitReal(3.0).\n",
        "true\nfalse",
    );
}
/// Fused multiply-add x*y+z. go: math.FMA · c: math.h.fma
#[test]
fn parity_math_fma() {
    assert_all_tiers(
        "## Main\nShow fma(2.0, 3.0, 4.0).\nShow fma(0.0 - 1.0, 5.0, 2.0).\n",
        "10\n-3",
    );
}
/// Gauss error function (A&S 7.1.26, ×100 to dodge float display). go: math.Erf · py: math.erf/statistics.erf · c: math.h.erf
#[test]
fn parity_math_erf() {
    assert_all_tiers(
        "## Main\nShow floor(erf(1.0) * 100.0).\nShow floor(erf(0.0) * 100.0).\n",
        "84\n0",
    );
}
/// Complementary error function 1-erf. go: math.Erfc · py: math.erfc · c: math.h.erfc
#[test]
fn parity_math_erfc() {
    assert_all_tiers("## Main\nShow floor(erfc(1.0) * 100.0).\n", "15");
}
/// IEEE remainder (round-half-to-even). go: math.Remainder · py: math.remainder · c: math.h.remainder
#[test]
fn parity_math_remainder() {
    assert_all_tiers("## Main\nShow remainderReal(10.0, 3.0).\nShow remainderReal(10.0, 4.0).\nShow remainderReal(6.0, 4.0).\n", "1\n2\n-2");
}
/// Round to nearest even integer (banker's rounding), Real result. go: math.RoundToEven
#[test]
fn parity_math_round_even() {
    assert_all_tiers(
        "## Main\nShow roundEvenReal(2.5).\nShow roundEvenReal(3.5).\nShow roundEvenReal(2.4).\n",
        "2\n4\n2",
    );
}
/// Absolute value of a real (×10 to dodge float display). py: statistics.fabs
#[test]
fn parity_math_fabs() {
    assert_all_tiers("## Main\nShow floor(absReal(0.0 - 3.5) * 10.0 + 0.5).\nShow floor(absReal(2.5) * 10.0 + 0.5).\n", "35\n25");
}
/// Relative-tolerance closeness (default rel_tol 1e-9, abs_tol 0). py: math.isclose
#[test]
fn parity_math_isclose() {
    assert_all_tiers("## Main\nShow isClose(1.0, 1.0000000001).\nShow isClose(1.0, 1.001).\nShow isClose(100.0, 100.00000001).\n", "true\nfalse\ntrue");
}
/// Product of the integers in [a, b] (empty range is 1). go: math/big.Int.MulRange
/// Doubles as the cross-tier arbitrary-precision probe: mulRange(1, 25) = 25! overflows i64.
#[test]
fn parity_math_mulrange() {
    assert_all_tiers(
        "## Main\nShow mulRange(3, 6).\nShow mulRange(5, 4).\nShow mulRange(1, 25).\n",
        "360\n1\n15511210043330985984000000",
    );
}
/// Jacobi symbol (a|n) for odd n>0 (generalizes Legendre; n need not be prime). go: math/big.Jacobi
#[test]
fn parity_math_jacobi() {
    assert_all_tiers("## Main\nShow jacobi(1, 3).\nShow jacobi(2, 3).\nShow jacobi(2, 7).\nShow jacobi(5, 21).\nShow jacobi(1001, 9907).\nShow jacobi(19, 45).\n", "1\n-1\n1\n1\n-1\n1");
}
/// Deterministic Miller-Rabin primality (strong witnesses through 37). go: math/big.Int.ProbablyPrime
#[test]
fn parity_math_probable_prime() {
    assert_all_tiers("## Main\nShow isProbablePrime(97).\nShow isProbablePrime(100).\nShow isProbablePrime(561).\nShow isProbablePrime(7919).\nShow isProbablePrime(104729).\nShow isProbablePrime(1).\n", "true\nfalse\nfalse\ntrue\ntrue\nfalse");
}
/// Modular square root via Tonelli-Shanks; asserted through r*r ≡ a (mod p) to dodge the ± ambiguity. go: math/big.Int.ModSqrt
#[test]
fn parity_math_modsqrt() {
    assert_all_tiers("## Main\nShow (modSqrt(2, 7) * modSqrt(2, 7)) mod 7.\nShow (modSqrt(2, 113) * modSqrt(2, 113)) mod 113.\nShow (modSqrt(5, 41) * modSqrt(5, 41)) mod 41.\n", "2\n2\n5");
}
/// Split a real into its truncated-toward-zero integer part and fractional remainder. go: math.Modf · c: math.h.modf
#[test]
fn parity_math_modf() {
    assert_all_tiers("## Main\nShow floor(modfInt(3.75)).\nShow floor(modfFrac(3.75) * 100.0 + 0.5).\nShow floor(modfInt(0.0 - 2.5)).\nShow floor((0.0 - modfFrac(0.0 - 2.5)) * 100.0 + 0.5).\n", "3\n75\n-2\n50");
}
/// The binary exponent (⌊log2|x|⌋) as an integer, computed exactly. go: math.Ilogb · c: math.h.ilogb
#[test]
fn parity_math_ilogb() {
    assert_all_tiers("## Main\nShow ilogb(8.0).\nShow ilogb(1.0).\nShow ilogb(0.5).\nShow ilogb(1023.0).\nShow ilogb(1024.0).\n", "3\n0\n-1\n9\n10");
}
/// The binary exponent as a real. go: math.Logb · c: math.h.logb
#[test]
fn parity_math_logb() {
    assert_all_tiers(
        "## Main\nShow floor(logb(8.0)).\nShow floor(logb(1024.0)).\nShow floor(logb(0.5)).\n",
        "3\n10\n-1",
    );
}
/// Decompose x into a mantissa in [0.5, 1) and a binary exponent (x = frac·2^exp). go: math.Frexp · c: math.h.frexp
#[test]
fn parity_math_frexp() {
    assert_all_tiers("## Main\nShow floor(frexpFrac(8.0) * 1000.0 + 0.5).\nShow frexpExp(8.0).\nShow floor(frexpFrac(3.0) * 1000.0 + 0.5).\nShow frexpExp(3.0).\nShow frexpExp(0.5).\n", "500\n4\n750\n2\n0");
}
/// Three-way sign of a-b. go: math/big.Int.Cmp
#[test]
fn parity_math_cmp() {
    assert_all_tiers(
        "## Main\nShow cmpInt(3, 5).\nShow cmpInt(5, 3).\nShow cmpInt(4, 4).\n",
        "-1\n1\n0",
    );
}
/// Three-way sign of |a|-|b|. go: math/big.Int.CmpAbs
#[test]
fn parity_math_cmp_abs() {
    assert_all_tiers("## Main\nShow cmpAbsInt(0 - 5, 3).\nShow cmpAbsInt(0 - 3, 4).\nShow cmpAbsInt(0 - 4, 4).\n", "1\n-1\n0");
}
/// Truncated (toward-zero) integer division — quotient and remainder (remainder has sign of a). go: math/big.Int.QuoRem · c: stdlib.h.div
#[test]
fn parity_math_quorem() {
    assert_all_tiers("## Main\nShow quoRemQ(7, 3).\nShow quoRemR(7, 3).\nShow quoRemQ(0 - 7, 3).\nShow quoRemR(0 - 7, 3).\nShow quoRemQ(7, 0 - 3).\nShow quoRemR(7, 0 - 3).\nShow quoRemQ(0 - 7, 0 - 3).\nShow quoRemR(0 - 7, 0 - 3).\n", "2\n1\n-2\n-1\n-2\n1\n2\n-1");
}
/// Euclidean integer division — quotient and remainder (remainder always in [0, |b|)). go: math/big.Int.DivMod
#[test]
fn parity_math_divmod() {
    assert_all_tiers("## Main\nShow divModQ(7, 3).\nShow divModM(7, 3).\nShow divModQ(0 - 7, 3).\nShow divModM(0 - 7, 3).\nShow divModQ(7, 0 - 3).\nShow divModM(7, 0 - 3).\nShow divModQ(0 - 7, 0 - 3).\nShow divModM(0 - 7, 0 - 3).\n", "2\n1\n-3\n2\n-2\n1\n3\n2");
}
/// The constant pi. py: math.pi  (floor(pi*100)=314 dodges float-format fragility)
#[test]
fn parity_math_pi() {
    assert_all_tiers("## Main\nShow floor(piReal() * 100.0).\n", "314");
}
/// The constant e. py: math.e
#[test]
fn parity_math_e() {
    assert_all_tiers("## Main\nShow floor(eReal() * 100.0).\n", "271");
}
/// The constant tau = 2*pi. py: math.tau / statistics.tau
#[test]
fn parity_math_tau() {
    assert_all_tiers("## Main\nShow floor(tauReal() * 100.0).\n", "628");
}
