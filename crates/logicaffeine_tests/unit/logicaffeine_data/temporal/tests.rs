use super::*;

#[test]
fn date_to_ymd_epoch() {
    let date = LogosDate(0);
    let (y, m, d) = date.to_ymd();
    assert_eq!((y, m, d), (1970, 1, 1));
}

#[test]
fn date_display() {
    let date = LogosDate(20593); // 2026-05-20
    assert_eq!(date.to_string(), "2026-05-20");
}
