//! Call hierarchy over the `SymbolIndex`'s call sites: who calls this
//! function, and what does it call. `## Main` callers have no hierarchy item
//! (Main is the program, not a function) — their calls simply don't appear
//! as incoming edges.

use tower_lsp::lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, Position, Range,
    SymbolKind, Url,
};

use crate::document::DocumentState;
use crate::index::DefinitionKind;

pub fn prepare(
    doc: &DocumentState,
    position: Position,
    uri: &Url,
) -> Option<Vec<CallHierarchyItem>> {
    let offset = doc.line_index.offset(position);
    let def_idx = doc
        .symbol_index
        .references
        .iter()
        .find(|r| r.span.start <= offset && offset < r.span.end)
        .and_then(|r| r.definition_idx)
        .or_else(|| {
            doc.symbol_index
                .definitions
                .iter()
                .position(|d| d.span.start <= offset && offset < d.span.end)
        })?;
    if doc.symbol_index.definitions[def_idx].kind != DefinitionKind::Function {
        return None;
    }
    Some(vec![item_for(doc, def_idx, uri)])
}

pub fn incoming_calls(
    doc: &DocumentState,
    item: &CallHierarchyItem,
    uri: &Url,
) -> Vec<CallHierarchyIncomingCall> {
    let Some(callee) = function_index_by_name(doc, &item.name) else {
        return Vec::new();
    };

    // Group call sites by calling function, preserving first-seen order.
    let mut callers: Vec<(usize, Vec<Range>)> = Vec::new();
    for site in &doc.symbol_index.call_sites {
        if site.callee != callee {
            continue;
        }
        let Some(caller) = site.caller else { continue };
        let range = to_range(doc, site.span);
        match callers.iter_mut().find(|(ix, _)| *ix == caller) {
            Some((_, ranges)) => ranges.push(range),
            None => callers.push((caller, vec![range])),
        }
    }

    callers
        .into_iter()
        .map(|(caller, from_ranges)| CallHierarchyIncomingCall {
            from: item_for(doc, caller, uri),
            from_ranges,
        })
        .collect()
}

pub fn outgoing_calls(
    doc: &DocumentState,
    item: &CallHierarchyItem,
    uri: &Url,
) -> Vec<CallHierarchyOutgoingCall> {
    let Some(caller) = function_index_by_name(doc, &item.name) else {
        return Vec::new();
    };

    let mut callees: Vec<(usize, Vec<Range>)> = Vec::new();
    for site in &doc.symbol_index.call_sites {
        if site.caller != Some(caller) {
            continue;
        }
        let range = to_range(doc, site.span);
        match callees.iter_mut().find(|(ix, _)| *ix == site.callee) {
            Some((_, ranges)) => ranges.push(range),
            None => callees.push((site.callee, vec![range])),
        }
    }

    callees
        .into_iter()
        .map(|(callee, from_ranges)| CallHierarchyOutgoingCall {
            to: item_for(doc, callee, uri),
            from_ranges,
        })
        .collect()
}

fn function_index_by_name(doc: &DocumentState, name: &str) -> Option<usize> {
    doc.symbol_index.name_to_defs.get(name)?.iter().copied().find(|&ix| {
        doc.symbol_index.definitions[ix].kind == DefinitionKind::Function
    })
}

fn item_for(doc: &DocumentState, def_idx: usize, uri: &Url) -> CallHierarchyItem {
    let def = &doc.symbol_index.definitions[def_idx];
    let range = to_range(doc, def.span);
    CallHierarchyItem {
        name: def.name.clone(),
        kind: SymbolKind::FUNCTION,
        tags: None,
        detail: def.detail.clone(),
        uri: uri.clone(),
        range,
        selection_range: range,
        data: None,
    }
}

fn to_range(doc: &DocumentState, span: logicaffeine_language::token::Span) -> Range {
    Range {
        start: doc.line_index.position(span.start),
        end: doc.line_index.position(span.end),
    }
}
