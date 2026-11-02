//! The standard library's native KERNELS — the `## To native` functions of `crypto.lg` (ML-KEM's NTT,
//! sampling, and serialization; the high-level ML-KEM / ML-DSA primitives) — as ONE table every tier
//! reads.
//!
//! A `## To native` declaration has an EMPTY body: the engine supplies the implementation. The compiled
//! tier links `logicaffeine_system`'s kernel ([`NativeKernel::rust_path`]); the tree-walker calls
//! [`NativeKernel::call`]; the VM lowers a kernel call to `CallBuiltin(NativeKernel)` whose first
//! argument is the kernel's [`NativeKernel::code`], and the builtin runs the same [`NativeKernel::call`].
//! So every tier runs the very same Rust function, and a kernel added here reaches all of them at once.
//!
//! [`NativeKernel::code`] is the kernel's POSITION in [`NativeKernel::ALL`] and travels in bytecode (and
//! the binary wire format), so new kernels are APPENDED — `goldens/native_kernel_codes.txt` locks it.

use crate::interpreter::{ListRepr, RuntimeValue};
use logicaffeine_base::{Word16, WordVal};
use logicaffeine_data::LogosSeq;
use logicaffeine_system::{mldsa, mlkem, ntt};
use std::cell::RefCell;
use std::rc::Rc;

/// One native kernel of the standard library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeKernel {
    MlkemNtt,
    MlkemBaseMulW16,
    ToMontW16,
    Cbd2W16,
    CompressW16,
    DecompressW16,
    ByteEncodeW16,
    ByteDecodeW16,
    SampleAW16,
    SampleMatrixW16,
    MlkemNoiseBatch,
    AddModQW16,
    SubModQW16,
    ZerosW16,
    MlkemInvNtt,
    MlkemBaseMul,
    Cbd2,
    Cbd3,
    Compress,
    Decompress,
    ByteEncode,
    ByteDecode,
    SampleNtt,
    SampleA,
    ToMont,
    MlkemKeypair,
    MlkemEncapsKem,
    MlkemDecapsKem,
    MldsaKeypair,
    MldsaSign,
    MldsaVerify,
}

impl NativeKernel {
    /// Every kernel, in code order. Append only.
    pub const ALL: &'static [NativeKernel] = &[
        NativeKernel::MlkemNtt,
        NativeKernel::MlkemBaseMulW16,
        NativeKernel::ToMontW16,
        NativeKernel::Cbd2W16,
        NativeKernel::CompressW16,
        NativeKernel::DecompressW16,
        NativeKernel::ByteEncodeW16,
        NativeKernel::ByteDecodeW16,
        NativeKernel::SampleAW16,
        NativeKernel::SampleMatrixW16,
        NativeKernel::MlkemNoiseBatch,
        NativeKernel::AddModQW16,
        NativeKernel::SubModQW16,
        NativeKernel::ZerosW16,
        NativeKernel::MlkemInvNtt,
        NativeKernel::MlkemBaseMul,
        NativeKernel::Cbd2,
        NativeKernel::Cbd3,
        NativeKernel::Compress,
        NativeKernel::Decompress,
        NativeKernel::ByteEncode,
        NativeKernel::ByteDecode,
        NativeKernel::SampleNtt,
        NativeKernel::SampleA,
        NativeKernel::ToMont,
        NativeKernel::MlkemKeypair,
        NativeKernel::MlkemEncapsKem,
        NativeKernel::MlkemDecapsKem,
        NativeKernel::MldsaKeypair,
        NativeKernel::MldsaSign,
        NativeKernel::MldsaVerify,
    ];

    /// The LOGOS name the standard library declares it under.
    pub const fn name(self) -> &'static str {
        match self {
            NativeKernel::MlkemNtt => "mlkemNtt",
            NativeKernel::MlkemBaseMulW16 => "mlkemBaseMulW16",
            NativeKernel::ToMontW16 => "toMontW16",
            NativeKernel::Cbd2W16 => "cbd2W16",
            NativeKernel::CompressW16 => "compressW16",
            NativeKernel::DecompressW16 => "decompressW16",
            NativeKernel::ByteEncodeW16 => "byteEncodeW16",
            NativeKernel::ByteDecodeW16 => "byteDecodeW16",
            NativeKernel::SampleAW16 => "sampleAW16",
            NativeKernel::SampleMatrixW16 => "sampleMatrixW16",
            NativeKernel::MlkemNoiseBatch => "mlkemNoiseBatch",
            NativeKernel::AddModQW16 => "addModQW16",
            NativeKernel::SubModQW16 => "subModQW16",
            NativeKernel::ZerosW16 => "zerosW16",
            NativeKernel::MlkemInvNtt => "mlkemInvNtt",
            NativeKernel::MlkemBaseMul => "mlkemBaseMul",
            NativeKernel::Cbd2 => "cbd2",
            NativeKernel::Cbd3 => "cbd3",
            NativeKernel::Compress => "compress",
            NativeKernel::Decompress => "decompress",
            NativeKernel::ByteEncode => "byteEncode",
            NativeKernel::ByteDecode => "byteDecode",
            NativeKernel::SampleNtt => "sampleNtt",
            NativeKernel::SampleA => "sampleA",
            NativeKernel::ToMont => "toMont",
            NativeKernel::MlkemKeypair => "mlkemKeypair",
            NativeKernel::MlkemEncapsKem => "mlkemEncapsKem",
            NativeKernel::MlkemDecapsKem => "mlkemDecapsKem",
            NativeKernel::MldsaKeypair => "mldsaKeypair",
            NativeKernel::MldsaSign => "mldsaSign",
            NativeKernel::MldsaVerify => "mldsaVerify",
        }
    }

    /// How many arguments the declaration takes.
    pub const fn arity(self) -> usize {
        match self {
            NativeKernel::MlkemNtt
            | NativeKernel::ToMontW16
            | NativeKernel::Cbd2W16
            | NativeKernel::SampleMatrixW16
            | NativeKernel::ZerosW16
            | NativeKernel::MlkemInvNtt
            | NativeKernel::Cbd2
            | NativeKernel::Cbd3
            | NativeKernel::SampleNtt
            | NativeKernel::ToMont
            | NativeKernel::MldsaKeypair => 1,
            NativeKernel::MlkemBaseMulW16
            | NativeKernel::CompressW16
            | NativeKernel::DecompressW16
            | NativeKernel::ByteEncodeW16
            | NativeKernel::ByteDecodeW16
            | NativeKernel::AddModQW16
            | NativeKernel::SubModQW16
            | NativeKernel::MlkemBaseMul
            | NativeKernel::Compress
            | NativeKernel::Decompress
            | NativeKernel::ByteEncode
            | NativeKernel::ByteDecode
            | NativeKernel::MlkemKeypair
            | NativeKernel::MlkemEncapsKem
            | NativeKernel::MlkemDecapsKem => 2,
            NativeKernel::SampleAW16
            | NativeKernel::MlkemNoiseBatch
            | NativeKernel::SampleA
            | NativeKernel::MldsaSign => 3,
            NativeKernel::MldsaVerify => 4,
        }
    }

    /// The `logicaffeine_system` `(module, function)` the compiled tier calls.
    pub const fn rust_path(self) -> (&'static str, &'static str) {
        match self {
            NativeKernel::MlkemNtt => ("ntt", "mlkem_ntt"),
            NativeKernel::MlkemBaseMulW16 => ("ntt", "mlkem_base_mul_w16_seq"),
            NativeKernel::ToMontW16 => ("ntt", "mlkem_to_mont_w16_seq"),
            NativeKernel::Cbd2W16 => ("ntt", "mlkem_cbd2_w16_from_int"),
            NativeKernel::CompressW16 => ("ntt", "mlkem_compress_w16_seq"),
            NativeKernel::DecompressW16 => ("ntt", "mlkem_decompress_w16_seq"),
            NativeKernel::ByteEncodeW16 => ("ntt", "mlkem_byte_encode_w16_to_int"),
            NativeKernel::ByteDecodeW16 => ("ntt", "mlkem_byte_decode_w16_from_int"),
            NativeKernel::SampleAW16 => ("ntt", "mlkem_sample_a_w16_from_int"),
            NativeKernel::SampleMatrixW16 => ("ntt", "mlkem_sample_matrix_w16_from_int"),
            NativeKernel::MlkemNoiseBatch => ("mlkem", "mlkem_noise_batch_from_int"),
            NativeKernel::AddModQW16 => ("ntt", "mlkem_add_mod_q_w16"),
            NativeKernel::SubModQW16 => ("ntt", "mlkem_sub_mod_q_w16"),
            NativeKernel::ZerosW16 => ("ntt", "mlkem_zeros_w16"),
            NativeKernel::MlkemInvNtt => ("ntt", "mlkem_inv_ntt"),
            NativeKernel::MlkemBaseMul => ("ntt", "mlkem_base_mul"),
            NativeKernel::Cbd2 => ("ntt", "mlkem_cbd2"),
            NativeKernel::Cbd3 => ("ntt", "mlkem_cbd3"),
            NativeKernel::Compress => ("ntt", "mlkem_compress"),
            NativeKernel::Decompress => ("ntt", "mlkem_decompress"),
            NativeKernel::ByteEncode => ("ntt", "mlkem_byte_encode"),
            NativeKernel::ByteDecode => ("ntt", "mlkem_byte_decode"),
            NativeKernel::SampleNtt => ("ntt", "mlkem_sample_ntt"),
            NativeKernel::SampleA => ("ntt", "mlkem_sample_a"),
            NativeKernel::ToMont => ("ntt", "mlkem_to_mont"),
            NativeKernel::MlkemKeypair => ("mlkem", "mlkem_keypair_seq"),
            NativeKernel::MlkemEncapsKem => ("mlkem", "mlkem_encaps_seq"),
            NativeKernel::MlkemDecapsKem => ("mlkem", "mlkem_decaps_seq"),
            NativeKernel::MldsaKeypair => ("mldsa", "mldsa_keypair_seq"),
            NativeKernel::MldsaSign => ("mldsa", "mldsa_sign_seq"),
            NativeKernel::MldsaVerify => ("mldsa", "mldsa_verify_seq"),
        }
    }

    /// The kernel's stable code — its position in [`Self::ALL`].
    pub fn code(self) -> i64 {
        Self::ALL
            .iter()
            .position(|&k| k == self)
            .expect("every kernel is in ALL") as i64
    }

    /// The kernel with this code.
    pub fn from_code(code: i64) -> Option<NativeKernel> {
        usize::try_from(code)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// The kernel declared under `name`, if any.
    pub fn from_name(name: &str) -> Option<NativeKernel> {
        Self::ALL.iter().copied().find(|k| k.name() == name)
    }

    /// Why a backend that cannot link the Rust kernels refuses a program that calls this one.
    pub fn refusal(self, backend: &str) -> String {
        format!(
            "`{}` is a standard-library native kernel (crypto.lg) implemented in Rust — the {backend} \
             backend cannot link it; run the program on the interpreter or the VM, or build it with the \
             Rust tier (`largo build`)",
            self.name()
        )
    }

    /// Run the kernel on already-evaluated arguments.
    pub fn call(self, args: &[RuntimeValue]) -> Result<RuntimeValue, String> {
        if args.len() != self.arity() {
            return Err(format!(
                "{}() takes {} argument{}, got {}",
                self.name(),
                self.arity(),
                if self.arity() == 1 { "" } else { "s" },
                args.len()
            ));
        }
        let name = self.name();
        let int = |i: usize| int_arg(name, &args[i]);
        let ints = |i: usize| int_seq(name, &args[i]);
        let w16 = |i: usize| word16_seq(name, &args[i]);
        let ran = match self {
            NativeKernel::MlkemNtt => ntt::mlkem_ntt(&ints(0)?).map(from_ints),
            NativeKernel::MlkemBaseMulW16 => {
                ntt::mlkem_base_mul_w16_seq(&w16(0)?, &w16(1)?).map(from_w16)
            }
            NativeKernel::ToMontW16 => ntt::mlkem_to_mont_w16_seq(&w16(0)?).map(from_w16),
            NativeKernel::Cbd2W16 => ntt::mlkem_cbd2_w16_from_int(&ints(0)?).map(from_w16),
            NativeKernel::CompressW16 => {
                ntt::mlkem_compress_w16_seq(&w16(0)?, int(1)?).map(from_w16)
            }
            NativeKernel::DecompressW16 => {
                ntt::mlkem_decompress_w16_seq(&w16(0)?, int(1)?).map(from_w16)
            }
            NativeKernel::ByteEncodeW16 => {
                ntt::mlkem_byte_encode_w16_to_int(&w16(0)?, int(1)?).map(from_ints)
            }
            NativeKernel::ByteDecodeW16 => {
                ntt::mlkem_byte_decode_w16_from_int(&ints(0)?, int(1)?).map(from_w16)
            }
            NativeKernel::SampleAW16 => {
                ntt::mlkem_sample_a_w16_from_int(&ints(0)?, int(1)?, int(2)?).map(from_w16)
            }
            NativeKernel::SampleMatrixW16 => {
                ntt::mlkem_sample_matrix_w16_from_int(&ints(0)?).map(from_w16)
            }
            NativeKernel::MlkemNoiseBatch => {
                mlkem::mlkem_noise_batch_from_int(&ints(0)?, int(1)?, int(2)?).map(from_w16)
            }
            NativeKernel::AddModQW16 => ntt::mlkem_add_mod_q_w16(&w16(0)?, &w16(1)?).map(from_w16),
            NativeKernel::SubModQW16 => ntt::mlkem_sub_mod_q_w16(&w16(0)?, &w16(1)?).map(from_w16),
            NativeKernel::ZerosW16 => ntt::mlkem_zeros_w16(int(0)?).map(from_w16),
            NativeKernel::MlkemInvNtt => ntt::mlkem_inv_ntt(&ints(0)?).map(from_ints),
            NativeKernel::MlkemBaseMul => ntt::mlkem_base_mul(&ints(0)?, &ints(1)?).map(from_ints),
            NativeKernel::Cbd2 => ntt::mlkem_cbd2(&ints(0)?).map(from_ints),
            NativeKernel::Cbd3 => ntt::mlkem_cbd3(&ints(0)?).map(from_ints),
            NativeKernel::Compress => ntt::mlkem_compress(&ints(0)?, int(1)?).map(from_ints),
            NativeKernel::Decompress => ntt::mlkem_decompress(&ints(0)?, int(1)?).map(from_ints),
            NativeKernel::ByteEncode => ntt::mlkem_byte_encode(&ints(0)?, int(1)?).map(from_ints),
            NativeKernel::ByteDecode => ntt::mlkem_byte_decode(&ints(0)?, int(1)?).map(from_ints),
            NativeKernel::SampleNtt => ntt::mlkem_sample_ntt(&ints(0)?).map(from_ints),
            NativeKernel::SampleA => {
                ntt::mlkem_sample_a(&ints(0)?, int(1)?, int(2)?).map(from_ints)
            }
            NativeKernel::ToMont => ntt::mlkem_to_mont(&ints(0)?).map(from_ints),
            NativeKernel::MlkemKeypair => {
                mlkem::mlkem_keypair_seq(&ints(0)?, &ints(1)?).map(from_ints)
            }
            NativeKernel::MlkemEncapsKem => {
                mlkem::mlkem_encaps_seq(&ints(0)?, &ints(1)?).map(from_ints)
            }
            NativeKernel::MlkemDecapsKem => {
                mlkem::mlkem_decaps_seq(&ints(0)?, &ints(1)?).map(from_ints)
            }
            NativeKernel::MldsaKeypair => mldsa::mldsa_keypair_seq(&ints(0)?).map(from_ints),
            NativeKernel::MldsaSign => {
                mldsa::mldsa_sign_seq(&ints(0)?, &ints(1)?, &ints(2)?).map(from_ints)
            }
            NativeKernel::MldsaVerify => {
                mldsa::mldsa_verify_seq(&ints(0)?, &ints(1)?, &ints(2)?, &ints(3)?)
                    .map(RuntimeValue::Int)
            }
        };
        ran.map_err(|why| self.failure(&why))
    }

    /// The report a refused call stops the program with, on every tier: the kernel's name, then why
    /// its `logicaffeine_system` function refused the arguments. The compiled tier panics with the same
    /// text (see the Rust backend's native-function emission).
    pub fn failure(self, why: &str) -> String {
        format!("{}(): {why}", self.name())
    }
}

/// The refusal for a backend that lowers compiled bytecode but cannot link the Rust kernels: `Err` at
/// the first kernel call that can RUN (named when its code is a constant, as the VM compiler emits
/// it). A kernel in a function nothing reaches is code the loader spliced in and nothing calls, so it
/// refuses nothing — [`crate::vm::reach`] is the same answer the backends lower by.
pub fn refuse_kernel_calls(
    program: &crate::vm::CompiledProgram,
    interner: &crate::Interner,
    backend: &str,
) -> Result<(), String> {
    refuse_kernel_calls_with(
        program,
        interner,
        backend,
        &crate::vm::reach::HostEntries::default(),
    )
}

/// [`refuse_kernel_calls`], judging what the host's own `entries` reach as well as Main — the same
/// roots a module built with those entries lowers.
pub fn refuse_kernel_calls_with(
    program: &crate::vm::CompiledProgram,
    interner: &crate::Interner,
    backend: &str,
    entries: &crate::vm::reach::HostEntries,
) -> Result<(), String> {
    use crate::semantics::builtins::BuiltinId;
    use crate::vm::{Constant, Op};
    for (pc, op) in crate::vm::reach::running_ops_with(program, interner, entries) {
        let Op::CallBuiltin {
            builtin: BuiltinId::NativeKernel,
            args_start,
            ..
        } = *op
        else {
            continue;
        };
        let kernel = program.code[..pc]
            .iter()
            .rev()
            .find_map(|prior| match *prior {
                Op::LoadConst { dst, idx } if dst == args_start => {
                    match program.constants.get(idx as usize) {
                        Some(Constant::Int(code)) => Some(NativeKernel::from_code(*code)),
                        _ => Some(None),
                    }
                }
                _ => None,
            });
        return Err(match kernel.flatten() {
            Some(k) => k.refusal(backend),
            None => format!(
                "the program calls a standard-library native kernel (crypto.lg) implemented in Rust — the \
                 {backend} backend cannot link it"
            ),
        });
    }
    Ok(())
}

fn int_arg(kernel: &str, v: &RuntimeValue) -> Result<i64, String> {
    match v {
        RuntimeValue::Int(n) => Ok(*n),
        other => Err(format!(
            "{kernel}() needs an Int here, got {}",
            other.type_name()
        )),
    }
}

fn int_seq(kernel: &str, v: &RuntimeValue) -> Result<Vec<i64>, String> {
    let RuntimeValue::List(items) = v else {
        return Err(format!(
            "{kernel}() needs a Seq of Int here, got {}",
            v.type_name()
        ));
    };
    items
        .borrow()
        .to_values()
        .iter()
        .enumerate()
        .map(|(i, e)| match e {
            RuntimeValue::Int(n) => Ok(*n),
            other => Err(format!(
                "{kernel}() needs a Seq of Int here; item {} is {}",
                i + 1,
                other.type_name()
            )),
        })
        .collect()
}

fn word16_seq(kernel: &str, v: &RuntimeValue) -> Result<Vec<Word16>, String> {
    let RuntimeValue::List(items) = v else {
        return Err(format!(
            "{kernel}() needs a Seq of Word16 here, got {}",
            v.type_name()
        ));
    };
    items
        .borrow()
        .to_values()
        .iter()
        .enumerate()
        .map(|(i, e)| match e {
            RuntimeValue::Word(WordVal::W16(w)) => Ok(*w),
            other => Err(format!(
                "{kernel}() needs a Seq of Word16 here; item {} is {}",
                i + 1,
                other.type_name()
            )),
        })
        .collect()
}

fn from_ints(seq: LogosSeq<i64>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(seq.to_vec()))))
}

fn from_w16(seq: LogosSeq<Word16>) -> RuntimeValue {
    let values = seq
        .to_vec()
        .into_iter()
        .map(|w| RuntimeValue::Word(WordVal::W16(w)))
        .collect();
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(values))))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/native_kernels/tests.rs"]
mod tests;
