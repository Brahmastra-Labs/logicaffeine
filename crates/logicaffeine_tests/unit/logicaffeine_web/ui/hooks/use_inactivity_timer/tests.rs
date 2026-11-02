use super::*;

#[test]
fn test_inactivity_state_default() {
    let state = InactivityState::default();
    assert!(!state.is_inactive);
    assert_eq!(state.inactive_duration_ms, 0);
}

#[test]
fn test_default_threshold() {
    assert_eq!(DEFAULT_INACTIVITY_THRESHOLD_MS, 5000);
}
