# OS_SUPER.md — booting Logos as an operating system

Standing reference for the "Logos OS" question: *how far are we from booting our own operating system, what does an OS actually consist of, what is state-of-the-art, and what would we need?* We use **Linux as the yardstick** — the most complete worked example of "everything an OS does" — not as a thing to clone. Companion to `QUANTUM_MAP.md` and `FINISH_INTERPRETER.md`.

---

## 0. Thesis

An operating system is **what runs when nothing runs underneath it.** The defining move is owning the most-privileged level — ring 0 on x86, EL1 on ARM — being the first code firmware jumps to, bringing the machine up, and mediating all hardware from there.

Keep two axes separate; conflating them is what makes "build an OS" sound impossible:

1. **Kernel architecture** — monolithic / microkernel / unikernel / exokernel / language-based. *How* the OS is structured.
2. **Deployment substrate** — hypervisor (VM) / bare-metal physical / browser-WASM. *What it boots on.* This axis, not the first, dominates the driver cost.

The surprising result of the gap analysis (§4): Logos's missing slice is **small on a hypervisor and across-the-board on bare metal**, and the runtime it already has is shaped like a unikernel kernel. Logos already owns the hard, exotic part of the stack — source → executing machine code (AOT `Logos→Rust→LLVM→native`, plus a copy-patch JIT that emits x64/aarch64 and mmaps executable pages), a deterministic scheduler, a value heap, concurrency and networking semantics, and a proof kernel. What it lacks is the *floor*: boot, a `no_std` codegen profile, an allocator/pager, interrupts/timer, and device drivers.

---

## 1. What actually makes something an operating system

The canonical responsibilities, each with *why it exists* and *what Logos has today*:

| Responsibility | What it is / why it exists | Logos today |
|---|---|---|
| **Boot / bring-up** | Firmware (BIOS/UEFI on x86; device-tree/PSCI on ARM; iBoot on Apple) hands control to your entry point; you set CPU mode, stack, descriptor tables, enable MMU. | **Missing.** No `_start`, linker script, or freestanding target. |
| **CPU / privilege** | ring0/ring3 (x86) or EL1/EL0 (ARM); traps, context switch, the user/kernel boundary. | **N/A by design** (the unikernel route has no boundary; see §2). |
| **Memory management** | Physical frame allocator + virtual memory (page tables) + the heap allocator + MMU protection. | **Partial.** AST uses bumpalo arenas; runtime values are `Rc<RefCell<Vec/HashMap>>` (`crates/logicaffeine_data`). Heap = libc malloc today. No pager. |
| **Scheduling** | Runqueue, preemption, context switch, the task/thread abstraction. | **Have, and strong.** `crates/logicaffeine_runtime` — pure, zero-dependency, deterministic, tokio-free, `no_std`-portable cooperative M:1 scheduler; tasks, channels, select, timers, blocked-states all in-memory. The work-stealing M:N executor uses `std::thread::scope` (the one std tie to replace). |
| **Interrupts & timers** | IDT/IRQ + interrupt controller (PIC/APIC on x86, GIC on ARM) + a timer tick to drive preemption and `sleep`. | **Missing the hardware side.** But the scheduler *already models* `BlockedTimer`; it just consumes `std::time` instead of a real clock. |
| **Device drivers / HAL** | The only way to talk to physical hardware (MMIO / port-IO / DMA / IRQs): NIC, disk, display, input, serial, clock. | **Missing.** (`logicaffeine_system` is a *host-OS* HAL, not a hardware HAL — but it is the right shape; see §4.) |
| **Filesystem / storage** | Block device → FS (ext4/FAT) → VFS → file API. | **Partial.** A real VFS trait + 4 backends exist (`crates/logicaffeine_system/src/fs/mod.rs`: native tokio, io-uring, OPFS/IndexedDB). It rides on a host FS — no block device or on-disk FS of its own. |
| **Networking stack** | NIC driver → ethernet → IP → TCP/UDP → sockets. | **Partial-high.** Wire codec (`marshal.rs`), WS relay, libp2p mesh, gossip, CRDT sync, a quantum-safe transport roadmap (`QUANTUM_MAP.md`). All ride on tokio/host TCP; no NIC driver or own IP stack. `Listen`/`Connect` parse but are not fully wired. |
| **IPC / syscall boundary** | How userland asks the kernel for services; the security seam. | **N/A by design** on the unikernel route (function calls, not syscalls). |
| **Userland / init / ELF loading** | PID 1, loading programs, shell, libc. | **N/A** for a single-image unikernel; relevant only for the Linux-compat route. |
| **Time / entropy / power** | Clocks (RTC/TSC), RNG, ACPI/power. | **Have via host.** `logicaffeine_system`: `time.rs` (`SystemTime`), `random.rs` (`getrandom`/RDRAND). Swap host source for hardware source. |
| **Security / isolation** | Privilege separation, capabilities, users, memory protection. | **Language-enforced** (Rust safety + a Calculus-of-Constructions proof kernel `crates/logicaffeine_kernel`) — the differentiator (§5). |

**Reading of the table:** Logos has the *upper* and *middle* of the stack (scheduling, concurrency, networking semantics, FS abstraction, safety). It is missing the *floor* (boot, pager, interrupts, hardware drivers). That floor is the OS.

---

## 1.5 Linux as the reference skeleton — what it has, what's essential, our distance

We are not cloning Linux. We use it as the **yardstick**: its source tree doubles as a checklist of "everything an OS does." Walk the top-level directories and the usual intimidation inverts — **the overwhelming majority of Linux is the device tail, which a VM-first unikernel does not need.**

| Linux subsystem (dir) | What it does | Share of source | Needed for a *minimal own-OS*? | Logos distance |
|---|---|---|---|---|
| `drivers/` | Every device: NIC, GPU, USB, NVMe, sound, sensors… | **~60–70%** | **No** on a VM (virtio replaces it, ~4 drivers); **yes & unbounded** on bare metal | Far on bare metal; ~4 virtio drivers on a VM (`virtio-drivers` crate) |
| `arch/` | Per-CPU boot, MMU, traps, context switch (x86, arm64, riscv…) | ~10% | **Yes** (this is bring-up) | **Missing** — `logicaffeine_boot` (§4) |
| `mm/` | Buddy allocator, slab, page tables, virtual memory, mmap, swap | sizable | **Yes**, but a unikernel needs a *fraction* (identity-mapped, single AS, no swap) | Partial — have heaps (bumpalo/Rc), missing frame allocator + pager |
| `kernel/` | Scheduler (CFS/EEVDF), timers/hrtimers, signals, futex, irq core, cgroups | sizable | **Scheduler + timers + irq = yes; signals/futex/cgroups = no** (no multiprocess) | **Scheduler: have, strong** (`logicaffeine_runtime`). Timers: modeled, need HW clock. irq: missing |
| `fs/` + `block/` | VFS, ext4/btrfs/overlayfs, block layer, I/O scheduler | sizable | **A VFS + one simple FS = yes; the FS zoo = no** | Have a VFS trait + 4 backends; missing on-disk FS + block driver |
| `net/` | Sockets, TCP/IP, netfilter, routing, qdiscs | sizable | **A basic IP/TCP = yes; the rest = no** | Have wire codec/relay/mesh semantics; missing `no_std` IP/TCP (port smoltcp) |
| `ipc/` | SysV IPC (shm, semaphores, msg queues) | small | **No** — superseded by in-language channels/CRDTs | Have channels/select/CRDTs already |
| `security/` | LSM, SELinux, AppArmor, capabilities | small | **No** for a single-trust unikernel (the language enforces it) | Replaced by Rust safety + proof kernel |
| `init/` | `start_kernel`, mounting root, exec PID 1 | tiny | **Yes** (the boot finale) — but trivially small | Missing (part of `logicaffeine_boot`) |
| `crypto/` | In-kernel crypto API | small | **Optional** | Have a transport-crypto roadmap (`QUANTUM_MAP.md`) |
| `sound/`, `drivers/gpu/` | Audio, graphics | sizable | **No** (headless server OS) | Out of scope |
| `kernel/bpf/`, `kernel/trace/` | eBPF, ftrace — programmable observability | sizable | **No**, but *philosophically interesting*: Logos's JIT + proof kernel is a far more powerful "safe code injected into the kernel" story than eBPF | The differentiator, not a gap |

**The irreducible OS core** — strip Linux of the device tail, the FS zoo, and the multiprocess/security machinery, and what remains is small and fixed: **boot/arch + a memory manager + a scheduler + interrupts/timer + minimal I/O + init.** That residue *is* "what an OS is." Logos already owns the scheduler half of that core outright and the I/O half in abstraction; the genuinely-missing pieces are **boot/arch, the pager, and interrupts/timer hardware** — three focused bodies of work, not a million-line ocean.

**What Linux *has* that we'd consciously choose NOT to build (and why):** multiprocess isolation via page tables (we use language isolation — Singularity/Theseus), a syscall ABI (we use function calls — unikernel), users/permissions/namespaces/cgroups (single trust domain), the driver universe (virtio on VMs; one board on metal), loadable foreign binaries/ELF userland (we are the single image). Each omission is a *design decision with precedent*, not a missing feature — and each removes a major chunk of what makes Linux large.

---

## 2. The design space — kernel architectures, and the choices people make

| Architecture | Examples | Idea | Why chosen / cost | Relevance to Logos |
|---|---|---|---|---|
| **Monolithic** | Linux, FreeBSD | Everything (drivers, FS, net) in ring 0. | Fast; vast driver ecosystem. Cost: huge TCB, one driver bug = whole-system crash. | The thing you'd *run on*, not clone. |
| **Microkernel** | seL4, QNX, MINIX3, Zircon/Fuchsia | Kernel = IPC + scheduling + memory only; drivers/FS/net in user space. | Small TCB, fault isolation, **formally verifiable** (seL4 is fully proven). Cost: IPC overhead. Chosen for avionics, medical, Fuchsia. | The verification angle rhymes with the proof kernel. |
| **Hybrid** | Windows NT, XNU/macOS | Pragmatic mix. | Engineering compromise. | — |
| **Unikernel / library OS** | MirageOS, IncludeOS, OSv, Unikraft, Nanos, **Hermit (Rust)** | The *application* is linked with only the OS libraries it needs into one single-address-space image that boots directly on a hypervisor. | No syscalls (function calls), no user/kernel split, tiny attack surface, **~ms boot**. Cost: single app, single trust domain. | **The Logos-shaped answer.** Hermit proves a Rust `no_std` runtime-as-kernel on KVM works. |
| **Exokernel** | MIT Aegis/XOK | Kernel only multiplexes raw hardware; apps build their own abstractions. | Max flexibility; research lineage that produced unikernels. | Intellectual ancestor. |
| **Language-based / SASOS** | **Singularity (C#)**, **Theseus (Rust)**, Verve, JX, House (Haskell) | Isolation enforced by the *type system/compiler*, not hardware page tables — a single address space the language proves safe. | Eliminates page-table cost; enables live evolution. Cost: trust the compiler. | **The direct ancestors of a "Logos OS."** Theseus + Singularity are exactly "the language guarantees the isolation the MMU used to." |

**Where the Rust ecosystem already is** (proves feasibility, gives parts to reuse): `bootloader`/`bootimage` + the `x86_64` crate + blog_os (Phil Oppermann's tutorial OS), **Redox OS** (full microkernel + userland in Rust), **Theseus** (intralingual SAS), **Hermit/RustyHermit** (Rust unikernel on KVM), **Hubris** (Oxide, embedded), **Tock** (embedded), **smoltcp** (a `no_std` TCP/IP stack you can drop in), `uefi-rs`, the Limine bootloader, `virtio-drivers` (a `no_std` virtio crate).

---

## 3. The deployment substrate — and the driver question, answered

*Do we even need drivers, and what do they drive?* Drivers drive **physical devices**, because hardware only speaks device-specific protocols over memory-mapped registers (MMIO), port-IO, and DMA, signalled by interrupts. Abstracting those is most of what an OS *is*. **But how many drivers you need is set by the substrate, not the kernel design:**

| Substrate | Drivers needed | Why | Logos status |
|---|---|---|---|
| **Browser / WASM** | **Zero.** | The browser *is* the HAL; the WASM sandbox handles memory + execution. | **Already runs here** (VM compiled to wasm32; bytecode→WASM JIT). This is arguably already an "OS-less execution environment," and it proves the runtime lives without `std`. |
| **Hypervisor / VM (virtio)** | **~4–5.** virtio-net, virtio-blk, virtio-console (serial), virtio-rng, + the platform timer/interrupt controller. | **virtio is ONE simple, documented ring-buffer protocol** shared by every device. ~a few thousand LOC total; reusable via the `virtio-drivers` crate. This is how *every* unikernel ships. | **This is "barely any drivers."** The recommended first target on all your machines. |
| **Bare-metal physical** | **Many, board-specific.** | Real NIC (e1000/realtek/Intel), disk (AHCI/NVMe), USB (xHCI), PCIe enumeration, ACPI/power, framebuffer/GPU, keyboard. Each is a distinct protocol. | **The millions-of-lines, never-finished part of Linux.** Cost scales with #devices. Do exactly ONE board, late. |

**The fleet, concretely** — each is a different bring-up story, which is *why* the VM-first strategy matters (virtio gives one uniform surface across all of them):

- **x86-64 mini PCs** — UEFI + ACPI + APIC; the most-documented platform; QEMU/KVM trivial. Bare metal: e1000/realtek + NVMe/AHCI + xHCI.
- **Raspberry Pi (ARM)** — device-tree boot (GPU-first via `start.elf`, no standard UEFI), GIC interrupt controller, generic timer, BCM SoC peripherals, PSCI for SMP. Well-trodden by the bare-metal-Rust community.
- **Apple Silicon (aarch64)** — the hardest: Apple's iBoot chain, custom AIC interrupt controller, no standard ACPI/PCIe-for-everything; reverse-engineered by **m1n1 / Asahi Linux**. Bare metal here is a research project. A **VM on macOS** (Virtualization.framework / QEMU-hvf) is easy and gives the same virtio surface.

**Strategy that falls out:** target the **hypervisor on every box first** (one virtio code path covers x86 + ARM + Apple-via-VM), then pick **one** bare-metal board (a Pi or an x86 mini) for the physical-boot milestone. **Apple Silicon bare metal is explicitly deferred** (Asahi-grade effort).

---

## 4. The precise gap, mapped to Logos crates

| OS need | Have | Missing | Lands in |
|---|---|---|---|
| Freestanding boot | — | target JSON (`x86_64-unknown-none`, `aarch64-unknown-none`), linker script, `_start`, multiboot2/UEFI/device-tree stub | new `logicaffeine_boot` |
| `no_std` codegen profile | AOT emits `use logicaffeine_data; use logicaffeine_system;` (both `std`); **a `wasm32` profile already drops tokio + system-full** | a third `freestanding`/`bare` profile that targets `alloc` + a hardware-HAL trait instead of `logicaffeine_system` | `crates/logicaffeine_compile/src/codegen/program.rs` |
| Scheduler | **`logicaffeine_runtime` cooperative M:1 — zero external deps** | nothing for single-core | — |
| SMP | work-stealing M:N driver | replace `std::thread::scope` + mpsc with AP bring-up + per-CPU runqueues | `crates/logicaffeine_runtime/src/executor.rs` |
| Heap allocator | libc malloc | a global `alloc` allocator (linked-list → buddy/slab) | `logicaffeine_boot` |
| Pager / memory protection | bumpalo (AST), Rc heap (values) | frame allocator + page tables (`x86_64`/`aarch64` crates or hand-rolled). **Unikernel = mostly identity-mapped, single AS → much simpler than a multiprocess OS** | `logicaffeine_boot` |
| Interrupts + timer | scheduler **already models `BlockedTimer`** | IDT/GIC + a timer tick; swap `std::time`→TSC/HPET/generic-timer | `logicaffeine_boot` + `logicaffeine_system` (new cfg backend) |
| Drivers | host VFS + relay/mesh semantics | virtio-net/blk/console/rng + UART (reuse `virtio-drivers`) | `logicaffeine_system` (new cfg backend) |
| Net stack | wire codec, relay, gossip, CRDT sync | a `no_std` IP/TCP (port **smoltcp**, or grow our own atop the wire codec) under `Listen`/`Connect` | `logicaffeine_system/src/net*` |
| Time / entropy / env | `time.rs`, `random.rs`, `env.rs` | swap host source → TSC/generic-timer, RDRAND/RDSEED/virtio-rng, boot cmdline | `logicaffeine_system` (new cfg backend) |
| JIT on metal | forge mmaps RWX via libc | on a unikernel **you own page tables, so W^X is easier**; or ship AOT-only | `crates/logicaffeine_forge` (optional) |

**The structural keystone:** `logicaffeine_system` is *already* the HAL and is *already* cfg-split `native` vs `wasm32`. **Adding a third backend `cfg(logos_os)` is the same shape the project already maintains** — this is "lift and shift left," not a rewrite. The scheduler core is already pure and zero-dependency; the wasm profile already proves codegen can drop `std`. The bulk of the *language semantics* are already OS-agnostic; the work concentrates in one new backend + one new boot crate.

---

## 5. What "TRUE perfection" would be for a Logos OS

The SOTA frontier of "perfect" today: **seL4** (a microkernel with a machine-checked proof of functional correctness), **Theseus** (intralingual single-address-space, runtime state in safe-language "cells," live evolution), **MirageOS** (typed unikernels, whole-system specialization). Each owns *one* of the three perfection axes — proven-correct, language-isolated, minimal-specialized.

**Logos is uniquely positioned to combine all three**, because it already ships a Calculus-of-Constructions proof kernel + a Z3/SAT/BMC verification stack + deterministic, replay-checked, byte-identical cross-tier execution. The "true perfection" target — genuinely novel, nothing in the wild has it — is a **proof-carrying unikernel**:

- the scheduler's determinism is **machine-checked** (cross-tier byte-identity is already proven);
- isolation is **language-enforced** (Singularity/Theseus model — no page-table tax inside the image);
- drivers carry **proofs of MMIO safety** (verified device models, cf. seL4 + verified drivers);
- the image is **reproducible**, the TCB **tiny**, the "syscall ABI" is **function calls with proven contracts**;
- it **boots in ms on any hypervisor across x86/ARM**, is **hot-swappable** (HOTSWAP exists) and **live-migratable**.

In one line: **a typed unikernel with a theorem prover in the box and deterministic replay.** That is the differentiated "perfection" worth aiming at — not a Linux clone.

---

## 6. Staged path (phases, not line-by-line)

**Track A — Logos Unikernel (the spine; VM-first, multi-arch from P0):**

- **P0 — Bring-up:** target JSON (x86_64 + aarch64 `-none`), linker script, `_start`, serial UART, the `freestanding` codegen profile → a `.lg` program prints "hello" in QEMU on **both** x86 and ARM `virt`. *Proof: boots + prints, two arches.*
- **P1 — Memory:** global `alloc` allocator + frame allocator + minimal (identity) paging. *Proof: a heap-using `.lg` (lists/maps) runs.*
- **P2 — Time + interrupts:** IDT/GIC + timer tick wired to the scheduler's existing `BlockedTimer`; swap `std::time`. *Proof: `Sleep`, `Launch a task`, `Await the first of … After N seconds` run under the real timer.*
- **P3 — Block + console:** virtio-blk + virtio-console wired to the VFS trait + `io.rs`. *Proof: `file.read`/`file.write` on a virtio disk.*
- **P4 — Network:** virtio-net + a `no_std` IP/TCP (port smoltcp or grow our own atop the wire codec) under `Listen`/`Connect`. *Proof: two Logos unikernels talk; relay/`Sync` works machine-to-machine.*
- **P5 — SMP:** boot APs, per-CPU runqueues, replace the `std::thread` executor with the bare scheduler. *Proof: work-stealing M:N stays byte-identical to cooperative, on real cores.*
- **P6 — Self-host:** `largo` + the compiler run *as Logos programs on Logos OS* (the Futamura endgame). *Proof: build a `.lg` from inside the OS.*

**Track B — Bare metal (only after A is proven on VM): pick ONE board.** Raspberry Pi 4/5 (device-tree, GIC, BCM peripherals) **or** an x86 mini (UEFI, ACPI, NVMe/e1000). Add real drivers for *that board only*. **Apple Silicon bare-metal explicitly deferred** (Asahi-grade reverse engineering).

**Track C — Linux/POSIX-compat (separate; likely a non-goal).** Two honest options: (a) target the Linux **syscall ABI from codegen** so Logos *binaries* run on a Linux kernel — essentially already done via the native AOT path; (b) a Linux-syscall **emulation layer** inside the unikernel (gVisor/WSL1 style) to run *foreign* binaries. **Do not clone Linux.** If you want Linux, run on it.

---

## 7. How we'd stack up

| | Logos-OS (envisioned) | Linux | seL4 | MirageOS | Theseus | Redox |
|---|---|---|---|---|---|---|
| Architecture | proof-carrying unikernel | monolithic | microkernel | unikernel | language SAS | microkernel |
| Isolation | language + proofs | page tables | page tables (proven) | language (types) | language (Rust) | page tables |
| TCB | tiny | huge | ~10k LOC proven | small | small | small |
| Drivers | virtio (then 1 board) | everything | user-space | virtio | limited | growing |
| Boot | ~ms (VM) | seconds | ms | ~ms | ms | seconds |
| Verification | **CoC kernel + Z3 in-box** | none | full functional proof | types | types | none |
| Source | **English/Logos → FOL → native** | C | C | OCaml | Rust | Rust |

Differentiators: **English/Logos source, a theorem prover in the image, and deterministic replay** — a combination nobody ships.

---

## 8. Risks / honest non-goals

- **Bare-metal driver tail is unbounded** — stay on virtio; do one board, late.
- **A full TCP stack is real work** — port smoltcp first; grow our own only deliberately.
- **Apple Silicon bare metal ≈ research** (Asahi-grade). VM-on-macOS instead.
- **Heap model:** `Rc<RefCell>` is fine single-core; SMP shared mutable state needs design (lean on the deterministic scheduler + message passing already in hand).
- **Don't reimplement Linux.** It is a different project and a worse use of the proof/determinism assets.

---

## 9. Smallest real proof-point (so the roadmap isn't abstract)

The first buildable step: a **`logicaffeine_boot` crate + a `freestanding` codegen profile** (a third sibling to the existing `native` and `wasm32` profiles in `crates/logicaffeine_compile/src/codegen/program.rs`) that swaps `logicaffeine_system` for a hardware-HAL trait, emitting a **multiboot2 image QEMU boots and that prints via serial from a compiled `.lg`**. That is P0, and it is the load-bearing experiment: it proves the `no_std` codegen seam and the boot path in one shot, on both x86 and ARM (`qemu-system-x86_64` and `qemu-system-aarch64 -M virt`).

---

## Appendix — claims verified against the tree (2026-06-26)

- `crates/logicaffeine_runtime/Cargo.toml` — *"Pure, WASM-safe, tokio-free — never linked into AOT-compiled binaries"*; `[dependencies]` intentionally empty; cooperative M:1 is the default/WASM path, work-stealing M:N is the native multicore path auto-gated to `cfg(not(wasm32))`.
- Workspace (`Cargo.toml`) — 19 crate members across `crates/` + `apps/`; `wasm32-unknown-unknown` is a declared docs.rs target.
- `crates/logicaffeine_system` — native-vs-`wasm32` cfg split; VFS trait + 4 backends in `src/fs/mod.rs`; `time.rs`/`random.rs`/`env.rs` host wrappers.
- `crates/logicaffeine_compile` — AOT `Logos→Rust→LLVM→native cdylib`; a `wasm32` codegen profile that drops tokio + the `system` "full" feature already exists (the template for a `freestanding` profile).
- `crates/logicaffeine_forge` — copy-patch JIT; `#![no_std]` stencil unit (`stencils/int_stencils.rs`); executable pages via libc `mmap`/`mprotect` (Unix) / `VirtualAlloc` (Windows); x86_64 + aarch64.
- `crates/logicaffeine_kernel` — Calculus-of-Constructions proof kernel (minimal deps).
