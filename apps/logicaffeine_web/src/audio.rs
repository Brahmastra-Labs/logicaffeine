//! Sound effect playback for gamification feedback.
//!
//! Provides audio cues for learning events like correct answers, XP gains,
//! combo achievements, and streak status. Uses JavaScript interop on WASM
//! targets; no-op on native targets for testing.
//!
//! # Usage
//!
//! ```no_run
//! use logicaffeine_web::audio::{SoundEffect, play_sound};
//!
//! // Play a sound when the user answers correctly
//! play_sound(SoundEffect::Correct);
//!
//! // Play combo sound for streak multipliers
//! play_sound(SoundEffect::ComboUp);
//! ```
//!
//! # JavaScript Integration
//!
//! On WASM targets, this module expects a global `window.playSound(name)` function
//! to be defined in the host page. The function receives the sound effect name
//! as a string (e.g., "correct", "combo_up").

/// Audio cues for gamification events.
///
/// Each variant maps to a distinct sound file. The [`SoundEffect::as_str`] method
/// returns the identifier passed to the JavaScript audio system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEffect {
    /// Played when the user earns XP from any source.
    XpGain,
    /// Played for bonus XP events (perfect answers, fast responses).
    CriticalHit,
    /// Played when the combo multiplier increases.
    ComboUp,
    /// Played when a combo streak is broken by an incorrect answer.
    ComboBreak,
    /// Played when an achievement is unlocked.
    Achievement,
    /// Played when the user gains a level.
    LevelUp,
    /// Played when a daily streak is preserved (e.g., freeze used).
    StreakSaved,
    /// Played when a daily streak is lost.
    StreakLost,
    /// Played for correct answers.
    Correct,
    /// Played for incorrect answers.
    Incorrect,
}

impl SoundEffect {
    /// Returns the string identifier for this sound effect.
    ///
    /// This identifier is passed to the JavaScript `playSound` function
    /// and should match the audio file naming convention.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::XpGain => "xp_gain",
            Self::CriticalHit => "critical",
            Self::ComboUp => "combo_up",
            Self::ComboBreak => "combo_break",
            Self::Achievement => "achievement",
            Self::LevelUp => "level_up",
            Self::StreakSaved => "streak_saved",
            Self::StreakLost => "streak_lost",
            Self::Correct => "correct",
            Self::Incorrect => "incorrect",
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::SoundEffect;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = window, js_name = playSound)]
        fn play_sound_js(effect: &str);
    }

    /// Plays the specified sound effect through the browser audio system.
    pub fn play_sound(effect: SoundEffect) {
        play_sound_js(effect.as_str());
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::play_sound;

/// Plays a sound effect (no-op on non-WASM targets).
///
/// On WASM targets, this calls the JavaScript `window.playSound()` function.
/// On native targets, this is a no-op to allow testing without audio setup.
#[cfg(not(target_arch = "wasm32"))]
pub fn play_sound(_effect: SoundEffect) {
    // No-op on non-wasm targets
}
