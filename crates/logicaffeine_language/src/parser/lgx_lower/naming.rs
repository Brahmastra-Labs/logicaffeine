/// The name of a component's render function: `__render_<Name>`. The VM builds the same name
/// from a `Mount`, so the two sides agree without a table between them.
pub fn render_fn_name(component: &str) -> String {
    format!("__render_{component}")
}

/// `__View` — the enum a page program's View values are: one variant per `-> View` template,
/// carrying that template's arguments (`lgx_lower::views`). Registered by the discovery pass, so
/// every engine is handed it.
pub fn view_type_name() -> &'static str {
    "__View"
}

/// `__View_<f>` — the variant a call of template `f` is.
pub fn view_variant_name(template: &str) -> String {
    format!("__View_{template}")
}

/// `__View_none` — the empty view: what a View prop holds before anything is handed to it.
pub fn empty_view_variant_name() -> &'static str {
    "__View_none"
}

/// `__view_<f>(depth, scope, params…)` — template `f`'s nodes drawn onto the tape, `depth` views
/// deep.
pub fn view_draw_fn_name(template: &str) -> String {
    format!("__view_{template}")
}

/// `__lgx_place_view(view, scope, depth)` — draw whichever template a View value is a call of.
pub fn place_view_fn_name() -> &'static str {
    "__lgx_place_view"
}

/// The parameter a drawing takes how deep it is nested in its own kind in: a template among
/// templates (`__view_<f>`), a served component among served components (`__serve_<C>`). Past
/// `RENDER_DEPTH_LIMIT` the drawing fails in the render-depth words.
pub fn draw_depth_param() -> &'static str {
    "__lgx_depth"
}

/// The parameter a template's drawing takes the placing component's scope class in (`""` when
/// that component has no `Style:`).
pub fn view_scope_param() -> &'static str {
    "__lgx_scope"
}

/// The parameter [`place_view_fn_name`] takes the View value in.
pub fn view_value_param() -> &'static str {
    "__lgx_view"
}

/// The module binding that remembers WHERE one updatable site's node landed.
///
/// The compiled half of Stage B: the render function writes each patchable site's node into its
/// own global, and a per-turn patcher writes new values into those nodes — so a turn that moves
/// one field costs one `setText` instead of clearing the mount and rebuilding the subtree.
///
/// The name is derived from the site's `SiteDeps::key()` (`text@0.1` → `__SITE_App_text_0_1`), so
/// the dependency walk, the render walk and the patcher all address a site by ONE string.
pub fn site_binding(component: &str, key: &str) -> String {
    let sanitized: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("__SITE_{component}_{sanitized}")
}

/// The per-turn patcher: `__patch_<Component>_<turn>`.
pub fn patch_fn_name(component: &str, turn: &str) -> String {
    let sanitized: String = turn
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("__patch_{component}_{sanitized}")
}

/// The module-level binding a component's state field lives in. Prefixed with the component's
/// name so two components' `n` are two different bindings.
/// The parameter every instance-aware synthesized function carries: WHICH instance it is acting
/// for. A component's state is a map from this key to that instance's value, so two placements of
/// one component keep separate records, each instance addressed by its key path.
///
/// The ROOT instance's key is the empty string, so a component placed once — every page that
/// worked before — is the general case with a constant key rather than a second mechanism.
pub fn instance_key_param() -> &'static str {
    "__lgx_key"
}

pub fn state_binding(component: &str, field: &str) -> String {
    format!("__S_{component}_{field}")
}

/// The class a component's `Style:` is scoped to, and which its own elements carry. The server
/// walk stamps the same class, or the two would render the same page with different class lists.
/// The module binding holding whether this component's stylesheet has been emitted in the CURRENT
/// paint. A `Style:` block is one `<style>` per component TYPE per render pass, however many
/// instances render: the first instance to render claims it, and a new paint clears the claims.
pub fn styled_binding(component: &str) -> String {
    format!("__STYLED_{component}")
}

pub fn scope_class(component: &str) -> String {
    format!("lgx-{}", component.to_ascii_lowercase())
}

/// `__html_<Component>()` — the server-rendered markup of a component's view.
/// The parameter a rendered component reads its `{children}` out of in a BROWSER: the node
/// holding the caller's children, or `0` for a placement that gave none.
///
/// ★ A HANDLE, WHERE THE SERVER PASSES A STRING. Both tiers render the children in the CALLER's
/// scope and hand the finished thing down — a server can finish markup into text, and a browser
/// finishes it into nodes, so the same rule takes the shape each tier has for "already drawn".
pub fn render_slot_param() -> &'static str {
    "__lgx_slot"
}

/// The parameter a layout's render reads its `{block <name>}` out of: the node holding what the
/// extending view drew for that block in its own scope, or `0` when it filled none and the layout
/// draws its default.
pub fn block_slot_param(block: &str) -> String {
    format!("__lgx_block_{block}")
}

/// `__html_<C>()` — component `C`'s markup, as `html of C` asks for it: `C` drawn by its server
/// render into a root no host holds, serialized, and the root discarded.
pub fn html_fn_name(component: &str) -> String {
    format!("__html_{component}")
}

/// `__serve_<C>(props…, slot, blocks…)` — component `C` drawn onto the tape to be serialized: the
/// same walk as its `__render_<C>` over its initial state, writing only the records its markup is
/// made of (`LowerTrail::serving`).
pub fn serve_fn_name(component: &str) -> String {
    format!("__serve_{component}")
}

/// Is this a name the COMPILER wrote, rather than one the author typed?
///
/// ★ AN AUTHOR CANNOT ACT ON A DIAGNOSTIC ABOUT CODE THEY DID NOT WRITE. Every LGX page lowers
/// into synthesized functions — a dispatcher, a render, a turn — and an analysis that runs over
/// the lowered program will happily report on those. It did: opening any page with a handler put
/// two `use-after-move` errors in the editor about `__lgx_sel`, a dispatcher PARAMETER, spanning
/// the whole file, advising the author to "Use Show instead". A name they have never seen, at a
/// place they cannot look, about a transfer they did not write.
///
/// The rule lives HERE, beside the functions that mint the names, for the same reason
/// `is_browser_only_lowering` does: a consumer matching strings of its own would be a second
/// spelling of a decision only this module makes, and it would fall out of step the first time a
/// new family is added. This is the WIDER question of the two — `__html_` is compiler-written and
/// is not browser-only — so the two predicates are separate on purpose.
pub fn is_compiler_written(name: &str) -> bool {
    // ★ ONE MARK, NOT A LIST OF FAMILIES.
    //
    // The first version of this enumerated the prefixes — `__lgx_`, `__render_`, `__act_`, and so
    // on — and was ALREADY INCOMPLETE when it was written: the lowering also mints `__ARG_`,
    // `__DUE_`, `__SITE_`, `__STYLED_`, `__Store_`, `__error_page_`, `__middleware_`, and bare
    // temporaries like `__out` and `__item` that begin no family at all. An enumeration of a set
    // that grows every time a construct lands is a list that is wrong between the two commits, and
    // wrong SILENTLY — the symptom is a false error in someone's editor, not a failing test.
    //
    // Every name this module mints begins `__`, and nothing an author writes does: the whole
    // stdlib contains no `__` identifier. So the leading underscores ARE the mark, and a new
    // construct inherits the answer by spelling its temporaries the way every other one does.
    name.starts_with("__")
}

/// ★ WHICH SYNTHESIZED FUNCTIONS NEED A DOCUMENT — the one place the answer lives.
///
/// A component lowers into two families. `__html_<Name>` builds a string out of `htmlEscape` and
/// `+`, which is ordinary code on any tier. `__render_`, `__act_`, `__mount_`, `__bind_`,
/// `__patch_`, `__dir_`, `__cb_` and the `__lgx_*` dispatcher and page drivers call `createElement` and `appendChild` —
/// a browser's document, which a server binary does not have.
///
/// Emitting the second family into AOT-Rust is what made the component model look unshippable, and
/// the blanket refusal that followed meant **per-request server rendering did not exist in
/// production**: markup could only be produced at BUILD time by pre-rendering, so nothing whose
/// content depends on the request could be server-rendered at all.
///
/// The names are MINTED HERE, by the functions directly above, so this is where the question is
/// answerable without guessing. A backend asking "is this browser-only?" by matching strings of
/// its own would be a second spelling of a decision only this module makes.
pub fn is_browser_only_lowering(name: &str) -> bool {
    // The per-COMPONENT families. Each is `<prefix><Component>[_<member>]`, so a prefix is the
    // whole rule and it cannot be extended by accident — a new one would need a new prefix here.
    const NEEDS_A_DOCUMENT: &[&str] = &[
        "__render_",
        "__act_",
        "__mount_",
        "__bind_",
        "__patch_",
        "__dir_",
        "__cb_",
        "__join_",
        "__publish_",
        "__refresh_",
        "__sh_",
    ];
    if NEEDS_A_DOCUMENT.iter().any(|p| name.starts_with(p)) {
        return true;
    }
    // ★ THE `__lgx_` FAMILY IS ENUMERATED, NOT PREFIX-MATCHED, AND THE DEFAULT IS "EMIT IT".
    //
    // This was `name.starts_with("__lgx_")` with ONE exception for `__lgx_route_handler`, and it
    // broke the moment the router grew: `__lgx_route_for`, `__lgx_route_status_for`,
    // `__lgx_route_ctype_for`, `__lgx_route_headers_for` and `__lgx_route_response_for` were all
    // skipped from the server binary while `routeWith` and friends still CALLED them, so **every
    // program with a `## Route` failed to build** with "cannot find function `__lgx_route_for`".
    // Nothing caught it on the tree-walker or the VM, which resolve lazily; only rustc says so.
    //
    // The two sets are not symmetric, and that is what decides the shape. The browser-only names
    // are a CLOSED set — the dispatcher, the listener table's answer and retirement, the
    // page-level drivers, each minted by a named function just below — while the server-side
    // `__lgx_` names are an
    // OPEN family that grows whenever the router learns a new question. Enumerating the closed set
    // and defaulting to EMIT means a new router walk works with no edit here; a new page-level
    // entry needs one line, next to the function that names it.
    [
        dispatch_fn_name(),
        answer_fn_name(),
        forget_listener_fn_name(),
        own_frag_fn_name(),
        gone_fn_name(),
        drain_directives_fn_name(),
        commit_fn_name(),
        raise_fn_name(),
        tick_fn_name(),
        rearm_fn_name(),
        on_failure_fn_name(),
        route_changed_fn_name(),
        repaint_root_fn_name(),
        shared_arrival_fn_name(),
    ]
    .contains(&name)
}

/// The binding holding where a component last rendered, so a turn can repaint it.
/// `__bind_<Component>_<field>(v)` — the write-back a bound input performs.
///
/// A FUNCTION rather than statements inlined into the listener closure, because a closure
/// captures the names it touches and a captured state binding is a COPY: the write would land on
/// the copy and the page would never change. A function resolves the module binding directly,
/// which is the same reason the dispatcher's `bind_arm` has always worked.
pub fn bind_fn_name(component: &str, field: &str) -> String {
    format!("__bind_{component}_{field}")
}

/// The module binding holding a component's PROP values, per instance.
///
/// ★ A REPAINT MUST PASS THE PROPS AGAIN. A turn redraws the instance whose state moved by calling
/// its render function, and that function takes the props its parent handed it — which the turn does
/// not have. Recorded here at render time and read back at repaint, per instance, exactly as state
/// and sites are.
pub fn prop_binding(component: &str, prop: &str) -> String {
    format!("__P_{component}_{prop}")
}

/// The module binding holding a CALLBACK PROP's own identity, per instance: which `(component,
/// action)` pair to run when the prop is called, as that pair's tag (`events.rs::action_tag`) —
/// never the pair itself, because the two scalars a map CAN hold are an index and a key, never a
/// function reference.
///
/// ★ NOT [`prop_binding`]'S MAP. `__P_<Component>_<prop>` holds the prop's VALUE, and a callback
/// prop's value is a closure — the one shape the wasm backend refuses inside a map, measured
/// (`events.rs::ListenerSite`'s own doc comment). This binding never holds the closure: it holds
/// which action the closure at THIS instance currently means, so a driving verb — which cannot
/// call a closure it found at run time — can still resolve and run it.
pub fn callback_tag_binding(component: &str, prop: &str) -> String {
    format!("__P_{component}_{prop}_TAG")
}

/// The twin of [`callback_tag_binding`]: whose own instance the tag's action runs against — the
/// placing component's key, recorded beside the tag so the two travel together.
pub fn callback_key_binding(component: &str, prop: &str) -> String {
    format!("__P_{component}_{prop}_KEY")
}

/// Where a callback prop's own declared parameters are kept, per instance — the twin of
/// [`listener_arg_binding`] for a handler whose target varies by placement instead of being fixed
/// at the component's own declaration. Typed by the prop's own function type, which every action
/// bound to it is required to match exactly, so one map serves every target the tag can name.
pub fn callback_arg_binding(component: &str, prop: &str, index: usize) -> String {
    format!("__ARG_{component}_{prop}_cb{index}")
}

/// The page's OUTBOX: the element events turns have asked for and not yet had delivered — how many
/// ([`outbox_count_binding`]) and, at positions `1..=count`, three parallel `Int`-keyed maps of
/// selector, event and carried value — appended where the asking statement RUNS and emptied by the
/// flush that follows a turn's write-back.
pub fn outbox_count_binding() -> &'static str {
    "__LGX_OUTBOX_N"
}

pub fn outbox_selector_binding() -> &'static str {
    "__LGX_OUTBOX_SEL"
}

pub fn outbox_event_binding() -> &'static str {
    "__LGX_OUTBOX_EV"
}

pub fn outbox_value_binding() -> &'static str {
    "__LGX_OUTBOX_VAL"
}

/// The reserved event a turn's epilogue dispatches to flush the outbox (`lower_lgx_dispatcher`'s
/// first arm).
pub fn outbox_flush_event() -> &'static str {
    "__lgx_outbox"
}

/// `__cb_<Component>_<prop>(key, a…)` — the one function every call of a callback prop goes
/// through: it reads the instance's recorded identity and runs the action it names.
pub fn callback_trampoline_fn_name(component: &str, prop: &str) -> String {
    format!("__cb_{component}_{prop}")
}

/// Where a registered listener's ARGUMENTS are kept, per instance.
///
/// ★ AN ARGUMENT IS EVALUATED WHERE THE LISTENER IS ATTACHED, not where it is dispatched.
/// `on-click={remove(todo's id)}` names a loop item that exists only inside the render — the
/// dispatcher has no `todo` and never will. So the VALUE is recorded at registration, keyed by the
/// same element+event, and typed by the action's own declared parameter.
pub fn listener_arg_binding(component: &str, action: &str, index: usize) -> String {
    format!("__ARG_{component}_{action}_{index}")
}

/// `__LISTENER_SPACE` — how many listener entries (event names, and event-and-key pairs) the program
/// registers. A listener's SLOT is `node * __LISTENER_SPACE + code`, so every (element, entry) pair
/// has a slot of its own however many entries the program has; declared once every entry is known.
pub fn listener_space_binding() -> &'static str {
    "__LISTENER_SPACE"
}

/// `__LISTENER_FN` — the site each registered listener runs, by its lookup: its slot for the first
/// listener waiting there, a spilled lookup ([`listener_spill_binding`]) for each one after it.
pub fn listener_fn_binding() -> &'static str {
    "__LISTENER_FN"
}

/// `__LISTENER_MORE` — the lookups of every listener past the first waiting on one slot, in the
/// order they registered: one element can wait for one event several times over (a directive and a
/// sentence, two sentences, two instances naming it), and each answers. Empty unless a slot has
/// several listeners, so the common slot costs nothing here.
pub fn listener_spill_binding() -> &'static str {
    "__LISTENER_MORE"
}

/// `__LISTENER_MORE_AT` — the slot each spilled listener waits on, so retiring it finds its list.
pub fn listener_spill_slot_binding() -> &'static str {
    "__LISTENER_MORE_AT"
}

/// `__LISTENER_SPILLED` — how many listeners have ever spilled. The n-th spilled listener's lookup is
/// `-1 - n`: below every slot (slots are never negative), never reused, and never
/// [`UNUSED_LOOKUP`], which the listener maps are seeded with.
pub fn listener_spill_count_binding() -> &'static str {
    "__LISTENER_SPILLED"
}

/// The lookup no listener ever has: the seed entry of every map keyed by lookup, which types the
/// map's key and is never looked up.
pub const UNUSED_LOOKUP: i64 = -1;

/// `__LISTENER_HELD` — how many deliveries in flight hold each lookup. A delivery fixes the
/// listeners it will run before running any, as a browser does, so a listener whose element an
/// earlier listener's turn redrew still answers: retiring a held lookup waits for its last holder.
pub fn listener_held_binding() -> &'static str {
    "__LISTENER_HELD"
}

/// `__LISTENER_LEFT` — the held lookups retired while held, forgotten when their last holder lets go.
pub fn listener_left_binding() -> &'static str {
    "__LISTENER_LEFT"
}

/// `__lgx_forget_listener(lookup)` — retire one registration: its entry in every map keyed by
/// lookup and its place in a spilled slot's list; or, while a delivery holds it, mark it to go when
/// the delivery lets go.
pub fn forget_listener_fn_name() -> &'static str {
    "__lgx_forget_listener"
}

/// `__LGX_ONCE` — the identity of every `.once` listener that has answered, mapped to `true`. An
/// identity is the listener's author-level address — its instance key, its static site, and its row
/// in every enclosing `{for each}` — so it survives a repaint that draws the element again with a
/// fresh node, and a spent listener stays spent however its element is redrawn.
pub fn once_spent_binding() -> &'static str {
    "__LGX_ONCE"
}

/// `__LISTENER_ONCE` — the `.once` identity of each registered listener, keyed like
/// [`listener_fn_binding`], so the dispatcher can spend the listener it resolved.
pub fn listener_once_binding() -> &'static str {
    "__LISTENER_ONCE"
}

/// `__ONCE_<C>` — every `.once` identity an instance of `C` has spent, per instance key, so the
/// unmount sweep can forget them when the instance leaves and a remount starts with its listeners
/// armed.
pub fn once_record_binding(component: &str) -> String {
    format!("__ONCE_{component}")
}

/// The instance key each registered listener belongs to — the twin of [`listener_fn_binding`].
///
/// Two maps rather than a map of records because they are written at one site and read at one site,
/// so they cannot drift, and a record would need a synthesized type for no gain.
pub fn listener_key_binding() -> &'static str {
    "__LISTENER_KEY"
}

/// `__LISTENER_SITES_<C>` — every listener lookup THIS instance registered in its LAST paint.
///
/// ★ A REPAINT CREATES FRESH NODE IDS, so its predecessor's listener keys are dead the moment
/// `clearChildren` runs — but `__LISTENER_FN`/`__LISTENER_KEY` never learned that: they only ever
/// gain entries, one per element per paint, for the life of the page. A list that repaints its rows
/// a thousand times leaks a thousand rows of dead listener bookkeeping it will never look up again.
/// This is the record a render checks at its own top (has THIS instance key painted before?) to
/// retire what it registered last time before it registers what it is about to.
pub fn listener_sites_binding(component: &str) -> String {
    format!("__LISTENER_SITES_{component}")
}

/// The attribute a pre-rendered document's `<body>` carries: its markup is the page's own first paint,
/// which the mount adopts (`vdHydrate`) rather than clearing and drawing again. Written by the build's
/// pre-render, read by the mount, and taken off by the host as the hydration opens.
pub const PRERENDERED_ATTR: &str = "data-logos-prerendered";

/// Every `vdom.lg` function a lowered page calls — the tape it writes, the commit, the sites and
/// fragments it finds its nodes by. None of them appears in the author's source, so the prelude
/// splices vdom.lg for a page and the pruner keeps exactly these as roots (`loader.rs`);
/// `vdom_calls_lock` holds this list to the calls the lowering writes.
pub const VDOM_CALLS: &[&str] = &[
    "vdAbort",
    "vdAnchor",
    "vdApply",
    "vdClearGone",
    "vdClose",
    "vdCommit",
    "vdDiscardRoot",
    "vdEnsureSite",
    "vdFragClose",
    "vdFragNode",
    "vdFragOpen",
    "vdFreeSite",
    "vdGone",
    "vdGoneCount",
    "vdHydrate",
    "vdListen",
    "vdListenAt",
    "vdNewFrag",
    "vdNewRoot",
    "vdNewSite",
    "vdNodeOfWire",
    "vdOpen",
    "vdOpenKeyed",
    "vdOpenSoft",
    "vdPatchChecked",
    "vdPatchText",
    "vdPatchValue",
    "vdQueryText",
    "vdRangeClose",
    "vdRangeOpen",
    "vdReleaseAnchors",
    "vdSerializeChildren",
    "vdSetAttribute",
    "vdSetChecked",
    "vdSetHtml",
    "vdSetValue",
    "vdSite",
    "vdSiteAt",
    "vdSiteNode",
    "vdSiteOf",
    "vdSlot",
    "vdTapeLength",
    "vdTapeSite",
    "vdTapeStash",
    "vdTapeUnstash",
    "vdText",
    "vdTextSite",
    "vdWire",
];

/// A `vdom.lg` path only some pages take: every `vdom.lg` function named with `prefix` runs only on a
/// tape holding records one of `writers` laid down, so the module's dead-code analysis follows a
/// call into it once a writer is reached (`vm::reach`) — a page that never calls a writer ships none
/// of the feature. A new function of the feature is pruned by being named for it.
#[derive(Debug, Clone, Copy)]
pub struct VdomFeature {
    pub prefix: &'static str,
    pub writers: &'static [&'static str],
}

/// Every [`VdomFeature`]: firm-key checks, matching by key, range resolution, hydration.
pub const VDOM_FEATURES: &[VdomFeature] = &[
    VdomFeature {
        prefix: "vdKeyed",
        writers: &["vdOpenKeyed"],
    },
    VdomFeature {
        prefix: "vdKeys",
        writers: &["vdOpenKeyed", "vdOpenSoft"],
    },
    VdomFeature {
        prefix: "vdSlots",
        writers: &["vdRangeOpen", "vdSlot"],
    },
    VdomFeature {
        prefix: "vdAdopt",
        writers: &["vdHydrate"],
    },
];

/// The render function's parameter holding the fragment its instance is drawn as — what the sites
/// its paint hands out are kept against (`vdAnchor`).
pub fn render_frag_param() -> &'static str {
    "__lgx_frag"
}

/// The render function's local holding where its own records start on the tape — what a sentence
/// asks for its element from.
pub(crate) fn tape_from_local() -> &'static str {
    "__lgx_from"
}

/// `__LGX_FRAGC` — the component (its position among the lowered components, from 1) each
/// fragment id is an instance of; `0` for an id no instance holds.
pub fn frag_owner_binding() -> &'static str {
    "__LGX_FRAGC"
}

/// `__LGX_FRAGK` — the instance key each fragment id is drawn for.
pub fn frag_key_binding() -> &'static str {
    "__LGX_FRAGK"
}

/// `__lgx_own_frag(frag, who, key)` — records which instance a fresh fragment id is drawn for.
pub fn own_frag_fn_name() -> &'static str {
    "__lgx_own_frag"
}

/// `__lgx_gone()` — the unmount sweep: every instance whose fragment the last commit removed loses
/// its registries, tells its `When this component disappears:` handler, and hands its sites back.
pub fn gone_fn_name() -> &'static str {
    "__lgx_gone"
}

/// `__lgx_commit(node)` — commits the tape into `node`'s retained children and hands the stream to
/// the host, or fails the turn with the commit's refusal and the document untouched.
pub fn commit_fn_name() -> &'static str {
    "__lgx_commit"
}

/// `__lgx_drain_directives()` — runs every directive occurrence the committed paint drew.
pub fn drain_directives_fn_name() -> &'static str {
    "__lgx_drain_directives"
}

/// `__DIRQS_<d>` — the sites of the elements directive `d` sits on, queued by the render for the
/// commit.
pub fn directive_queue_site_binding(directive: &str) -> String {
    format!("__DIRQS_{directive}")
}

/// `__DIRQV_<d>` — the value each queued occurrence of directive `d` is drawn with.
pub fn directive_queue_value_binding(directive: &str) -> String {
    format!("__DIRQV_{directive}")
}

/// `__DIRQK_<d>` — the identity of each queued occurrence of a directive with `When removed:`.
pub fn directive_queue_key_binding(directive: &str) -> String {
    format!("__DIRQK_{directive}")
}

/// `__LGX_HEARD_<C>_<i>` — whether the page already forwards what handler `i` of `C` waits for (a
/// window event, a relay topic) to its dispatcher; the forwarder is attached once per page.
pub fn forwarded_binding(component: &str, index: usize) -> String {
    format!("__LGX_HEARD_{component}_{index}")
}

/// The render function's own local accumulator: every listener key IT registers this paint,
/// written back to [`listener_sites_binding`] once, at the end, so next paint knows what to retire.
///
/// A local rather than a second write into the per-instance map at registration time: a render
/// walks its view once and registers as it goes, so collecting into a plain local and writing the
/// map ONCE is one map write per paint instead of one per listener.
pub(crate) fn paint_sites_local() -> &'static str {
    "__lgx_paint_sites"
}

/// `__SEEN_<C>` — the key of every instance of `C` that has ever rendered, mapped to the PAINT
/// PASS it last rendered in.
///
/// ★ THIS IS THE VDOM SIDE OF THE PAGE, and it is what makes `When this component disappears:`
/// answerable without asking the host anything. A paint stamps every instance it draws with the
/// current pass number; after the paint, an instance still carrying an older number was not drawn,
/// which means it left the page. The DOM says what is on screen and this map says what the program
/// believes is on screen — the two linked, so a diff between them IS the unmount set.
pub fn seen_binding(component: &str) -> String {
    format!("__SEEN_{component}")
}

/// `__KEYS_<C>` — every instance key of `C` that has ever rendered, in first-render order.
///
/// ★ IT EXISTS BECAUSE THE SWEEP MUST BE ITERABLE ON EVERY TIER. The seen-map answers "which pass
/// was this instance last drawn in" in one lookup, but WALKING a map is not something the wasm
/// backend lowers ("unsupported iteration over a non-sequence value"), and a sweep that ran on two
/// tiers and not the third would be the exact shape this campaign exists to prevent. A sequence
/// walks everywhere, so the keys live in one and the passes in the other.
pub fn keys_binding(component: &str) -> String {
    format!("__KEYS_{component}")
}

/// The key the per-instance maps are SEEDED with, purely to give themselves a type.
///
/// ★ IT MUST NOT BE A KEY ANY INSTANCE COULD HAVE, and the root key `""` is. Seeded with `""`, the
/// timer maps reported the ROOT instance as already known on its first render — so its countdown
/// was never armed and the page's clock never started, silently. An instance key is a path built
/// from component names, so a leading control character can never collide with one.
pub fn seed_key() -> &'static str {
    "\u{1}"
}

/// `__DUE_<C>_<i>` — the page time (`__LGX_NOW`'s clock) at which handler `i` of `C` next fires,
/// per instance. `-1` is "not running", which is what a `Stop the … timer.` writes and what a
/// one-shot leaves behind.
pub fn due_binding(component: &str, index: usize) -> String {
    format!("__DUE_{component}_{index}")
}

/// `__SEQ_<C>_<i>` — when handler `i` of `C` was armed on each instance, in arming order across the
/// page: two timers due at one moment fire in the order they were armed.
pub fn seq_binding(component: &str, index: usize) -> String {
    format!("__SEQ_{component}_{index}")
}

/// `__PERIOD_<C>_<i>` — handler `i` of `C`'s period in milliseconds on each instance, as its
/// sentence read when the instance was drawn: what a repeating timer re-arms by and what
/// `Start the timer …` restarts it from.
pub fn period_binding(component: &str, index: usize) -> String {
    format!("__PERIOD_{component}_{index}")
}

/// `__LGX_NOW` — the page's own clock, in milliseconds since the page started, moved only by
/// `__lgx_tick(ms)`.
pub fn now_binding() -> &'static str {
    "__LGX_NOW"
}

/// `__LGX_TIMER_SEQ` — how many timers the page has armed, the next arming's order.
pub fn timer_seq_binding() -> &'static str {
    "__LGX_TIMER_SEQ"
}

/// `__lgx_rearm()` — ask the host to wake the page when its earliest running timer comes due
/// (`wakeAfter`), or to wake it for none. Called after every arming, stop and start, and at the end
/// of every tick.
pub fn rearm_fn_name() -> &'static str {
    "__lgx_rearm"
}

/// `__LGX_FAIL_C` / `__LGX_FAIL_K` — WHICH turn is running right now: a component id and an
/// instance key, written at the top of every turn.
///
/// ★ THE HOST CANNOT KNOW THIS, AND DOES NOT HAVE TO. A trap unwinds to the host, which learns
/// only that something failed; it has no idea which component's action was mid-flight. The module
/// does, because it just wrote it down — so `When an action fails (e: Text):` is dispatched by
/// compiled LOGOS exactly like every other trigger, and the host's whole contribution is to say
/// "that trapped, here is the message".
pub fn failing_component_binding() -> &'static str {
    "__LGX_FAIL_C"
}

pub fn failing_key_binding() -> &'static str {
    "__LGX_FAIL_K"
}

/// `__LGX_OPEN_TURNS` — the journal names of the turns in progress, innermost last: a turn pushes
/// its name first and pops it just before it journals itself, so a trap leaves exactly the turns it
/// interrupted here, for the failure door to journal innermost-first.
pub fn open_turns_binding() -> &'static str {
    "__LGX_OPEN_TURNS"
}

/// `__LGX_OPEN_KEYS` — the journal instance of each open turn, beside its name in
/// [`open_turns_binding`].
pub fn open_keys_binding() -> &'static str {
    "__LGX_OPEN_KEYS"
}

/// `__LGX_CASCADE` — how many rounds of store watchers are running inside one another: a round
/// that has watchers to run enters before they run and leaves after, and a round past
/// [`crate::ast::component::STORE_CASCADE_LIMIT`] fails the page as one that never settles.
pub fn cascade_binding() -> &'static str {
    "__LGX_CASCADE"
}

/// `__LGX_CONTEXT` — what the page is doing that a trap should be named by (a directive's block),
/// in the words every tier puts in front of the trap's message; `""` outside any.
pub fn failure_context_binding() -> &'static str {
    "__LGX_CONTEXT"
}

/// `__lgx_failure_context() -> Text` — the context a trap left open, taken (the page is back
/// outside any once a host has read it).
pub fn failure_context_fn_name() -> &'static str {
    "__lgx_failure_context"
}

/// `__LGX_RENDER_DEPTH` — how many renders of components that can be drawn inside themselves are
/// running inside one another; one past [`crate::ast::component::RENDER_DEPTH_LIMIT`] fails the
/// paint as one with no finite rendering.
pub fn render_depth_binding() -> &'static str {
    "__LGX_RENDER_DEPTH"
}

/// `__LGX_APPEAR_WHO` — which component each owed `When this component appears:` belongs to, as its
/// place (from 1) among the components that have one; beside [`appear_key_binding`], one entry per
/// instance a paint drew for the first time, in the order it drew them.
pub fn appear_who_binding() -> &'static str {
    "__LGX_APPEAR_WHO"
}

/// `__LGX_APPEAR_KEY` — the instance each owed `When this component appears:` belongs to.
pub fn appear_key_binding() -> &'static str {
    "__LGX_APPEAR_KEY"
}

/// `__LGX_APPEAR_AT` — the next owed entry [`appear_fn_name`] runs.
pub fn appear_at_binding() -> &'static str {
    "__LGX_APPEAR_AT"
}

/// `__lgx_appear()` — every owed `When this component appears:`, first drawn first, each a turn of
/// its own; run where a paint ends, never inside one.
pub fn appear_fn_name() -> &'static str {
    "__lgx_appear"
}

/// `__MOVED_<Store>_<field>` — set by a store action whose commit moved a field some component
/// watches, and taken by the next store-watcher round ([`wake_stores_fn_name`]).
pub fn store_moved_binding(store: &str, field: &str) -> String {
    format!("__MOVED_{store}_{field}")
}

/// `__LGX_STORES_DIRTY` — set by every store action's commit and cleared by every repaint from the
/// page root: whether the page may still show a store as it was before a commit. A turn that moved
/// a store repaints from the root; one that trapped first never did, and the failure door repaints
/// for it.
pub fn stores_dirty_binding() -> &'static str {
    "__LGX_STORES_DIRTY"
}

/// `__lgx_wake_stores()` — one store-watcher round: take every moved-field flag, and run each
/// watcher of a field that moved once per live instance. Every turn ends with one.
pub fn wake_stores_fn_name() -> &'static str {
    "__lgx_wake_stores"
}

/// `__LGX_WATCH_CHANGES` — the changes the watching turn's action committed, as `noteTurn` takes
/// them: what the failure door journals for that turn when one of its handlers traps.
pub fn watch_changes_binding() -> &'static str {
    "__LGX_WATCH_CHANGES"
}

/// `__LGX_WATCHING` — which component's `When <field> changes:` handlers are running (its id among
/// the components that have them, `0` for none): its action has committed, so a trap there is
/// absorbed rather than undoing it. `__LGX_WATCH_K` is that instance's key, and
/// `__LGX_WATCH_DEPTH` how many turns were open when the handlers began (the watching turn's own
/// entry the last of them).
pub fn watching_binding() -> &'static str {
    "__LGX_WATCHING"
}

pub fn watch_key_binding() -> &'static str {
    "__LGX_WATCH_K"
}

pub fn watch_depth_binding() -> &'static str {
    "__LGX_WATCH_DEPTH"
}

/// `__lgx_on_failure(message)` — run the failure handler of whichever component was mid-turn. The
/// browser host calls it from the one place that catches.
pub fn on_failure_fn_name() -> &'static str {
    "__lgx_on_failure"
}

/// Who enters an [`LGX_ENTRIES`] function by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryHost {
    /// A page's host — the browser runtime and the in-process page host alike. A page's wasm module
    /// exports it and the dead-code walk roots at it.
    Page,
    /// Only the in-process host of a `## Test`: a verb with no browser counterpart.
    Test,
    /// A server's request loop, native or in-process.
    Server,
}

/// A function a host enters a compiled program by name.
#[derive(Clone, Copy, Debug)]
pub struct HostEntry {
    /// The function's name, or a family's prefix: every member is the prefix and a non-empty rest.
    pub name: &'static str,
    pub family: bool,
    pub host: EntryHost,
    /// Why a host enters it.
    pub why: &'static str,
}

/// ★ EVERY FUNCTION A HOST ENTERS BY NAME, ONCE. A page's wasm module exports its page entries under
/// these names and nothing else of the lowering's, the dead-code walk roots at them, `runtime.js`
/// enters the module only through them and the in-process page host calls only them —
/// `lgx_entry_abi_lock` reads each of those sides against this table.
pub const LGX_ENTRIES: &[HostEntry] = &[
    HostEntry {
        name: "__lgx_dispatch_node",
        family: false,
        host: EntryHost::Page,
        why: "The one event path: a host fires an event at the node it happened on.",
    },
    HostEntry {
        name: "__lgx_on_failure",
        family: false,
        host: EntryHost::Page,
        why: "A host that caught a trapped turn hands the page the failure's message.",
    },
    HostEntry {
        name: "__lgx_tick",
        family: false,
        host: EntryHost::Page,
        why: "The page clock: a host hands it the milliseconds that passed.",
    },
    HostEntry {
        name: "__lgx_route_changed",
        family: false,
        host: EntryHost::Page,
        why: "A host tells the page its location moved.",
    },
    HostEntry {
        name: "__lgx_owns",
        family: false,
        host: EntryHost::Page,
        why: "A host asks whether a path is this page's, to navigate in place rather than load a document.",
    },
    HostEntry {
        name: "__lgx_failure_context",
        family: false,
        host: EntryHost::Page,
        why: "A host that caught a trap asks what the page was doing when it trapped, to name it in the failure.",
    },
    HostEntry {
        name: "__lgx_shared",
        family: false,
        host: EntryHost::Page,
        why: "A host hands a shared store the frame another replica published on its topic.",
    },
    HostEntry {
        name: "__mount_",
        family: true,
        host: EntryHost::Page,
        why: "A host mounts a component into a parent node.",
    },
    HostEntry {
        name: "__lgx_raise",
        family: false,
        host: EntryHost::Test,
        why: "`Raise` delivers a named event to the page.",
    },
    HostEntry {
        name: "__act_",
        family: true,
        host: EntryHost::Test,
        why: "`Replay these actions:` runs a component's action by name.",
    },
    HostEntry {
        name: "route",
        family: false,
        host: EntryHost::Server,
        why: "A server asks the program's routes for a request's body.",
    },
    HostEntry {
        name: "routeStatus",
        family: false,
        host: EntryHost::Server,
        why: "A server asks the program's routes for a request's status.",
    },
    HostEntry {
        name: "routeContentType",
        family: false,
        host: EntryHost::Server,
        why: "A server asks the program's routes for a response's content type.",
    },
];

/// The [`LGX_ENTRIES`] row `name` enters by: its own row, or its family's.
pub fn host_entry_of(name: &str) -> Option<&'static HostEntry> {
    LGX_ENTRIES.iter().find(|e| match e.family {
        true => name.len() > e.name.len() && name.starts_with(e.name),
        false => name == e.name,
    })
}

/// `__lgx_tick(ms)` — the page's clock moves forward by `ms`, and every timer that time makes due
/// fires, in due order.
///
/// ★ ONE IMPLEMENTATION FOR EVERY TIER, WHICH IS THE WHOLE POINT. A browser has real timers and a
/// test has none, so the obvious build gives the two tiers different code and hopes they agree.
/// Instead the page keeps its own clock (`__LGX_NOW`) and every due time in compiled LOGOS, and the
/// tiers differ only in what calls this: a browser hands it the milliseconds that passed — before
/// every entry, and when `wakeAfter` says the earliest timer is due — and `Advance time by …` calls
/// it directly. A timer fires at the page time it was armed for, whatever steps the clock took.
pub fn tick_fn_name() -> &'static str {
    "__lgx_tick"
}

/// `__lgx_descends(k, under)` — whether instance key `k` was placed inside the instance keyed by
/// `under` (a key and its trailing `/`); what a repaint of a placed component sweeps within.
pub fn descends_fn_name() -> &'static str {
    "__lgx_descends"
}

/// The paint-pass counter every instance stamps itself with. One per program.
pub fn pass_binding() -> &'static str {
    "__LGX_PASS"
}

pub fn mount_binding(component: &str) -> String {
    format!("__M_{component}")
}

/// The program-wide event dispatcher: `__lgx_dispatch(selector, event)`. A driving verb on the
/// bytecode tier calls it, and it routes to the action the view attached — decided here, as data,
/// because a page has no way to call a closure it found at run time.
pub fn dispatch_fn_name() -> &'static str {
    "__lgx_dispatch"
}

/// The dispatcher's node half: `__lgx_dispatch_node(node, event, value)` — a host that already
/// holds the element (a focus change: the element that lost focus, the one that gained it) raises
/// an event on it without a selector naming it.
pub fn dispatch_node_fn_name() -> &'static str {
    "__lgx_dispatch_node"
}

/// `__lgx_answer(lookup, value) -> Bool` — run the listener one listener-table entry names, for the
/// instance that registered it; the node dispatcher asks it for the entry waiting for the pressed
/// key and for the one waiting for any.
pub fn answer_fn_name() -> &'static str {
    "__lgx_answer"
}

/// `__lgx_route_changed()` — deliver a move of the location to every live instance of every
/// component with a `When the route changes:` handler. The page registers it with the host once,
/// at start (`onRoute`), and a turn whose own `Navigate to` moved the location calls it at its
/// epilogue, so a placed component hears the move exactly as a mounted one does.
pub fn route_changed_fn_name() -> &'static str {
    "__lgx_route_changed"
}

/// `__lgx_owns(path) -> Bool` — whether this module's `## Page` serves `path` (its query and fragment
/// set aside): the one rule deciding whether a move to `path` stays in the page (`navigate`, a
/// pushState) or loads another document (`assignLocation`). `Navigate to` branches on it, and a
/// host's link-following asks it.
pub fn owns_fn_name() -> &'static str {
    "__lgx_owns"
}

/// `__LGX_MOVED` — set when a turn moved the location within the page, and taken by that turn's
/// epilogue, which delivers the move to the page's route listeners.
pub fn moved_binding() -> &'static str {
    "__LGX_MOVED"
}

/// `__lgx_repaint_root()` — repaint the page from its root (the instance keyed `""`), the answer to
/// a change any component may show: what a turn that moved a store paints, and what a shared store
/// runs when another replica's write arrives.
pub fn repaint_root_fn_name() -> &'static str {
    "__lgx_repaint_root"
}

/// The storage area a `shared on` store's fields live in, keyed `"<topic>.<field>"`: the topics
/// every page on them shares.
pub const SHARED_AREA: &str = "shared";

/// Whether the shared-store replica `store` has joined its topic.
pub fn shared_joined_binding(store: &str) -> String {
    format!("__SJ_{store}")
}

/// `__join_<S>()` — the replica joins its topic: it adopts what the topic holds and publishes
/// the rest. Runs before the first statement that reads one of its fields.
pub fn shared_join_fn_name(store: &str) -> String {
    format!("__join_{store}")
}

/// `__publish_<S>()` — publish every field the replica's action moved, then refresh the topic's
/// other joined replicas.
pub fn shared_publish_fn_name(store: &str) -> String {
    format!("__publish_{store}")
}

/// `__refresh_<S>()` — a joined replica adopts what its topic holds.
pub fn shared_refresh_fn_name(store: &str) -> String {
    format!("__refresh_{store}")
}

/// `__lgx_shared(topic, key, frame)` — the relay's entry: a frame another page published.
pub fn shared_arrival_fn_name() -> &'static str {
    "__lgx_shared"
}

/// This page's Lamport clock for its shared stores.
pub fn shared_clock_binding() -> &'static str {
    "__SH_CLOCK"
}

/// This page's replica id, drawn once per page load by a page with a convergent shared field: the
/// part of every tag and node id that no other page's can share.
pub fn shared_replica_binding() -> &'static str {
    "__SH_REPLICA"
}

/// The plain value a convergent field of shared store `store` last adopted — what the next
/// action's change to `field` is read against.
pub fn shared_base_binding(store: &str, field: &str) -> String {
    format!("__SB_{store}_{field}")
}

/// The node id behind each element of shared sequence `field` of `store`, in its order.
pub fn shared_ids_binding(store: &str, field: &str) -> String {
    format!("__SI_{store}_{field}")
}

/// `Raise "offline".` on the bytecode tier.
pub fn raise_fn_name() -> &'static str {
    "__lgx_raise"
}

/// The record type a `## Store` lowers to. A store is a singleton with named fields, which is
/// what a record is — so `Session's name` stays an ORDINARY field access on both tiers and no
/// expression anywhere needs rewriting.
pub fn store_type_name(store: &str) -> String {
    format!("__Store_{store}")
}

/// The function a `## Directive` lowers to. A directive is behavior attached to an element, and
/// on this tier that is exactly what a function call is.
pub fn directive_fn_name(directive: &str) -> String {
    format!("__dir_{directive}")
}

/// `__dir_<name>__removed` — a directive's `When removed:` block, run with the element and value
/// its occurrence was last attached with, once that occurrence is no longer drawn.
pub fn directive_removed_fn_name(directive: &str) -> String {
    format!("__dir_{directive}__removed")
}

/// `__DIRK_<name>` — every attached occurrence of a directive with a `When removed:` block, by its
/// identity (the instance key, `\u{1}`, its static site, and its row in every enclosing loop).
pub fn directive_keys_binding(directive: &str) -> String {
    format!("__DIRK_{directive}")
}

/// `__DIRS_<name>` — the paint pass each attached occurrence was last drawn in.
pub fn directive_stamp_binding(directive: &str) -> String {
    format!("__DIRS_{directive}")
}

/// `__DIRN_<name>` — the element each attached occurrence was last drawn on.
pub fn directive_node_binding(directive: &str) -> String {
    format!("__DIRN_{directive}")
}

/// `__DIRV_<name>` — the value each attached occurrence was last drawn with.
pub fn directive_value_binding(directive: &str) -> String {
    format!("__DIRV_{directive}")
}

/// The entry point a `Mount` calls: record where the component renders, run its load handlers
/// once, then paint. Distinct from the render function, which runs again on every repaint — a
/// load handler that ran on each repaint would not be a load handler.
pub fn mount_fn_name(component: &str) -> String {
    format!("__mount_{component}")
}

pub fn action_fn_name(component: &str, action: &str) -> String {
    format!("__act_{component}_{action}")
}

/// `__route_<METHOD>_<slug>` — the function a `## Route` becomes.
pub fn route_fn_name(method: &str, path: &str) -> String {
    let slug: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("__route_{}_{}", method.to_ascii_uppercase(), slug)
}

/// The construct an author wrote that the compiler-written function `name` was lowered from, named
/// as the author knows it: `` `## Route GET /agent` ``, `` the `add` action of `## Component
/// Counter` ``, `` `## Directive focus` ``, or `` the `## Route` table `` for the walks that serve
/// every route. Built on the forward helpers above over the constructs the program declares, so
/// there is no inverse mapping to keep in step. `None` for a name no construct owns — a synthesized
/// name never becomes text an author reads.
pub fn authored_construct(
    name: &str,
    program: &[crate::ast::stmt::Stmt<'_>],
    interner: &logicaffeine_base::Interner,
) -> Option<String> {
    use super::server::{
        ROUTE_BODY_WALK, ROUTE_CTYPE_WALK, ROUTE_HANDLER, ROUTE_HEADERS_WALK, ROUTE_RESPONSE_WALK,
        ROUTE_STATUS_WALK,
    };
    use crate::ast::stmt::Stmt;
    const WALKS: [&str; 6] = [
        ROUTE_HANDLER,
        ROUTE_BODY_WALK,
        ROUTE_STATUS_WALK,
        ROUTE_CTYPE_WALK,
        ROUTE_HEADERS_WALK,
        ROUTE_RESPONSE_WALK,
    ];
    if WALKS.contains(&name) {
        return Some("the `## Route` table".to_string());
    }
    let mut components: Vec<&crate::ast::component::ComponentDef<'_>> = Vec::new();
    for s in program {
        match s {
            Stmt::RouteDef { method, path, .. }
                if route_fn_name(interner.resolve(*method), interner.resolve(*path)) == name =>
            {
                return Some(format!(
                    "`{}`",
                    super::gate::route_heading(*method, *path, interner)
                ));
            }
            Stmt::DirectiveDef {
                name: directive, ..
            } if directive_fn_name(interner.resolve(*directive)) == name => {
                return Some(format!("`## Directive {}`", interner.resolve(*directive)));
            }
            Stmt::ComponentDef(def) => components.push(def),
            _ => {}
        }
    }
    // Longest component name first, so a patch function of `Counter_x` is never claimed by `Counter`.
    components.sort_by_key(|def| std::cmp::Reverse(interner.resolve(def.name).len()));
    for def in components {
        let component = interner.resolve(def.name);
        let heading = super::gate::construct_heading(def, interner);
        for action in &def.actions {
            let action_name = interner.resolve(action.name);
            if action_fn_name(component, action_name) == name {
                return Some(format!("the `{action_name}` action of `{heading}`"));
            }
        }
        let owned = [
            render_fn_name(component),
            html_fn_name(component),
            mount_fn_name(component),
        ]
        .iter()
        .any(|n| n == name)
            || name.starts_with(&patch_fn_name(component, ""))
            || def
                .state
                .iter()
                .any(|f| bind_fn_name(component, interner.resolve(f.name)) == name);
        if owned {
            return Some(format!("`{heading}`"));
        }
    }
    None
}
