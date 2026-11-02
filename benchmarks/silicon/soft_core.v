// The LOGOS SOFT-CORE — a fetch-decode-execute processor that runs LOGOS bytecode. It ties every
// realized unit together: a program counter + instruction memory + a 16×32 register file + decode +
// execute over the PROVEN general ALU (`alu.v`) and control flow (branch/jump/halt) + a `Show` output
// strobe. The instruction opcodes ARE the `binisa` wire opcodes (LoadConst=5, Move=6, Add=8, …,
// Jump=34, JumpIfFalse=35, JumpIfTrue=36, Halt=43, Show=89), so the chip decodes the same bytes the
// wire carries. Chip instruction word: [opcode:8][dst:8][a:8][b:8]; LoadConst/Jump use [op][dst][imm:16].
// HEAP (dmem depth) and FRAMES (call-stack depth) are parameters: simulation uses the full defaults
// (256-word heap, 16-deep stack); synthesis overrides them small (`chparam -set HEAP 64 -set FRAMES 4`)
// so the design fits without exploding the per-access address-decode muxes. A real chip sizes the heap to
// external DRAM (see the DRAM executor) and the stack to a RAM macro.
// MAXELT/MAXPAIR/MAXKEY bound the UNROLL of the collection/map ops (a list gather, a set union, a map
// key-scan): simulation uses the full defaults; synthesis overrides them small (`-set MAXELT 4 ...`) so the
// per-cell dmem writes don't explode into a huge address-decode. A real chip microcodes these as multi-cycle
// loops over one RAM port; here the bound is a compile-time knob.
module soft_core #(parameter HEAP = 256, parameter FRAMES = 64, parameter REGS = 256, parameter IMEM_WORDS = 1024,
                   parameter MAXELT = 16, parameter MAXPAIR = 8, parameter MAXKEY = 8) (
  input clk,
  input rst,
`ifdef SOFT_CORE_PROG_LOAD
  input        prog_we,      // program-load strobe: imem[prog_addr]<=prog_data (SoC opt-in)
  input  [15:0] prog_addr,   //   guarded by SOFT_CORE_PROG_LOAD; 16-bit reaches a wide (thousands-of-words) imem
  input [31:0] prog_data,    //   read-only boot ROM (lean fit); nexys_soc.v defines the macro
`endif
`ifdef SOFT_CORE_DATA_LOAD
  // DATA-load port (SoC opt-in via SOFT_CORE_DATA_LOAD): the SoC streams a DATA SEGMENT into the heap
  // WHILE the core is held in reset — weights loaded separately from the kernel, like an accelerator's
  // HBM/SRAM. `hp_init` seeds the bump allocator PAST the preloaded block so program allocations don't
  // collide with it. The compiled program reads the block as a `Seq of Int` at heap pointer 0 (the
  // `loaded()` builtin = Index over dmem[0..]). Writes fold into the ONE dmem write port (reset branch).
  input        data_we,      // data-load strobe: dmem[data_addr]<=data_data (only meaningful during reset)
  input  [15:0] data_addr,   //   heap cell to write (0..HEAP-1)
  input  [31:0] data_data,   //   the weight word
  input  [15:0] hp_init,     //   bump-allocator start after the preloaded block (dmem[0..hp_init-1] reserved)
`endif
  output reg [31:0] out,     // last value produced by a Show
  output reg out_valid,      // pulses high the cycle a Show executes
  output reg [63:0] outf,    // last value produced by a FLOAT Show (ShowF) — raw IEEE-754 f64 bits
  output reg outf_valid,     // pulses high the cycle a ShowF executes (the host formats the bits)
  // TEXT (string) Show: a `ShowText` streams its char list over this port — one word per strobe, the FIRST
  // carrying the char count (`outs_first` high), then each codepoint. The reply tag is 2 = string (len +
  // chars). The host (and the iverilog TB) reassembles the codepoints into a UTF-8 string. Kept separate
  // from `out`/`outf` so an int/float Show is byte-identical to before (the string burst is opt-in).
  output reg [31:0] outs,    // string burst word: the length (when outs_first) then each codepoint
  output reg outs_valid,     // pulses high each cycle a `ShowText` streams a word
  output reg outs_first,     // high on the FIRST word of a string (which carries the char count)
  // WORD32 (ℤ/2³²) Show: a `ShowWord32` (opcode 221) emits the SAME 32-bit ALU/heap result as an int Show,
  // but the host must format it UNSIGNED (0..2³²−1) — SHA-256's hash words exceed 2³¹, so the int `out`
  // port (formatted signed) would misprint them. A separate opt-in port (like `outf`) so int/float/string
  // Shows stay byte-identical; the host/TB prints `outw` as an unsigned decimal.
  output reg [31:0] outw,    // last value produced by a Word32 Show (ShowWord32) — host formats UNSIGNED
  output reg outw_valid,     // pulses high the cycle a ShowWord32 executes
  output reg halted,
  // FAILURE: `failed` is high once the core has trap-halted (a `FailWith`, a failed policy check, an
  // invalid codepoint, a `parseInt` error) rather than halted cleanly. `reasoned` is high when that trap
  // is a `FailWith` whose reason the program streamed over `outs` immediately before it (the assembler
  // lowers every LOGOS failure that way) — so the host reads the LAST Text record as the reason. A trap
  // with no text sets `failed` alone.
  output failed,
  output reg reasoned
);
  // FF-backed (the open flow drops distributed-LUT-RAM writes — the loader's program would never land):
  // the scalar-core fix. 256 words covers the benchmark subset (≤180). Combinational read keeps the FSM simple.
  localparam RW = $clog2(REGS);
  localparam IAW = $clog2(IMEM_WORDS);
  // Instruction memory = sync-read block-RAM — the SAME congestion/capacity fix as the register file below.
  // The old FF-backed `(* ram_style = "registers" *)` imem with a COMBINATIONAL `wire instr = imem[pc]`
  // synthesizes into a wide read mux that congests the open-flow router and caps a program at ~256 words.
  // Here imem is block-RAM with TWO registered read ports: `instr_q` (the fetch, at pc) and `imdata_q` (the
  // immediate / second word, at im_rb, defaulting to pc+1). yosys replicates the 2-read-port block into two
  // RAMBs; the loader / `$readmemh` write the single logical array, so a program can be thousands of words.
  (* ram_style = "block" *) reg [31:0] imem [0:IMEM_WORDS-1];
  reg [IAW-1:0] im_rb;       // data read-port address (immediate/second word; default pc+1; FSMs override to pc+2 / the text pool)
  reg [31:0]    instr_q;     // REGISTERED fetch — imem[pc], valid one cycle later; decode reads THIS, not imem[pc]
  reg [31:0]    imdata_q;    // REGISTERED data word — imem[im_rb], valid one cycle later; multi-word ops read this
  // SYNCHRONOUS-READ BRAM REGISTER FILE. The old `reg regs[]` with COMBINATIONAL variable-indexed reads
  // (av=regs[ra], bv=regs[rb], regs[dst], regs[b_src+i], the arg-copy scan …) synthesizes at large REGS
  // into huge N:1 MUXF7/F8 read muxes that CONGEST the open-flow router — it routes at REGS=32 but not at
  // REGS=256. The fix: three IDENTICAL block-RAM mirrors (rf0/rf1/rf2), all written on the one write port,
  // each giving one REGISTERED read port (av/bv/dv). A read is valid ONE cycle after its address is
  // presented. `a_ra/a_rb/a_dst` default to the current instruction's ra/rb/dst; an FSM that must read a
  // COMPUTED register overrides them and consumes the registered value one cycle later (a wait state).
  (* ram_style = "block" *) reg [31:0] rf0 [0:REGS-1];  // read port A -> av
  (* ram_style = "block" *) reg [31:0] rf1 [0:REGS-1];  // read port B -> bv
  (* ram_style = "block" *) reg [31:0] rf2 [0:REGS-1];  // read port C -> dv
  reg        rf_we;                   // register write enable (this cycle) — declared here (drives all three mirrors)
  reg [RW-1:0] rf_wa;                 // register write address (defaults to dst)
  reg [31:0] rf_wd;                   // register write data
  reg [RW-1:0] a_ra, a_rb, a_dst;     // BRAM read addresses (default: instr ra/rb/dst; FSMs override)
  reg [31:0] av, bv, dv;              // REGISTERED read data — valid the cycle after a_ra/a_rb/a_dst are set
  reg [15:0] pc;             // 16-bit PC (split-byte branch targets reach a wide imem)
  // OPERAND BUBBLE (now 2-DEEP). Fetch AND operands are both registered BRAM reads in series: pc -> instr_q
  // (imem sync-read) -> a_ra=ra -> av (regfile sync-read). So av/bv/dv reflect imem[pc] only TWO cycles after
  // the PC moves. `pc_q` is last cycle's PC, `pc_qq` two cycles ago; operands_ready is high only once the PC
  // has held for two full cycles (fetch latched imem[pc], then the read ports latched its ra/rb/dst). The main
  // dispatch fires only when operands_ready. The multi-cycle sub-FSMs HOLD the PC, so pc==pc_qq stays true and
  // they run every cycle with no extra bubble (exactly as before — only the straight-line fetch gains a cycle).
  reg [15:0] pc_q, pc_qq;
  wire operands_ready = (pc == pc_qq);

  // The LOAD-STORE unit's heap: a flat data memory. A list at pointer `p` is `dmem[p] = length`
  // followed by its elements at `dmem[p+1 .. p+length]`, so the 1-based `item i of xs` is `dmem[p+i]`.
  // `hp` is the bump-allocator's next-free cell. (Mirrors the Rust `run_soft_core` heap model + the VM's
  // `RuntimeValue::List`.) NewList's gather is a constant-bound-16 unroll (synthesizable; lists ≤ 16).
  // DMEM_REGISTERS forces the heap FF-backed (opt-in, default OFF = BRAM as before): on the open FOSS flow
  // a BRAM heap's read-during-write / hold-marginal timing diverges from the iverilog `reg` model, which
  // silently corrupts data-dependent heap reads on real silicon (the first heap-heavy program surfaced it).
  // Registers match the sim exactly. Needs a small HEAP to fit — the SoC that defines it sets one.
`ifdef DMEM_REGISTERS
  (* ram_style = "registers" *)
`endif
  reg [31:0] dmem [0:HEAP-1];
  // HEAP ADDRESS WIDTH: hp and every dmem address / heap-pointer / base+offset signal are HPW bits so the
  // bump allocator covers the whole dmem (HEAP=2048 -> 11 bits), not a 256-word wrap. Pointers stored into a
  // register are kept as full 32-bit values; re-reading one as an address slices it back to [HPW-1:0].
  localparam HPW = $clog2(HEAP);
  localparam ZPAD = 32 - HPW;         // zero-extension of an HPW-wide heap value back up to a 32-bit word
  reg [HPW-1:0]  hp;
  integer m;

  // The iterator stack: each `for x in xs` pushes a (collection pointer, 1-based cursor); IterNext reads
  // dmem[ptr+cursor] and advances, branching to the loop's exit (IterPop) when the cursor passes the length.
  reg [HPW-1:0] iter_ptr [0:7];
  reg [HPW-1:0] iter_cur [0:7];
  reg [3:0] isp;
  reg [31:0] globals [0:REGS-1]; // globals table (Main top-level Lets promoted to globals) — survives Call/Return
  // Select arms: `Await the first of:` registers up to 4 (chan reg, var reg) recv arms (or timeout arms);
  // SelectWait fires the first ready recv, else the timeout arm, and reports the fired index.
  reg [RW-1:0] sel_chan [0:3];
  reg [31:0]   sel_cptr [0:3];        // RESOLVED channel heap pointer (regs[chan_reg] captured at arm time via dv)
  reg [RW-1:0] sel_var  [0:3];
  reg       sel_tmo  [0:3];
  reg [3:0] sel_n;
  reg [3:0] sel_fired;
  reg [3:0] sel_to;                  // SelectWait: combinational lowest-timeout-arm temp (sel_tmo scan, no dmem)
  integer   si;
  reg found;                 // SetAdd/Union membership scratch (combinational within the always block)
  // MAP scratch: a map is `[MAP_TAG|npairs, k0,v0,…]` (keys are char-list ptrs). key_eq compares char lists.
  localparam MAP_TAG = 32'h4000_0000;
  reg [31:0] map_val;        // looked-up value
  reg [4:0]  map_found;      // matching pair index (0x1f = none)
  reg        keq;            // key-equality scratch
  integer    pi, kj;         // pair / char scan indices
  reg [31:0] fmt_n, fmt_len; // FormatValue: decimal conversion scratch
  reg [7:0]  fmt_digit [0:11]; // up to 10 digits + a leading '-' + slack
  integer    di;
  // FormatValue as a COMPACT MULTI-CYCLE FSM (`fmtf`): peel one decimal digit per cycle (÷10/%10 on ONE
  // shared constant-divider instead of the old 10-wide combinational unroll that congested the router), then
  // hand the digit buffer to the uniform build-microcode. Runs entirely inside the core (the host no longer
  // has to format `Text + Int` interpolation — the chip produces the char list).
  reg        fmtf;           // FormatValue digit-peel FSM in progress (stalls dispatch)
  reg [31:0] fmtf_n;         // running |value|, divided by 10 each cycle
  reg [3:0]  fmtf_k;         // digit cursor (0..) — how many digits emitted so far
  reg        fmtf_neg;       // the value was negative (a leading '-')
  reg [RW-1:0] fmtf_dst;     // FormatValue result register
  // TEXT (string) scratch. A Text is a char-code list `[len, c0, …]` on `dmem`; its OPERATIONS ride the
  // list/heap datapath (Concat=32, Length=62, Index=59). TWO chip-synthetic ops carry the string surface:
  // `TextConst`=216 (copy a pooled char block out of imem into a fresh heap char-list) and `ShowText`=217
  // (stream the char list out over `outs`). `TextCharAt`=218 builds a single-char Text for `item i of text`;
  // `TextEq`=219 / `TextNe`=220 compare two char lists by CONTENT.
  reg        stx;            // ShowText streaming FSM in progress (stalls dispatch)
  reg [2:0]  stx_ph;         // 0 = read length, 1 = emit length + issue char-0, 2 = stream chars
  reg [HPW-1:0] stx_ptr;     // the char-list base pointer (regs[ra])
  reg [31:0] stx_len, stx_i; // char count + 1-based cursor
  reg [IAW-1:0] tc_off;      // TextConst: imem pool word-offset of c0 (imdata_q=imem[pc+1], latched at dispatch); IAW-wide so the pool can sit anywhere in imem
  reg        teq;            // TextEq/TextNe content-compare FSM in progress
  reg [2:0]  teq_ph;         // 0 = read the two lengths, 1 = compare chars, 2 = deliver
  reg [HPW-1:0] teq_p, teq_q;// the two char-list base pointers
  reg [31:0] teq_len, teq_i; // char count (equal-length precondition) + 1-based cursor
  reg        teq_eqop;       // 1 = TextEq (219), 0 = TextNe (220)
  reg        teq_res;        // running equality result
  reg [31:0] teq_lp, teq_lq; // the two lengths while reading them
  reg [RW-1:0] teq_dst;      // TextEq/TextNe result register
  // TEXT / SEQ BUILTIN plane (chip-synthetic opcodes 233–237 — the deterministic string/fill/parse/encode
  // builtins the full-core encoder emits). `Chr`=233 (a fresh 1-char Text `[1, codepoint]`, a 2-write wbuf
  // like TextCharAt) and `RepeatSeq`=235 (an `n`-slot scalar fill) reuse the wbuf/drng + `bld` build-microcode
  // (a new `b_op == 235` data-function fills every element with the scalar `b_fill`). `TextContains`=234,
  // `ParseInt`=236 and `TextBytes`=237 are multi-cycle char-scan FSMs over the char list on `dmem`.
  reg [31:0] b_fill;         // RepeatSeq (235): the scalar element written to every filled slot (regs[elem])
  // SUBSTRING SCAN FSM (`TextContains`=234): `regs[dst] = 1` iff `needle` occurs as a contiguous codepoint run
  // in `haystack`. A double loop over the single synchronous read port — an OUTER window start over the
  // haystack, an INNER per-char compare of needle against that window — byte-identical to the model's nested
  // `for start { for k … }` (and to the VM's `str::contains`). The empty needle is contained in everything.
  reg        tcs;            // TextContains scan in progress (stalls dispatch)
  reg [2:0]  tcs_ph;         // 0 = read |hay|, 1 = read |needle|, 2 = window-char read/compare, 3 = deliver
  reg [HPW-1:0] tcs_hp, tcs_np;   // haystack / needle char-list base pointers (regs[ra], regs[rb])
  reg [HPW-1:0] tcs_hl, tcs_nl;   // haystack / needle char counts (headers)
  reg [HPW-1:0] tcs_start, tcs_k; // outer window start (0-based), inner char cursor (0-based)
  reg        tcs_rp;         // inner read sub-phase: 0 = issue hay[start+k], 1 = have hay char -> read needle[k]
  reg [31:0] tcs_hc;         // the haystack codepoint just read (compared against the needle codepoint)
  reg        tcs_res;        // running found flag
  reg [RW-1:0] tcs_dst;      // result register
  // DECIMAL PARSE FSM (`ParseInt`=236): scan the Text char-list at `regs[ra]` -> the signed value in regs[dst],
  // EXACTLY the model's `str::trim().parse::<i64>()` boundary — trim Unicode whitespace off both ends (space,
  // tab, newline, CR; the differential corpus stays inside these), accept an optional single leading `+`/`-`,
  // then require >=1 ASCII digit, accumulating in the NEGATIVE magnitude (`acc = acc*10 - digit`) with a
  // checked overflow -> trap. The RTL accumulator is 32-bit (the datapath width); the i64 boundary cases
  // (i64::MIN/MAX, the >i32 overflow gap) are MODEL-level proofs — the RTL differential corpus stays in the
  // i32 range where the 32-bit accumulation reproduces the model exactly, non-numeric/empty inputs trap on
  // both. A non-digit char, no digit at all, or a 32-bit overflow -> the trap-halt (pre-trap output stands).
  reg        pin;            // ParseInt scan in progress (stalls dispatch)
  reg [2:0]  pin_ph;         // 0 = read len, 1 = read+trim chars into a buffer, 2 = post-trim sign/digit scan, 3 = deliver
  reg [HPW-1:0] pin_ptr;     // the Text char-list base pointer (regs[ra])
  reg [HPW-1:0] pin_len;     // char count (header)
  reg [HPW-1:0] pin_i;       // read cursor over the char list (1-based into the payload)
  reg [HPW-1:0] pin_lo, pin_hi; // trimmed [lo, hi) window over the buffered codepoints (0-based)
  reg [31:0] pin_buf [0:63]; // buffered codepoints (the corpus texts fit; a longer text is out of the differential)
  reg        pin_neg;        // a leading '-' (accumulate the true sign at delivery)
  reg        pin_trim;       // the ph2 trim+sign resolve ran (once); after it, ph2 walks the digit run
  reg [31:0] pin_acc;        // running NEGATIVE magnitude (`acc = acc*10 - digit`)
  reg        pin_saw;        // saw >=1 ASCII digit (an empty digit run is an error)
  reg        pin_err;        // a non-digit char or a 32-bit overflow was hit (-> trap)
  reg [RW-1:0] pin_dst;      // result register
  reg [31:0] pin_mul, pin_sub; // 32-bit checked `acc*10` / `-digit` scratch (overflow detection)
  // UTF-8 ENCODE FSM (`TextBytes`=237): the Text char-list at `regs[ra]` -> a fresh Seq `[n, b0, b1, …]` of its
  // UTF-8 bytes, the model's `utf8_of_codepoint` range encoder (1 byte below 0x80, 2 below 0x800, 3 below
  // 0x10000, else 4). The byte count is known only after the scan, so the bytes stream to `hp+1…` one per
  // cycle while the next codepoint's read is in flight, and the header is the last write.
  reg        tbx;            // TextBytes encode in progress (stalls dispatch)
  reg [1:0]  tbx_ph;         // 0 = latch len, 1 = codepoint in -> lead byte, 2 = continuation bytes, 3 = header + deliver
  reg [HPW-1:0] tbx_ptr;     // the Text char-list base pointer (regs[ra])
  reg [HPW-1:0] tbx_len;     // char count (header)
  reg [HPW-1:0] tbx_i;       // 1-based cursor over the char list
  reg [HPW-1:0] tbx_base;    // the new Seq's header address (hp at dispatch)
  reg [HPW-1:0] tbx_w;       // next byte address
  reg [31:0] tbx_cp;         // the codepoint being encoded
  reg [2:0]  tbx_n;          // its UTF-8 length (1..4)
  reg [2:0]  tbx_k;          // index of the next continuation byte (1..n-1)
  reg [RW-1:0] tbx_dst;      // result register
  // Networking OFFLINE-LOOPBACK inbox: NetSend publishes (peer key, message); NetAwait drains by key.
  reg [31:0] ibox_key  [0:3]; // peer key of each buffered message
  reg [31:0] ibox_msg  [0:3]; // the buffered message
  reg        ibox_val  [0:3]; // slot occupied
  reg [31:0] peer_seq;        // fresh peer-identity counter (NetMakePeer)
  reg [2:0]  ib_slot;         // scan/allocation index
  reg [2:0]  ib_hit;          // matched slot (0x4 = none)
  integer n2;                // inner-loop index for the set-algebra ops
  reg [31:0] ucount;         // running element count while building a union/intersection
  reg [7:0]  ridx;           // RemoveFrom: 1-based index of the element to drop (0 = absent)

  // Call/return frame stack: each Call snapshots the whole register window + return PC + result reg; Return
  // restores the window and places the returned value in the caller's result reg (recursion-capable). The
  // saved windows live in a SYNCHRONOUS-READ BRAM (`savedregs`) addressed frame*REGS + reg — one write port
  // and one registered read port (sv, valid the cycle after sr_ra), so Call/Return became MULTI-CYCLE FSMs
  // (`cl`/`rt`) that move one register per cycle. Combinational async-read of a FRAMES×REGS array would
  // explode the router exactly like the old regfile did; the sequential frame engine is the fix.
  localparam SAW = $clog2(FRAMES * REGS);
  (* ram_style = "block" *) reg [31:0] savedregs [0:FRAMES*REGS-1]; // flat call-stack: index = frame*REGS + reg
  reg [SAW-1:0] sr_wa;                 // savedregs write address
  reg [31:0]    sr_wd;                 // savedregs write data
  reg           sr_we;                 // savedregs write enable
  reg [SAW-1:0] sr_ra;                 // savedregs read address (combinational)
  reg [31:0]    sv;                    // savedregs read data (registered — valid the cycle after sr_ra)
  // FLOAT call-stack, PARALLEL to `savedregs`: the model snapshots BOTH the int AND the float register
  // window per Call and restores both on Return (frames = (int window, float window, …)). The float bank
  // is a flat 256-entry BRAM shared across frames, so a callee's float writes would clobber the caller's
  // fregs without this — a float-returning function (spectral_norm's `aVal`) or a float arg needs it. Two
  // 32-bit BRAMs (lo/hi) like every other float store; one write port + one registered read port (sfv).
  (* ram_style = "block" *) reg [31:0] savedfregs_lo [0:FRAMES*REGS-1];
  (* ram_style = "block" *) reg [31:0] savedfregs_hi [0:FRAMES*REGS-1];
  reg [SAW-1:0] sfr_wa;                 // savedfregs write address
  reg [63:0]    sfr_wd;                 // savedfregs write data (f64 bits)
  reg           sfr_we;                 // savedfregs write enable
  reg [SAW-1:0] sfr_ra;                 // savedfregs read address (combinational)
  reg [63:0]    sfv;                    // savedfregs read data (registered — valid the cycle after sfr_ra)
  // retpc/retdst are small (a few bits × FRAMES) — they stay flops (no read-mux blowup).
  (* ram_style = "registers" *) reg [15:0]    retpc  [0:FRAMES-1];
  (* ram_style = "registers" *) reg [RW-1:0]  retdst [0:FRAMES-1];
  reg [$clog2(FRAMES):0] sp;          // frame pointer (0..FRAMES) — width tracks the FRAMES toggle
  // Call/Return sequential frame engine.
  reg           cl;                    // Call/CallValue/Spawn/SpawnHandle frame-save in progress (stalls dispatch)
  reg [1:0]     cl_ph;                 // 0 = SAVE caller regs, 1 = SEED callee regs (arg-copy), 2 = finish
  reg [1:0]     cl_mode;               // 0 = Call (arg-copy seed), 1 = CallValue, 2 = Spawn, 3 = SpawnHandle (no seed)
  reg           cl_needentry;          // CallValue: capture the closure entry from dm_rq at SAVE start
  reg [RW:0]    cl_j;                  // per-register cursor (0..REGS)  (extra bit so the ==REGS compare is clean)
  reg [7:0]     cl_as, cl_ac;          // args_start, arg_count (Call convention)
  reg [15:0]    cl_entry, cl_retpc;    // callee entry pc, caller return pc
  reg [RW-1:0]  cl_retdst;             // caller result register
  reg [1:0]     cl_seed_ph;            // SEED / SAVE / RESTORE sub-phase (issue read, then consume+write)
  reg           cb2;                   // CallBuiltin Min/Max second cycle: reads regs[args_start+1] via port B
  reg [7:0]     cb_op;                 // Min (7) / Max (8)
  reg [31:0]    cb_av;                 // the first operand regs[args_start], captured at dispatch
  reg [RW-1:0]  cb_dst;                // result register
  reg           rsw;                   // RevSwap second-write in progress (a 2-write op through the 1 write port)
  reg [RW-1:0]  rsw_wa;                 // RevSwap: the second write's address (regs[dst])
  reg [31:0]    rsw_wd;                 // RevSwap: the second write's data (old regs[ra])
  reg           p2w;                    // pending SECOND register write, pc-neutral (SelectWait: value + fired-index)
  reg [RW-1:0]  p2w_wa;                 // its address
  reg [31:0]    p2w_wd;                 // its data
  reg           rt;                    // Return in progress (stalls dispatch)
  reg [1:0]     rt_ph;                 // 0 = RESTORE caller regs from frame sp-1, 1 = place result, 2 = finish
  reg [RW:0]    rt_j;                  // per-register cursor
  reg [31:0]    rt_rv;                 // captured return value (callee's regs[ra])
  reg [63:0]    rt_frv;                // captured FLOAT return value (callee's fregs[ra]) — placed into fregs[retdst]
  reg           rt_hasrv;             // Return(41) writes the result; ReturnNothing(42) does not
  reg [$clog2(FRAMES):0] rt_fp;        // the frame being restored (sp-1, latched)
  reg           trapped;               // FailWith raised — an error trap-halt (vs a clean Halt)
  assign failed = trapped;
  integer j;
  // SINGLE WRITE PORT for the register file. Instead of ~48 case arms each writing `regs[dst] <= …` (which
  // forces yosys to build a 48-way mux per register), each arm sets rf_we/rf_wa/rf_wd (declared with the BRAM
  // mirrors above) and ONE place writes all three mirrors identically. The few genuinely-multi-write ops
  // (RevSwap, DestructureTuple) stage a SECOND write through the same port across cycles.
  // SINGLE WRITE PORT for the data memory + a MICROCODE FSM so multi-cell ops write ONE cell/cycle. Instead of
  // ~50 arms (many with unrolled per-element loops) each writing `dmem[…] <= …` (which forces yosys to mem2reg
  // dmem into flops), each arm sets these controls and ONE place does `if (dm_we) dmem[dm_wa] <= dm_wd`. The
  // allocating build ops (NewList/NewStruct/ChanNew/FormatValue/NewRange) run a UNIFORM build-microcode: while
  // `bld` they write dmem[b_base + b_i] = f(b_i) one cell per cycle (PC frozen), finishing at b_i == b_n.
  reg        dm_we;                   // dmem write enable (this cycle)
  reg [HPW-1:0] dm_wa;                // dmem write address (full heap width)
  reg [31:0] dm_wd;                   // dmem write data
  // SINGLE SYNCHRONOUS read port for dmem — the E2f fix that lets memory_bram map dmem to ECP5 block RAM
  // (asynchronous reads cannot map, which is why the async version explodes to 97K LUT + timing loops).
  // dm_ra is driven combinationally by the active op/FSM; dm_rq holds dmem[dm_ra] ONE cycle later. A
  // dmem-reading op therefore runs in phases: issue the address(es), then use the registered data dm_rq.
  reg [HPW-1:0] dm_ra;                // dmem read address (combinational, full heap width)
  reg [31:0] dm_rq;                   // dmem read data (registered — valid the cycle after dm_ra is set)
  // FLOAT-HEAP PLANE (`fdmem`): a dense f64 payload plane PARALLEL to `dmem` at the SAME address space —
  // a `Seq of Float`'s structure (header `[length,…]` + int placeholder cells) lives in `dmem`, its element
  // f64 PAYLOADS ride here at the identical indices (mirrors the Rust model's `fheap`, kept the same length
  // as `heap`). The open FOSS BRAM can't do 64-bit-wide reliably (same reason `fregs` is split), so this is
  // TWO 32-bit BRAMs (lo/hi) written/read as a pair — exactly like frf0_lo/frf0_hi. One write port
  // (fdm_we/fdm_wa/fdm_wd) and one SYNCHRONOUS read port (fdm_ra → fdm_rq one cycle later). The float-list
  // ops 213/214/215 drive it; the int-heap structure ops never touch it (a plain int-list COW leaves the
  // parallel float cells untouched, exactly as the model does — only the FLOAT COW, op 213, copies them).
`ifdef DMEM_REGISTERS
  (* ram_style = "registers" *)
`endif
  reg [31:0] fdmem_lo [0:HEAP-1];     // float payload plane, low 32 bits  -> fdm_rq[31:0]
`ifdef DMEM_REGISTERS
  (* ram_style = "registers" *)
`endif
  reg [31:0] fdmem_hi [0:HEAP-1];     // float payload plane, high 32 bits -> fdm_rq[63:32]
  reg        fdm_we;                  // float-heap write enable (this cycle)
  reg [HPW-1:0] fdm_wa;               // float-heap write address (full heap width)
  reg [63:0] fdm_wd;                  // float-heap write data (f64 bits)
  reg [HPW-1:0] fdm_ra;              // float-heap read address (combinational)
  reg [63:0] fdm_rq;                  // float-heap read data (registered — valid the cycle after fdm_ra)
  reg        ld;                      // a synchronous load is in progress (op re-enters to consume dm_rq)
  reg [7:0]  ld_op;                   // which op is mid-load
  reg [3:0]  ld_ph;                   // load phase (for multi-read ops)
  reg [31:0] ld_t0, ld_t1;            // scratch carrying earlier reads into later phases
  reg        bld;                     // multi-cycle build in progress
  reg [HPW-1:0] b_base;               // = hp at the start of the build (heap address)
  reg [HPW-1:0] b_i;                  // current cell index (0 = header) — offset added to b_base
  reg [HPW-1:0] b_n;                  // last cell index (element count for a list) — feeds b_base+b_n address math
  reg [RW-1:0]  b_dst;                   // result register
  reg [7:0]  b_op;                    // which op is being built (data-function selector)
  reg [RW-1:0]  b_src;                   // source base register (NewList gather)
  reg [31:0] b_lo;                    // low bound (NewRange)
  reg [31:0] b_hdr;                   // custom header value (NewInductive variant tag)
  reg [HPW-1:0] b_csrc, b_csrc2;      // copy-source base pointers (DeepClone/Slice/Concat)
  reg [HPW-1:0] b_len1;               // length of source 1 (Concat phase boundary) — used in b_csrc2+i-b_len1
  reg        b_ph;                    // build sub-phase: 0 = issue source read (copy builds), 1 = write the cell
  // WRITE BUFFER: a fixed-multi-write op (a channel enqueue/dequeue, a list push, …) stages up to 4
  // (address, data) pairs into these cheap flops in its decode cycle, then the `drng` drain-FSM writes them
  // to dmem ONE per cycle through the single port. So even a 3-write op needs only one dmem write port.
  reg [HPW-1:0] wbuf_a [0:3];
  reg [31:0] wbuf_d [0:3];
  reg [2:0]  wbuf_n;                  // number staged
  reg [2:0]  wbuf_i;                  // drain index
  reg        drng;                    // draining the write buffer
  // ITERATE FSM for the loop ops (CrdtMerge in-place add, Union/Intersect filter-merge, RemoveFrom shift):
  // scan a source one element per cycle through the single write port, with a running output count.
  reg        itr;                     // iterate in progress
  reg [7:0]  i_op;                    // which op
  reg        i_ph;                    // phase (Union: 0=copy lhs, 1=scan rhs)
  reg [HPW-1:0] i_si;                 // source scan index (1-based) — added to base pointers to index dmem
  reg [HPW-1:0] i_w;                  // output write count — added to i_dst to form a dmem address
  reg [HPW-1:0] i_dst, i_s1, i_s2;    // output base, source-1, source-2 pointers (heap addresses)
  reg [HPW-1:0] i_l1, i_l2;           // source-1, source-2 lengths — feed base+len dmem address math
  reg [31:0] i_val;                   // filter value (RemoveFrom) / scratch
  reg [RW-1:0]  i_rd;                    // result register
  reg [2:0]  i_rp;                    // iterate READ sub-phase (synchronous-read: 0 = issue, later = consume)
  reg [HPW-1:0] i_m;                  // inner membership-scan cursor (Union/Intersect) — added to a base pointer
  reg        i_found;                 // inner membership-scan result
  // MAP KEY-SCAN FSM (Index/SetIndex on maps): a map is [MAP_TAG|npairs, k0_ptr, v0, …]; keys are char-lists,
  // so matching a key is DOUBLE-INDIRECT (dmem[dmem[map+1+2*pi]]). This sequentially scans pairs and, per
  // pair, compares the query key char-by-char through the single synchronous read port.
  reg        ms;                      // map key-scan active
  reg [7:0]  ms_op;                   // 59/60 = Index, 57/58 = SetIndex
  reg [HPW-1:0] ms_map, ms_qk;        // map base ptr, query-key ptr (heap addresses)
  reg [RW-1:0]  ms_dst;                  // Index: result register
  reg [31:0] ms_wv;                   // SetIndex: value to write
  reg [HPW-1:0] ms_np, ms_pi, ms_kj;  // pair count, current pair (map+2+2*pi), current key char (kptr+kj)
  reg [HPW-1:0] ms_kptr, ms_qlen;     // current pair's key ptr, query-key length (both feed dmem address math)
  reg [31:0] ms_qc, ms_val;           // current query char, Index accumulated value
  reg [HPW-1:0] ms_found;             // SetIndex: matched pair index + 1 (0 = not found) — feeds map+2+((found-1)<<1)
  reg        ms_keq;                  // keys equal so far
  reg [2:0]  ms_ph;                   // sub-phase
  reg [RW-1:0]  ms_reg;              // SetIndex: the map's destination register — repointed when an append COWs the block
  // COW COPY sub-machine (string-key SetIndex append when the map is NOT on top of the heap): a `TextConst`
  // key char-list is allocated before each `Set item "k" of m`, so after the first insert the map block is
  // buried under later allocations. A bare in-place append would scatter the pair past those cells while the
  // pair-scan still addresses `map+1+2*pi`, corrupting the map. So — exactly like the ListPush COW (op 54) —
  // copy the whole map block `[header, k0,v0, …]` to a fresh contiguous block on top, append the new pair
  // there, and repoint the map register. `ms_cow` drives the copy; `ms_ci` walks the `1+2*np` source cells.
  reg        ms_cow;                 // COW copy in progress (append when the map isn't on top)
  reg [HPW-1:0] ms_ci;               // copy cursor (0-based over the map's 1+2*np cells)
  reg        ms_crp;                 // copy read sub-phase (0 = issue read, 1 = write it)
  reg [HPW-1:0] ms_nb;               // COW target base (the fresh block on top of the heap)

  // INT-KEY MAP SCAN FSM (MapIndexInt 223 / MapSetIndexInt 224): a `Map of Int to …` stores the key as a
  // DIRECT integer in the key cell (not a char-list pointer), so a match is a single-indirect integer `==`
  // (dmem[map+1+2*pi] == key) — no key-length read, no per-char loop. A distinct engine from the char-key
  // `ms` FSM so the proven string-key path is untouched; it walks the pair cells through the same single
  // synchronous read port. 223 delivers the matched value (0 on miss); 224 overwrites the value on a hit,
  // else appends `(key, value)` on top of the heap and bumps the pair count (same contiguity rule as the
  // char-key SetIndex — the int-key map stays on top of the heap so the in-place append is sound).
  reg        msi;                     // int-key map scan active
  reg [7:0]  msi_op;                  // 223 = MapIndexInt, 224 = MapSetIndexInt
  reg [HPW-1:0] msi_map;              // map base ptr (heap address)
  reg [RW-1:0]  msi_dst;              // MapIndexInt: result register
  reg [31:0] msi_key, msi_wv;         // query key (int), value to write (224)
  reg [HPW-1:0] msi_np, msi_pi;       // pair count, current pair index
  reg [HPW-1:0] msi_found;            // matched pair index + 1 (0 = not found)
  reg        msi_ph;                  // 0 = issue this pair's key read, 1 = compare + decide

  // Decode the instruction at the PC.
  wire [31:0] instr  = instr_q;   // the REGISTERED fetch (imem[pc] latched last cycle) — not a combinational read
  wire [7:0]  opcode = instr[31:24];
  wire [7:0]  dst    = instr[23:16];
  wire [7:0]  ra     = instr[15:8];
  wire [7:0]  rb     = instr[7:0];
  wire [15:0] imm    = instr[15:0];
  // av/bv/dv are now the REGISTERED BRAM read ports (declared above) — valid one cycle after a_ra/a_rb/a_dst.

  // The proven general ALU computes every arithmetic/bitwise/comparison op straight from the opcode.
  wire [31:0] alu_y;
  alu the_alu(.opcode(opcode), .a(av), .b(bv), .y(alu_y));

  // ONE shared iterative divider serves Div/ExactDiv/Mod/FloorDiv (opcodes 12-15): instead of a
  // ~22,000-LUT combinational divide per op, the core pulses `div_start`, STALLS the pipeline (pc
  // holds, so av/bv stay stable) until `div_done`, then retires the quotient (Div), remainder (Mod),
  // or the floor-adjusted quotient (FloorDiv). DivPow2 (16) stays in the ALU (it is a cheap shift).
  reg          div_start, div_busy;
  reg  [RW-1:0]   div_dst;
  reg  [7:0]   div_op;
  wire [31:0]  div_q, div_r;
  wire         div_done;
  shared_divider #(.W(32)) u_div(.clk(clk), .rst(rst), .start(div_start), .a(av), .b(bv),
                                 .q(div_q), .r(div_r), .done(div_done));

  // ONE sequential POWER unit serves `a ** b` (Pow, opcode 18): rather than an unbounded combinational
  // exponentiator, the core latches base/exponent, STALLS the pipeline (pc holds), and multiplies an
  // accumulator by `base` once per cycle for `exp` cycles (acc starts at 1 → `base**0 == 1`), then retires
  // `acc`. A 32-bit wrapping multiply, EXACTLY like the ALU `Mul` — so it matches the model over the in-range
  // differential (the model auto-promotes to BigInt on overflow; the RTL wraps at 2^32, the same width
  // contract every value op carries). The exponent is treated as unsigned (a negative Int power errors in
  // the VM, so no program reaches the unit with exp < 0).
  reg           pow_busy;               // a power computation is running (stalls dispatch)
  reg  [RW-1:0] pow_dst;                // destination register
  reg  [31:0]   pow_base;               // the (stable) base operand
  reg  [31:0]   pow_acc;                // the running product (starts at 1)
  reg  [31:0]   pow_cnt;                // remaining multiplications (counts the exponent down to 0)

  // CRDT LATTICE-JOIN coprocessor (CrdtJoin, op 113): the `crdt_merge` block computes the per-replica
  // MAX of two 4-lane G-Counters in ONE combinational step — the state-based, idempotent join. The
  // core assembles the target's and source's lanes from dmem into cj_a/cj_b, the block joins them, and
  // the result cj_m is written back. This is the dedicated coprocessor issued on the core's own port
  // (vs op 81's multi-cycle field-wise SUM loop). cj_l caps at 4 (the block width).
  reg          cj;                    // crdt-join in progress (stalls decode)
  reg          cj_ph;                 // 0 = read/assemble lanes, 1 = write the join back
  reg  [2:0]   cj_rp;                 // read sub-phase (synchronous dmem read: issue, then consume)
  reg  [HPW-1:0] cj_dst, cj_src;      // target/source lane-array heap base pointers
  reg  [7:0]   cj_l, cj_i;            // lane count / lane index (<=4; also selects the 32-bit slice of cj_a/cj_b)
  reg  [127:0] cj_a, cj_b;            // assembled {lane3..lane0} inputs to the block
  wire [127:0] cj_m;                  // per-replica MAX (the join)
  wire [33:0]  cj_value;              // sum of merged lanes (unused here)
  crdt_merge u_cm(.a(cj_a), .b(cj_b), .merged(cj_m), .value(cj_value));

  // 256-BIT BIGINT LANE (BigIntWide, op 114): an 8×256-bit wide register bank routed to the proven
  // bigint_alu/bigint_div. NO_BIGINT drops the whole lane (the benchmark subset never issues op 114) so
  // the wide core routes on nextpnr-xilinx without the 256-bit DSP cascade — the small control regs below
  // stay declared (their resets/defaults are unconditional) and are dead-code-eliminated in that build.
  reg          wf_we;                 // wide-bank write enable
  reg  [2:0]   wf_wa;                 // wide-bank write address
  reg  [255:0] wf_wd;                 // wide-bank write data
  reg          bd_start, bd_busy;
  reg  [2:0]   bd_a, bd_b, bd_dst;
`ifndef NO_BIGINT
  reg  [255:0] wide [0:7];            // the wide register bank
  wire [255:0] bi_a = wide[ra[2:0]];  // operand A (combinational read of the wide bank)
  wire [255:0] bi_b = wide[rb[2:0]];  // operand B
  wire [255:0] bi_sum, bi_diff;
  wire         bi_cout, bi_borrow, bi_eq, bi_lt;
  bigint_alu u_bi(.a(bi_a), .b(bi_b), .sum(bi_sum), .cout(bi_cout),
                  .diff(bi_diff), .borrow(bi_borrow), .eq(bi_eq), .lt(bi_lt));
  wire [255:0] bi_mul = bi_a * bi_b;  // BigIntWide Mul (kind 6): low 256 bits of a*b (schoolbook, 1-cycle)
  wire [255:0] bd_q, bd_r;            // BigIntWide Div (kind 5): sequential 256-bit bigint_div (256 cycles)
  wire         bd_done;
  bigint_div #(.W(256)) u_bd(.clk(clk), .rst(rst), .start(bd_start),
                             .a(wide[bd_a]), .b(wide[bd_b]), .q(bd_q), .r(bd_r), .done(bd_done));
`endif

  // ── FLOAT DATAPATH: a 64-bit float bank + the proven IEEE-754 f64 FPU (fpu.v) ──────────────────────
  // A parallel 64-bit float bank, SAME index space as the int regfile (VM register R holds either an int
  // in regs[R] or an f64 in fregs[R], disambiguated by the statically-lowered op type). Two SYNCHRONOUS-
  // read block-RAM mirrors give the two float read ports (fav/fbv), valid ONE cycle after their address —
  // the SAME operand bubble as the int file, so a combinational float op fires when operands_ready. One
  // write port (ff_we/ff_wa/ff_wd) drives both mirrors. Float opcodes are the chip-synthetic 201..=212
  // (above the ISA table like LoadConstWide=200), matching codegen_sva::full_asm + run_soft_core_typed.
  // SILICON ROBUSTNESS: the 64-bit float bank is stored as TWO 32-bit BRAMs per read port (a low half and a
  // high half), exactly mirroring the proven-on-silicon 32-bit int regfile (rf0/rf1/rf2). A single 64-bit-wide
  // block-RAM inference through the yosys/nextpnr/prjxray open flow can misbuild (width/2×RAMB36 mapping) so
  // every float value read back as 0 on real xc7 while iverilog sim stayed correct. Splitting into two
  // independent 32-bit BRAMs — the exact width proven to work — forces the safe mapping. The write port fans
  // ff_wd[31:0]→lo and ff_wd[63:32]→hi (both on ff_we); the read reconstructs fav={hi,lo} registered.
  (* ram_style = "block" *) reg [31:0] frf0_lo [0:REGS-1];  // float read port A, low 32 bits  -> fav[31:0]
  (* ram_style = "block" *) reg [31:0] frf0_hi [0:REGS-1];  // float read port A, high 32 bits -> fav[63:32]
  (* ram_style = "block" *) reg [31:0] frf1_lo [0:REGS-1];  // float read port B, low 32 bits  -> fbv[31:0]
  (* ram_style = "block" *) reg [31:0] frf1_hi [0:REGS-1];  // float read port B, high 32 bits -> fbv[63:32]
  reg          ff_we;                 // float register write enable (this cycle)
  reg [RW-1:0] ff_wa;                 // float register write address (defaults to dst)
  reg [63:0]   ff_wd;                 // float register write data
  // FConst (op 201) SEQUENTIALIZER. imem is sync-read block-RAM with ONE data port (imdata_q), so its two
  // 32-bit words can't both be read in one cycle. This 1-bit FSM splits them: dispatch latches the low word
  // (imdata_q = imem[pc+1], the default data address) and steers the data port to imem[pc+2]; the fc step
  // reads that high word off imdata_q. pc HOLDS through the FSM; the value written is bit-identical.
  reg          fc;                    // FConst in progress (stalls decode; pc holds)
  reg [31:0]   fc_w1;                 // latched low word (imem[pc+1]) — the high word is read in the fc step
  reg [RW-1:0] fa_ra, fa_rb;          // float read addresses (default: instr ra/rb; FSMs may override)
  reg [63:0]   fav, fbv;              // REGISTERED float read data — valid the cycle after fa_ra/fa_rb
  // fp_cmp stays COMBINATIONAL (no shifters). Its lt/eq bits are latched into an int reg on the next edge
  // like every other write; fed the two float operands fav/fbv.
  wire         fp_lt, fp_eq;          // fp_cmp results (1-bit each)
  fp_cmp    u_fcm(.a(fav), .b(fbv), .lt(fp_lt), .eq(fp_eq));
  // FLOAT min/max coprocessor (CallBuiltin op 38, ordinals 37/38 — `min`/`max` on floats, and the two-way
  // FMax the `is approximately` decomposition emits). It needs BOTH operands as floats: fregs[ra] and
  // fregs[ra+1] (the encoder places the two arguments in CONSECUTIVE float registers, matching the model's
  // `fregs[ra]`/`fregs[ra+1]` read). fbv defaults to fregs[rb]=the ORDINAL at dispatch, so a second cycle
  // re-reads fregs[ra+1] into fbv (like the int `cb2` path re-reads regs[args_start+1]). fcb picks max vs
  // min; the result matches Rust `f64::max`/`f64::min` incl. NaN (if one operand is NaN return the OTHER —
  // fp_cmp gives lt=eq=0 on any NaN, so an explicit NaN test drives the select).
  reg          fcb;                   // FMin/FMax second cycle in progress (pc holds; stalls dispatch)
  reg          fcb_max;               // 1 = FMax (ordinal 38), 0 = FMin (ordinal 37)
  reg [RW-1:0] fcb_dst;               // result float register
  wire         fav_is_nan = (fav[62:52] == 11'h7FF) && (fav[51:0] != 52'd0);
  wire         fbv_is_nan = (fbv[62:52] == 11'h7FF) && (fbv[51:0] != 52'd0);
  // Multi-cycle FPU units: add/sub, mul, digit-recurrence div (~56 cyc), sqrt (~59 cyc). Add/sub and mul
  // were sequentialized (a small shifter-staging FSM per unit) so FPGA synth memory stays bounded; ALL
  // four now use a start/done handshake. The core STALLS (pc holds → fav/fbv stay stable) from the issue
  // pulse until `done`, then writes fregs[dst]. `fsub` is a CONTINUOUS assign off the decoded opcode
  // (203 = FSub); pc holds while fp_addsub runs, so the opcode (hence fsub) stays stable across the FSM.
  wire         fsub = (opcode == 8'd203); // fp_addsub op select: 0 = FAdd, 1 = FSub
  reg          fas_start, fmul_start, fdiv_start, fsqrt_start; // one-cycle issue pulses
  reg          fpm_busy;                // a multi-cycle float op is running (stalls dispatch)
  reg  [1:0]   fpm_sel;                 // which unit: 0 = addsub, 1 = mul, 2 = div, 3 = sqrt
  reg [RW-1:0] fpm_dst;                 // result float register
  wire [63:0]  fp_as_y, fp_mul_y, fp_div_y, fp_sqrt_y;
  wire         fp_as_done, fp_mul_done, fp_div_done, fp_sqrt_done;
  fp_addsub u_fas(.clk(clk), .rst(rst), .start(fas_start),  .a(fav), .b_in(fbv), .sub(fsub), .y(fp_as_y),   .done(fp_as_done));
  fp_mul    u_fml(.clk(clk), .rst(rst), .start(fmul_start), .a(fav), .b(fbv),                 .y(fp_mul_y),  .done(fp_mul_done));
  fp_div  u_fdv(.clk(clk), .rst(rst), .start(fdiv_start),  .a(fav), .b(fbv), .y(fp_div_y),  .done(fp_div_done));
  fp_sqrt u_fsq(.clk(clk), .rst(rst), .start(fsqrt_start), .a(fav),         .y(fp_sqrt_y), .done(fp_sqrt_done));

  // ── Int ↔ Float conversions (the boundary the encoder inserts around a mixed op, e.g. `2.0 * k`). ──
  // i32_to_f64: an i32 fits EXACTLY in f64 (53-bit mantissa > 32 bits), so this is a lossless normalize.
  // f64_to_i32: truncation toward zero (Rust's `as i32` for in-range values), used by FToInt.
  function [63:0] i32_to_f64(input [31:0] n);
    reg        s;
    reg [31:0] m;         // magnitude
    integer    lead;      // position of the leading 1 (0..31)
    reg [10:0] be;        // biased exponent
    reg [51:0] frac;
    integer    i;
    reg [63:0] shifted;
    begin
      if (n == 32'd0) i32_to_f64 = 64'd0;
      else begin
        s = n[31];
        m = s ? (~n + 32'd1) : n;               // abs
        lead = 0;
        for (i = 31; i >= 0; i = i - 1) if (m[i] && lead == 0) lead = i;
        be = 11'd1023 + lead[10:0];
        // place the significand so the leading 1 sits at bit 52, then take the low 52 bits as the fraction
        shifted = ({32'd0, m}) << (52 - lead);  // leading 1 now at bit 52
        frac = shifted[51:0];
        i32_to_f64 = {s, be, frac};
      end
    end
  endfunction

  function [31:0] f64_to_i32(input [63:0] x);
    reg        s;
    reg [10:0] e;
    reg [52:0] sig;       // 1.frac (implicit bit + 52)
    integer    ue;        // unbiased exponent
    reg [63:0] mag;       // integer magnitude before sign
    begin
      s = x[63]; e = x[62:52];
      if (e < 11'd1023) f64_to_i32 = 32'd0;     // |x| < 1 -> 0
      else begin
        sig = {1'b1, x[51:0]};
        ue = e - 11'd1023;                       // 0..; the value is sig * 2^(ue-52)
        if (ue >= 52) mag = {11'd0, sig} << (ue - 52);
        else          mag = {11'd0, sig} >> (52 - ue);
        f64_to_i32 = s ? (~mag[31:0] + 32'd1) : mag[31:0];
      end
    end
  endfunction

  integer k;
  initial begin
    pc = 0; halted = 0; out_valid = 0; out = 0; outf_valid = 0; outf = 0; sp = 0; hp = 0; isp = 0; sel_n = 0; peer_seq = 0; trapped = 0; reasoned = 0; bld = 0; drng = 0; itr = 0; ld = 0; dm_ra = 0; dm_rq = 0; b_ph = 0; i_rp = 0; i_m = 0; i_found = 0; ms = 0; ms_ph = 0; ms_found = 0; div_busy = 0; div_start = 0; cj = 0; cj_ph = 0; cj_rp = 0; cj_i = 0; bd_busy = 0; bd_start = 0;
    outs = 0; outs_valid = 0; outs_first = 0; stx = 0; stx_ph = 0; stx_ptr = 0; stx_len = 0; stx_i = 0; tc_off = 0; teq = 0; teq_ph = 0; teq_res = 0; fmtf = 0; fmtf_n = 0; fmtf_k = 0; fmtf_neg = 0;
    outw = 0; outw_valid = 0;
    pc_q = 16'hffff; pc_qq = 16'hffff; instr_q = 0; imdata_q = 0; im_rb = 0; av = 0; bv = 0; dv = 0; a_ra = 0; a_rb = 0; a_dst = 0; sv = 0; sr_ra = 0;
    cl = 0; cl_ph = 0; cl_j = 0; cl_seed_ph = 0; rt = 0; rt_ph = 0; rt_j = 0; rt_hasrv = 0; rsw = 0; cb2 = 0; cl_needentry = 0; p2w = 0;
    fav = 0; fbv = 0; fa_ra = 0; fa_rb = 0; fpm_busy = 0; fas_start = 0; fmul_start = 0; fdiv_start = 0; fsqrt_start = 0; fpm_sel = 0; fc = 0; fc_w1 = 0;
    fcb = 0; fcb_max = 0; fcb_dst = 0;
    fdm_ra = 0; fdm_rq = 0; fdm_we = 0; fdm_wa = 0; fdm_wd = 0;
    sfr_wa = 0; sfr_wd = 0; sfr_we = 0; sfr_ra = 0; sfv = 0; rt_frv = 0;
    // Zero the whole register file (all three mirrors) + the float bank — the model starts every register at 0.
    for (k = 0; k < REGS; k = k + 1) begin rf0[k] = 0; rf1[k] = 0; rf2[k] = 0; globals[k] = 0; frf0_lo[k] = 0; frf0_hi[k] = 0; frf1_lo[k] = 0; frf1_hi[k] = 0; end
    // The float-heap plane starts all zeros (the model's fheap grows lazily filled with 0.0).
    for (k = 0; k < HEAP; k = k + 1) begin fdmem_lo[k] = 0; fdmem_hi[k] = 0; end
    ibox_val[0] = 0; ibox_val[1] = 0; ibox_val[2] = 0; ibox_val[3] = 0;
    // Boot ROM: the compiled program (a `binisa`-opcode chip-word image). The harness may overwrite
    // `imem` for other programs; for synthesis this bakes the loop demo in as a ROM.
    $readmemh("program.hex", imem);
  end

`ifdef SOFT_CORE_PROG_LOAD
  // Program-load port (SoC opt-in via SOFT_CORE_PROG_LOAD): the SoC streams instruction words in
  // while the core is held in reset. Guarded so the standalone build keeps imem a read-only boot
  // ROM ($readmemh only) — making it writable here would force imem out of ROM into registers and
  // inflate the fit. imem has no other writer, so there is no port conflict.
  always @(posedge clk) if (prog_we) imem[prog_addr] <= prog_data;
`endif

  always @(posedge clk) begin
    if (rst) begin
      pc <= 0; halted <= 0; out_valid <= 0; outf_valid <= 0; sp <= 0; hp <= 0; isp <= 0; trapped <= 0; reasoned <= 1'b0; bld <= 1'b0; drng <= 1'b0; itr <= 1'b0; ld <= 1'b0; b_ph <= 1'b0; i_rp <= 3'd0; ms <= 1'b0; ms_ph <= 3'd0; ms_cow <= 1'b0; ms_crp <= 1'b0; msi <= 1'b0; msi_ph <= 1'b0; div_busy <= 1'b0; div_start <= 1'b0; cj <= 1'b0; cj_ph <= 1'b0; bd_busy <= 1'b0; bd_start <= 1'b0; pow_busy <= 1'b0;
      pc_q <= 16'hffff; pc_qq <= 16'hffff; cl <= 1'b0; cl_ph <= 2'd0; cl_seed_ph <= 2'd0; rt <= 1'b0; rt_ph <= 2'd0; rsw <= 1'b0; cb2 <= 1'b0; p2w <= 1'b0;
      fpm_busy <= 1'b0; fas_start <= 1'b0; fmul_start <= 1'b0; fdiv_start <= 1'b0; fsqrt_start <= 1'b0; fc <= 1'b0; fcb <= 1'b0;
      peer_seq <= 32'd0;
      outs_valid <= 1'b0; outs_first <= 1'b0; stx <= 1'b0; stx_ph <= 3'd0; teq <= 1'b0; teq_ph <= 3'd0; fmtf <= 1'b0;
      tcs <= 1'b0; tcs_ph <= 3'd0; tcs_rp <= 1'b0; pin <= 1'b0; pin_ph <= 3'd0; pin_trim <= 1'b0; tbx <= 1'b0; tbx_ph <= 2'd0;
      outw_valid <= 1'b0;
      ibox_val[0] <= 1'b0; ibox_val[1] <= 1'b0; ibox_val[2] <= 1'b0; ibox_val[3] <= 1'b0;
`ifdef SOFT_CORE_DATA_LOAD
      // Preload the DATA SEGMENT (weights) into the heap while held in reset, and seed the bump
      // allocator past it. This later `hp` assignment overrides the `hp <= 0` above (last-write wins).
      hp <= hp_init[HPW-1:0];
      if (data_we) dmem[data_addr[HPW-1:0]] <= data_data;
`endif
    end else begin
      out_valid <= 1'b0;                 // default: no output this cycle
      outf_valid <= 1'b0;                // default: no float output this cycle
      outs_valid <= 1'b0;                // default: no string output this cycle
      outs_first <= 1'b0;
      outw_valid <= 1'b0;                // default: no word32 output this cycle
      if (!halted) begin
        rf_we = 1'b0; rf_wa = dst[RW-1:0]; rf_wd = 32'd0;   // register write-port defaults; arms set rf_we/rf_wd
        dm_we = 1'b0; dm_wa = 8'd0; dm_wd = 32'd0;       // dmem write-port defaults; arms set dm_we/dm_wa/dm_wd
        wf_we = 1'b0; wf_wa = 3'd0; wf_wd = 256'd0;      // wide-bank write-port defaults; the BigInt lane sets them
        ff_we = 1'b0; ff_wa = dst[RW-1:0]; ff_wd = 64'd0; // float write-port defaults (fsub is a continuous wire)
        dm_ra = 8'd0;                                    // dmem read-address default; load-issue arms set it
        fdm_we = 1'b0; fdm_wa = {HPW{1'b0}}; fdm_wd = 64'd0; // float-heap write-port defaults; 213/215 set them
        fdm_ra = {HPW{1'b0}};                            // float-heap read-address default; 214 sets it
        // BRAM read-port addresses DEFAULT to the current instruction's fields; an FSM that needs to read
        // another register overrides a_ra/a_rb/a_dst (its registered value lands next cycle). savedregs read
        // address defaults idle; the Call/Return engines drive it.
        a_ra = ra[RW-1:0]; a_rb = rb[RW-1:0]; a_dst = dst[RW-1:0];
        fa_ra = ra[RW-1:0]; fa_rb = rb[RW-1:0];          // float read-port addresses default to the instr fields
        im_rb = pc[IAW-1:0] + 1'b1;                      // imem data port defaults to the immediate/second word (pc+1); FSMs override to pc+2 / the text pool
        sr_we = 1'b0; sr_wa = {SAW{1'b0}}; sr_wd = 32'd0; sr_ra = {SAW{1'b0}};
        sfr_we = 1'b0; sfr_wa = {SAW{1'b0}}; sfr_wd = 64'd0; sfr_ra = {SAW{1'b0}}; // float call-stack port defaults
        div_start <= 1'b0;                               // the divider start is a one-cycle pulse
        bd_start <= 1'b0;                                // the bigint divider start is a one-cycle pulse
        fas_start <= 1'b0; fmul_start <= 1'b0; fdiv_start <= 1'b0; fsqrt_start <= 1'b0; // float issue pulses (one cycle)
        if (cl) begin
          // CALL frame engine — SEQUENTIAL over the BRAM regfile + BRAM call stack (pc holds throughout):
          //   ph0 SAVE : savedregs[sp*REGS + j] <= regs[j]  for j in 0..REGS  (snapshot the caller window)
          //   ph1 SEED : regs[j] <= (j<arg_count) ? saved_caller[args_start+j] : 0  (arg-copy — reads the
          //              just-written snapshot, so the register file is never read while being overwritten)
          //   ph2 FIN  : retpc/retdst latched, sp++, jump to entry.
          // Registered reads (dv, sv): each register is a strict 2-step — issue the address (cl_seed_ph 0),
          // consume the registered datum + write (cl_seed_ph 1). Unambiguous latency over pipelined speed.
          if (cl_ph == 2'd0) begin                                   // SAVE: regs[cl_j] (via dv) -> savedregs
            // CallValue: the closure entry was read from dmem at dispatch (dm_rq valid the FIRST SAVE cycle).
            if (cl_needentry) begin cl_entry <= dm_rq[15:0]; cl_needentry <= 1'b0; end
            if (cl_seed_ph == 2'd0) begin
              a_dst = cl_j[RW-1:0]; fa_ra = cl_j[RW-1:0]; cl_seed_ph <= 2'd1; // issue: present regs[cl_j] (dv) AND fregs[cl_j] (fav)
            end else begin
              sr_we = 1'b1; sr_wa = sp[$clog2(FRAMES)-1:0] * REGS + cl_j[RW-1:0]; sr_wd = dv; // consume dv=regs[cl_j]
              sfr_we = 1'b1; sfr_wa = sp[$clog2(FRAMES)-1:0] * REGS + cl_j[RW-1:0]; sfr_wd = fav; // snapshot fregs[cl_j] too
              cl_seed_ph <= 2'd0;
              if (cl_j == REGS - 1) begin cl_ph <= (cl_mode <= 2'd1) ? 2'd1 : 2'd2; cl_j <= {(RW+1){1'b0}}; end // Call/CallValue seed; Spawn skips to FIN
              else cl_j <= cl_j + 1'b1;
            end
          end else if (cl_ph == 2'd1) begin                          // SEED: (cl_j<ac) ? saved[as+cl_j] -> regs[cl_j] (BOTH banks)
            if (cl_seed_ph == 2'd0) begin
              sr_ra = sp[$clog2(FRAMES)-1:0] * REGS + cl_as[RW-1:0] + cl_j[RW-1:0];
              sfr_ra = sp[$clog2(FRAMES)-1:0] * REGS + cl_as[RW-1:0] + cl_j[RW-1:0]; cl_seed_ph <= 2'd1; // issue snapshot reads (int + float)
            end else begin
              // Call (mode 0) writes every register — args from saved[as+j], the rest ZEROED (a fresh window).
              // CallValue (mode 1) only writes the args and LEAVES the non-arg registers untouched: the callee
              // runs in the caller's window (its captures live there), so a non-arg write must not clobber it.
              if (cl_j < {1'b0, cl_ac}) begin
                rf_we = 1'b1; rf_wa = cl_j[RW-1:0]; rf_wd = sv;   // consume sv=saved caller[as+j]
                ff_we = 1'b1; ff_wa = cl_j[RW-1:0]; ff_wd = sfv;  // seed the float arg too (a float param rides fregs)
              end else if (cl_mode == 2'd0) begin
                rf_we = 1'b1; rf_wa = cl_j[RW-1:0]; rf_wd = 32'd0; // Call zero-fills the non-arg window
                ff_we = 1'b1; ff_wa = cl_j[RW-1:0]; ff_wd = 64'd0;
              end
              cl_seed_ph <= 2'd0;
              if (cl_j == REGS - 1) cl_ph <= 2'd2;
              else cl_j <= cl_j + 1'b1;
            end
          end else begin                                             // FIN: retpc/retdst, sp++, jump to entry
            retpc[sp[$clog2(FRAMES)-1:0]] <= cl_retpc; retdst[sp[$clog2(FRAMES)-1:0]] <= cl_retdst;
            sp <= sp + 1'b1; pc <= cl_entry; cl <= 1'b0;
          end
        end else if (rt) begin
          // RETURN frame engine — SEQUENTIAL restore from frame rt_fp (= latched sp-1), pc holds:
          //   ph0 RESTORE: regs[j] <= savedregs[rt_fp*REGS + j]  for j in 0..REGS  (strict 2-step per reg)
          //   ph1 RESULT : (Return 41 only) regs[retdst] <= rt_rv  (the captured callee return value)
          //   ph2 FIN    : sp--, jump to retpc.
          if (rt_ph == 2'd0) begin
            if (cl_seed_ph == 2'd0) begin
              sr_ra = rt_fp[$clog2(FRAMES)-1:0] * REGS + rt_j[RW-1:0];
              sfr_ra = rt_fp[$clog2(FRAMES)-1:0] * REGS + rt_j[RW-1:0]; cl_seed_ph <= 2'd1;   // issue savedregs + savedfregs reads
            end else begin
              rf_we = 1'b1; rf_wa = rt_j[RW-1:0]; rf_wd = sv;                                 // consume sv=savedregs[rt_fp][rt_j]
              ff_we = 1'b1; ff_wa = rt_j[RW-1:0]; ff_wd = sfv;                                 // restore the caller's float window too
              cl_seed_ph <= 2'd0;
              if (rt_j == REGS - 1) rt_ph <= (rt_hasrv ? 2'd1 : 2'd2);
              else rt_j <= rt_j + 1'b1;
            end
          end else if (rt_ph == 2'd1) begin                          // place the return value AFTER the restore (BOTH banks)
            rf_we = 1'b1; rf_wa = cl_retdst; rf_wd = rt_rv;
            ff_we = 1'b1; ff_wa = cl_retdst; ff_wd = rt_frv; rt_ph <= 2'd2;                    // the caller reads the correctly-typed one
          end else begin                                             // FIN: sp--, jump to caller's return pc
            sp <= sp - 1'b1; pc <= cl_retpc; rt <= 1'b0;
          end
        end else if (p2w) begin
          // Pending SECOND register write (pc-neutral) — SelectWait writes both the received value AND the
          // fired-arm index; the drng drain that follows advances pc, so this one must not.
          rf_we = 1'b1; rf_wa = p2w_wa; rf_wd = p2w_wd; p2w <= 1'b0;
        end else if (rsw) begin
          // RevSwap second write: regs[dst] <= old regs[ra] (staged last cycle). pc advances now.
          rf_we = 1'b1; rf_wa = rsw_wa; rf_wd = rsw_wd; rsw <= 1'b0; pc <= pc + 8'd1;
        end else if (cb2) begin
          // CallBuiltin two-operand finish: bv now holds regs[args_start+1] (issued last cycle). Retire +
          // advance. Min(7)/Max(8) are signed selects; the WORD32 ops treat cb_av's 32 bits AS UNSIGNED —
          // Rotl(30)/Rotr(31) are barrel rotations (count = bv[4:0], mod 32), Word32Shr(32) is a LOGICAL
          // right shift (zero-fill). Bit-exact to logicaffeine_base::Word32 (rotate_left/right, wrapping_shr).
          rf_we = 1'b1; rf_wa = cb_dst;
          rf_wd = (cb_op == 8'd7)  ? (($signed(cb_av) < $signed(bv)) ? cb_av : bv)
                : (cb_op == 8'd8)  ? (($signed(cb_av) > $signed(bv)) ? cb_av : bv)
                : (cb_op == 8'd30) ? ((cb_av << bv[4:0]) | (cb_av >> ((5'd0 - bv[4:0]) & 5'h1f))) // Rotl
                : (cb_op == 8'd31) ? ((cb_av >> bv[4:0]) | (cb_av << ((5'd0 - bv[4:0]) & 5'h1f))) // Rotr
                :                    (cb_av >> bv[4:0]);                                          // Word32Shr (logical)
          cb2 <= 1'b0; pc <= pc + 8'd1;
        end else if (div_busy) begin
          // Stall while the shared divider runs (pc holds → av/bv are the stable operands). On done,
          // retire: Mod → remainder; FloorDiv → floor-adjusted quotient; Div/ExactDiv → quotient.
          // `!div_start` skips the pulse cycle, by which the divider has cleared any STALE `done`
          // held in IDLE from the previous divide (else we would retire the prior result).
          if (div_done && !div_start) begin
            rf_we = 1'b1; rf_wa = div_dst;
            rf_wd = (div_op == 8'd15) ? div_r
                  : (div_op == 8'd14) ? (div_q - (((div_r != 32'd0) && (av[31] ^ bv[31])) ? 32'd1 : 32'd0))
                  : div_q;
            pc <= pc + 8'd1; div_busy <= 1'b0;
          end
        end else if (pow_busy) begin
          // Stall while the sequential power unit runs (pc holds → the operands stay stable). Each cycle
          // multiplies the accumulator by the base and counts the exponent down; when it reaches 0 the
          // result (base**exp, wrapping 32-bit — acc initialized to 1 covers exp==0) is retired to pow_dst.
          if (pow_cnt == 32'd0) begin
            rf_we = 1'b1; rf_wa = pow_dst; rf_wd = pow_acc;
            pc <= pc + 8'd1; pow_busy <= 1'b0;
          end else begin
            pow_acc <= pow_acc * pow_base;
            pow_cnt <= pow_cnt - 32'd1;
          end
        end else if (fpm_busy) begin
          // Stall while the multi-cycle FPU (addsub ~6 cyc / mul ~4 cyc / div ~56 cyc / sqrt ~59 cyc) runs
          // — pc holds, so fav/fbv stay the stable operands. On done, write the result to fregs[dst] and
          // advance. The `!*_start` guard skips the issue-pulse cycle so a STALE `done` held in the unit's
          // IDLE isn't retired. fpm_sel picks the unit: 0=addsub, 1=mul, 2=div, 3=sqrt.
          case (fpm_sel)
            2'd0: if (fp_as_done   && !fas_start)   begin ff_we = 1'b1; ff_wa = fpm_dst; ff_wd = fp_as_y;   pc <= pc + 8'd1; fpm_busy <= 1'b0; end
            2'd1: if (fp_mul_done  && !fmul_start)  begin ff_we = 1'b1; ff_wa = fpm_dst; ff_wd = fp_mul_y;  pc <= pc + 8'd1; fpm_busy <= 1'b0; end
            2'd2: if (fp_div_done  && !fdiv_start)  begin ff_we = 1'b1; ff_wa = fpm_dst; ff_wd = fp_div_y;  pc <= pc + 8'd1; fpm_busy <= 1'b0; end
            2'd3: if (fp_sqrt_done && !fsqrt_start) begin ff_we = 1'b1; ff_wa = fpm_dst; ff_wd = fp_sqrt_y; pc <= pc + 8'd1; fpm_busy <= 1'b0; end
          endcase
        end else if (fcb) begin
          // FLOAT min/max FINISH: fbv now holds fregs[ra+1] (re-issued last cycle), fav still fregs[ra] (pc
          // held → fa_ra defaulted to ra). Select per Rust f64::max/min: if either operand is NaN return the
          // OTHER; else max = (fav<fbv ? fbv : fav), min = (fav<fbv ? fav : fbv). fp_lt is the combinational
          // fp_cmp(fav,fbv) (0 on any NaN, so the NaN tests take priority). Write fregs[dst], advance.
          ff_we = 1'b1; ff_wa = fcb_dst;
          ff_wd = fav_is_nan ? fbv
                : fbv_is_nan ? fav
                : fcb_max    ? (fp_lt ? fbv : fav)   // FMax
                :              (fp_lt ? fav : fbv);  // FMin
          fcb <= 1'b0; pc <= pc + 8'd1;
        end
        else if (stx) begin
          // SHOWTEXT streaming FSM (pc holds): read the char list at stx_ptr and burst it over `outs`. The
          // read must be RE-ISSUED every cycle it is needed the next (dm_ra defaults to 0 each cycle), so each
          // phase both consumes last cycle's `dm_rq` AND issues the read whose result the next phase uses.
          //   ph0 : latch length (dm_rq from dispatch); re-issue char-1's read
          //   ph1 : emit the LENGTH word (outs_first); keep char-1's read live for ph2
          //   ph2 : emit dm_rq (the current codepoint); issue the NEXT char, until all len chars are sent
          if (stx_ph == 3'd0) begin
            stx_len <= dm_rq; dm_ra = stx_ptr + 8'd1;      // latch length; issue char 1
            stx_i <= 32'd1; stx_ph <= 3'd1;
          end else if (stx_ph == 3'd1) begin
            outs <= stx_len; outs_valid <= 1'b1; outs_first <= 1'b1; // the length word (host allocates the string)
            if (stx_len == 32'd0) begin pc <= pc + 8'd1; stx <= 1'b0; end // empty string: nothing to stream
            else begin dm_ra = stx_ptr + 8'd1; stx_ph <= 3'd2; end // KEEP char-1's read live for ph2
          end else begin
            outs <= dm_rq; outs_valid <= 1'b1;             // stream one codepoint (dm_rq = dmem[stx_ptr+stx_i])
            if (stx_i >= stx_len) begin pc <= pc + 8'd1; stx <= 1'b0; end
            else begin stx_i <= stx_i + 32'd1; dm_ra = stx_ptr + stx_i[HPW-1:0] + 8'd1; end // issue the next char
          end
        end
        else if (teq) begin
          // TEXTEQ/TEXTNE content-compare FSM (pc holds): compare the two char lists by CONTENT.
          //   ph0 : read length p (dm_rq), issue length q
          //   ph1 : read length q; if lengths differ -> not equal (deliver in ph3); else start char scan
          //   ph2 : per index, read p[i] (ph2a) then q[i] (ph2b) and compare
          //   ph3 : deliver 0/1 (equality XOR the op)
          if (teq_ph == 3'd0) begin
            teq_lp <= dm_rq; dm_ra = teq_q; teq_ph <= 3'd1; // latch |p|; issue |q|
          end else if (teq_ph == 3'd1) begin
            teq_lq <= dm_rq;
            if (teq_lp != dm_rq) begin teq_res <= 1'b0; teq_ph <= 3'd3; end // lengths differ -> unequal
            else if (teq_lp == 32'd0) begin teq_res <= 1'b1; teq_ph <= 3'd3; end // both empty -> equal
            else begin teq_len <= teq_lp; teq_i <= 32'd1; teq_res <= 1'b1; dm_ra = teq_p + 8'd1; teq_ph <= 3'd2; i_rp <= 3'd0; end
          end else if (teq_ph == 3'd2) begin
            if (i_rp == 3'd0) begin i_val <= dm_rq; dm_ra = teq_q + teq_i[HPW-1:0]; i_rp <= 3'd1; end // p[i]; issue q[i]
            else begin
              if (dm_rq != i_val) teq_res <= 1'b0;         // a char differs -> unequal
              if (teq_i >= teq_len) teq_ph <= 3'd3;
              else begin teq_i <= teq_i + 32'd1; dm_ra = teq_p + teq_i[HPW-1:0] + 8'd1; i_rp <= 3'd0; end
            end
          end else begin
            rf_we = 1'b1; rf_wa = teq_dst; rf_wd = {31'd0, (teq_eqop ? teq_res : ~teq_res)};
            pc <= pc + 8'd1; teq <= 1'b0;
          end
        end
        else if (tcs) begin
          // TEXTCONTAINS substring scan (pc holds): does `needle` occur as a contiguous codepoint run in
          // `haystack`? A double loop over the single synchronous read port, byte-identical to the model's
          //   if nl == 0 { found = 1 } else if nl <= hl { for start in 0..=hl-nl { for k in 0..nl … } }
          //   ph0 : latch |hay| (dm_rq), issue |needle|
          //   ph1 : latch |needle|; empty needle -> found; needle longer than hay -> not found; else start
          //         the window at start=0 by issuing hay[0]
          //   ph2 : per (start, k): rp0 read hay[start+k] (latch tcs_hc, issue needle[k]); rp1 compare with
          //         needle[k] -> on mismatch advance the OUTER window (or fail if past the last start),
          //         on match advance the INNER cursor (or succeed when the whole needle matched)
          //   ph3 : deliver 0/1
          if (tcs_ph == 3'd0) begin
            tcs_hl <= dm_rq[HPW-1:0]; dm_ra = tcs_np; tcs_ph <= 3'd1;         // |hay|; issue |needle|
          end else if (tcs_ph == 3'd1) begin
            tcs_nl <= dm_rq[HPW-1:0];
            if (dm_rq == 32'd0) begin tcs_res <= 1'b1; tcs_ph <= 3'd3; end    // empty needle is contained in everything
            else if (dm_rq[HPW-1:0] > tcs_hl) begin tcs_res <= 1'b0; tcs_ph <= 3'd3; end // needle longer than hay -> absent
            else begin
              tcs_start <= {HPW{1'b0}}; tcs_k <= {HPW{1'b0}}; tcs_rp <= 1'b0; // window 0, inner 0
              dm_ra = tcs_hp + 8'd1;                                          // issue hay[start=0 + k=0]
              tcs_ph <= 3'd2;
            end
          end else if (tcs_ph == 3'd2) begin
            if (!tcs_rp) begin
              tcs_hc <= dm_rq; dm_ra = tcs_np + 8'd1 + tcs_k;                 // latch hay[start+k]; issue needle[k]
              tcs_rp <= 1'b1;
            end else begin
              if (tcs_hc == dm_rq) begin                                      // this codepoint matches
                if (tcs_k + 8'd1 >= tcs_nl) begin tcs_res <= 1'b1; tcs_ph <= 3'd3; end // whole needle matched
                else begin tcs_k <= tcs_k + 8'd1; tcs_rp <= 1'b0; dm_ra = tcs_hp + 8'd1 + tcs_start + tcs_k + 8'd1; end // next inner char
              end else begin                                                 // mismatch -> slide the window
                if (tcs_start + 8'd1 > tcs_hl - tcs_nl) begin tcs_res <= 1'b0; tcs_ph <= 3'd3; end // no window left -> absent
                else begin tcs_start <= tcs_start + 8'd1; tcs_k <= {HPW{1'b0}}; tcs_rp <= 1'b0; dm_ra = tcs_hp + 8'd1 + tcs_start + 8'd1; end // hay[(start+1)+0]
              end
            end
          end else begin
            rf_we = 1'b1; rf_wa = tcs_dst; rf_wd = {31'd0, tcs_res};
            pc <= pc + 8'd1; tcs <= 1'b0;
          end
        end
        else if (pin) begin
          // PARSEINT decimal scan (pc holds): the Text char-list at pin_ptr -> a signed value, EXACTLY the
          // model's `str::trim().parse::<i64>()` boundary (32-bit datapath; the i64/overflow edges are the
          // model-level proof, the RTL corpus stays in the i32 range).
          //   ph0 : latch len (dm_rq); if 0 -> error (empty). Else issue char[1], start buffering.
          //   ph1 : buffer each codepoint into pin_buf[i-1]; when all len chars are in, TRIM whitespace off
          //         both ends (pin_lo/pin_hi window), read the optional leading sign, then go to the digit scan.
          //   ph2 : one buffered char per cycle: require an ASCII digit, accumulate acc = acc*10 - digit with
          //         a checked 32-bit overflow -> pin_err.
          //   ph3 : deliver acc's true sign, or TRAP (empty / no digit / non-digit / overflow).
          if (pin_ph == 3'd0) begin
            pin_len <= dm_rq[HPW-1:0];
            if (dm_rq == 32'd0) begin pin_err <= 1'b1; pin_ph <= 3'd3; end    // "" -> error
            else begin pin_i <= 8'd1; dm_ra = pin_ptr + 8'd1; pin_ph <= 3'd1; end // buffer from char[1]
          end else if (pin_ph == 3'd1) begin
            pin_buf[pin_i - 8'd1] <= dm_rq;                                    // stash codepoint i-1
            if (pin_i >= pin_len) begin
              // all chars buffered -> TRIM Unicode whitespace (the corpus stays within space/tab/nl/cr) off
              // both ends, then read the optional single leading sign, then start the digit scan at ph2.
              pin_ph <= 3'd2; pin_acc <= 32'd0; pin_saw <= 1'b0; pin_neg <= 1'b0; pin_err <= 1'b0; pin_trim <= 1'b0;
              // (the trim + sign are resolved at ph2's first entry, gated by pin_trim; we hand ph2 the raw
              //  buffer + the true length, and it walks lo..hi with the sign peeled)
              pin_lo <= {HPW{1'b0}}; pin_hi <= pin_len;
            end else begin pin_i <= pin_i + 8'd1; dm_ra = pin_ptr + 8'd1 + pin_i; end
          end else if (pin_ph == 3'd2) begin
            // First entry (pin_trim not yet set): resolve the trim window + sign ONCE, then fall into the
            // per-char digit walk using pin_i as the 0-based cursor over pin_buf (pin_hi may equal pin_len, so
            // a distinct `pin_trim` flag — not `pin_i == pin_len` — marks the resolve so the walk-end doesn't
            // re-trigger it).
            if (!pin_trim) begin
              pin_trim <= 1'b1;
              pin_lo = {HPW{1'b0}};
`ifndef SOFT_CORE_NO_PARSEINT
              // TRIM leading whitespace
              while (pin_lo < pin_hi &&
                     (pin_buf[pin_lo] == 32'd32 || pin_buf[pin_lo] == 32'd9 ||
                      pin_buf[pin_lo] == 32'd10 || pin_buf[pin_lo] == 32'd13)) pin_lo = pin_lo + 8'd1;
              // TRIM trailing whitespace
              while (pin_hi > pin_lo &&
                     (pin_buf[pin_hi - 8'd1] == 32'd32 || pin_buf[pin_hi - 8'd1] == 32'd9 ||
                      pin_buf[pin_hi - 8'd1] == 32'd10 || pin_buf[pin_hi - 8'd1] == 32'd13)) pin_hi = pin_hi - 8'd1;
`endif
              // SOFT_CORE_NO_PARSEINT (open-flow synth): the two whitespace-trim `while`s above are
              // data-dependent (yosys cannot unroll them). A build that never calls `parseInt` (e.g. the
              // transformer inference SoC) defines it to drop them — parseInt then skips whitespace trimming.
              // optional single leading sign
              if (pin_lo < pin_hi && pin_buf[pin_lo] == 32'd45) begin pin_neg <= 1'b1; pin_lo = pin_lo + 8'd1; end
              else if (pin_lo < pin_hi && pin_buf[pin_lo] == 32'd43) begin pin_neg <= 1'b0; pin_lo = pin_lo + 8'd1; end
              pin_i <= pin_lo;                                                 // cursor -> first digit candidate
            end else if (pin_i < pin_hi) begin
              if (pin_buf[pin_i] < 32'd48 || pin_buf[pin_i] > 32'd57) begin pin_err <= 1'b1; pin_ph <= 3'd3; end // non-digit -> error
              else begin
                pin_saw <= 1'b1;
                // acc = acc*10 - digit in the NEGATIVE magnitude, with a checked 32-bit overflow.
                pin_mul = pin_acc * 32'd10;
                pin_sub = pin_mul - (pin_buf[pin_i] - 32'd48);
                // overflow iff acc*10 doesn't divide back (acc != 0 and mul/10 != acc) or the subtract wrapped
                // ABOVE the (already non-positive) running value — i.e. the negative magnitude got smaller.
                if ((pin_acc != 32'd0 && ($signed(pin_mul) / 32'sd10) != $signed(pin_acc)) ||
                    $signed(pin_sub) > $signed(pin_mul)) begin pin_err <= 1'b1; pin_ph <= 3'd3; end
                else begin pin_acc <= pin_sub; pin_i <= pin_i + 8'd1; end
              end
            end else begin
              // walked the whole digit run
              if (!pin_saw) begin pin_err <= 1'b1; end                        // no digit at all -> error
              pin_ph <= 3'd3;
            end
          end else begin
            if (pin_err || !pin_saw) begin halted <= 1'b1; trapped <= 1'b1; end // trap: pre-trap output stands
            else begin
              // deliver the true sign: negative keeps the running magnitude; positive negates it (and would
              // overflow at i32::MIN's positive twin, matching the model's `checked_neg` — a model-level edge).
              if (pin_neg) begin rf_we = 1'b1; rf_wa = pin_dst; rf_wd = pin_acc; pc <= pc + 8'd1; pin <= 1'b0; end
              else if ($signed(pin_acc) == $signed(32'h80000000)) begin halted <= 1'b1; trapped <= 1'b1; end // +2^31 overflows i32
              else begin rf_we = 1'b1; rf_wa = pin_dst; rf_wd = (32'd0 - pin_acc); pc <= pc + 8'd1; pin <= 1'b0; end
            end
          end
        end
        else if (tbx) begin
          // TEXTBYTES UTF-8 encode (pc holds): char-list at tbx_ptr -> bytes at tbx_base+1…, header last.
          //   ph0 : latch len (dm_rq); an empty text goes straight to the header. Else issue char[1].
          //   ph1 : codepoint in (dm_rq): write its LEAD byte, and either issue the next char (1-byte code)
          //         or hold it in tbx_cp for ph2.
          //   ph2 : write continuation byte k = 0x80 | 6-bit group (n-1-k), most significant first; after the
          //         last one, issue the next char (ph1) or finish (ph3).
          //   ph3 : header = byte count, regs[dst] <- base, hp past the last byte.
          if (tbx_ph == 2'd0) begin
            tbx_len <= dm_rq[HPW-1:0];
            if (dm_rq == 32'd0) tbx_ph <= 2'd3;
            else begin tbx_i <= 8'd1; dm_ra = tbx_ptr + 8'd1; tbx_ph <= 2'd1; end
          end else if (tbx_ph == 2'd1) begin
            dm_we = 1'b1; dm_wa = tbx_w; tbx_w <= tbx_w + 8'd1; tbx_cp <= dm_rq;
            if (dm_rq < 32'h80) begin
              dm_wd = {24'd0, dm_rq[7:0]};
              if (tbx_i >= tbx_len) tbx_ph <= 2'd3;
              else begin tbx_i <= tbx_i + 8'd1; dm_ra = tbx_ptr + 8'd1 + tbx_i; end
            end else begin
              if (dm_rq < 32'h800)        begin dm_wd = {24'd0, 3'b110,  dm_rq[10:6]};  tbx_n <= 3'd2; end
              else if (dm_rq < 32'h10000) begin dm_wd = {24'd0, 4'b1110, dm_rq[15:12]}; tbx_n <= 3'd3; end
              else                        begin dm_wd = {24'd0, 5'b11110, dm_rq[20:18]}; tbx_n <= 3'd4; end
              tbx_k <= 3'd1; tbx_ph <= 2'd2;
            end
          end else if (tbx_ph == 2'd2) begin
            dm_we = 1'b1; dm_wa = tbx_w; tbx_w <= tbx_w + 8'd1;
            case (tbx_n - 3'd1 - tbx_k)
              3'd0:    dm_wd = {24'd0, 2'b10, tbx_cp[5:0]};
              3'd1:    dm_wd = {24'd0, 2'b10, tbx_cp[11:6]};
              default: dm_wd = {24'd0, 2'b10, tbx_cp[17:12]};
            endcase
            if (tbx_k + 3'd1 < tbx_n) tbx_k <= tbx_k + 3'd1;
            else if (tbx_i >= tbx_len) tbx_ph <= 2'd3;
            else begin tbx_i <= tbx_i + 8'd1; dm_ra = tbx_ptr + 8'd1 + tbx_i; tbx_ph <= 2'd1; end
          end else begin
            dm_we = 1'b1; dm_wa = tbx_base; dm_wd = {{ZPAD{1'b0}}, tbx_w - tbx_base} - 32'd1;
            rf_we = 1'b1; rf_wa = tbx_dst; rf_wd = {{ZPAD{1'b0}}, tbx_base};
            hp <= tbx_w; pc <= pc + 8'd1; tbx <= 1'b0;
          end
        end
        else if (fmtf) begin
          // FORMATVALUE digit-peel (pc holds): one ÷10/%10 per cycle. fmt_digit[fmtf_k] gets the next
          // least-significant digit; when the running value reaches 0 the buffer holds the digits LSB-first
          // (fmt_digit[0]=ones … fmt_digit[k-1]=MSB). Then, if negative, place '-' at [k], and hand the whole
          // char list to the uniform build-microcode (which writes it MSB-first with the leading sign).
          fmt_digit[fmtf_k] = 8'd48 + (fmtf_n % 32'd10);
          if (fmtf_n < 32'd10) begin                                   // this was the last (most-significant) digit
            if (fmtf_neg) fmt_digit[fmtf_k + 4'd1] = 8'd45;            // leading '-'
            // total chars = (fmtf_k+1 digits) + (neg ? 1 : 0)
            bld <= 1'b1; b_op <= 8'd64; b_base <= hp; b_i <= 8'd0;
            b_n <= {4'd0, fmtf_k} + (fmtf_neg ? 8'd1 : 8'd0) + 8'd1;   // b_n = count-1 for the 1-based build (header + b_n cells)
            fmt_len <= {28'd0, fmtf_k} + 32'd1;                        // number of DIGITS (sign excluded) — the build reverse-index base
            b_dst <= fmtf_dst;
            fmtf <= 1'b0;
          end else begin
            fmtf_n <= fmtf_n / 32'd10; fmtf_k <= fmtf_k + 4'd1;
          end
        end
`ifndef NO_BIGINT
        else if (bd_busy) begin
          // Stall while the 256-bit bigint_div runs (pc holds → the latched wide operands stay stable).
          // Retire the quotient to the wide bank on done; `!bd_start` skips the pulse cycle (stale done).
          if (bd_done && !bd_start) begin
            wf_we = 1'b1; wf_wa = bd_dst; wf_wd = bd_q;
            pc <= pc + 8'd1; bd_busy <= 1'b0;
          end
        end
`endif
        else if (cj) begin
          // CRDT LATTICE JOIN coprocessor: assemble the target's + source's lanes from dmem, feed the
          // `crdt_merge` block (combinational per-lane MAX), then write the join back. pc holds (stall).
          if (!cj_ph) begin                                  // READ phase: 2-phase synchronous read per lane
            if (cj_i <= cj_l) begin
              if (cj_rp == 3'd0) begin dm_ra = cj_dst + cj_i; cj_rp <= 3'd1; end
              else if (cj_rp == 3'd1) begin cj_a[(cj_i - 8'd1)*32 +: 32] <= dm_rq; dm_ra = cj_src + cj_i; cj_rp <= 3'd2; end
              else begin cj_b[(cj_i - 8'd1)*32 +: 32] <= dm_rq; cj_i <= cj_i + 8'd1; cj_rp <= 3'd0; end
            end else begin cj_ph <= 1'b1; cj_i <= 8'd1; end   // all lanes read -> WRITE phase
          end else begin                                     // WRITE phase: store the block's per-lane MAX
            if (cj_i <= cj_l) begin
              dm_we = 1'b1; dm_wa = cj_dst + cj_i; dm_wd = cj_m[(cj_i - 8'd1)*32 +: 32]; cj_i <= cj_i + 8'd1;
            end else begin pc <= pc + 8'd1; cj <= 1'b0; end
          end
        end else
        if (bld) begin
          // UNIFORM build-microcode: write dmem[b_base + b_i] one cell/cycle; header at b_i==0, elements after.
          // COPY builds (DeepClone/SliceOp/Concat) read a source dmem cell — a synchronous 2-phase step per
          // element: phase 0 issues dm_ra, phase 1 writes dm_rq. GATHER builds (NewList/NewTuple/NewInductive)
          // read a source REGISTER regs[b_src+b_i-1] — same 2-phase step but through read port C (dv), since
          // the register file is now a registered-read BRAM. Header cell (b_i==0) and computed builds skip it.
          if (!b_ph && ((b_op == 8'd66) || (((b_op == 8'd65) || (b_op == 8'd33) || (b_op == 8'd32)) && b_i != 8'd0))) begin
            dm_ra = ((b_op == 8'd33) || (b_op == 8'd32))
                    ? ((b_i <= b_len1) ? (b_csrc + b_i) : (b_csrc2 + b_i - b_len1))
                    : (b_csrc + b_i);
            b_ph <= 1'b1;                                 // dmem read phase: issue, don't write or advance
          end else if (!b_ph && ((b_op == 8'd48) || (b_op == 8'd67) || (b_op == 8'd77)) && b_i != 8'd0) begin
            a_dst = b_src + b_i[RW-1:0] - 1'b1;           // GATHER read phase: present regs[b_src+b_i-1] on port C
            b_ph <= 1'b1;
          end else if (!b_ph && (b_op == 8'd216) && b_i != 8'd0) begin
            im_rb = tc_off + b_i - 16'd1;                 // TEXT-POOL read phase: present imem[tc_off+b_i-1] on the data port (imem is sync-read now)
            b_ph <= 1'b1;
          end else begin
            dm_we = 1'b1; dm_wa = b_base + b_i;
            case (b_op)
              8'd48, 8'd67: dm_wd = (b_i == 8'd0) ? {{ZPAD{1'b0}}, b_n} : dv;                            // NewList/NewTuple gather (dv=regs[b_src+b_i-1])
              8'd74:        dm_wd = (b_i == 8'd0) ? {{ZPAD{1'b0}}, b_n} : 32'd0;                         // NewStruct (zeroed fields)
              8'd92:        dm_wd = 32'd0;                                                              // ChanNew ([head,count,8 slots] = 0)
              8'd64:        dm_wd = (b_i == 8'd0) ? {{ZPAD{1'b0}}, b_n}                                    // FormatValue: header = char count
                                    : (fmtf_neg && b_i == 8'd1) ? {24'd0, 8'd45}                          //   leading '-' (negative)
                                    : {24'd0, fmt_digit[fmt_len - b_i + (fmtf_neg ? 32'd1 : 32'd0)]};      //   digits MSB-first (reverse of the LSB-first buffer, sign-shifted)
              8'd216:       dm_wd = (b_i == 8'd0) ? {{ZPAD{1'b0}}, b_n} : imdata_q;                        // TextConst: header = len, then copy codepoints (imdata_q = imem[tc_off+b_i-1], read the prior cycle)
              8'd53:        dm_wd = (b_i == 8'd0) ? {{ZPAD{1'b0}}, b_n} : (b_lo + {{ZPAD{1'b0}}, b_i} - 32'd1); // NewRange (lo + i - 1)
              8'd235:       dm_wd = (b_i == 8'd0) ? {{ZPAD{1'b0}}, b_n} : b_fill;                             // RepeatSeq: header = count, every slot = the scalar element
              8'd77:        dm_wd = (b_i == 8'd0) ? b_hdr : dv;                                          // NewInductive (header = variant tag, then args gathered via dv)
              8'd66:        dm_wd = dm_rq;                                                               // DeepClone (source cell just read)
              8'd65:        dm_wd = (b_i == 8'd0) ? {{ZPAD{1'b0}}, b_n} : dm_rq;                         // SliceOp (header = len, copy from src+offset)
              8'd33, 8'd32: dm_wd = (b_i == 8'd0) ? b_hdr : dm_rq;                                      // SeqConcat/Concat (lhs then rhs)
              default:      dm_wd = 32'd0;
            endcase
            b_ph <= 1'b0;
            if (b_i >= b_n) begin                          // last cell written -> finish
              rf_we = 1'b1; rf_wa = b_dst; rf_wd = {{ZPAD{1'b0}}, b_base};
              // TextConst (op 216) is a TWO-word instruction (op word + pool offset), so pc advances by 2.
              hp <= b_base + b_n + 8'd1; pc <= pc + ((b_op == 8'd216) ? 16'd2 : 16'd1); bld <= 1'b0;
            end else b_i <= b_i + 8'd1;
          end
        end else if (drng) begin
          // Drain the write buffer: one staged (addr, data) to dmem per cycle, then resume.
          dm_we = 1'b1; dm_wa = wbuf_a[wbuf_i]; dm_wd = wbuf_d[wbuf_i];
          if (wbuf_i + 3'd1 >= wbuf_n) begin drng <= 1'b0; pc <= pc + 8'd1; end
          else wbuf_i <= wbuf_i + 3'd1;
        end else if (itr) begin
          // ITERATE FSM — SYNCHRONOUS reads: each per-element access is issued (i_rp==0) then consumed a
          // cycle later. The membership scans (Union/Intersect) run a SEQUENTIAL inner scan (i_m cursor) over
          // the other set through the single read port — the unified scan engine that replaces the old
          // unrolled `for m` combinational read.
          case (i_op)
            8'd81: begin                                                            // CrdtMerge: dmem[tgt+i] += dmem[src+i]
              if (i_si <= i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_dst + i_si; i_rp <= 3'd1; end                               // read target cell
                else if (i_rp == 3'd1) begin i_val <= dm_rq; dm_ra = i_s1 + i_si; i_rp <= 3'd2; end            // save target; read source cell
                else begin dm_we = 1'b1; dm_wa = i_dst + i_si; dm_wd = i_val + dm_rq; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else begin pc <= pc + 8'd1; itr <= 1'b0; end
            end
            8'd56: begin                                                            // RemoveFrom: ph0 = find first matching index (i_m); ph1 = shift down + length--
              if (!i_ph) begin
                if (i_si <= i_l1) begin
                  if (i_rp == 3'd0) begin dm_ra = i_dst + i_si; i_rp <= 3'd1; end
                  else begin if (dm_rq == i_val && i_m == 8'd0) i_m <= i_si; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
                end else if (i_m != 8'd0) begin i_ph <= 1'b1; i_si <= i_m; i_rp <= 3'd0; end                   // found -> shift from that index
                else begin pc <= pc + 8'd1; itr <= 1'b0; end                                                   // absent -> no-op
              end else if (i_si < i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_dst + i_si + 8'd1; i_rp <= 3'd1; end                         // read the next cell
                else begin dm_we = 1'b1; dm_wa = i_dst + i_si; dm_wd = dm_rq; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else begin
                dm_we = 1'b1; dm_wa = i_dst; dm_wd = {{ZPAD{1'b0}}, (i_l1 - 8'd1)};
                pc <= pc + 8'd1; itr <= 1'b0;
              end
            end
            8'd68: begin                                                            // Union: phase 0 = copy lhs, phase 1 = append rhs∉lhs
              if (!i_ph) begin
                if (i_si <= i_l1) begin
                  if (i_rp == 3'd0) begin dm_ra = i_s1 + i_si; i_rp <= 3'd1; end
                  else begin dm_we = 1'b1; dm_wa = i_dst + i_si; dm_wd = dm_rq; i_w <= i_si; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
                end else begin i_ph <= 1'b1; i_si <= 8'd1; i_rp <= 3'd0; end
              end else begin
                if (i_si <= i_l2) begin
                  if (i_rp == 3'd0) begin dm_ra = i_s2 + i_si; i_m <= 8'd1; i_found <= 1'b0; i_rp <= 3'd1; end  // read the rhs element
                  else if (i_rp == 3'd1) begin i_val <= dm_rq;                                                 // save it; start the lhs membership scan
                    if (i_l1 == 8'd0) i_rp <= 3'd3; else begin dm_ra = i_s1 + 8'd1; i_rp <= 3'd2; end
                  end else if (i_rp == 3'd2) begin                                                             // inner scan: compare one lhs element/cycle
                    if (dm_rq == i_val) i_found <= 1'b1;
                    if (i_m < i_l1) begin i_m <= i_m + 8'd1; dm_ra = i_s1 + i_m + 8'd1; end else i_rp <= 3'd3;
                  end else begin                                                                               // decide: append rhs∉lhs
                    if (!i_found) begin dm_we = 1'b1; dm_wa = i_dst + 8'd1 + i_w; dm_wd = i_val; i_w <= i_w + 8'd1; end
                    i_si <= i_si + 8'd1; i_rp <= 3'd0;
                  end
                end else begin
                  dm_we = 1'b1; dm_wa = i_dst; dm_wd = {{ZPAD{1'b0}}, i_w};
                  rf_we = 1'b1; rf_wa = i_rd; rf_wd = {{ZPAD{1'b0}}, i_dst};
                  hp <= i_dst + i_w + 8'd1; pc <= pc + 8'd1; itr <= 1'b0;
                end
              end
            end
            8'd69: begin                                                            // Intersect: scan lhs, keep if ∈ rhs
              if (i_si <= i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_s1 + i_si; i_m <= 8'd1; i_found <= 1'b0; i_rp <= 3'd1; end    // read the lhs element
                else if (i_rp == 3'd1) begin i_val <= dm_rq;                                                   // save it; start the rhs membership scan
                  if (i_l2 == 8'd0) i_rp <= 3'd3; else begin dm_ra = i_s2 + 8'd1; i_rp <= 3'd2; end
                end else if (i_rp == 3'd2) begin
                  if (dm_rq == i_val) i_found <= 1'b1;
                  if (i_m < i_l2) begin i_m <= i_m + 8'd1; dm_ra = i_s2 + i_m + 8'd1; end else i_rp <= 3'd3;
                end else begin                                                                                // decide: keep lhs∈rhs
                  if (i_found) begin dm_we = 1'b1; dm_wa = i_dst + 8'd1 + i_w; dm_wd = i_val; i_w <= i_w + 8'd1; end
                  i_si <= i_si + 8'd1; i_rp <= 3'd0;
                end
              end else begin
                dm_we = 1'b1; dm_wa = i_dst; dm_wd = {{ZPAD{1'b0}}, i_w};
                rf_we = 1'b1; rf_wa = i_rd; rf_wd = {{ZPAD{1'b0}}, i_dst};
                hp <= i_dst + i_w + 8'd1; pc <= pc + 8'd1; itr <= 1'b0;
              end
            end
            8'd47: begin                                                            // ListPushField COW: copy old list, then buffer {header, append, repoint}
              if (i_si <= i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_s1 + i_si; i_rp <= 3'd1; end
                else begin dm_we = 1'b1; dm_wa = i_dst + i_si; dm_wd = dm_rq; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else begin
                wbuf_a[0] <= i_dst;                wbuf_d[0] <= {{ZPAD{1'b0}}, (i_l1 + 8'd1)};   // new length
                wbuf_a[1] <= i_dst + 8'd1 + i_l1;  wbuf_d[1] <= i_val;                  // the pushed elem
                wbuf_a[2] <= i_s2;                 wbuf_d[2] <= {{ZPAD{1'b0}}, i_dst};  // repoint the field
                wbuf_n <= 3'd3; wbuf_i <= 3'd0; drng <= 1'b1; itr <= 1'b0; hp <= i_dst + 8'd2 + i_l1;
              end
            end
            8'd54: begin                                                            // ListPush/CrdtAppend COW: copy old list, then buffer {header, append} + repoint the result REGISTER
              if (i_si <= i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_s1 + i_si; i_rp <= 3'd1; end
                else begin dm_we = 1'b1; dm_wa = i_dst + i_si; dm_wd = dm_rq; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else begin
                wbuf_a[0] <= i_dst;                wbuf_d[0] <= {{ZPAD{1'b0}}, (i_l1 + 8'd1)};   // new length
                wbuf_a[1] <= i_dst + 8'd1 + i_l1;  wbuf_d[1] <= i_val;                  // the pushed elem
                rf_we = 1'b1; rf_wa = i_rd; rf_wd = {{ZPAD{1'b0}}, i_dst};              // repoint the list register -> new base
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; itr <= 1'b0; hp <= i_dst + 8'd2 + i_l1;
              end
            end
            8'd213: begin                                                           // FListPush COW: copy old list on BOTH planes, then buffer {header, placeholder} + write the float payload + repoint the register
              if (i_si <= i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_s1 + i_si; fdm_ra = i_s1 + i_si; i_rp <= 3'd1; end                 // issue BOTH plane reads (synchronous, 1-cycle)
                else begin dm_we = 1'b1; dm_wa = i_dst + i_si; dm_wd = dm_rq;                                        // write the int structure cell
                           fdm_we = 1'b1; fdm_wa = i_dst + i_si; fdm_wd = fdm_rq;                                   // AND the float payload cell (the seq's data survives the copy)
                           i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else begin
                wbuf_a[0] <= i_dst;                wbuf_d[0] <= {{ZPAD{1'b0}}, (i_l1 + 8'd1)};   // new length
                wbuf_a[1] <= i_dst + 8'd1 + i_l1;  wbuf_d[1] <= 32'd0;                  // the appended int placeholder (payload rides fdmem)
                fdm_we = 1'b1; fdm_wa = i_dst + 8'd1 + i_l1; fdm_wd = fav;              // the f64 payload at the appended slot (fav = fregs[ra], stable while pc held)
                rf_we = 1'b1; rf_wa = i_rd; rf_wd = {{ZPAD{1'b0}}, i_dst};              // repoint the list register -> new base
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; itr <= 1'b0; hp <= i_dst + 8'd2 + i_l1;
              end
            end
            8'd63: begin                                                            // Contains: sequential membership scan -> 1/0
              if (i_si <= i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_s1 + i_si; i_rp <= 3'd1; end
                else begin if (dm_rq == i_val) i_found <= 1'b1; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else begin rf_we = 1'b1; rf_wa = i_rd; rf_wd = i_found ? 32'd1 : 32'd0; pc <= pc + 8'd1; itr <= 1'b0; end
            end
            8'd55: begin                                                            // SetAdd: membership scan; append (dedup) if absent
              if (i_si <= i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_s1 + i_si; i_rp <= 3'd1; end
                else begin if (dm_rq == i_val) i_found <= 1'b1; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else if (i_found) begin pc <= pc + 8'd1; itr <= 1'b0; end          // already present -> no-op
              else begin                                                            // absent -> append value + length++
                wbuf_a[0] <= i_s1 + 8'd1 + i_l1; wbuf_d[0] <= i_val;
                wbuf_a[1] <= i_s1;               wbuf_d[1] <= {{ZPAD{1'b0}}, (i_l1 + 8'd1)};
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; itr <= 1'b0; hp <= hp + 8'd1;
              end
            end
            8'd71: begin                                                            // DestructureTuple: scatter dmem[src+1+i] -> regs[base+i], one/cycle (0-based count i_l1)
              if (i_si < i_l1) begin
                if (i_rp == 3'd0) begin dm_ra = i_s1 + 8'd1 + i_si; i_rp <= 3'd1; end
                else begin rf_we = 1'b1; rf_wa = i_rd + i_si[3:0]; rf_wd = dm_rq; i_si <= i_si + 8'd1; i_rp <= 3'd0; end
              end else begin pc <= pc + 8'd1; itr <= 1'b0; end
            end
            default: begin pc <= pc + 8'd1; itr <= 1'b0; end
          endcase
        end else if (ms) begin
          // MAP KEY-SCAN: ph7 latches the query-key length + issues pair-0's key ptr; then per pair,
          // ph0 = key ptr, ph1 = pair key length (keq init), ph3/ph4 = compare one key char/cycle, ph5 =
          // decide (Index reads the value in ph6; SetIndex records the matched pair). Double-indirect
          // reads (dmem[dmem[...]]) are the two-step ph0->ph1 and ph3->ph4.
          if (ms_cow) begin
            // COW COPY of a buried map before an append: relocate the `1+2*np` cells `[header, k0,v0, …]`
            // to the fresh block at `ms_nb`, one cell/cycle (crp 0 = issue read, crp 1 = write). When all
            // cells are copied, buffer the corrected header (np+1 pairs) + the new `(key ptr, value)` pair
            // and repoint the map register at `ms_nb`.
            if (ms_ci <= (ms_np << 1)) begin
              if (!ms_crp) begin dm_ra = ms_map + ms_ci; ms_crp <= 1'b1; end
              else begin dm_we = 1'b1; dm_wa = ms_nb + ms_ci; dm_wd = dm_rq; ms_ci <= ms_ci + 8'd1; ms_crp <= 1'b0; end
            end else begin
              wbuf_a[0] <= ms_nb;                          wbuf_d[0] <= MAP_TAG | ({{ZPAD{1'b0}}, ms_np} + 32'd1);
              wbuf_a[1] <= ms_nb + 8'd1 + (ms_np << 1);    wbuf_d[1] <= {{ZPAD{1'b0}}, ms_qk};
              wbuf_a[2] <= ms_nb + 8'd2 + (ms_np << 1);    wbuf_d[2] <= ms_wv;
              rf_we = 1'b1; rf_wa = ms_reg; rf_wd = {{ZPAD{1'b0}}, ms_nb};
              wbuf_n <= 3'd3; wbuf_i <= 3'd0; drng <= 1'b1; ms <= 1'b0; ms_cow <= 1'b0;
              hp <= ms_nb + 8'd3 + (ms_np << 1);
            end
          end else if (ms_ph == 3'd7) begin
            ms_qlen <= dm_rq[7:0]; dm_ra = ms_map + 8'd1; ms_ph <= 3'd0;
          end else if (ms_pi < ms_np) begin
            case (ms_ph)
              3'd0: begin ms_kptr <= dm_rq[HPW-1:0]; dm_ra = dm_rq[HPW-1:0]; ms_ph <= 3'd1; end
              3'd1: begin ms_keq <= (ms_qlen == dm_rq[7:0]);
                      if (ms_qlen == 8'd0) ms_ph <= 3'd5;
                      else begin ms_kj <= 8'd1; dm_ra = ms_qk + 8'd1; ms_ph <= 3'd3; end
                    end
              3'd3: begin ms_qc <= dm_rq; dm_ra = ms_kptr + ms_kj; ms_ph <= 3'd4; end
              3'd4: begin if (dm_rq != ms_qc) ms_keq <= 1'b0;
                      if (ms_kj < ms_qlen && ms_kj < (MAXKEY - 1)) begin ms_kj <= ms_kj + 8'd1; dm_ra = ms_qk + ms_kj + 8'd1; ms_ph <= 3'd3; end
                      else ms_ph <= 3'd5;
                    end
              3'd5: begin
                      if (ms_op == 8'd59 || ms_op == 8'd60) begin                                 // Index
                        if (ms_keq) begin dm_ra = ms_map + 8'd2 + (ms_pi << 1); ms_ph <= 3'd6; end
                        else begin ms_pi <= ms_pi + 8'd1; ms_ph <= 3'd0; dm_ra = ms_map + 8'd1 + ((ms_pi + 8'd1) << 1); end
                      end else begin                                                              // SetIndex: record match (last wins)
                        if (ms_keq) ms_found <= ms_pi + 8'd1;
                        ms_pi <= ms_pi + 8'd1; ms_ph <= 3'd0; dm_ra = ms_map + 8'd1 + ((ms_pi + 8'd1) << 1);
                      end
                    end
              3'd6: begin ms_val <= dm_rq; ms_pi <= ms_pi + 8'd1; ms_ph <= 3'd0; dm_ra = ms_map + 8'd1 + ((ms_pi + 8'd1) << 1); end
              default: ms_ph <= 3'd5;
            endcase
          end else if (ms_op == 8'd59 || ms_op == 8'd60) begin                                    // Index result (0 if no key matched)
            rf_we = 1'b1; rf_wa = ms_dst; rf_wd = ms_val; pc <= pc + 8'd1; ms <= 1'b0;
          end else if (ms_found != 8'd0) begin                                                    // SetIndex: key present -> update its value
            dm_we = 1'b1; dm_wa = ms_map + 8'd2 + ((ms_found - 8'd1) << 1); dm_wd = ms_wv; pc <= pc + 8'd1; ms <= 1'b0;
          end else if (ms_map + 8'd1 + (ms_np << 1) == hp) begin                                  // SetIndex absent, map ON TOP -> append (key ptr, value, header++) in place
            wbuf_a[0] <= hp;        wbuf_d[0] <= {{ZPAD{1'b0}}, ms_qk};
            wbuf_a[1] <= hp + 8'd1; wbuf_d[1] <= ms_wv;
            wbuf_a[2] <= ms_map;    wbuf_d[2] <= MAP_TAG | ({{ZPAD{1'b0}}, ms_np} + 32'd1);
            wbuf_n <= 3'd3; wbuf_i <= 3'd0; drng <= 1'b1; ms <= 1'b0; hp <= hp + 8'd2;
          end else begin                                                                          // SetIndex absent, map BURIED (a fresh TextConst key was allocated after it) -> COW the whole block to a fresh contiguous one on top, then append there
            ms_cow <= 1'b1; ms_ci <= 8'd0; ms_crp <= 1'b0; ms_nb <= hp;
          end
        end else if (msi) begin
          // INT-KEY MAP SCAN: per pair, ph0 issues the read of this pair's key cell (dmem[map+1+2*pi]);
          // ph1 compares it to msi_key by direct integer `==`. On a hit, msi_found records the pair (+1) and
          // the scan jumps to done (msi_pi=msi_np). Once scanning is over: 223 (MapIndexInt) reads the
          // matched value cell (msi_found!=0) then delivers it — or delivers 0 on a miss; 224
          // (MapSetIndexInt) overwrites the matched value, else appends (key, value) on top of the heap and
          // bumps the pair count (the int-key map stays contiguous on top, so the in-place append is sound —
          // same rule as the char-key SetIndex append above).
          if (msi_pi < msi_np) begin
            if (msi_ph == 1'b0) begin
              dm_ra = msi_map + 8'd1 + (msi_pi << 1); msi_ph <= 1'b1;
            end else begin
              if (dm_rq == msi_key) begin msi_found <= msi_pi + 8'd1; msi_pi <= msi_np; msi_ph <= 1'b0; end
              else begin msi_pi <= msi_pi + 8'd1; msi_ph <= 1'b0; end
            end
          end else if (msi_op == 8'd223) begin                                            // MapIndexInt: deliver the value
            if (msi_found != 8'd0 && msi_ph == 1'b0) begin
              dm_ra = msi_map + 8'd2 + ((msi_found - 8'd1) << 1); msi_ph <= 1'b1;         // issue the matched value read; deliver next cycle
            end else if (msi_found != 8'd0) begin
              rf_we = 1'b1; rf_wa = msi_dst; rf_wd = dm_rq; pc <= pc + 8'd1; msi <= 1'b0;  // dm_rq = dmem[map+2+2*(found-1)] = the value
            end else begin
              rf_we = 1'b1; rf_wa = msi_dst; rf_wd = 32'd0; pc <= pc + 8'd1; msi <= 1'b0;  // no key matched -> 0
            end
          end else if (msi_found != 8'd0) begin                                           // MapSetIndexInt: key present -> update its value
            dm_we = 1'b1; dm_wa = msi_map + 8'd2 + ((msi_found - 8'd1) << 1); dm_wd = msi_wv; pc <= pc + 8'd1; msi <= 1'b0;
          end else begin                                                                  // MapSetIndexInt: absent -> append (key, value, header++) on top
            wbuf_a[0] <= hp;        wbuf_d[0] <= msi_key;
            wbuf_a[1] <= hp + 8'd1; wbuf_d[1] <= msi_wv;
            wbuf_a[2] <= msi_map;   wbuf_d[2] <= MAP_TAG | ({{ZPAD{1'b0}}, msi_np} + 32'd1);
            wbuf_n <= 3'd3; wbuf_i <= 3'd0; drng <= 1'b1; msi <= 1'b0; hp <= hp + 8'd2;
          end
        end else if (ld) begin
          // LOAD COMPLETION: dm_rq now holds the word requested last cycle. Finish the op (single-read
          // ops complete here; multi-read ops advance ld_ph and issue the next dm_ra). See the issue arms.
          ld <= 1'b0;
          case (ld_op)
            8'd62: begin rf_we = 1'b1; rf_wd = (dm_rq & MAP_TAG) ? (dm_rq & 32'h3fffffff) : dm_rq; pc <= pc + 8'd1; end // Length
            8'd44: begin if (dm_rq != 32'd0) pc <= pc + 8'd1; else begin halted <= 1'b1; trapped <= 1'b1; end end       // CheckPolicy: grant iff field truthy
            8'd78: begin rf_we = 1'b1; rf_wd = (dm_rq == {24'd0, rb}) ? 32'd1 : 32'd0; pc <= pc + 8'd1; end             // TestArm: variant tag match
            8'd79: begin rf_we = 1'b1; rf_wd = dm_rq; pc <= pc + 8'd1; end                                             // BindArm: field read
            8'd76: begin rf_we = 1'b1; rf_wd = dm_rq; pc <= pc + 8'd1; end                                             // GetField
            8'd80: begin dm_we = 1'b1; dm_wa = dv[HPW-1:0] + 8'd1 + {4'd0, rb[3:0]};                           // CrdtBump: read-modify-write
                         dm_wd = dm_rq + (rb[4] ? (32'd0 - av) : av); pc <= pc + 8'd1; end
            8'd70: begin                                                                                              // ListPop: ph0 got len -> read element; ph1 write len--
              if (ld_ph == 4'd0) begin ld_t0 <= dm_rq; dm_ra = av[HPW-1:0] + dm_rq[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              else begin rf_we = 1'b1; rf_wd = dm_rq; dm_we = 1'b1; dm_wa = av[HPW-1:0]; dm_wd = ld_t0 - 32'd1; pc <= pc + 8'd1; end
            end
            8'd93: begin                                                                                              // ChanSend: ph0 head -> read count; ph1 stage enqueue
              if (ld_ph == 4'd0) begin ld_t0 <= dm_rq; dm_ra = dv[HPW-1:0] + 8'd1; ld <= 1'b1; ld_ph <= 4'd1; end
              else begin
                wbuf_a[0] <= dv[HPW-1:0] + 8'd2 + ((ld_t0 + dm_rq) & 32'd7); wbuf_d[0] <= av;                 // slot = (head+count) mod 8
                wbuf_a[1] <= dv[HPW-1:0] + 8'd1; wbuf_d[1] <= dm_rq + 32'd1;                                  // count++
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1;
              end
            end
            8'd94: begin                                                                                              // ChanRecv: ph0 head -> read count; ph1 -> read element; ph2 stage dequeue
              if (ld_ph == 4'd0) begin ld_t0 <= dm_rq; dm_ra = av[HPW-1:0] + 8'd1; ld <= 1'b1; ld_ph <= 4'd1; end
              else if (ld_ph == 4'd1) begin ld_t1 <= dm_rq; dm_ra = av[HPW-1:0] + 8'd2 + (ld_t0[7:0] & 8'd7); ld <= 1'b1; ld_ph <= 4'd2; end
              else begin
                rf_we = 1'b1; rf_wd = dm_rq;                                                                          // element at ring[head]
                wbuf_a[0] <= av[HPW-1:0];        wbuf_d[0] <= (ld_t0 + 32'd1) & 32'd7;                                     // head = (head+1) mod 8
                wbuf_a[1] <= av[HPW-1:0] + 8'd1; wbuf_d[1] <= ld_t1 - 32'd1;                                              // count--
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1;
              end
            end
            8'd95: begin                                                                                              // ChanTrySend: ph0 count guard -> read head; ph1 stage enqueue + success=1
              if (ld_ph == 4'd0) begin
                if (dm_rq >= 32'd8) begin rf_we = 1'b1; rf_wd = 32'd0; pc <= pc + 8'd1; end                           // full -> success=0
                else begin ld_t1 <= dm_rq; dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              end else begin
                wbuf_a[0] <= av[HPW-1:0] + 8'd2 + ((dm_rq + ld_t1) & 32'd7); wbuf_d[0] <= bv;                             // slot=(head+count) mod 8
                wbuf_a[1] <= av[HPW-1:0] + 8'd1; wbuf_d[1] <= ld_t1 + 32'd1;                                              // count++
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; rf_we = 1'b1; rf_wd = 32'd1;                            // success=1
              end
            end
            8'd96: begin                                                                                              // ChanTryRecv: ph0 count guard -> read head; ph1 -> read element; ph2 stage dequeue
              if (ld_ph == 4'd0) begin
                if (dm_rq == 32'd0) begin rf_we = 1'b1; rf_wd = 32'd0; pc <= pc + 8'd1; end                           // empty -> value=0
                else begin ld_t1 <= dm_rq; dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              end else if (ld_ph == 4'd1) begin ld_t0 <= dm_rq; dm_ra = av[HPW-1:0] + 8'd2 + (dm_rq[7:0] & 8'd7); ld <= 1'b1; ld_ph <= 4'd2; end
              else begin
                rf_we = 1'b1; rf_wd = dm_rq;                                                                          // element
                wbuf_a[0] <= av[HPW-1:0];        wbuf_d[0] <= (ld_t0 + 32'd1) & 32'd7;                                     // head++
                wbuf_a[1] <= av[HPW-1:0] + 8'd1; wbuf_d[1] <= ld_t1 - 32'd1;                                              // count--
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1;
              end
            end
            8'd39: begin pc <= dm_rq[7:0]; end                                                                        // CallValue: jump to the closure entry just read
            8'd86: begin                                                                                              // IterNext: ph0 got length -> exit or read element; ph1 deliver element + advance
              if (ld_ph == 4'd0) begin
                if (iter_cur[isp - 4'd1] > dm_rq) pc <= ra;                                                           // cursor past length -> loop exit
                else begin dm_ra = iter_ptr[isp - 4'd1] + iter_cur[isp - 4'd1]; ld <= 1'b1; ld_ph <= 4'd1; end
              end else begin
                rf_we = 1'b1; rf_wd = dm_rq; iter_cur[isp - 4'd1] <= iter_cur[isp - 4'd1] + 8'd1; pc <= pc + 8'd1;
              end
            end
            8'd231: begin                                                                                             // FIterNext: ph0 got length off dmem -> exit or issue the FLOAT payload read; ph1 deliver f64 -> fregs[dst] + advance
              if (ld_ph == 4'd0) begin
                if (iter_cur[isp - 4'd1] > dm_rq) pc <= ra;                                                           // cursor past length -> loop exit (like op 86)
                else begin fdm_ra = iter_ptr[isp - 4'd1] + iter_cur[isp - 4'd1]; ld <= 1'b1; ld_ph <= 4'd1; end       // read fdmem[ptr+cursor] (the f64 payload plane)
              end else begin
                ff_we = 1'b1; ff_wd = fdm_rq; iter_cur[isp - 4'd1] <= iter_cur[isp - 4'd1] + 8'd1; pc <= pc + 8'd1;    // fregs[dst] = the f64 payload; advance the cursor
              end
            end
            8'd66: begin bld <= 1'b1; b_op <= 8'd66; b_base <= hp; b_i <= 8'd0; b_n <= dm_rq[HPW-1:0];                 // DeepClone: source length read -> start the copy build
                         b_dst <= dst[RW-1:0]; b_csrc <= av[HPW-1:0]; b_ph <= 1'b0; end
            8'd32, 8'd33: begin                                                                                       // Concat/SeqConcat: ph0 lhs length -> read rhs length; ph1 start the build
              if (ld_ph == 4'd0) begin ld_t0 <= dm_rq; dm_ra = bv[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              else begin
                bld <= 1'b1; b_op <= ld_op; b_base <= hp; b_i <= 8'd0; b_n <= ld_t0[HPW-1:0] + dm_rq[HPW-1:0]; b_dst <= dst[RW-1:0];
                b_hdr <= ld_t0 + dm_rq; b_len1 <= ld_t0[HPW-1:0]; b_csrc <= av[HPW-1:0]; b_csrc2 <= bv[HPW-1:0]; b_ph <= 1'b0;
              end
            end
            8'd81: begin itr <= 1'b1; i_op <= 8'd81; i_dst <= dv[HPW-1:0]; i_s1 <= av[HPW-1:0];                    // CrdtMerge: source length read -> start the field-wise sum
                         i_l1 <= dm_rq[HPW-1:0]; i_si <= 8'd1; i_rp <= 3'd0; end
            8'd113: begin cj <= 1'b1; cj_dst <= dv[HPW-1:0]; cj_src <= av[HPW-1:0];                               // CrdtJoin: source lane count read -> start the crdt_merge coprocessor
                          cj_l <= (dm_rq[7:0] > 8'd4) ? 8'd4 : dm_rq[7:0]; cj_i <= 8'd1; cj_rp <= 3'd0; cj_ph <= 1'b0; cj_a <= 128'd0; cj_b <= 128'd0; end
            8'd68: begin                                                                                              // Union: ph0 lhs length -> read rhs length; ph1 start the merge
              if (ld_ph == 4'd0) begin ld_t0 <= dm_rq; dm_ra = bv[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              else begin itr <= 1'b1; i_op <= 8'd68; i_ph <= 1'b0; i_dst <= hp; i_s1 <= av[HPW-1:0]; i_s2 <= bv[HPW-1:0];
                         i_l1 <= ld_t0[HPW-1:0]; i_l2 <= dm_rq[HPW-1:0]; i_si <= 8'd1; i_w <= 8'd0; i_rd <= dst[RW-1:0]; i_rp <= 3'd0; end
            end
            8'd69: begin                                                                                              // Intersect: ph0 lhs length -> read rhs length; ph1 start the filter
              if (ld_ph == 4'd0) begin ld_t0 <= dm_rq; dm_ra = bv[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              else begin itr <= 1'b1; i_op <= 8'd69; i_dst <= hp; i_s1 <= av[HPW-1:0]; i_s2 <= bv[HPW-1:0];
                         i_l1 <= ld_t0[HPW-1:0]; i_l2 <= dm_rq[HPW-1:0]; i_si <= 8'd1; i_w <= 8'd0; i_rd <= dst[RW-1:0]; i_rp <= 3'd0; end
            end
            8'd63: begin itr <= 1'b1; i_op <= 8'd63; i_s1 <= av[HPW-1:0]; i_l1 <= dm_rq[HPW-1:0]; i_val <= bv;             // Contains: length read -> start the membership scan
                         i_rd <= dst[RW-1:0]; i_si <= 8'd1; i_found <= 1'b0; i_rp <= 3'd0; end
            8'd55: begin itr <= 1'b1; i_op <= 8'd55; i_s1 <= dv[HPW-1:0]; i_l1 <= dm_rq[HPW-1:0]; i_val <= av; // SetAdd: length read -> start the membership scan
                         i_si <= 8'd1; i_found <= 1'b0; i_rp <= 3'd0; end
            8'd54: begin                                                                                              // ListPush/CrdtAppend: length read -> in-place append if on top of heap, else COW
              if (dv[HPW-1:0] + 8'd1 + dm_rq[HPW-1:0] == hp) begin                                        // list is on top of the heap -> append in place
                wbuf_a[0] <= dv[HPW-1:0] + 8'd1 + dm_rq[HPW-1:0]; wbuf_d[0] <= av;
                wbuf_a[1] <= dv[HPW-1:0];                     wbuf_d[1] <= dm_rq + 32'd1;
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; hp <= hp + 8'd1;
              end else begin                                                                                        // else copy-on-write via the iterate FSM, repointing the list register
                itr <= 1'b1; i_op <= 8'd54; i_dst <= hp; i_s1 <= dv[HPW-1:0]; i_l1 <= dm_rq[HPW-1:0]; i_si <= 8'd1;
                i_val <= av; i_rd <= dst[RW-1:0]; i_rp <= 3'd0;
              end
            end
            8'd213: begin                                                                                             // FListPush: length read -> in-place append (int placeholder + float payload) or COW (BOTH planes)
              if (dv[HPW-1:0] + 8'd1 + dm_rq[HPW-1:0] == hp) begin                                        // list on top of the heap -> append in place
                wbuf_a[0] <= dv[HPW-1:0] + 8'd1 + dm_rq[HPW-1:0]; wbuf_d[0] <= 32'd0;                     // int placeholder cell (the payload rides fdmem)
                wbuf_a[1] <= dv[HPW-1:0];                     wbuf_d[1] <= dm_rq + 32'd1;                 // header: length + 1
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; hp <= hp + 8'd1;
                fdm_we = 1'b1; fdm_wa = dv[HPW-1:0] + 8'd1 + dm_rq[HPW-1:0]; fdm_wd = fav;                // the f64 payload at the appended slot
              end else begin                                                                                        // else copy-on-write BOTH planes via the iterate FSM, repointing the list register
                itr <= 1'b1; i_op <= 8'd213; i_dst <= hp; i_s1 <= dv[HPW-1:0]; i_l1 <= dm_rq[HPW-1:0]; i_si <= 8'd1;
                i_val <= 32'd0; i_rd <= dst[RW-1:0]; i_rp <= 3'd0;                                                  // the pushed int cell is a placeholder-0; the float rides fav (stable, pc held)
              end
            end
            8'd214: begin ff_we = 1'b1; ff_wa = dst[RW-1:0]; ff_wd = fdm_rq; pc <= pc + 8'd1; end // FIndex: fdm_rq (fdmem[regs[ra]+regs[rb]]) is valid now -> fregs[dst]
            8'd56: begin itr <= 1'b1; i_op <= 8'd56; i_ph <= 1'b0; i_dst <= dv[HPW-1:0]; i_l1 <= dm_rq[HPW-1:0];   // RemoveFrom: length read -> start the find-then-shift scan
                         i_val <= av; i_si <= 8'd1; i_m <= 8'd0; i_rp <= 3'd0; end
            8'd47: begin                                                                                              // ListPushField: ph0 list_ptr -> read list_len; ph1 in-place append or COW
              if (ld_ph == 4'd0) begin ld_t0 <= dm_rq; dm_ra = dm_rq[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              else if (ld_t0 + 8'd1 + dm_rq == {{ZPAD{1'b0}}, hp}) begin                                            // list is on top of the heap -> append in place
                wbuf_a[0] <= ld_t0[HPW-1:0] + 8'd1 + dm_rq[HPW-1:0]; wbuf_d[0] <= av;
                wbuf_a[1] <= ld_t0[HPW-1:0];                 wbuf_d[1] <= dm_rq + 32'd1;
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; hp <= hp + 8'd1;
              end else begin                                                                                        // else copy-on-write via the iterate FSM
                itr <= 1'b1; i_op <= 8'd47; i_dst <= hp; i_s1 <= ld_t0[HPW-1:0]; i_l1 <= dm_rq[HPW-1:0]; i_si <= 8'd1;
                i_val <= av; i_s2 <= dv[HPW-1:0] + 8'd1 + rb; i_rp <= 3'd0;
              end
            end
            8'd104: begin                                                                                             // SelectWait: poll arms (ld_ph 0..3) -> decide (7) -> dequeue (8..10)
              // sel_cptr[k] = the channel heap pointer, RESOLVED (regs[chan_reg]) at arm-registration time —
              // so the poll/dequeue read dmem directly without a per-arm register-read bubble.
              if (ld_ph <= 4'd6) begin                                                                               // POLL: ld_ph = arm index; record first ready recv
                if (!sel_tmo[ld_ph[1:0]] && dm_rq > 32'd0 && sel_fired == 4'hf) sel_fired <= ld_ph;
                if (ld_ph + 4'd1 < sel_n) begin dm_ra = sel_cptr[ld_ph + 4'd1][HPW-1:0] + 8'd1; ld <= 1'b1; ld_ph <= ld_ph + 4'd1; end
                else begin ld <= 1'b1; ld_ph <= 4'd7; end
              end else if (ld_ph == 4'd7) begin                                                                      // DECIDE (sel_fired now settled)
                if (sel_fired != 4'hf) begin dm_ra = sel_cptr[sel_fired[1:0]][HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd8; end
                else begin
                  sel_to = 4'hf;
                  for (si = 3; si >= 0; si = si - 1) if (si < sel_n && sel_tmo[si]) sel_to = si[3:0];
                  rf_we = 1'b1; rf_wd = (sel_to == 4'hf) ? 32'hffffffff : {28'd0, sel_to}; pc <= pc + 8'd1; sel_n <= 4'd0;
                end
              end else if (ld_ph == 4'd8) begin ld_t0 <= dm_rq; dm_ra = sel_cptr[sel_fired[1:0]][HPW-1:0] + 8'd1; ld <= 1'b1; ld_ph <= 4'd9; end       // head -> read count
              else if (ld_ph == 4'd9) begin ld_t1 <= dm_rq; dm_ra = sel_cptr[sel_fired[1:0]][HPW-1:0] + 8'd2 + (ld_t0[7:0] & 8'd7); ld <= 1'b1; ld_ph <= 4'd10; end // count -> read element
              else begin                                                                                            // element -> deliver value + fired index (two reg writes) + stage dequeue
                rf_we = 1'b1; rf_wa = sel_var[sel_fired[1:0]]; rf_wd = dm_rq;                                        // regs[var] = received value
                p2w <= 1'b1; p2w_wa = dst[RW-1:0]; p2w_wd = {28'd0, sel_fired};                                     // regs[dst] = fired arm index (pc-neutral second write)
                wbuf_a[0] <= sel_cptr[sel_fired[1:0]][HPW-1:0];        wbuf_d[0] <= (ld_t0 + 32'd1) & 32'd7;
                wbuf_a[1] <= sel_cptr[sel_fired[1:0]][HPW-1:0] + 8'd1; wbuf_d[1] <= ld_t1 - 32'd1;
                wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; sel_n <= 4'd0;
              end
            end
            8'd59: begin                                                                                              // Index: ph0 header -> map key-scan (MAP) or list element read; ph1 deliver list element
              if (ld_ph == 4'd0) begin
                if (dm_rq & MAP_TAG) begin ms <= 1'b1; ms_op <= 8'd59; ms_map <= av[HPW-1:0]; ms_qk <= bv[HPW-1:0]; ms_dst <= dst[RW-1:0];
                                           ms_np <= dm_rq[7:0]; ms_pi <= 8'd0; ms_val <= 32'd0; ms_ph <= 3'd7; dm_ra = bv[HPW-1:0]; end
                else begin dm_ra = av[HPW-1:0] + bv[HPW-1:0]; ld <= 1'b1; ld_ph <= 4'd1; end
              end else begin rf_we = 1'b1; rf_wd = dm_rq; pc <= pc + 8'd1; end
            end
            8'd57: begin                                                                                              // SetIndex: header -> map key-scan (MAP) or list write
              if (dm_rq & MAP_TAG) begin ms <= 1'b1; ms_op <= 8'd57; ms_map <= dv[HPW-1:0]; ms_qk <= av[HPW-1:0]; ms_wv <= bv; ms_reg <= dst[RW-1:0]; ms_cow <= 1'b0;
                                         ms_np <= dm_rq[7:0]; ms_pi <= 8'd0; ms_found <= 8'd0; ms_ph <= 3'd7; dm_ra = av[HPW-1:0]; end
              else begin dm_we = 1'b1; dm_wa = dv[HPW-1:0] + av[HPW-1:0]; dm_wd = bv; pc <= pc + 8'd1; end
            end
            8'd223: begin                                                                                             // MapIndexInt: header read -> start the int-key scan
              msi <= 1'b1; msi_op <= 8'd223; msi_map <= av[HPW-1:0]; msi_key <= bv; msi_dst <= dst[RW-1:0];
              msi_np <= dm_rq[7:0]; msi_pi <= 8'd0; msi_found <= 8'd0; msi_ph <= 1'b0;
            end
            8'd224: begin                                                                                             // MapSetIndexInt: header read -> start the int-key scan
              msi <= 1'b1; msi_op <= 8'd224; msi_map <= dv[HPW-1:0]; msi_key <= av; msi_wv <= bv;
              msi_np <= dm_rq[7:0]; msi_pi <= 8'd0; msi_found <= 8'd0; msi_ph <= 1'b0;
            end
            8'd218: begin                                                                                             // TextCharAt: dm_rq = the codepoint -> build a fresh single-char Text [1, codepoint]
              wbuf_a[0] <= hp;        wbuf_d[0] <= 32'd1;                    // header (length 1)
              wbuf_a[1] <= hp + 8'd1; wbuf_d[1] <= dm_rq;                    // the codepoint
              rf_we = 1'b1; rf_wd = {{ZPAD{1'b0}}, hp};                      // regs[dst] <- the new list base
              wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; ld <= 1'b0; hp <= hp + 8'd2;
            end
            default: begin pc <= pc + 8'd1; end
          endcase
        end else if (fc) begin
          // FConst SEQUENTIALIZER step (pc has held on the op-201 instruction throughout): the low word is
          // already latched in fc_w1 (imem[pc+1], read at dispatch). The data port was steered to pc+2 at
          // dispatch, so imdata_q now holds the HIGH word — write the full f64. ff_wa defaults to the held dst.
          ff_we = 1'b1; ff_wd = {imdata_q, fc_w1}; pc <= pc + 16'd3; fc <= 1'b0;
        end else if (operands_ready)
        // MAIN DISPATCH — fires only when the registered operands (av/bv/dv) reflect imem[pc]. When
        // !operands_ready (the one bubble cycle right after a PC move) NONE of the arms above are active and
        // this branch is skipped: pc holds, the read ports latch imem[pc]'s ra/rb/dst, and next cycle
        // operands_ready goes high so the instruction executes with valid operands.
        case (opcode)
          // Reversible/adiabatic charge-recovery core (wire opcodes 0..4) — the abelian bijection datapath
          // (mirrors rev_alu_core.v), integrated so ONE core executes every op.
          8'd0:  begin begin rf_we = 1'b1; rf_wd = dv + av; end pc <= pc + 8'd1; end      // RevAdd {place=dst, delta=ra}  (dv=regs[dst])
          8'd1:  begin begin rf_we = 1'b1; rf_wd = dv - av; end pc <= pc + 8'd1; end      // RevSub
          8'd2:  begin begin rf_we = 1'b1; rf_wd = dv ^ av; end pc <= pc + 8'd1; end      // RevXor
          8'd3:  begin begin rf_we = 1'b1; rf_wd = (~dv) + 32'd1; end pc <= pc + 8'd1; end // RevNeg {place=dst}
          8'd4:  begin rf_we = 1'b1; rf_wa = ra[RW-1:0]; rf_wd = dv;                       // RevSwap {a=dst, b=ra}: regs[ra]<=old regs[dst]
                       rsw <= 1'b1; rsw_wa <= dst[RW-1:0]; rsw_wd <= av; end                // pc HOLDS; the rsw step writes regs[dst]<=old regs[ra] next cycle, then advances
          8'd5:  begin begin rf_we = 1'b1; rf_wd = {{16{imm[15]}}, imm}; end pc <= pc + 8'd1; end   // LoadConst (sign-extended 16-bit)
          8'd200:begin begin rf_we = 1'b1; rf_wd = imdata_q; end pc <= pc + 16'd2; end                // LoadConstWide (full 32-bit next word — imdata_q = imem[pc+1] at dispatch)
          // LoadConstWide64 (op 222) — a 3-word op [222][dst] ‖ bits[31:0] ‖ bits[63:32] carrying a full i64
          // constant (the over-i32 values the encoder emits at EVERY width: word32(2^32-1), the PRNG moduli
          // 2^31/2^32). On this 32-bit datapath the register holds the LOW 32 bits (imdata_q = imem[pc+1]);
          // the high word (imem[pc+2]) is beyond the datapath and is SKIPPED (pc += 3). Before this arm, op
          // 222 fell to `default` → loaded 0 AND advanced pc by 1, mis-reading its two immediate words as
          // opcodes — a $50M silent-zero + PC-desync hole `fifty_million_dollar_test.rs` now forbids.
          8'd222:begin begin rf_we = 1'b1; rf_wd = imdata_q; end pc <= pc + 16'd3; end                // LoadConstWide64 (low 32 bits; skip the high word)
          // ── FLOAT DATAPATH (chip-synthetic opcodes 201..=212) — fregs + fpu.v. add/sub/mul/div/sqrt all
          // issue the multi-cycle FSM (fpm_busy) and stall until done; cmp/IntToF/FToInt/FMove stay
          // single-cycle. fav/fbv are the REGISTERED float operands (valid because operands_ready gates
          // this dispatch — same bubble as the int ALU). IntToF reads the int port av; ShowF/FToInt read fav.
          8'd201:begin fc <= 1'b1; fc_w1 <= imdata_q; im_rb = pc[IAW-1:0] + 16'd2; end   // FConst: latch low word (imdata_q=imem[pc+1]); pc HOLDS; steer the data port to imem[pc+2] so the fc step reads the HIGH word next cycle. (bits: [63:32]=word2, [31:0]=word1)
          8'd202:begin fas_start  <= 1'b1; fpm_busy <= 1'b1; fpm_sel <= 2'd0; fpm_dst <= dst[RW-1:0]; end // FAdd  -> fp_addsub (fsub wire=0), stall until done
          8'd203:begin fas_start  <= 1'b1; fpm_busy <= 1'b1; fpm_sel <= 2'd0; fpm_dst <= dst[RW-1:0]; end // FSub  -> fp_addsub (fsub wire=1), stall until done
          8'd204:begin fmul_start <= 1'b1; fpm_busy <= 1'b1; fpm_sel <= 2'd1; fpm_dst <= dst[RW-1:0]; end // FMul  -> fp_mul, stall until done
          8'd205:begin fdiv_start <= 1'b1; fpm_busy <= 1'b1; fpm_sel <= 2'd2; fpm_dst <= dst[RW-1:0]; end // FDiv  -> fp_div, stall until done
          8'd206:begin fsqrt_start <= 1'b1; fpm_busy <= 1'b1; fpm_sel <= 2'd3; fpm_dst <= dst[RW-1:0]; end // FSqrt -> fp_sqrt, stall until done
          8'd207:begin rf_we = 1'b1; rf_wd = {31'd0, fp_lt}; pc <= pc + 8'd1; end        // FLt -> int 0/1 in regs
          8'd208:begin rf_we = 1'b1; rf_wd = {31'd0, fp_eq}; pc <= pc + 8'd1; end        // FEq -> int 0/1 in regs
          8'd229:begin rf_we = 1'b1; rf_wd = {31'd0, fp_lt | fp_eq}; pc <= pc + 8'd1; end // FLe -> int 0/1 (le = lt|eq; the fp_cmp NaN-false result matches the VM's f64 `<=`)
          8'd230:begin rf_we = 1'b1; rf_wd = {31'd0, ~fp_eq};       pc <= pc + 8'd1; end // FNe -> int 0/1 (ne = !eq; the fp_cmp NaN-unequal result matches the VM's f64 `!=`)
          8'd209:begin ff_we = 1'b1; ff_wd = i32_to_f64(av); pc <= pc + 8'd1; end        // IntToF: fregs[dst] = (f64)regs[ra]
          8'd210:begin rf_we = 1'b1; rf_wd = f64_to_i32(fav); pc <= pc + 8'd1; end       // FToInt: regs[dst] = (i32)fregs[ra]
          8'd211:begin ff_we = 1'b1; ff_wd = fav; pc <= pc + 8'd1; end                   // FMove: fregs[dst] = fregs[ra]
          8'd212:begin outf <= fav; outf_valid <= 1'b1; pc <= pc + 8'd1; end             // ShowF: emit the f64 bits (host formats)
          8'd6:  begin begin rf_we = 1'b1; rf_wd = av; end           pc <= pc + 8'd1; end          // Move
          8'd34: begin pc <= {ra, dst}; end                                         // Jump (target = {t_hi=ra, t_lo=dst})
          8'd35: begin if (av == 32'd0) pc <= {rb, dst}; else pc <= pc + 8'd1; end    // JumpIfFalse (cond=ra, target={t_hi=rb, t_lo=dst})
          8'd36: begin if (av != 32'd0) pc <= {rb, dst}; else pc <= pc + 8'd1; end    // JumpIfTrue
          8'd89: begin out <= av; out_valid <= 1'b1; pc <= pc + 8'd1; end             // Show (integer datapath, SIGNED)
          8'd221: begin outw <= av; outw_valid <= 1'b1; pc <= pc + 8'd1; end          // ShowWord32 — same 32-bit result, host formats UNSIGNED
          8'd43: begin halted <= 1'b1; end                                          // Halt
          8'd91: begin halted <= 1'b1; trapped <= 1'b1; reasoned <= (rb != 8'd0); end // FailWith {reasoned=rb} — trap-halt (error strobe); rb=1: the reason was just streamed over `outs`
          8'd44: begin dm_ra = av[HPW-1:0] + 8'd1 + rb; ld <= 1'b1; ld_op <= 8'd44; end // CheckPolicy {subject=ra, cap field=rb} — issue read of the capability field (av=regs[ra])
          8'd37: begin                                                              // Call — TWO words: [37][result:8][args_start:8][arg_count:8] then [entry:32]
            // Hardware arg-copy calling convention, now MULTI-CYCLE over the BRAM regfile/call-stack: the
            // `cl` engine snapshots the caller window into savedregs[sp], seeds r0.. from the caller's
            // r[args_start..args_start+arg_count-1] (rest 0), then jumps to the entry. pc HOLDS until FIN.
            cl <= 1'b1; cl_mode <= 2'd0; cl_ph <= 2'd0; cl_j <= {(RW+1){1'b0}}; cl_seed_ph <= 2'd0; cl_needentry <= 1'b0;
            cl_as <= ra; cl_ac <= rb; cl_retpc <= pc + 16'd2; cl_retdst <= dst[RW-1:0]; cl_entry <= imdata_q[15:0];
          end
          8'd41: begin                                                              // Return {src=ra} — capture rv NOW (av=callee regs[ra], fav=callee fregs[ra]), then restore
            rt <= 1'b1; rt_ph <= 2'd0; rt_j <= {(RW+1){1'b0}}; cl_seed_ph <= 2'd0; rt_hasrv <= 1'b1;
            rt_rv <= av; rt_frv <= fav; rt_fp <= sp - 1'b1; cl_retpc <= retpc[sp - 1'b1]; cl_retdst <= retdst[sp - 1'b1];
          end
          8'd42: begin                                                              // ReturnNothing — restore, no result placement
            rt <= 1'b1; rt_ph <= 2'd0; rt_j <= {(RW+1){1'b0}}; cl_seed_ph <= 2'd0; rt_hasrv <= 1'b0;
            rt_fp <= sp - 1'b1; cl_retpc <= retpc[sp - 1'b1];
          end
          8'd40: begin                                                              // MakeClosure {dst, func=ra} — closure = [entry]
            dm_we = 1'b1; dm_wa = hp; dm_wd = {24'd0, ra}; begin rf_we = 1'b1; rf_wd = hp; end hp <= hp + 8'd1; pc <= pc + 8'd1;
          end
          8'd39: begin                                                              // CallValue {dst=result, callee=ra, b: args_start[7:4] | arg_count[3:0]} — indirect call via a closure value
            // Save the full caller window (cl engine), capture the closure entry from dmem[regs[ra]] (issued
            // now, latched at the first SAVE cycle), then window-copy R[args_start..+arg_count] -> R[0..] so a
            // compiled closure body reads its params at r0.. . Unlike Call, the mode-1 seed PRESERVES the
            // non-arg registers (the callee shares the caller's window — captures live there).
            dm_ra = av[HPW-1:0];
            cl <= 1'b1; cl_mode <= 2'd1; cl_ph <= 2'd0; cl_j <= {(RW+1){1'b0}}; cl_seed_ph <= 2'd0; cl_needentry <= 1'b1;
            cl_as <= {4'd0, rb[7:4]}; cl_ac <= {4'd0, rb[3:0]}; cl_retpc <= pc + 16'd1; cl_retdst <= dst[RW-1:0];
          end
          // ── LOAD-STORE unit (structured heap) ──────────────────────────────────────────────
          8'd48, 8'd67: begin                                                       // NewList / NewTuple {dst, start=ra, count=rb}
            bld <= 1'b1; b_op <= opcode; b_base <= hp; b_i <= 8'd0; b_n <= rb; b_dst <= dst[RW-1:0]; b_src <= ra[RW-1:0];
          end
          8'd49, 8'd50, 8'd90, 8'd82: begin                                         // NewEmptyList / NewEmptyListI32 / Args / NewCrdt (empty seq/set, single-replica) {dst}
            dm_we = 1'b1; dm_wa = hp; dm_wd = 32'd0; begin rf_we = 1'b1; rf_wd = hp; end hp <= hp + 8'd1; pc <= pc + 8'd1;
          end
          8'd54, 8'd83: begin dm_ra = dv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd54; end // ListPush {list=dst, value=ra} / CrdtAppend — issue read of the length
          // ── FLOAT-HEAP PLANE (chip-synthetic opcodes 213/214/215) — a `Seq of Float`'s structure rides the
          // int heap (`dmem`); its f64 element PAYLOADS ride `fdmem` at the SAME slots. 213 mirrors the int
          // ListPush (op 54) structure ops on `dmem` AND writes fav to the appended slot on `fdmem`; 214 reads
          // an f64 payload into the float bank; 215 writes one. fav/fbv are the REGISTERED float operands.
          8'd213: begin dm_ra = dv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd213; end // FListPush {list=dst, value=ra} — issue read of the length (like op 54); the float payload rides fav
          8'd214: begin fdm_ra = av[HPW-1:0] + bv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd214; ld_ph <= 4'd0; end // FIndex {dst, coll=ra, index=rb} — issue read of fdmem[regs[ra]+regs[rb]]
          8'd215: begin fdm_we = 1'b1; fdm_wa = dv[HPW-1:0] + av[HPW-1:0]; fdm_wd = fbv; pc <= pc + 8'd1; end // FSetIndex {coll=dst, index=ra, value=rb} — fdmem[regs[dst]+regs[ra]] = fregs[rb]
          8'd59, 8'd60: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd59; ld_ph <= 4'd0; end // Index/IndexUnchecked {dst, coll=ra, idx/key=rb} — issue read of the header (map tag vs list)
          8'd62: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd62; end             // Length {dst, coll=ra} — issue sync read of the header
          // ── TEXT (string) PLANE (chip-synthetic 216..=220): a Text is a char-code list. TextConst copies a
          // pooled char block out of imem into a fresh heap char-list; ShowText streams a list over `outs`;
          // TextCharAt reads a single char into a 1-char Text; TextEq/TextNe compare two lists by content.
          8'd216: begin                                                              // TextConst {dst, len=word[15:0]} ‖ pool_word_offset
            // 2-word op: copy `len` codepoints from imem[pool_off .. pool_off+len] into a fresh heap [len,…].
            // The uniform build-microcode writes the header (b_i==0) then one char/cycle out of imem. NOTE the
            // header equals `len` (the char count), so b_n (the LAST cell index = char count) is also `len`.
            tc_off <= imdata_q[IAW-1:0];                                              // pool word-offset of char 0 (imdata_q = imem[pc+1] at dispatch)
            bld <= 1'b1; b_op <= 8'd216; b_base <= hp; b_i <= 8'd0; b_n <= instr[15:0]; b_dst <= dst[RW-1:0];
          end
          8'd217: begin stx <= 1'b1; stx_ph <= 3'd0; stx_ptr <= av[HPW-1:0]; dm_ra = av[HPW-1:0]; end // ShowText {src=ra} — issue length read, start the stream FSM
          8'd218: begin dm_ra = av[HPW-1:0] + bv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd218; ld_ph <= 4'd0; end // TextCharAt {dst, coll=ra, index=rb} — issue read of the codepoint at [ra]+[rb]
          8'd219, 8'd220: begin                                                      // TextEq (219) / TextNe (220) {dst, a=ra, b=rb}
            teq <= 1'b1; teq_ph <= 3'd0; teq_p <= av[HPW-1:0]; teq_q <= bv[HPW-1:0];
            teq_eqop <= (opcode == 8'd219); teq_dst <= dst[RW-1:0]; dm_ra = av[HPW-1:0]; // issue |p| read
          end
          8'd233: begin                                                              // Chr {dst, code=ra} — a fresh 1-char Text [1, codepoint]
            // A codepoint is VALID iff 0 <= code <= 0x10FFFF and it is NOT a surrogate (0xD800..=0xDFFF) — the
            // model's `u32::try_from(code).and_then(char::from_u32)`. A negative codepoint (high bit set) reads
            // as a value > 0x10FFFF here (matching `u32::try_from` failing on a negative i64), so ONE unsigned
            // range test + the surrogate hole covers it. Valid -> a 2-write wbuf [1, code] (like TextCharAt);
            // invalid -> the trap-halt (pre-trap output stands, byte-identical to the VM's `chr` error).
            if (av <= 32'h0010FFFF && !(av >= 32'h0000D800 && av <= 32'h0000DFFF)) begin
              wbuf_a[0] <= hp;        wbuf_d[0] <= 32'd1;                     // header (length 1)
              wbuf_a[1] <= hp + 8'd1; wbuf_d[1] <= av;                       // the codepoint
              rf_we = 1'b1; rf_wd = {{ZPAD{1'b0}}, hp};                      // regs[dst] <- the new list base
              wbuf_n <= 3'd2; wbuf_i <= 3'd0; drng <= 1'b1; hp <= hp + 8'd2;
            end else begin halted <= 1'b1; trapped <= 1'b1; end               // invalid codepoint -> trap
          end
          8'd234: begin                                                              // TextContains {dst, hay=ra, needle=rb}
            tcs <= 1'b1; tcs_ph <= 3'd0; tcs_hp <= av[HPW-1:0]; tcs_np <= bv[HPW-1:0];
            tcs_res <= 1'b0; tcs_dst <= dst[RW-1:0]; dm_ra = av[HPW-1:0];     // issue |hay| read, start the scan
          end
          8'd235: begin                                                              // RepeatSeq {dst, elem=ra, count=rb} — n-slot scalar fill
            // A fresh `count`-slot Seq, every slot = the scalar regs[elem]; `count <= 0` is empty. Runs on the
            // uniform build-microcode (b_op == 235): header = count, then count identical cells. `bv` is the
            // count (an i32; a negative count -> 0 slots via the signed guard), `av` the scalar element.
            bld <= 1'b1; b_op <= 8'd235; b_base <= hp; b_i <= 8'd0; b_dst <= dst[RW-1:0]; b_ph <= 1'b0;
            b_n <= ($signed(bv) > 0) ? bv[HPW-1:0] : {HPW{1'b0}}; b_fill <= av;
          end
          8'd236: begin                                                              // ParseInt {dst, text=ra} — decimal char-scan
            pin <= 1'b1; pin_ph <= 3'd0; pin_ptr <= av[HPW-1:0]; pin_dst <= dst[RW-1:0];
            pin_err <= 1'b0; pin_saw <= 1'b0; pin_neg <= 1'b0; pin_acc <= 32'd0;
            dm_ra = av[HPW-1:0];                                              // issue the length read, start the scan
          end
          8'd237: begin                                                              // TextBytes {dst, text=ra} — UTF-8 encode
            tbx <= 1'b1; tbx_ph <= 2'd0; tbx_ptr <= av[HPW-1:0]; tbx_dst <= dst[RW-1:0];
            tbx_base <= hp; tbx_w <= hp + 8'd1;                               // header at hp, bytes from hp+1
            dm_ra = av[HPW-1:0];                                              // issue the length read, start the encode
          end
          8'd52: begin dm_we = 1'b1; dm_wa = hp; dm_wd = MAP_TAG; begin rf_we = 1'b1; rf_wd = hp; end hp <= hp + 8'd1; pc <= pc + 8'd1; end // NewEmptyMap
          8'd64: begin                                                              // FormatValue {dst, src=ra} — decimal string of av
            // COMPACT MULTI-CYCLE: kick off the `fmtf` FSM, which peels one decimal digit per cycle on ONE
            // shared ÷10/%10 (not the old 10-wide combinational unroll that congested nextpnr) into fmt_digit,
            // then hands the buffer to the uniform build-microcode. `binary_trees` (`"…" + <int>`) needs this,
            // so it is now UNCONDITIONAL (no longer dropped under NO_BIGINT).
            fmtf <= 1'b1; fmtf_neg <= av[31]; fmtf_n <= av[31] ? (32'd0 - av) : av; fmtf_k <= 4'd0; fmtf_dst <= dst[RW-1:0];
          end
          8'd63: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd63; end              // Contains {dst, coll=ra, value=rb} — issue read of the length
          8'd70: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd70; ld_ph <= 4'd0; end // ListPop {dst, list=ra} — issue read of the length header
          8'd74: begin                                                              // NewStruct {dst, num_fields=rb}
            bld <= 1'b1; b_op <= opcode; b_base <= hp; b_i <= 8'd0; b_n <= rb; b_dst <= dst[RW-1:0];
          end
          8'd75, 8'd84: begin                                                       // StructInsert {obj=dst, value=ra, field=rb} / CrdtResolve (register last-write = field overwrite)
            dm_we = 1'b1; dm_wa = dv[HPW-1:0] + 8'd1 + rb; dm_wd = av; pc <= pc + 8'd1;
          end
          8'd76: begin dm_ra = av[HPW-1:0] + 8'd1 + rb; ld <= 1'b1; ld_op <= 8'd76; end  // GetField {dst, obj=ra, field=rb} — issue read of the field
          8'd85: begin                                                              // IterPrepare {iterable=ra}
            iter_ptr[isp[2:0]] <= av[HPW-1:0]; iter_cur[isp[2:0]] <= 8'd1; isp <= isp + 4'd1; pc <= pc + 8'd1;
          end
          8'd86: begin dm_ra = iter_ptr[isp - 4'd1]; ld <= 1'b1; ld_op <= 8'd86; ld_ph <= 4'd0; end // IterNext {dst, exit=ra-byte} — issue read of the collection length
          8'd231: begin dm_ra = iter_ptr[isp - 4'd1]; ld <= 1'b1; ld_op <= 8'd231; ld_ph <= 4'd0; end // FIterNext {dst, exit=ra-byte} — a `Seq of Float` iterates the SAME int structure; ph0 reads the length off dmem (like op 86), ph1 reads the f64 PAYLOAD off fdmem into fregs[dst]
          8'd87: begin isp <= isp - 4'd1; pc <= pc + 8'd1; end                       // IterPop
          8'd45: begin begin rf_we = 1'b1; rf_wd = globals[ra[RW-1:0]]; end pc <= pc + 8'd1; end       // GlobalGet {dst, idx=ra-byte}
          8'd46: begin globals[dst[RW-1:0]] <= av; pc <= pc + 8'd1; end                   // GlobalSet {idx=dst, src=ra}
          // ── CONCURRENCY coprocessor: FIFO channels (fixed 8-slot ring [head, count, s0..s7]) ──
          8'd92: begin                                                              // ChanNew {dst} — [head, count, 8 ring slots] = 10 cells of 0
            bld <= 1'b1; b_op <= opcode; b_base <= hp; b_i <= 8'd0; b_n <= 8'd9; b_dst <= dst[RW-1:0];
          end
          8'd93: begin dm_ra = dv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd93; ld_ph <= 4'd0; end // ChanSend {chan=dst, val=ra} — issue read of the head
          8'd94: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd94; ld_ph <= 4'd0; end             // ChanRecv {dst, chan=ra} — issue read of the head
          8'd95: begin dm_ra = av[HPW-1:0] + 8'd1; ld <= 1'b1; ld_op <= 8'd95; ld_ph <= 4'd0; end // ChanTrySend {dst=success, chan=ra, val=rb} — issue read of the count guard
          8'd96: begin dm_ra = av[HPW-1:0] + 8'd1; ld <= 1'b1; ld_op <= 8'd96; ld_ph <= 4'd0; end // ChanTryRecv {dst=value, chan=ra} — issue read of the count guard
          8'd88: begin pc <= pc + 8'd1; end                                          // Sleep — deterministic scheduler no-op
          8'd72, 8'd73: begin begin rf_we = 1'b1; rf_wd = 32'd0; end pc <= pc + 8'd1; end           // LoadToday / LoadNow — placeholder epoch (RTC on real HW)
          8'd7, 8'd61, 8'd112: begin pc <= pc + 8'd1; end                            // EnsureOwned / RegionBoundsGuard / Obs — value-transparent no-op
          // Offline networking (no peer/wire on a single-node chip): Connect/Listen/Send/Stream/Sync are
          // no-ops (Sync leaves its variable unchanged) — the VM's tw==VM-net==AOT offline model.
          8'd105, 8'd106, 8'd108, 8'd111: begin pc <= pc + 8'd1; end                  // NetConnect / NetListen / NetStream / NetSync — offline no-op
          8'd110: begin begin rf_we = 1'b1; rf_wd = peer_seq; end peer_seq <= peer_seq + 32'd1; pc <= pc + 8'd1; end // NetMakePeer — a fresh offline peer key
          8'd107: begin                                                             // NetSend {to=ra, msg=rb} — PUBLISH (peer key, message) to the loopback inbox
            ib_slot = 3'd4;
            for (di = 3; di >= 0; di = di - 1) if (!ibox_val[di]) ib_slot = di[2:0];  // lowest free slot
            if (ib_slot != 3'd4) begin
              ibox_key[ib_slot] <= av; ibox_msg[ib_slot] <= bv; ibox_val[ib_slot] <= 1'b1;   // av=regs[ra] peer key, bv=regs[rb] message
            end
            pc <= pc + 8'd1;
          end
          8'd109: begin                                                             // NetAwait {dst, from=ra} — DRAIN the inbox entry keyed by the peer
            ib_hit = 3'd4;
            for (di = 3; di >= 0; di = di - 1) if (ibox_val[di] && ibox_key[di] == av) ib_hit = di[2:0]; // av=regs[ra] peer key; lowest match
            if (ib_hit != 3'd4) begin begin rf_we = 1'b1; rf_wd = ibox_msg[ib_hit]; end ibox_val[ib_hit] <= 1'b0; end
            pc <= pc + 8'd1;
          end
          8'd98: begin                                                              // Spawn {entry=dst} — deterministic sync launch (fire-and-forget)
            // Save the full caller window (cl engine, no seed — the child shares the caller's regs), jump to dst.
            cl <= 1'b1; cl_mode <= 2'd2; cl_ph <= 2'd0; cl_j <= {(RW+1){1'b0}}; cl_seed_ph <= 2'd0; cl_needentry <= 1'b0;
            cl_retpc <= pc + 16'd1; cl_retdst <= {RW{1'b0}}; cl_entry <= {8'd0, dst};
          end
          8'd99: begin                                                              // SpawnHandle {handle=dst, entry=ra} — sync launch + handle
            begin rf_we = 1'b1; rf_wd = 32'd0; end                                                 // regs[dst]=0 (handle placeholder) BEFORE the frame snapshot
            cl <= 1'b1; cl_mode <= 2'd3; cl_ph <= 2'd0; cl_j <= {(RW+1){1'b0}}; cl_seed_ph <= 2'd0; cl_needentry <= 1'b0;
            cl_retpc <= pc + 16'd1; cl_retdst <= {RW{1'b0}}; cl_entry <= {8'd0, ra};
          end
          8'd101: begin pc <= pc + 8'd1; end                                        // TaskAbort {handle} — no-op (task already ran)
          8'd100: begin pc <= pc + 8'd1; end                                        // TaskAwait {dst, handle} — instant join (sync child already completed); control passes through
          8'd97: begin pc <= pc + 8'd1; end                                         // ChanClose {chan} — closed flag no receive reads; output-transparent
          8'd38: begin                                                              // CallBuiltin {dst, args_start=ra, builtin=rb} — kernel-builtin coprocessor
            case (rb)
              8'd5:  begin rf_we = 1'b1; rf_wd = av[31] ? (~av + 32'd1) : av; pc <= pc + 8'd1; end  // Abs (av=regs[args_start])
              // Two-operand ops need regs[args_start+1] (Min/Max's second value, or the WORD32 rotate/shift
              // count): issue the port-B read now, retire next cycle. Min=7, Max=8, Rotl=30, Rotr=31,
              // Word32Shr=32. The WORD ops compute over the 32-bit pattern AS UNSIGNED (a rotate; a LOGICAL
              // right shift that zero-fills, distinct from the ALU's ARITHMETIC Shr).
              8'd7, 8'd8, 8'd30, 8'd31, 8'd32: begin
                a_rb = ra[RW-1:0] + 1'b1; cb2 <= 1'b1; cb_op <= rb; cb_av <= av; cb_dst <= dst[RW-1:0]; // pc HOLDS
              end
              // FLOAT abs (36) — clear the sign bit of fregs[ra] (fav), combinational, one cycle. Matches
              // Rust `f64::abs` bit-for-bit (incl. NaN: only the sign flips).
              8'd36: begin ff_we = 1'b1; ff_wa = dst[RW-1:0]; ff_wd = {1'b0, fav[62:0]}; pc <= pc + 8'd1; end
              // FLOAT min (37) / max (38) — the second operand is fregs[ra+1] (the encoder places the two args
              // in CONSECUTIVE float registers). fbv currently = fregs[rb]=the ordinal, so re-issue the float
              // read at ra+1 and finish next cycle (the `fcb` block), mirroring the int `cb2` two-operand path.
              8'd37, 8'd38: begin fa_rb = ra[RW-1:0] + 1'b1; fcb <= 1'b1; fcb_max <= (rb == 8'd38); fcb_dst <= dst[RW-1:0]; end // pc HOLDS
              default: begin rf_we = 1'b1; rf_wd = 32'd0; pc <= pc + 8'd1; end
            endcase
          end
          8'd102: begin sel_chan[sel_n] <= dst[RW-1:0]; sel_cptr[sel_n] <= dv; sel_var[sel_n] <= ra[RW-1:0]; sel_tmo[sel_n] <= 1'b0; sel_n <= sel_n + 4'd1; pc <= pc + 8'd1; end // SelectArmRecv {chan=dst, var=ra}: resolve channel ptr (dv=regs[dst]) now
          8'd103: begin sel_tmo[sel_n] <= 1'b1; sel_var[sel_n] <= ra[RW-1:0]; sel_n <= sel_n + 4'd1; pc <= pc + 8'd1; end                              // SelectArmTimeout {ticks=ra}
          8'd104: begin                                                             // SelectWait {dst_arm} — poll arms synchronously, then dequeue the first-ready recv (else timeout)
            sel_fired <= 4'hf;
            if (sel_n == 4'd0) begin rf_we = 1'b1; rf_wd = 32'hffffffff; pc <= pc + 8'd1; end
            else begin dm_ra = sel_cptr[0][HPW-1:0] + 8'd1; ld <= 1'b1; ld_op <= 8'd104; ld_ph <= 4'd0; end
          end
          // ── CRDT coprocessor: convergent counter (bump + field-wise sum-merge) ───────────────
          8'd80: begin dm_ra = dv[HPW-1:0] + 8'd1 + {4'd0, rb[3:0]}; ld <= 1'b1; ld_op <= 8'd80; end // CrdtBump {obj=dst, amount=ra, field=rb[3:0], negate=rb[4]} — issue read of the counter field
          8'd81: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd81; end              // CrdtMerge {target=dst, source=ra} — issue read of the source length
          8'd113: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd113; end             // CrdtJoin {target=dst, source=ra} — issue read of the source lane count
`ifndef NO_BIGINT
          8'd114: begin                                                               // BigIntWide: op = dst[7:4]
            case (dst[7:4])
              4'd0: begin wf_we = 1'b1; wf_wa = dst[2:0]; wf_wd = {224'd0, av};        pc <= pc + 8'd1; end // FromInt: W[d]=zext(R[a])
              4'd1: begin wf_we = 1'b1; wf_wa = dst[2:0]; wf_wd = bi_sum;              pc <= pc + 8'd1; end // Add   (bigint_alu)
              4'd2: begin wf_we = 1'b1; wf_wa = dst[2:0]; wf_wd = bi_diff;             pc <= pc + 8'd1; end // Sub   (bigint_alu)
              4'd3: begin rf_we = 1'b1; rf_wa = dst[3:0]; rf_wd = {31'd0, bi_lt};      pc <= pc + 8'd1; end // Cmp: R[d]=(W[a]<W[b]) — dst[7:4]=kind, reg is the LOW nibble
              4'd4: begin rf_we = 1'b1; rf_wa = dst[3:0]; rf_wd = wide[ra[2:0]][31:0]; pc <= pc + 8'd1; end // Low: R[d]=W[a][31:0]
              4'd5: begin bd_start <= 1'b1; bd_busy <= 1'b1;                                                // Div: issue bigint_div, stall
                          bd_a <= ra[2:0]; bd_b <= rb[2:0]; bd_dst <= dst[2:0]; end                        //   (pc holds; bd_busy retires)
              4'd6: begin wf_we = 1'b1; wf_wa = dst[2:0]; wf_wd = bi_mul;              pc <= pc + 8'd1; end // Mul: low 256 bits of W[a]*W[b]
              default: pc <= pc + 8'd1;
            endcase
          end
`endif
          8'd77: begin                                                              // NewInductive {dst, ctor=ra-byte, args=rb[7:4], count=rb[3:0]}
            bld <= 1'b1; b_op <= opcode; b_base <= hp; b_i <= 8'd0; b_n <= {4'd0, rb[3:0]};
            b_dst <= dst[RW-1:0]; b_src <= rb[7:4]; b_hdr <= {24'd0, ra};
          end
          8'd78: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd78; end              // TestArm {dst, target=ra, variant=rb} — issue read of the variant tag
          8'd79: begin dm_ra = av[HPW-1:0] + 8'd1 + rb; ld <= 1'b1; ld_op <= 8'd79; end  // BindArm {dst, target=ra, index=rb} — issue read of the field
          8'd51: begin dm_we = 1'b1; dm_wa = hp; dm_wd = 32'd0; begin rf_we = 1'b1; rf_wd = hp; end hp <= hp + 8'd1; pc <= pc + 8'd1; end // NewEmptySet {dst}
          8'd55: begin dm_ra = dv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd55; end  // SetAdd {set=dst, value=ra} — issue read of the length
          8'd53: begin                                                              // NewRange {dst, start=ra, end=rb} — inclusive [start..=end]
            bld <= 1'b1; b_op <= opcode; b_base <= hp; b_i <= 8'd0;
            b_n <= ($signed(bv) >= $signed(av)) ? (bv[HPW-1:0] - av[HPW-1:0] + 8'd1) : 8'd0; b_dst <= dst[RW-1:0]; b_lo <= av; // signed inclusive [lo..=hi]: a negative lo is a valid start; empty when hi < lo
          end
          8'd68: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd68; ld_ph <= 4'd0; end // UnionOp {dst, lhs=ra, rhs=rb} — issue read of the lhs length
          8'd69: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd69; ld_ph <= 4'd0; end // IntersectOp {dst, lhs=ra, rhs=rb} — issue read of the lhs length
          8'd47: begin dm_ra = dv[HPW-1:0] + 8'd1 + rb; ld <= 1'b1; ld_op <= 8'd47; ld_ph <= 4'd0; end // ListPushField {obj=dst, src=ra, field=rb} — issue read of the list_ptr field
          8'd71: begin itr <= 1'b1; i_op <= 8'd71; i_s1 <= av[HPW-1:0]; i_l1 <= rb; i_rd <= dst[RW-1:0]; i_si <= 8'd0; i_rp <= 3'd0; end // DestructureTuple {start=dst, src=ra, count=rb} — sequential scatter into regs
          8'd65: begin                                                              // SliceOp {dst, coll=ra, start=rb[7:4], end=rb[3:0]} — 1-based inclusive
            bld <= 1'b1; b_op <= opcode; b_base <= hp; b_i <= 8'd0; b_dst <= dst[RW-1:0];
            b_n <= (rb[3:0] >= rb[7:4]) ? ({4'd0, rb[3:0]} - {4'd0, rb[7:4]} + 8'd1) : 8'd0;
            b_csrc <= av[HPW-1:0] + {4'd0, rb[7:4]} - 8'd1;
          end
          8'd56: begin dm_ra = dv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd56; end  // RemoveFrom {collection=dst, value=ra} — issue read of the length
          8'd57, 8'd58: begin dm_ra = dv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd57; ld_ph <= 4'd0; end // SetIndex {coll=dst, index/key=ra, value=rb} — issue read of the header (map tag vs list)
          8'd223: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd223; end // MapIndexInt {dst, map=ra, key=rb} — issue read of the map header (npairs)
          8'd224: begin dm_ra = dv[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd224; end // MapSetIndexInt {map=dst, key=ra, value=rb} — issue read of the map header (npairs)
          8'd66: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= 8'd66; end              // DeepClone {dst, src=ra} — issue read of the source length
          8'd33, 8'd32: begin dm_ra = av[HPW-1:0]; ld <= 1'b1; ld_op <= opcode; ld_ph <= 4'd0; end // SeqConcat / Concat — issue read of the lhs length
          8'd12, 8'd13, 8'd14, 8'd15: begin div_start <= 1'b1; div_busy <= 1'b1; div_dst <= dst[RW-1:0]; div_op <= opcode; end  // Div/ExactDiv/Mod/FloorDiv -> shared_divider (stall until done)
          8'd18: begin pow_busy <= 1'b1; pow_dst <= dst[RW-1:0]; pow_base <= av; pow_acc <= 32'd1; pow_cnt <= bv; end             // Pow {dst, lhs=a, rhs=b} -> sequential power unit (acc=1, multiply by base bv times; stall until done)
          default: begin begin rf_we = 1'b1; rf_wd = alu_y; end pc <= pc + 8'd1; end               // ALU ops
        endcase
        if (rf_we) begin rf0[rf_wa] <= rf_wd; rf1[rf_wa] <= rf_wd; rf2[rf_wa] <= rf_wd; end // ONE register write port -> all three BRAM mirrors
        if (ff_we) begin                                                                     // ONE float write port -> both float mirrors, each split lo/hi (two 32-bit BRAMs)
          frf0_lo[ff_wa] <= ff_wd[31:0];  frf0_hi[ff_wa] <= ff_wd[63:32];
          frf1_lo[ff_wa] <= ff_wd[31:0];  frf1_hi[ff_wa] <= ff_wd[63:32];
        end
        if (dm_we) dmem[dm_wa] <= dm_wd;   // the ONE dmem write port
        if (fdm_we) begin fdmem_lo[fdm_wa] <= fdm_wd[31:0]; fdmem_hi[fdm_wa] <= fdm_wd[63:32]; end // the ONE float-heap write port (split lo/hi)
        if (sr_we) savedregs[sr_wa] <= sr_wd;            // the ONE savedregs (call-stack) write port
        if (sfr_we) begin savedfregs_lo[sfr_wa] <= sfr_wd[31:0]; savedfregs_hi[sfr_wa] <= sfr_wd[63:32]; end // the ONE float call-stack write port (split lo/hi)
`ifndef NO_BIGINT
        if (wf_we) wide[wf_wa] <= wf_wd;   // the ONE wide-bank (256-bit) write port
`endif
        dm_rq <= dmem[dm_ra];              // the ONE synchronous dmem read port (data valid next cycle)
        fdm_rq <= {fdmem_hi[fdm_ra], fdmem_lo[fdm_ra]}; // the ONE synchronous float-heap read port (reconstructed {hi,lo})
        // The THREE registered register-file read ports + the savedregs read port. They latch every cycle
        // from the (possibly FSM-overridden) addresses, so av/bv/dv/sv are valid the cycle after their address.
        av <= rf0[a_ra];
        bv <= rf1[a_rb];
        dv <= rf2[a_dst];
        instr_q <= imem[pc[IAW-1:0]];      // the REGISTERED fetch — imem[pc] valid next cycle (the first bubble stage)
        imdata_q <= imem[im_rb];           // the REGISTERED data word — imem[im_rb] valid next cycle (multi-word ops)
        sv <= savedregs[sr_ra];
        sfv <= {savedfregs_hi[sfr_ra], savedfregs_lo[sfr_ra]}; // the registered float call-stack read port
        fav <= {frf0_hi[fa_ra], frf0_lo[fa_ra]};  // the TWO registered float read ports, each reconstructed
        fbv <= {frf1_hi[fa_rb], frf1_lo[fa_rb]};  // from its {hi,lo} 32-bit BRAMs — same bubble as av/bv/dv
        pc_q <= pc;                        // remember this cycle's PC — the operand-bubble reference
        pc_qq <= pc_q;                     // and the cycle before — the SECOND bubble stage (fetch + operand latency)
      end
    end
  end
endmodule
