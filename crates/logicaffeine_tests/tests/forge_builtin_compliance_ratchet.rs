//! FORGE BUILTIN COMPLIANCE RATCHET — a pit-of-success lock that pulls the copy-and-patch forge tier
//! toward compiling every builtin that CAN be native, and can only move forward.
//!
//! The forge natively lowers every tierable builtin it has codegen for. What began as a single native
//! builtin (`Sqrt`) is now the whole deterministic scalar/Word/`chr` surface — 27 builtins. This ratchet
//! keeps that HONEST, VISIBLE, and MONOTONE by classifying every builtin into three buckets:
//!
//!   • `Tiered`     — the forge emits native code for it (VERIFIED by a hot ground-truth program below).
//!   • `ShouldTier` — deterministic SCALAR compute (numeric + Word/crypto) that the forge SHOULD lower
//!                    but currently declines. THIS IS THE GAP — the RED target the codegen drives to GREEN.
//!                    Holds `word8`/`intOfWord8`: the ring ℤ/2⁸ pair, awaiting a Word8-width stencil.
//!   • `HostNA`     — host / IO / allocation / boxed-tower / SIMD-lane / nondeterministic builtins that
//!                    legitimately never tier on a scalar copy-and-patch backend (format/parse/UUID/
//!                    temporal/money/wire-codec/DOM/RTC/signals/file/random/Reed-Solomon/SIMD lanes).
//!
//! Compliance % = `Tiered / (Tiered + ShouldTier)` — the fraction of the tierABLE surface that is native.
//! Today 27 / (27 + 2) ≈ 93.1%. The `HostNA` bucket is EXCLUDED from the denominator so 100% is
//! actually reachable (honest N/A lane).
//!
//! RATCHET (RED → GREEN): to make a `ShouldTier` builtin native, add its forge codegen, move it to
//! `TIERED` with a hot ground-truth program (RED until it tiers, GREEN when `fn+region >= 1` and it
//! matches the tree-walker), and raise `TIERED_FLOOR`. The gap only shrinks; a builtin can never silently
//! fall back (the ground-truth catches it), and a NEW builtin trips the frozen catalog count, forcing an
//! explicit Tiered / ShouldTier / HostNA decision. Tasks #58–#60 (numeric → Word32/64 → `chr`) all cleared.

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::semantics::builtins::BuiltinId;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Builtins the forge lowers to native code — VERIFIED tiering by `hot_program` below. GROWS, never
/// shrinks. Today: the full numeric lane (`sqrt`/`abs`/`min`/`max`/bit-counts/`floor`/`ceil`/`round`/`pow`),
/// the entire Word32/64 crypto lane (15 ops), and `chr` — 27 builtins, the whole tierable surface.
const TIERED: &[BuiltinId] = &[
    BuiltinId::Sqrt,
    BuiltinId::Abs,
    BuiltinId::Min,
    BuiltinId::Max,
    BuiltinId::CountOnes,
    BuiltinId::LeadingZeros,
    BuiltinId::TrailingZeros,
    BuiltinId::Floor,
    BuiltinId::Ceil,
    BuiltinId::Round,
    BuiltinId::Pow,
    // Word (crypto) lane — carried UNBOXED as a masked i64, re-boxed via RegBox::Word{32,64}.
    BuiltinId::Word32,
    BuiltinId::Word64,
    BuiltinId::Word16Make,
    BuiltinId::Wand,
    BuiltinId::Wor,
    BuiltinId::Word64And,
    BuiltinId::IntOfWord32,
    BuiltinId::IntOfWord64,
    BuiltinId::IntOfWord16,
    BuiltinId::Word64Shl,
    BuiltinId::Word32Shr,
    BuiltinId::Wnot,
    BuiltinId::Rotl,
    BuiltinId::Rotr,
    BuiltinId::Word64Shr,
    // `chr(code)` — validated by a DEOPTABLE `ChrGuard` (an invalid scalar side-exits so the VM re-runs
    // `chr` and traps identically) and carried UNBOXED as its code point (`Kind::CharText`), re-boxed to a
    // 1-char `Text` via `RegBox::CharText` only at a region output. Consumed to a scalar within a region by
    // `length of chr(k)` — `Utf8Len` computes the UTF-8 byte length from the code register (byte-exact vs
    // the tree-walker's byte-count `Length`), so the whole `chr`→`length` idiom runs native with no alloc.
    BuiltinId::Chr,
];

/// Monotone floor on the tiered count. RAISE (never lower) when a builtin's forge codegen lands.
const TIERED_FLOOR: usize = 27;

/// THE GAP — deterministic scalar-compute builtins the forge SHOULD lower but currently declines to VM.
/// `word8`/`intOfWord8` — the ring ℤ/2⁸ pair, byte-for-byte the same shape as `word16`/`intOfWord16`
/// (already `TIERED`, ℤ/2¹⁶): mask an `Int` down to 8 bits, or read a `Word8`'s value back as an `Int`
/// (`crates/logicaffeine_compile/src/semantics/builtins.rs` `Word8Make`/`IntOfWord8` arms). The forge has
/// no Word8-width stencil yet, so they decline to the VM; a NEW tierable builtin is added here first
/// (RED), then implemented and MOVED to `TIERED`; a builtin whose only use is a genuine host effect goes
/// HostNA.
const SHOULD_TIER: &[BuiltinId] = &[BuiltinId::Word8Make, BuiltinId::IntOfWord8];

/// The frozen `BuiltinId` catalog size. A new builtin bumps `BuiltinId::ALL` and trips this, forcing an
/// explicit Tiered / ShouldTier / HostNA decision (so nothing joins the catalog unclassified).
///
/// Last bump: 269→270 for `readReals` (→ HostNA): a file's binary64 values read into a boxed `Seq of Real`,
/// a host file read with no scalar compute in it, like `readBytes` beside it.
///
/// Before that: 265→269 for `sealBox`, `openBox`, `sealValueBox`, `openValueBox` (→ HostNA): XChaCha20-
/// Poly1305 boxes, whose seals draw their nonce from the host's entropy and whose bytes live in boxed
/// sequences, a host crypto effect like `sealBytes` beside them.
///
/// Before that: 264→265 for `orderedInfinitesimal` (→ HostNA): an infinitesimal declared much smaller or
/// much larger than another, a scale on the boxed exact tower whose values have no scalar lane to lower,
/// like `infinitesimal` beside it.
///
/// Before that: 263→264 for `exactPower` (→ HostNA): `**` to an exponent that is an exact number, a root
/// of an algebraic number or of a hyperreal held on the boxed exact tower, with no scalar lane to lower,
/// like `exactSqrt` beside it.
///
/// Before that: 262→263 for `definedReal` (→ HostNA): the real number a function of the program defines by
/// its digits or its approximations, a boxed computable real that calls back into the program whenever a
/// refinement asks, with no scalar lane to lower.
///
/// Before that: 261→262 for `firstExit` (→ HostNA): how many passes a loop makes whose guard compares
/// terms linear in a counter it steps, read on each residue class of the infinite generator over the
/// boxed exact tower, with no scalar lane to lower, like `tripCount` beside it.
///
/// Before that: 260→261 for `hybridFlow` (→ HostNA): the end state of a supertask whose state flows by a
/// polynomial Euler step between jumps, an exact run of the ideal hybrid system over real algebraic
/// numbers with no scalar lane to lower, like `polynomialFlow` beside it.
///
/// Before that: 254→260 for `exactSqrt`, `exactExp`, `exactLn`, `exactPi`, `exactE`, `exactOmega` (→ HostNA):
/// `sqrt`, `exp`, `ln`, `pi`, `e` and `omega` in a context that declares an exact type — algebraic
/// numbers, computable reals and ordinals on the boxed exact tower, with no scalar lane to lower, like
/// `standardPart` beside them.
///
/// Before that: 253→254 for `polynomialFlow` (→ HostNA): the end state of a supertask that is an Euler
/// step of a polynomial differential equation, a validated integration over exact reals with no
/// scalar lane to lower, like `affinePower` beside it.
///
/// Before that: 238→253 for the infinitesimal and supertask builtins — `infinitesimal`,
/// `standardPart`, `isInfinitesimal`, `isLimited`, `infinitelyClose`, `generatorOf`, `limitOf`,
/// `provisional`, `atTheLimit`, `everyNatural`, `tripCount`, `affinePower`, `cycleAt`,
/// `searchCandidates`, `stateSize` (→ HostNA): operations on the boxed exact tower above the rationals —
/// hyperreals, infinite integers, residue-dependent and provisional values — with no scalar lane to
/// lower, like `Decimal`/`Complex` beside them.
///
/// Before that: 234→238 for `EnvGet`/`TimeNow`/`EnvHas`/`EnvValue` (→ HostNA): env.lg's `get` and
/// time.lg's `now`, the system natives the tree-walker and the VM now reach through one table, and the
/// two halves the VM expands `get` into — a lookup in the process environment and a read of the real
/// clock, host state with no scalar compute in it, like `FileRead`/`RandomFloat` beside them.
///
/// Before that: 232→234 for `VfsWriteFile`/`VfsMount` (→ HostNA): the VM compiler's reserved calls
/// for `Write … to file …` and `Mount … at …`, onto the same sandboxed virtual file system the
/// tree-walker writes and mounts through — host IO with no scalar compute in it, like
/// `VfsReadFile`/`FileWrite` beside them.
///
/// Before that: 231→232 for `VfsReadFile` (→ HostNA): the VM compiler's reserved call for `Read …
/// from file …`, onto the same sandboxed virtual file system the tree-walker reads through — a
/// host IO lookup with no scalar compute in it, like `FileRead`/`ReadBytes` beside it.
///
/// Before that: 230→231 for `DomDispatchEntry` (→ HostNA): `dispatchEntry(dispatcher, context, hook,
/// node, event, value)`, a driving verb the program runs on its own page taken as the host's entry
/// into the page — the engine calls the dispatcher back and contains a failure there. A host entry
/// with no scalar compute in it, like every other `Dom*`.
///
/// Before that: 229→230 for `DomEnterValue` (→ HostNA): `enterValue(verb, selector, value)`, the
/// browser's own default action of a `Type`/`Choose` on the bytecode tier — whether the control takes
/// the value, and the value entered into the recorder's document. A DOM effect with no scalar compute
/// in it, like every other `Dom*`.
///
/// Before that: 228→229 for `DomWakeAfter` (→ HostNA): `wakeAfter(ms)`, asking the browser host to
/// call the page's clock when its earliest timer comes due — a host effect with no scalar compute in
/// it, like every other `Dom*`.
///
/// Before that: 227→228 for `DomJournalDetail` (→ HostNA): `journalDetail()`, whether the host keeps
/// the turn journal's detail — a question to the browser host with no scalar compute in it, like
/// every other `Dom*`.
///
/// Before that: 226→227 for `DomApplyWords` (→ HostNA): `applyPatchWords`, the word-format twin of the
/// bulk `applyPatches` — hands `vdom.lg`'s patch stream to the page's host in one call. A browser host
/// effect with no scalar compute in it, like every other `Dom*`.
///
/// Before that: 224→226 for `DomListenFlags`/`DomAssignLocation` (→ HostNA): registering a browser
/// listener with its capture/passive/stop behaviour, and loading another document
/// (`location.assign`) — browser host effects with no scalar compute in them, like every other
/// `Dom*`.
///
/// Before that: 223→224 for `DomEventFileBytes` (→ HostNA, S17): a dropped file's real content, read
/// from an actual browser event exactly like `DomEventFileName`/`DomEventFileSize`/
/// `DomEventFileType` beside it — host IO with no scalar compute in it.
///
/// Before that: 221→223 for `DomEventFileSize`/`DomEventFileType` (→ HostNA, S17): a dropped file's
/// real size/MIME type, read from an actual browser event exactly like `DomEventFileName` beside
/// them — host IO with no scalar compute in it.
///
/// Before that: 220→221 for `InspectUnhandledDynamic` (→ HostNA): the VM compiler's own dynamic-
/// fallback call for a non-exhaustive `Inspect` with no statically-known enum — a host-agnostic
/// error message built from a runtime value's already-resolved constructor/type strings, exactly
/// like `Fail`/`FailReason` beside it, never a scalar the forge could tier.
///
/// Before that: 215→219 for `Word8Make`/`IntOfWord8` (→ `SHOULD_TIER`, see above) and
/// `NativeKernel`/`NoteTurn` (→ HostNA, below).
///
/// `nativeKernel(code, ...)` (`__logos_native_kernel`, reserved) dispatches to one of the ML-KEM/
/// ML-DSA post-quantum kernels declared `## To native` in `crypto.lg` (NTT butterflies, compression,
/// rejection sampling, keypair/sign/verify — `semantics::native_kernels::NativeKernel`). Every one of
/// them operates on a `Seq of Int`/`Seq of Word16` — a boxed collection, not a scalar register value —
/// exactly the shape that puts `Lanes4Word32Make`/`Sha1Rnds4`/`ReedSolomonEncode` in HostNA rather than
/// `TIERED`, so `NativeKernel` joins them.
///
/// `noteTurn(action)` (reserved) records `action` as the current turn's journal entry in the browser
/// component runtime's turn history — a host effect with no scalar compute in it, and one a device tier
/// has no journal to receive at all (its own doc comment says so) — so HostNA, exactly like the other
/// reserved/host builtins beside it.
/// Before that: 196→198 for `DomEventKey` and `TryReadBytes`.
///
/// `eventKey(event)` reports the key of the keyboard event being dispatched, and it exists so
/// `on-keydown.enter` can gate on WHICH key was pressed — before it, key modifiers were refused
/// outright and "press Enter to submit" could not reach a browser. It asks the host for a fact
/// only the host has and performs no scalar compute, so HostNA, exactly like `DomEventValue`
/// beside it. (The comparison and case-folding it feeds are compiled LOGOS, which is the whole
/// reason this stayed one import rather than a gate in the host.)
///
/// `tryReadBytes(path)` is the TOTAL form of `readBytes`: a file's bytes as a `Result` rather
/// than a trap. It is the same file IO, answered instead of thrown — an OS result no amount of
/// scalar lowering can produce — so HostNA, exactly like `ReadBytes`.
/// Before that: 195→196 for `DomPreventDefault` (`preventDefault(el, event)`), the `.prevent` modifier:
/// it tells the browser not to perform its OWN action for an event the page has handled, which is
/// what stops a handled `<form>` submitting itself a second time. A pure host effect returning
/// nothing, with no scalar compute in it — HostNA, exactly like the DOM builtins beside it.
/// Before that: 189→193 for the STORAGE quartet (`storageSet`/`storageGet`/`storageGetOr`/
/// `storageRemove`), which is how a page keeps state between page loads. Every one of them is a
/// call into the host's `localStorage`/`sessionStorage` — IO with no scalar compute in it, and a
/// result the host alone knows — so HostNA, like the DOM builtins beside them. (`storageGetOr`
/// answers a stored Text or a fallback, which is a choice made on a HOST answer, not compute the
/// forge could perform.)
/// Before that: 188→189 for `DomApplyPatches` (`applyPatches(bytes)`), the BULK DOM call: a whole
/// patch stream applied in one host crossing instead of one crossing per operation, so the browser
/// host can be a `for` loop over a buffer rather than fifty entry points with behaviour behind
/// them. A pure host effect returning nothing, so HostNA.
/// Before that: 187→188 for `DomSetChecked` (`setChecked(el, on)`), which sets a checkbox's or
/// radio's live CHECKED PROPERTY. It cannot be `setAttribute(el, "checked", …)`, because the
/// attribute is `defaultChecked` — writing it leaves the box a visitor sees unchanged. Like every
/// other DOM builtin it is a pure host effect returning nothing, so HostNA.
/// Before that: 186→187 for `DomBeginPaint` (`beginPaint()`), which marks where one paint of a
/// component's view ends and the next begins so a browser host can reconcile the repaint in place
/// instead of destroying and rebuilding the subtree. It calls out to the host and returns nothing
/// — a pure browser effect with no scalar compute in it — so HostNA, exactly like the DOM-routing
/// builtins below.
/// Before that: 183→186 for the FLOAT SIGNAL trio (`SignalNewFloat`/`SignalGetFloat`/`SignalSetFloat`),
/// which gave a `Real` reactive cell its own builtins rather than bit-punning a float through the
/// generic integer cell. Like every other `Signal*` builtin they read and write the runtime's reactive
/// graph and schedule a re-render — host effects, not scalar compute — so HostNA, exactly as the
/// integer/text/seq signal builtins already are (no `Signal*` appears in TIERED or SHOULD_TIER).
/// Before that: 178→183 for the DOM-routing builtins (`DomRoutePath`/`DomRouteQuery`/`DomNavigate`/
/// `DomReplaceRoute`/`DomOnRoute`) — all browser host effects, likewise HostNA.
/// 193→195 for the civil-date constructors (`MakeDate`/`MakeMoment`): they build boxed `Date`/`Moment`
/// tower values from the `base::temporal` substrate (`days_from_civil`), not scalar Int/Word compute the
/// forge lowers — so HostNA, exactly like the other temporal builtins (`DateOf`/`YearOf`/`SecondsBetween`).
/// 198→199 for `DomSetTitle` (`Set the title to "…".`): it writes `document.title` through the
/// browser host — an effect on the page, not scalar compute the forge could lower — so HostNA, the
/// same classification every other `Dom*` builtin carries.
/// 199→200 for `DomReload` (`Reload the page.`): a browser host effect that tears the document
/// down — HostNA, like every other `Dom*`.
/// 200→201 for `DomGoBack` (`Go back.`): the browser's own history stepped by the page — a host
/// effect, HostNA.
/// 201→202 for `DomGetAttribute` (the read half of the attribute pair, which lets `Add class` be
/// compiled LOGOS rather than another host function): a DOM read — HostNA.
/// 202→203 for `DomOnWindowEvent` (`When the app goes offline:` and its two siblings): registering
/// a browser listener — HostNA.
/// 203→204 for `DomScrollGap` (the one geometry reading `When the user scrolls to the bottom:`
/// needs): a browser measurement — HostNA.
/// 204→205 for `DomSetInterval` (the page's single clock, for `Every …:` / `After …:`): arming a
/// browser timer — HostNA.
/// 205→206 for `DomEventFileName` (`When a file is dropped on …:`): reading a name out of a
/// browser's file transfer — HostNA.
/// 206→207 for `DomElementCommand` (`Focus`/`Blur`/`Hover`/`Scroll to`/`Select the text in`): one
/// named DOM operation on one element — HostNA.
/// 207→210 for `DomClipboardWrite`/`DomDownloadFile`/`DomNotify` (`Copy … to the clipboard.`,
/// `Download … as …`, `Notify …`): three browser capabilities — HostNA.
/// 210→211 for `DomMoveChildren` (the `{children}` slot: a wrapper's caller draws its children in
/// its own scope, and the wrapper takes them where its template says): reparenting nodes in a live
/// document — HostNA, like every other `Dom*`.
/// 211→212 for `DomOnMessage` (`When a message arrives on "…":`): subscribing to a relay topic
/// through the browser host — a network effect, HostNA, like every other `Dom*`.
/// 212→213 for `DomPublish` (`Publish M on "t".`): the OUTBOUND twin of `DomOnMessage`, which
/// shipped without it — a page could subscribe and had no way to send. Sending a frame on a
/// socket is a network effect, HostNA. Only the outward leg is here: a publish also reaches the
/// publishing page's own subscribers, and that delivery is compiled LOGOS, so it needs no builtin
/// and tiers like any other code.
/// 213→215 for `Fail`/`FailReason` (`Fail with …` and a contract's `, otherwise <reason>`): they
/// STOP the program with a runtime Text — control flow to an abort, no scalar compute — so HostNA.
/// A hot region that reaches one deopts and the VM raises the identical error
/// (`failure_under_the_forge_is_the_vm_failure` in `failure_all_tiers.rs` pins it).
/// 219→220 for `DomSetHeadHtml` (a page's `Head:` section, lowered): it writes the document
/// `<head>`'s markup through the browser host — an effect on the page, not scalar compute the
/// forge could lower — so HostNA, the same classification `DomSetTitle` (its nearest sibling) and
/// every other `Dom*` builtin carries.
const BUILTIN_TOTAL_FROZEN: usize = 270;

/// A hot loop that accumulates a scalar Int expression `expr` (over `k = 1..600`) into `acc` — the shared
/// shape for the Word-lane ground truths, which each fold a Word computation back to an Int via `intOfWordN`.
fn word_hot(expr: &str) -> String {
    format!(
        "## Main\n\
        Let mutable acc be 0.\n\
        Let mutable k be 1.\n\
        While k is at most 600:\n\
        \x20   Set acc to acc + ({expr}).\n\
        \x20   Set k to k + 1.\n\
        Show acc.\n"
    )
}

/// A hot program (calls the builtin ≥ the tier-up threshold) that MUST tier the forge and match the
/// tree-walker — the ground truth that a `TIERED` claim is real, not asserted. One per `TIERED` builtin.
fn hot_program(b: BuiltinId) -> String {
    match b {
        // `sqrt(x)` ISOLATED in a pure-float hot region — no other builtin (a companion like `floor`/
        // `round`/`toReal` is itself in the gap and would decline the region, masking sqrt). The float
        // result is folded to an exact int accumulator via a float COMPARE (which the forge lowers),
        // keeping the `Show` deterministic without a second builtin.
        BuiltinId::Sqrt => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable fk be 1.0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Let r be sqrt(fk).\n\
            \x20   If r is greater than 10.0:\n\
            \x20       Set acc to acc + 1.\n\
            \x20   Set fk to fk + 1.0.\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // `abs(x)` in a hot loop over values that go NEGATIVE (300 − k) — exercises the branchless
        // sign-mask path (mask = all-ones for x<0). Sum of |300 − k| for k = 1..=600.
        BuiltinId::Abs => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Let x be 300 - k.\n\
            \x20   Set acc to acc + abs(x).\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // `min(a,b)` / `max(a,b)` in a hot loop over two crossing ramps (k, 600−k) — exercises both
        // branches of the branchless select (a<b and a≥b).
        BuiltinId::Min => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + min(k, 600 - k).\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        BuiltinId::Max => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + max(k, 600 - k).\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // `count_ones(k)` where `k` is the REGISTER-PINNED induction variable — the exact case that the
        // frame-reading stencil miscompiled (stale frame slot). The register-form `V_POPCNT[d][s]` reads
        // the pinned source register directly, so the hot popcount is correct. Sum of popcount(k), k=1..600.
        BuiltinId::CountOnes => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + count_ones(k).\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // `leading_zeros(k)` / `trailing_zeros(k)` on the register-pinned induction variable — the `lzcnt`
        // / `tzcnt` register-form stencils (`V_LZCNT`/`V_TZCNT`) read the pin directly (64 for 0, but k
        // starts at 1 so it never hits 0 here). Sum over k = 1..600.
        BuiltinId::LeadingZeros => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + leading_zeros(k).\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        BuiltinId::TrailingZeros => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + trailing_zeros(k).\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // `floor(fk)` / `ceil(fk)` over a hot XMM-pinned float that crosses zero (300.5 → −299.5), so the
        // negative-fraction case (where floor ≠ truncate) is exercised, and the `V_FLOORF`/`V_CEILF`
        // register form reads the float from its XMM pin. Result accumulates into an Int.
        BuiltinId::Floor => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable fk be 300.5.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + floor(fk).\n\
            \x20   Set fk to fk - 1.0.\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        BuiltinId::Ceil => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable fk be 300.5.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + ceil(fk).\n\
            \x20   Set fk to fk - 1.0.\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // `round(fk)` over a hot XMM-pinned float that crosses zero on the HALF (300.5 → −299.5) — every
        // value is an exact `.5` tie, so this stresses the ties-AWAY path (300.5→301, −0.5→−1, …) that a
        // ties-even `roundsd` would get wrong.
        BuiltinId::Round => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable fk be 300.5.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + round(fk).\n\
            \x20   Set fk to fk - 1.0.\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // `pow(k, 3)` over the register-pinned induction variable — the constant exponent 3 unrolls to two
        // checked Muls of `k`. Sum of k³ for k = 1..600 stays in i64 (no BigInt promotion here; the
        // overflow→BigInt deopt is exercised by the gauntlet's `pow(k, 8)` edge case).
        BuiltinId::Pow => "## Main\n\
            Let mutable acc be 0.\n\
            Let mutable k be 1.\n\
            While k is at most 600:\n\
            \x20   Set acc to acc + pow(k, 3).\n\
            \x20   Set k to k + 1.\n\
            Show acc.\n"
            .to_string(),
        // WORD lane — each produces a Word from the hot `k`, computes, then folds back to an Int (the region
        // output is a scalar). The Word rides unboxed as a masked i64; correctness is the differential vs tw.
        BuiltinId::Word32 | BuiltinId::IntOfWord32 => word_hot("intOfWord32(word32(k))"),
        BuiltinId::Word64 | BuiltinId::IntOfWord64 => word_hot("intOfWord64(word64(k))"),
        BuiltinId::Word16Make | BuiltinId::IntOfWord16 => word_hot("intOfWord16(word16(k))"),
        BuiltinId::Wand => word_hot("intOfWord32(word_and(word32(k), word32(600 - k)))"),
        BuiltinId::Wor => word_hot("intOfWord32(word_or(word32(k), word32(600 - k)))"),
        BuiltinId::Word64And => word_hot("intOfWord64(word64And(word64(k), word64(600 - k)))"),
        BuiltinId::Word64Shl => word_hot("intOfWord64(word64Shl(word64(k), 5))"),
        BuiltinId::Word32Shr => word_hot("intOfWord32(word32Shr(word32(k), 3))"),
        BuiltinId::Wnot => word_hot("intOfWord32(word_not(word32(k)))"),
        BuiltinId::Rotl => word_hot("intOfWord32(rotl(word32(k), 7))"),
        BuiltinId::Rotr => word_hot("intOfWord32(rotr(word32(k), 7))"),
        // A high-bit-set Word64 (`word64(0 − k)`) shifted right by a CONSTANT 5 — the LOGICAL shift must
        // clear the top bits, not sign-extend, matching the VM's `(w as u64) >> 5`.
        BuiltinId::Word64Shr => word_hot("intOfWord64(word64Shr(word64(0 - k), 5))"),
        // `chr(k)` for k = 1..600 spans 1-byte ASCII and 2-byte Latin-1/BMP scalars (all valid, so the
        // `ChrGuard` never deopts here); `length of chr(k)` folds each to its UTF-8 byte length via
        // `Utf8Len`, so `acc` is the byte-length SUM — byte-exact vs the tree-walker's boxed-`Text` `Length`.
        BuiltinId::Chr => word_hot("length of chr(k)"),
        other => panic!(
            "builtin {other:?} is in TIERED but has no `hot_program` ground truth — add one so its \
             tiering claim is verified, not asserted"
        ),
    }
}

#[test]
fn forge_builtin_compliance_ratchet() {
    // FREEZE — a new builtin forces an explicit Tiered / ShouldTier / HostNA decision.
    assert_eq!(
        BuiltinId::ALL.len(),
        BUILTIN_TOTAL_FROZEN,
        "the BuiltinId catalog changed ({} vs frozen {BUILTIN_TOTAL_FROZEN}). Classify the new/removed \
         builtin: TIERED (forge-native, add a ground-truth program) / SHOULD_TIER (deterministic scalar \
         compute the forge should lower) / HostNA (host·IO·alloc·boxed·SIMD, never tiers), then update \
         BUILTIN_TOTAL_FROZEN.",
        BuiltinId::ALL.len()
    );

    // No builtin may be in BOTH TIERED and SHOULD_TIER (a tiered one is no longer a gap).
    for t in TIERED {
        assert!(
            !SHOULD_TIER.contains(t),
            "{t:?} is in both TIERED and SHOULD_TIER — remove it from SHOULD_TIER (the gap shrank)"
        );
    }

    // MONOTONE floor — the tiered count only rises; when it climbs past the floor, force raising it.
    assert!(
        TIERED.len() >= TIERED_FLOOR,
        "tiered builtin count {} fell below the floor {TIERED_FLOOR} — a builtin stopped tiering; fix \
         the codegen, never lower the floor",
        TIERED.len()
    );
    assert_eq!(
        TIERED.len(),
        TIERED_FLOOR,
        "tiered coverage ({}) moved off the floor ({TIERED_FLOOR}) — RAISE TIERED_FLOOR to {} to lock it",
        TIERED.len(),
        TIERED.len()
    );

    // SCOREBOARD toward 100% of the tierABLE surface (HostNA excluded from the denominator).
    let tierable = TIERED.len() + SHOULD_TIER.len();
    let host_na = BuiltinId::ALL.len() - tierable;
    eprintln!(
        "forge builtin compliance: {}/{} of the tierable scalar surface native ({:.1}%); \
         {} still to tier (the gap); {} HostNA (never tiers)",
        TIERED.len(),
        tierable,
        100.0 * TIERED.len() as f64 / tierable as f64,
        SHOULD_TIER.len(),
        host_na
    );

    // GROUND TRUTH — every claimed-tiered builtin actually tiers AND matches the tree-walker.
    for &b in TIERED {
        let src = hot_program(b);
        let tw = tw_outcome_with_args(&src, &[]);
        let tier = ForgeTier::new();
        let forge = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        assert!(
            tw.error.is_none(),
            "[{b:?}] tree-walker errored: {:?}",
            tw.error
        );
        assert!(
            forge.error.is_none(),
            "[{b:?}] forge errored: {:?}",
            forge.error
        );
        // CORRECTNESS holds on EVERY architecture — the forge either emits native code or declines
        // gracefully to the VM, which matches the tree-walker.
        assert_eq!(
            forge.output.trim(),
            tw.output.trim(),
            "[{b:?}] forge diverged from the tree-walker on its ground-truth program"
        );
        // TIERING is x86_64-only (the forge emits x86 stencils). On any other target the forge declines
        // to the VM by design — correctness is unaffected — so the "did it tier" assertion is x86-gated.
        #[cfg(target_arch = "x86_64")]
        {
            let (_, fn_ok) = tier.function_counts();
            let (_, region_ok) = tier.region_counts();
            assert!(
                fn_ok + region_ok >= 1,
                "[{b:?}] is in TIERED but did NOT tier its hot program (fn={fn_ok} region={region_ok}) — \
                 it declines to VM; move it back to SHOULD_TIER or fix the codegen"
            );
        }
    }
}
