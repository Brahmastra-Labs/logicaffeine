//! Lean string literals elaborate to the constructor spelling of the export's own Lean release.
//! No Text primitive, UTF-8 byte indexing, or Unicode normalization is involved.
use super::profile::StringLiterals;
use super::*;

const CHAR: &str = "/s:\"Char\"";
const CHAR_OF_NAT: &str = "/s:\"Char\"/s:\"ofNat\"";
const STRING: &str = "/s:\"String\"";
const STRING_MK: &str = "/s:\"String\"/s:\"mk\"";
const STRING_OF_LIST: &str = "/s:\"String\"/s:\"ofList\"";
const LIST: &str = "/s:\"List\"";
const LIST_NIL: &str = "/s:\"List\"/s:\"nil\"";
const LIST_CONS: &str = "/s:\"List\"/s:\"cons\"";

/// The export holds exactly `literals`' release's `String`: its one constructor, and the packing
/// constant as that constructor or as a definition. The reading itself is the release's
/// ([`profile::release`](super::profile::release)), read from each release's own `Init.Prelude`.
fn held_by(literals: StringLiterals, ctx: &Context) -> bool {
    let constructors = ctx.get_constructors(STRING);
    constructors.len() == 1
        && constructors[0].0 == literals.string_constructor()
        && match literals {
            StringLiterals::ListOfChars => ctx.constructor_inductive(STRING_MK) == Some(STRING),
            StringLiterals::Utf8Bytes => {
                !ctx.is_constructor(STRING_OF_LIST)
                    && !ctx.is_inductive(STRING_OF_LIST)
                    && ctx.get_definition_body(STRING_OF_LIST).is_some()
            }
        }
}

pub(super) fn layout(row: &Value) -> Result<(usize, usize)> {
    let text = row["strVal"]
        .as_str()
        .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "malformed string literal"))?;
    let count = text.chars().count();
    Ok((
        count.saturating_mul(8).saturating_add(5),
        if count == 0 {
            3
        } else {
            count.saturating_add(4)
        },
    ))
}

pub(super) fn expand(
    importer: &Importer,
    row: &Value,
    source: usize,
    budget: &mut usize,
    mut context: Option<&mut Context>,
    report: &mut ImportReport,
    sharing: &mut Option<dag::Sharing>,
) -> Result<Term> {
    let (nodes, _) = layout(row)?;
    // Visit already charged one source node. Charge the remaining generated
    // application/constant/literal nodes before allocating any constructor tree.
    let remaining = nodes - 1;
    *budget = budget
        .checked_sub(remaining)
        .ok_or_else(|| declined(DiagnosticCode::ResourceLimit, "term expansion budget"))?;
    let text = row["strVal"].as_str().expect("validated string payload");
    let constant = |name: &str, arity: usize| -> Result<Term> {
        if importer.declarations.get(name) != Some(&arity)
            || importer.runtime_names.contains_key(name)
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                format!("string literal requires checked {name}"),
            ));
        }
        Ok(if arity == 0 {
            Term::Global(name.into())
        } else {
            Term::Const {
                name: name.into(),
                levels: vec![Universe::Prop],
            }
        })
    };
    let character = constant(CHAR, 0)?;
    let string = constant(STRING, 0)?;
    let of_nat = constant(CHAR_OF_NAT, 0)?;
    let list = apply(constant(LIST, 1)?, character.clone());
    let nil = apply(constant(LIST_NIL, 1)?, character.clone());
    let cons = apply(constant(LIST_CONS, 1)?, character.clone());
    // Two signals must agree: the header's release fixes how its kernel reads the literal, and the
    // export must hold exactly that release's `String`. Neither a header nor an environment alone
    // chooses the reading.
    let release = importer.lean_version.as_str();
    let packing = profile::release(release)
        .map(|r| r.string_literals)
        .ok_or_else(|| {
            declined(
                DiagnosticCode::UnsupportedExportVersion,
                format!("string literal: Lean release {release:?} has no pinned literal reading"),
            )
        })?;
    let ctx = context.as_deref().unwrap_or(&importer.ctx);
    if !held_by(packing, ctx)
        || !importer
            .declarations
            .contains_key(packing.packing_constant())
    {
        let built_by: Vec<&str> = ctx
            .get_constructors(STRING)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        return Err(declined(DiagnosticCode::MalformedExport, format!(
            "string literal: Lean {release} reads it through {} over a String built by {}, but the export's String is built by {built_by:?}",
            packing.packing_constant(), packing.string_constructor())));
    }
    let packed = constant(packing.packing_constant(), 0)?;
    // Reuse the importer's canonical Nat gate before generating code-point literals.
    importer.term_atom(
        &serde_json::json!({"natVal":"0"}),
        &[],
        &HashSet::new(),
        budget,
    )?;
    let arrow = |a: Term, b: Term| Term::Pi {
        param: "arena.string.argument".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    // Check all elaboration interfaces, including those unused by the empty
    // string. A similarly named function with a different type grants no rule.
    for (term, expected) in [
        (character.clone(), Term::Sort(Universe::Type(0))),
        (string.clone(), Term::Sort(Universe::Type(0))),
        (
            of_nat.clone(),
            arrow(Term::Global("Nat".into()), character.clone()),
        ),
        (nil.clone(), list.clone()),
        (
            cons.clone(),
            arrow(character, arrow(list.clone(), list.clone())),
        ),
    ] {
        exact(ctx, &lean_type(ctx, &term)?, &expected)?;
    }
    // The packing constant is the release's own `List Char → String`; an export whose constant of
    // that name has another type contradicts the release it names.
    exact(ctx, &lean_type(ctx, &packed)?, &arrow(list, string)).map_err(
        |verdict| match verdict {
            Verdict::Reject(_) => declined(
                DiagnosticCode::MalformedExport,
                format!(
                    "string literal: the export's {} is not Lean {release}'s List Char → String",
                    packing.packing_constant()
                ),
            ),
            decline => decline,
        },
    )?;
    let mut body = nil;
    let mut depth = 2usize;
    for scalar in text.chars().rev() {
        let value = Term::Lit(crate::Literal::Nat(logicaffeine_base::BigInt::from_u64(
            scalar as u64,
        )));
        body = apply(apply(cons.clone(), apply(of_nat.clone(), value)), body);
        depth = depth.max(3) + 1;
        // Use the existing doubly checked conservative-definition gateway.
        // Keep intermediate lists shallow rather than first building a deep tree.
        if depth >= dag::CLOSED_CHUNK_DEPTH {
            if let (Some(shared), Some(ctx)) = (sharing.as_mut(), context.as_deref_mut()) {
                let (cut, did_cut) = shared.cut_closed(ctx, report, source, body, budget)?;
                body = cut;
                if did_cut {
                    depth = 1;
                }
            }
        }
    }
    Ok(apply(packed, body))
}
