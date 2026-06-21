//! The Forge: the copy-and-patch JIT's executable-memory layer.
//!
//! [`JitPage`] allocates page-aligned memory, copies machine code into it,
//! makes it executable, and hands back a callable function pointer. This is
//! the foundation the copy-and-patch JIT builds on: at runtime, compiling a
//! function is `memcpy(stencil bytes)` + patch relocations, then flip the page
//! to executable.
//!
//! # W^X model per platform
//!
//! - **macOS/aarch64 (Apple Silicon)**: `mmap(PROT_RWX, MAP_JIT)` plus
//!   per-thread `pthread_jit_write_protect_np` toggling, and a mandatory
//!   `sys_icache_invalidate` after writing (ARM's I-cache is not coherent with
//!   stores). The write-protect toggle is PER-THREAD: all writes happen inside
//!   [`JitPage::new`] on the constructing thread, before any function pointer
//!   can escape, so no cross-thread W^X hazard exists.
//! - **Other Unix (Linux, Intel macOS)**: `mmap(RW)` → copy → `mprotect(RX)`
//!   (checked). On aarch64 Linux the I-cache is flushed with inline asm —
//!   `mprotect` does NOT do that for you.
//! - **Windows**: `VirtualAlloc(RW)` → copy → `VirtualProtect(EXECUTE_READ)`
//!   (checked) → `FlushInstructionCache`.
//!
//! NATIVE ONLY. A copy-and-patch JIT emits raw machine code and cannot run in
//! the WASM sandbox; the browser uses the bytecode VM instead.

#![cfg(not(target_arch = "wasm32"))]

use std::fmt;
use std::mem;

pub mod buffer;
pub mod jit;
pub mod patch;
#[cfg(target_arch = "x86_64")]
pub mod regalloc;
pub mod segv_trace;
#[cfg(target_arch = "x86_64")]
pub mod x64asm;
mod stencil_model;
pub use stencil_model::{HoleId, Reloc, RelocKind, Stencil};

// Build-time-extracted stencils (machine code of the Rust functions in
// `stencils/int_stencils.rs`). See build.rs.
include!(concat!(env!("OUT_DIR"), "/stencils.rs"));

/// Executable-memory errors.
#[derive(Debug)]
pub enum JitError {
    /// `JitPage::new` was given no code.
    EmptyCode,
    /// The executable mapping could not be created.
    Map(std::io::Error),
    /// The mapping could not be flipped to read+execute.
    Protect(std::io::Error),
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JitError::EmptyCode => write!(f, "JitPage: empty code"),
            JitError::Map(e) => write!(f, "executable mapping failed: {e}"),
            JitError::Protect(e) => write!(f, "marking page executable failed: {e}"),
        }
    }
}

impl std::error::Error for JitError {}

/// A page-aligned block of executable memory holding JIT-compiled machine code.
#[derive(Debug)]
pub struct JitPage {
    ptr: *mut u8,
    /// Bytes of machine code written.
    code_len: usize,
    /// Bytes actually mapped (code_len rounded up to the page size) — the
    /// length that must be passed back when unmapping.
    alloc_len: usize,
}

// SAFETY: the page's code is written and sealed inside `new` before the value
// exists; afterwards JitPage is an immutable handle to read/execute-only
// memory, which is safe to use AND share across threads (no interior
// mutability — execution only reads the mapping).
unsafe impl Send for JitPage {}
unsafe impl Sync for JitPage {}

impl JitPage {
    /// Allocate an executable page, copy `code` into it, and make it runnable.
    pub fn new(code: &[u8]) -> Result<JitPage, JitError> {
        if code.is_empty() {
            return Err(JitError::EmptyCode);
        }
        let page = page_size();
        let alloc_len = code.len().div_ceil(page) * page;
        let ptr = map_executable(alloc_len)?;
        debug_assert_eq!(
            ptr as usize % page,
            0,
            "executable mapping is not page-aligned"
        );
        unsafe {
            write_code(ptr, code)?;
        }
        Ok(JitPage { ptr, code_len: code.len(), alloc_len })
    }

    /// Patch one 8-byte LITERAL-POOL word after sealing (the self-call
    /// entry: a chain's own base address becomes known only after layout).
    /// Data-only — the word lives in the pool, never decoded as code — so
    /// no instruction-cache concerns; the brief RW window happens on the
    /// compiling thread before the chain ever runs.
    #[cfg(all(unix, not(all(target_os = "macos", target_arch = "aarch64"))))]
    pub fn patch_word(&self, offset: usize, value: u64) -> Result<(), JitError> {
        assert!(offset + 8 <= self.code_len, "patch outside the mapping");
        let page = page_size();
        let start = (self.ptr as usize + offset) & !(page - 1);
        let end = self.ptr as usize + offset + 8;
        let len = end - start;
        unsafe {
            let rc = libc::mprotect(
                start as *mut libc::c_void,
                len,
                libc::PROT_READ | libc::PROT_WRITE,
            );
            if rc != 0 {
                return Err(JitError::Protect(std::io::Error::last_os_error()));
            }
            std::ptr::write_unaligned(self.ptr.add(offset) as *mut u64, value);
            let rc = libc::mprotect(
                start as *mut libc::c_void,
                len,
                libc::PROT_READ | libc::PROT_EXEC,
            );
            if rc != 0 {
                return Err(JitError::Protect(std::io::Error::last_os_error()));
            }
        }
        Ok(())
    }

    /// Self-entry patching is unsupported on MAP_JIT targets (Apple
    /// Silicon) — callers fall back to the table-indirect call stencil.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pub fn patch_word(&self, _offset: usize, _value: u64) -> Result<(), JitError> {
        Err(JitError::EmptyCode)
    }

    /// Two-phase construction for PATCHED code: maps first (so the final base
    /// address is known), lets `fill` produce the bytes against that base,
    /// then writes and seals. `fill` must return exactly `len` bytes.
    pub fn with_layout(
        len: usize,
        fill: impl FnOnce(u64) -> Vec<u8>,
    ) -> Result<JitPage, JitError> {
        if len == 0 {
            return Err(JitError::EmptyCode);
        }
        let page = page_size();
        let alloc_len = len.div_ceil(page) * page;
        let ptr = map_executable(alloc_len)?;
        let code = fill(ptr as u64);
        debug_assert_eq!(code.len(), len, "fill returned a different length");
        unsafe {
            write_code(ptr, &code)?;
        }
        Ok(JitPage { ptr, code_len: len, alloc_len })
    }

    /// Reinterpret the page as an `extern "C" fn(i64, i64) -> i64`.
    ///
    /// # Safety
    /// The caller must guarantee the page actually contains valid machine code
    /// implementing this exact signature, and must not call the pointer after
    /// the `JitPage` is dropped.
    pub unsafe fn as_fn_i64_i64(&self) -> extern "C" fn(i64, i64) -> i64 {
        mem::transmute::<*mut u8, extern "C" fn(i64, i64) -> i64>(self.ptr)
    }

    /// Raw pointer to the executable code (for patching / inspection).
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Length of the machine code in bytes (the mapping itself is
    /// [`JitPage::alloc_len`] bytes).
    pub fn len(&self) -> usize {
        self.code_len
    }

    /// Bytes actually mapped (page-size multiple).
    pub fn alloc_len(&self) -> usize {
        self.alloc_len
    }

    /// Whether the page is empty (always false; `new` rejects empty code).
    pub fn is_empty(&self) -> bool {
        self.code_len == 0
    }
}

impl Drop for JitPage {
    fn drop(&mut self) {
        unsafe {
            #[cfg(unix)]
            {
                let rc = libc::munmap(self.ptr as *mut libc::c_void, self.alloc_len);
                debug_assert_eq!(rc, 0, "munmap failed for a JitPage mapping");
            }
            #[cfg(windows)]
            {
                let rc = windows_sys::Win32::System::Memory::VirtualFree(
                    self.ptr as *mut core::ffi::c_void,
                    0,
                    windows_sys::Win32::System::Memory::MEM_RELEASE,
                );
                debug_assert_ne!(rc, 0, "VirtualFree failed for a JitPage mapping");
            }
        }
    }
}

/// The system page size (Apple Silicon uses 16 KiB; most x86-64 systems 4 KiB).
fn page_size() -> usize {
    #[cfg(unix)]
    unsafe {
        let sz = libc::sysconf(libc::_SC_PAGESIZE);
        if sz > 0 {
            sz as usize
        } else {
            4096
        }
    }
    #[cfg(windows)]
    unsafe {
        let mut info = mem::zeroed::<windows_sys::Win32::System::SystemInformation::SYSTEM_INFO>();
        windows_sys::Win32::System::SystemInformation::GetSystemInfo(&mut info);
        info.dwPageSize as usize
    }
}

// ---------------------------------------------------------------------------
// macOS / Apple Silicon: MAP_JIT + write-protect toggling.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod sys {
    /// `MAP_JIT` is required for executable allocations under the hardened
    /// runtime on Apple Silicon.
    pub const MAP_JIT: libc::c_int = 0x800;

    extern "C" {
        /// Toggle the calling thread's JIT region between writable (`0`) and
        /// executable (`1`). PER-THREAD state — confined to `JitPage::new`.
        pub fn pthread_jit_write_protect_np(enabled: libc::c_int);
        /// Flush the instruction cache for a region after writing code into it.
        pub fn sys_icache_invalidate(start: *mut libc::c_void, len: libc::size_t);
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn map_executable(len: usize) -> Result<*mut u8, JitError> {
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANON | sys::MAP_JIT,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(JitError::Map(std::io::Error::last_os_error()));
    }
    Ok(ptr as *mut u8)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
unsafe fn write_code(dst: *mut u8, code: &[u8]) -> Result<(), JitError> {
    // Make the JIT region writable for THIS thread, copy, then re-protect.
    sys::pthread_jit_write_protect_np(0);
    std::ptr::copy_nonoverlapping(code.as_ptr(), dst, code.len());
    sys::pthread_jit_write_protect_np(1);
    // ARM requires an explicit I-cache flush of freshly written code.
    sys::sys_icache_invalidate(dst as *mut libc::c_void, code.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Other Unix (Linux, Intel macOS): mmap RW, then mprotect to RX.
// ---------------------------------------------------------------------------

#[cfg(all(unix, not(all(target_os = "macos", target_arch = "aarch64"))))]
fn map_executable(len: usize) -> Result<*mut u8, JitError> {
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(JitError::Map(std::io::Error::last_os_error()));
    }
    Ok(ptr as *mut u8)
}

#[cfg(all(unix, not(all(target_os = "macos", target_arch = "aarch64"))))]
unsafe fn write_code(dst: *mut u8, code: &[u8]) -> Result<(), JitError> {
    std::ptr::copy_nonoverlapping(code.as_ptr(), dst, code.len());
    let rc = libc::mprotect(
        dst as *mut libc::c_void,
        code.len(),
        libc::PROT_READ | libc::PROT_EXEC,
    );
    if rc != 0 {
        return Err(JitError::Protect(std::io::Error::last_os_error()));
    }
    // `mprotect` does NOT flush the instruction cache on ARM.
    #[cfg(target_arch = "aarch64")]
    flush_icache_aarch64(dst, code.len());
    Ok(())
}

/// aarch64 I-cache flush (Linux): clean D-cache to the point of unification,
/// invalidate I-cache, then synchronize. 64-byte lines is a safe lower bound.
#[cfg(all(unix, target_arch = "aarch64", not(target_os = "macos")))]
unsafe fn flush_icache_aarch64(start: *mut u8, len: usize) {
    const LINE: usize = 64;
    let begin = start as usize & !(LINE - 1);
    let end = start as usize + len;
    let mut addr = begin;
    while addr < end {
        core::arch::asm!("dc cvau, {0}", in(reg) addr, options(nostack, preserves_flags));
        addr += LINE;
    }
    core::arch::asm!("dsb ish", options(nostack, preserves_flags));
    let mut addr = begin;
    while addr < end {
        core::arch::asm!("ic ivau, {0}", in(reg) addr, options(nostack, preserves_flags));
        addr += LINE;
    }
    core::arch::asm!("dsb ish", "isb", options(nostack, preserves_flags));
}

// ---------------------------------------------------------------------------
// Windows: VirtualAlloc RW → copy → VirtualProtect RX → FlushInstructionCache.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn map_executable(len: usize) -> Result<*mut u8, JitError> {
    use windows_sys::Win32::System::Memory::{
        VirtualAlloc, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE,
    };
    let ptr = unsafe {
        VirtualAlloc(
            std::ptr::null(),
            len,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if ptr.is_null() {
        return Err(JitError::Map(std::io::Error::last_os_error()));
    }
    Ok(ptr as *mut u8)
}

#[cfg(windows)]
unsafe fn write_code(dst: *mut u8, code: &[u8]) -> Result<(), JitError> {
    use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
    use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READ};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    std::ptr::copy_nonoverlapping(code.as_ptr(), dst, code.len());
    let mut old = 0u32;
    let rc = VirtualProtect(
        dst as *const core::ffi::c_void,
        code.len(),
        PAGE_EXECUTE_READ,
        &mut old,
    );
    if rc == 0 {
        return Err(JitError::Protect(std::io::Error::last_os_error()));
    }
    let rc = FlushInstructionCache(
        GetCurrentProcess(),
        dst as *const core::ffi::c_void,
        code.len(),
    );
    if rc == 0 {
        return Err(JitError::Protect(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Machine code for `extern "C" fn(i64, i64) -> i64 { a.wrapping_add(b) }`
    /// on aarch64: `add x0, x0, x1` ; `ret`.
    #[cfg(target_arch = "aarch64")]
    const ADD_CODE: [u8; 8] = [0x00, 0x00, 0x01, 0x8b, 0xc0, 0x03, 0x5f, 0xd6];

    /// Same on x86-64 (SysV ABI: a=rdi, b=rsi, ret=rax):
    /// `lea rax, [rdi+rsi]` (48 8D 04 37) ; `ret` (C3).
    #[cfg(target_arch = "x86_64")]
    const ADD_CODE: [u8; 5] = [0x48, 0x8d, 0x04, 0x37, 0xc3];

    #[test]
    fn jit_add_3_5_equals_8() {
        let page = JitPage::new(&ADD_CODE).expect("could not allocate executable page");
        let add = unsafe { page.as_fn_i64_i64() };
        assert_eq!(add(3, 5), 8);
        assert_eq!(add(100, -1), 99);
        assert_eq!(add(0, 0), 0);
        // The hardware ADD wraps (matching the interpreter's wrapping arithmetic).
        assert_eq!(add(i64::MAX, 1), i64::MIN);
    }

    #[test]
    fn jit_page_reports_lengths_and_alignment() {
        let page = JitPage::new(&ADD_CODE).unwrap();
        assert_eq!(page.len(), ADD_CODE.len());
        assert!(!page.is_empty());
        assert!(!page.as_ptr().is_null());
        // The mapping is page-aligned and page-granular.
        let ps = super::page_size();
        assert_eq!(page.alloc_len() % ps, 0);
        assert!(page.alloc_len() >= page.len());
        assert_eq!(page.as_ptr() as usize % ps, 0);
    }

    #[test]
    fn jit_page_empty_code_is_error() {
        match JitPage::new(&[]) {
            Err(JitError::EmptyCode) => {}
            other => panic!("expected EmptyCode error, got {:?}", other.map(|p| p.len())),
        }
    }

    #[test]
    fn jit_page_built_on_one_thread_executes_on_another() {
        // Send + cross-thread execution: validates the icache flush and the
        // claim that W^X toggling is confined to the constructing thread.
        let page = JitPage::new(&ADD_CODE).unwrap();
        let handle = std::thread::spawn(move || {
            let add = unsafe { page.as_fn_i64_i64() };
            add(20, 22)
        });
        assert_eq!(handle.join().unwrap(), 42);
    }

    #[test]
    fn jit_many_threads_create_write_execute_drop() {
        // W^X per-thread toggling must not interfere across threads.
        let handles: Vec<_> = (0..8)
            .map(|k| {
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        let page = JitPage::new(&ADD_CODE).unwrap();
                        let add = unsafe { page.as_fn_i64_i64() };
                        assert_eq!(add(k, 1), k + 1);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn jit_create_and_drop_many_pages() {
        // munmap correctness smoke: leaking mappings would exhaust the address
        // space quota long before 1000 iterations of page-sized maps.
        for _ in 0..1000 {
            let page = JitPage::new(&ADD_CODE).unwrap();
            let add = unsafe { page.as_fn_i64_i64() };
            assert_eq!(add(1, 2), 3);
        }
    }

    // ---- F2: stencil table introspection ------------------------------------

    fn stencil(name: &str) -> &'static Stencil {
        super::STENCILS
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("stencil '{name}' missing from table"))
    }

    #[test]
    fn stencil_table_contains_the_full_cps_set() {
        for name in [
            "logos_stencil_const",
            "logos_stencil_addi",
            "logos_stencil_subi",
            "logos_stencil_muli",
            "logos_stencil_lti",
            "logos_stencil_branch_if",
            "logos_stencil_return",
            "logos_stencil_add",
        ] {
            let st = stencil(name);
            assert!(!st.code.is_empty(), "{name} has no code");
        }
    }

    #[test]
    fn checked_div_mod_stencils_have_two_distinct_cont_holes() {
        for name in ["logos_stencil_divi_checked", "logos_stencil_modi_checked"] {
            let st = stencil(name);
            let conts: std::collections::HashSet<_> = st
                .relocs
                .iter()
                .filter_map(|r| match r.target {
                    HoleId::Cont(n) => Some(n),
                    _ => None,
                })
                .collect();
            assert_eq!(
                conts,
                [0u8, 1u8].into_iter().collect(),
                "{name} must expose both the success and side-exit continuations"
            );
        }
    }

    #[test]
    fn deopt_stencil_has_const_hole_and_no_continuations() {
        let st = stencil("logos_stencil_deopt");
        assert!(
            st.relocs.iter().any(|r| matches!(r.target, HoleId::ConstI64(0))),
            "deopt stencil lacks its status-cell address hole: {:?}",
            st.relocs
        );
        assert!(
            !st.relocs.iter().any(|r| matches!(r.target, HoleId::Cont(_))),
            "deopt is a terminal — it must not continue anywhere"
        );
    }

    #[test]
    fn checked_div_chain_computes_and_side_exits() {
        use crate::jit::{compile_straightline, ChainOutcome, MicroOp};
        // frame[2] = frame[0] / frame[1]; return frame[2].
        let prog = [
            MicroOp::Div { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Return { src: 2 },
        ];
        let chain = compile_straightline(&prog).expect("compile");
        let mut frame = [40i64, 5, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(8));
        // The kernel's locked wrapping edge: i64::MIN / -1 = i64::MIN.
        let mut frame = [i64::MIN, -1, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(i64::MIN));
        // Zero divisor: side exit, then the NEXT run is clean (cell resets).
        let mut frame = [40i64, 0, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Deopt(1));
        let mut frame = [40i64, 4, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(10));
    }

    #[test]
    fn checked_mod_chain_computes_and_side_exits() {
        use crate::jit::{compile_straightline, ChainOutcome, MicroOp};
        let prog = [
            MicroOp::Mod { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Return { src: 2 },
        ];
        let chain = compile_straightline(&prog).expect("compile");
        let mut frame = [43i64, 5, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(3));
        let mut frame = [i64::MIN, -1, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(0));
        let mut frame = [-7i64, 2, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(-1));
        let mut frame = [43i64, 0, 0];
        assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Deopt(1));
    }

    /// A float `Move` whose SOURCE and DESTINATION are both XMM-pinned must move
    /// the value REGISTER-to-REGISTER (V_FMOV f→f), not round-trip a stale frame
    /// cell. The pinned-float emitter used the GP location (`loc`) for `Move`, so
    /// a float move between pins wrote/read frame bits that the register-resident
    /// value had never been spilled to — corrupting the loop-carried accumulator
    /// (the `Set zr to zr2` step of mandelbrot). Reproduces `x = x*x` over a pin:
    /// without the fix `x` keeps its prologue value (3.0) instead of 9.0.
    #[test]
    fn float_move_between_xmm_pins_threads_the_register() {
        use crate::jit::{compile_straightline_pinned_float, ChainOutcome, MicroOp};
        let prog = [
            MicroOp::MulF { dst: 2, lhs: 0, rhs: 0 }, // sq = x * x   (f-reg threaded)
            MicroOp::Move { dst: 0, src: 2 },          // x = sq       (f→f move)
            MicroOp::Return { src: 0 },
        ];
        let chain = compile_straightline_pinned_float(&prog, &[], &[0, 2], None).expect("compile");
        let mut frame = [3.0f64.to_bits() as i64, 0, 0];
        assert_eq!(
            chain.run_with_frame(&mut frame),
            ChainOutcome::Return(9.0f64.to_bits() as i64),
            "Move between XMM pins must copy the register value (9.0), not a stale frame cell"
        );
    }

    /// A float `Move` from a FRAME slot into an XMM pin (the common `Set acc to
    /// temp` shape where `temp` is unpinned) must reload the frame bits into the
    /// pinned register — otherwise the epilogue spills the pin's stale prologue
    /// value back over the result.
    #[test]
    fn float_move_frame_into_xmm_pin_reloads() {
        use crate::jit::{compile_straightline_pinned_float, ChainOutcome, MicroOp};
        let prog = [
            MicroOp::Move { dst: 0, src: 2 }, // x(pin) = temp(frame)
            MicroOp::Return { src: 0 },
        ];
        let chain = compile_straightline_pinned_float(&prog, &[], &[0], None).expect("compile");
        let mut frame = [1.0f64.to_bits() as i64, 0, 7.5f64.to_bits() as i64];
        assert_eq!(
            chain.run_with_frame(&mut frame),
            ChainOutcome::Return(7.5f64.to_bits() as i64),
            "Move from frame into an XMM pin must load 7.5 into the pin"
        );
    }

    #[test]
    fn return_stencil_has_zero_relocs() {
        assert!(stencil("logos_stencil_return").relocs.is_empty());
        assert!(stencil("logos_stencil_add").relocs.is_empty());
    }

    #[test]
    fn binop_stencils_have_exactly_one_cont_hole() {
        for name in ["logos_stencil_addi", "logos_stencil_subi", "logos_stencil_muli", "logos_stencil_lti"] {
            let st = stencil(name);
            let conts: Vec<_> = st
                .relocs
                .iter()
                .filter(|r| matches!(r.target, HoleId::Cont(0)))
                .collect();
            assert_eq!(conts.len(), 1, "{name}: expected exactly one Cont(0) reloc, got {:?}", st.relocs);
        }
    }

    #[test]
    fn const_stencil_has_const_hole_and_cont_hole() {
        let st = stencil("logos_stencil_const");
        assert!(
            st.relocs.iter().any(|r| matches!(r.target, HoleId::ConstI64(0))),
            "const stencil lacks its ConstI64 hole: {:?}",
            st.relocs
        );
        assert!(
            st.relocs.iter().any(|r| matches!(r.target, HoleId::Cont(0))),
            "const stencil lacks its continuation: {:?}",
            st.relocs
        );
    }

    #[test]
    fn branch_if_has_two_distinct_cont_holes() {
        let st = stencil("logos_stencil_branch_if");
        let has0 = st.relocs.iter().any(|r| matches!(r.target, HoleId::Cont(0)));
        let has1 = st.relocs.iter().any(|r| matches!(r.target, HoleId::Cont(1)));
        assert!(has0 && has1, "branch_if needs Cont(0) and Cont(1): {:?}", st.relocs);
    }

    #[test]
    fn every_reloc_is_in_bounds_and_aligned() {
        for st in super::STENCILS {
            for r in st.relocs {
                assert!((r.offset as usize) < st.code.len(), "{}: reloc out of bounds", st.name);
                #[cfg(target_arch = "aarch64")]
                assert_eq!(r.offset % 4, 0, "{}: arm64 reloc misaligned", st.name);
            }
        }
    }

    /// The headline: a stencil written in Rust, compiled to an object at build
    /// time, its machine code extracted, then JIT-loaded and executed.
    #[test]
    fn jit_extracted_rust_stencil_runs() {
        assert!(!super::ADD_STENCIL.is_empty(), "stencil extraction produced no bytes");
        let page = JitPage::new(super::ADD_STENCIL).expect("load extracted stencil");
        let add = unsafe { page.as_fn_i64_i64() };
        assert_eq!(add(3, 5), 8);
        assert_eq!(add(40, 2), 42);
        assert_eq!(add(-10, 7), -3);
    }

    /// The extracted stencil must be a relocation-free leaf — small, and ending
    /// in `ret` — so it can be copied verbatim into a JIT page. (The exact `add`
    /// encoding is rustc's choice: `add` is commutative, so `x0+x1` and `x1+x0`
    /// are both valid; we assert the structural property, not the bytes.)
    #[test]
    fn extracted_add_is_a_relocation_free_leaf() {
        #[cfg(target_arch = "aarch64")]
        {
            let bytes = super::ADD_STENCIL;
            // An `add x0, _, _` ; `ret` leaf is two 4-byte instructions.
            assert!(bytes.len() <= 16, "leaf stencil unexpectedly large: {} bytes", bytes.len());
            assert!(
                bytes.ends_with(&[0xc0, 0x03, 0x5f, 0xd6]),
                "stencil does not end in `ret`: {:02x?}",
                bytes
            );
            // First instruction is in the ADD (shifted register, 64-bit) family:
            // top byte 0x8b.
            assert_eq!(bytes[3], 0x8b, "first instruction is not a 64-bit ADD");
        }
    }
}
