// The copy-and-patch JIT extracts stencils from rustc's tail-call codegen, which
// only holds under the x86_64 System V ABI (Linux + macOS). On Windows x64 (MS
// ABI) and aarch64, rustc emits a plain `call` at continuation sites, so the
// stencil chain would grow the stack — those targets run the bytecode VM instead
// (this crate compiles to nothing there, exactly like wasm32).
#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
#![doc = include_str!("../README.md")]

use std::fmt;
use std::mem;

pub mod buffer;
pub mod jit;
pub mod patch;
#[cfg(target_arch = "x86_64")]
pub mod regalloc;
pub mod segv_trace;
mod stencil_model;
pub mod vectorize;
#[cfg(target_arch = "x86_64")]
pub mod x64asm;
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
        Ok(JitPage {
            ptr,
            code_len: code.len(),
            alloc_len,
        })
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
    pub fn with_layout(len: usize, fill: impl FnOnce(u64) -> Vec<u8>) -> Result<JitPage, JitError> {
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
        Ok(JitPage {
            ptr,
            code_len: len,
            alloc_len,
        })
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_forge/tests.rs"]
mod tests;
