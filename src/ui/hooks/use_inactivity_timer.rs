//! Inactivity timer hook for detecting when users need help.
//!
//! This hook monitors user activity and triggers a callback when the user
//! has been inactive for a specified duration. Used to trigger Socratic hints
//! when students are struggling.

use dioxus::prelude::*;

/// Default inactivity threshold (5 seconds)
pub const DEFAULT_INACTIVITY_THRESHOLD_MS: u64 = 5000;

/// State returned by the inactivity timer hook
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InactivityState {
    /// Whether the user is currently inactive (threshold exceeded)
    pub is_inactive: bool,
    /// Duration of inactivity in milliseconds
    pub inactive_duration_ms: u64,
}

impl Default for InactivityState {
    fn default() -> Self {
        Self {
            is_inactive: false,
            inactive_duration_ms: 0,
        }
    }
}

/// Hook that tracks user inactivity and triggers when threshold is exceeded.
///
/// # Arguments
/// * `threshold_ms` - Milliseconds of inactivity before triggering
/// * `on_inactive` - Callback invoked when user becomes inactive
///
/// # Returns
/// A tuple of:
/// - `InactivityState` - Current inactivity state
/// - `reset_fn` - Function to call when user performs an action
///
/// # Example
/// ```ignore
/// let (inactivity, reset_activity) = use_inactivity_timer(5000, move || {
///     // Show hint to user
/// });
///
/// // In input handler:
/// oninput: move |_| reset_activity(),
/// ```
#[cfg(target_arch = "wasm32")]
pub fn use_inactivity_timer(
    threshold_ms: u64,
    on_inactive: impl Fn() + 'static,
) -> (Signal<InactivityState>, impl FnMut()) {
    use gloo_timers::callback::Interval;

    let mut state = use_signal(InactivityState::default);
    let mut last_activity_time = use_signal(|| js_sys::Date::now());
    let mut callback_triggered = use_signal(|| false);

    // Store the callback in a resource that lives for the component lifetime
    let on_inactive = std::rc::Rc::new(on_inactive);
    let on_inactive_clone = on_inactive.clone();

    // Set up interval to check inactivity
    use_effect(move || {
        let on_inactive = on_inactive_clone.clone();

        let interval = Interval::new(1000, move || {
            let now = js_sys::Date::now();
            let last = *last_activity_time.read();
            let elapsed_ms = (now - last) as u64;

            let is_inactive = elapsed_ms >= threshold_ms;

            // Update state
            state.set(InactivityState {
                is_inactive,
                inactive_duration_ms: elapsed_ms,
            });

            // Trigger callback once when becoming inactive
            if is_inactive && !*callback_triggered.read() {
                callback_triggered.set(true);
                on_inactive();
            }
        });

        // Keep interval alive by forgetting it (cleanup handled by component unmount)
        std::mem::forget(interval);
    });

    // Reset function to call when user is active
    let reset_activity = move || {
        #[cfg(target_arch = "wasm32")]
        {
            last_activity_time.set(js_sys::Date::now());
        }
        callback_triggered.set(false);
        state.set(InactivityState::default());
    };

    (state, reset_activity)
}

/// Non-WASM fallback that does nothing (for testing)
#[cfg(not(target_arch = "wasm32"))]
pub fn use_inactivity_timer(
    _threshold_ms: u64,
    _on_inactive: impl Fn() + 'static,
) -> (Signal<InactivityState>, impl FnMut()) {
    let state = use_signal(InactivityState::default);
    let reset_activity = || {};
    (state, reset_activity)
}

/// Simpler version that just returns whether user is inactive
#[cfg(target_arch = "wasm32")]
pub fn use_is_inactive(threshold_ms: u64) -> Signal<bool> {
    use gloo_timers::callback::Interval;

    let mut is_inactive = use_signal(|| false);
    let mut last_activity_time = use_signal(|| js_sys::Date::now());

    use_effect(move || {
        let interval = Interval::new(1000, move || {
            let now = js_sys::Date::now();
            let last = *last_activity_time.read();
            let elapsed_ms = (now - last) as u64;

            is_inactive.set(elapsed_ms >= threshold_ms);
        });

        // Keep interval alive by forgetting it (cleanup handled by component unmount)
        std::mem::forget(interval);
    });

    is_inactive
}

#[cfg(not(target_arch = "wasm32"))]
pub fn use_is_inactive(_threshold_ms: u64) -> Signal<bool> {
    use_signal(|| false)
}

/// Hook to get a function that resets the activity timer
/// Call this in oninput, onclick, onkeydown handlers
#[cfg(target_arch = "wasm32")]
pub fn use_activity_resetter() -> (Signal<f64>, impl FnMut()) {
    let last_activity = use_signal(|| js_sys::Date::now());

    let reset = {
        let mut last_activity = last_activity;
        move || {
            last_activity.set(js_sys::Date::now());
        }
    };

    (last_activity, reset)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn use_activity_resetter() -> (Signal<f64>, impl FnMut()) {
    let last_activity = use_signal(|| 0.0);
    let reset = || {};
    (last_activity, reset)
}

#[cfg(test)]
mod tests {
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
}
