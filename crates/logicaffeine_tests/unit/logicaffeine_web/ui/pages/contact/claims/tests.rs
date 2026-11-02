use super::*;

#[test]
fn the_curriculum_is_counted() {
    let figures = learn_figures();
    let engine = ContentEngine::shared();
    assert_eq!(figures.modules, engine.eras().iter().map(|e| e.modules.len()).sum::<usize>());
    assert!(figures.modules > 0 && figures.exercises > figures.modules, "{figures:?}");
}
