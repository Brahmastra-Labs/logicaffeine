use super::*;

#[test]
fn newer_semver_is_detected() {
    assert!(version_is_newer("0.10.1", "0.10.0"));
    assert!(version_is_newer("0.11.0", "0.10.9"));
    assert!(version_is_newer("1.0.0", "0.99.99"));
    assert!(!version_is_newer("0.10.0", "0.10.0"));
    assert!(!version_is_newer("0.9.9", "0.10.0"));
    assert!(!version_is_newer("garbage", "0.10.0"));
}

#[test]
fn check_lines_align_and_carry_symbols() {
    let ok = render_check(&Check {
        status: Status::Ok,
        name: "cargo",
        detail: "1.80".into(),
    });
    assert!(ok.starts_with("✓ cargo"), "{ok}");
    let fail = render_check(&Check {
        status: Status::Fail,
        name: "project",
        detail: "broken".into(),
    });
    assert!(fail.starts_with("✗ project"), "{fail}");
    let warn = render_check(&Check {
        status: Status::Warn,
        name: "node",
        detail: "x".into(),
    });
    // The detail column aligns in CHARACTER position across rows (the
    // symbols differ in UTF-8 byte width, so compare past the symbol).
    let rest_ok: String = ok.chars().skip(1).collect();
    let rest_warn: String = warn.chars().skip(1).collect();
    assert_eq!(
        rest_ok.find("1.80"),
        rest_warn.find('x'),
        "aligned detail column"
    );
}
