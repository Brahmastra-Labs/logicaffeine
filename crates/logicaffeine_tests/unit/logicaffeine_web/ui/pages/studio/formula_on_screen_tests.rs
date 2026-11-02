use super::{formula_on_screen, CompileResult, OutputFormat};

fn result(kripke: Option<&str>) -> CompileResult {
    CompileResult {
        logic: Some("full".into()),
        simple_logic: Some("simple".into()),
        kripke_logic: kripke.map(str::to_string),
        ast: None,
        readings: Vec::new(),
        simple_readings: Vec::new(),
        kripke_readings: Vec::new(),
        tokens: Vec::new(),
        error: None,
    }
}

/// The formula the output shows in `format`, and whether it is read as the full formula.
fn shown(result: &CompileResult, format: OutputFormat) -> Option<(String, bool)> {
    formula_on_screen(result, format).map(|(logic, view)| (logic, view == OutputFormat::Unicode))
}

/// Each view's formula, read with the names the view that printed it uses.
#[test]
fn each_view_shows_its_own_formula() {
    let r = result(Some("deep"));
    assert!(formula_on_screen(&r, OutputFormat::SimpleFOL).is_some_and(|(l, v)| l == "simple" && v == OutputFormat::SimpleFOL));
    assert!(formula_on_screen(&r, OutputFormat::Kripke).is_some_and(|(l, v)| l == "deep" && v == OutputFormat::Kripke));
    assert_eq!(shown(&r, OutputFormat::Unicode), Some(("full".to_string(), true)));
}

/// LaTeX typesets the full formula, so the dictionary reads the full formula.
#[test]
fn latex_reads_the_full_formula() {
    assert_eq!(shown(&result(None), OutputFormat::LaTeX), Some(("full".to_string(), true)));
}

/// Deep with no Kripke reading shows the full formula, and the dictionary follows it.
#[test]
fn deep_without_a_kripke_reading_falls_back_to_the_full_formula() {
    assert_eq!(shown(&result(None), OutputFormat::Kripke), Some(("full".to_string(), true)));
}

#[test]
fn no_formula_shows_nothing() {
    let mut r = result(None);
    r.logic = None;
    r.simple_logic = None;
    for format in [OutputFormat::SimpleFOL, OutputFormat::Unicode, OutputFormat::LaTeX, OutputFormat::Kripke] {
        assert!(formula_on_screen(&r, format).is_none());
    }
}
