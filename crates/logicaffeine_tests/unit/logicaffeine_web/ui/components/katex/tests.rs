use super::*;

#[test]
fn test_parse_plain_text() {
    let parts = parse_latex_in_text("Hello world");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0], TextPart::Plain("Hello world".to_string()));
}

#[test]
fn test_parse_inline_latex() {
    let parts = parse_latex_in_text("The formula $x + y$ is simple");
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], TextPart::Plain("The formula ".to_string()));
    assert_eq!(
        parts[1],
        TextPart::Latex {
            content: "x + y".to_string(),
            display: false
        }
    );
    assert_eq!(parts[2], TextPart::Plain(" is simple".to_string()));
}

#[test]
fn test_parse_display_latex() {
    let parts = parse_latex_in_text("Consider: $$\\forall x$$ as shown");
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], TextPart::Plain("Consider: ".to_string()));
    assert_eq!(
        parts[1],
        TextPart::Latex {
            content: "\\forall x".to_string(),
            display: true
        }
    );
    assert_eq!(parts[2], TextPart::Plain(" as shown".to_string()));
}

#[test]
fn test_parse_mixed() {
    let parts = parse_latex_in_text("$A$ and $B$ implies $$A \\land B$$");
    assert_eq!(parts.len(), 5);
    assert_eq!(
        parts[0],
        TextPart::Latex {
            content: "A".to_string(),
            display: false
        }
    );
    assert_eq!(parts[1], TextPart::Plain(" and ".to_string()));
    assert_eq!(
        parts[2],
        TextPart::Latex {
            content: "B".to_string(),
            display: false
        }
    );
    assert_eq!(parts[3], TextPart::Plain(" implies ".to_string()));
    assert_eq!(
        parts[4],
        TextPart::Latex {
            content: "A \\land B".to_string(),
            display: true
        }
    );
}

#[test]
fn test_unclosed_inline() {
    let parts = parse_latex_in_text("Start $unclosed");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], TextPart::Plain("Start ".to_string()));
    assert_eq!(parts[1], TextPart::Plain("$unclosed".to_string()));
}
