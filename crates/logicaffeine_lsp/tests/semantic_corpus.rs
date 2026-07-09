//! The semantic-highlighting corpus golden: every token of a representative
//! program, rendered `lexeme→class[+modifiers]`, committed and eyeballable.
//! When highlighting changes — deliberately or by accident — the diff IS the
//! review. Structural invariants then sweep the wider extension corpus, and
//! a quickguide ratchet keeps every canonical form painted.

#[path = "harness/quickguide.rs"]
mod quickguide;

use logicaffeine_lsp::document::DocumentState;
use quickguide::quickguide_surface_forms;
use logicaffeine_lsp::semantic_tokens::{
    encode_document_tokens, MOD_DECLARATION, MOD_DEFAULT_LIBRARY, MOD_MODIFICATION, MOD_READONLY,
};

const CLASS_NAMES: [&str; 13] = [
    "keyword", "type", "function", "variable", "string", "number", "operator", "namespace",
    "modifier", "property", "comment", "parameter", "enumMember",
];

/// Render a document's semantic tokens per source line, one `lexeme→class`
/// entry per token, modifiers as `+decl/+ro/+mut/+std`.
fn render(source: &str) -> String {
    let doc = DocumentState::new(source.to_string(), 1);
    let tokens = encode_document_tokens(&doc);

    let mut lines: Vec<Vec<String>> = vec![Vec::new(); source.lines().count() + 1];
    let (mut line, mut character) = (0u32, 0u32);
    for token in tokens {
        line += token.delta_line;
        if token.delta_line > 0 {
            character = token.delta_start;
        } else {
            character += token.delta_start;
        }
        let src_line = source.lines().nth(line as usize).unwrap_or("");
        let utf16: Vec<u16> = src_line.encode_utf16().collect();
        let text = String::from_utf16_lossy(
            &utf16[(character as usize).min(utf16.len())
                ..((character + token.length) as usize).min(utf16.len())],
        );
        let mut entry = format!("{text}→{}", CLASS_NAMES[token.token_type as usize]);
        if token.token_modifiers_bitset & MOD_DECLARATION != 0 {
            entry.push_str("+decl");
        }
        if token.token_modifiers_bitset & MOD_READONLY != 0 {
            entry.push_str("+ro");
        }
        if token.token_modifiers_bitset & MOD_MODIFICATION != 0 {
            entry.push_str("+mut");
        }
        if token.token_modifiers_bitset & MOD_DEFAULT_LIBRARY != 0 {
            entry.push_str("+std");
        }
        if let Some(slot) = lines.get_mut(line as usize) {
            slot.push(entry);
        }
    }

    lines
        .iter()
        .enumerate()
        .filter(|(_, entries)| !entries.is_empty())
        .map(|(i, entries)| format!("{i}: {}", entries.join(" ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_representative_program_paints_exactly_this() {
    let source = "\
## To double (n: Int) -> Int:
    Return n * 2.

## A Point has:
    An x: Int.

## Main
Let answer be double(21).
Let mutable count be 0.
Set count to answer.
Let p be a new Point.
Show p's x.
";
    let expected = "\
0: ## To→namespace double→function+decl (→operator n→parameter+decl :→operator Int→type )→operator ->→operator Int→type :→operator
1: Return→keyword n→parameter *→operator 2→number .→operator
3: ## A→namespace Point→type+decl has→function :→operator
4: An→keyword x→property+decl :→operator Int→type .→operator
6: ## Main→namespace
7: Let→keyword answer→variable+decl+ro be→keyword double→function (→operator 21→number )→operator .→operator
8: Let→keyword mutable→modifier count→variable+decl be→keyword 0→number .→operator
9: Set→keyword count→variable+mut to→keyword answer→variable+ro .→operator
10: Let→keyword p→variable+decl+ro be→keyword a→keyword new→keyword Point→type .→operator
11: Show→keyword p→variable+ro 's→operator x→property .→operator
";
    assert_eq!(
        render(source) + "\n",
        expected,
        "\n--- actual rendering ---\n{}\n",
        render(source)
    );
}


#[test]
fn interpolation_interiors_paint_as_code_within_the_string() {
    let source = "## Main\nLet name be \"px\".\nShow \"Hello {name}!\".\n";
    let rendered = render(source);
    let line = rendered
        .lines()
        .find(|l| l.starts_with("2:"))
        .expect("the Show line paints");
    assert_eq!(
        line,
        "2: Show→keyword \"Hello →string {→operator name→variable+ro }→operator !\"→string .→operator",
        "the {{name}} interior is CODE — the variable paints as itself, braces as operators"
    );
}

/// The server-side mirror of the extension's grammar ratchet: every
/// canonical surface form in the quickguide must produce at least one
/// semantic token through the REAL pipeline. Fails in both drift
/// directions via the allowlist (currently empty — everything paints).
#[test]
fn every_quickguide_form_paints_through_the_real_pipeline() {
    const PROSE_ONLY: &[&str] = &[];

    let guide = include_str!("../../../LOGOS_QUICKGUIDE.md");
    let forms = quickguide_surface_forms(guide);
    assert!(
        forms.len() > 80,
        "the quickguide should yield a rich form set, got {}",
        forms.len()
    );

    let mut unpainted = Vec::new();
    let mut promotable = Vec::new();
    let mut crashers = Vec::new();
    for form in &forms {
        // A fragment must NEVER panic the pipeline — in production this
        // panic lands inside the server's analysis task.
        let probe = form.clone();
        let painted = match std::panic::catch_unwind(move || {
            let doc = DocumentState::new(probe, 1);
            !encode_document_tokens(&doc).is_empty()
        }) {
            Ok(painted) => painted,
            Err(_) => {
                crashers.push(form.clone());
                continue;
            }
        };
        let allowlisted = PROSE_ONLY.contains(&form.as_str());
        if !painted && !allowlisted {
            unpainted.push(form.clone());
        }
        if painted && allowlisted {
            promotable.push(form.clone());
        }
    }
    assert!(
        crashers.is_empty(),
        "these surface forms PANIC the analysis pipeline:\n{crashers:#?}"
    );
    assert!(
        unpainted.is_empty(),
        "surface forms with NO semantic tokens — classify them or allowlist with a reason:\n{unpainted:#?}"
    );
    assert!(
        promotable.is_empty(),
        "allowlisted forms that now paint — remove from PROSE_ONLY:\n{promotable:#?}"
    );
}
