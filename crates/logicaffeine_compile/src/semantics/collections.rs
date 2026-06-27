//! Collection operations: indexing, length, membership, mutation, set algebra.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::RuntimeValue;

use super::compare::values_equal;

/// The interned single-character `Text` for an ASCII byte. `item k of text`
/// on an ASCII string is the hot path of every character-scanning loop
/// (string search, parsing); allocating a fresh one-char `Rc<String>` per
/// access dominated those loops. The 128 ASCII single-char strings are
/// immutable and shared per thread, so indexing collapses to a refcount
/// bump. (Texts are never mutated in place, so sharing the `Rc` is safe.)
fn ascii_char_text(b: u8) -> Rc<String> {
    debug_assert!(b < 128, "caller guarantees an ASCII byte");
    thread_local! {
        static CACHE: RefCell<[Option<Rc<String>>; 128]> =
            RefCell::new(std::array::from_fn(|_| None));
    }
    CACHE.with(|c| {
        c.borrow_mut()[b as usize]
            .get_or_insert_with(|| Rc::new((b as char).to_string()))
            .clone()
    })
}

/// Cached `(is_ascii, char_len)` for a `Text`, keyed by Rc identity AND byte
/// length. A character-scanning loop (`item i of text` for i = 1..n) otherwise
/// re-runs the O(n) `is_ascii()` / `chars().count()` on EVERY access — turning
/// the scan O(n²). The only in-place Text mutation is append (`add_assign`),
/// which grows the byte length, so a length mismatch detects staleness and
/// recomputes; holding the Rc clone keeps the pointer from being reused while
/// cached. (`index_set` has no Text arm — no same-length mutation exists — so
/// the metadata cannot silently change.)
fn text_metrics(s: &Rc<String>) -> (bool, usize) {
    thread_local! {
        static CACHE: RefCell<Vec<(Rc<String>, usize, bool, usize)>> =
            const { RefCell::new(Vec::new()) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let len = s.len();
        if let Some(e) = c.iter().find(|(rc, l, _, _)| *l == len && Rc::ptr_eq(rc, s)) {
            return (e.2, e.3);
        }
        let ascii = s.is_ascii();
        let char_len = if ascii { len } else { s.chars().count() };
        if c.len() >= 8 {
            c.remove(0);
        }
        c.push((s.clone(), len, ascii, char_len));
        (ascii, char_len)
    })
}

/// Whether a `Text` is pure ASCII, answered in O(1) per call via the same
/// `(is_ascii, char_len)` cache the indexing hot path uses (the first call for
/// a given `Rc`/length pays the vectorized `is_ascii()` scan, every later one is
/// a cache hit). The VM's tier-up seam asks this on every hot back-edge crossing
/// and again at every region entry: a Text-as-bytes pin is sound ONLY for ASCII
/// (char index == byte index, char count == byte length), so a non-ASCII Text
/// must never pin.
pub fn text_is_ascii(s: &Rc<String>) -> bool {
    text_metrics(s).0
}

/// 1-based index for List/Tuple/Text; key lookup for Map; Text-keyed field
/// read for Struct.
pub fn index_get(coll: &RuntimeValue, idx: &RuntimeValue) -> Result<RuntimeValue, String> {
    match (coll, idx) {
        (RuntimeValue::List(items), RuntimeValue::Int(i)) => {
            let i = *i as usize;
            let items = items.borrow();
            if i == 0 || i > items.len() {
                return Err(format!("Index {} out of bounds", i));
            }
            Ok(items.get(i - 1).expect("bounds checked above"))
        }
        (RuntimeValue::Tuple(items), RuntimeValue::Int(i)) => {
            let i = *i as usize;
            if i == 0 || i > items.len() {
                return Err(format!("Index {} out of bounds", i));
            }
            Ok(items[i - 1].clone())
        }
        (RuntimeValue::Text(s), RuntimeValue::Int(i)) => {
            let i = *i as usize;
            // ASCII fast path: byte position == char position and the
            // in-bounds check over bytes equals the check over chars. The
            // `is_ascii` scan is vectorized — far cheaper than the per-char
            // decode of `chars().nth` that the general path needs.
            let bytes = s.as_bytes();
            // Cached metrics: ASCII-ness and char length are O(1) per access
            // (recomputed only when the string's byte length changes), so a
            // scan loop stays O(n) instead of O(n²).
            let (ascii, char_len) = text_metrics(s);
            if i != 0 && i <= bytes.len() && ascii {
                return Ok(RuntimeValue::Text(ascii_char_text(bytes[i - 1])));
            }
            if i == 0 || i > char_len {
                return Err(format!("Index {} out of bounds", i));
            }
            // Index validated against the char count just above.
            Ok(RuntimeValue::Text(Rc::new(
                s.chars().nth(i - 1).unwrap().to_string(),
            )))
        }
        (RuntimeValue::Map(map), key) => {
            let map = map.borrow();
            match map.get(key) {
                Some(val) => Ok(val.clone()),
                None => Err(format!("Key '{}' not found in map", key.to_display_string())),
            }
        }
        // Struct field read via index syntax (`item "field" of struct`).
        (RuntimeValue::Struct(s), RuntimeValue::Text(field)) => {
            match s.fields.get(field.as_str()) {
                Some(val) => Ok(val.clone()),
                None => Err(format!("Struct has no field '{}'", field)),
            }
        }
        _ => Err(format!(
            "Cannot index {} with {}",
            coll.type_name(),
            idx.type_name()
        )),
    }
}

/// `Set item idx of collection to value` — 1-based list set, or map insert.
/// (Struct field set needs an environment reassign and stays engine-side.)
pub fn index_set(coll: &RuntimeValue, idx: &RuntimeValue, value: RuntimeValue) -> Result<(), String> {
    match (coll, idx) {
        (RuntimeValue::List(items), RuntimeValue::Int(n)) => {
            let i = *n as usize;
            let mut items = items.borrow_mut();
            if i == 0 || i > items.len() {
                return Err(format!(
                    "Index {} out of bounds for list of length {}",
                    i,
                    items.len()
                ));
            }
            items.set(i - 1, value);
            Ok(())
        }
        (RuntimeValue::Map(map), key) => {
            map.borrow_mut().insert(key.clone(), value);
            Ok(())
        }
        (RuntimeValue::List(_), _) => Err("List index must be an integer".to_string()),
        _ => Err(format!("Cannot index into {}", coll.type_name())),
    }
}

/// 1-indexed, inclusive-end slice of a List. Out-of-range slices are empty.
pub fn slice(
    coll: &RuntimeValue,
    start: &RuntimeValue,
    end: &RuntimeValue,
) -> Result<RuntimeValue, String> {
    match (coll, start, end) {
        (RuntimeValue::List(items), RuntimeValue::Int(s), RuntimeValue::Int(e)) => {
            let items = items.borrow();
            let start = (*s as usize).saturating_sub(1);
            let end = *e as usize;
            // Same out-of-range semantics as `slice.get(start..end)`: empty.
            let payload = if start < end && end <= items.len() {
                items.slice(start, end - 1)
            } else {
                crate::interpreter::ListRepr::Boxed(Vec::new())
            };
            Ok(RuntimeValue::List(Rc::new(RefCell::new(payload))))
        }
        _ => Err("Slice requires List and Int indices".to_string()),
    }
}

/// `length of x`. NOTE: Text length is BYTES (while Text indexing is chars) —
/// a pinned tree-walker behavior.
pub fn length_of(coll: &RuntimeValue) -> Result<RuntimeValue, String> {
    match coll {
        RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
        RuntimeValue::Tuple(items) => Ok(RuntimeValue::Int(items.len() as i64)),
        RuntimeValue::Set(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
        RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
        RuntimeValue::Map(map) => Ok(RuntimeValue::Int(map.borrow().len() as i64)),
        RuntimeValue::Crdt(c) => Ok(RuntimeValue::Int(c.borrow().len() as i64)),
        _ => Err(format!("Cannot get length of {}", coll.type_name())),
    }
}

/// Membership: `values_equal` scan for Set/List, key lookup for Map,
/// substring/char for Text.
pub fn contains(coll: &RuntimeValue, val: &RuntimeValue) -> Result<RuntimeValue, String> {
    match coll {
        RuntimeValue::List(items) => {
            Ok(RuntimeValue::Bool(items.borrow().contains(val)))
        }
        RuntimeValue::Set(items) => {
            let items = items.borrow();
            let found = items.iter().any(|item| values_equal(item, val));
            Ok(RuntimeValue::Bool(found))
        }
        RuntimeValue::Map(entries) => Ok(RuntimeValue::Bool(entries.borrow().contains_key(val))),
        RuntimeValue::Text(s) => {
            if let RuntimeValue::Text(needle) = val {
                Ok(RuntimeValue::Bool(s.contains(needle.as_str())))
            } else if let RuntimeValue::Char(c) = val {
                Ok(RuntimeValue::Bool(s.contains(*c)))
            } else {
                Err(format!("Cannot check if Text contains {}", val.type_name()))
            }
        }
        RuntimeValue::Crdt(c) => Ok(RuntimeValue::Bool(c.borrow().contains(val)?)),
        _ => Err(format!("Cannot check contains on {}", coll.type_name())),
    }
}

/// Set union — left's elements, then right's not already present.
pub fn union(left: &RuntimeValue, right: &RuntimeValue) -> Result<RuntimeValue, String> {
    match (left, right) {
        (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            let mut result = a.clone();
            for item in b.iter() {
                if !result.iter().any(|x| values_equal(x, item)) {
                    result.push(item.clone());
                }
            }
            Ok(RuntimeValue::Set(Rc::new(RefCell::new(result))))
        }
        _ => Err(format!(
            "Cannot union {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Set intersection — left's elements present in right, in left's order.
pub fn intersection(left: &RuntimeValue, right: &RuntimeValue) -> Result<RuntimeValue, String> {
    match (left, right) {
        (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            let result: Vec<RuntimeValue> = a
                .iter()
                .filter(|item| b.iter().any(|x| values_equal(x, item)))
                .cloned()
                .collect();
            Ok(RuntimeValue::Set(Rc::new(RefCell::new(result))))
        }
        _ => Err(format!(
            "Cannot intersect {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// `a to b` — inclusive integer range as a List.
pub fn range(start: &RuntimeValue, end: &RuntimeValue) -> Result<RuntimeValue, String> {
    match (start, end) {
        (RuntimeValue::Int(s), RuntimeValue::Int(e)) => {
            let range: Vec<i64> = (*s..=*e).collect();
            Ok(RuntimeValue::List(Rc::new(RefCell::new(
                crate::interpreter::ListRepr::Ints(range),
            ))))
        }
        _ => Err("Range requires Int bounds".to_string()),
    }
}

/// The `Repeat` iteration snapshot: the collection is materialized ONCE before
/// the loop, so mutation inside the body cannot extend or shrink the
/// iteration. Text iterates per char (as 1-char Texts); a Map yields (key,
/// value) Tuples in its (nondeterministic) iteration order.
pub fn iteration_snapshot(v: &RuntimeValue) -> Result<Vec<RuntimeValue>, String> {
    match v {
        RuntimeValue::List(list) => Ok(list.borrow().to_values()),
        RuntimeValue::Set(set) => Ok(set.borrow().clone()),
        RuntimeValue::Text(s) => Ok(s
            .chars()
            .map(|c| RuntimeValue::Text(Rc::new(c.to_string())))
            .collect()),
        RuntimeValue::Map(map) => Ok(map
            .borrow()
            .iter()
            .map(|(k, v)| RuntimeValue::Tuple(Rc::new(vec![k.clone(), v.clone()])))
            .collect()),
        _ => Err(format!("Cannot iterate over {}", v.type_name())),
    }
}

/// `Push value to obj's field` — pushes into a struct's List field through
/// the shared allocation. Every error string is the spec.
pub fn push_to_struct_field(
    obj: &RuntimeValue,
    field_name: &str,
    val: RuntimeValue,
) -> Result<(), String> {
    if let RuntimeValue::Struct(s) = obj {
        if let Some(RuntimeValue::List(items)) = s.fields.get(field_name) {
            items.borrow_mut().push(val);
            Ok(())
        } else {
            Err(format!("Field '{}' is not a List", field_name))
        }
    } else {
        Err("Cannot push to field of non-struct".to_string())
    }
}

/// `Push value to list` — mutates the shared allocation in place.
pub fn list_push(coll: &RuntimeValue, value: RuntimeValue) -> Result<(), String> {
    match coll {
        RuntimeValue::List(items) => {
            items.borrow_mut().push(value);
            Ok(())
        }
        _ => Err("Can only push to a List".to_string()),
    }
}

/// `Pop from list` — removes and returns the last element, or Nothing when
/// the list is empty (popping an empty list is NOT an error).
pub fn list_pop(coll: &RuntimeValue) -> Result<RuntimeValue, String> {
    match coll {
        RuntimeValue::List(items) => {
            Ok(items.borrow_mut().pop().unwrap_or(RuntimeValue::Nothing))
        }
        _ => Err("Can only pop from a List".to_string()),
    }
}

/// `Add value to set` — dedups via `values_equal`.
pub fn set_add(coll: &RuntimeValue, value: RuntimeValue) -> Result<(), String> {
    match coll {
        RuntimeValue::Set(items) => {
            let already_present = items.borrow().iter().any(|x| values_equal(x, &value));
            if !already_present {
                items.borrow_mut().push(value);
            }
            Ok(())
        }
        RuntimeValue::Crdt(c) => c.borrow_mut().insert(&value),
        _ => Err("Can only add to a Set".to_string()),
    }
}

/// `Remove value from set/map`.
pub fn remove_from(coll: &RuntimeValue, value: &RuntimeValue) -> Result<(), String> {
    match coll {
        RuntimeValue::Set(items) => {
            items.borrow_mut().retain(|x| !values_equal(x, value));
            Ok(())
        }
        RuntimeValue::Map(map) => {
            map.borrow_mut().remove(value);
            Ok(())
        }
        RuntimeValue::Crdt(c) => c.borrow_mut().remove(value),
        _ => Err("Can only remove from a Set or Map".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: Vec<RuntimeValue>) -> RuntimeValue {
        RuntimeValue::List(Rc::new(RefCell::new(crate::interpreter::ListRepr::from_values(
            items,
        ))))
    }

    #[test]
    fn index_is_one_based_with_exact_messages() {
        let xs = list(vec![RuntimeValue::Int(5), RuntimeValue::Int(6)]);
        assert!(matches!(index_get(&xs, &RuntimeValue::Int(1)).unwrap(), RuntimeValue::Int(5)));
        assert_eq!(index_get(&xs, &RuntimeValue::Int(0)).unwrap_err(), "Index 0 out of bounds");
        assert_eq!(index_get(&xs, &RuntimeValue::Int(3)).unwrap_err(), "Index 3 out of bounds");
        // A negative index wraps through `as usize` and prints the wrapped
        // number — a pinned tree-walker behavior.
        let e = index_get(&xs, &RuntimeValue::Int(-1)).unwrap_err();
        assert_eq!(e, format!("Index {} out of bounds", (-1i64) as usize));
    }

    #[test]
    fn text_indexing_is_chars_but_length_is_bytes() {
        let s = RuntimeValue::Text(Rc::new("héllo".to_string()));
        // 5 chars, 6 bytes.
        let c = index_get(&s, &RuntimeValue::Int(2)).unwrap();
        assert!(matches!(&c, RuntimeValue::Text(t) if **t == "é"));
        assert!(matches!(length_of(&s).unwrap(), RuntimeValue::Int(6)));
    }

    #[test]
    fn slice_is_one_indexed_inclusive_and_oob_is_empty() {
        let xs = list((1..=5).map(RuntimeValue::Int).collect());
        let s = slice(&xs, &RuntimeValue::Int(2), &RuntimeValue::Int(4)).unwrap();
        if let RuntimeValue::List(items) = &s {
            let v: Vec<i64> = items
                .borrow()
                .to_values()
                .iter()
                .map(|x| if let RuntimeValue::Int(n) = x { *n } else { panic!() })
                .collect();
            assert_eq!(v, vec![2, 3, 4]);
        } else {
            panic!("slice did not return a list");
        }
        let s = slice(&xs, &RuntimeValue::Int(4), &RuntimeValue::Int(99)).unwrap();
        if let RuntimeValue::List(items) = &s {
            assert!(items.borrow().is_empty());
        }
    }

    #[test]
    fn pop_of_empty_list_is_nothing_not_error() {
        let xs = list(vec![]);
        assert!(matches!(list_pop(&xs).unwrap(), RuntimeValue::Nothing));
    }

    #[test]
    fn set_add_dedups_with_epsilon_equality() {
        let s = RuntimeValue::Set(Rc::new(RefCell::new(vec![RuntimeValue::Float(0.3)])));
        set_add(&s, RuntimeValue::Float(0.1 + 0.2)).unwrap();
        if let RuntimeValue::Set(items) = &s {
            assert_eq!(items.borrow().len(), 1, "epsilon-equal float must dedup");
        }
    }

    #[test]
    fn range_requires_int_bounds() {
        assert_eq!(
            range(&RuntimeValue::Int(1), &RuntimeValue::Float(2.5)).unwrap_err(),
            "Range requires Int bounds"
        );
    }
}
