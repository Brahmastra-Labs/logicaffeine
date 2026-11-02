//! The curated regions the accelerator suites share: scalar functions of every shape the region lowering
//! takes, the argument vectors each is called with, and the program that shows one call on the VM.
#![allow(dead_code)]

use logicaffeine_compile::accel::cells::CellOp;
use logicaffeine_compile::accel::region::{discover, Discovered, Halt, Kind, RegionCfg};
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, Divergence, FuncUnit};
use logicaffeine_compile::compile::{compile_to_program_optimized_named, vm_outcome};
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::arith;

/// An argument, spelled so the VM's parser reads exactly this value.
#[derive(Clone, Copy, Debug)]
pub enum Arg {
    I(i64),
    F(f64),
    B(bool),
    /// A NaN, made at run time by `sqrt` of a negative (no literal spells one).
    Nan,
    /// An infinity of this sign, made at run time by a float power that overflows.
    Inf(bool),
}

impl Arg {
    pub fn word(self) -> i64 {
        match self {
            Arg::I(n) => n,
            Arg::F(f) => f.to_bits() as i64,
            Arg::B(b) => i64::from(b),
            Arg::Nan => f64::NAN.to_bits() as i64,
            Arg::Inf(neg) => if neg { f64::NEG_INFINITY } else { f64::INFINITY }.to_bits() as i64,
        }
    }

    pub fn source(self) -> String {
        match self {
            Arg::I(n) if n == i64::MIN => "(0 - 9223372036854775807 - 1)".to_string(),
            Arg::I(n) if n < 0 => format!("(0 - {})", -n),
            Arg::I(n) => n.to_string(),
            Arg::B(b) => b.to_string(),
            Arg::Nan => "sqrt(0.0 - 1.0)".to_string(),
            Arg::Inf(false) => "(2.0 ** 1024.0)".to_string(),
            Arg::Inf(true) => "(0.0 - (2.0 ** 1024.0))".to_string(),
            Arg::F(f) => {
                if f == 0.0 && f.is_sign_negative() {
                    return "(0.0 * (0.0 - 1.0))".to_string();
                }
                let mag = logicaffeine_data::fmt::fmt_f64(f.abs());
                let lit = if mag.contains('.') { mag } else { format!("{mag}.0") };
                if f < 0.0 {
                    format!("(0.0 - {lit})")
                } else {
                    lit
                }
            }
        }
    }
}

/// A function, its name, and the argument vectors it is called with.
pub struct Case {
    pub src: &'static str,
    pub func: &'static str,
    pub calls: Vec<Vec<Arg>>,
}

pub fn ints(vals: &[i64]) -> Vec<Vec<Arg>> {
    vals.iter().map(|v| vec![Arg::I(*v)]).collect()
}

pub fn int_pairs(vals: &[(i64, i64)]) -> Vec<Vec<Arg>> {
    vals.iter().map(|(a, b)| vec![Arg::I(*a), Arg::I(*b)]).collect()
}

pub fn cases() -> Vec<Case> {
    let edge = [0i64, 1, -1, 7, -7, 1000, i64::MAX, i64::MIN, i64::MAX / 2, 3_037_000_500];
    vec![
        Case {
            src: "## To add3 (a: Int, b: Int, c: Int) -> Int:\n    Return a + b + c.\n",
            func: "add3",
            calls: vec![
                vec![Arg::I(1), Arg::I(2), Arg::I(3)],
                vec![Arg::I(i64::MAX), Arg::I(1), Arg::I(-5)],
                vec![Arg::I(i64::MIN), Arg::I(-1), Arg::I(0)],
                vec![Arg::I(-40), Arg::I(12), Arg::I(9)],
            ],
        },
        Case { src: "## To poly (x: Int) -> Int:\n    Return ((2 * x + 5) * x - 3) * x + 100.\n", func: "poly", calls: ints(&edge) },
        Case {
            src: "## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n    If x is less than lo:\n        Return lo.\n    If x is greater than hi:\n        Return hi.\n    Return x.\n",
            func: "clamp",
            calls: vec![
                vec![Arg::I(5), Arg::I(0), Arg::I(10)],
                vec![Arg::I(-5), Arg::I(0), Arg::I(10)],
                vec![Arg::I(50), Arg::I(0), Arg::I(10)],
                vec![Arg::I(i64::MIN), Arg::I(i64::MIN), Arg::I(i64::MAX)],
            ],
        },
        Case {
            src: "## To sumto (n: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set s to s + i.\n        Set i to i + 1.\n    Return s.\n",
            func: "sumto",
            calls: ints(&[0, 1, 10, 100, -3, 5000]),
        },
        Case {
            src: "## To squares (a: Int, b: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from a to b:\n        Set s to s + i * i.\n    Return s.\n",
            func: "squares",
            calls: int_pairs(&[(1, 10), (5, 4), (-3, 3), (0, 0), (3_037_000_499, 3_037_000_500), (i64::MAX - 2, i64::MAX)]),
        },
        Case {
            src: "## To divmix (a: Int, b: Int) -> Int:\n    Return a / b + a % b * 7 - a // b.\n",
            func: "divmix",
            calls: int_pairs(&[(17, 5), (-17, 5), (17, -5), (-17, -5), (0, 3), (5, 0), (i64::MIN, -1), (i64::MIN, 2)]),
        },
        Case {
            src: "## To pw (a: Int, e: Int) -> Int:\n    Return a ** e.\n",
            func: "pw",
            calls: int_pairs(&[(2, 10), (-3, 5), (2, 62), (2, 63), (-2, 63), (0, 0), (5, -1), (1, 100_000), (10, 19)]),
        },
        Case {
            src: "## To lerp (a: Float, b: Float, t: Float) -> Float:\n    Return a + (b - a) * t.\n",
            func: "lerp",
            calls: vec![
                vec![Arg::F(0.0), Arg::F(10.0), Arg::F(0.25)],
                vec![Arg::F(-1.5), Arg::F(1e300), Arg::F(0.1)],
                vec![Arg::F(1.0 / 3.0), Arg::F(-0.0), Arg::F(3.0)],
                vec![Arg::F(5e-324), Arg::F(2.5), Arg::F(0.5)],
            ],
        },
        Case {
            src: "## To hyp (x: Float, y: Float) -> Float:\n    Let d be sqrt(x * x + y * y).\n    If d is greater than 10.0:\n        Return d / 2.0.\n    Return d.\n",
            func: "hyp",
            calls: vec![
                vec![Arg::F(3.0), Arg::F(4.0)],
                vec![Arg::F(30.0), Arg::F(40.0)],
                vec![Arg::F(-0.5), Arg::F(0.1)],
                vec![Arg::F(1e200), Arg::F(1e200)],
            ],
        },
        Case {
            src: "## To scale (n: Int, f: Float) -> Float:\n    Return n * f + 1.\n",
            func: "scale",
            calls: vec![vec![Arg::I(3), Arg::F(0.5)], vec![Arg::I(i64::MAX), Arg::F(2.0)], vec![Arg::I(-7), Arg::F(-0.1)]],
        },
        Case {
            src: "## To inside (x: Int, lo: Int, hi: Int) -> Bool:\n    Return x is at least lo and x is at most hi.\n",
            func: "inside",
            calls: vec![vec![Arg::I(5), Arg::I(0), Arg::I(10)], vec![Arg::I(-1), Arg::I(0), Arg::I(10)], vec![Arg::I(11), Arg::I(0), Arg::I(10)]],
        },
        Case {
            src: "## To bits (a: Int, b: Int) -> Int:\n    Return (a ^ b) & (a shifted left by 3) + (b shifted right by 2).\n",
            func: "bits",
            calls: int_pairs(&[(12, 10), (-1, 5), (i64::MIN, 3), (0x0F0F, 0x3333)]),
        },
        Case {
            src: "## To fl (x: Float) -> Int:\n    Return floor(x) + 1.\n",
            func: "fl",
            calls: vec![vec![Arg::F(2.5)], vec![Arg::F(-2.5)], vec![Arg::F(1e19)], vec![Arg::F(-0.0)], vec![Arg::F(9.2e18)]],
        },
        Case {
            src: "## To grid (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to n:\n        Repeat for j from 1 to i:\n            Set s to s + i * j.\n    Return s.\n",
            func: "grid",
            calls: ints(&[0, 1, 5, 30]),
        },
        Case {
            src: "## To firstsq (n: Int) -> Int:\n    Repeat for i from 1 to n:\n        If i * i is greater than n:\n            Return i.\n    Return 0.\n",
            func: "firstsq",
            calls: ints(&[0, 1, 10, 99, 100, 10_000]),
        },
        Case {
            src: "## To inv (x: Float) -> Float:\n    Return 1.0 / x.\n",
            func: "inv",
            calls: vec![vec![Arg::F(4.0)], vec![Arg::F(0.0)], vec![Arg::F(-0.0)], vec![Arg::F(3.0)]],
        },
        Case { src: "## To pc (x: Int) -> Int:\n    Return count_ones(x) * 2.\n", func: "pc", calls: ints(&edge) },
        Case {
            src: "## To cmps (a: Int, b: Int) -> Int:\n    Let mutable s be 0.\n    If a is less than b:\n        Set s to s + 1.\n    If a is greater than b:\n        Set s to s + 2.\n    If a is at most b:\n        Set s to s + 4.\n    If a is at least b:\n        Set s to s + 8.\n    If a is equal to b:\n        Set s to s + 16.\n    If a is not b:\n        Set s to s + 32.\n    Return s.\n",
            func: "cmps",
            calls: int_pairs(&[(3, 3), (2, 3), (3, 2), (i64::MIN, i64::MIN), (i64::MAX, i64::MIN), (-1, 0), (0, -1)]),
        },
        Case {
            src: "## To fcmps (a: Float, b: Float) -> Int:\n    Let mutable s be 0.\n    If a is less than b:\n        Set s to s + 1.\n    If a is greater than b:\n        Set s to s + 2.\n    If a is at most b:\n        Set s to s + 4.\n    If a is at least b:\n        Set s to s + 8.\n    If a is equal to b:\n        Set s to s + 16.\n    If a is not b:\n        Set s to s + 32.\n    Return s.\n",
            func: "fcmps",
            calls: vec![
                vec![Arg::F(1.5), Arg::F(1.5)],
                vec![Arg::F(1.0), Arg::F(1.5)],
                vec![Arg::F(1.5), Arg::F(1.0)],
                vec![Arg::F(0.0), Arg::F(-0.0)],
                vec![Arg::Nan, Arg::F(1.0)],
                vec![Arg::Nan, Arg::Nan],
                vec![Arg::Inf(true), Arg::Inf(false)],
                vec![Arg::F(5e-324), Arg::F(0.0)],
            ],
        },
        Case {
            src: "## To flags (a: Bool, b: Bool) -> Bool:\n    If a:\n        Return not b.\n    Return a ^ b.\n",
            func: "flags",
            calls: vec![vec![Arg::B(true), Arg::B(true)], vec![Arg::B(true), Arg::B(false)], vec![Arg::B(false), Arg::B(true)], vec![Arg::B(false), Arg::B(false)]],
        },
    ]
}

/// The program that defines `src`'s function `func` and shows one call of it.
pub fn show_program(src: &str, func: &str, args: &[Arg]) -> String {
    let list: Vec<String> = args.iter().map(|a| a.source()).collect();
    format!("{src}\n## Main\n    Show {func}({}).\n", list.join(", "))
}

/// The program that defines `case`'s function and shows one call of it.
pub fn program(case: &Case, args: &[Arg]) -> String {
    show_program(case.src, case.func, args)
}

pub fn region_of(case: &Case) -> RegionCfg {
    let first = program(case, &case.calls[0]);
    let (prog, names) = compile_to_program_optimized_named(&first).unwrap_or_else(|e| panic!("{}: does not compile: {e:?}", case.func));
    let found = discover(&prog, &names);
    let at = names.iter().position(|n| n == case.func).unwrap_or_else(|| panic!("{}: not in the optimized program (functions: {names:?})", case.func));
    match &found[at] {
        Discovered::Region(r) => r.clone(),
        Discovered::Host(h) => panic!("{}: stays on the host: {}", case.func, h.reason),
    }
}

/// How the VM prints a region value of kind `k`.
pub fn shown(k: Kind, v: i64) -> String {
    match k {
        Kind::Int => v.to_string(),
        Kind::Bool => (v != 0).to_string(),
        Kind::Float => logicaffeine_data::fmt::fmt_f64(f64::from_bits(v as u64)),
    }
}


/// Whether the VM's kernel promotes `op(a, b)` past `i64`.
pub fn kernel_promotes(op: CellOp, a: i64, b: i64) -> bool {
    let (x, y) = (RuntimeValue::Int(a), RuntimeValue::Int(b));
    let r = match op {
        CellOp::Alu(AluOp::Add) => arith::add(x, y),
        CellOp::Alu(AluOp::Sub) => arith::subtract(x, y),
        CellOp::Alu(AluOp::Mul) => arith::multiply(x, y),
        CellOp::Unit(FuncUnit::DivS) => arith::divide(x, y),
        CellOp::Unit(FuncUnit::FloorDivS) => arith::floor_divide(x, y),
        CellOp::Unit(FuncUnit::Pow) => arith::power(x, y),
        _ => return false,
    };
    matches!(r, Ok(RuntimeValue::BigInt(_)))
}

/// Judge one region call against the VM running the same call: `None` when they agree.
///
/// A value must be the VM's value as the VM prints it. A hand-back must be what the VM's own run shows:
/// its failure message for a zero divisor or a refused exponent, and for an `i64` overflow a kernel
/// promotion at exactly the operands the region stopped on.
pub fn judge_call(src: &str, func: &str, ret: Kind, args: &[Arg], got: &Result<i64, Halt>) -> Option<String> {
    let vm = vm_outcome(&show_program(src, func, args));
    let said = format!("{func}({args:?})");
    match got {
        Ok(v) => {
            let want = shown(ret, *v);
            if vm.error.is_some() || vm.output.trim_end() != want {
                Some(format!("{said}: region {want:?}, VM {:?} / {:?}", vm.output.trim_end(), vm.error))
            } else {
                None
            }
        }
        Err(Halt::Declined { divergence, op, a, b }) => {
            let err = vm.error.clone().unwrap_or_default();
            let justified = match divergence {
                Divergence::DivideByZero => err.contains("Division by zero") || err.contains("Modulo by zero"),
                Divergence::FloatDivideByZero => err.contains("Division by zero"),
                Divergence::NegativeExponent => err.contains("negative exponent"),
                Divergence::ExponentTooLarge => err.contains("exponent too large"),
                Divergence::IntOverflow => kernel_promotes(*op, *a, *b),
            };
            (!justified).then(|| format!("{said}: region handed back {divergence} at {}({a}, {b}), VM {:?} / {:?}", op.name(), vm.output.trim_end(), vm.error))
        }
        Err(Halt::OutOfFuel) => Some(format!("{said}: the region ran out of fuel")),
    }
}
