//! io_uring Worker Thread
//!
//! Dedicated OS thread that owns the `IoUring` instance and processes file I/O
//! commands submitted from async Tokio tasks via crossbeam channel. Results
//! return through tokio oneshot senders.
//!
//! Hot-path operations (read, write, fsync) go through io_uring SQEs for true
//! kernel-async I/O with batched syscalls. Metadata operations (mkdir, rename,
//! unlink, readdir) execute synchronously on the worker thread — still better
//! than tokio's blocking pool because the worker is dedicated and contention-free.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};
use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use io_uring::{opcode, types, IoUring};
use tokio::sync::oneshot;

use super::{DirEntry, VfsError, VfsResult};

// ─── Constants ───────────────────────────────────────────────────────────────

const APPEND_POOL_SIZE: usize = 64;
const APPEND_BUF_SIZE: usize = 4096;

// ─── Commands ────────────────────────────────────────────────────────────────

pub(crate) enum UringCommand {
    Read {
        path: PathBuf,
        tx: oneshot::Sender<VfsResult<Vec<u8>>>,
    },
    ReadToString {
        path: PathBuf,
        tx: oneshot::Sender<VfsResult<String>>,
    },
    Write {
        path: PathBuf,
        data: Vec<u8>,
        tx: oneshot::Sender<VfsResult<()>>,
    },
    Append {
        path: PathBuf,
        data: Vec<u8>,
        tx: oneshot::Sender<VfsResult<()>>,
    },
    Exists {
        path: PathBuf,
        tx: oneshot::Sender<VfsResult<bool>>,
    },
    Remove {
        path: PathBuf,
        tx: oneshot::Sender<VfsResult<()>>,
    },
    CreateDirAll {
        path: PathBuf,
        tx: oneshot::Sender<VfsResult<()>>,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        tx: oneshot::Sender<VfsResult<()>>,
    },
    ListDir {
        path: PathBuf,
        tx: oneshot::Sender<VfsResult<Vec<DirEntry>>>,
    },
    Shutdown,
}

// ─── Read completion dispatch ────────────────────────────────────────────────

enum ReadCompletion {
    Bytes(oneshot::Sender<VfsResult<Vec<u8>>>),
    String(oneshot::Sender<VfsResult<String>>),
}

impl ReadCompletion {
    fn send_ok(self, data: Vec<u8>) {
        match self {
            ReadCompletion::Bytes(tx) => {
                let _ = tx.send(Ok(data));
            }
            ReadCompletion::String(tx) => {
                let result = String::from_utf8(data).map_err(|e| {
                    VfsError::IoError(io::Error::new(io::ErrorKind::InvalidData, e))
                });
                let _ = tx.send(result);
            }
        }
    }

    fn send_err(self, err: VfsError) {
        match self {
            ReadCompletion::Bytes(tx) => {
                let _ = tx.send(Err(err));
            }
            ReadCompletion::String(tx) => {
                let _ = tx.send(Err(err));
            }
        }
    }
}

// ─── In-flight operation state machine ───────────────────────────────────────

enum InflightOp {
    Read {
        fd: RawFd,
        buf: Vec<u8>,
        completion: ReadCompletion,
    },
    AtomicWrite {
        fd: RawFd,
        buf: Vec<u8>, // Must stay alive — io_uring holds a raw pointer
        temp_path: PathBuf,
        dest_path: PathBuf,
        tx: oneshot::Sender<VfsResult<()>>,
    },
    AtomicWriteFsync {
        fd: RawFd,
        temp_path: PathBuf,
        dest_path: PathBuf,
        tx: oneshot::Sender<VfsResult<()>>,
    },
    AppendWrite {
        fd: RawFd,
        buf: Vec<u8>, // Must stay alive — io_uring holds a raw pointer
        tx: oneshot::Sender<VfsResult<()>>,
        pooled: bool,
    },
    AppendFsync {
        fd: RawFd,
        tx: oneshot::Sender<VfsResult<()>>,
    },
}

// ─── Buffer pool ─────────────────────────────────────────────────────────────

struct BufferPool {
    buffers: Vec<Vec<u8>>,
}

impl BufferPool {
    fn new() -> Self {
        let buffers = (0..APPEND_POOL_SIZE)
            .map(|_| Vec::with_capacity(APPEND_BUF_SIZE))
            .collect();
        Self { buffers }
    }

    fn acquire(&mut self, data: &[u8]) -> (Vec<u8>, bool) {
        if data.len() <= APPEND_BUF_SIZE {
            if let Some(mut buf) = self.buffers.pop() {
                buf.clear();
                buf.extend_from_slice(data);
                return (buf, true);
            }
        }
        (data.to_vec(), false)
    }

    fn release(&mut self, buf: Vec<u8>) {
        if self.buffers.len() < APPEND_POOL_SIZE && buf.capacity() >= APPEND_BUF_SIZE {
            self.buffers.push(buf);
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn close_fd(fd: RawFd) {
    // SAFETY: fd is valid and exclusively owned by us at this point.
    unsafe {
        drop(fs::File::from_raw_fd(fd));
    }
}

fn cqe_error(negative_result: i32) -> VfsError {
    VfsError::from(io::Error::from_raw_os_error(-negative_result))
}

/// Try to push an SQE, retrying once after submitting pending entries to free space.
///
/// # Safety
/// The SQE must reference valid memory that will outlive the io_uring operation.
unsafe fn push_sqe(ring: &mut IoUring, entry: &io_uring::squeue::Entry) -> bool {
    if ring.submission().push(entry).is_ok() {
        return true;
    }
    let _ = ring.submit();
    ring.submission().push(entry).is_ok()
}

// ─── Worker entry point ──────────────────────────────────────────────────────

pub(crate) fn run_worker(mut ring: IoUring, rx: Receiver<UringCommand>) {
    let mut inflight: HashMap<u64, InflightOp> = HashMap::new();
    let mut next_id: u64 = 1;
    let mut pool = BufferPool::new();

    loop {
        // Phase 1: Acquire at least one command.
        // Block if nothing in-flight; non-blocking poll otherwise.
        let first_cmd = if inflight.is_empty() {
            match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => break,
            }
        } else {
            rx.try_recv().ok()
        };

        if matches!(first_cmd, Some(UringCommand::Shutdown)) {
            drain_all_inflight(&mut ring, &mut inflight, &mut next_id, &mut pool);
            break;
        }

        if let Some(cmd) = first_cmd {
            dispatch_command(cmd, &mut ring, &mut inflight, &mut next_id, &mut pool);
        }

        // Phase 2: Batch — drain all pending commands non-blocking.
        loop {
            match rx.try_recv() {
                Ok(UringCommand::Shutdown) => {
                    drain_all_inflight(&mut ring, &mut inflight, &mut next_id, &mut pool);
                    return;
                }
                Ok(cmd) => {
                    dispatch_command(cmd, &mut ring, &mut inflight, &mut next_id, &mut pool);
                }
                Err(_) => break,
            }
        }

        // Phase 3: Submit all batched SQEs and wait for at least one completion.
        if !inflight.is_empty() {
            if ring.submit_and_wait(1).is_err() {
                continue; // Retry on EINTR / transient errors
            }
            process_completions(&mut ring, &mut inflight, &mut next_id, &mut pool);
        }
    }
}

// ─── Command dispatch ────────────────────────────────────────────────────────

fn dispatch_command(
    cmd: UringCommand,
    ring: &mut IoUring,
    inflight: &mut HashMap<u64, InflightOp>,
    next_id: &mut u64,
    pool: &mut BufferPool,
) {
    match cmd {
        UringCommand::Read { path, tx } => {
            prepare_read(ring, inflight, next_id, path, ReadCompletion::Bytes(tx));
        }
        UringCommand::ReadToString { path, tx } => {
            prepare_read(ring, inflight, next_id, path, ReadCompletion::String(tx));
        }
        UringCommand::Write { path, data, tx } => {
            prepare_atomic_write(ring, inflight, next_id, path, data, tx);
        }
        UringCommand::Append { path, data, tx } => {
            prepare_append(ring, inflight, next_id, pool, path, data, tx);
        }
        UringCommand::Exists { path, tx } => {
            let _ = tx.send(Ok(path.exists()));
        }
        UringCommand::Remove { path, tx } => {
            let _ = tx.send(fs::remove_file(&path).map_err(VfsError::from));
        }
        UringCommand::CreateDirAll { path, tx } => {
            let _ = tx.send(fs::create_dir_all(&path).map_err(VfsError::from));
        }
        UringCommand::Rename { from, to, tx } => {
            let _ = tx.send(fs::rename(&from, &to).map_err(VfsError::from));
        }
        UringCommand::ListDir { path, tx } => {
            let _ = tx.send(list_dir_sync(&path));
        }
        UringCommand::Shutdown => unreachable!("handled by caller"),
    }
}

// ─── SQE preparation ────────────────────────────────────────────────────────

fn prepare_read(
    ring: &mut IoUring,
    inflight: &mut HashMap<u64, InflightOp>,
    next_id: &mut u64,
    path: PathBuf,
    completion: ReadCompletion,
) {
    let metadata = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => {
            completion.send_err(VfsError::from(e));
            return;
        }
    };

    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            completion.send_err(VfsError::from(e));
            return;
        }
    };

    let fd = file.into_raw_fd();
    let len = metadata.len() as usize;
    let mut buf = vec![0u8; len];

    let id = *next_id;
    *next_id += 1;

    let sqe = opcode::Read::new(types::Fd(fd), buf.as_mut_ptr(), len as u32)
        .build()
        .user_data(id);

    // SAFETY: buf is stored in inflight and stays alive until CQE is processed.
    if unsafe { push_sqe(ring, &sqe) } {
        inflight.insert(id, InflightOp::Read { fd, buf, completion });
    } else {
        let result = fallback_read_and_close(fd);
        match result {
            Ok(data) => completion.send_ok(data),
            Err(e) => completion.send_err(e),
        }
    }
}

fn prepare_atomic_write(
    ring: &mut IoUring,
    inflight: &mut HashMap<u64, InflightOp>,
    next_id: &mut u64,
    dest_path: PathBuf,
    data: Vec<u8>,
    tx: oneshot::Sender<VfsResult<()>>,
) {
    if let Some(parent) = dest_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            let _ = tx.send(Err(VfsError::from(e)));
            return;
        }
    }

    let temp_path = dest_path.with_extension("tmp");

    let file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp_path)
    {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(Err(VfsError::from(e)));
            return;
        }
    };

    let fd = file.into_raw_fd();
    let len = data.len() as u32;

    let id = *next_id;
    *next_id += 1;

    let sqe = opcode::Write::new(types::Fd(fd), data.as_ptr(), len)
        .build()
        .user_data(id);

    // SAFETY: data is stored in inflight as `buf` and stays alive until CQE.
    if unsafe { push_sqe(ring, &sqe) } {
        inflight.insert(
            id,
            InflightOp::AtomicWrite {
                fd,
                buf: data,
                temp_path,
                dest_path,
                tx,
            },
        );
    } else {
        let _ = tx.send(fallback_atomic_write_and_close(fd, &data, &temp_path, &dest_path));
    }
}

fn prepare_append(
    ring: &mut IoUring,
    inflight: &mut HashMap<u64, InflightOp>,
    next_id: &mut u64,
    pool: &mut BufferPool,
    path: PathBuf,
    data: Vec<u8>,
    tx: oneshot::Sender<VfsResult<()>>,
) {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            let _ = tx.send(Err(VfsError::from(e)));
            return;
        }
    }

    let file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(Err(VfsError::from(e)));
            return;
        }
    };

    let fd = file.into_raw_fd();
    let (buf, pooled) = pool.acquire(&data);
    let len = buf.len() as u32;

    let id = *next_id;
    *next_id += 1;

    let sqe = opcode::Write::new(types::Fd(fd), buf.as_ptr(), len)
        .build()
        .user_data(id);

    // SAFETY: buf is stored in inflight and stays alive until CQE.
    if unsafe { push_sqe(ring, &sqe) } {
        inflight.insert(
            id,
            InflightOp::AppendWrite {
                fd,
                buf,
                tx,
                pooled,
            },
        );
    } else {
        let _ = tx.send(fallback_append_and_close(fd, &buf));
        if pooled {
            pool.release(buf);
        }
    }
}

// ─── CQE processing + state transitions ─────────────────────────────────────

fn process_completions(
    ring: &mut IoUring,
    inflight: &mut HashMap<u64, InflightOp>,
    next_id: &mut u64,
    pool: &mut BufferPool,
) {
    // Collect CQEs first to release the borrow on ring.completion(),
    // so we can push new SQEs during processing.
    let cqes: Vec<(u64, i32)> = ring
        .completion()
        .map(|cqe| (cqe.user_data(), cqe.result()))
        .collect();

    for (id, result) in cqes {
        let Some(op) = inflight.remove(&id) else {
            continue;
        };

        match op {
            InflightOp::Read {
                fd,
                mut buf,
                completion,
            } => {
                close_fd(fd);
                if result < 0 {
                    completion.send_err(cqe_error(result));
                } else {
                    buf.truncate(result as usize);
                    completion.send_ok(buf);
                }
            }

            InflightOp::AtomicWrite {
                fd,
                buf,
                temp_path,
                dest_path,
                tx,
            } => {
                drop(buf); // Buffer no longer needed — io_uring is done with it.
                if result < 0 {
                    close_fd(fd);
                    let _ = fs::remove_file(&temp_path);
                    let _ = tx.send(Err(cqe_error(result)));
                } else {
                    // Write succeeded → submit Fsync SQE.
                    let fsync_id = *next_id;
                    *next_id += 1;
                    let sqe = opcode::Fsync::new(types::Fd(fd))
                        .build()
                        .user_data(fsync_id);

                    // SAFETY: No buffer pointers — Fsync only references the fd.
                    if unsafe { push_sqe(ring, &sqe) } {
                        inflight.insert(
                            fsync_id,
                            InflightOp::AtomicWriteFsync {
                                fd,
                                temp_path,
                                dest_path,
                                tx,
                            },
                        );
                    } else {
                        let _ = tx.send(fallback_fsync_rename_and_close(
                            fd, &temp_path, &dest_path,
                        ));
                    }
                }
            }

            InflightOp::AtomicWriteFsync {
                fd,
                temp_path,
                dest_path,
                tx,
            } => {
                close_fd(fd);
                if result < 0 {
                    let _ = fs::remove_file(&temp_path);
                    let _ = tx.send(Err(cqe_error(result)));
                } else {
                    let _ =
                        tx.send(fs::rename(&temp_path, &dest_path).map_err(VfsError::from));
                }
            }

            InflightOp::AppendWrite {
                fd,
                buf,
                tx,
                pooled,
            } => {
                if pooled {
                    pool.release(buf);
                } else {
                    drop(buf);
                }

                if result < 0 {
                    close_fd(fd);
                    let _ = tx.send(Err(cqe_error(result)));
                } else {
                    let fsync_id = *next_id;
                    *next_id += 1;
                    let sqe = opcode::Fsync::new(types::Fd(fd))
                        .build()
                        .user_data(fsync_id);

                    // SAFETY: Fsync only references the fd.
                    if unsafe { push_sqe(ring, &sqe) } {
                        inflight.insert(fsync_id, InflightOp::AppendFsync { fd, tx });
                    } else {
                        let _ = tx.send(fallback_fsync_and_close(fd));
                    }
                }
            }

            InflightOp::AppendFsync { fd, tx } => {
                close_fd(fd);
                if result < 0 {
                    let _ = tx.send(Err(cqe_error(result)));
                } else {
                    let _ = tx.send(Ok(()));
                }
            }
        }
    }
}

// ─── Shutdown ────────────────────────────────────────────────────────────────

fn drain_all_inflight(
    ring: &mut IoUring,
    inflight: &mut HashMap<u64, InflightOp>,
    next_id: &mut u64,
    pool: &mut BufferPool,
) {
    while !inflight.is_empty() {
        if ring.submit_and_wait(1).is_err() {
            // Unrecoverable — force-close all fds.
            for (_, op) in inflight.drain() {
                match op {
                    InflightOp::Read { fd, .. }
                    | InflightOp::AtomicWrite { fd, .. }
                    | InflightOp::AtomicWriteFsync { fd, .. }
                    | InflightOp::AppendWrite { fd, .. }
                    | InflightOp::AppendFsync { fd, .. } => close_fd(fd),
                }
            }
            break;
        }
        process_completions(ring, inflight, next_id, pool);
    }
}

// ─── Sync fallbacks (ring full) ──────────────────────────────────────────────

fn fallback_read_and_close(fd: RawFd) -> VfsResult<Vec<u8>> {
    use std::io::Read;
    // SAFETY: fd is valid and exclusively owned by us.
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(VfsError::from)?;
    // file drops here, closing fd
    Ok(data)
}

fn fallback_atomic_write_and_close(
    fd: RawFd,
    data: &[u8],
    temp_path: &Path,
    dest_path: &Path,
) -> VfsResult<()> {
    use std::io::Write;
    // SAFETY: fd is valid and exclusively owned by us.
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    if let Err(e) = file.write_all(data) {
        drop(file);
        let _ = fs::remove_file(temp_path);
        return Err(VfsError::from(e));
    }
    if let Err(e) = file.sync_all() {
        drop(file);
        let _ = fs::remove_file(temp_path);
        return Err(VfsError::from(e));
    }
    drop(file); // close before rename
    fs::rename(temp_path, dest_path).map_err(VfsError::from)
}

fn fallback_append_and_close(fd: RawFd, data: &[u8]) -> VfsResult<()> {
    use std::io::Write;
    // SAFETY: fd is valid and exclusively owned by us.
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    file.write_all(data).map_err(VfsError::from)?;
    file.sync_all().map_err(VfsError::from)?;
    // file drops here, closing fd
    Ok(())
}

fn fallback_fsync_rename_and_close(
    fd: RawFd,
    temp_path: &Path,
    dest_path: &Path,
) -> VfsResult<()> {
    // SAFETY: fd is valid and exclusively owned by us.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    if let Err(e) = file.sync_all() {
        drop(file);
        let _ = fs::remove_file(temp_path);
        return Err(VfsError::from(e));
    }
    drop(file); // close before rename
    fs::rename(temp_path, dest_path).map_err(VfsError::from)
}

fn fallback_fsync_and_close(fd: RawFd) -> VfsResult<()> {
    // SAFETY: fd is valid and exclusively owned by us.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let result = file.sync_all().map_err(VfsError::from);
    // file drops here, closing fd
    result
}

fn list_dir_sync(path: &Path) -> VfsResult<Vec<DirEntry>> {
    let mut entries = Vec::new();
    let read_dir = fs::read_dir(path).map_err(VfsError::from)?;

    for entry_result in read_dir {
        let entry = entry_result.map_err(VfsError::from)?;
        let metadata = entry.metadata().map_err(VfsError::from)?;
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_directory: metadata.is_dir(),
        });
    }

    entries.sort_by(|a, b| match (a.is_directory, b.is_directory) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}
