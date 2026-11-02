use lsp_types::{GotoDefinitionResponse, Location, Position, Range, Url};

use crate::document::DocumentState;

/// Handle go-to-definition request.
///
/// Given a cursor position, find the token at that position, look up its
/// name in the symbol index, and return the definition's location.
pub fn goto_definition(
    doc: &DocumentState,
    position: Position,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    let offset = doc.line_index.offset(position);

    // ★ A RESOLVED REFERENCE WINS OVER THE TOKEN UNDER THE CURSOR, and inside a template it is
    // the only thing there is: a `View:` body arrives as ONE `RawBlock` token, so the lookup
    // below would read the WHOLE template as the name and match nothing. Jumping from `{count}`
    // to the `Let count be 0.` it names — the most common movement in an LGX file — was
    // impossible for that reason alone.
    //
    // It is the better answer everywhere else too: a reference was resolved against the nearest
    // scope when the index was built, while the token path only knows a name.
    if let Some(reference) = doc
        .symbol_index
        .references
        .iter()
        .find(|r| offset >= r.span.start && offset < r.span.end)
    {
        if let Some(def) = reference
            .definition_idx
            .and_then(|i| doc.symbol_index.definitions.get(i))
        {
            if def.span != logicaffeine_language::token::Span::default() {
                return Some(GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: doc.line_index.position(def.span.start),
                        end: doc.line_index.position(def.span.end),
                    },
                }));
            }
        }
    }

    // Find the token at the cursor position
    let token = doc
        .tokens
        .iter()
        .find(|t| offset >= t.span.start && offset < t.span.end)?;

    let name = doc.source.get(token.span.start..token.span.end)?;

    // Look up definitions for this name
    let defs = doc.symbol_index.definitions_of(name);
    if defs.is_empty() {
        return None;
    }

    let locations: Vec<Location> = defs
        .iter()
        .filter(|d| d.span != logicaffeine_language::token::Span::default())
        .map(|d| Location {
            uri: uri.clone(),
            range: Range {
                start: doc.line_index.position(d.span.start),
                end: doc.line_index.position(d.span.end),
            },
        })
        .collect();

    if locations.is_empty() {
        None
    } else if locations.len() == 1 {
        Some(GotoDefinitionResponse::Scalar(
            locations.into_iter().next().unwrap(),
        ))
    } else {
        Some(GotoDefinitionResponse::Array(locations))
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/definition/tests.rs"]
mod tests;
