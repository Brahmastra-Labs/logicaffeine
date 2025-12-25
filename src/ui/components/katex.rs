use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = renderKaTeX)]
    fn render_katex(element_id: &str, latex: &str, display_mode: bool);
}

static KATEX_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_katex_id() -> String {
    let id = KATEX_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("katex-{}", id)
}

#[component]
pub fn KatexSpan(latex: String, #[props(default = false)] display: bool) -> Element {
    let element_id = use_signal(|| next_katex_id());

    use_effect(move || {
        let id = element_id.read().clone();
        let latex = latex.clone();
        render_katex(&id, &latex, display);
    });

    rsx! {
        span {
            id: "{element_id}",
            class: if display { "katex-display" } else { "katex-inline" }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPart {
    Plain(String),
    Latex { content: String, display: bool },
}

pub fn parse_latex_in_text(text: &str) -> Vec<TextPart> {
    let mut parts = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        let display_pos = remaining.find("$$");
        let inline_pos = remaining.find('$');

        match (display_pos, inline_pos) {
            (Some(d), Some(i)) if d <= i => {
                if d > 0 {
                    parts.push(TextPart::Plain(remaining[..d].to_string()));
                }
                remaining = &remaining[d + 2..];
                if let Some(end) = remaining.find("$$") {
                    parts.push(TextPart::Latex {
                        content: remaining[..end].to_string(),
                        display: true,
                    });
                    remaining = &remaining[end + 2..];
                } else {
                    parts.push(TextPart::Plain(format!("$${}", remaining)));
                    break;
                }
            }
            (_, Some(i)) => {
                if i > 0 {
                    parts.push(TextPart::Plain(remaining[..i].to_string()));
                }
                remaining = &remaining[i + 1..];
                if let Some(end) = remaining.find('$') {
                    parts.push(TextPart::Latex {
                        content: remaining[..end].to_string(),
                        display: false,
                    });
                    remaining = &remaining[end + 1..];
                } else {
                    parts.push(TextPart::Plain(format!("${}", remaining)));
                    break;
                }
            }
            (Some(d), None) => {
                if d > 0 {
                    parts.push(TextPart::Plain(remaining[..d].to_string()));
                }
                remaining = &remaining[d + 2..];
                if let Some(end) = remaining.find("$$") {
                    parts.push(TextPart::Latex {
                        content: remaining[..end].to_string(),
                        display: true,
                    });
                    remaining = &remaining[end + 2..];
                } else {
                    parts.push(TextPart::Plain(format!("$${}", remaining)));
                    break;
                }
            }
            (None, None) => {
                parts.push(TextPart::Plain(remaining.to_string()));
                break;
            }
        }
    }

    parts
}

#[cfg(test)]
mod tests {
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
        assert_eq!(parts[1], TextPart::Latex { content: "x + y".to_string(), display: false });
        assert_eq!(parts[2], TextPart::Plain(" is simple".to_string()));
    }

    #[test]
    fn test_parse_display_latex() {
        let parts = parse_latex_in_text("Consider: $$\\forall x$$ as shown");
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], TextPart::Plain("Consider: ".to_string()));
        assert_eq!(parts[1], TextPart::Latex { content: "\\forall x".to_string(), display: true });
        assert_eq!(parts[2], TextPart::Plain(" as shown".to_string()));
    }

    #[test]
    fn test_parse_mixed() {
        let parts = parse_latex_in_text("$A$ and $B$ implies $$A \\land B$$");
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0], TextPart::Latex { content: "A".to_string(), display: false });
        assert_eq!(parts[1], TextPart::Plain(" and ".to_string()));
        assert_eq!(parts[2], TextPart::Latex { content: "B".to_string(), display: false });
        assert_eq!(parts[3], TextPart::Plain(" implies ".to_string()));
        assert_eq!(parts[4], TextPart::Latex { content: "A \\land B".to_string(), display: true });
    }

    #[test]
    fn test_unclosed_inline() {
        let parts = parse_latex_in_text("Start $unclosed");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], TextPart::Plain("Start ".to_string()));
        assert_eq!(parts[1], TextPart::Plain("$unclosed".to_string()));
    }
}
