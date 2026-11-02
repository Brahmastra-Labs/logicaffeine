//! Custom Dioxus hooks for the Logicaffeine UI.
//!
//! This module provides reusable hooks for common UI patterns.

pub mod use_device_pixel_grid;
pub mod use_inactivity_timer;
pub mod use_media;
pub mod use_viewport_width;
pub mod use_visual_viewport;

pub use use_device_pixel_grid::{use_device_pixel_grid, PixelGrid};
pub use use_inactivity_timer::{use_inactivity_timer, InactivityState};
pub use use_media::use_media;
pub use use_viewport_width::use_viewport_width;
pub use use_visual_viewport::use_keyboard_inset;
