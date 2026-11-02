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
