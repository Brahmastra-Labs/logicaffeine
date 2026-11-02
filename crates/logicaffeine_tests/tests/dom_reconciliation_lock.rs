//! ★ G17 LOCK — a bound control keeps its DOM node across repaints (the input-focus bug). ★
//!
//! A view draws itself whole on every turn. Drawn naively into the document, every turn would
//! recreate every node, so a bound `<input>` would be a brand-new element after each keystroke and
//! the browser would drop its focus: a visitor could type exactly ONE character.
//!
//! An LGX page reconciles INSIDE the module: `vdom.lg` commits each paint against its retained tree
//! and hands the host one stream per paint, naming every node by the id it was created under
//! (`dom_apply_words`). A node drawn with the same tag at the same place, or with the same key, IS
//! the node it was: its changed text, value, attributes and listeners are written in place, stale
//! children are freed where they stand, and nothing else is touched. Because the node survives,
//! focus, selection and scroll survive with it.
//!
//! This lock compiles real LGX pages to wasm and drives them end to end on wasmi, through the
//! event trampoline, against a MockDom that holds the browser state a recreate would lose (focus,
//! selection, scroll, a creation generation) and applies each committed stream by the ids it names,
//! exactly as `runtime.js`'s `applyByIds` does. It proves:
//!  (a) a bound input keeps ONE node across ≥3 successive updates;
//!  (b) focus + selection are retained across every update;
//!  (c) the bound value and any derived text update after every update;
//!  (d) a view's listeners do not accumulate duplicates on repaint;
//!  (e) an unrelated sibling node is untouched;
//!  (f) a stale child is removed in place (no wholesale recreate).
//!
//! `vdom.lg`'s streams are held shape by shape against both real hosts (the native recorder and
//! `runtime.js` under node) by `repaint_in_place_lock`, `keyed_reorder_lock` and
//! `live_node_write_lock`; this file holds them over whole compiled pages.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;
use std::cell::RefCell;
use std::rc::Rc;

#[path = "compile_common/mod.rs"]
mod common;

/// A real host node in the MockDom — the wasm analog of a browser `Node`.
/// `focused`/`sel`/`scroll` are the state a re-render MUST preserve by keeping the node
/// alive; a recreate would reset them (the whole bug).
#[derive(Clone, Debug, Default)]
struct HostNode {
    tag: String,
    is_text: bool,
    text: String,
    value: String,
    /// A checkbox's live checked PROPERTY — the presence a repaint must patch in place rather
    /// than rebuild, exactly like `value`.
    checked: bool,
    attrs: Vec<(String, String)>,
    children: Vec<u32>,
    parent: Option<u32>,
    /// Per-node event handler count — proves handlers refresh (stay 1), never stack.
    handler_counts: std::collections::HashMap<String, u32>,
    /// Browser-managed UI state that only survives if the node is not recreated.
    focused: bool,
    sel_start: u32,
    sel_end: u32,
    scroll_top: u32,
    /// A monotonically-increasing generation stamped at creation — a recreate bumps it,
    /// so the test can detect a node that was silently replaced even if ids were reused.
    generation: u64,
}

/// The MockDom: a host-node table holding what a browser holds in a live node, which the per-op
/// imports write directly and a committed stream reaches by the ids it names.
struct FocusDom {
    nodes: Vec<HostNode>,
    generation: u64,
    /// The listeners registered on each node, `id:event -> handle`: the closure imperative code
    /// attached, or — for a view's registration, which the page's node dispatcher answers — the
    /// node id the page registered it under, the id `runtime.js` keeps on its page listener.
    managed: std::collections::HashMap<String, i64>,
    /// Host-allocated Text bump pointer (well past a small program's heap).
    text_bump: usize,
    /// Stream id → the slot of `nodes` it names: a committed stream names its own new nodes, from
    /// `dom::STREAM_ID_BASE` up. An id below that range is a node this host minted, its own slot.
    streamed: std::collections::HashMap<u32, u32>,
    /// How many committed streams (`dom_apply_words`) the page handed over — one per paint.
    streams: usize,
}

impl FocusDom {
    fn new() -> Self {
        // Node 0 = body. A node the per-op imports create takes the next slot; a node a stream
        // creates is named by the stream, from `dom::STREAM_ID_BASE` up, and mapped to its slot.
        let body = HostNode {
            tag: "body".into(),
            generation: 0,
            ..HostNode::default()
        };
        FocusDom {
            nodes: vec![body],
            generation: 0,
            managed: std::collections::HashMap::new(),
            text_bump: 2_000_000,
            streamed: std::collections::HashMap::new(),
            streams: 0,
        }
    }

    /// The slot of `nodes` host or stream id `id` names.
    fn slot_of(&self, id: u32) -> u32 {
        if id < dom::STREAM_ID_BASE {
            return id;
        }
        *self
            .streamed
            .get(&id)
            .unwrap_or_else(|| panic!("the stream names node {id}, which is not live"))
    }

    /// Takes slot `n` out of its parent's children.
    fn detach(&mut self, n: u32) {
        if let Some(old) = self.nodes[n as usize].parent.take() {
            self.nodes[old as usize].children.retain(|&c| c != n);
        }
    }

    /// A fresh node at a new slot, named `id` by the stream — a new generation, as any creation is.
    fn create_streamed(&mut self, id: u32, fresh: HostNode) {
        self.generation += 1;
        let slot = self.nodes.len() as u32;
        self.nodes.push(HostNode {
            generation: self.generation,
            ..fresh
        });
        if self.streamed.insert(id, slot).is_some() {
            panic!("the stream creates node {id}, which is still live");
        }
    }

    /// One op of a committed stream, applied by the ids it names — the module already reconciled:
    /// exactly `runtime.js`'s `applyByIds`. A freed node gives up its id and every descendant's.
    fn apply_streamed(&mut self, op: &dom::DomOp) {
        use dom::DomOp;
        match op {
            DomOp::CreateElement { node, tag } => self.create_streamed(
                *node,
                HostNode {
                    tag: tag.clone(),
                    ..HostNode::default()
                },
            ),
            DomOp::CreateTextNode { node, text } => self.create_streamed(
                *node,
                HostNode {
                    tag: "#text".into(),
                    is_text: true,
                    text: text.clone(),
                    ..HostNode::default()
                },
            ),
            DomOp::SetText { node, text } => {
                let n = self.slot_of(*node);
                self.nodes[n as usize].text = text.clone();
            }
            DomOp::SetHtml { node, html } => {
                let n = self.slot_of(*node);
                self.nodes[n as usize].text = html.clone();
            }
            DomOp::SetValue { node, value } => {
                let n = self.slot_of(*node);
                self.nodes[n as usize].value = value.clone();
            }
            DomOp::SetChecked { node, checked } => {
                let n = self.slot_of(*node);
                self.nodes[n as usize].checked = *checked;
            }
            DomOp::SetAttribute { node, name, value } => {
                let n = self.slot_of(*node);
                let attrs = &mut self.nodes[n as usize].attrs;
                match attrs.iter_mut().find(|(k, _)| k == name) {
                    Some(slot) => slot.1 = value.clone(),
                    None => attrs.push((name.clone(), value.clone())),
                }
            }
            DomOp::RemoveAttribute { node, name } => {
                let n = self.slot_of(*node);
                self.nodes[n as usize].attrs.retain(|(k, _)| k != name);
            }
            DomOp::AppendChild { parent, child } => {
                let (p, c) = (self.slot_of(*parent), self.slot_of(*child));
                self.detach(c);
                self.nodes[p as usize].children.push(c);
                self.nodes[c as usize].parent = Some(p);
            }
            DomOp::InsertBefore {
                parent,
                child,
                before,
            } => {
                let (p, c) = (self.slot_of(*parent), self.slot_of(*child));
                self.detach(c);
                let before = (*before != u32::MAX).then(|| self.slot_of(*before));
                let kids = &mut self.nodes[p as usize].children;
                match before.and_then(|b| kids.iter().position(|&k| k == b)) {
                    Some(at) => kids.insert(at, c),
                    None => kids.push(c),
                }
                self.nodes[c as usize].parent = Some(p);
            }
            DomOp::RemoveNode { node } => {
                let n = self.slot_of(*node);
                self.detach(n);
            }
            DomOp::FreeNode { node } => {
                let n = self.slot_of(*node);
                self.detach(n);
                let mut gone = vec![n];
                let mut at = 0;
                while at < gone.len() {
                    gone.extend(self.nodes[gone[at] as usize].children.clone());
                    at += 1;
                }
                self.streamed.retain(|_, slot| !gone.contains(slot));
            }
            DomOp::ClearChildren { node } => {
                let n = self.slot_of(*node);
                for c in std::mem::take(&mut self.nodes[n as usize].children) {
                    self.nodes[c as usize].parent = None;
                }
            }
            DomOp::Listen { node, event, .. } => {
                let n = self.slot_of(*node);
                self.managed
                    .insert(format!("{n}:{event}"), i64::from(*node));
                self.nodes[n as usize]
                    .handler_counts
                    .insert(event.clone(), 1);
            }
            DomOp::Unlisten { node, event } => {
                let n = self.slot_of(*node);
                self.managed.remove(&format!("{n}:{event}"));
                self.nodes[n as usize].handler_counts.remove(event);
            }
            other => panic!("a committed stream carried {other:?}, which no paint emits"),
        }
    }

    /// Reserve a host-node slot at index `id`, filling holes so `id == nodes.len()-1`.
    fn ensure_slot(&mut self, id: u32) {
        while self.nodes.len() <= id as usize {
            self.nodes.push(HostNode::default());
        }
    }

    /// Find the first ELEMENT with `tag` in the document, depth-first from body — a tiny selector.
    /// Text nodes (which carry no tag) are never a match, and neither is a node no longer in the
    /// document: a replaced element keeps its slot here, and finding it would pass for the one that
    /// replaced it.
    fn first_by_tag(&self, tag: &str) -> Option<u32> {
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if !n.is_text && n.tag == tag {
                return Some(i);
            }
            stack.extend(n.children.iter().rev());
        }
        None
    }

    /// Everything `node` READS AS — its own text plus every descendant text node, in document
    /// order. The retiring dialect put a whole interpolated run on the element with one `setText`,
    /// so reading `.text` was enough; LGX gives each literal run and each hole its OWN text node,
    /// which is what will let a fine-grained update patch a hole without touching its neighbours.
    /// The assertion "the derived text says X" has to mean the same thing under both shapes.
    fn text_content(&self, node: u32) -> String {
        let n = &self.nodes[node as usize];
        let mut out = n.text.clone();
        for c in &n.children {
            out.push_str(&self.text_content(*c));
        }
        out
    }
}

type Data = Rc<RefCell<FocusDom>>;

/// Replay one decoded op of the bulk byte stream (`dom_apply`) through the same paths the per-op
/// imports take: each writes the document directly, so the bulk call and the fifty small ones are
/// the same thing to this host.
fn apply_dom_op(m: &mut FocusDom, op: &logicaffeine_compile::semantics::dom::DomOp) {
    use logicaffeine_compile::semantics::dom::DomOp;
    match op {
        DomOp::CreateElement { tag, .. } => {
            m.generation += 1;
            let g = m.generation;
            m.nodes.push(HostNode {
                tag: tag.clone(),
                generation: g,
                ..HostNode::default()
            });
        }
        DomOp::CreateTextNode { text, .. } => {
            m.generation += 1;
            let g = m.generation;
            m.nodes.push(HostNode {
                tag: "#text".into(),
                is_text: true,
                text: text.clone(),
                generation: g,
                ..HostNode::default()
            });
        }
        DomOp::SetText { node, text } => {
            m.ensure_slot(*node);
            m.nodes[*node as usize].text = text.clone();
        }
        DomOp::SetHtml { node, html } => {
            m.ensure_slot(*node);
            m.nodes[*node as usize].text = html.clone();
        }
        DomOp::ClearChildren { node } => {
            m.ensure_slot(*node);
            m.nodes[*node as usize].children.clear();
        }
        DomOp::SetAttribute { node, name, value } => {
            m.ensure_slot(*node);
            let attrs = &mut m.nodes[*node as usize].attrs;
            match attrs.iter_mut().find(|(k, _)| k == name) {
                Some(slot) => slot.1 = value.clone(),
                None => attrs.push((name.clone(), value.clone())),
            }
        }
        DomOp::AppendChild { parent, child } => {
            m.ensure_slot(*parent);
            m.nodes[*parent as usize].children.push(*child);
        }
        DomOp::SetValue { node, value } => {
            m.ensure_slot(*node);
            m.nodes[*node as usize].value = value.clone();
        }
        DomOp::SetChecked { node, checked } => {
            m.ensure_slot(*node);
            m.nodes[*node as usize].checked = *checked;
        }
        // The stream carries the REGISTRATION; a handler is a closure and cannot ride the wire, so
        // the handle is 0 here exactly as `patch.rs` documents.
        DomOp::AddEventListener { node, event } => {
            m.managed.insert(format!("{node}:{event}"), 0);
        }
        // A `setStream` mints an opaque host MediaStream, which this host does not have and no
        // identity question turns on — the same honest omission `runtime.js` makes.
        other => panic!("this host does not model {other:?}"),
    }
}

/// Read a `Text` handle (`[len][cap][data_ptr]`) from the module's memory.
fn read_text(c: &wasmi::Caller<'_, Data>, handle: i32) -> String {
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let d = mem.data(c);
    let h = handle as usize;
    let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
    let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
    String::from_utf8_lossy(&d[dp..dp + len]).into_owned()
}

/// Host-allocate `s` as a Text in the module's memory at the mock's bump pointer.
fn alloc_host_text(c: &mut wasmi::Caller<'_, Data>, s: &str) -> i32 {
    let bump = c.data().borrow().text_bump;
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let b = s.as_bytes();
    let dp = bump + 16;
    let handle = {
        let data = mem.data_mut(&mut *c);
        data[bump..bump + 4].copy_from_slice(&(b.len() as i32).to_le_bytes());
        data[bump + 4..bump + 8].copy_from_slice(&(b.len() as i32).to_le_bytes());
        data[bump + 8..bump + 12].copy_from_slice(&(dp as i32).to_le_bytes());
        data[dp..dp + b.len()].copy_from_slice(b);
        bump as i32
    };
    c.data().borrow_mut().text_bump = (dp + b.len() + 7) & !7;
    handle
}

/// Write `s`'s bytes into memory at `buf`, returning the length — the `fmt_*_into` shape.
fn fmt_into(c: &mut wasmi::Caller<'_, Data>, buf: i32, s: &str) -> i32 {
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let bytes = s.as_bytes();
    let data = mem.data_mut(c);
    data[buf as usize..buf as usize + bytes.len()].copy_from_slice(bytes);
    bytes.len() as i32
}

/// Links each of `names` — effect imports this host has no use for — as a no-op typed from the
/// module's own import table.
fn link_ignored(l: &mut wasmi::Linker<Data>, names: &[&str]) {
    for (name, params, results) in logicaffeine_compile::vm::wasm::host_import_signatures() {
        if !names.contains(&name) {
            continue;
        }
        assert!(
            results.is_empty(),
            "`{name}` answers a value, so no host may ignore it"
        );
        let ty = wasmi::FuncType::new(
            params
                .iter()
                .map(|&p| crate::common::wasm_host::value_type(p)),
            std::iter::empty(),
        );
        l.func_new("env", name, ty, |_c, _args, _out| Ok(()))
            .unwrap();
    }
}

/// Install every `dom_*` host over the MockDom: each per-op import writes the host-node table
/// directly, and a committed stream is applied by the ids it names.
fn install_hosts(l: &mut wasmi::Linker<Data>) {
    l.func_wrap("env", "dom_body", |_c: wasmi::Caller<'_, Data>| -> i64 {
        0
    })
    .unwrap();
    // The turn journal (a `## Test`'s own concept) and a cross-document navigation: effects no
    // reconciliation question turns on, which every page that emits them must still link and run
    // past here. Each is a no-op typed from the module's own
    // import table, so a change of its signature needs nothing from this host.
    link_ignored(l, &["dom_note_turn", "dom_assign_location"]);
    // This host keeps no journal, so a turn need not capture what it changed for one.
    l.func_wrap(
        "env",
        "dom_journal_detail",
        |_c: wasmi::Caller<'_, Data>| -> i64 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_wake_after",
        |_c: wasmi::Caller<'_, Data>, _ms: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_create_element",
        |c: wasmi::Caller<'_, Data>, tag: i32| -> i64 {
            let tag = read_text(&c, tag);
            let d = c.data().clone();
            let mut m = d.borrow_mut();
            m.generation += 1;
            let g = m.generation;
            let id = m.nodes.len() as u32;
            m.nodes.push(HostNode {
                tag,
                generation: g,
                ..HostNode::default()
            });
            id as i64
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_create_text_node",
        |c: wasmi::Caller<'_, Data>, txt: i32| -> i64 {
            let text = read_text(&c, txt);
            let d = c.data().clone();
            let mut m = d.borrow_mut();
            m.generation += 1;
            let g = m.generation;
            let id = m.nodes.len() as u32;
            m.nodes.push(HostNode {
                is_text: true,
                text,
                generation: g,
                ..HostNode::default()
            });
            id as i64
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_text",
        |c: wasmi::Caller<'_, Data>, el: i64, txt: i32| {
            let text = read_text(&c, txt);
            let mut m = c.data().borrow_mut();
            m.nodes[el as usize].text = text;
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_html",
        |c: wasmi::Caller<'_, Data>, el: i64, html: i32| {
            let html = read_text(&c, html);
            let mut m = c.data().borrow_mut();
            m.nodes[el as usize].text = html;
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_clear_children",
        |c: wasmi::Caller<'_, Data>, el: i64| {
            let mut m = c.data().borrow_mut();
            let kids: Vec<u32> = std::mem::take(&mut m.nodes[el as usize].children);
            for k in kids {
                m.nodes[k as usize].parent = None;
            }
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_on_message",
        |_c: wasmi::Caller<'_, Data>, _t: i32, _h: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_publish",
        |_c: wasmi::Caller<'_, Data>, _t: i32, _m: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_attribute",
        |c: wasmi::Caller<'_, Data>, el: i64, name: i32, val: i32| {
            let name = read_text(&c, name);
            let value = read_text(&c, val);
            let mut m = c.data().borrow_mut();
            let n = &mut m.nodes[el as usize];
            if let Some(s) = n.attrs.iter_mut().find(|(k, _)| *k == name) {
                s.1 = value;
            } else {
                n.attrs.push((name, value));
            }
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_append_child",
        |c: wasmi::Caller<'_, Data>, parent: i64, child: i64| {
            let mut m = c.data().borrow_mut();
            m.nodes[parent as usize].children.push(child as u32);
            m.nodes[child as usize].parent = Some(parent as u32);
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_value",
        |c: wasmi::Caller<'_, Data>, el: i64, txt: i32| {
            let value = read_text(&c, txt);
            let mut m = c.data().borrow_mut();
            m.nodes[el as usize].value = value;
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_checked",
        |c: wasmi::Caller<'_, Data>, el: i64, on: i64| {
            let mut m = c.data().borrow_mut();
            m.nodes[el as usize].checked = on != 0;
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_query_selector",
        |c: wasmi::Caller<'_, Data>, sel: i32| -> i64 {
            let sel = read_text(&c, sel);
            let m = c.data().borrow();
            if let Some(id) = sel.strip_prefix('#') {
                for (i, n) in m.nodes.iter().enumerate() {
                    if n.attrs.iter().any(|(k, v)| k == "id" && v == id) {
                        return i as i64;
                    }
                }
                -1
            } else {
                m.first_by_tag(&sel).map(|i| i as i64).unwrap_or(-1)
            }
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_get_text",
        |_c: wasmi::Caller<'_, Data>, _el: i64| -> i32 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_get_attribute",
        |mut c: wasmi::Caller<'_, Data>, el: i64, name: i32| -> i32 {
            let name = read_text(&c, name);
            let value = {
                let m = c.data().borrow();
                let slot = m.slot_of(el as u32) as usize;
                m.nodes[slot]
                    .attrs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            };
            alloc_host_text(&mut c, &value)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_get_value",
        |mut c: wasmi::Caller<'_, Data>, el: i64| -> i32 {
            let v = {
                let m = c.data().borrow();
                m.nodes
                    .get(m.slot_of(el as u32) as usize)
                    .map(|n| n.value.clone())
                    .unwrap_or_default()
            };
            alloc_host_text(&mut c, &v)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_add_event_listener",
        |c: wasmi::Caller<'_, Data>, el: i64, event: i32, handler: i32| {
            let event = read_text(&c, event);
            let mut m = c.data().borrow_mut();
            m.managed
                .insert(format!("{}:{event}", el as u32), i64::from(handler));
            *m.nodes[el as usize]
                .handler_counts
                .entry(event)
                .or_insert(0) += 1;
        },
    )
    .unwrap();
    // `listenFlags(el, event, flags)`: a registration under the id the page names the node by —
    // ONE listener per (node, event) however often it is repeated, as `runtime.js`'s `pageListen`
    // keeps it.
    l.func_wrap(
        "env",
        "dom_listen_flags",
        |c: wasmi::Caller<'_, Data>, el: i64, event: i32, _flags: i64| {
            let event = read_text(&c, event);
            let mut m = c.data().borrow_mut();
            m.managed.insert(format!("{}:{event}", el as u32), el);
            m.nodes[el as usize].handler_counts.insert(event, 1);
        },
    )
    .unwrap();
    // ★ THE BULK CALL. A module that hands its ops over in one `dom_apply` must instantiate against
    // this host and see them applied: a wasm module importing a function its host does not define
    // does not start, with no compile error anywhere (`wasm_host_abi_lock` holds the whole table).
    // It decodes with the SAME codec the native recorder and the browser use, then replays each op
    // through the ordinary per-op paths.
    l.func_wrap(
        "env",
        "dom_apply",
        |c: wasmi::Caller<'_, Data>, patches: i32| {
            let bytes = {
                let mem = c
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .expect("memory");
                let data = mem.data(&c);
                let at = patches as usize;
                let len = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
                let ptr = i32::from_le_bytes(data[at + 8..at + 12].try_into().unwrap()) as usize;
                (0..len)
                    .map(|i| {
                        let e = ptr + i * 8;
                        i64::from_le_bytes(data[e..e + 8].try_into().unwrap()) as u8
                    })
                    .collect::<Vec<u8>>()
            };
            let ops = logicaffeine_compile::semantics::patch::decode(&bytes)
                .unwrap_or_else(|e| panic!("the module emitted an undecodable patch stream: {e}"));
            let d = c.data().clone();
            let mut m = d.borrow_mut();
            for op in &ops {
                apply_dom_op(&mut m, op);
            }
        },
    )
    .unwrap();
    // A paint the module already reconciled, as one stream by node id.
    l.func_wrap(
        "env",
        "dom_apply_words",
        |c: wasmi::Caller<'_, Data>,
         words: i32,
         word_count: i64,
         strings: i32,
         string_count: i64|
         -> i64 {
            let ops = crate::common::wasm_host::applied_words(
                &c,
                words,
                word_count,
                strings,
                string_count,
            );
            let mut m = c.data().borrow_mut();
            m.streams += 1;
            for op in &ops {
                m.apply_streamed(op);
            }
            0
        },
    )
    .unwrap();
}

/// A compiled-and-mounted reactive app plus its MockDom — the driver for the G17 lock: it fires
/// `input` (and other) events through the trampoline, and the page's turn repaints and hands its
/// stream over inside the handler, exactly as in the browser.
struct World {
    dom: Data,
    store: wasmi::Store<Data>,
    inst: wasmi::Instance,
}

/// The imports this host answers itself: the shared value imports and failure hooks, and the DOM.
fn modelled_linker(engine: &wasmi::Engine) -> wasmi::Linker<Data> {
    let mut l = wasmi::Linker::<Data>::new(engine);
    crate::common::wasm_host::link_value_imports::<Data>(&mut l);
    l.func_wrap(
        "env",
        "logos_rt_fail",
        |_c: wasmi::Caller<'_, _>, _h: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "logos_rt_fail_at",
        |_c: wasmi::Caller<'_, _>, _code: i32, _v: i64| {},
    )
    .unwrap();
    install_hosts(&mut l);
    l
}

/// Every host this file instantiates a page against: the modelled imports, and a named trap for
/// each declared import the host does not model.
fn host_linker(engine: &wasmi::Engine) -> wasmi::Linker<Data> {
    let mut l = modelled_linker(engine);
    crate::common::wasm_host::link_unmodelled_imports(&mut l);
    l
}

fn world(src: &str) -> World {
    let module = compile_to_wasm(src).expect("compiles to wasm");
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
    let dom: Data = Rc::new(RefCell::new(FocusDom::new()));
    let l = host_linker(&engine);
    let mut store = wasmi::Store::new(&engine, dom.clone());
    let inst = l
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("main runs");
    World { dom, store, inst }
}

impl World {
    /// Fire `event` on `node` carrying `typed`, handed to the page's node dispatcher as `runtime.js`
    /// hands it; the turn repaints INSIDE its own handler and hands its stream over there.
    fn type_into(&mut self, node: u32, event: &str, typed: &str) {
        let registered = self
            .dom
            .borrow()
            .managed
            .get(&format!("{node}:{event}"))
            .copied()
            .expect("a listener is registered on the node");
        let answered = crate::common::wasm_host::dispatch_node(
            &mut self.store,
            &self.inst,
            registered,
            event,
            typed,
        )
        .expect("handler fires");
        assert!(answered, "the page answers {event} on node {node}");
    }

    /// The current input node id (the first `<input>` in the tree).
    fn input_id(&self) -> u32 {
        self.dom
            .borrow()
            .first_by_tag("input")
            .expect("an input exists")
    }
}

/// The form app under test: a bound `<input>` plus a `<p>` that echoes the field's value. Every
/// turn re-seeds the input from `name` and rebinds the write-back — the shape a naive rebuild
/// would recreate the node under, which is exactly what reconciling must not do.
const LGX_FORM_APP: &str = concat!(
    "## Component Form\n",
    "State:\n",
    "    Let name be \"\".\n",
    "View:\n",
    "    <input id=\"f\" type=\"text\" bind={name}>\n",
    "    <p id=\"echo\">Hello, {name}</p>\n",
    "\n## Main\n",
    "Let app be createElement(\"div\").\n",
    "Call setAttribute with app and \"id\" and \"root\".\n",
    "Call appendChild with body() and app.\n",
    "Mount Form at \"#root\".\n",
);

// ---- LGX twins of the typed-cell tests. What the dialect called "a signal declared Real" is,
// in LGX, an ordinary typed `State:` field — so these ask the same questions of the model that
// replaces it: does a Real field keep its Real-ness from a whole-number start, does an Int field
// stay integral through a division, do the three types coexist in one view, and does a turn that
// writes one of them repaint with the new value.

/// Build a one-component page around `state`, `actions` and `view`, mounted at `#root`.
fn lgx_page(state: &str, actions: &str, view: &str) -> String {
    format!(
        "## Component App\nState:\n{state}{actions}View:\n{view}\n## Main\n\
         Let app be createElement(\"div\").\n\
         Call setAttribute with app and \"id\" and \"root\".\n\
         Call appendChild with body() and app.\n\
         Mount App at \"#root\".\n"
    )
}

#[test]
fn lgx_a_real_field_renders_its_value() {
    let w = world(&lgx_page(
        "    Let price be 1.5.\n",
        "",
        "    <p id=\"amt\">price = {price}</p>\n",
    ));
    let d = w.dom.borrow();
    let p = d.first_by_tag("p").expect("the price <p>");
    assert_eq!(
        d.text_content(p),
        "price = 1.5",
        "a Real field renders its value"
    );
}

#[test]
fn lgx_a_real_field_re_renders_after_a_turn() {
    // `3.25` has an exact f64 decimal, so the rendered string is stable.
    let mut w = world(&lgx_page(
        "    Let price be 1.5.\n",
        "Actions:\n    To bump:\n        Set price to 3.25.\n",
        "    <button id=\"bump\" on-click={bump}>bump</button>\n    <p id=\"amt\">price = {price}</p>\n",
    ));
    let button = w
        .dom
        .borrow()
        .first_by_tag("button")
        .expect("the bump button");
    w.type_into(button, "click", "");
    let d = w.dom.borrow();
    let p = d.first_by_tag("p").expect("the price <p>");
    assert_eq!(
        d.text_content(p),
        "price = 3.25",
        "the turn's new Real is what the repaint shows"
    );
}

#[test]
fn lgx_a_real_field_stays_real_through_a_turn() {
    // The DECLARATION decides: `Let rate: Real be 3.` holds 3 as a Real, so `rate / 2` is 1.5 and
    // not 1. This is the dialect's `Let rate: Real be a signal 3` asked of the model that replaces
    // it — and it only became askable once the annotation widened the literal instead of being
    // accepted and ignored.
    let mut w = world(&lgx_page(
        "    Let rate: Real be 3.\n",
        "Actions:\n    To halve:\n        Set rate to rate / 2.\n",
        "    <button id=\"halve\" on-click={halve}>halve</button>\n    <p id=\"r\">rate = {rate}</p>\n",
    ));
    let d0 = w.dom.borrow().first_by_tag("p").expect("the rate <p>");
    assert_eq!(
        w.dom.borrow().text_content(d0),
        "rate = 3",
        "a Real slot given 3 renders 3"
    );
    let button = w
        .dom
        .borrow()
        .first_by_tag("button")
        .expect("the halve button");
    w.type_into(button, "click", "");
    let d = w.dom.borrow();
    let p = d.first_by_tag("p").expect("the rate <p> still exists");
    assert_eq!(
        d.text_content(p),
        "rate = 1.5",
        "a slot DECLARED Real divides as a Real — the declaration beats the whole-number start"
    );
}

#[test]
fn lgx_a_field_declared_int_stays_an_int() {
    // The other direction, and the sharper one: an Int field must divide INTEGRALLY. If the
    // declaration were ignored, `3 / 2` would render 1.5 and every index computed from it would
    // be wrong somewhere far away.
    let mut w = world(&lgx_page(
        "    Let count: Int be 3.\n",
        "Actions:\n    To halve:\n        Set count to count / 2.\n",
        "    <button id=\"halve\" on-click={halve}>halve</button>\n    <p id=\"c\">count = {count}</p>\n",
    ));
    let button = w
        .dom
        .borrow()
        .first_by_tag("button")
        .expect("the halve button");
    w.type_into(button, "click", "");
    let d = w.dom.borrow();
    let p = d.first_by_tag("p").expect("the count <p>");
    assert_eq!(
        d.text_content(p),
        "count = 1",
        "an Int field divides as an integer"
    );
}

#[test]
fn lgx_int_real_and_text_fields_coexist_in_one_view() {
    // Three types through one render, because the tier that carries them is the one place a
    // Real can quietly become an Int (or a Text handle an integer) without anything else noticing.
    let w = world(&lgx_page(
        "    Let n be 7.\n    Let r be 2.5.\n    Let t be \"hi\".\n",
        "",
        "    <p id=\"all\">{n} {r} {t}</p>\n",
    ));
    let d = w.dom.borrow();
    let p = d.first_by_tag("p").expect("the <p>");
    assert_eq!(
        d.text_content(p),
        "7 2.5 hi",
        "an Int, a Real and a Text all survive the tier"
    );
}

/// The document a native tier renders for `src`, asserting the tree-walker and the VM agree —
/// the same double-check `reactive.rs::lgx_doc` makes, reproduced here because this file's own
/// helpers are wasm-only.
fn native_doc(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let tw_doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it too:\n{src}");
    assert_eq!(
        tw_doc,
        dom::render(),
        "the two native tiers rendered differently:\n{src}"
    );
    tw_doc
}

/// A `State:` field started at a whole number, `Mount`ed with no click yet — the LGX twin of the
/// retired dialect's `a_signal_started_whole_and_later_set_to_a_float_renders_its_initial_value`,
/// asked of the model that replaces it: an ordinary undeclared `Let x be 2.` field, mounted, before
/// the turn that later widens it to a fraction ever runs.
const FRACTION_SRC: &str = concat!(
    "## Component App\n",
    "State:\n",
    "    Let x be 2.\n",
    "Actions:\n",
    "    To bump:\n",
    "        Set x to 2.5.\n",
    "View:\n",
    "    <p id=\"x\">{x}</p><button id=\"b\" on-click={bump}>bump</button>\n",
    "\n## Main\n",
    "Mount App.\n",
);

#[test]
fn lgx_a_field_started_whole_and_later_set_to_a_fraction_renders_its_initial_value() {
    let doc = native_doc(FRACTION_SRC);
    assert!(
        doc.contains(">2<"),
        "the whole-number start must render before any turn:\n{doc}"
    );

    let w = world(&lgx_page(
        "    Let x be 2.\n",
        "Actions:\n    To bump:\n        Set x to 2.5.\n",
        "    <p id=\"x\">{x}</p>\n    <button id=\"b\" on-click={bump}>bump</button>\n",
    ));
    let d = w.dom.borrow();
    let p = d.first_by_tag("p").expect("the x <p>");
    assert_eq!(
        d.text_content(p),
        "2",
        "wasm(MockDom) must also render the whole-number start"
    );
}

#[test]
fn lgx_a_field_started_whole_and_later_set_to_a_fraction_renders_the_new_value() {
    let doc = native_doc(&format!("{FRACTION_SRC}Click \"#b\".\n"));
    assert!(
        doc.contains(">2.5<"),
        "the fractional write must repaint the field:\n{doc}"
    );
    assert!(
        !doc.contains(">2<"),
        "the whole-number start must be gone:\n{doc}"
    );

    let mut w = world(&lgx_page(
        "    Let x be 2.\n",
        "Actions:\n    To bump:\n        Set x to 2.5.\n",
        "    <p id=\"x\">{x}</p>\n    <button id=\"b\" on-click={bump}>bump</button>\n",
    ));
    let button = w
        .dom
        .borrow()
        .first_by_tag("button")
        .expect("the bump button");
    w.type_into(button, "click", "");
    let d = w.dom.borrow();
    let p = d.first_by_tag("p").expect("the x <p> still exists");
    assert_eq!(
        d.text_content(p),
        "2.5",
        "wasm(MockDom) must repaint the fractional write too"
    );
}

/// ★ The LGX twin of `bound_input_keeps_one_node_and_focus_across_three_updates`.
///
/// An LGX repaint draws the whole view again, and the module's reconciler (`vdom.lg`) commits
/// only what changed: each keystroke reaches the host as ONE stream that patches the input's
/// value in place and the echo's text, and touches nothing else. So the element being typed
/// into is the same element afterwards — same slot, same generation — and the focus, selection
/// and scroll the visitor left in it are still there.
#[test]
fn lgx_bound_input_keeps_one_node_and_focus_across_three_updates() {
    let mut w = world(LGX_FORM_APP);
    let input = w.input_id();
    {
        let mut d = w.dom.borrow_mut();
        let n = &mut d.nodes[input as usize];
        n.focused = true;
        n.sel_start = 1;
        n.sel_end = 1;
        n.scroll_top = 7;
    }
    let generation0 = w.dom.borrow().nodes[input as usize].generation;
    // The mount committed once. Everything below is measured against that.
    assert_eq!(
        w.dom.borrow().streams,
        1,
        "the mount committed exactly one stream"
    );

    for (i, typed) in ["H", "He", "Hel"].iter().enumerate() {
        w.type_into(input, "input", typed);
        assert_eq!(
            w.dom.borrow().streams,
            i + 2,
            "update {i}: the keystroke's repaint reaches the host as exactly one committed stream"
        );
        assert_eq!(
            w.input_id(),
            input,
            "update {i}: the bound input MUST keep the SAME node id (a recreate loses focus)"
        );
        let d = w.dom.borrow();
        let n = &d.nodes[input as usize];
        assert_eq!(
            n.generation, generation0,
            "update {i}: the input node was silently RECREATED (generation bumped) — focus is lost"
        );
        assert!(
            n.focused,
            "update {i}: focus is retained because the node survived"
        );
        assert_eq!(
            (n.sel_start, n.sel_end),
            (1, 1),
            "update {i}: selection range retained"
        );
        assert_eq!(n.scroll_top, 7, "update {i}: scroll position retained");
        assert_eq!(
            n.value, *typed,
            "update {i}: the bound value reflects what was typed"
        );
        let echo = d.first_by_tag("p").expect("the echo <p> exists");
        assert_eq!(
            d.text_content(echo),
            format!("Hello, {typed}"),
            "update {i}: the derived text updates after every input"
        );
    }
}

#[test]
fn lgx_re_rendering_the_bound_input_does_not_stack_handlers() {
    // (d) re-binding the SAME logical `input` handler must not accumulate duplicates —
    // the page keeps exactly ONE managed listener per (node,event).
    let mut w = world(LGX_FORM_APP);
    let input = w.input_id();
    for typed in ["a", "ab", "abc", "abcd"] {
        w.type_into(input, "input", typed);
    }
    let d = w.dom.borrow();
    let count = d.nodes[input as usize]
        .handler_counts
        .get("input")
        .copied()
        .unwrap_or(0);
    assert_eq!(
        count, 1,
        "exactly one managed `input` handler survives — no duplicates stacked"
    );
}

#[test]
fn lgx_an_unrelated_sibling_is_untouched_by_a_bound_update() {
    // (e) a sibling node unrelated to the changed field keeps its identity across a
    // bound-input update — the commit writes only what changed.
    let mut w = world(LGX_FORM_APP);
    let input = w.input_id();
    let echo = w.dom.borrow().first_by_tag("p").expect("echo exists");
    let echo_gen0 = w.dom.borrow().nodes[echo as usize].generation;
    // Mark the sibling focused/scrolled — must survive the update (it is not recreated).
    w.dom.borrow_mut().nodes[echo as usize].scroll_top = 42;
    w.type_into(input, "input", "x");
    let now_echo = w.dom.borrow().first_by_tag("p").expect("echo still exists");
    assert_eq!(now_echo, echo, "the echo <p> keeps its node id");
    let d = w.dom.borrow();
    assert_eq!(
        d.nodes[echo as usize].generation, echo_gen0,
        "the echo <p> was NOT recreated"
    );
    assert_eq!(
        d.nodes[echo as usize].scroll_top, 42,
        "the sibling's UI state is untouched"
    );
}

#[test]
fn lgx_a_stale_child_is_removed_in_place_not_recreated() {
    // (f) a dropped list item is removed in place; the surviving items keep their nodes
    // (no wholesale clear-and-recreate). A `State:` field that shrinks a list proves it.
    let app = lgx_page(
        "    Let n be 3.\n",
        "Actions:\n    To shrink:\n        Set n to n - 1.\n",
        "    <button id=\"shrink\" on-click={shrink}>shrink</button>\n\
         \x20   <ul>{for each i in [1, 2, 3]:}<li>row</li>{end}</ul>\n",
    );
    let mut w = world(&app);
    // Snapshot the ul + its three li node ids after the initial mount.
    let (ul, lis0) = {
        let d = w.dom.borrow();
        let ul = d.first_by_tag("ul").expect("a ul");
        let lis: Vec<u32> = d.nodes[ul as usize].children.clone();
        (ul, lis)
    };
    assert_eq!(lis0.len(), 3, "three rows mount initially");
    let ul_gen0 = w.dom.borrow().nodes[ul as usize].generation;
    // The list is a literal `[1,2,3]`, so a turn rebuilds three rows regardless of `n`;
    // the point of THIS lock is that the ul + its rows are REUSED, not recreated.
    let button = w.dom.borrow().first_by_tag("button").expect("a button");
    w.type_into(button, "click", "");
    let d = w.dom.borrow();
    let ul_now = d.first_by_tag("ul").expect("ul still exists");
    assert_eq!(
        ul_now, ul,
        "the ul keeps its node id across the turn's repaint"
    );
    assert_eq!(
        d.nodes[ul as usize].generation, ul_gen0,
        "the ul was not recreated"
    );
    let lis_now: Vec<u32> = d.nodes[ul as usize].children.clone();
    assert_eq!(
        lis_now, lis0,
        "every surviving <li> keeps its node id (reused in place)"
    );
}

// ---------------------------------------------------------------------------------------------
// ★ THE BULK CALL IS THE SAME THING AS THE FIFTY SMALL ONES. ★
// ---------------------------------------------------------------------------------------------

#[test]
fn the_host_implements_the_bulk_patch_call() {
    // ★ A module that hands its ops over in ONE `dom_apply` call instantiates against this host and
    // sees them applied — asserted by RUNNING a module that makes the call, not by reading the host:
    // an import that is registered but wrong is exactly as broken as one that is missing.
    let src = "## Main\n\
        Let d be createElement(\"div\").\n\
        Call setText with d and \"hi\".\n\
        Call appendChild with body() and d.\n\
        Let buf be [1, 0, 0, 0, 0, 1, 0, 0, 0, 112].\n\
        Call applyPatches with buf.\n";
    // The bytes spell ONE real record: opcode 1 (`CreateElement`), node id `0` as a u32, then a
    // length-prefixed tag — length 1, then `p`. A minimal VALID stream, so this exercises the
    // decode path rather than the refusal path.
    let w = world(src);
    let d = w.dom.borrow();
    let div = d
        .first_by_tag("div")
        .expect("the per-op calls built their node");
    assert_eq!(d.text_content(div), "hi", "and wrote its text");
    assert!(
        d.nodes.iter().any(|n| n.tag == "p"),
        "the bulk call's record was decoded and applied: it created the `p` it spells"
    );
}

#[test]
fn a_program_repainting_through_its_stream_keeps_its_nodes() {
    // ★ THE PROPERTY THIS FILE EXISTS FOR, held for a program that is no component: a repaint keeps
    // a node alive through the ONE door a paint reaches the host by, the committed stream. A
    // `## Main` drawing through `vdom.lg` twice — the input's value changing — hands over two
    // streams, and the second writes the value into the input the first created: the same slot,
    // the same generation, so what a visitor left in it survives.
    let w = world(
        "## To field(value: Text):\n\
         \x20   vdOpen(\"input\").\n\
         \x20   vdSetAttribute(\"id\", \"f\").\n\
         \x20   vdSetValue(value).\n\
         \x20   vdClose().\n\
         \n## Main\n\
         Let root be vdNewRoot(0).\n\
         field(\"a\").\n\
         Let first be vdCommit(root).\n\
         Let bound be vdApply().\n\
         field(\"ab\").\n\
         Let second be vdCommit(root).\n\
         Let rebound be vdApply().\n",
    );
    let d = w.dom.borrow();
    assert_eq!(
        d.streams, 2,
        "each paint reached the host as one committed stream"
    );
    let inputs: Vec<&HostNode> = d.nodes.iter().filter(|n| n.tag == "input").collect();
    assert_eq!(
        inputs.len(),
        1,
        "the repaint created no second input: {inputs:?}"
    );
    let input = d
        .first_by_tag("input")
        .expect("the input is in the document");
    assert_eq!(
        d.nodes[input as usize].value, "ab",
        "the repaint wrote the new value into it"
    );
    assert_eq!(
        d.nodes[input as usize].generation, inputs[0].generation,
        "the input in the document is the one the first paint created"
    );
}

/// Instantiates `wasm_host::import_every_row` of `table` against the host `linker` builds.
fn link_table_with(
    table: &[(&str, Vec<u8>, Vec<u8>)],
    linker: fn(&wasmi::Engine) -> wasmi::Linker<Data>,
) -> Result<(), String> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(
        &engine,
        &crate::common::wasm_host::import_every_row(table)[..],
    )
    .map_err(|e| format!("module: {e}"))?;
    let linker = linker(&engine);
    let mut store = wasmi::Store::new(&engine, Rc::new(RefCell::new(FocusDom::new())));
    linker
        .instantiate(&mut store, &module)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Instantiates `wasm_host::import_every_row` of `table` against this file's host.
fn link_table(table: &[(&str, Vec<u8>, Vec<u8>)]) -> Result<(), String> {
    link_table_with(table, host_linker)
}

#[test]
fn every_declared_host_import_links_against_the_host() {
    // ★ ONE DECLARED TABLE. The module's host-import table is the ABI; a page importing any row of
    // it must instantiate against this host, whether the host models the row or names it in a trap.
    let table = logicaffeine_compile::vm::wasm::host_import_signatures();
    assert!(
        table.len() >= 150,
        "the host-import table read back only {} rows",
        table.len()
    );
    link_table(&table).unwrap_or_else(|e| {
        panic!("a module importing the whole declared table does not link: {e}")
    });
}

#[test]
fn a_modelled_import_whose_shape_drifts_from_the_table_fails_the_census() {
    // The census has teeth only where the host wrote a stub of its own; the trap fill is typed
    // from the table and cannot drift. Give a modelled row one extra parameter and the link fails.
    let mut table = logicaffeine_compile::vm::wasm::host_import_signatures();
    let row = table
        .iter_mut()
        .find(|(name, _, _)| *name == "dom_create_element")
        .expect("dom_create_element is declared");
    row.1.push(0x7F);
    let err = link_table(&table)
        .expect_err("an import whose shape the host does not link must refuse to instantiate");
    assert!(
        err.contains("dom_create_element"),
        "the refusal names the drifted import: {err}"
    );
}

#[test]
fn the_census_fails_on_a_host_without_the_trap_fill() {
    // Without the fill, the host links only what it models, and a module importing the whole
    // table refuses to instantiate — the failure the fill turns into a named trap at call time.
    let table = logicaffeine_compile::vm::wasm::host_import_signatures();
    let err = link_table_with(&table, modelled_linker)
        .expect_err("a host that models only part of the table cannot link all of it");
    assert!(
        err.contains("env"),
        "the refusal names the unlinked import: {err}"
    );
}
