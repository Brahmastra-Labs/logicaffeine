use super::*;

#[test]
fn the_clock_reads_the_instant_the_environment_pins_and_the_system_clock_otherwise() {
    let pinned_nanos: i64 = (20_000 * 86_400 + 45_296) * 1_000_000_000;
    std::env::set_var("LOGOS_FIXED_DAYS", "20000");
    std::env::set_var("LOGOS_FIXED_NANOS", pinned_nanos.to_string());
    assert_eq!(today(), LogosDate(20_000));
    assert_eq!(now(), LogosMoment(pinned_nanos));
    assert_eq!(now().to_string(), "2024-10-04 12:34");

    std::env::set_var("LOGOS_FIXED_DAYS", " 19753 ");
    assert_eq!(today(), LogosDate(19_753), "the pinned day is read as the number it spells");
    std::env::set_var("LOGOS_FIXED_DAYS", "not a day");
    std::env::set_var("LOGOS_FIXED_NANOS", "");
    let system_days = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() / 86_400) as i32;
    assert!((today().0 - system_days).abs() <= 1, "a variable that spells no number pins nothing");

    std::env::remove_var("LOGOS_FIXED_DAYS");
    std::env::remove_var("LOGOS_FIXED_NANOS");
    let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as i64;
    let read = now().0;
    let after = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as i64;
    assert!(before <= read && read <= after, "unpinned, `now` is the system clock to the nanosecond");
}
