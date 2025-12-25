use logos::compile;

#[test]
fn noun_to_verb_tabled() {
    let output = compile("The committee tabled the discussion.").unwrap();
    eprintln!("DEBUG tabled: {}", output);
    assert!(
        output.contains("Table("),
        "Should coerce 'tabled' to verb with correct lemma: got '{}'",
        output
    );
}

#[test]
fn noun_to_verb_emailed() {
    let output = compile("She emailed him.").unwrap();
    eprintln!("DEBUG emailed: {}", output);
    assert!(
        output.contains("Email"),
        "Should coerce 'emailed' to verb: got '{}'",
        output
    );
}

#[test]
fn noun_to_verb_google() {
    let output = compile("John googled the answer.").unwrap();
    eprintln!("DEBUG googled: {}", output);
    assert!(
        output.contains("Google("),
        "Should coerce 'googled' to verb with correct lemma: got '{}'",
        output
    );
}

#[test]
fn noun_to_verb_with_modal() {
    let output = compile("You should table the motion.").unwrap();
    eprintln!("DEBUG modal+noun: {}", output);
    assert!(
        output.contains("Table") || output.contains("table"),
        "Should coerce noun after modal: got '{}'",
        output
    );
}
