//! ★ LOCK — a server that DECODES JSON into its own record actually builds. ★
//!
//! `largo check` passed and `largo build --release` FAILED with a rustc error inside generated
//! code — *"the generated Rust failed to compile — this is a LOGOS compiler bug"*:
//!
//! ```text
//! Memo { id: __lgx_f_id.clone(), … }
//!            ^^^^^^^^^^^^^^^^^^ expected `i64`, found `LogosInt`
//! ```
//!
//! ★ WHY IT IS INVISIBLE EVERYWHERE ELSE. `derived_json_lock` and `from_json_lock` prove the
//! decoder on the tree-walker and the VM — 44 tests between them — and neither calls
//! `compile_to_rust`. The tiers a test drives infer loosely; the tier a server SHIPS as runs
//! rustc. Green suite, broken deploy, in the shape this campaign keeps finding.
//!
//! ★ AND `compile_to_rust` RETURNING `Ok` DOES NOT CATCH IT. There is no `compile_error!` here —
//! the emitted Rust looks perfectly well-formed and rustc rejects it on a type. So the only
//! honest check is to BUILD, which is why this is an `#[ignore]`d heavy beside the other deploy
//! tests rather than a cheap scan.
//!
//! ★ THE CAUSE, AND THE FIX. It was never about JSON. `assets/std/json.lg` is a full RFC 8259
//! parser and canonical serializer in pure LOGOS, conformance-locked, and the derived decoders call
//! straight into it. The defect was one level down: **LOGOS has ONE `Int` (arbitrary precision) and
//! this tier has TWO machine forms of it (`i64` and `LogosInt`), and which one a SLOT got was
//! decided by name-keyed syntactic predictors that met on the same value and disagreed.** JSON was
//! simply where they met most often, because json.lg parses EXACT integers so a `JNum` payload is
//! promoted while a user's `id` field is not.
//!
//! Two structural bugs, both in `codegen/bigint_promote.rs` and its wiring:
//!
//!  1. **The mutual recursion ran exactly TWICE and stopped.** A promoted FIELD promotes the
//!     functions that read it, which promote the fields THEY build, which promote the arguments
//!     handed to them. Two iterations is correct for the shapes someone tried and the next shape is
//!     a new bug wearing the old one's clothes — which is exactly what the three attempts below
//!     kept hitting. It is a FIXPOINT now (`promotion_facts`); every set is monotone in its input,
//!     so it terminates, and the answer no longer depends on how many times it was run.
//!  2. **The facts were installed twice from different seeds**, under a comment asserting the two
//!     were identical. The struct-definition emitter read a NARROW set; everything else recomputed
//!     a strictly wider one that was never installed. A field could be typed `i64` while the value
//!     filling it was typed `LogosInt`, by construction rather than by accident. Computed once now,
//!     at codegen entry, before anything is emitted.
//!
//! And three missing flow edges: a local assigned from a promoted FIELD (`Set top to m's id`); a
//! field initialised with a compile-time bignum CONSTANT (`a new Cell with v (2 ** 70)`); and the
//! EMITTER failing to see a promoted field read as a bignum operand (`c's v * 2` came out as a
//! plain `*` between a `LogosInt` and an integer literal). The emitter now asks the SAME predicate
//! the analysis asks, because two spellings of "is this field exact" is how they came to disagree.
//!
//! ★ THE MOVE THAT MADE IT TRACTABLE: **widen at the boundary rather than require agreement.** A
//! field stored as `LogosInt` accepts an `i64` and widens at the site (`LogosInt::from` — identity
//! when it is already one). Requiring both sides to have been PREDICTED alike is what made the area
//! fragile. Nothing NARROWS anywhere, which is what kills attempt 3 below permanently.
//!
//! ★ THE THREE ATTEMPTS THAT FAILED FIRST, kept because the constraints they establish are what the
//! real fix had to satisfy:
//!
//!  1. **Propagate promotion out of the enum payload** the decoder unwraps. It works, and erases
//!     `Result<T, E>`'s type parameter — `Ok { value }` is ONE NAME FOR MANY TYPES. 19 rustc
//!     errors, strictly worse than the bug. (A SLOT-keyed fixpoint separates them.)
//!  2. **Promote every `Int` field a derived decoder writes.** The field becomes `LogosInt`
//!     correctly — and its WRITERS break: `nextId(…) -> Int` no longer fits the field it fills.
//!     (A fixpoint promotes the writers in the same pass; that case is now a test.)
//!  3. **Narrow at the construction site.** Unsound: `LogosInt::to_i64` returns an `Option`, so a
//!     value that genuinely exceeded `i64` would be silently replaced — a wrong answer where there
//!     is currently a loud build failure.
//!
//! The whole-program half of this lives in `crates/logicaffeine_tests/tests/int_representation_lock.rs`,
//! which builds eight Int-flow shapes with rustc and compares each against the tree-walker.
//!
//! Found by writing an ordinary notes app whose server reads its store from a JSON file.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

/// A server that decodes its own record — the shape of every store-backed backend.
const BACKEND: &str = "# Store\n\n\
    ## A Memo has:\n\
    \x20   an id, which is Int.\n\
    \x20   a title, which is Text.\n\n\
    ## To load () -> Seq of Memo:\n\
    \x20   Inspect fromJsonSeq_Memo(readOrEmpty()):\n\
    \x20       When Ok (list):\n\
    \x20           Return list.\n\
    \x20       When Err (e):\n\
    \x20           Return a new Seq of Memo.\n\n\
    ## To readOrEmpty () -> Text:\n\
    \x20   Return \"[]\".\n\n\
    ## Route GET /api/memos -> Seq of Memo:\n\
    \x20   Return load().\n\n\
    ## Main\n\n\
    Serve on port 8080.\n";

#[test]
#[ignore = "builds a real release binary — run with --ignored beside the other deploy heavies"]
fn a_server_that_decodes_json_builds_as_a_binary() {
    let dir = tempfile::tempdir().unwrap();
    let out = largo_in(dir.path(), &["new", "decoder", "--web"]);
    assert!(out.status.success(), "largo new --web: {}", stderr(&out));
    let root = dir.path().join("decoder");
    std::fs::write(root.join("src/main.lg"), BACKEND).unwrap();

    // It must pass the cheap check…
    let checked = largo_in(&root, &["check"]);
    assert!(
        checked.status.success(),
        "largo check:\n{}",
        stderr(&checked)
    );

    // …and it must actually BUILD, which is the part that fails.
    let built = largo_in(&root, &["build", "--release"]);
    assert!(
        built.status.success(),
        "a backend that decodes its own record passes `largo check` and does NOT build. A record \
         field typed `Int` is emitted as `i64` while the decoder hands it the `LogosInt` that came \
         out of `Json`'s `JNum`, so rustc rejects generated code. Every store-backed LOGOS server \
         has this shape.\n{}",
        stderr(&built)
    );
}
