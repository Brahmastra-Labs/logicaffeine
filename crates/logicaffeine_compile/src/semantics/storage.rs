//! Browser storage — the AREAS a page can keep text in, and the one place every tier decides what
//! an area IS.
//!
//! A page that says it persists something must actually persist it. Before this, the only
//! storage anywhere was a map inside the running program: a `## Store … persisted as "key"`
//! compiled to WebAssembly kept its state in the module's own memory, which dies with the page.
//! The sentence said "persisted" and a reload started from the initializers.
//!
//! ★ TEXT IN, TEXT OUT, AND NOTHING IS SPECIAL. A stored value is an opaque string: the browser
//! API is already length-delimited, so there is no separator to escape and no byte that means
//! anything. A quote, a newline, a NUL and a `☃` are ordinary content. This is the same reason
//! the patch stream length-prefixes its text rather than delimiting it — a format with no
//! delimiter has no escaping bug class.
//!
//! ★ A MISSING KEY IS `None`, NEVER `""`. An empty string is a value a page legitimately stores
//! (an empty draft is exactly what a cleared form field holds), and a reader that cannot tell
//! "nothing is saved" from "an empty string is saved" will overwrite one with the other.

use std::cell::RefCell;
use std::collections::BTreeMap;

/// Survives closing the tab — what `persisted as "key"` means.
pub const LOCAL: &str = "local";
/// Survives a RELOAD and ends with the tab — what `preserved across reloads` means.
pub const SESSION: &str = "session";
/// The topics every page on them shares — what `## Store … shared on "topic"` keeps its fields in,
/// keyed `<topic>.<field>`. A write is a frame the other pages on the topic receive
/// ([`take_shared_writes`]); the page that receives one merges it (`__lgx_shared`), the host never
/// does. Survives a reload and closing the tab: it lives with the topic, not the page.
pub const SHARED: &str = "shared";

#[derive(Default)]
struct Areas {
    local: BTreeMap<String, String>,
    session: BTreeMap<String, String>,
    shared: BTreeMap<String, String>,
    /// Every write to [`SHARED`] no other page has been handed yet, `(key, frame)`, in order.
    unsent: Vec<(String, String)>,
}

impl Areas {
    fn area(&mut self, name: &str) -> &mut BTreeMap<String, String> {
        match name {
            LOCAL => &mut self.local,
            SESSION => &mut self.session,
            _ => &mut self.shared,
        }
    }
}

thread_local! {
    static AREAS: RefCell<Areas> = RefCell::new(Areas::default());
}

/// Check an area name, or say what the areas are.
///
/// Not a silent no-op and not a third area invented on the spot: a page written against an area
/// nobody defines would store its state where nothing ever reads it, and would look like it
/// worked right up until the reload it was written for.
pub fn check_area(area: &str) -> Result<(), String> {
    if area == LOCAL || area == SESSION || area == SHARED {
        return Ok(());
    }
    Err(format!(
        "I do not know a storage area called \"{area}\". A page can store text in \"{LOCAL}\", \
         which survives closing the tab, \"{SESSION}\", which survives a reload and ends with the \
         tab, or \"{SHARED}\", which every page on a topic shares. Which did you mean?"
    ))
}

pub fn set(area: &str, key: &str, value: &str) -> Result<(), String> {
    check_area(area)?;
    AREAS.with(|a| {
        let mut a = a.borrow_mut();
        a.area(area).insert(key.to_string(), value.to_string());
        if area == SHARED {
            a.unsent.push((key.to_string(), value.to_string()));
        }
    });
    Ok(())
}

pub fn get(area: &str, key: &str) -> Result<Option<String>, String> {
    check_area(area)?;
    Ok(AREAS.with(|a| a.borrow_mut().area(area).get(key).cloned()))
}

/// Every write to [`SHARED`] since the last look, `(topic, key, frame)`, in order — taken, so the
/// host hands each to the other pages on its topic once, as a relay delivers a published frame.
/// The topic is the key's part before its first `.`.
pub fn take_shared_writes() -> Vec<(String, String, String)> {
    AREAS
        .with(|a| std::mem::take(&mut a.borrow_mut().unsent))
        .into_iter()
        .map(|(key, frame)| (key.split('.').next().unwrap_or("").to_string(), key, frame))
        .collect()
}

/// Removing a key that was never there is not an error: a page clearing state it may or may not
/// have written must not have to ask first.
pub fn remove(area: &str, key: &str) -> Result<(), String> {
    check_area(area)?;
    AREAS.with(|a| {
        a.borrow_mut().area(area).remove(key);
    });
    Ok(())
}

/// A FRESH BROWSER PROFILE — every area empty.
///
/// This is what starting a program means in a sealed world: an AOT-compiled binary and a browser
/// opened for the first time both begin with nothing stored. A run that inherited another run's
/// storage would make every test depend on the order the tests happened to run in.
pub fn reset() {
    AREAS.with(|a| *a.borrow_mut() = Areas::default());
}

/// Every area, saved. What [`take`] returns and [`restore`] puts back.
#[derive(Default)]
pub struct StorageState(Areas);

/// Take the current storage, leaving a fresh (empty) profile in its place — [`reset`], but the
/// profile it replaces comes back to the caller instead of being dropped. A nested run (a candidate
/// page tier compared against the reference) takes the caller's storage before writing its own, and
/// [`restore`]s it when it is done, so the caller's own storage survives the comparison instead of
/// leaking into — or out of — the candidate's run.
pub fn take() -> StorageState {
    StorageState(AREAS.with(RefCell::take))
}

/// Put back a storage profile a matching [`take`] returned.
pub fn restore(state: StorageState) {
    AREAS.with(|a| *a.borrow_mut() = state.0);
}

/// CLOSING THE TAB — the session area ends; the local and shared areas survive.
///
/// This is the whole difference between the two areas, and the only operation that can tell them
/// apart from the outside. Without it "preserved across reloads" and "persisted" would be two
/// spellings of the same lifetime.
pub fn end_session() {
    AREAS.with(|a| a.borrow_mut().session.clear());
}

/// Every key an area holds, in sorted order — for a test or a dev panel that wants to see what a
/// page has left on a visitor's machine.
pub fn keys(area: &str) -> Result<Vec<String>, String> {
    check_area(area)?;
    Ok(AREAS.with(|a| a.borrow_mut().area(area).keys().cloned().collect()))
}
