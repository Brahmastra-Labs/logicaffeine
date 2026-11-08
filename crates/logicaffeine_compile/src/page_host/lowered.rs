//! A lowered page, driven the way `runtime.js` drives it.
//!
//! A page compiled for a browser is ordinary LOGOS: `__mount_<C>`, the entries a host calls it
//! through (`__lgx_dispatch_node` for an element's event, `__lgx_tick`, `__lgx_on_failure`, the rest
//! of `lgx_lower::LGX_ENTRIES`), and closures it hands the host
//! (a fetch completion, a route handler, an interval). The browser runs `main`, then calls in:
//! each event, completion and tick through one guarded door that reports a trap, hands its
//! message to the page's failure handler, and settles the page. [`LoweredHost`] is that host's
//! state, and its driver works any engine that can call a lowered function by name and a closure by
//! value ([`LoweredCalls`]) — the bytecode VM and the tree-walker alike. The host OWNS NO ENGINE: it
//! borrows one for the statement it is carrying out, so the tree-walker, which also runs the test
//! driving the page, can hold its page's host itself. [`LoweredPage`] is a host and the engine it
//! drives held together, for a host process that owns both.

use super::Drive;
use crate::interpreter::RuntimeValue;
use crate::semantics::dom;
use crate::semantics::drive::{self as answers, Answer, Verb};
use logicaffeine_language::parser::lgx_lower;
use std::collections::BTreeSet;
use std::rc::Rc;

/// An engine a lowered page runs on, seen from the host: call a function the page defines, call
/// a closure it handed out, ask whether it defines a name, and reload it.
pub trait LoweredCalls {
    fn call(&mut self, name: &str, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String>;
    fn call_value(
        &mut self,
        callee: &RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String>;
    fn has(&self, name: &str) -> bool;
    /// The page reloaded: a fresh instance of the same program, its main run again.
    fn reload(&mut self) -> Result<(), String>;
    /// Every line the engine has shown, main and every call since, in order.
    fn lines(&self) -> &[String];
    /// The failure site (`Stmt::Site`) in force when the most recent failed call trapped — where
    /// in the page's source it failed — for an engine that tracks one.
    fn failure_site(&self) -> Option<u32> {
        None
    }
    /// A failure the page's own code contained while it ran — a turn a driving verb the program
    /// runs on its own page took ([`crate::semantics::builtins::BuiltinId::DomDispatchEntry`]),
    /// whose failure door already heard it and no handler absorbed — with its failure site, for
    /// the host to hold as the page's failure. `None` for an engine whose entries report through
    /// this host alone.
    fn take_unhandled(&mut self) -> Option<(String, Option<u32>)> {
        None
    }
}

impl LoweredCalls for crate::vm::session::Session<'_> {
    fn take_unhandled(&mut self) -> Option<(String, Option<u32>)> {
        crate::vm::session::Session::take_unhandled(self)
    }
    fn call(&mut self, name: &str, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        self.invoke(name, args)
    }
    fn call_value(
        &mut self,
        callee: &RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String> {
        crate::vm::session::Session::call_value(self, callee, args)
    }
    fn has(&self, name: &str) -> bool {
        crate::vm::session::Session::has(self, name)
    }
    fn reload(&mut self) -> Result<(), String> {
        crate::vm::session::Session::reload(self)
    }
    fn lines(&self) -> &[String] {
        crate::vm::session::Session::lines(self)
    }
}

/// Why a statement did not drive the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveError {
    /// The page answered with a failure — the one the reference tier raises for the same
    /// statement, and the one a test author reads.
    Failed(String),
    /// This tier cannot carry the statement at all; the reason says what is missing.
    Refused(String),
}

/// One interval the page armed, and when it next comes due on the page's clock.
struct Armed {
    period_ms: i64,
    handler: RuntimeValue,
    next_due_ms: i64,
}

/// The host state a browser keeps for a lowered page. It owns no engine: every statement it
/// carries out borrows the engine the page runs on for exactly that statement
/// ([`LoweredHost::drive`]).
#[derive(Default)]
pub struct LoweredHost {
    /// The page's clock, in virtual milliseconds since it started.
    now_ms: i64,
    armed: Vec<Armed>,
    /// Requests the host has already decided about. A request is answered by what answers it
    /// when it is sent, once — the reference tier's rule — so a stub arranged later does not
    /// reach back and answer a request that had none.
    decided: BTreeSet<i64>,
    serve_routes: bool,
    /// The first action failure no handler absorbed.
    unhandled: Option<String>,
    /// Where that failure happened, when the engine tracks it ([`LoweredCalls::failure_site`]).
    unhandled_site: Option<u32>,
    /// The mounted component's own name — a page mounts itself, so this is set before `main` runs
    /// for one, and by a [`Drive::Mount`] for an explicit `Mount`. What `Replay these actions:`
    /// runs its actions against.
    mounted_component: Option<String>,
    /// The `## Page` component the program mounts by itself, if any — what loading the page
    /// mounts, so a reload mounts it again.
    page: Option<String>,
}

impl LoweredHost {
    /// The host of a page that has not started.
    pub fn new() -> Self {
        LoweredHost::default()
    }

    /// The host of a program whose `## Page` component `page` names (`None` for a program that
    /// mounts nothing by itself): what its reload mounts again, and what `Replay` runs against
    /// before any `Mount` of the test's own.
    pub fn of_page(page: Option<String>) -> Self {
        LoweredHost {
            mounted_component: page.clone(),
            page,
            ..LoweredHost::default()
        }
    }

    /// The first action failure no handler absorbed, if any.
    pub fn unhandled_failure(&self) -> Option<&str> {
        self.unhandled.as_deref()
    }

    /// The first action failure no handler absorbed, taken — the program's own failure once the
    /// program that hosts the page has finished.
    pub fn take_unhandled_failure(&mut self) -> Option<String> {
        self.unhandled.take()
    }

    /// Where the first unabsorbed failure happened, when the engine tracks it.
    pub fn unhandled_failure_site(&self) -> Option<u32> {
        self.unhandled_site
    }

    /// Start the page on `calls`: its main runs — the mount a `## Page` performs by itself, and
    /// whatever else the program does at the top level — and the page settles. A failing main is
    /// the program's failure. `auto_mounted` names the `## Page` component the program mounts by
    /// itself, if any ([`crate::page_host::auto_mounted_page_component`]) — set before `main` runs,
    /// since a page's own mount is not a [`Drive::Mount`] this host ever sees.
    pub fn start<L: LoweredCalls + ?Sized>(
        &mut self,
        calls: &mut L,
        auto_mounted: Option<String>,
        main: impl FnOnce(&mut L) -> Result<(), String>,
    ) -> Result<(), DriveError> {
        self.mounted_component = auto_mounted.clone();
        self.page = auto_mounted;
        main(calls).map_err(|message| DriveError::Failed(in_failure_context(calls, message)))?;
        self.hold_contained_failure(calls);
        Driver { host: self, calls }.settle();
        Ok(())
    }

    /// Hold a failure the page's own code contained while it ran ([`LoweredCalls::take_unhandled`])
    /// as the page's failure, when it is the first: its failure door already ran.
    fn hold_contained_failure<L: LoweredCalls + ?Sized>(&mut self, calls: &mut L) {
        if let Some((message, site)) = calls.take_unhandled() {
            if self.unhandled.is_none() {
                self.unhandled = Some(message);
                self.unhandled_site = site;
            }
        }
    }

    /// Deliver a popstate the page is owed and complete every request that has an answer — what the
    /// host does after anything runs in the page (`Driver::settle`).
    pub fn settle<L: LoweredCalls + ?Sized>(&mut self, calls: &mut L) {
        Driver { host: self, calls }.settle();
    }

    /// Carry one driving statement out on the page `calls` runs, exactly as the browser host
    /// would, then settle.
    pub fn drive<L: LoweredCalls + ?Sized>(
        &mut self,
        calls: &mut L,
        d: &Drive,
    ) -> Result<(), DriveError> {
        Driver { host: self, calls }.drive(d)
    }
}

/// A lowered page and its host, held together by a process that owns both — the bytecode VM's
/// page, driven from outside the interpreter running the test.
pub struct LoweredPage<L: LoweredCalls> {
    calls: L,
    host: LoweredHost,
}

impl<L: LoweredCalls> LoweredPage<L> {
    /// Host a page whose engine has not run yet.
    pub fn new(calls: L) -> Self {
        LoweredPage {
            calls,
            host: LoweredHost::new(),
        }
    }

    /// The engine the page runs on.
    pub fn calls(&mut self) -> &mut L {
        &mut self.calls
    }

    /// The first action failure no handler absorbed, if any.
    pub fn unhandled_failure(&self) -> Option<&str> {
        self.host.unhandled_failure()
    }

    /// Every line the page has shown, newline-joined — the engine's own output convention, so a
    /// host that also runs a program's plain `Show`s (a `## Main` that both prints and drives a
    /// page) reports the same text the straight-through compiled path would.
    pub fn output(&self) -> String {
        self.calls.lines().join("\n")
    }

    /// Start the page: see [`LoweredHost::start`].
    pub fn start(
        &mut self,
        auto_mounted: Option<String>,
        main: impl FnOnce(&mut L) -> Result<(), String>,
    ) -> Result<(), DriveError> {
        self.host.start(&mut self.calls, auto_mounted, main)
    }

    /// Carry one driving statement out: see [`LoweredHost::drive`].
    pub fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        self.host.drive(&mut self.calls, d)
    }
}

/// One statement being carried out: the host's state and the engine the page runs on, borrowed
/// together for exactly as long as the statement takes.
struct Driver<'d, L: ?Sized> {
    host: &'d mut LoweredHost,
    calls: &'d mut L,
}

impl<L: LoweredCalls + ?Sized> Driver<'_, L> {
    /// Carry one driving statement out, exactly as the browser host would, then settle.
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        match d {
            Drive::Mount { component, at } => self.mount(component, at.as_deref())?,
            Drive::Click { selector } => {
                self.default_action(Verb::Click, selector, None);
                self.click(selector)?
            }
            Drive::Submit { selector } => {
                self.default_action(Verb::Submit, selector, None);
                self.submit(selector)?
            }
            Drive::Press { key, selector } => {
                self.default_action(Verb::Press, selector, None);
                self.element_verb(Verb::Press, selector, key, Some(key), true)?;
            }
            Drive::Type { text, selector } => {
                self.default_action(Verb::Type, selector, Some(text));
                self.element_verb(Verb::Type, selector, text, None, true)?;
            }
            Drive::Choose { value, selector } => {
                self.default_action(Verb::Choose, selector, Some(value));
                self.element_verb(Verb::Choose, selector, value, None, true)?;
            }
            Drive::Hover { selector } => {
                self.default_action(Verb::Hover, selector, None);
                let answer = self.dispatch(selector, Verb::Hover.events()[0], "");
                if let Some(msg) =
                    answers::answer_failure(answer, Verb::Hover.events(), None, selector, true)
                {
                    return Err(DriveError::Failed(msg));
                }
            }
            Drive::Focus { selector } => {
                dom::element_focus(require_element(selector, "Focus")?);
                self.deliver_raised_events();
            }
            Drive::Blur { selector } => {
                dom::element_blur(require_element(selector, "Blur")?);
                self.deliver_raised_events();
            }
            Drive::Raise {
                event,
                target,
                value,
            } => {
                if dom::query_selector(target) < 0 {
                    return Err(DriveError::Failed(answers::no_element(target)));
                }
                let raise = lgx_lower::raise_fn_name();
                if self.calls.has(raise) {
                    let args = vec![text(target), text(event), text(value)];
                    self.guarded(|c| c.call(raise, args));
                }
            }
            Drive::DropFile { target, file } => {
                if dom::query_selector(target) < 0 {
                    return Err(DriveError::Failed(answers::no_element(target)));
                }
                dom::set_dropped_file(file.clone());
                let raise = lgx_lower::raise_fn_name();
                if self.calls.has(raise) {
                    let args = vec![text(target), text("filedropped"), text(&file.name)];
                    self.guarded(|c| c.call(raise, args));
                }
            }
            Drive::Navigate { url } => {
                dom::navigate(url);
                self.deliver_route_change();
            }
            // The page loads again, as a browser reloads it: a fresh document at the same address
            // with the test's server still answering (`dom::reload`), a fresh clock, the page's
            // main run again, and the page it mounts by itself mounted again.
            Drive::Reload => {
                dom::reload();
                answers::reset_clock();
                self.host.armed.clear();
                self.host.decided.clear();
                self.host.now_ms = 0;
                self.calls.reload().map_err(DriveError::Failed)?;
                if let Some(page) = self.host.page.clone() {
                    self.mount(&page, None)?;
                }
            }
            Drive::AdvanceTime { ms } => {
                logicaffeine_system::testworld::advance_clock(*ms);
                self.advance_clock(*ms as i64);
            }
            Drive::AnswerFetch {
                method,
                url,
                status,
                body,
            } => {
                dom::stub_fetch(method, url, *status, body);
            }
            Drive::ServeFetchesWithRoutes => {
                if !self.calls.has("route") {
                    return Err(DriveError::Failed(answers::no_routes_to_serve()));
                }
                self.host.serve_routes = true;
            }
            Drive::Replay { actions } => self.replay(actions)?,
        }
        self.settle();
        Ok(())
    }

    /// `Replay these actions: a, b, c.` — re-run the mounted component's own actions, by name, in
    /// order, calling each one's lowered entry (`__act_<C>_<action>`) exactly as the reference
    /// tier's `replay_actions` (`Handler::Action { args: Vec::new(), .. }`) does: no ARGUMENTS of
    /// its own, but every `__act_` function's FIRST parameter is the instance key it runs against
    /// (`lower_component_action`'s own `instance_key_param()` binding, ahead of `action.params`) —
    /// `""`, the root instance's key, exactly what `root_key()`'s own doc names this driving-verb
    /// case for: "the driving-verb dispatcher, which matches a selector and so cannot say which
    /// instance rendered the element it names." `Replay` names no element either.
    fn replay(&mut self, actions: &[String]) -> Result<(), DriveError> {
        let Some(component) = self.host.mounted_component.clone() else {
            return Err(DriveError::Failed(
                "`Replay these actions:` needs a page to replay them against. Mount the component \
                 first."
                    .to_string(),
            ));
        };
        for action in actions {
            let entry = lgx_lower::action_fn_name(&component, action);
            if !self.calls.has(&entry) {
                return Err(DriveError::Failed(format!(
                    "`{action}` is not an action of the mounted component, so there is nothing to \
                     replay. Declare it in `Actions:` as `To {action} …:`."
                )));
            }
            self.guarded(|c| c.call(&entry, vec![text("")]));
        }
        Ok(())
    }

    /// `Mount C [at S]` — what the page's main does for a mount: find the parent, call
    /// `__mount_C(parent)`.
    fn mount(&mut self, component: &str, at: Option<&str>) -> Result<(), DriveError> {
        let entry = lgx_lower::mount_fn_name(component);
        if !self.calls.has(&entry) {
            return Err(DriveError::Refused(format!(
                "`Mount {component}` has no lowered entry `{entry}` on this tier"
            )));
        }
        let parent = match at {
            Some(sel) => {
                let node = dom::query_selector(sel);
                if node < 0 {
                    return Err(DriveError::Failed(answers::mount_target_missing(
                        component, sel,
                    )));
                }
                node
            }
            None => dom::body(),
        };
        // `When the page loads:` runs as the page mounts, not as a turn the page is handed, so its
        // failure is the program's: a browser's page main ends where its mount traps.
        let mounted = self
            .calls
            .call(&entry, vec![RuntimeValue::Int(parent)])
            .map_err(|message| DriveError::Failed(in_failure_context(&mut *self.calls, message)))?;
        self.after_call(Ok(mounted));
        self.host.mounted_component = Some(component.to_string());
        Ok(())
    }

    /// `Click SEL.` — the page hears the click, and the browser performs the click's default
    /// action: the link the click is on — the element's own `<a href>`, or the one it sits inside
    /// ([`dom::link_of`]) — is followed unless a listener the click reached cancels it. The link
    /// needs no listener, since the browser itself answers a click on one.
    fn click(&mut self, selector: &str) -> Result<(), DriveError> {
        let node = dom::query_selector(selector);
        if let Some(control) = (node >= 0).then(|| dom::toggle_control(node)).flatten() {
            return self.toggle(node, control);
        }
        let link = if node >= 0 { dom::link_of(node) } else { None };
        let prevented = self.element_verb(Verb::Click, selector, "", None, link.is_none())?;
        match link {
            Some(link) if link.here && !prevented => self.follow(&link.href),
            _ => Ok(()),
        }
    }

    /// A click on a checkbox or a radio: the browser moves the control itself — a checkbox flips, a
    /// radio is selected — before the page hears the `click`, and puts it back when a listener
    /// cancels the click; otherwise `input` and `change` follow. Each event carries the control's
    /// new state (`"true"`/`"false"`). The browser answers the click, so no listener is required.
    fn toggle(&mut self, node: i64, control: dom::ToggleControl) -> Result<(), DriveError> {
        let was = dom::get_checked(node);
        let now = match control {
            dom::ToggleControl::Checkbox => !was,
            dom::ToggleControl::Radio => true,
        };
        dom::set_checked(node, now);
        let state = if now { "true" } else { "false" };
        let (_, prevented) = self.deliver(node, "click", state);
        if prevented {
            dom::set_checked(node, was);
            return Ok(());
        }
        self.deliver(node, "input", state);
        self.deliver(node, "change", state);
        Ok(())
    }

    /// A link followed. A path the page answers itself — the module decides, through
    /// `__lgx_owns`, the same `matchRoute` its `Navigate to` branches on — is client navigation:
    /// the location moves in place (`pushState`) and the page's route handlers run. Any other path,
    /// and every path of a module that answers none, is a document load: another page file's
    /// document, outside the test ([`dom::assign_location`]).
    fn follow(&mut self, href: &str) -> Result<(), DriveError> {
        const OWNS: &str = "__lgx_owns";
        let path = href.split(['?', '#']).next().unwrap_or(href).to_string();
        let owned = self.calls.has(OWNS)
            && matches!(
                self.guarded(|c| c.call(OWNS, vec![text(&path)])),
                Some(RuntimeValue::Bool(true))
            );
        if owned {
            dom::navigate(href);
            self.deliver_route_change();
        } else {
            dom::assign_location(href);
        }
        Ok(())
    }

    /// `Submit SEL.` — `requestSubmit` on the form `SEL` is or belongs to, with `SEL` as the
    /// submitter when it is a submit button. The browser first validates the form
    /// ([`dom::form_constraints`]): an invalid control is answered with `invalid`, the first of them
    /// takes focus, and nothing is submitted. A valid form hears `submit`, and one nothing prevents
    /// is then submitted by the browser: a `get` form navigates with its fields in the query string,
    /// any other method sends them form-encoded to its action. A handler is an addition to that, so
    /// a form needs no listener; an element in no form must be answered.
    fn submit(&mut self, selector: &str) -> Result<(), DriveError> {
        let target = dom::query_selector(selector);
        let Some(node) = (target >= 0).then(|| dom::form_of(target)).flatten() else {
            self.element_verb(Verb::Submit, selector, "", None, true)?;
            return Ok(());
        };
        let submitter = (target != node && dom::submits_its_form(target)).then_some(target);
        let invalid = dom::form_constraints(node, submitter).map_err(DriveError::Refused)?;
        if !invalid.is_empty() {
            let mut first = None;
            for control in invalid {
                let (_, cancelled) = self.deliver(control, "invalid", "");
                if !cancelled && first.is_none() {
                    first = Some(control);
                }
            }
            if let Some(first) = first {
                dom::element_focus(first);
                self.deliver_raised_events();
            }
            return Ok(());
        }
        if self.deliver(node, "submit", "").1 {
            return Ok(());
        }
        let action = dom::attribute(node, "action").unwrap_or_else(dom::route_path);
        let method = dom::attribute(node, "method").unwrap_or_else(|| "get".to_string());
        let body = answers::form_urlencoded(&dom::form_fields(node));
        if method.eq_ignore_ascii_case("get") {
            let url = if body.is_empty() {
                action
            } else {
                format!("{action}?{body}")
            };
            dom::navigate(&url);
            self.deliver_route_change();
        } else {
            let req = dom::fetch_send(
                &method.to_uppercase(),
                &action,
                &body,
                vec![(
                    "content-type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                )],
                RuntimeValue::Nothing,
            );
            self.follow_post(req);
        }
        Ok(())
    }

    /// A form the browser submitted itself: its request is answered at once, by the rule every
    /// request is answered by — the test's answer for it, then the program's own routes — and the
    /// browser shows the answer as the new document ([`dom::follow_response`]), of the answer's own
    /// type. A request nothing answers stays sent, the page as it was.
    fn follow_post(&mut self, req: i64) {
        let Some(r) = dom::fetch_requests().into_iter().find(|r| r.req == req) else {
            return;
        };
        self.host.decided.insert(req);
        let stubbed = dom::stubbed_answer(req).is_some();
        let Some((_, body)) = self.answer_for(&r) else {
            return;
        };
        let content_type = if stubbed {
            dom::STUB_CONTENT_TYPE.to_string()
        } else {
            self.route_content_type(&r)
        };
        dom::follow_response(&r.url, &content_type, &body);
    }

    /// The type the program's own routes answer `r` with (`routeContentType`), plain text when the
    /// program declares none.
    fn route_content_type(&mut self, r: &dom::FetchRequest) -> String {
        const ROUTE_CONTENT_TYPE: &str = "routeContentType";
        if !self.calls.has(ROUTE_CONTENT_TYPE) {
            return "text/plain; charset=utf-8".to_string();
        }
        let args = vec![text(&r.method), text(&r.url)];
        match self.guarded(|c| c.call(ROUTE_CONTENT_TYPE, args)) {
            Some(RuntimeValue::Text(t)) => t.to_string(),
            _ => "text/plain; charset=utf-8".to_string(),
        }
    }

    /// An element verb: each of its events delivered at the element `selector` names
    /// ([`Driver::deliver`]), `value` carried with each — the typed text, the chosen value, the
    /// pressed key — and what came of them read back into the message the reference tier gives.
    /// Typing and choosing raise their events only where the control takes them
    /// ([`dom::answers_by_itself`]) and are then answered by the browser's own default action;
    /// where it cannot, nothing is raised and the verb is refused. Whether a listener the events
    /// reached cancelled their default action.
    fn element_verb(
        &mut self,
        verb: Verb,
        selector: &str,
        value: &str,
        key: Option<&str>,
        must_answer: bool,
    ) -> Result<bool, DriveError> {
        let node = dom::query_selector(selector);
        let entry = matches!(verb, Verb::Type | Verb::Choose);
        if entry && node >= 0 && !dom::answers_by_itself(verb, selector, value) {
            return Err(DriveError::Failed(answers::cannot_take(
                verb, selector, value,
            )));
        }
        let must_answer = must_answer && !entry;
        let (answer, prevented) = if node < 0 {
            (Answer::NoElement, false)
        } else {
            let (mut answered, mut prevented) = (false, false);
            for event in verb.events() {
                let (a, p) = self.deliver(node, event, value);
                answered |= a;
                prevented |= p;
            }
            (
                if answered {
                    Answer::Answered
                } else {
                    Answer::Unanswered
                },
                prevented,
            )
        };
        match answers::answer_failure(answer, verb.events(), key, selector, must_answer) {
            Some(msg) => Err(DriveError::Failed(msg)),
            None => Ok(prevented),
        }
    }

    /// Dispatch `event` at `node` as a browser does: along [`dom::dispatch_path`] — the capture
    /// listeners from the root down, the target, the bubbling listeners back up, ending where one
    /// stops propagation — each stop reached through the page's node dispatcher, found as the page
    /// stands when its turn comes. Whether anything answered (a turn that failed answered, its
    /// failure handed to the page's handler or reported), and whether a listener on the walk
    /// cancelled the event's default action.
    fn deliver(&mut self, node: i64, event: &str, value: &str) -> (bool, bool) {
        let path = dom::dispatch_path(node, event);
        let dispatch = lgx_lower::dispatch_node_fn_name();
        let mut answered = false;
        if self.calls.has(dispatch) {
            for stop in &path.stops {
                let at = dom::query_selector(stop);
                if at < 0 {
                    continue;
                }
                let args = vec![RuntimeValue::Int(at), text(event), text(value)];
                let answer = self.guarded(|c| c.call(dispatch, args));
                answered |= !matches!(answer, Some(RuntimeValue::Bool(false)));
            }
        }
        (answered, path.prevented)
    }

    /// The browser's own default action of `verb` on the element `selector` names, carried out
    /// before the page hears the verb's own events and in the order a browser performs it: focus
    /// moves where the verb sends it ([`dom::focus_by`], after [`Verb::focus_target`]), raising
    /// `blur` and then `focus` on the elements that lost and gained it; then a verb that enters a
    /// `value` leaves it in the control ([`dom::enter_value`]). Each step's per-verb rule lives in
    /// its own table, so every verb walks every step.
    fn default_action(&mut self, verb: Verb, selector: &str, value: Option<&str>) {
        for (element, event) in dom::focus_by(verb, selector) {
            self.dispatch_node(element, event);
        }
        if let Some(value) = value {
            dom::enter_value(verb, selector, value);
        }
    }

    /// Dispatch `event` at the element `node` itself — one the host already holds, which a selector
    /// need not name exactly ([`Driver::deliver`]).
    fn dispatch_node(&mut self, node: i64, event: &str) {
        self.deliver(node, event, "");
    }

    /// Dispatch `event` at the element `selector` names, carrying `value` ([`Driver::deliver`]).
    fn dispatch(&mut self, selector: &str, event: &str, value: &str) -> Answer {
        let node = dom::query_selector(selector);
        if node < 0 {
            return Answer::NoElement;
        }
        match self.deliver(node, event, value).0 {
            true => Answer::Answered,
            false => Answer::Unanswered,
        }
    }

    /// The location moved under the page (a test's `Navigate to`, a submitted `get` form): every
    /// handler the page registered with `onRoute` runs, in the order it registered them, as a
    /// browser's `popstate` reaches every listener.
    fn deliver_route_change(&mut self) {
        for handler in dom::deliver_pop(&dom::route_path()) {
            let args = event_args(&handler);
            self.guarded(|c| c.call_value(&handler, args));
        }
    }

    /// Move the page's clock forward by `ms`, as a browser's would over that much time: the page's
    /// own timers through `__lgx_tick`, which fires what the time that passed makes due, in due
    /// order; and every interval a program armed itself (`setInterval`), each time it comes due.
    /// The page's clock is carried up to each interval before it fires, so the two kinds share one
    /// timeline.
    fn advance_clock(&mut self, ms: i64) {
        let until = self.host.now_ms.saturating_add(ms.max(0));
        loop {
            self.adopt_intervals();
            let next = self
                .host
                .armed
                .iter()
                .enumerate()
                .filter(|(_, a)| a.period_ms > 0 && a.next_due_ms <= until)
                .min_by_key(|(i, a)| (a.next_due_ms, *i))
                .map(|(i, _)| i);
            let Some(i) = next else { break };
            let due = self.host.armed[i].next_due_ms;
            self.pass_page_time(due - self.host.now_ms);
            self.host.now_ms = due;
            self.host.armed[i].next_due_ms += self.host.armed[i].period_ms;
            let handler = self.host.armed[i].handler.clone();
            let args = event_args(&handler);
            self.guarded(|c| c.call_value(&handler, args));
            self.settle();
        }
        self.pass_page_time(until - self.host.now_ms);
        self.host.now_ms = until;
    }

    /// Hand the page's own clock the `ms` that passed (`__lgx_tick`), when it has one and time moved.
    fn pass_page_time(&mut self, ms: i64) {
        let tick = lgx_lower::tick_fn_name();
        if ms <= 0 || !self.calls.has(tick) {
            return;
        }
        self.guarded(|c| c.call(tick, vec![RuntimeValue::Int(ms)]));
        self.settle();
    }

    /// Start the clock on every interval the page armed since the host last looked.
    fn adopt_intervals(&mut self) {
        let all = answers::intervals();
        for interval in all.into_iter().skip(self.host.armed.len()) {
            self.host.armed.push(Armed {
                period_ms: interval.period_ms,
                handler: interval.handler,
                next_due_ms: self.host.now_ms + interval.period_ms,
            });
        }
    }

    /// Deliver a popstate the location owes the page — a step back taken outside any call into it —
    /// then complete every request that has an answer, each through the completion closure the page
    /// handed over with it, until no completion sends a request that has one.
    fn settle(&mut self) {
        if dom::take_popped() {
            self.deliver_route_change();
        }
        loop {
            self.adopt_intervals();
            let fresh: Vec<dom::FetchRequest> = dom::fetch_requests()
                .into_iter()
                .filter(|r| !self.host.decided.contains(&r.req))
                .collect();
            if fresh.is_empty() {
                return;
            }
            for r in fresh {
                self.host.decided.insert(r.req);
                if r.aborted {
                    continue;
                }
                let Some((status, body)) = self.answer_for(&r) else {
                    continue;
                };
                let closure = dom::begin_fetch_resolve(r.req, status, &[], &body);
                if let Some(closure) = closure.filter(|c| matches!(c, RuntimeValue::Function(_))) {
                    let args = vec![RuntimeValue::Int(r.req)];
                    self.guarded(|c| c.call_value(&closure, args));
                }
                dom::end_fetch_resolve();
            }
        }
    }

    /// What answers `r`: a test's stub first, then — when the test asked for it — the program's
    /// own route table.
    fn answer_for(&mut self, r: &dom::FetchRequest) -> Option<(i64, String)> {
        if let Some(answer) = dom::stubbed_answer(r.req) {
            return Some(answer);
        }
        if !self.host.serve_routes {
            return None;
        }
        let args = vec![text(&r.method), text(&r.url)];
        let body = self
            .guarded(|c| c.call("route", args.clone()))?
            .to_display_string();
        let status = if self.calls.has("routeStatus") {
            match self.guarded(|c| c.call("routeStatus", args)) {
                Some(RuntimeValue::Int(n)) => n,
                _ => 200,
            }
        } else {
            200
        };
        Some((status, body))
    }

    /// One call into the page through the host's single guarded door. The events the page's own
    /// element commands raised during the call (`focus()` moving focus raises `blur` and `focus`)
    /// are delivered as it returns, whether it answered or failed, and so is the `popstate` a
    /// `Go back.` in it raised ([`dom::take_popped`]) — to every route handler the page registered;
    /// a `Reload the page.` it asked for ([`dom::take_reload_request`]) is then carried out, as the
    /// browser reloads once the running script is done; a failure is then handed to the page's
    /// `When an action fails` handler, and one nothing absorbs is reported; and last, the document
    /// the page asked to load ([`dom::take_load_request`]) replaces it, as the browser commits the
    /// navigation once the script — its failure handler included — is done.
    fn guarded(
        &mut self,
        call: impl FnOnce(&mut L) -> Result<RuntimeValue, String>,
    ) -> Option<RuntimeValue> {
        let outcome = match call(&mut *self.calls) {
            Ok(v) => Ok(v),
            Err(message) => Err(in_failure_context(&mut *self.calls, message)),
        };
        self.after_call(outcome)
    }

    /// What follows every call into the page, in [`Driver::guarded`]'s order: its turns' contained
    /// failures held, the events and the `popstate` it raised delivered, the reload it asked for carried
    /// out, its own failure handed to the page's handler, and the document it asked to load committed.
    fn after_call(&mut self, outcome: Result<RuntimeValue, String>) -> Option<RuntimeValue> {
        self.host.hold_contained_failure(&mut *self.calls);
        self.deliver_raised_events();
        if dom::take_popped() {
            self.deliver_route_change();
        }
        // The frames this page published to its shared topics: a relay would carry them to every
        // other page on the topic, and an in-process host drives this one page alone, so there is
        // no other page to hand them to.
        let _ = crate::semantics::storage::take_shared_writes();
        if dom::take_reload_request() {
            if let Err(DriveError::Failed(message) | DriveError::Refused(message)) =
                self.drive(&Drive::Reload)
            {
                self.on_failure(message);
            }
        }
        let answered = match outcome {
            Ok(v) => Some(v),
            Err(message) => {
                self.on_failure(message);
                None
            }
        };
        if let Some(url) = dom::take_load_request() {
            dom::assign_location(&url);
        }
        answered
    }

    /// Deliver every event an element command raised ([`dom::take_raised_events`]) to the element
    /// it was raised on, until delivering them raises no more.
    fn deliver_raised_events(&mut self) {
        loop {
            let raised = dom::take_raised_events();
            if raised.is_empty() {
                return;
            }
            for (element, event) in raised {
                self.dispatch_node(element, event);
            }
        }
    }

    /// `__lgx_on_failure(message)` answers whether a handler absorbed the failure. A failure
    /// nothing absorbs — no handler, or a handler that itself failed — is the page's to report.
    fn on_failure(&mut self, message: String) {
        let site = self.calls.failure_site();
        let hook = lgx_lower::on_failure_fn_name();
        let absorbed = self.calls.has(hook)
            && matches!(
                self.calls.call(hook, vec![text(&message)]),
                Ok(RuntimeValue::Bool(true))
            );
        if !absorbed && self.host.unhandled.is_none() {
            self.host.unhandled = Some(message);
            self.host.unhandled_site = site;
        }
    }
}

/// `message`, named by what the page was doing when it trapped: the context the page left open
/// (`__lgx_failure_context` — a directive's block, in the sentence every tier uses), taken before
/// anything else runs in the page.
fn in_failure_context<L: LoweredCalls + ?Sized>(calls: &mut L, message: String) -> String {
    let taken = lgx_lower::failure_context_fn_name();
    if !calls.has(taken) {
        return message;
    }
    match calls.call(taken, Vec::new()) {
        Ok(RuntimeValue::Text(context)) if !context.is_empty() => format!("{context}{message}"),
        _ => message,
    }
}

/// The element a focus or blur acts on, or the refusal the reference tier gives when there is
/// none.
fn require_element(selector: &str, verb: &str) -> Result<i64, DriveError> {
    let node = dom::query_selector(selector);
    if node < 0 {
        return Err(DriveError::Failed(answers::element_required(
            verb, selector,
        )));
    }
    Ok(node)
}

/// What a host hands a handler it fires: the event, as one Int, when the handler takes one —
/// `__logos_call_closure(handle, 0)` — and nothing when it takes none.
fn event_args(handler: &RuntimeValue) -> Vec<RuntimeValue> {
    match handler {
        RuntimeValue::Function(c) if c.param_names.is_empty() => Vec::new(),
        _ => vec![RuntimeValue::Int(0)],
    }
}

fn text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(s.to_string()))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/page_host/lowered/host_tests.rs"]
mod host_tests;
