use super::*;
use crate::model::program::{Cut, Identity};

fn decode_step() -> Identity {
    Identity { cut: Cut::Decode { ctx: 4096 }, chips: 1, sha256: [7; 32] }
}

fn manifest(program: Option<&str>) -> String {
    match program {
        Some(line) => serde_json::json!({ "program": line, "layers": [] }).to_string(),
        None => serde_json::json!({ "layers": [] }).to_string(),
    }
}

#[test]
fn a_manifest_naming_the_decode_step_is_the_bundle_of_it() {
    let whole = decode_step();
    assert_eq!(manifest_names(&manifest(Some(&whole.line())), &whole), Ok(()));
}

#[test]
fn a_manifest_naming_another_cut_is_refused_naming_both() {
    let whole = decode_step();
    let other = Identity { cut: Cut::Decode { ctx: 2048 }, ..whole };
    let e = manifest_names(&manifest(Some(&other.line())), &whole).unwrap_err();
    assert!(e.0.contains(&other.line()) && e.0.contains(&whole.line()), "{e}");
}

#[test]
fn a_manifest_naming_another_text_of_the_same_cut_is_refused() {
    let whole = decode_step();
    let other = Identity { sha256: [8; 32], ..whole };
    assert!(manifest_names(&manifest(Some(&other.line())), &whole).is_err());
}

#[test]
fn a_manifest_without_a_program_line_or_not_json_is_refused() {
    let whole = decode_step();
    assert!(manifest_names(&manifest(None), &whole).is_err());
    assert!(manifest_names("not json", &whole).is_err());
}

#[test]
fn a_values_file_reads_back_to_the_same_bits() {
    let dir = std::env::temp_dir().join(format!("bringup_values_file_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("v.f64");
    let values = [1.0 / 3.0, 0.1 + 0.2, -2.5, 3.0, -7.0, 1e-300 * 3.0, f64::MIN_POSITIVE, 5e-324, 6.02214076e23, -0.0078125, -0.0];
    write_reals(&path, &values).unwrap();
    let back = logicaffeine_system::file::read_reals(path.display().to_string()).unwrap();
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&back), bits(&values), "every value crosses the file with its bits");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_declaration_sizes_its_tensor() {
    assert_eq!(declared_len("64 by 96 tensor of Real."), Some(6144));
    assert_eq!(declared_len("1 by 5120 tensor of Real."), Some(5120));
    assert_eq!(declared_len("tensor of Real."), None);
    assert_eq!(declared_len("x by 4 tensor of Real."), None);
}

/// The decode step binds its token and its position each to a line of its own, and a run reads their names off the
/// step: `tok` and `pos` on the hybrid, every other line the same at any token and position.
#[test]
fn the_decode_step_names_its_token_and_its_position() {
    let spec = crate::codegen_cuda::qwen38::Qwen38Cfg::tiny().spec();
    let ((text, _, _), _) = written(&spec, Cut::Decode { ctx: 16 });
    assert_eq!(inputs(&spec, 16, &text), Ok(("tok".to_string(), "pos".to_string())));
    let other = text.replace("Let tok be 1.", "Let tok be 7.");
    assert!(inputs(&spec, 16, &other).is_err(), "a step whose token is bound elsewhere than at the placeholder is refused");
}

/// A generation's shown tokens read back a list a sequence, in order, each from 1 in the program and from 0 here; a
/// run that shows more or fewer tokens than its sequences generate, a line that is not a token, or token 0 is refused.
#[test]
fn a_generations_shown_tokens_read_back_a_list_a_sequence() {
    let seqs = [Sequence { prompt: vec![1], tokens: vec![0, 0] }, Sequence { prompt: vec![2, 3], tokens: vec![0] }];
    let shown = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    assert_eq!(read_generated(&shown(&["5", " 9 ", "1"]), &seqs), Ok(vec![vec![4, 8], vec![0]]));
    assert!(read_generated(&shown(&["5", "9"]), &seqs).is_err(), "a run that shows too few tokens");
    assert!(read_generated(&shown(&["5", "9", "1", "2"]), &seqs).is_err(), "a run that shows too many");
    assert!(read_generated(&shown(&["5", "x", "1"]), &seqs).is_err(), "a line that is not a token");
    assert!(read_generated(&shown(&["5", "0", "1"]), &seqs).is_err(), "token 0 is below the vocabulary");
}

#[test]
fn agreement_is_the_leading_run() {
    assert_eq!(agreeing(&[1, 2, 3], &[1, 2, 3]), 3);
    assert_eq!(agreeing(&[1, 9, 3], &[1, 2, 3]), 1, "the steps after a parting do not count");
    assert_eq!(agreeing(&[1, 2], &[1, 2, 3]), 2, "a shorter run agrees on what it generated");
    assert_eq!(agreeing(&[], &[1]), 0);
}

#[test]
fn a_cuts_manifest_builds_only_the_runtime_it_reads_with() {
    let staged = "[package]\nname = \"logos_output\"\n\n[dependencies]\nlogicaffeine-data = { path = \"./crates/logicaffeine_data\" }\n\
                  logicaffeine-system = { path = \"./crates/logicaffeine_system\", features = [\"full\"] }\n\
                  tokio = { version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }\n\n\
                  [target.'cfg(target_os = \"linux\")'.dependencies]\n\
                  logicaffeine-system = { path = \"./crates/logicaffeine_system\", features = [\"full\", \"io-uring\"] }\n\n\
                  [profile.release]\nlto = true\nopt-level = 3\ncodegen-units = 1\npanic = \"abort\"\n";
    let lean = lean_manifest(staged, "fn main() {}");
    assert!(!lean.contains("\"full\"") && !lean.contains("io-uring"), "no feature past file reading: {lean}");
    assert_eq!(lean.matches("features = [\"persistence\"]").count(), 2, "{lean}");
    assert!(!lean.contains("lto =") && !lean.contains("codegen-units =") && lean.contains("opt-level = 3"), "{lean}");
    assert!(!lean.contains("tokio"), "tokio only when the program names it: {lean}");
    assert!(lean_manifest(staged, "#[tokio::main] async fn main() {}").contains("tokio ="), "a program naming tokio keeps it");
}

/// The manifest `compile_to_dir` stages is one `lean_manifest` cuts down: should its spelling change, a cut would build
/// the whole runtime again, and this says so.
#[test]
fn the_staged_manifest_is_one_the_cut_leans() {
    let dir = std::env::temp_dir().join(format!("bringup_lean_manifest_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::compile::compile_to_dir("## Main\nShow 1.\n", &dir).expect("a program stages");
    let staged = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let lean = lean_manifest(&staged, "fn main() {}");
    assert!(staged.contains("\"full\"") && !lean.contains("\"full\""), "the staged manifest asks for the whole runtime and the lean one does not:\n{staged}");
    assert!(lean.contains("persistence"), "{lean}");
    let _ = std::fs::remove_dir_all(&dir);
}
