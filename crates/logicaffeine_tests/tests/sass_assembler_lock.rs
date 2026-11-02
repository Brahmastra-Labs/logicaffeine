//! sass_assembler_lock — proves our sm_86 SASS assembler foundation on a REAL cubin: locate the machine code
//! in the ELF, decode every instruction's control codes bit-exactly (cross-checked against nvdisasm), and PATCH
//! the machine code (change a stall count) so the change survives a re-disassembly. This is the read+write SASS
//! capability the MaxAs/turingas lever is built on — the path past the ptxas ceiling toward cuBLAS parity.
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_sass::{
    find_text_section, instructions, nvdisasm, patch_hi, Control,
};
use std::process::Command;

fn have_cuda() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Command::new("nvdisasm")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// A tiny kernel compiled to a real sm_86 cubin.
fn build_cubin(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let src = dir.join("k.cu");
    let cubin = dir.join("k.cubin");
    std::fs::write(&src, "extern \"C\" __global__ void addk(const int* a, const int* b, int* c, int n){int i=blockIdx.x*blockDim.x+threadIdx.x; if(i<n) c[i]=a[i]*3+b[i];}").ok()?;
    let o = Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-cubin", "-o"])
        .arg(&cubin)
        .arg(&src)
        .output()
        .ok()?;
    if !o.status.success() {
        eprintln!("nvcc: {}", String::from_utf8_lossy(&o.stderr));
        return None;
    }
    Some(cubin)
}

#[test]
fn we_locate_decode_and_patch_real_sm86_machine_code() {
    if !have_cuda() {
        eprintln!("SKIP: needs nvcc + nvdisasm");
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_sass_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let cubin_path = build_cubin(&dir).expect("build cubin");
    let bytes = std::fs::read(&cubin_path).expect("read cubin");

    // (1) locate the kernel's machine code in the ELF and parse instructions.
    let (off, size) = find_text_section(&bytes).expect("find .text.<kernel>");
    assert!(
        size >= 16 && size % 16 == 0,
        "text size {size} must be a whole number of 128-bit insns"
    );
    let insns = instructions(&bytes);
    assert!(!insns.is_empty(), "must parse at least one instruction");
    eprintln!(
        "kernel .text at file offset 0x{off:x}, {} instructions",
        insns.len()
    );

    // (2) our byte parse must match nvdisasm's own hex encodings (bit-exact). Both the lo word (inline after
    //     the mnemonic) and the hi word (its own line) appear as `/* 0x<16hex> */`; extract them all in order.
    let disasm = nvdisasm(&cubin_path).expect("nvdisasm");
    let all_hex = extract_hex(&disasm);
    assert!(
        all_hex.len() >= 2 * insns.len(),
        "nvdisasm should emit lo+hi per instruction"
    );
    for (i, ins) in insns.iter().enumerate() {
        assert_eq!(
            (ins.lo, ins.hi),
            (all_hex[2 * i], all_hex[2 * i + 1]),
            "insn {i}: ELF byte-parse must match nvdisasm encoding"
        );
    }

    // (3) every instruction's control codes must be well-formed (barriers 0-7, stall 0-15), and at least one
    //     variable-latency op must set a write barrier (proving we read real scheduling, not noise).
    let mut any_wr_barrier = false;
    for ins in &insns {
        let c = Control::decode(ins.hi);
        assert!(
            c.stall <= 15
                && c.wr_barrier <= 7
                && c.rd_barrier <= 7
                && c.wait_mask <= 0x3F
                && c.reuse <= 0xF
        );
        assert_eq!(
            c.apply(ins.hi),
            ins.hi,
            "decode∘apply must be identity (we fully model the control field)"
        );
        if c.wr_barrier != 7 {
            any_wr_barrier = true;
        }
    }
    assert!(
        any_wr_barrier,
        "a real kernel must have variable-latency ops that set write barriers"
    );

    // (4) PATCH: rewrite instruction 0's stall to the max (15), write the cubin, and verify nvdisasm reads the
    //     change back — i.e. we produced valid machine code the toolchain accepts.
    let mut patched = bytes.clone();
    let orig = Control::decode(insns[0].hi);
    let mut newc = orig;
    newc.stall = 15;
    patch_hi(&mut patched, 0, newc.apply(insns[0].hi));
    let patched_path = dir.join("k_patched.cubin");
    std::fs::write(&patched_path, &patched).expect("write patched");

    let reparsed = instructions(&patched);
    assert_eq!(
        Control::decode(reparsed[0].hi).stall,
        15,
        "our own byte-patch must set stall=15"
    );
    assert_ne!(
        orig.stall, 15,
        "the patch must be a real change (orig stall was not already 15)"
    );

    // and nvdisasm must agree the machine code changed: insn0's lo unchanged, hi = our patched word.
    let dis2 = nvdisasm(&patched_path).expect("nvdisasm patched");
    let hex2 = extract_hex(&dis2);
    assert_eq!(
        hex2[0], insns[0].lo,
        "lo word must be untouched by the patch"
    );
    assert_eq!(
        hex2[1],
        newc.apply(insns[0].hi),
        "nvdisasm must read back our patched hi word — valid machine code"
    );

    eprintln!("PATCHED sm_86 machine code: insn0 stall {} -> 15, verified by re-disassembly. We can read AND write SASS.", orig.stall);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Extract every `/* 0x<hex> */` encoding word from nvdisasm -c -hex output, in order (lo,hi per instruction).
fn extract_hex(s: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(p) = rest.find("/* 0x") {
        let after = &rest[p + 5..];
        let end = after.find(' ').unwrap_or(after.len());
        if let Ok(v) = u64::from_str_radix(&after[..end], 16) {
            out.push(v);
        }
        rest = &after[end..];
    }
    out
}
