//! Structural coverage locks for the Syntax Guide.
//!
//! `guide_examples.rs` proves every example RUNS as declared; this file locks
//! the guide's STRUCTURE:
//!
//!   * **Anchor lock** — section ids are DOM anchors and appear in public URLs
//!     (`main_nav.rs` emits `/guide#{id}`), so a published id may never vanish.
//!     The list is append-only; removing an entry requires the sanctioned
//!     dead-anchor ritual (an explicit comment at the removal site explaining
//!     why the anchor may break).
//!
//!   * **Reference integrity** — prose that points at "§N" / "Section N" /
//!     "Part <roman>" must point at something that exists. Renumbering and
//!     re-parting are display-only and expected (the overhaul renumbers per
//!     wave); this lock is what lets that churn happen without rotting a
//!     single cross-reference.

#![cfg(feature = "web-tests")]

use logicaffeine_compile::semantics::builtins::BuiltinId;
use logicaffeine_web::ui::pages::guide::content::{Runnability, SECTIONS};

/// Every section id ever published. APPEND-ONLY: new sections add their id
/// here; existing entries never leave (they are live URLs in the wild).
const ANCHOR_LOCK: &[&str] = &[
    "introduction",
    "reading-english",
    "getting-started",
    "tutorial-project",
    "part-1-checkpoint",
    "variables-and-types",
    // "numbers" + "everyday-types" — SANCTIONED REMOVALS (same-day, never deployed):
    // both merged INTO variables-and-types ("all the types in the Types section",
    // owner's call) hours after creation; no published URL ever carried them.
    "text",
    "operators",
    "control-flow",
    "functions",
    "closures",
    "collections",
    "user-types",
    "generics",
    "memory-ownership",
    "zones",
    "machine-words",
    "concurrency",
    "interop",
    "part-3-checkpoint",
    "crdt",
    "security",
    "networking",
    "part-4-checkpoint",
    "error-handling",
    "part-2-checkpoint",
    // "advanced-features" — SANCTIONED DEAD ANCHOR (W2.4, roadmap §4): the 2-example
    // grab-bag dissolved into `z3-verification` (refinement-types) and
    // `assertions-trust` (assertions). The one deliberate anchor break of the overhaul.
    "modules",
    "cli-largo",
    "stdlib",
    "execution-tiers",
    "part-5-checkpoint",
    "logic-mode",
    "assertions-trust",
    "z3-verification",
    "proofs",
    "part-6-checkpoint",
    "complete-examples",
    "recipes",
    "quick-reference",
    "appendix-keywords",
    "appendix-errors",
    "appendix-runnability",
    "building-web-apps",
];

#[test]
fn published_anchors_never_break() {
    let live: std::collections::HashSet<&str> = SECTIONS.iter().map(|s| s.id).collect();
    let dead: Vec<&&str> = ANCHOR_LOCK
        .iter()
        .filter(|id| !live.contains(**id))
        .collect();

    assert!(
        dead.is_empty(),
        "\npublished section anchor(s) no longer exist — these are live URLs: {dead:?}\n\
         (if a removal is truly sanctioned, remove the entry here IN THE SAME CHANGE \
         with a comment explaining the accepted dead anchor)\n"
    );
}

/// New sections must be added to the anchor lock the moment they exist —
/// otherwise the lock silently stops covering them.
#[test]
fn every_section_is_anchor_locked() {
    let locked: std::collections::HashSet<&str> = ANCHOR_LOCK.iter().copied().collect();
    let unlocked: Vec<&str> = SECTIONS
        .iter()
        .map(|s| s.id)
        .filter(|id| !locked.contains(id))
        .collect();

    assert!(
        unlocked.is_empty(),
        "\nsection id(s) missing from ANCHOR_LOCK — append them (the lock is what makes \
         them permanent URLs): {unlocked:?}\n"
    );
}

/// Every "§N" / "Section N" token in guide prose must name a live section
/// number, and every "Part <roman>" must match an existing part heading.
/// Renumbering (planned, per wave) goes red here wherever it breaks prose.
#[test]
fn prose_cross_references_resolve() {
    let live_numbers: std::collections::HashSet<u32> =
        SECTIONS.iter().map(|s| s.number as u32).collect();
    let parts: Vec<&str> = {
        let mut v = Vec::new();
        for s in SECTIONS.iter() {
            if !v.contains(&s.part) {
                v.push(s.part);
            }
        }
        v
    };

    let mut broken: Vec<String> = Vec::new();

    for section in SECTIONS.iter() {
        // §N and "Section N" numeric references.
        for cap in regex_lite_numbers(section.content) {
            if !live_numbers.contains(&cap) {
                broken.push(format!(
                    "[{}] references section number {} which does not exist",
                    section.id, cap
                ));
            }
        }
        // "Part <roman>" references must prefix-match a real part heading.
        for roman in regex_lite_parts(section.content) {
            let prefix = format!("Part {roman}");
            if !parts.iter().any(|p| p.starts_with(&prefix)) {
                broken.push(format!(
                    "[{}] references \"{prefix}\" but no part heading starts with it",
                    section.id
                ));
            }
        }
    }

    assert!(
        broken.is_empty(),
        "\nbroken prose cross-reference(s):\n{}\n",
        broken.join("\n")
    );
}

/// Where a builtin lives in the guide.
enum Home {
    /// Taught in this section (the id must exist).
    Section(&'static str),
    /// Will be taught in this section once its wave lands (the id must NOT
    /// exist yet — the moment the section is created, this entry goes red and
    /// must flip to `Section`, so every content wave gets its census RED free).
    Planned { section: &'static str },
    /// Compiler/runtime plumbing that is deliberately not a guide teaching
    /// surface. The reason is mandatory — it is the editorial decision on
    /// record.
    Internal(&'static str),
}

/// The census: every builtin declares its guide home. NO wildcard arm — a new
/// builtin fails to compile this test until someone decides where it is taught
/// (or records why it isn't). That is the point.
fn guide_home(b: BuiltinId) -> Home {
    use BuiltinId as B;
    match b {
        // ── Taught today ────────────────────────────────────────────────
        B::Length | B::MapOf | B::SetOf | B::RepeatSeq => Home::Section("collections"),
        B::Format | B::Abs | B::Min | B::Max => Home::Section("stdlib"),
        B::Copy => Home::Section("memory-ownership"),

        // ── Failure plumbing ────────────────────────────────────────────
        // Authors write `Fail with …` / `…, otherwise <reason>`; the compiler lowers those to these.
        B::Fail | B::FailReason => Home::Internal(
            "reserved names the compiler writes for `Fail with` and contract reasons; the statements are the teaching surface",
        ),

        // ── Text (landed W4) ────────────────────────────────────────────
        B::ParseInt | B::ParseFloat | B::Chr | B::Contains => Home::Section("text"),

        // ── Numbers (landed W3) ─────────────────────────────────────────
        B::Sqrt | B::Floor | B::Ceil | B::Round | B::Pow | B::Decimal | B::Complex
        | B::Modular => Home::Section("variables-and-types"),

        // ── Infinitesimals and supertasks ───────────────────────────────
        // `x is infinitesimal`, `x is limited`, `x is infinitely close to y` and `the standard part
        // of x` are the sentences; the names are taught beside them, and `generatorOf`/`limitOf`
        // as the calls they are, once the section is written.
        B::StandardPart | B::IsInfinitesimal | B::IsLimited | B::InfinitelyClose | B::GeneratorOf
        | B::LimitOf => Home::Planned {
            section: "infinitesimals",
        },
        // `sqrt`, `exp`, `ln`, `pi`, `e` and `omega` in a context that declares an exact type are these, and
        // `**` to an exact exponent is `exactPower`; the exact types' section teaches both spellings.
        B::ExactSqrt | B::ExactExp | B::ExactLn | B::ExactPi | B::ExactE | B::ExactOmega | B::ExactPower => {
            Home::Planned {
                section: "infinitesimals",
            }
        }
        B::Infinitesimal | B::OrderedInfinitesimal | B::AtTheLimit | B::EveryNatural | B::Provisional => Home::Internal(
            "reserved names the compiler writes for `Let … be an infinitesimal`, `Let … be an \
             infinitesimal much smaller than …`, `At the limit, take …`, `Repeat for every natural n` \
             and a search's provisional answer; the sentences are the teaching surface",
        ),
        B::DefinedReal => Home::Internal(
            "the reserved name the compiler writes for `the real number whose digits in base k \
             are f` and `the real number approximated by f`; the phrases are the teaching surface",
        ),
        B::TripCount
        | B::AffinePower
        | B::CycleAt
        | B::SearchCandidates
        | B::StateSize
        | B::PolynomialFlow
        | B::HybridFlow
        | B::FirstExit => Home::Internal(
            "the supertask engine's run-time operations, called by the statements it writes for a \
             resolved supertask; no program spells them",
        ),

        // ── Complex components (not yet taught) ─────────────────────────
        // `conjugate(z)`, `re(z)`, `im(z)` — the accessors that go with `complex`/`ComplexF`.
        // `complex` itself is taught in variables-and-types, but these three components are not
        // yet written up; when a wave documents them this Planned home turns RED to flip.
        B::Conjugate | B::RealPart | B::ImagPart => {
            Home::Planned { section: "complex-components" }
        }

        // ── W4: Time, Money, and Measures ───────────────────────────────
        B::Quantity | B::Money | B::SetRate | B::ToCurrency | B::SetRates | B::Convert
        | B::Uuid | B::UuidNil | B::UuidMax | B::UuidVersion | B::UuidDns | B::UuidUrl
        | B::UuidOid | B::UuidX500 | B::ParseTimestamp | B::FormatTimestamp | B::YearOf
        | B::MonthOf | B::DayOf | B::WeekdayOf | B::HourOf | B::MinuteOf | B::SecondOf
        | B::WeekOf | B::QuarterOf | B::DateOf | B::TimeOf | B::SecondsBetween
        | B::MonthsBetween | B::YearsBetween | B::AddSeconds | B::InZone
        | B::MakeDate | B::MakeMoment => {
            Home::Section("variables-and-types")
        }

        // ── Bits, Bytes, and Machine Words (landed W3) ──────────────────
        B::TextBytes | B::TextFromBytes | B::CountOnes | B::LeadingZeros | B::TrailingZeros
        | B::Word32 | B::Word64 | B::Rotl
        | B::Rotr | B::Wand | B::Wor | B::Wnot | B::IntOfWord32 | B::IntOfWord64
        | B::Word64Shl | B::Word64Shr | B::Word64And | B::Word32Shr | B::Word16Make
        | B::IntOfWord16 | B::Word8Make | B::IntOfWord8 => Home::Section("machine-words"),

        // ── Native-kernel plumbing ──────────────────────────────────────
        // A stdlib `## To native` kernel declaration (crypto.lg) is compiled to a call of this
        // reserved name carrying the kernel's code; the declaration is the only writer.
        B::NativeKernel => Home::Internal(
            "reserved name (`__logos_native_kernel`) the VM compiler writes for a stdlib \
             `## To native` kernel declaration; an author never calls it, and the kernels are \
             documented by the stdlib's literate notes",
        ),
        // ── Turn-journal plumbing ────────────────────────────────────────
        // Reserved name (`noteTurn`): the lowering emits the call as the first statement of every
        // `__act_` so `the journal`/`Replay` work on a lowered tier; an author never writes it.
        // Reserved name (`journalDetail`): the lowering asks it before capturing a turn's changed
        // fields, so a shipped page skips the capture; an author never writes it.
        B::DomJournalDetail => Home::Internal(
            "reserved name (`journalDetail`) the lowering asks before recording a turn's changed \
             fields, so a shipped page pays nothing for a journal nobody keeps; an author never \
             calls it — `the journal` is the teaching surface",
        ),
        B::NoteTurn => Home::Internal(
            "reserved name (`noteTurn`) the lowering writes at the top of every compiled action so \
             the journal survives lowering; an author never calls it — `the journal` and `Replay` \
             are the teaching surface",
        ),
        // ── Page-clock plumbing ─────────────────────────────────────────
        // Reserved name (`wakeAfter`): the lowering asks the host to wake the page when its
        // earliest timer comes due; an author never writes it.
        B::DomWakeAfter => Home::Internal(
            "reserved name (`wakeAfter`) the lowering writes after every timer is armed, stopped or \
             fired, asking the browser to wake the page when its earliest timer comes due; an author \
             never calls it — `Every … seconds:` and `After … seconds:` are the teaching surface",
        ),
        // ── Driving-verb plumbing ───────────────────────────────────────
        // The VM compiler's own call for a `Type`/`Choose`: the browser's default action of
        // entering the value; an author never calls it.
        B::DomEnterValue => Home::Internal(
            "the call the VM compiler writes for a `Type` or `Choose` — the browser's own default \
             action of entering the value into the control; it has no source name, and `Type … \
             into …` / `Choose … in …` are the teaching surface",
        ),
        B::DomDispatchEntry => Home::Internal(
            "the call the VM compiler writes for a driving verb a program runs on its own page — the \
             host's entry into the page, which contains a failing turn; it has no source name, and \
             `Click`/`Type`/`Choose`/`Press`/`Submit` are the teaching surface",
        ),
        // ── File-read plumbing ──────────────────────────────────────────
        // Reserved name (`__logos_read_from_file`): the VM compiler's own call for `Read … from
        // file`, onto the same sandboxed VFS the tree-walker reads through; an author never
        // writes it.
        B::VfsReadFile => Home::Internal(
            "reserved name (`__logos_read_from_file`) the VM compiler writes for `Read … from \
             file …` — the same sandboxed virtual file system the tree-walker reads through; an \
             author never calls it — `Read … from file …` is the teaching surface",
        ),
        B::VfsWriteFile => Home::Internal(
            "reserved name (`__logos_write_to_file`) the VM compiler writes for `Write … to \
             file …` — the same sandboxed virtual file system the tree-walker writes through; an \
             author never calls it — `Write … to file …` is the teaching surface",
        ),
        B::VfsMount => Home::Internal(
            "reserved name (`__logos_mount_at`) the VM compiler writes for `Mount … at …` — the \
             journal's text from the same sandboxed virtual file system the tree-walker mounts \
             through, or empty Text when there is none yet; an author never calls it — `Mount … \
             at …` is the teaching surface",
        ),
        // ── Inspect exhaustiveness plumbing ──────────────────────────────
        // The VM compiler's own dynamic-fallback call for a non-exhaustive `Inspect` with no
        // statically-known enum; an author never calls it — `Inspect`/`Otherwise` are the
        // teaching surface, not the message that fires when a match is missing an arm.
        B::InspectUnhandledDynamic => Home::Internal(
            "reserved name the VM compiler writes for a non-exhaustive `Inspect`'s dynamic \
             fallback; an author never calls it — `Inspect` and `Otherwise` are the teaching \
             surface",
        ),

        // ── Deliberately outside the guide ──────────────────────────────
        B::WireBytes | B::ReadWireProgram | B::WriteWireResidual | B::RunAccepted => {
            Home::Internal(
                "wire/partial-evaluation host protocol (resident PE server, shipped-function \
                 sandbox) — engine plumbing, not a teaching surface",
            )
        }
        B::UuidBytes | B::UuidFromBytes => Home::Internal(
            "uuid.lg byte-level substrate; the uuid stdlib module is the teaching surface",
        ),
        B::FileRead | B::FileWrite => Home::Internal(
            "file.lg's native seam (`## To native read`/`write`) shared by the tree-walker \
             and the VM; the file stdlib module's literate notes are the teaching surface",
        ),
        B::RandomInt | B::RandomFloat => Home::Internal(
            "random.lg's native seam (`## To native randomInt`/`randomFloat`) shared by the \
             tree-walker and the VM; the random stdlib module's literate notes are the \
             teaching surface",
        ),
        B::EnvGet | B::TimeNow => Home::Internal(
            "env.lg's `get` and time.lg's `now`, native seams shared by the tree-walker and the \
             VM; the env and time stdlib modules' literate notes are the teaching surface",
        ),
        B::EnvHas | B::EnvValue => Home::Internal(
            "the two halves the VM compiler expands env.lg's `get` into — reserved names an author \
             never spells",
        ),
        B::LocalInstant => Home::Internal(
            "lowering target for `the <component> of <moment> in \"<zone>\"` — the surface \
             form is taught in variables-and-types",
        ),
        B::Lanes4Word32Make | B::Lanes4Of | B::SeqOfLanes4W32 | B::Sha1Rnds4 | B::Sha1Msg1
        | B::Sha1Msg2 | B::Sha1Nexte | B::Lanes16Word8Make | B::SeqOfLanes16W8
        | B::Splat16Word8 | B::Shuffle16 | B::ShrBytes16 | B::InterleaveLo16
        | B::InterleaveHi16 | B::ByteAdd16 | B::Maddubs16 | B::Packus16 | B::Lanes8Word32
        | B::SeqOfLanes8 | B::Splat8Word32 | B::Lanes4Word64 | B::SeqOfLanes4
        | B::Mul32x32To64 | B::HsumLanes4 | B::Splat4Word64 | B::AndNot4 | B::Lanes16Word16
        | B::SeqOfLanes16 | B::Splat16Word16 | B::Mulhi16 | B::Montmul32 | B::NttBcastLo
        | B::NttBcastHi | B::NttBlend => Home::Internal(
            "SIMD lane vocabulary for the LOGOS-written crypto substrate (SHA-NI, Keccak, \
             NTT); documented by the stdlib's literate notes, deliberately outside the \
             guide's teaching surface",
        ),
        B::DomBody
        | B::DomCreateElement
        | B::DomCreateTextNode
        | B::DomSetText
        | B::DomSetHtml
        | B::DomClearChildren
        | B::DomMoveChildren
        | B::DomOnMessage
        | B::DomPublish
        | B::DomSetAttribute
        | B::DomAppendChild
        | B::DomGetText
        | B::DomGetValue
        | B::DomSetValue
        | B::DomQuerySelector
        | B::DomAddEventListener
        | B::DomEventValue
        | B::DomEventTarget
        | B::DomFetchText
        | B::DomResponseText
        | B::DomFetchSend
        | B::DomResponseStatus
        | B::DomResponseBody
        | B::DomResponseHeader
        | B::DomResponseError
        | B::DomFetchAbort
        | B::DomFetchSetTimeout
        | B::DomRoutePath
        | B::DomRouteQuery
        | B::DomNavigate
        | B::DomReplaceRoute
        | B::DomOnRoute
        | B::DomSetTitle
        | B::DomSetHeadHtml
        | B::DomReload
        | B::DomGoBack
        | B::DomGetAttribute
        | B::DomOnWindowEvent
        | B::DomScrollGap
        | B::DomSetInterval
        | B::DomEventFileName
        | B::DomEventFileSize
        | B::DomEventFileType
        | B::DomEventFileBytes
        | B::DomListenFlags
        | B::DomAssignLocation
        | B::DomElementCommand
        | B::DomClipboardWrite
        | B::DomDownloadFile
        | B::DomNotify
        | B::ReadBytes
        | B::TryReadBytes
        | B::SignalNew
        | B::SignalSet
        | B::SignalGetInt
        | B::SignalGetText
        | B::SignalBeginRender
        | B::SignalNewText
        | B::SignalSetText
        | B::SignalNewSeq
        | B::SignalGetSeq
        | B::SignalSetSeq
        | B::SignalNewFloat
        | B::SignalGetFloat
        | B::SignalSetFloat
        | B::SignalEnterComponent
        | B::DomSetStream
        | B::DomBeginPaint
        | B::DomSetChecked
        | B::DomApplyPatches
        | B::DomApplyWords
        | B::DomPreventDefault
        | B::DomEventKey
        | B::StorageSet
        | B::StorageGet
        | B::StorageGetOr
        | B::StorageRemove
        | B::SignalExitComponent => Home::Internal(
            "the browser/DOM tier (LOGOS→WASM driving the DOM); documented by the \
             dom.lg stdlib module's literate notes, outside the core guide surface",
        ),

        // ── Numbers read at run time ────────────────────────────────────
        B::ReadReals => Home::Internal(
            "the numbers a program reads at run time rather than carries as code (a model's weights, \
             written by `largo bringup --run`); documented in the imperative-mode guide beside `Read … from \
             file`, not a core guide section",
        ),

        // ── Wire seal/compress crypto substrate ─────────────────────────
        B::CompressBytes | B::DecompressBytes | B::SealBytes | B::OpenBytes | B::FromWireBytes
        | B::SealFrame | B::OpenFrame | B::SealBox | B::OpenBox | B::SealValueBox
        | B::OpenValueBox => Home::Internal(
            "wire seal/compress substrate: authenticated encryption (ChaCha20-Poly1305 under the \
             caller's nonce, XChaCha20-Poly1305 boxes under a drawn one, never compressed first) + \
             wire (de)serialization for the relay transport — engine plumbing documented by the \
             imperative-mode guide's crypto paragraphs, not a core guide section",
        ),

        // ── Forward error correction (Reed-Solomon) ─────────────────────
        B::FecProtect | B::FecRestore | B::ReedSolomonEncode | B::ReedSolomonRecover => {
            Home::Internal(
                "forward-error-correction substrate (Reed-Solomon protect/recover) for the wire/relay \
                 transport — engine plumbing, outside the core guide surface",
            )
        }

        // ── Browser WebRTC peer transport tier ──────────────────────────
        B::RtcPeerConnection | B::RtcDataChannel | B::RtcCreateOffer | B::RtcCreateAnswer
        | B::RtcSetLocal | B::RtcSetRemote | B::RtcAddIce | B::RtcSend => Home::Internal(
            "the browser WebRTC peer transport tier (LOGOS→WASM), like the DOM tier; documented by \
             the stdlib's transport literate notes, outside the core guide surface",
        ),
    }
}

/// Raise-only floor for `BuiltinId::ALL` — the census list never shrinks.
const BUILTIN_FLOOR: usize = 125;

/// The construct catalog and section class lists live in the shared module
/// (`tests/common/guide_catalog.rs`) so this battery and `guide_pedagogy.rs`
/// can never disagree about what is taught where.
#[path = "web_common/guide_catalog.rs"]
mod guide_catalog;
use guide_catalog::{CHECKPOINT_SECTIONS, LANGUAGE_SURFACE, META_SECTIONS};

/// Raise-only floor for the construct catalog.
const SURFACE_FLOOR: usize = 60;

#[test]
fn every_cataloged_construct_is_taught() {
    let mut wrong: Vec<String> = Vec::new();

    for c in LANGUAGE_SURFACE {
        let Some(section) = SECTIONS.iter().find(|s| s.id == c.section) else {
            wrong.push(format!(
                "{:?} claims section {:?} which does not exist",
                c.feature, c.section
            ));
            continue;
        };
        let haystack: String = std::iter::once(section.content)
            .chain(section.examples.iter().map(|e| e.code))
            .collect::<Vec<_>>()
            .join("\n");
        if !c.probes.iter().any(|p| haystack.contains(p)) {
            wrong.push(format!(
                "{:?} — none of its probes {:?} appear in section {:?}",
                c.feature, c.probes, c.section
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "\nconstruct(s) no longer taught where the catalog says:\n{}\n",
        wrong.join("\n")
    );
}

/// Every section must be claimed by the catalog (or be an explicit meta
/// section, or a Checkpoint — checkpoints review, they never teach) — a new
/// section without catalog entries is teaching surface the locks don't cover.
#[test]
fn every_section_is_claimed_by_the_catalog() {
    let claimed: std::collections::HashSet<&str> =
        LANGUAGE_SURFACE.iter().map(|c| c.section).collect();
    let meta: std::collections::HashSet<&str> = META_SECTIONS.iter().copied().collect();
    let checkpoints: std::collections::HashSet<&str> =
        CHECKPOINT_SECTIONS.iter().copied().collect();

    let unclaimed: Vec<&str> = SECTIONS
        .iter()
        .map(|s| s.id)
        .filter(|id| !claimed.contains(id) && !meta.contains(id) && !checkpoints.contains(id))
        .collect();

    assert!(
        unclaimed.is_empty(),
        "\nsection(s) not claimed by LANGUAGE_SURFACE (add construct entries or, if truly \
         narrative-only, add to META_SECTIONS with justification): {unclaimed:?}\n"
    );
}

#[test]
fn surface_catalog_never_shrinks() {
    assert!(
        LANGUAGE_SURFACE.len() >= SURFACE_FLOOR,
        "LANGUAGE_SURFACE shrank ({} < {SURFACE_FLOOR}) — the catalog only ever grows",
        LANGUAGE_SURFACE.len()
    );
}

#[test]
fn every_builtin_has_a_guide_home() {
    assert!(
        BuiltinId::ALL.len() >= BUILTIN_FLOOR,
        "BuiltinId::ALL shrank ({} < {BUILTIN_FLOOR}) — variants are never removed silently",
        BuiltinId::ALL.len()
    );

    let live: std::collections::HashSet<&str> = SECTIONS.iter().map(|s| s.id).collect();
    let mut wrong: Vec<String> = Vec::new();

    for &b in BuiltinId::ALL {
        match guide_home(b) {
            Home::Section(id) => {
                if !live.contains(id) {
                    wrong.push(format!(
                        "{b:?} claims Section({id:?}) but no such section exists"
                    ));
                }
            }
            Home::Planned { section } => {
                if live.contains(section) {
                    wrong.push(format!(
                        "{b:?} is Planned for {section:?} which NOW EXISTS — flip it to \
                         Home::Section (the content wave landed; the census keeps up)"
                    ));
                }
            }
            Home::Internal(reason) => {
                if reason.trim().is_empty() {
                    wrong.push(format!("{b:?} is Internal with an empty reason"));
                }
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "\nbuiltin census violation(s):\n{}\n",
        wrong.join("\n")
    );
}

/// The runnability appendix is the canonical honesty table: an example id
/// appears in it (as a backticked code span) if and only if the example is
/// `CompiledOnly`. Promotions and demotions can never leave a stale row.
#[test]
fn runnability_appendix_matches_the_compiled_only_set() {
    let appendix = SECTIONS
        .iter()
        .find(|s| s.id == "appendix-runnability")
        .expect("the runnability appendix section must exist");

    let mut wrong: Vec<String> = Vec::new();
    for section in SECTIONS.iter() {
        for example in section.examples.iter() {
            let listed = appendix.content.contains(&format!("`{}`", example.id));
            let compiled = matches!(example.runnability, Runnability::CompiledOnly { .. });
            if compiled && !listed {
                wrong.push(format!(
                    "`{}` is CompiledOnly but missing from the appendix table",
                    example.id
                ));
            }
            if listed && !compiled {
                wrong.push(format!(
                    "`{}` appears in the appendix table but is not CompiledOnly — stale row",
                    example.id
                ));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "\nrunnability appendix out of sync:\n{}\n",
        wrong.join("\n")
    );
}

/// Section numbers must be 1..=N in display order — renumbering is display-only
/// and this keeps it impossible to fumble (a gap or duplicate after inserting a
/// section is caught immediately).
#[test]
fn section_numbers_are_contiguous_display_order() {
    let numbers: Vec<u8> = SECTIONS.iter().map(|s| s.number).collect();
    let expected: Vec<u8> = (1..=SECTIONS.len() as u8).collect();
    assert_eq!(
        numbers, expected,
        "section numbers must be exactly 1..=N in display order"
    );
}

/// Extract the N from every `§N` / `Section N` / `section N` in the text.
fn regex_lite_numbers(text: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    let mut push_number_at = |mut i: usize| {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i > start {
            if let Ok(n) = text[start..i].parse::<u32>() {
                found.push(n);
            }
        }
    };
    for (idx, _) in text.match_indices('§') {
        push_number_at(idx + '§'.len_utf8());
    }
    for (idx, m) in text.match_indices("Section ") {
        push_number_at(idx + m.len());
    }
    for (idx, m) in text.match_indices("section ") {
        push_number_at(idx + m.len());
    }
    found
}

/// Extract the roman numeral from every `Part <roman>` in the text.
fn regex_lite_parts(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (idx, m) in text.match_indices("Part ") {
        let rest = &text[idx + m.len()..];
        let roman: String = rest
            .chars()
            .take_while(|c| matches!(c, 'I' | 'V' | 'X'))
            .collect();
        if !roman.is_empty() {
            found.push(roman);
        }
    }
    found
}

/// Every `largo <subcommand>` the guide's prose mentions must be a REAL subcommand: one of the
/// variants of `pub enum Commands` in `apps/logicaffeine_cli/src/cli.rs`, read from that file, so a
/// command added to the CLI is known here the moment it exists and a command removed from it fails
/// every section that still teaches it.
fn real_largo_commands() -> Vec<String> {
    let cli = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/logicaffeine_cli/src/cli.rs"),
    )
    .expect("the CLI's command table is readable");
    let start = cli
        .find("pub enum Commands {")
        .expect("cli.rs declares `pub enum Commands`");
    let body = &cli[start..];
    let end = body.find("\n}").expect("`pub enum Commands` closes");
    body[..end]
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("    ")?;
            let first = rest.chars().next()?;
            if !first.is_ascii_uppercase() {
                return None;
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            let after = rest[name.len()..].trim_start();
            (after.starts_with('{') || after.starts_with(',') || after.starts_with('('))
                .then(|| name.to_lowercase())
        })
        .collect()
}

#[test]
fn largo_commands_are_real() {
    let commands = real_largo_commands();
    assert!(
        ["new", "build", "run", "serve", "test", "check"].iter().all(|c| commands.iter().any(|k| k == c)),
        "the Commands census lost a core subcommand — the cli.rs reader no longer matches: {commands:?}"
    );
    let real: std::collections::HashSet<&str> = commands.iter().map(String::as_str).collect();
    let mut fake: Vec<String> = Vec::new();
    for section in SECTIONS.iter() {
        for (idx, m) in section.content.match_indices("largo ") {
            let rest = &section.content[idx + m.len()..];
            let word: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if !word.is_empty()
                && word.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && !real.contains(word.as_str())
            {
                fake.push(format!("[{}] largo {word}", section.id));
            }
        }
    }
    assert!(
        fake.is_empty(),
        "
guide prose mentions largo subcommand(s) that do not exist:
{}
",
        fake.join(
            "
"
        )
    );
}

/// The operator appendix is COMPLETE by construction: every English operator
/// form the guide teaches anywhere must appear in the appendix table.
const ESSENTIAL_VOCAB: &[&str] = &[
    "plus",
    "minus",
    "times",
    "divided by",
    "modulo",
    "equals",
    "is not",
    "is less than",
    "is greater than",
    "is at most",
    "is at least",
    "is between",
    "is even",
    "is odd",
    "is divisible by",
    "is approximately",
    "xor",
    "shifted left by",
    "shifted right by",
    "followed by",
    "item N of",
    "copy of",
    "length of",
    "contains",
    "union",
    "intersection",
];

#[test]
fn operator_appendix_is_complete() {
    let appendix = SECTIONS
        .iter()
        .find(|s| s.id == "appendix-keywords")
        .expect("the operator appendix must exist");
    let missing: Vec<&&str> = ESSENTIAL_VOCAB
        .iter()
        .filter(|v| !appendix.content.contains(**v))
        .collect();
    assert!(
        missing.is_empty(),
        "
operator appendix is missing vocabulary the guide teaches: {missing:?}
"
    );
}

// ─── The dependency-order lock (pedagogy charter P10) ───────────────────────
//
// No example or exercise may USE a construct before the section that teaches
// it (display order) — Vygotsky's "no forward knowledge" made mechanical.
// Detectors are opt-in distinctive surfaces (see the catalog); string
// literals are stripped before scanning and matches are boundary-checked, so
// prose inside `Show "..."` never trips the lock.

/// Sections whose pedagogical MODE is forward-preview — montage surfaces
/// whose whole job is "here is the flavor before the mechanics". Exempt
/// wholesale; the reason is the editorial decision on record, and each must
/// carry a prose signpost saying the preview is a preview.
const PREVIEW_SECTIONS: &[(&str, &str)] = &[
    (
        "reading-english",
        "the mental-model section deliberately previews indexing, comparisons and If — \
         its job is the flavor of the language before the mechanics",
    ),
    (
        "getting-started",
        "the hello-world montage shows a real program (struct + function) before either \
         is taught — signposted in prose",
    ),
    (
        "tutorial-project",
        "the staged build-one-program tutorial teaches at point of use (the Rust-Book-ch.2 \
         precedent); its five stages introduce-before-use INTERNALLY, audited not scanned",
    ),
];

/// One-off sanctioned forward references inside teaching sections — the ZPD
/// "taste of the next rung". (item_id, feature, reason). Shrink-biased: a
/// stale entry that no longer trips the checker FAILS the suite, and if this
/// list ever grows past ~15 the content should be reordered instead.
const FORWARD_SANCTIONS: &[(&str, &str, &str)] = &[];

/// Blank out string-literal contents (keeping length) so detectors never
/// match prose inside `Show "..."`. Handles the guide's double-quoted
/// literals; escaped quotes inside LOGOS strings don't occur in guide code.
fn strip_string_literals(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_str = false;
    for c in code.chars() {
        if c == '"' {
            in_str = !in_str;
            out.push('"');
        } else if in_str && c != '\n' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Boundary-checked substring: the match may not extend an alphanumeric run
/// on either side (kills `Set `-inside-`SharedSet ` classes).
fn code_uses(stripped: &str, detector: &str) -> bool {
    let bytes = stripped.as_bytes();
    let mut from = 0;
    while let Some(pos) = stripped[from..].find(detector) {
        let start = from + pos;
        let end = start + detector.len();
        let before_ok = start == 0
            || !bytes[start - 1].is_ascii_alphanumeric()
            || !detector
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric());
        let after_ok = end >= bytes.len()
            || !bytes[end].is_ascii_alphanumeric()
            || !detector
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

fn ordering_violations(scan_exercises: bool) -> Vec<(String, String)> {
    let position: std::collections::HashMap<&str, usize> = SECTIONS
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, i))
        .collect();
    let previews: std::collections::HashSet<&str> =
        PREVIEW_SECTIONS.iter().map(|(id, _)| *id).collect();

    let mut violations: Vec<(String, String)> = Vec::new();
    for (si, section) in SECTIONS.iter().enumerate() {
        if previews.contains(section.id) {
            continue;
        }
        let mut scan = |item_id: &str, code: &str| {
            let stripped = strip_string_literals(code);
            for c in LANGUAGE_SURFACE.iter().filter(|c| !c.detectors.is_empty()) {
                let Some(&taught) = position.get(c.section) else {
                    continue;
                };
                if taught <= si {
                    continue;
                }
                if c.detectors.iter().any(|d| code_uses(&stripped, d)) {
                    violations.push((item_id.to_string(), c.feature.to_string()));
                }
            }
        };
        for e in section.examples.iter() {
            if scan_exercises {
                continue;
            }
            scan(e.id, e.code);
        }
        if scan_exercises {
            for x in section.exercises.iter() {
                scan(x.id, x.starter);
                scan(x.id, x.reference);
            }
        }
    }
    violations
}

fn assert_ordering(scan_exercises: bool, what: &str) {
    let sanctioned: std::collections::HashSet<(&str, &str)> = FORWARD_SANCTIONS
        .iter()
        .map(|(id, feature, _)| (*id, *feature))
        .collect();
    let violations = ordering_violations(scan_exercises);

    let unsanctioned: Vec<String> = violations
        .iter()
        .filter(|(id, f)| !sanctioned.contains(&(id.as_str(), f.as_str())))
        .map(|(id, f)| format!("`{id}` uses {f:?} before its teaching section"))
        .collect();
    assert!(
        unsanctioned.is_empty(),
        "\n{what} forward-reference violation(s) — reorder, rewrite, or sanction WITH a \
         prose signpost:\n{}\n",
        unsanctioned.join("\n")
    );

    // Stale sanctions fail loudly — an excuse that excuses nothing is a
    // promotion signal, same as the CompiledOnly partition.
    let live: std::collections::HashSet<(&str, &str)> = violations
        .iter()
        .map(|(id, f)| (id.as_str(), f.as_str()))
        .collect();
    let stale: Vec<String> = FORWARD_SANCTIONS
        .iter()
        .filter(|(id, feature, _)| !live.contains(&(*id, *feature)))
        .map(|(id, feature, _)| format!("({id}, {feature})"))
        .collect();
    assert!(
        stale.is_empty(),
        "\nstale FORWARD_SANCTIONS — the checker no longer trips on these; remove them:\n{}\n",
        stale.join("\n")
    );
}

#[test]
fn no_example_uses_a_construct_before_its_section() {
    // Preview sections must exist and carry their signpost obligation.
    for (id, _) in PREVIEW_SECTIONS {
        assert!(
            SECTIONS.iter().any(|s| s.id == *id),
            "PREVIEW_SECTIONS names `{id}` which does not exist"
        );
    }
    assert_ordering(false, "example");
}

#[test]
fn no_exercise_uses_a_construct_before_its_section() {
    assert_ordering(true, "exercise");
}

/// Detector liveness: a detector that matches nothing at-or-after its home is
/// a typo silently providing zero coverage.
#[test]
fn every_detector_detects_something() {
    let position: std::collections::HashMap<&str, usize> = SECTIONS
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, i))
        .collect();
    let mut dead: Vec<String> = Vec::new();
    for c in LANGUAGE_SURFACE.iter().filter(|c| !c.detectors.is_empty()) {
        let Some(&taught) = position.get(c.section) else {
            dead.push(format!(
                "{:?} claims unknown section {:?}",
                c.feature, c.section
            ));
            continue;
        };
        for d in c.detectors {
            let hit = SECTIONS.iter().enumerate().any(|(si, s)| {
                si >= taught
                    && (s
                        .examples
                        .iter()
                        .any(|e| code_uses(&strip_string_literals(e.code), d))
                        || s.exercises.iter().any(|x| {
                            code_uses(&strip_string_literals(x.starter), d)
                                || code_uses(&strip_string_literals(x.reference), d)
                        }))
            });
            if !hit {
                dead.push(format!("{:?} detector {d:?} matches nothing", c.feature));
            }
        }
    }
    assert!(
        dead.is_empty(),
        "\ndead detector(s):\n{}\n",
        dead.join("\n")
    );
}
