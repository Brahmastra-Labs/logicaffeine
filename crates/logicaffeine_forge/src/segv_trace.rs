//! Diagnostic SIGSEGV/SIGBUS tracer for root-causing faults inside JIT'd code.
//!
//! Installed only when `LOGOS_SEGV_TRAP` is set. On a fault it writes the
//! faulting data address (`si_addr`) and the instruction pointer (RIP) to
//! stderr, then restores the default handler and re-raises so the process
//! still dies with the original signal. The faulting *address value* is the
//! tell: a small/odd value or a recognizable float bit-pattern (e.g.
//! `0x3ff0000000000000` = `1.0`) means a non-pointer was dereferenced.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn install() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    if std::env::var_os("LOGOS_SEGV_TRAP").is_none() {
        return;
    }
    ONCE.call_once(|| unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler as usize;
        sa.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::null_mut());
    });
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn install() {}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
extern "C" fn handler(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    ucontext: *mut libc::c_void,
) {
    unsafe {
        // si_addr lives at offset 16 of siginfo_t on x86-64 Linux (after
        // si_signo/si_errno/si_code = 3 * i32 + padding to 8).
        let addr_ptr = (info as *const u8).add(16) as *const usize;
        let fault_addr = addr_ptr.read_unaligned();
        // glibc x86-64: ucontext_t.uc_mcontext.gregs starts at offset 40.
        // gregs index order: R8,R9,R10,R11,R12,R13,R14,R15,RDI,RSI,RBP,RBX,
        // RDX,RAX,RCX,RSP,RIP (0..16).
        let uc = ucontext as *const u8;
        let greg = |i: usize| (uc.add(40 + i * 8) as *const i64).read_unaligned();
        let (rdi, rsi, rdx, rax, rcx, rip) =
            (greg(8), greg(9), greg(12), greg(13), greg(14), greg(16));
        let r8 = greg(0);
        let r9 = greg(1);
        let rbx = greg(11);
        // Faulting instruction bytes (rip is in an executable JIT page).
        let mut code = [0u8; 16];
        for (k, b) in code.iter_mut().enumerate() {
            *b = (rip as *const u8).add(k).read();
        }
        let msg = format!(
            "\n*** LOGOS_SEGV_TRAP: sig {sig} fault=0x{fault_addr:016x} rip=0x{rip:016x}\n    rdi(base)=0x{rdi:016x} rsi(sp)=0x{rsi:016x} rax=0x{rax:016x} rdx=0x{rdx:016x} rcx=0x{rcx:016x}\n    r8=0x{r8:016x} r9=0x{r9:016x} rbx=0x{rbx:016x}\n    code@rip={code:02x?} ***\n"
        );
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        // Restore default and re-raise so we still die with the real signal.
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(sig, &sa, std::ptr::null_mut());
        libc::raise(sig);
    }
}
