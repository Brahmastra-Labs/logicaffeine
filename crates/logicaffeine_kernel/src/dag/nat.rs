//! Lean's natural numbers on top of [`logicaffeine_base::BigInt`].
//!
//! `BigInt` supplies the arithmetic; this wrapper supplies `Nat`'s semantics — values never go
//! negative, subtraction truncates at zero, and `n / 0 = 0`, `n % 0 = n` — so the kernel's
//! literal extension computes exactly what Lean's `Nat` definitions compute.

use logicaffeine_base::BigInt;

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Nat(BigInt);

impl Nat {
    pub fn zero() -> Nat {
        Nat(BigInt::zero())
    }

    pub fn from_u64(v: u64) -> Nat {
        Nat(BigInt::from_u64(v))
    }

    /// A non-negative integer as a natural number (a LOGOS `Nat` literal holds one).
    pub fn from_bigint(v: &BigInt) -> Nat {
        debug_assert!(!v.is_negative(), "a natural number is not negative");
        Nat(v.clone())
    }

    pub fn to_bigint(&self) -> BigInt {
        self.0.clone()
    }

    pub fn parse_decimal(s: &str) -> Option<Nat> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        BigInt::parse_decimal(s).map(Nat)
    }

    pub fn to_decimal(&self) -> String {
        self.0.to_string()
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    pub fn to_u64(&self) -> Option<u64> {
        self.0.to_u64()
    }

    pub fn succ(&self) -> Nat {
        Nat(self.0.add(&BigInt::from_u64(1)))
    }

    pub fn add(&self, o: &Nat) -> Nat {
        Nat(self.0.add(&o.0))
    }

    /// Truncated subtraction.
    pub fn sub(&self, o: &Nat) -> Nat {
        if self.0 <= o.0 {
            Nat::zero()
        } else {
            Nat(self.0.sub(&o.0))
        }
    }

    pub fn mul(&self, o: &Nat) -> Nat {
        Nat(self.0.mul(&o.0))
    }

    pub fn div(&self, o: &Nat) -> Nat {
        match self.0.div_rem(&o.0) {
            Some((q, _)) => Nat(q),
            None => Nat::zero(),
        }
    }

    pub fn rem(&self, o: &Nat) -> Nat {
        match self.0.div_rem(&o.0) {
            Some((_, r)) => Nat(r),
            None => self.clone(),
        }
    }

    pub fn gcd(&self, o: &Nat) -> Nat {
        Nat(self.0.gcd(&o.0))
    }

    /// `self ^ exp`; the caller bounds `exp` (Lean's kernel reduces `Nat.pow` only for
    /// exponents up to 2²⁴).
    pub fn pow(&self, exp: u32) -> Nat {
        Nat(self.0.pow(exp))
    }

    pub fn land(&self, o: &Nat) -> Nat {
        Nat(self
            .0
            .bitand_natural(&o.0)
            .expect("naturals are non-negative"))
    }

    pub fn lor(&self, o: &Nat) -> Nat {
        Nat(self
            .0
            .bitor_natural(&o.0)
            .expect("naturals are non-negative"))
    }

    pub fn xor(&self, o: &Nat) -> Nat {
        Nat(self
            .0
            .bitxor_natural(&o.0)
            .expect("naturals are non-negative"))
    }

    pub fn shl(&self, bits: u64) -> Nat {
        Nat(self.0.shl_bits(bits))
    }

    pub fn shr(&self, bits: u64) -> Nat {
        Nat(self.0.shr_bits(bits).expect("naturals are non-negative"))
    }

    pub fn bit_len(&self) -> u64 {
        self.0.bit_len()
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/nat/tests.rs"]
mod tests;
