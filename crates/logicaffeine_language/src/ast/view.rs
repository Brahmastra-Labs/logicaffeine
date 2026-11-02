//! The LGX **view IR** — what a template PARSES INTO before anything is lowered.
//!
//! A view is a tree of elements, text, holes, control flow, and component uses. The
//! parser builds this tree first (pure structure, no statements), then a separate
//! lowering emits whatever the execution tiers consume. Keeping the tree explicit is
//! what lets the same template be checked, server-rendered, compiled to fine-grained
//! DOM patches, inspected by tooling, and printed back (`shape`) for locks.
//!
//! Every expression a node carries is an arena `&'a Expr<'a>` parsed by the ordinary
//! expression parser, so holes and conditions are first-class LOGOS expressions; the
//! tree itself is owned (`Vec` children), like the rest of the imperative AST.

use crate::ast::stmt::{Block, Expr, Literal, Pattern, StringPart};
use crate::token::Span;
use logicaffeine_base::{Interner, Symbol};

/// The position `{for each x in xs, at index i:}` gives the first item.
///
/// Positions in LOGOS are 1-based — `item 1 of xs` is the first item and `item 0 of xs` is an
/// error — so a template index names the same numbers: the row showing `{i}` is the row
/// `item i of xs` reads. Every walk that draws a loop (the page lowering, the server walk) starts
/// its counter here.
pub const LOOP_INDEX_BASE: i64 = 1;

/// One complete view: a `Render:` block, a `Render into <el>:` block, or a component's
/// `View:` — in source order they appear in [`crate::parser::Parser::views`].
#[derive(Debug, Clone)]
pub struct ViewDef<'a> {
    /// The component this view belongs to (`None` for a bare `Render:`).
    pub component: Option<Symbol>,
    /// Where the tree renders.
    pub root: ViewRoot<'a>,
    pub nodes: Vec<ViewNode<'a>>,
    /// Every identifier the template MENTIONS, with its span in the FILE.
    ///
    /// ★ THE ONE THING A TOKEN WALK CANNOT RECOVER. A `View:` body reaches the parser as a single
    /// `RawBlock` token, so a name used only in a template is not in the token stream at all —
    /// which is why an editor could not jump from `{count}` to the `Let count be 0.` it names,
    /// the most common movement there is in an LGX file. The positions are known HERE and only
    /// here: `TExpr` carries its offset in the template body, and the parser holds the map from
    /// body offsets to file offsets. Recorded on the way past rather than re-derived, because a
    /// second copy of that arithmetic is a second thing to get wrong.
    pub mentions: Vec<(Symbol, Span)>,
    pub span: Span,
}

/// The render target of a [`ViewDef`].
#[derive(Debug, Clone)]
pub enum ViewRoot<'a> {
    /// `Render:` — appended under the document body (the initial mount, never cleared).
    Body,
    /// `Render into <expr>:` — the target is cleared, then rebuilt.
    Into(&'a Expr<'a>),
    /// A component's `View:` — rendered as a fragment into the parent the caller passes.
    ComponentParent,
    /// A page's `Head:` — rendered at BUILD time into the document's `<head>`, never
    /// mounted. It is a separate root rather than a `ComponentParent` because nothing
    /// in a browser ever paints it: the document already carries it when the module
    /// loads, so re-rendering it would be work with no observer.
    Head,
}

/// One node of a view tree.
#[derive(Debug, Clone)]
pub enum ViewNode<'a> {
    /// An HTML element with its attributes (in source order) and children.
    Element {
        tag: Symbol,
        attrs: Vec<Attr<'a>>,
        children: Vec<ViewNode<'a>>,
        span: Span,
    },
    /// A text child: a literal or interpolated string expression (escaped on render).
    Text { text: &'a Expr<'a>, span: Span },
    /// A use of a declared component (`Counter`, `<TodoRow …/>`). `prop_spans[i]` is where
    /// `props[i]` was written — the whole `name={value}` attribute — so a refusal about one prop
    /// points at that prop.
    Component {
        name: Symbol,
        props: Vec<(Symbol, &'a Expr<'a>)>,
        prop_spans: Vec<Span>,
        children: Vec<ViewNode<'a>>,
        span: Span,
    },
    /// `for each <pattern> in <iterable>[, keyed by <key>][, at index <i>]:` — one body per item;
    /// `empty` renders when the collection has no items. `index` counts from
    /// [`LOOP_INDEX_BASE`].
    Each {
        pattern: Pattern,
        iterable: &'a Expr<'a>,
        key: Option<&'a Expr<'a>>,
        index: Option<Symbol>,
        body: Vec<ViewNode<'a>>,
        empty: Vec<ViewNode<'a>>,
        span: Span,
    },
    /// `when <cond>:` … `otherwise:` — arms in order, then the fallback.
    If {
        arms: Vec<(&'a Expr<'a>, Vec<ViewNode<'a>>)>,
        otherwise: Option<Vec<ViewNode<'a>>>,
        span: Span,
    },
    /// An event handler on the containing element: the legacy `on click:` + statements, or
    /// an LGX `on-click={action}` directive (whose expression becomes a one-call body).
    /// `modifiers` are the `.prevent` / `.stop` / `.enter` … qualifiers, already classified by
    /// the event-modifier catalogue ([`crate::lgx::html::EventModifier`]).
    Handler {
        event: Symbol,
        modifiers: Vec<crate::lgx::html::EventModifier>,
        body: Block<'a>,
        span: Span,
    },
    /// `raw <string>` — trusted markup inserted verbatim.
    Raw { html: &'a Expr<'a>, span: Span },
    /// `{inspect v:} {when Pat:} … {end}` — a pattern match whose arms are subtrees.
    Inspect {
        scrutinee: &'a Expr<'a>,
        arms: Vec<ViewArm<'a>>,
        span: Span,
    },
    /// `{let x be e}` — a template-local binding, in scope for the following siblings.
    Let {
        var: Symbol,
        value: &'a Expr<'a>,
        span: Span,
    },
    /// `{extends Layout}` — this view fills the named layout's blocks.
    Extends { layout: Symbol, span: Span },
    /// `{block name:} … {end}` (with a default) or `{block name}` (a slot to fill).
    Block {
        name: Symbol,
        default: Option<Vec<ViewNode<'a>>>,
        span: Span,
    },
    /// `{children}` — where a component's caller-supplied children render.
    Children { span: Span },
}

impl<'a> ViewNode<'a> {
    /// Where this node was written.
    ///
    /// ★ EVERY VARIANT ALREADY CARRIED A `span` AND NOTHING COULD ASK FOR IT. The browser-tier
    /// refusals in `lgx_lower` all walk this tree, and each one destructured the node with `..` —
    /// so a page refused because of ONE element reported at `Span::default()`, which renders as
    /// the top of the file. The author was told their page could not be lowered and left to find
    /// which of forty nodes did it.
    ///
    /// TOTAL, deliberately: a new `ViewNode` variant does not compile until it says where it is.
    pub fn span(&self) -> Span {
        match self {
            ViewNode::Element { span, .. }
            | ViewNode::Text { span, .. }
            | ViewNode::Component { span, .. }
            | ViewNode::Each { span, .. }
            | ViewNode::If { span, .. }
            | ViewNode::Handler { span, .. }
            | ViewNode::Raw { span, .. }
            | ViewNode::Inspect { span, .. }
            | ViewNode::Let { span, .. }
            | ViewNode::Extends { span, .. }
            | ViewNode::Block { span, .. }
            | ViewNode::Children { span } => *span,
        }
    }
}

impl<'a> ViewNode<'a> {
    /// The construct as an author writes it, for a message that names what it is about.
    pub fn construct(&self) -> &'static str {
        match self {
            ViewNode::Element { .. } => "an element",
            ViewNode::Text { .. } => "a text hole",
            ViewNode::Component { .. } => "a placed component",
            ViewNode::Each { .. } => "`{for each}`",
            ViewNode::If { .. } => "`{if}`",
            ViewNode::Handler { .. } => "an `on-<event>` handler",
            ViewNode::Raw { .. } => "`{raw}`",
            ViewNode::Inspect { .. } => "`{inspect}`",
            ViewNode::Let { .. } => "`{let}`",
            ViewNode::Extends { .. } => "`{extends}`",
            ViewNode::Block { .. } => "`{block}`",
            ViewNode::Children { .. } => "`{children}`",
        }
    }
}

/// One `{when Pattern:}` arm of a view `{inspect …:}`.
#[derive(Debug, Clone)]
pub struct ViewArm<'a> {
    /// The variant name (`Ok`, `JObj`), or `None` for `Otherwise`.
    pub variant: Option<Symbol>,
    /// `(field, binding)` pairs the pattern binds.
    pub bindings: Vec<(Symbol, Symbol)>,
    pub body: Vec<ViewNode<'a>>,
}

/// One attribute-position item of an element, in source order.
#[derive(Debug, Clone)]
pub enum Attr<'a> {
    /// `name=<expr>` — a plain (possibly dynamic) attribute.
    Plain { name: Symbol, value: &'a Expr<'a> },
    /// `bound to <field>` / `bind={field}` — two-way form binding.
    Bind { field: Symbol },
    /// `class:name={condition}` — the class is present exactly while the condition holds.
    Class { name: Symbol, cond: &'a Expr<'a> },
    /// A declared `## Directive` used as an attribute (`tooltip={text}`).
    Directive { name: Symbol, value: &'a Expr<'a> },
}

/// What a `bind={field}` on an element actually WRITES — the one place the whole language
/// decides, so no two walks can disagree about a checkbox.
///
/// An `<input type="text">` holds its state in `value`, a string. An `<input type="checkbox">`
/// (or `type="radio"`) holds its state in `checked`, a PRESENCE — and not in the `checked`
/// ATTRIBUTE either, which is `defaultChecked`. Writing a checkbox's state as a value renders
/// `value="true"` and draws the box UNCHECKED while the page believes it is checked, which is
/// worse than not supporting `bind` on a checkbox at all: the author has been told their
/// binding was understood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundInput {
    /// `value` — a text/number/date input, a `<textarea>`, a `<select>`.
    Value,
    /// `checked`, and a click FLIPS it — a checkbox.
    Checkbox,
    /// `checked`, and a click SELECTS it — a radio. Split from [`BoundInput::Checkbox`] because
    /// the two differ exactly where it matters: clicking a chosen radio leaves it chosen, and a
    /// lowering that flipped it would let a second click deselect it, which no radio group has
    /// ever done.
    Radio,
    /// The element's `type` is computed, so which of the two this is cannot be known until the
    /// page runs. Refused rather than guessed: guessing `Value` is the silent wrong answer above.
    Unknown,
    /// `<input type="file">`. A browser **will not let a page set a file input's value** — a
    /// deliberate restriction, because a page that could would be a page that can read any file on
    /// your disk by naming it. So a `bind` here writes nothing, silently, forever.
    File,
    /// `<select multiple>`. Its value is the LIST of chosen options, so writing one text into it
    /// selects at most one and reading it back returns at most one — a visitor's other selections
    /// vanish with no error anywhere.
    SelectMultiple,
}

impl BoundInput {
    /// Whether this binding writes the `checked` PROPERTY rather than `value`.
    pub fn writes_checked(self) -> bool {
        matches!(self, BoundInput::Checkbox | BoundInput::Radio)
    }

    /// Why a [`BoundInput::Unknown`] binding is refused — held here, in ONE place, so the checker
    /// and the lowering refuse it in the same words. Two refusals of the same thing in different
    /// words send whoever reads one message searching for the other.
    pub const UNKNOWN_REFUSAL: &'static str =
        "a `bind` sits on an element whose `type` is computed, so whether it binds a value or a \
         checkbox's checked state cannot be known until the page runs. Write the type literally \
         (`type=\"checkbox\"` or `type=\"text\"`), or use two elements";

    /// Why this binding cannot be lowered, or `None` when it can.
    ///
    /// ★ TOTAL, AND THAT IS THE POINT. A new variant does not compile until somebody decides
    /// whether it lowers or is refused BY NAME — silently absent is not one of the options, which
    /// is the discipline `action_verb_tier_lock` holds over every `UiStmt`. Two of these arms exist
    /// because the question used to be asked of the ATTRIBUTES ALONE: with no tag to look at, a
    /// file input and a multiple select both fell through to `Value` and got a text write that
    /// does nothing, while the author was told their binding was understood.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            BoundInput::Value | BoundInput::Checkbox | BoundInput::Radio => None,
            BoundInput::Unknown => Some(Self::UNKNOWN_REFUSAL),
            BoundInput::File => Some(
                "a `bind` sits on `<input type=\"file\">`, and a browser will not let a page set a \
                 file input's value — a page that could would be a page that can read any file on \
                 the visitor's disk by naming it. So the binding would do nothing at all. Read the \
                 chosen file in a `When chosen:` handler instead",
            ),
            BoundInput::SelectMultiple => Some(
                "a `bind` sits on a `<select multiple>`, whose value is the LIST of chosen options \
                 rather than one text. Binding it to a single value would select at most one and \
                 silently drop the visitor's other choices. Drop `multiple` if one choice is what \
                 you meant, or read the selection in a `When chosen:` handler",
            ),
        }
    }
}

/// Decide, from an element's TAG and attributes, what a `bind` on it writes. See [`BoundInput`].
///
/// ★ THE TAG IS NOT DECORATION HERE. Asking the attributes alone cannot tell a `<select multiple>`
/// from a `<textarea>` — neither carries a `type` — so both answered `Value`, and one of those
/// answers was wrong in a way nothing reported.
pub fn bound_input_kind(tag: Symbol, attrs: &[Attr<'_>], interner: &Interner) -> BoundInput {
    let has = |want: &str| {
        attrs
            .iter()
            .any(|a| matches!(a, Attr::Plain { name, .. } if interner.resolve(*name) == want))
    };
    if interner.resolve(tag) == "select" {
        // `multiple` is an HTML boolean attribute: its PRESENCE is its state.
        return if has("multiple") {
            BoundInput::SelectMultiple
        } else {
            BoundInput::Value
        };
    }
    for attr in attrs {
        let Attr::Plain { name, value } = attr else {
            continue;
        };
        if interner.resolve(*name) != "type" {
            continue;
        }
        return match value {
            Expr::Literal(Literal::Text(t)) => match interner.resolve(*t) {
                "checkbox" => BoundInput::Checkbox,
                "radio" => BoundInput::Radio,
                "file" => BoundInput::File,
                _ => BoundInput::Value,
            },
            _ => BoundInput::Unknown,
        };
    }
    // No `type` at all is a text input, which is what HTML itself says.
    BoundInput::Value
}

impl<'a> ViewDef<'a> {
    /// Every expression and every statement block the tree references (holes, attribute
    /// values, conditions, iterables, keys, handler bodies) — the traversal surface for
    /// depth bounds, scope analysis and tooling walks.
    pub fn exprs_and_blocks(&self) -> (Vec<&'a Expr<'a>>, Vec<Block<'a>>) {
        let mut exprs = Vec::new();
        let mut blocks = Vec::new();
        if let ViewRoot::Into(e) = self.root {
            exprs.push(e);
        }
        fn go<'a>(
            nodes: &[ViewNode<'a>],
            exprs: &mut Vec<&'a Expr<'a>>,
            blocks: &mut Vec<Block<'a>>,
        ) {
            for n in nodes {
                match n {
                    ViewNode::Element {
                        attrs, children, ..
                    } => {
                        for a in attrs {
                            match a {
                                Attr::Plain { value, .. } | Attr::Directive { value, .. } => {
                                    exprs.push(value)
                                }
                                Attr::Class { cond, .. } => exprs.push(cond),
                                Attr::Bind { .. } => {}
                            }
                        }
                        go(children, exprs, blocks);
                    }
                    ViewNode::Text { text, .. } => exprs.push(text),
                    ViewNode::Component {
                        props, children, ..
                    } => {
                        for (_, v) in props {
                            exprs.push(v);
                        }
                        go(children, exprs, blocks);
                    }
                    ViewNode::Each {
                        iterable,
                        key,
                        body,
                        empty,
                        ..
                    } => {
                        exprs.push(iterable);
                        if let Some(k) = key {
                            exprs.push(k);
                        }
                        go(body, exprs, blocks);
                        go(empty, exprs, blocks);
                    }
                    ViewNode::If {
                        arms, otherwise, ..
                    } => {
                        for (c, b) in arms {
                            exprs.push(c);
                            go(b, exprs, blocks);
                        }
                        if let Some(o) = otherwise {
                            go(o, exprs, blocks);
                        }
                    }
                    ViewNode::Handler { body, .. } => blocks.push(body),
                    ViewNode::Raw { html, .. } => exprs.push(html),
                    ViewNode::Extends { .. } | ViewNode::Children { .. } => {}
                    ViewNode::Inspect {
                        scrutinee, arms, ..
                    } => {
                        exprs.push(scrutinee);
                        for a in arms {
                            go(&a.body, exprs, blocks);
                        }
                    }
                    ViewNode::Let { value, .. } => exprs.push(value),
                    ViewNode::Block { default, .. } => {
                        if let Some(d) = default {
                            go(d, exprs, blocks);
                        }
                    }
                }
            }
        }
        go(&self.nodes, &mut exprs, &mut blocks);
        (exprs, blocks)
    }

    /// A compact, deterministic one-line rendering of the tree — the snapshot surface
    /// `view_ir_lock` pins and `largo expand` prints.
    pub fn shape(&self, interner: &Interner) -> String {
        let head = match (&self.component, &self.root) {
            (Some(c), _) => format!("view of {}", interner.resolve(*c)),
            (None, ViewRoot::Body) => "render into body".to_string(),
            (None, ViewRoot::Into(e)) => format!("render into {}", expr_shape(e, interner)),
            (None, ViewRoot::ComponentParent) => "view".to_string(),
            (None, ViewRoot::Head) => "head".to_string(),
        };
        format!("{head}{}", braces(&self.nodes, interner))
    }

    /// Just the node tree, without the `view of <Name>` head — what a component's own
    /// shape embeds.
    pub fn body_shape(&self, interner: &Interner) -> String {
        braces(&self.nodes, interner)
    }
}

fn braces(nodes: &[ViewNode<'_>], interner: &Interner) -> String {
    if nodes.is_empty() {
        return String::new();
    }
    let inner: Vec<String> = nodes.iter().map(|n| node_shape(n, interner)).collect();
    format!("{{{}}}", inner.join(", "))
}

fn node_shape(node: &ViewNode<'_>, interner: &Interner) -> String {
    match node {
        ViewNode::Element {
            tag,
            attrs,
            children,
            ..
        } => {
            let attrs_s = if attrs.is_empty() {
                String::new()
            } else {
                let items: Vec<String> = attrs
                    .iter()
                    .map(|a| match a {
                        Attr::Plain { name, value } => format!(
                            "{}={}",
                            interner.resolve(*name),
                            expr_shape(value, interner)
                        ),
                        Attr::Bind { field } => format!("bind={}", interner.resolve(*field)),
                        Attr::Class { name, cond } => format!(
                            "class:{}={}",
                            interner.resolve(*name),
                            expr_shape(cond, interner)
                        ),
                        Attr::Directive { name, value } => format!(
                            "{}!={}",
                            interner.resolve(*name),
                            expr_shape(value, interner)
                        ),
                    })
                    .collect();
                format!("[{}]", items.join(", "))
            };
            format!(
                "{}{attrs_s}{}",
                interner.resolve(*tag),
                braces(children, interner)
            )
        }
        ViewNode::Text { text, .. } => expr_shape(text, interner),
        ViewNode::Component {
            name,
            props,
            children,
            ..
        } => {
            let props_s: Vec<String> = props
                .iter()
                .map(|(k, v)| format!(" {}={}", interner.resolve(*k), expr_shape(v, interner)))
                .collect();
            format!(
                "<{}{}>{}",
                interner.resolve(*name),
                props_s.join(""),
                braces(children, interner)
            )
        }
        ViewNode::Each {
            pattern,
            iterable,
            key,
            index,
            body,
            empty,
            ..
        } => {
            let pat = match pattern {
                Pattern::Identifier(s) => interner.resolve(*s).to_string(),
                Pattern::Tuple(items) => {
                    let names: Vec<&str> = items.iter().map(|s| interner.resolve(*s)).collect();
                    format!("({})", names.join(", "))
                }
            };
            let key_s = key
                .map(|k| format!(" keyed by {}", expr_shape(k, interner)))
                .unwrap_or_default();
            let index_s = index
                .map(|i| format!(" at index {}", interner.resolve(i)))
                .unwrap_or_default();
            let empty_s = if empty.is_empty() {
                String::new()
            } else {
                format!(" else{}", braces(empty, interner))
            };
            format!(
                "each {pat} in {}{key_s}{index_s}{}{empty_s}",
                expr_shape(iterable, interner),
                braces(body, interner)
            )
        }
        ViewNode::If {
            arms, otherwise, ..
        } => {
            let mut s = String::new();
            for (i, (cond, body)) in arms.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&format!(
                    "when {}{}",
                    expr_shape(cond, interner),
                    braces(body, interner)
                ));
            }
            if let Some(o) = otherwise {
                s.push_str(&format!(" otherwise{}", braces(o, interner)));
            }
            s
        }
        ViewNode::Handler {
            event,
            modifiers,
            body,
            ..
        } => {
            let mods: String = modifiers.iter().map(|m| format!(".{}", m.word())).collect();
            format!("on:{}{mods}{{{}}}", interner.resolve(*event), body.len())
        }
        ViewNode::Raw { html, .. } => format!("raw({})", expr_shape(html, interner)),
        ViewNode::Inspect {
            scrutinee, arms, ..
        } => {
            let a: Vec<String> = arms
                .iter()
                .map(|arm| {
                    let head = match arm.variant {
                        None => "otherwise".to_string(),
                        Some(v) => {
                            let bs: Vec<&str> = arm
                                .bindings
                                .iter()
                                .map(|(_, b)| interner.resolve(*b))
                                .collect();
                            if bs.is_empty() {
                                interner.resolve(v).to_string()
                            } else {
                                format!("{} ({})", interner.resolve(v), bs.join(", "))
                            }
                        }
                    };
                    format!("when {head}{}", braces(&arm.body, interner))
                })
                .collect();
            format!(
                "inspect {}{{{}}}",
                expr_shape(scrutinee, interner),
                a.join(" ")
            )
        }
        ViewNode::Let { var, value, .. } => format!(
            "let {} = {}",
            interner.resolve(*var),
            expr_shape(value, interner)
        ),
        ViewNode::Extends { layout, .. } => format!("extends {}", interner.resolve(*layout)),
        ViewNode::Block { name, default, .. } => match default {
            None => format!("block {}", interner.resolve(*name)),
            Some(d) => format!("block {}{}", interner.resolve(*name), braces(d, interner)),
        },
        ViewNode::Children { .. } => "children".to_string(),
    }
}

/// A compact expression rendering for shapes: literals verbatim, identifiers by name,
/// calls as `f(args)`, field access as `a's b`, interpolation with `{…}` holes; anything
/// else prints as `_`.
pub fn expr_shape(e: &Expr<'_>, interner: &Interner) -> String {
    match e {
        Expr::Literal(Literal::Text(s)) => format!("{:?}", interner.resolve(*s)),
        Expr::Literal(Literal::Number(n)) => n.to_string(),
        Expr::Literal(Literal::Float(f)) => f.to_string(),
        Expr::Literal(Literal::Boolean(b)) => b.to_string(),
        Expr::Literal(Literal::Nothing) => "nothing".to_string(),
        Expr::Literal(Literal::Char(c)) => format!("{c:?}"),
        // Temporal literals keep their unit visible: `60 seconds` is a Duration, not a number.
        Expr::Literal(Literal::Duration(ms)) => format!("{ms}ms"),
        Expr::Literal(Literal::Date(d)) => format!("date {d}"),
        Expr::Literal(Literal::Moment(m)) => format!("moment {m}"),
        Expr::Literal(Literal::Time(t)) => format!("time {t}"),
        Expr::Literal(Literal::Span { months, days }) => format!("span {months}m{days}d"),
        Expr::Identifier(s) => interner.resolve(*s).to_string(),
        Expr::Call { function, args } => {
            let a: Vec<String> = args.iter().map(|x| expr_shape(x, interner)).collect();
            format!("{}({})", interner.resolve(*function), a.join(", "))
        }
        Expr::FieldAccess { object, field } => format!(
            "{}'s {}",
            expr_shape(object, interner),
            interner.resolve(*field)
        ),
        Expr::BinaryOp { op, left, right } => {
            format!(
                "({} {:?} {})",
                expr_shape(left, interner),
                op,
                expr_shape(right, interner)
            )
        }
        Expr::List(items) => {
            let a: Vec<String> = items.iter().map(|x| expr_shape(x, interner)).collect();
            format!("[{}]", a.join(", "))
        }
        Expr::New { type_name, .. } => format!("new {}", interner.resolve(*type_name)),
        Expr::Length { collection } => format!("length of {}", expr_shape(collection, interner)),
        Expr::InterpolatedString(parts) => {
            let mut s = String::from("\"");
            for p in parts {
                match p {
                    StringPart::Literal(t) => s.push_str(interner.resolve(*t)),
                    StringPart::Expr { value, .. } => {
                        s.push('{');
                        s.push_str(&expr_shape(value, interner));
                        s.push('}');
                    }
                }
            }
            s.push('"');
            s
        }
        _ => "_".to_string(),
    }
}

/// The literal segments of [`duplicate_keyed_loop_message`], PUBLIC so a compiled tier's own
/// lowering of `Each` can build the identical text out of runtime `Expr`s (string concatenation,
/// `loop_desc` baked in as a literal since it is known at lowering time; `key`/the two positions
/// are not) without a second, hand-copied version of the wording that could drift from this one.
pub mod duplicate_keyed_loop_message_parts {
    pub const PREFIX: &str = "the loop over ";
    pub const AFTER_LOOP_DESC: &str = " keys item ";
    pub const AFTER_FIRST_POSITION: &str = " and item ";
    pub const AFTER_SECOND_POSITION: &str = " both \"";
    pub const SUFFIX: &str = "\" — a `keyed by` loop's keys must be unique among its own \
         siblings; give the duplicate its own key, or drop `keyed by` if position alone already \
         identifies each row here.";
}

/// The message a `keyed by` loop whose siblings share a key fails with — the SAME wording on every
/// tier, because the lowering of `Each` builds it at run time from the segments this function
/// formats ([`duplicate_keyed_loop_message_parts`]). `loop_desc` names what is being iterated
/// (`expr_shape` on the loop's own `iterable`, e.g. `"todos"`); `key` is the colliding key's own
/// display value (its evaluated VALUE, not source text — two items collide on what a key IS, at
/// runtime, never on how it was spelled).
///
/// `first_position`/`second_position` are 1-BASED — every position an author sees is (Tristen's
/// decision), matching `at index`, which already counts from [`LOOP_INDEX_BASE`], never from 0.
/// `Each.index` being 0-based is this crate's own implementation detail; a caller converts before
/// calling (`i + 1`), so a raw internal index can never reach this message un-shifted.
pub fn duplicate_keyed_loop_message(
    loop_desc: &str,
    key: &str,
    first_position: usize,
    second_position: usize,
) -> String {
    use duplicate_keyed_loop_message_parts as p;
    format!(
        "{}{loop_desc}{}{first_position}{}{second_position}{}{key}{}",
        p::PREFIX,
        p::AFTER_LOOP_DESC,
        p::AFTER_FIRST_POSITION,
        p::AFTER_SECOND_POSITION,
        p::SUFFIX,
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_language/ast/view/duplicate_keyed_loop_message_tests.rs"]
mod duplicate_keyed_loop_message_tests;
