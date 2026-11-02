use super::*;

#[test]
fn test_studio_mode_hardware() {
    // Hardware is a first-class Studio mode alongside Logic/Code/Math.
    assert_eq!(StudioMode::Hardware.extension(), "hw");
    assert_eq!(StudioMode::Hardware.display_name(), "Hardware");
    assert_eq!(StudioMode::from_extension("hw"), Some(StudioMode::Hardware));
    assert_eq!(StudioMode::from_extension("HW"), Some(StudioMode::Hardware));
    // The existing modes keep resolving unchanged.
    assert_eq!(StudioMode::from_extension("logic"), Some(StudioMode::Logic));
    assert_eq!(StudioMode::from_extension("logos"), Some(StudioMode::Code));
    assert_eq!(StudioMode::from_extension("math"), Some(StudioMode::Math));
    assert_eq!(StudioMode::from_extension("txt"), None);
}

#[test]
fn test_studio_mode_web() {
    // Web is a first-class Studio mode — a live in-browser reactive-app preview.
    assert_eq!(StudioMode::Web.extension(), "web");
    assert_eq!(StudioMode::Web.display_name(), "Web");
    assert_eq!(StudioMode::from_extension("web"), Some(StudioMode::Web));
    assert_eq!(StudioMode::from_extension("WEB"), Some(StudioMode::Web));
}

#[test]
fn studio_mode_all_lists_every_variant_once() {
    // Adding a variant breaks this match until it is listed in `ALL` too.
    for mode in StudioMode::ALL {
        match mode {
            StudioMode::Logic | StudioMode::Code | StudioMode::Math | StudioMode::Hardware | StudioMode::Web => {}
        }
    }
    let mut seen = std::collections::HashSet::new();
    for mode in StudioMode::ALL {
        assert!(seen.insert(format!("{mode:?}")), "{mode:?} is listed twice in StudioMode::ALL");
    }
    assert_eq!(seen.len(), 5);
}

#[test]
fn for_path_routes_each_extension() {
    assert_eq!(StudioMode::for_path("/examples/logic/prover-demo.logic"), Some(StudioMode::Logic));
    assert_eq!(StudioMode::for_path("/examples/code/algorithms/fibonacci.logos"), Some(StudioMode::Code));
    assert_eq!(StudioMode::for_path("/workspace/untitled.math"), Some(StudioMode::Math));
    assert_eq!(StudioMode::for_path("/workspace/proof.vernac"), Some(StudioMode::Math));
    assert_eq!(StudioMode::for_path("/workspace/untitled.hw"), Some(StudioMode::Hardware));
    assert_eq!(StudioMode::for_path("/examples/hardware/arbiter.v"), Some(StudioMode::Hardware));
    assert_eq!(StudioMode::for_path("/workspace/untitled.web"), Some(StudioMode::Web));
}

#[test]
fn for_path_ignores_extension_case() {
    assert_eq!(StudioMode::for_path("/workspace/NOTES.LOGIC"), Some(StudioMode::Logic));
    assert_eq!(StudioMode::for_path("/workspace/App.Logos"), Some(StudioMode::Code));
    assert_eq!(StudioMode::for_path("/workspace/Spec.HW"), Some(StudioMode::Hardware));
}

#[test]
fn for_path_lets_the_folder_decide_what_a_logos_file_is() {
    // `.logos` is the source language of three modes; the example tree says which.
    assert_eq!(StudioMode::for_path("/examples/math/natural-numbers.logos"), Some(StudioMode::Math));
    assert_eq!(StudioMode::for_path("/examples/code/web/counter.logos"), Some(StudioMode::Web));
    assert_eq!(StudioMode::for_path("/examples/code/basics/variables.logos"), Some(StudioMode::Code));
    // A folder only disambiguates `.logos`; every other extension already names its mode.
    assert_eq!(StudioMode::for_path("/examples/math/notes.logic"), Some(StudioMode::Logic));
    assert_eq!(StudioMode::for_path("/workspace/web/spec.hw"), Some(StudioMode::Hardware));
}

#[test]
fn for_path_matches_whole_folder_names_only() {
    assert_eq!(StudioMode::for_path("/workspace/mathematics/sum.logos"), Some(StudioMode::Code));
    assert_eq!(StudioMode::for_path("/workspace/webby/app.logos"), Some(StudioMode::Code));
    assert_eq!(StudioMode::for_path("/workspace/math.logos"), Some(StudioMode::Code));
}

#[test]
fn for_path_reads_the_extension_from_the_file_name_only() {
    assert_eq!(StudioMode::for_path("/workspace/v1.logic/readme"), None);
    assert_eq!(StudioMode::for_path("/workspace/archive.hw/notes.txt"), None);
    assert_eq!(StudioMode::for_path("logos"), None);
}

#[test]
fn for_path_has_no_mode_for_foreign_files() {
    assert_eq!(StudioMode::for_path("/workspace/notes.txt"), None);
    assert_eq!(StudioMode::for_path("/workspace/README"), None);
    assert_eq!(StudioMode::for_path("/workspace/.logos"), None);
    assert_eq!(StudioMode::for_path(""), None);
    assert_eq!(StudioMode::for_path("/"), None);
}

#[test]
fn a_new_file_reopens_in_the_mode_that_created_it() {
    // "New file" names the file `untitled.<mode extension>` under /workspace; with no mode
    // switch in the Studio, reopening it must land back in the same mode.
    for mode in StudioMode::ALL {
        let path = format!("/workspace/untitled.{}", mode.extension());
        assert_eq!(StudioMode::for_path(&path), Some(mode), "{path}");
        let numbered = format!("/workspace/untitled-3.{}", mode.extension());
        assert_eq!(StudioMode::for_path(&numbered), Some(mode), "{numbered}");
    }
}
