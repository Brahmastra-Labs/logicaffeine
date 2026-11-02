//! EXTRACTION NAMES A SHADOW BY THE NAME ITS AUTHOR WROTE.
//!
//! A document's `Definition add_zero …` is registered under a fresh identity (`add_zero✝⟨1⟩`,
//! ruling of 2026-09-24), which no Rust identifier can spell. Extraction emits it as the name the
//! author wrote while that name denotes it — so an imperative `Require that check_add_zero().`
//! calls the check it means — as `<name>_shadowed<n>` once a later declaration has taken the name,
//! and a declaration a live shadow hides as `<key>_root`. No kernel identity reaches the Rust.

use logicaffeine_compile::extract_math_rust_from_source;
use logicaffeine_compile::extraction::{emit_property_check, extract_programs, rust_global};
use logicaffeine_kernel::interface::Repl;

const NUM: &str = "Inductive Num := Z : Num | S : Num -> Num.
Definition add : Num -> Num -> Num := fix rec => fun n : Num => fun m : Num => match n return (fun _ : Num => Num) with | Z => m | S k => S (rec k m).
Definition one : Num := S Z.
Definition two : Num := add one one.
Definition add_zero : Eq Num (add Z one) one := refl Num one.";

fn session(document: &str) -> Repl {
    let mut repl = Repl::new();
    repl.execute_document(document)
        .unwrap_or_else(|e| panic!("the document must be accepted: {e}\n{document}"));
    repl
}

fn shadow_key(repl: &Repl, written: &str) -> String {
    repl.shadows()
        .into_iter()
        .find(|(w, _)| w == written)
        .map(|(_, key)| key)
        .unwrap_or_else(|| panic!("`{written}` is not shadowed: {:?}", repl.shadows()))
}

fn assert_no_identity(rust: &str) {
    assert!(
        !rust.contains('\u{271d}') && !rust.contains('\u{27e8}') && !rust.contains('\u{27e9}'),
        "a kernel identity reached the Rust:\n{rust}"
    );
}

#[test]
fn a_live_shadow_extracts_under_its_written_name() {
    let repl = session(NUM);
    let ctx = repl.context();
    let add = shadow_key(&repl, "add");
    let add_zero = shadow_key(&repl, "add_zero");
    assert_eq!(rust_global(ctx, &add), "add");
    assert_eq!(rust_global(ctx, &add_zero), "add_zero");

    let rust = extract_programs(ctx, &[add.as_str(), "two"]).expect("extraction");
    assert_no_identity(&rust);
    assert!(rust.contains("pub fn add(n: Num, m: Num) -> Num"), "{rust}");
    assert!(
        rust.contains("add(one(), one())"),
        "`two` calls the shadow by its written name:\n{rust}"
    );

    let statement = ctx
        .get_global(&add_zero)
        .expect("the shadowing theorem")
        .clone();
    let check = emit_property_check(ctx, &add_zero, &statement).expect("a runnable property");
    assert_no_identity(&check);
    assert!(check.contains("pub fn check_add_zero() -> bool"), "{check}");
}

#[test]
fn a_shadow_shadowed_again_and_the_declaration_a_shadow_hides_keep_distinct_rust_names() {
    let mut repl = session(NUM);
    let first = shadow_key(&repl, "add");
    repl.execute("Definition add : Num -> Num -> Num := fun n : Num => fun m : Num => m.")
        .expect("a second `add`");
    let second = shadow_key(&repl, "add");
    let ctx = repl.context();
    assert_eq!(rust_global(ctx, &second), "add");
    assert_eq!(rust_global(ctx, &first), "add_shadowed1");
    let rust = extract_programs(ctx, &[second.as_str(), "two"]).expect("extraction");
    assert_no_identity(&rust);
    assert!(
        rust.contains("pub fn add(") && rust.contains("pub fn add_shadowed1("),
        "{rust}"
    );
    assert!(
        rust.contains("add_shadowed1(one(), one())"),
        "`two` keeps the first `add`:\n{rust}"
    );

    let hiding = session(
        "Definition Nat_add : Nat -> Nat -> Nat := fun a : Nat => fun b : Nat => a.
         Definition via_root : Nat := _root_.Nat_add (Succ Zero) Zero.",
    );
    let shadow = shadow_key(&hiding, "Nat_add");
    let ctx = hiding.context();
    assert_eq!(rust_global(ctx, &shadow), "Nat_add");
    assert_eq!(rust_global(ctx, "Nat_add"), "Nat_add_root");
    let rust = extract_programs(ctx, &[shadow.as_str(), "via_root"]).expect("extraction");
    assert_no_identity(&rust);
    assert!(
        rust.contains("pub fn Nat_add(") && rust.contains("pub fn Nat_add_root("),
        "{rust}"
    );
    assert!(
        rust.contains("Nat_add_root("),
        "`via_root` calls the environment's `Nat_add`:\n{rust}"
    );
}

/// The standalone module a Math document compiles to, self-verifying `main` included, names every
/// shadow as written — and it BUILDS and RUNS, which is the only check that the names agree.
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles and runs the extracted module with rustc (slow); run in the full suite"]
#[test]
fn e2e_the_extracted_module_of_a_shadowing_document_builds_and_runs() {
    let rust = extract_math_rust_from_source(NUM);
    assert_no_identity(&rust);
    let dir = std::env::temp_dir().join(format!("logos_shadow_extraction_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let source = dir.join("main.rs");
    std::fs::write(&source, &rust).expect("the module");
    let binary = dir.join("extracted");
    let built = std::process::Command::new("rustc")
        .args(["--edition", "2021", "-O", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .expect("rustc");
    assert!(
        built.status.success(),
        "{}\n{rust}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = std::process::Command::new(&binary)
        .output()
        .expect("the extracted binary");
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let stdout = String::from_utf8_lossy(&ran.stdout);
    assert!(
        stdout.contains("add_zero holds"),
        "the property check ran under its written name:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
