//! ANSI terminal color styling for error messages.
//!
//! This module provides simple ANSI escape code wrappers for colorizing
//! terminal output in error messages. All colors automatically reset at the end.

/// ANSI escape code styling utilities.
pub struct Style;

impl Style {
    pub const RESET: &'static str = "\x1b[0m";
    pub const BOLD: &'static str = "\x1b[1m";
    pub const RED: &'static str = "\x1b[31m";
    pub const GREEN: &'static str = "\x1b[32m";
    pub const YELLOW: &'static str = "\x1b[33m";
    pub const BLUE: &'static str = "\x1b[34m";
    pub const CYAN: &'static str = "\x1b[36m";

    pub fn red(s: &str) -> String {
        format!("{}{}{}", Self::RED, s, Self::RESET)
    }

    pub fn blue(s: &str) -> String {
        format!("{}{}{}", Self::BLUE, s, Self::RESET)
    }

    pub fn cyan(s: &str) -> String {
        format!("{}{}{}", Self::CYAN, s, Self::RESET)
    }

    pub fn yellow(s: &str) -> String {
        format!("{}{}{}", Self::YELLOW, s, Self::RESET)
    }

    pub fn green(s: &str) -> String {
        format!("{}{}{}", Self::GREEN, s, Self::RESET)
    }

    pub fn bold(s: &str) -> String {
        format!("{}{}{}", Self::BOLD, s, Self::RESET)
    }

    pub fn bold_red(s: &str) -> String {
        format!("{}{}{}{}", Self::BOLD, Self::RED, s, Self::RESET)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_wraps_string() {
        let result = Style::red("error");
        assert!(result.contains("\x1b[31m"));
        assert!(result.contains("error"));
        assert!(result.contains("\x1b[0m"));
    }

    #[test]
    fn bold_red_combines_codes() {
        let result = Style::bold_red("Error");
        assert!(result.contains("\x1b[1m"));
        assert!(result.contains("\x1b[31m"));
    }
}
