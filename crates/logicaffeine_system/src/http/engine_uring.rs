//! The thread-per-core io_uring HTTP engine — the nginx/seastar architecture
//! under a compiled LOGOS `Serve`.
//!
//! **Shards.** N independent shards (default: one per core; `LOGOS_HTTP_SHARDS`
//! overrides via [`super::engine::shard_count_from`]). Each shard is one OS
//! thread owning its OWN `SO_REUSEPORT` listener (the kernel load-balances
//! accepts across them), its OWN `io_uring` ring, and its OWN connection slab.
//! Nothing crosses shards: no locks, no work stealing, and a connection lives
//! and dies on the shard that accepted it.
//!
//! **Ring discipline.** Accepts use multishot accept (`AcceptMulti`) when the
//! kernel supports it (probed at bind; single-shot `Accept` re-armed per
//! completion otherwise). Receives land directly in each connection's REUSED
//! input buffer (its spare capacity — zero copies, zero per-request
//! allocations at steady state); sends drain a per-connection reused output
//! buffer with short-write continuation. Submission is batched: every loop
//! iteration drains completions, runs the protocol state machine, and submits
//! everything with ONE `io_uring_enter` (a wait timeout keeps shutdown
//! observable). Provided buffer rings (`IORING_OP_PROVIDE_BUFFERS`/buf_ring)
//! were considered and deliberately NOT used: at this crate version the
//! buf_ring surface is a raw unsafe API, and its win is memory footprint for
//! very large idle-connection counts — not throughput, which comes from the
//! syscall batching and thread-per-core layout shipped here.
//!
//! **Protocol.** The state machine speaks through the EXACT wire core the
//! tokio engine uses — `parse_request_consuming` (leftover bytes preserved for
//! pipelining), `request_wants_keep_alive`, `serialize_response_conn`, and the
//! shared `bad_request`/`internal_error` — so the two engines are
//! byte-identical on the wire by construction, and the engine differential
//! asserts it.
//!
//! **Safety invariant.** At most ONE io_uring op is in flight per connection,
//! and a connection's buffers are never touched (or freed) while its op is in
//! flight. Every `unsafe` block below leans on that invariant; teardown
//! cancels in-flight ops and drains them before buffers are dropped (and
//! deliberately LEAKS the buffers of any op a pathological kernel refuses to
//! cancel — a bounded leak beats a use-after-free).

use std::collections::VecDeque;
use std::io;
use std::mem::ManuallyDrop;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use io_uring::types::{Fd, SubmitArgs, Timespec};
use io_uring::{cqueue, opcode, IoUring};

use super::engine::{shard_count_from, StaticResponses};
use super::server::{
    bad_request, internal_error, read_error_response, try_upgrade, UpgradeHandler, Upgrades,
    MAX_INFLIGHT,
};
use super::wire::{
    parse_request_consuming, request_wants_keep_alive, serialize_response_conn, HttpRequest,
    HttpResponse, ParseError,
};

/// Bytes requested per receive: one op usually captures a whole request.
const RECV_CHUNK: usize = 16 * 1024;

/// How long one ring wait blocks before re-checking the shutdown flag.
const WAIT_TIMEOUT_NSEC: u32 = 100_000_000;

/// Teardown patience: drain attempts (at ~50ms each) before leaking buffers.
const TEARDOWN_TRIES: usize = 40;

/// The engine's tuning knobs. `max_inflight` is a GLOBAL budget split across
/// shards (per-shard cap = `ceil(max_inflight / shards)`, minimum 1), mirroring
/// the tokio engine's [`MAX_INFLIGHT`] connection bound.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    pub shards: usize,
    pub max_inflight: usize,
}

impl EngineConfig {
    /// The deployed shape: shards from `LOGOS_HTTP_SHARDS` (else one per
    /// core), the shared [`MAX_INFLIGHT`] budget.
    pub fn from_env() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        EngineConfig {
            shards: shard_count_from(
                std::env::var("LOGOS_HTTP_SHARDS").ok().as_deref(),
                parallelism,
            ),
            max_inflight: MAX_INFLIGHT,
        }
    }
}

/// Can a ring be set up on this box at all? The selection probe.
pub fn probe() -> io::Result<()> {
    IoUring::new(8).map(|_| ())
}

// ─── user_data encoding ──────────────────────────────────────────────────────
// [ 8 bits kind | 16 bits generation | 32 bits slot ]. The generation guards
// against a stale completion (post-cancel) touching a recycled slot.

const KIND_ACCEPT: u64 = 1;
const KIND_RECV: u64 = 2;
const KIND_SEND: u64 = 3;
const KIND_CANCEL: u64 = 4;

fn ud(kind: u64, gen: u16, slot: u32) -> u64 {
    (kind << 56) | ((gen as u64) << 32) | slot as u64
}

fn ud_kind(ud: u64) -> u64 {
    ud >> 56
}

fn ud_gen(ud: u64) -> u16 {
    (ud >> 32) as u16
}

fn ud_slot(ud: u64) -> u32 {
    ud as u32
}

const ACCEPT_UD: u64 = KIND_ACCEPT << 56;

// ─── Connection state machine ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    None,
    Recv,
    Send,
}

struct ConnState {
    fd: RawFd,
    /// Accumulated unparsed request bytes; receives append into its spare
    /// capacity, each parsed request drains exactly its frame (pipelined
    /// remainders stay), and the allocation is reused for the whole connection.
    inbuf: Vec<u8>,
    /// Serialized responses pending write — reused across requests.
    outbuf: Vec<u8>,
    out_pos: usize,
    close_after_write: bool,
    op: Op,
    /// The request just parsed asked to upgrade a registered path, and the handshake completed:
    /// the handler to hand the connection to and the `101` response to write first. The shard
    /// stops serving the connection here — see [`hand_off`].
    upgrade: Option<(UpgradeHandler, String)>,
    /// The request matched a registered upgrade path whose handshake cannot complete: the
    /// connection is dropped with no response, as the tokio engine drops it.
    refuse: bool,
}

impl ConnState {
    fn new(fd: RawFd) -> Self {
        ConnState {
            fd,
            inbuf: Vec::with_capacity(RECV_CHUNK),
            outbuf: Vec::new(),
            out_pos: 0,
            close_after_write: false,
            op: Op::None,
            upgrade: None,
            refuse: false,
        }
    }
}

struct Slot {
    gen: u16,
    conn: Option<ConnState>,
}

// ─── Shard ───────────────────────────────────────────────────────────────────

struct Shard<H: ?Sized> {
    /// Declared FIRST so it drops before the slab: the ring must be gone
    /// before any buffer an (impossible-to-cancel) op might reference.
    ring: IoUring,
    listener: TcpListener,
    slots: Vec<Slot>,
    free: Vec<u32>,
    live: usize,
    cap: usize,
    /// Accepted-but-parked fds while the shard is at its in-flight cap; the
    /// bound (`cap` more) mirrors "further connections wait in the OS backlog".
    parked: VecDeque<RawFd>,
    accept_armed: bool,
    accept_cooldown: bool,
    accept_multishot: bool,
    stopping: bool,
    ext_arg_ok: bool,
    handler: Arc<H>,
    statics: Option<Arc<StaticResponses>>,
    upgrades: Upgrades,
    /// The tokio runtime an upgraded connection is handed to — `Some` exactly when `upgrades`
    /// registers a path, captured at bind (see [`UringServer::bind_with_upgrades`]).
    runtime: Option<tokio::runtime::Handle>,
    shutdown: Arc<AtomicBool>,
}

/// Try to push an SQE, retrying once after a submit to free queue space.
///
/// # Safety
/// The SQE must reference memory that outlives the operation (the per-shard
/// one-op-per-connection invariant).
unsafe fn push_sqe(ring: &mut IoUring, entry: &io_uring::squeue::Entry) -> bool {
    if ring.submission().push(entry).is_ok() {
        return true;
    }
    let _ = ring.submit();
    ring.submission().push(entry).is_ok()
}

/// TCP_NODELAY on an accepted socket — request/response servers must not sit
/// behind Nagle. Borrow-only: the fd stays owned by the caller.
fn prepare_accepted_socket(fd: RawFd) -> io::Result<()> {
    // SAFETY: the fd is a valid connected socket; ManuallyDrop keeps this
    // temporary TcpStream from closing it.
    let stream = ManuallyDrop::new(unsafe { TcpStream::from_raw_fd(fd) });
    stream.set_nodelay(true)
}

fn close_fd(fd: RawFd) {
    // SAFETY: exclusively owned at the call sites (no op in flight).
    unsafe {
        libc::close(fd);
    }
}

/// Bind a TCP listener with `SO_REUSEPORT` (+`SO_REUSEADDR`) — the shard
/// listener std cannot express (the option must be set before `bind`).
fn bind_reuseport(addr: SocketAddr) -> io::Result<TcpListener> {
    let domain = match addr {
        SocketAddr::V4(_) => libc::AF_INET,
        SocketAddr::V6(_) => libc::AF_INET6,
    };
    // SAFETY: plain syscalls on a locally owned fd; every failure path closes it.
    unsafe {
        let fd = libc::socket(domain, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let close_on = |fd: RawFd, e: io::Error| -> io::Error {
            libc::close(fd);
            e
        };
        let one: libc::c_int = 1;
        for opt in [libc::SO_REUSEADDR, libc::SO_REUSEPORT] {
            if libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            ) != 0
            {
                return Err(close_on(fd, io::Error::last_os_error()));
            }
        }
        let mut storage: libc::sockaddr_storage = std::mem::zeroed();
        let len = match addr {
            SocketAddr::V4(v4) => {
                let sin = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in);
                sin.sin_family = libc::AF_INET as libc::sa_family_t;
                sin.sin_port = v4.port().to_be();
                sin.sin_addr = libc::in_addr {
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                };
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t
            }
            SocketAddr::V6(v6) => {
                let sin6 = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6);
                sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sin6.sin6_port = v6.port().to_be();
                sin6.sin6_addr = libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                };
                sin6.sin6_flowinfo = v6.flowinfo();
                sin6.sin6_scope_id = v6.scope_id();
                std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t
            }
        };
        if libc::bind(fd, &storage as *const _ as *const libc::sockaddr, len) != 0 {
            return Err(close_on(fd, io::Error::last_os_error()));
        }
        if libc::listen(fd, 1024) != 0 {
            return Err(close_on(fd, io::Error::last_os_error()));
        }
        Ok(TcpListener::from_raw_fd(fd))
    }
}

// ─── The server handle ───────────────────────────────────────────────────────

/// A running thread-per-core engine. Dropping it (or calling
/// [`UringServer::shutdown`]) stops every shard: in-flight ops are cancelled
/// and drained, sockets closed, threads joined.
pub struct UringServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl UringServer {
    /// Bind `addr` across `cfg.shards` `SO_REUSEPORT` listeners, stand up one
    /// ring per shard, and start serving. Every ring and listener is created
    /// HERE (not in the shard threads) so a kernel without io_uring fails the
    /// bind — and the caller falls back to tokio — rather than dying late.
    pub fn bind<H>(
        addr: &str,
        cfg: EngineConfig,
        handler: Arc<H>,
        statics: Option<Arc<StaticResponses>>,
    ) -> io::Result<UringServer>
    where
        H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized + 'static,
    {
        Self::bind_with_upgrades(
            addr,
            cfg,
            handler,
            statics,
            Arc::new(std::collections::BTreeMap::new()),
        )
    }

    /// [`UringServer::bind`] with a table of protocol-upgrade paths. A registered path is served
    /// by handing its connection to the tokio runtime the caller is running on, so a table with
    /// no runtime to hand to refuses the bind here — the caller falls back to the tokio engine
    /// loudly rather than serve a path it cannot finish.
    pub(super) fn bind_with_upgrades<H>(
        addr: &str,
        cfg: EngineConfig,
        handler: Arc<H>,
        statics: Option<Arc<StaticResponses>>,
        upgrades: Upgrades,
    ) -> io::Result<UringServer>
    where
        H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized + 'static,
    {
        let runtime = if upgrades.is_empty() {
            None
        } else {
            Some(tokio::runtime::Handle::try_current().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "an upgrade path is taken over on a tokio runtime, and none is running",
                )
            })?)
        };
        let sock_addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
        let shards = cfg.shards.max(1);
        let cap = cfg.max_inflight.max(1).div_ceil(shards).max(1);

        let first = bind_reuseport(sock_addr)?;
        let real_addr = first.local_addr()?;
        let mut listeners = vec![first];
        for _ in 1..shards {
            listeners.push(bind_reuseport(real_addr)?);
        }

        let entries = ((cap * 2 + 8).next_power_of_two() as u32).clamp(256, 4096);
        let mut rings = Vec::with_capacity(shards);
        for _ in 0..shards {
            rings.push(IoUring::new(entries)?);
        }

        // Probe once: multishot accept (kernel 5.19+), else single-shot re-arm.
        let accept_multishot = {
            let mut probe = io_uring::Probe::new();
            rings[0].submitter().register_probe(&mut probe).is_ok()
                && probe.is_supported(opcode::AcceptMulti::CODE)
        };

        let shutdown = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::with_capacity(shards);
        for (i, (ring, listener)) in rings.into_iter().zip(listeners).enumerate() {
            let shard = Shard {
                ring,
                listener,
                slots: Vec::new(),
                free: Vec::new(),
                live: 0,
                cap,
                parked: VecDeque::new(),
                accept_armed: false,
                accept_cooldown: false,
                accept_multishot,
                stopping: false,
                ext_arg_ok: true,
                handler: Arc::clone(&handler),
                statics: statics.clone(),
                upgrades: Arc::clone(&upgrades),
                runtime: runtime.clone(),
                shutdown: Arc::clone(&shutdown),
            };
            let thread = std::thread::Builder::new()
                .name(format!("logos-http-shard-{i}"))
                .spawn(move || shard_loop(shard))
                .map_err(|e| {
                    shutdown.store(true, Ordering::SeqCst);
                    e
                })?;
            threads.push(thread);
        }

        Ok(UringServer {
            addr: real_addr,
            shutdown,
            threads,
        })
    }

    /// The actual bound address (resolves an ephemeral `:0` port).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop every shard and join their threads. Idempotent.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for UringServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ─── Shard loop ──────────────────────────────────────────────────────────────

fn shard_loop<H>(mut sh: Shard<H>)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    arm_accept(&mut sh);
    let timeout = Timespec::new().sec(0).nsec(WAIT_TIMEOUT_NSEC);
    loop {
        submit_and_wait_briefly(&mut sh, &timeout);
        let cqes: Vec<(u64, i32, u32)> = sh
            .ring
            .completion()
            .map(|c| (c.user_data(), c.result(), c.flags()))
            .collect();
        for (user_data, result, flags) in cqes {
            handle_cqe(&mut sh, user_data, result, flags);
        }
        if sh.shutdown.load(Ordering::SeqCst) {
            teardown(&mut sh);
            return;
        }
        maintain_accept(&mut sh);
    }
}

/// One batched `io_uring_enter`: submit everything queued and wait for at
/// least one completion, bounded by `timeout` so the shutdown flag is
/// observed. Falls back to an unbounded wait if the kernel predates
/// `EXT_ARG` (shutdown then rides the next event — degraded, never wrong).
fn submit_and_wait_briefly<H: ?Sized>(sh: &mut Shard<H>, timeout: &Timespec) {
    let result = if sh.ext_arg_ok {
        let args = SubmitArgs::new().timespec(timeout);
        sh.ring.submitter().submit_with_args(1, &args)
    } else {
        sh.ring.submit_and_wait(1)
    };
    match result {
        Ok(_) => {}
        Err(e) => match e.raw_os_error() {
            Some(libc::ETIME) | Some(libc::EINTR) | Some(libc::EBUSY) => {}
            Some(libc::EINVAL) if sh.ext_arg_ok => {
                sh.ext_arg_ok = false;
            }
            _ => {}
        },
    }
}

fn arm_accept<H: ?Sized>(sh: &mut Shard<H>) {
    let listener_fd = sh.listener.as_raw_fd();
    let entry = if sh.accept_multishot {
        opcode::AcceptMulti::new(Fd(listener_fd))
            .build()
            .user_data(ACCEPT_UD)
    } else {
        opcode::Accept::new(Fd(listener_fd), std::ptr::null_mut(), std::ptr::null_mut())
            .build()
            .user_data(ACCEPT_UD)
    };
    // SAFETY: accept SQEs reference no user memory beyond the listener fd.
    sh.accept_armed = unsafe { push_sqe(&mut sh.ring, &entry) };
}

/// Re-arm accepting when there is room: below the cap, park queue not full,
/// not stopping, and not cooling down after an accept error.
fn maintain_accept<H: ?Sized>(sh: &mut Shard<H>) {
    if sh.accept_cooldown {
        sh.accept_cooldown = false;
        return;
    }
    if !sh.stopping && !sh.accept_armed && sh.live < sh.cap && sh.parked.len() < sh.cap {
        arm_accept(sh);
    }
}

fn handle_cqe<H>(sh: &mut Shard<H>, user_data: u64, result: i32, flags: u32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    match ud_kind(user_data) {
        KIND_ACCEPT => on_accept(sh, result, flags),
        KIND_RECV => on_recv(sh, ud_gen(user_data), ud_slot(user_data), result),
        KIND_SEND => on_send(sh, ud_gen(user_data), ud_slot(user_data), result),
        _ => {} // a cancel's own completion carries nothing to do
    }
}

fn on_accept<H>(sh: &mut Shard<H>, result: i32, flags: u32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    // A multishot accept keeps delivering while F_MORE is set; its final CQE
    // (or any single-shot CQE) disarms.
    if !(sh.accept_multishot && cqueue::more(flags)) {
        sh.accept_armed = false;
    }
    if result < 0 {
        // ECANCELED is the cap/teardown cancel coming home; anything else
        // (EMFILE…) cools accepting down for one loop so errors cannot spin.
        if result != -libc::ECANCELED {
            sh.accept_cooldown = true;
        }
        return;
    }
    let fd = result as RawFd;
    if sh.stopping {
        close_fd(fd);
        return;
    }
    let _ = prepare_accepted_socket(fd);
    if sh.live >= sh.cap {
        sh.parked.push_back(fd);
        if sh.parked.len() >= sh.cap && sh.accept_armed {
            // Park queue full: stop accepting (the OS backlog holds the rest),
            // exactly the tokio engine's pause-at-the-cap discipline.
            let cancel =
                opcode::AsyncCancel::new(ACCEPT_UD)
                    .build()
                    .user_data(ud(KIND_CANCEL, 0, 0));
            // SAFETY: cancel SQEs reference no user memory.
            unsafe { push_sqe(&mut sh.ring, &cancel) };
        }
    } else {
        open_conn(sh, fd);
    }
}

fn open_conn<H>(sh: &mut Shard<H>, fd: RawFd)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let slot = match sh.free.pop() {
        Some(s) => s,
        None => {
            sh.slots.push(Slot { gen: 0, conn: None });
            (sh.slots.len() - 1) as u32
        }
    };
    sh.slots[slot as usize].conn = Some(ConnState::new(fd));
    sh.live += 1;
    arm_recv(sh, slot);
}

fn close_conn<H>(sh: &mut Shard<H>, slot: u32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let s = &mut sh.slots[slot as usize];
    if let Some(conn) = s.conn.take() {
        debug_assert_eq!(
            conn.op,
            Op::None,
            "closing a connection with an op in flight"
        );
        close_fd(conn.fd);
        s.gen = s.gen.wrapping_add(1);
        sh.free.push(slot);
        sh.live -= 1;
    }
    admit_parked(sh);
}

/// Open parked connections into the slots a closed or handed-off connection freed.
fn admit_parked<H>(sh: &mut Shard<H>)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    if !sh.stopping {
        while sh.live < sh.cap {
            match sh.parked.pop_front() {
                Some(fd) => open_conn(sh, fd),
                None => break,
            }
        }
    }
}

/// Hand an upgraded connection over: it leaves the shard (no op is in flight — it has just
/// completed), and on the tokio runtime its socket writes whatever responses were pending, then
/// the `101`, and then belongs to the upgrade's handler along with any bytes already read past
/// the upgrade request.
fn hand_off<H>(sh: &mut Shard<H>, slot: u32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let s = &mut sh.slots[slot as usize];
    let Some(mut conn) = s.conn.take() else {
        return;
    };
    debug_assert_eq!(
        conn.op,
        Op::None,
        "handing off a connection with an op in flight"
    );
    s.gen = s.gen.wrapping_add(1);
    sh.free.push(slot);
    sh.live -= 1;
    let (upgrade, response) = conn
        .upgrade
        .take()
        .expect("a handed-off connection carries its upgrade");
    let mut pending = conn.outbuf.split_off(conn.out_pos);
    pending.extend_from_slice(response.as_bytes());
    let prefix = std::mem::take(&mut conn.inbuf);
    let fd = conn.fd;
    match &sh.runtime {
        Some(runtime) => {
            runtime.spawn(async move {
                // SAFETY: the connection left the shard above with no op in flight, so nothing
                // else reads, writes or closes this fd; the stream now owns it.
                let socket = unsafe { TcpStream::from_raw_fd(fd) };
                if socket.set_nonblocking(true).is_err() {
                    return;
                }
                let Ok(mut stream) = tokio::net::TcpStream::from_std(socket) else {
                    return;
                };
                if tokio::io::AsyncWriteExt::write_all(&mut stream, &pending)
                    .await
                    .is_err()
                {
                    return;
                }
                upgrade(stream, prefix).await;
            });
        }
        None => close_fd(fd),
    }
    admit_parked(sh);
}

fn arm_recv<H>(sh: &mut Shard<H>, slot: u32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let gen = sh.slots[slot as usize].gen;
    let conn = sh.slots[slot as usize]
        .conn
        .as_mut()
        .expect("arm_recv on a live slot");
    let len = conn.inbuf.len();
    conn.inbuf.reserve(RECV_CHUNK);
    // SAFETY: the receive writes into inbuf's reserved spare capacity; the
    // one-op invariant means inbuf is not touched (so never reallocated or
    // freed) until this op's completion is handled.
    let ptr = unsafe { conn.inbuf.as_mut_ptr().add(len) };
    let entry = opcode::Recv::new(Fd(conn.fd), ptr, RECV_CHUNK as u32)
        .build()
        .user_data(ud(KIND_RECV, gen, slot));
    conn.op = Op::Recv;
    if !unsafe { push_sqe(&mut sh.ring, &entry) } {
        // Queue exhaustion: never leave a connection silently unarmed.
        sh.slots[slot as usize].conn.as_mut().unwrap().op = Op::None;
        close_conn(sh, slot);
    }
}

fn arm_send<H>(sh: &mut Shard<H>, slot: u32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let gen = sh.slots[slot as usize].gen;
    let conn = sh.slots[slot as usize]
        .conn
        .as_mut()
        .expect("arm_send on a live slot");
    let remaining = conn.outbuf.len() - conn.out_pos;
    // SAFETY: outbuf is stable (one-op invariant) until this completion.
    let ptr = unsafe { conn.outbuf.as_ptr().add(conn.out_pos) };
    let entry = opcode::Send::new(Fd(conn.fd), ptr, remaining as u32)
        .build()
        .user_data(ud(KIND_SEND, gen, slot));
    conn.op = Op::Send;
    if !unsafe { push_sqe(&mut sh.ring, &entry) } {
        sh.slots[slot as usize].conn.as_mut().unwrap().op = Op::None;
        close_conn(sh, slot);
    }
}

/// A completion for a slot that has since been recycled (generation mismatch)
/// or emptied is stale — post-cancel noise, safely ignored.
fn live_conn<'a, H: ?Sized>(
    sh: &'a mut Shard<H>,
    gen: u16,
    slot: u32,
) -> Option<&'a mut ConnState> {
    let s = sh.slots.get_mut(slot as usize)?;
    if s.gen != gen {
        return None;
    }
    s.conn.as_mut()
}

fn on_recv<H>(sh: &mut Shard<H>, gen: u16, slot: u32, result: i32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let stopping = sh.stopping;
    let handler = Arc::clone(&sh.handler);
    let statics = sh.statics.clone();
    let upgrades = Arc::clone(&sh.upgrades);
    let Some(conn) = live_conn(sh, gen, slot) else {
        return;
    };
    conn.op = Op::None;
    if stopping {
        close_conn(sh, slot);
        return;
    }
    if result < 0 {
        if result == -libc::EINTR || result == -libc::EAGAIN {
            arm_recv(sh, slot);
        } else {
            close_conn(sh, slot);
        }
        return;
    }
    let n = result as usize;
    if n == 0 {
        // EOF. A clean end between requests closes; leftover bytes are a
        // truncated request — the tokio engine's 400-and-close law.
        if conn.inbuf.is_empty() {
            close_conn(sh, slot);
        } else {
            conn.inbuf.clear();
            conn.outbuf
                .extend_from_slice(&serialize_response_conn(&bad_request(), false));
            conn.close_after_write = true;
            arm_send(sh, slot);
        }
        return;
    }
    // SAFETY: the completed receive wrote exactly `n` bytes into the spare
    // capacity reserved by arm_recv.
    unsafe { conn.inbuf.set_len(conn.inbuf.len() + n) };
    process_input(conn, &*handler, statics.as_deref(), &upgrades);
    continue_conn(sh, slot);
}

/// After a batch of requests: drop a refused upgrade, hand an accepted one over, or go on
/// writing what was answered or reading what comes next.
fn continue_conn<H>(sh: &mut Shard<H>, slot: u32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let conn = sh.slots[slot as usize]
        .conn
        .as_ref()
        .expect("continuing a live connection");
    if conn.refuse {
        close_conn(sh, slot);
    } else if conn.upgrade.is_some() {
        hand_off(sh, slot);
    } else if !conn.outbuf.is_empty() {
        arm_send(sh, slot);
    } else {
        arm_recv(sh, slot);
    }
}

fn on_send<H>(sh: &mut Shard<H>, gen: u16, slot: u32, result: i32)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    let stopping = sh.stopping;
    let handler = Arc::clone(&sh.handler);
    let statics = sh.statics.clone();
    let upgrades = Arc::clone(&sh.upgrades);
    let Some(conn) = live_conn(sh, gen, slot) else {
        return;
    };
    conn.op = Op::None;
    if stopping || result < 0 {
        close_conn(sh, slot);
        return;
    }
    conn.out_pos += result as usize;
    if conn.out_pos < conn.outbuf.len() {
        // Short write: continue from where the kernel stopped.
        arm_send(sh, slot);
        return;
    }
    conn.outbuf.clear();
    conn.out_pos = 0;
    if conn.close_after_write {
        close_conn(sh, slot);
        return;
    }
    // Pipelined bytes that arrived with an earlier request may already hold
    // complete requests — serve them before waiting on the socket again.
    process_input(conn, &*handler, statics.as_deref(), &upgrades);
    continue_conn(sh, slot);
}

/// Drain every complete request out of `inbuf` into serialized responses in
/// `outbuf` — the protocol law, shared verbatim with the tokio engine: a request
/// for a registered upgrade path ends the batch (its connection is handed over,
/// or dropped when the handshake cannot complete — [`try_upgrade`] decides for
/// both engines), the static table answers bare GETs it knows, the handler
/// (panic-shielded to the shared 500) answers the rest, `request_wants_keep_alive`
/// decides the disposition, a parse error answers the shared 400 and closes.
fn process_input<H>(
    conn: &mut ConnState,
    handler: &H,
    statics: Option<&StaticResponses>,
    upgrades: &std::collections::BTreeMap<String, UpgradeHandler>,
) where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    loop {
        match parse_request_consuming(&conn.inbuf) {
            Ok((req, used)) => {
                conn.inbuf.drain(..used);
                match try_upgrade(&req, upgrades) {
                    Some(Ok((response, upgrade))) => {
                        conn.upgrade = Some((upgrade, response));
                        return;
                    }
                    Some(Err(())) => {
                        conn.refuse = true;
                        return;
                    }
                    None => {}
                }
                let keep_alive = request_wants_keep_alive(&req);
                match statics.and_then(|s| s.wire_frame(&req, keep_alive)) {
                    Some(frame) => conn.outbuf.extend_from_slice(frame),
                    None => {
                        let resp =
                            match crate::failure::caught(std::panic::AssertUnwindSafe(|| {
                                handler(req)
                            })) {
                                Ok(resp) => resp,
                                Err(_) => internal_error(),
                            };
                        conn.outbuf
                            .extend_from_slice(&serialize_response_conn(&resp, keep_alive));
                    }
                }
                if !keep_alive {
                    conn.close_after_write = true;
                    conn.inbuf.clear();
                    return;
                }
            }
            Err(ParseError::Incomplete) => return,
            Err(e) => {
                // Same defined refusal law as the tokio engine: an oversized
                // body/header block is a `413`, any other parse failure a `400`.
                conn.outbuf
                    .extend_from_slice(&serialize_response_conn(&read_error_response(&e), false));
                conn.close_after_write = true;
                conn.inbuf.clear();
                return;
            }
        }
    }
}

/// Stop the shard: cancel the accept and every in-flight op, drain the ring
/// until nothing references connection memory, then close what remains. Any
/// op the kernel refuses to cancel within the deadline gets its buffers
/// deliberately leaked (never freed under a live op).
fn teardown<H>(sh: &mut Shard<H>)
where
    H: Fn(HttpRequest) -> HttpResponse + Send + Sync + ?Sized,
{
    sh.stopping = true;
    let mut cancels = Vec::new();
    if sh.accept_armed {
        cancels.push(
            opcode::AsyncCancel::new(ACCEPT_UD)
                .build()
                .user_data(ud(KIND_CANCEL, 0, 0)),
        );
    }
    for (i, s) in sh.slots.iter().enumerate() {
        if let Some(conn) = &s.conn {
            let kind = match conn.op {
                Op::Recv => KIND_RECV,
                Op::Send => KIND_SEND,
                Op::None => continue,
            };
            cancels.push(
                opcode::AsyncCancel::new(ud(kind, s.gen, i as u32))
                    .build()
                    .user_data(ud(KIND_CANCEL, 0, 0)),
            );
        }
    }
    for entry in &cancels {
        // SAFETY: cancel SQEs reference no user memory.
        unsafe { push_sqe(&mut sh.ring, entry) };
    }

    let timeout = Timespec::new().sec(0).nsec(50_000_000);
    for _ in 0..TEARDOWN_TRIES {
        let inflight = sh
            .slots
            .iter()
            .any(|s| s.conn.as_ref().is_some_and(|c| c.op != Op::None));
        if !inflight {
            break;
        }
        submit_and_wait_briefly(sh, &timeout);
        let cqes: Vec<(u64, i32, u32)> = sh
            .ring
            .completion()
            .map(|c| (c.user_data(), c.result(), c.flags()))
            .collect();
        for (user_data, result, flags) in cqes {
            handle_cqe(sh, user_data, result, flags);
        }
    }

    for fd in sh.parked.drain(..) {
        close_fd(fd);
    }
    for slot in &mut sh.slots {
        if let Some(mut conn) = slot.conn.take() {
            close_fd(conn.fd);
            if conn.op != Op::None {
                // The kernel still holds this op: leak the buffers it may
                // write into rather than free them under it.
                std::mem::forget(std::mem::take(&mut conn.inbuf));
                std::mem::forget(std::mem::take(&mut conn.outbuf));
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_system/http/engine_uring/tests.rs"]
mod tests;
