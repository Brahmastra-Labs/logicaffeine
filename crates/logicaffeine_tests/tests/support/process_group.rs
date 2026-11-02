//! Ending a process group a test started, through the syscall and never the `kill` binary (see
//! `signal_blast_radius_lock`).

/// Send SIGKILL to every process in the group `pgid` leads. A `pgid` of 0 or 1 would name the caller's own group or
/// every process the caller may signal, so no signal is sent for either.
pub fn end_group(pgid: u32) {
    if pgid > 1 {
        unsafe { libc::kill(-(pgid as libc::pid_t), libc::SIGKILL) };
    }
}
