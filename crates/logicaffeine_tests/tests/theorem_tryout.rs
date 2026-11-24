//! A scratch door for trying theorem text against the prover and the kernel without a rebuild:
//! `LOGOS_TRYOUT=<dir> cargo test --test theorem_tryout -- --ignored --nocapture` compiles every
//! `*.logic` file in the directory as a theorem and prints whether the kernel certified it.
use logicaffeine_compile::compile_theorem_for_ui;

#[test]
#[ignore]
fn try_every_theorem_in_the_directory() {
    let dir = std::env::var("LOGOS_TRYOUT").expect("LOGOS_TRYOUT names a directory of .logic files");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "logic")).collect();
    paths.sort();
    for path in paths {
        let src = std::fs::read_to_string(&path).unwrap();
        let result = compile_theorem_for_ui(&src);
        let verdict = if result.verified { "CERTIFIED".to_string() } else if result.derivation.is_some() { format!("UNCERTIFIED: {}", result.verification_error.clone().unwrap_or_default()) } else { format!("NO PROOF: {}", result.verification_error.clone().or(result.error.clone()).unwrap_or_default()) };
        println!("TRYOUT {} => {}", path.file_name().unwrap().to_string_lossy(), verdict.replace('\n', " "));
    }
}
