use std::rc::Rc;

use super::{Overload, SelfInterpreted, Stop, BYTE_ELEMENT_REFUSAL};
use crate::interpreter::RuntimeValue;
use crate::semantics::builtins::{call_builtin, BuiltinId};

/// The builtins the self-interpreter runs, by overload or by its own definition. May only rise.
const SELF_INTERPRETED_BUILTINS: usize = 42;

/// The types the self-interpreter carries a value of, each with a sample value and the name the tree-walker gives a
/// value of it.
const CARRIED: &[&str] = &["Int", "Real", "Text", "Bool", "Bytes", "Duration", "Date", "Moment", "Span", "Time", "Char"];

/// A list as the tree-walker holds one.
fn list_of(items: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(std::cell::RefCell::new(crate::interpreter::ListRepr::from_values(items))))
}

fn sample(ty: &str) -> RuntimeValue {
    match ty {
        "Int" => RuntimeValue::Int(7),
        "Real" => RuntimeValue::Float(2.5),
        "Text" => RuntimeValue::Text(Rc::new("7".to_string())),
        "Bool" => RuntimeValue::Bool(true),
        "Bytes" => list_of(vec![RuntimeValue::Int(104), RuntimeValue::Int(105)]),
        "Duration" => RuntimeValue::Duration(1_500_000_007),
        "Date" => RuntimeValue::Date(20_000),
        "Moment" => RuntimeValue::Moment(1_710_498_645_123_456_789),
        "Span" => RuntimeValue::Span { months: 14, days: 3 },
        "Time" => RuntimeValue::Time(37_845_500_000_000),
        "Char" => RuntimeValue::Char('x'),
        other => panic!("no sample of `{other}`"),
    }
}

/// A well-formed argument of type `ty` for `id`: its sample, or for a decompression the sample compressed — a
/// decompression reads a compressed frame.
fn sample_for(id: BuiltinId, ty: &str) -> RuntimeValue {
    match (id, ty) {
        (BuiltinId::DecompressBytes, "Bytes") => {
            call_builtin(BuiltinId::CompressBytes, vec![sample("Bytes")]).expect("the sample compresses")
        }
        (BuiltinId::ParseTimestamp, "Text") => RuntimeValue::Text(Rc::new("2024-03-15T10:30:45.5Z".to_string())),
        (BuiltinId::InZone | BuiltinId::LocalInstant, "Text") => RuntimeValue::Text(Rc::new("America/New_York".to_string())),
        (BuiltinId::MakeDate, "Int") | (BuiltinId::MakeMoment, "Int") => RuntimeValue::Int(3),
        _ => sample(ty),
    }
}

fn runtime_name(ty: &str) -> &'static str {
    match ty {
        "Int" => "Int",
        "Real" => "Float",
        "Text" => "Text",
        "Bool" => "Bool",
        "Bytes" => "List",
        "Duration" => "Duration",
        "Date" => "Date",
        "Moment" => "Moment",
        "Span" => "Span",
        "Time" => "Time",
        "Char" => "Char",
        other => panic!("no runtime name of `{other}`"),
    }
}

/// Whether a parameter of type `ty` takes an argument the tree-walker names `got`: its own, or an `Int` for a `Real`.
fn takes(ty: &str, got: &str) -> bool {
    runtime_name(ty) == got || (ty == "Real" && got == "Int")
}

fn typed() -> Vec<(BuiltinId, &'static [Overload], &'static str)> {
    BuiltinId::ALL
        .iter()
        .filter_map(|id| match id.self_interpreted() {
            SelfInterpreted::Overloads { overloads, refusal } => Some((*id, overloads, refusal)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_table_names_only_values_the_self_interpreter_carries() {
    for (id, overloads, _) in typed() {
        assert!(!overloads.is_empty(), "{id:?} lists no overload");
        for o in overloads {
            for ty in o.params.iter().chain(std::iter::once(&o.answer)) {
                assert!(CARRIED.contains(ty), "{id:?}: `{ty}` is no value the self-interpreter carries");
            }
        }
    }
}

#[test]
fn every_overload_answers_on_the_tree_walker_as_the_table_says() {
    for (id, overloads, _) in typed() {
        for o in overloads {
            let args: Vec<RuntimeValue> = o.params.iter().map(|ty| sample_for(id, ty)).collect();
            let answer = call_builtin(id, args).unwrap_or_else(|e| panic!("{id:?} {o:?}: the tree-walker refuses it: {e}"));
            assert_eq!(
                answer.type_name(),
                runtime_name(o.answer),
                "{id:?} {o:?}: the tree-walker answers a {}",
                answer.type_name()
            );
        }
    }
}

#[test]
fn every_refusal_is_the_tree_walkers() {
    let strangers = [
        RuntimeValue::Bool(false),
        RuntimeValue::Nothing,
        RuntimeValue::Text(Rc::new("x".to_string())),
        RuntimeValue::Float(1.5),
    ];
    for (id, overloads, refusal) in typed() {
        let first = overloads[0];
        for position in 0..first.params.len() {
            let stranger = strangers
                .iter()
                .find(|v| overloads.iter().all(|o| !takes(o.params[position], v.type_name())))
                .unwrap_or_else(|| panic!("{id:?}: every candidate is taken at argument {}", position + 1));
            let args: Vec<RuntimeValue> = first
                .params
                .iter()
                .enumerate()
                .map(|(i, ty)| if i == position { stranger.clone() } else { sample(ty) })
                .collect();
            let names: Vec<&str> = args.iter().map(|a| a.type_name()).collect();
            let mut expected = refusal.to_string();
            for (i, name) in names.iter().enumerate() {
                expected = expected.replace(&format!("{{{}}}", i + 1), name);
            }
            assert_eq!(
                call_builtin(id, args).err().as_deref(),
                Some(expected.as_str()),
                "{id:?} with a {} at argument {}",
                stranger.type_name(),
                position + 1
            );
        }
    }
}

#[test]
fn every_stop_is_the_tree_walkers_own_refusal() {
    let mut checked = 0;
    for id in BuiltinId::ALL {
        for Stop { params, report } in id.self_interpreted_stops() {
            assert!(
                matches!(id.self_interpreted(), SelfInterpreted::Overloads { .. }),
                "{id:?} stops on argument types but the self-interpreter does not run it"
            );
            for ty in params.iter() {
                assert!(CARRIED.contains(ty), "{id:?}: a stop on `{ty}`, which the self-interpreter does not carry");
            }
            let args: Vec<RuntimeValue> = params.iter().map(|ty| sample_for(*id, ty)).collect();
            assert_eq!(call_builtin(*id, args).err().as_deref(), Some(*report), "{id:?} on {params:?}");
            checked += 1;
        }
    }
    assert!(checked >= 4, "the clock components and time_of refuse a Date in their own words: {checked}");
}

#[test]
fn a_byte_list_holding_another_value_stops_as_the_tree_walker_stops() {
    let mut checked = 0;
    for (id, overloads, _) in typed() {
        for o in overloads {
            for (position, ty) in o.params.iter().enumerate() {
                if *ty != "Bytes" {
                    continue;
                }
                checked += 1;
                let args: Vec<RuntimeValue> = o
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        if i == position {
                            list_of(vec![RuntimeValue::Int(104), RuntimeValue::Text(Rc::new("i".to_string()))])
                        } else {
                            sample(t)
                        }
                    })
                    .collect();
                assert_eq!(
                    call_builtin(id, args).err().as_deref(),
                    Some(BYTE_ELEMENT_REFUSAL.replace("{n}", "2").as_str()),
                    "{id:?}: a byte list whose second item is Text"
                );
            }
        }
    }
    assert!(checked >= 3, "the table reads byte lists");
}

#[test]
fn the_self_interpreter_runs_at_least_its_floor_of_builtins() {
    let run = BuiltinId::ALL
        .iter()
        .filter(|id| !matches!(id.self_interpreted(), SelfInterpreted::Awaits(_)))
        .count();
    assert!(
        run >= SELF_INTERPRETED_BUILTINS,
        "the self-interpreter runs {run} builtins, below its floor {SELF_INTERPRETED_BUILTINS}"
    );
}
