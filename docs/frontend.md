# Frontend — pages, components and the browser

A LOGOS frontend is a set of pages written in LGX. A page is one file: `State:` holds what it
knows, `Actions:` says what it does in English sentences, `View:` is HTML with `{…}` holes, and
`Style:` is CSS scoped to it. Each page compiles to one WebAssembly module for the browser, and the
same source runs in-process on the tree-walker and the bytecode VM, which is where `largo test`
drives it.

This guide builds up in the order an application needs the pieces: a page, state and actions,
lists, components, forms, shared state, triggers, fetching, several pages, styling, testing, and
shipping. The server half — `## Route`, sessions, the database — is the [Web](web.md) guide.

Every example below is a complete page. The documentation suite mounts each one, renders it on the
tree-walker and the VM (which must agree), compiles it for the browser, and runs its `## Test`
blocks, so an example that stops working fails the build.

## Start a project

```bash
largo new mysite --web
cd mysite
largo serve
```

| Path | What it is |
|------|-----------|
| `pages/` | One page per file. `pages/app.lg` is `/` unless it declares another path |
| `static/` | Files served verbatim at the site root; every top-level `.css` is linked into every page |
| `src/main.lg` | The backend — see [Web](web.md) |

`largo new mysite --site` scaffolds `pages/` and `static/` with no backend and no `entry` in
`Largo.toml`: a page mounts itself, so a site has no program to run.

`largo serve` builds every page, serves it with `static/`, and proxies anything else to the
backend; a site with no backend answers anything else `404`, as a static host would. Saving a page
rebuilds it and reloads the browser; a page that does not compile shows the error, with its line, in
place of the page.

## A first page

```logos-dom
## Page /

State:
    Let name be "world".

View:
    <h1 id="greeting">Hello, {name}!</h1>

## Test: the page greets the world

Expect that the text of "#greeting" is equal to "Hello, world!".
```

`## Page /` declares a page and the path it serves at. A page is the root of the document at that
address, so it mounts itself: there is no `## Main` and no `Mount` to write. Without a header the
file name decides — `pages/app.lg` is `/`, `pages/about.lg` is `/about` — and a page that wants a
nested URL says so in its header. A file serves one URL, so a second `## Page` in the same file is
refused.

A `{hole}` holds any LOGOS expression and is escaped wherever it appears, attribute values
included: a hole cannot inject markup.

The `## Test` at the bottom is part of the file. `largo test` runs it against the page; the page
is already mounted when the test starts, so the test only drives it and asserts.

## State, actions and turns

```logos-dom
## Page /

State:
    Let clicks be 0.
    Let label always be "clicked {clicks} times".

Actions:
    To bump:
        Set clicks to clicks + 1.

    To reset:
        Set clicks to 0.

View:
    <button id="more" on-click={bump}>+1</button>
    <button id="reset" on-click={reset}>reset</button>
    <p id="count">{label}</p>

## Test: each click is one turn

Click "#more".
Click "#more".
Expect that the text of "#count" is equal to "clicked 2 times".
Click "#reset".
Expect that the text of "#count" is equal to "clicked 0 times".
```

`State:` is a plain record: each `Let` is a field with an initial value — plain `Let` or `Let
mutable` both work, since `mutable` is optional intent everywhere and a no-op inside `State:`
(every field is changed by `Set` in an action either way). `Let … always be …` declares a
**derived** field, recomputed whenever what it reads changes; nothing has to remember to update it.

`Actions:` holds the only code that changes state. `To bump:` declares an action; `on-click={bump}`
names it as the handler. An action that takes arguments is called with them in the handler:
`on-click={remove(todo's id)}`.

One event is one **turn**: the action runs, derived fields recompute, and the page repaints once.
A turn is a transaction. If an action fails partway, every field it touched returns to its value at
the start of the turn, the failure is delivered to `When an action fails`, and the page stays live:

```logos-dom
## Page /

State:
    Let drinks be ["tea", "coffee"].
    Let chosen be "nothing".
    Let problem be "".

Actions:
    To pick (n: Int):
        Set chosen to "picking".
        Set chosen to item n of drinks.

    When an action fails (error: Text):
        Set problem to "no such drink".

View:
    <button id="first" on-click={pick(1)}>first</button>
    <button id="tenth" on-click={pick(10)}>tenth</button>
    <p id="chosen">{chosen}</p>
    <p id="problem">{problem}</p>

## Test: a failed turn leaves no trace in the state

Click "#tenth".
Expect that the text of "#chosen" is equal to "nothing".
Expect that the text of "#problem" is equal to "no such drink".
Click "#first".
Expect that the text of "#chosen" is equal to "tea".
```

`item 10 of drinks` fails, so the turn's `Set chosen to "picking"` is rolled back with it. There are
no hooks and no dependency arrays: a component's identity is its position in the page, never the
order its code happened to run in.

`Toggle open.` flips a `Bool` field in one sentence.

## Lists and keys

```logos-dom
## A Todo has:
    an id, which is Int.
    a text, which is Text.
    a done, which is Bool.

## To openCount (todos: Seq of Todo) -> Int:
    Let mutable n be 0.
    Repeat for t in todos:
        If t's done is equal to false:
            Set n to n + 1.
    Return n.

## Page /

State:
    Let todos be a new Seq of Todo.
    Let draft be "".
    Let nextId be 1.
    Let left always be openCount(todos).

Actions:
    To add:
        If draft is equal to "":
            Return.
        Push (a new Todo with id nextId and text draft and done false) to todos.
        Set nextId to nextId + 1.
        Set draft to "".

    To toggle (id: Int):
        Let mutable next be a new Seq of Todo.
        Repeat for t in todos:
            If t's id is equal to id:
                Push (a new Todo with id t's id and text t's text and done (not t's done)) to next.
            Otherwise:
                Push t to next.
        Set todos to next.

    To remove (id: Int):
        Remove every t from todos where t's id is equal to id.

View:
    <form id="add" on-submit.prevent={add}>
      <input id="draft" type="text" bind={draft} placeholder="What needs doing?">
      <button type="submit">Add</button>
    </form>
    <ul>
      {for each t in todos, keyed by t's id:}
        <li class="todo" class:done={t's done}>
          <button id="toggle-{t's id}" on-click={toggle(t's id)}>{t's text}</button>
          <button id="remove-{t's id}" on-click={remove(t's id)}>remove</button>
        </li>
      {else:}
        <li class="empty">Nothing to do.</li>
      {end}
    </ul>
    <p id="left">{left} left</p>

## Test: todos are added, finished and removed

Expect that the text of ".empty" is equal to "Nothing to do.".
Type "milk" into "#draft".
Submit "#add".
Type "bread" into "#draft".
Submit "#add".
Expect that the count of ".todo" is equal to 2.
Expect that the text of "#left" is equal to "2 left".
Click "#toggle-1".
Expect that the count of ".done" is equal to 1.
Expect that the text of "#left" is equal to "1 left".
Click "#remove-1".
Expect that the count of ".todo" is equal to 1.
Expect that the text of "#toggle-2" is equal to "bread".
```

`{for each t in todos:}` … `{end}` repeats its body once per item; an `{else:}` arm renders when
the collection is empty. `keyed by t's id` gives each row an identity, so when the list is
reordered or an item is removed, each row's element — with its focus, its selection, and any text
typed into it — moves with its item rather than being rewritten in place. Key any list whose
items can move.

`at index i` binds the item's position. Positions count from 1, as every LOGOS index does:

```logos-dom
## Page /

State:
    Let podium be ["Ada", "Grace", "Barbara"].

View:
    <ol>{for each name in podium, at index i:}<li id="place-{i}">{i}. {name}</li>{end}</ol>

## Test: positions count from one

Expect that the text of "#place-1" is equal to "1. Ada".
Expect that the text of "#place-3" is equal to "3. Barbara".
```

`length of todos` is the number of items; `todos contains x` asks membership. A template tag and an
element may not cross: `{if a:}<p>{end}</p>` is refused, so a `{for each}` goes around the `<li>`,
not half inside it.

## Components

A component is a piece a page places. Its header names it and declares its **props**; the page
places it with a capitalised tag whose attributes are those props:

```logos-dom
## Component Badge (label: Text) and (count: Int)

View:
    <span class="badge">{label}: {count}</span>

## Page /

State:
    Let unread be 3.

View:
    <nav><Badge label="Inbox" count={unread}/> <Badge label="Drafts" count={0}/></nav>

## Test: each badge shows its own props

Expect that the count of ".badge" is equal to 2.
Expect that the text of "nav" is equal to "Inbox: 3 Drafts: 0".
```

Props are read-only inside the component: it cannot `Set` one. A literal attribute (`label="Inbox"`)
passes text; a hole (`count={unread}`) passes any value. A component that keeps state of its own
declares `State:` and `Actions:` exactly as a page does, and each placement gets its own instance.

### Passing actions down

A child changes its parent's state by calling an action the parent handed it. A prop of type
`fn(…) -> Unit` carries one:

```logos-dom
## Component Row (label: Text) and (remove: fn(Text) -> Unit)

View:
    <li class="row">{label} <button id="remove-{label}" on-click={remove(label)}>remove</button></li>

## Page /

State:
    Let names be ["ada", "grace"].

Actions:
    To drop (who: Text):
        Remove every n from names where n is equal to who.

View:
    <ul>{for each n in names, keyed by n:}<Row label={n} remove={drop}/>{end}</ul>

## Test: the child calls the parent's action

Click "#remove-ada".
Expect that the count of ".row" is equal to 1.
Expect that the text of ".row" is equal to "grace remove".
```

The call runs as the parent's turn: the parent's state changes and the whole page repaints once.

### Children

`{children}` marks where the markup a caller writes between a component's tags lands:

```logos-dom
## Component Card (title: Text)

View:
    <section class="card"><h2>{title}</h2>{children}</section>

Style:
    .card { border: 1px solid var(--border); border-radius: var(--radius); padding: 1rem; }

## Page /

State:
    Let tasks be 4.

View:
    <Card title="This week"><p id="tasks">{tasks} tasks</p></Card>

## Test: the caller's markup lands inside the card

Expect that the text of ".card h2" is equal to "This week".
Expect that the text of "#tasks" is equal to "4 tasks".
```

Children are rendered in the scope they were written in: `{tasks}` above is the page's field, even
though it appears inside `Card`.

## Events

Any `on-<event>` handler can carry qualifiers after dots — `on-click.stop={close}`. An event that
bubbles (most do) reaches the target's own listener first, then each listening ancestor's, nearest
first. `.stop` ends that walk right after the listener that carries it, so an ancestor never hears
a click `.stop` already answered. `.once` removes its own listener after the first event, but does
not stop the event bubbling on. `.capture` runs on the way DOWN from the root, before the target's
own listener, rather than after it on the way up. `.passive` promises never to cancel the event's
default action — and asking for both `.prevent` and `.passive` on one handler is refused at compile
time: a passive listener that also demands `.prevent` is a contradiction, not a choice. The full
catalogue, including the keys (`on-keydown.enter={…}`) is in the
[reference](lgx-reference.md#event-modifiers).

```logos-dom
## Page /

State:
    Let outerHits be 0.
    Let innerHits be 0.

Actions:
    To outerClicked:
        Set outerHits to outerHits + 1.

    To innerClicked:
        Set innerHits to innerHits + 1.

    To onceClicked:
        Set innerHits to innerHits + 100.

View:
    <div id="outer" on-click={outerClicked}>
      <button id="bubbles" on-click={innerClicked}>bubbles</button>
      <button id="stopped" on-click.stop={innerClicked}>stops</button>
      <button id="once" on-click.once={onceClicked}>once</button>
    </div>
    <p id="outer-hits">{outerHits}</p>
    <p id="inner-hits">{innerHits}</p>

## Test: a click bubbles unless it is stopped, and `.once` answers only the first

Click "#bubbles".
Expect that the text of "#outer-hits" is equal to "1".
Expect that the text of "#inner-hits" is equal to "1".
Click "#stopped".
Expect that the text of "#outer-hits" is equal to "1".
Expect that the text of "#inner-hits" is equal to "2".
Click "#once".
Click "#once".
Expect that the text of "#inner-hits" is equal to "102".
```

`#bubbles` fires its own handler and then `#outer`'s. `#stopped` carries `.stop`, so `#outer` never
hears it — `outer-hits` stays at 1. `#once` answers its first click and is then a plain button with
no handler of its own, so the second click adds nothing more to `inner-hits` (the walk still bubbles
past it to `#outer`, which is why `.once` is about the listener, not the event).

## Forms

`bind={field}` connects a control to a state field in both directions: typing writes the field,
and a turn that sets the field updates the control. The page keeps focus and the cursor while the
visitor types, because a repaint patches only the nodes that changed.

```logos-dom
## Page /

State:
    Let name be "".
    Let size be "medium".
    Let gift be false.
    Let note be "".

View:
    <input id="name" type="text" bind={name}>
    <select id="size" bind={size}>
      <option>small</option>
      <option>medium</option>
      <option>large</option>
    </select>
    <label><input id="gift" type="checkbox" bind={gift}> gift wrap</label>
    <textarea id="note" bind={note}></textarea>
    <p id="order">{name}: {size}{if gift:}, wrapped{end}</p>

## Test: every control writes its field

Type "Ada" into "#name".
Choose "large" in "#size".
Click "#gift".
Expect that the text of "#order" is equal to "Ada: large, wrapped".
Expect that "#gift" is checked.
```

A checkbox binds to a `Bool` and renders as a **presence** — `checked`, never `checked="true"`.
The same holds for `disabled`, `readonly` and every other boolean attribute: `disabled={false}`
renders no attribute at all, because a browser treats `disabled="false"` as disabled.

### Submitting, and saying what was wrong

`on-submit.prevent={save}` handles the submit and stops the browser's own navigation.
[`forms.lg`](../crates/logicaffeine_compile/assets/std/forms.lg) collects what was wrong with a
submission: each check takes the complaints so far and returns them with its own added. The form
below carries `novalidate` — it writes its own messages, so the browser's built-in checks (the next
section) are turned off rather than racing this one to answer first.

```logos-dom
## Page /signup

State:
    Let email be "".
    Let password be "".
    Let errors be a new Seq of FieldError.
    Let welcomed be false.

Actions:
    To submit:
        Let mutable e be noErrors().
        Set e to checkRequired(e, "email", email).
        Set e to checkEmail(e, "email", email).
        Set e to checkMinLength(e, "password", password, 8).
        Set errors to e.
        If isValid(e):
            Set welcomed to true.

View:
    <form id="signup" novalidate on-submit.prevent={submit}>
      <input id="email" type="email" bind={email}>
      <p id="email-error">{errorFor(errors, "email")}</p>
      <input id="password" type="password" bind={password}>
      <p id="password-error">{errorFor(errors, "password")}</p>
      <button type="submit">Sign up</button>
    </form>
    {if welcomed:}<p id="welcome">Welcome aboard.</p>{end}

## Test: a bad submission names each problem

Type "ada" into "#email".
Type "short" into "#password".
Submit "#signup".
Expect that the text of "#email-error" is equal to "This does not look like an email address.".
Expect that the text of "#password-error" is equal to "Use at least 8 characters.".
Expect that "#welcome" is hidden.

## Test: a good submission is accepted

Type "ada@example.com" into "#email".
Type "correct horse" into "#password".
Submit "#signup".
Expect that the text of "#welcome" is equal to "Welcome aboard.".
```

The complaints are a `Seq of FieldError` — a `field` and a `message` — rather than a map, so
`errorFor(errors, "email")` answers `""` for a field with no complaint instead of failing on a
missing key. The first check to fail on a field is the one kept, so order checks from general to
particular.

| Function | What it does |
|----------|--------------|
| `noErrors()` | An empty complaint list — the start of every validation |
| `checkRequired(errors, field, value)` | Complains when `value` is empty or only whitespace |
| `checkMinLength(errors, field, value, n)` / `checkMaxLength(…)` | Length after trimming |
| `checkRange(errors, field, value, low, high)` | An `Int` from `low` to `high`, both included |
| `checkSame(errors, field, a, b, message)` | Complains when `a` and `b` differ — a password confirmation |
| `checkEmail(errors, field, value)` | Complains when a non-empty `value` is not shaped like an address |
| `addError(errors, field, message)` | Records a complaint of your own; a field keeps its first |
| `isValid(errors)` · `hasError(errors, field)` · `errorFor(errors, field)` | Reading the result |
| `errorSummary(errors)` | Every message, one per line, for a summary above the form |
| `looksLikeEmail(value)` · `isBlank(value)` | The predicates underneath |

`checkEmail` checks the shape — one `@`, something before it, a dotted domain after it — and not
the address; only a message that arrives proves an address works. A form that posts to a server
revalidates there: the browser's checks are for the visitor, the server's are for the data.

### Native validation

`required`, `pattern`, `min`/`max` and the shape a `type="email"`/`type="number"`/… input already
implies are the browser's OWN checks, matched on every tier. A submit that fails them never runs
`on-submit`'s action at all: instead `on-invalid={action}` fires once per failing control and focus
moves to the first of them. `novalidate` on the `<form>` turns this off — the shape the section
above uses, since a form writing its own messages does not want the browser's silent check racing
it to answer first.

```logos-dom
## Page /join

State:
    Let submitted be 0.
    Let invalids be 0.

Actions:
    To join:
        Set submitted to submitted + 1.

    To flagged:
        Set invalids to invalids + 1.

View:
    <form id="join" on-submit.prevent={join}>
      <input id="name" required on-invalid={flagged}>
      <button type="submit">Join</button>
    </form>
    <p id="counts">{submitted} {invalids}</p>

## Test: a required field blocks the submit until it is filled

Submit "#join".
Expect that the text of "#counts" is equal to "0 1".
Type "Ada" into "#name".
Submit "#join".
Expect that the text of "#counts" is equal to "1 1".
```

A radio group's `required` is satisfied by any one member being checked, not every member; a
`disabled` or `readonly` control is exempt from every check, since a visitor cannot fix what they
cannot touch.

## Shared state: stores

A `## Store` is state and actions with no view. Components `Use` it, read its fields with a
possessive, and change it only by calling its actions:

```logos-dom
## Store Cart persisted as "cart"

State:
    Let count be 0.

Actions:
    To add:
        Set count to count + 1.

    To empty:
        Set count to 0.

## Component CartBadge

Use Cart.

View:
    <span id="badge">{Cart's count} in the cart</span>

## Page /

Use Cart.

Actions:
    To buy:
        Call Cart's add.

View:
    <header><CartBadge/></header>
    <button id="buy" on-click={buy}>Buy</button>

## Test: every reader sees the store change

Click "#buy".
Click "#buy".
Expect that the text of "#badge" is equal to "2 in the cart".
```

A store's fields change only through its own actions: `Call Cart's add.` Arguments follow `with`:
`Call Cart's addItem with "tea" and 2.`

How long a store's state outlives the page is part of its header:

| Header | Lifetime |
|--------|----------|
| `## Store Cart` | This page load |
| `## Store Cart preserved across reloads` | The tab: a reload keeps it, closing the tab ends it |
| `## Store Cart persisted as "cart"` | Until cleared: kept under the key `"cart"` in local storage |
| `## Store Room shared on "room"` | Every page and tab on the topic `"room"` — see [Stores shared across pages](#stores-shared-across-pages) |

A kept store writes each field under its own key, so a value holding quotes, newlines or emoji
round-trips untouched. A field the edited page no longer declares is not read back, and a field it
has just added starts from its initializer, so a store changes shape without a migration. A kept
store holds any value a page's own `State:` can — `Text`, `Int`, `Real`, `Bool`, a list, or a
record — each field encoded through the same derived JSON codec `Save`/`Load` use.

The same storage is available as functions. `storageSet(area, key, value)` writes text,
`storageRemove(area, key)` clears it, `storageGetOr(area, key, fallback)` reads it or the fallback,
and `storageGet(area, key)` answers an `Option of Text`, which tells *nothing is saved* apart from
*an empty string is saved*. The area is `"local"` (survives closing the tab) or `"session"` (ends
with the tab); any other area is refused by name. Inside an action, `Save value as "key".`,
`Load "key" into field.` and `Forget "key".` do the same for one field.

### Stores shared across pages

A store declared `shared on "topic"` is one state for every page and tab on that topic:

```logos-dom
## Store Room shared on "room"

State:
    Let visitors: Tally be 0.
    Let topic be "none yet".

Actions:
    To arrive:
        Increase visitors by 1.

    To retitle (t: Text):
        Set topic to t.

## Page /

Use Room.

Actions:
    When the page loads:
        Call Room's arrive.

    To pick:
        Call Room's retitle with "lunch".

View:
    <p id="here">{Room's visitors} here</p>
    <p id="topic">{Room's topic}</p>
    <button id="pick" on-click={pick}>pick</button>

## Test: the page joins the room and writes through its actions

Expect that the text of "#here" is equal to "1 here".
Click "#pick".
Expect that the text of "#topic" is equal to "lunch".
```

The replicas meet through the relay the page's own server answers at `/_relay` — `largo serve`, and
a backend that serves pages using it. A page's replica joins the topic the first time something
reads one of the store's fields, adopting what the topic already holds, and every action publishes
what it changed.

A plain field is a last-writer-wins register: each write carries the page's Lamport time, the
greater time wins and the value breaks a tie, so every page settles on the same write. A field typed
`Tally`, `ConvergentCount`, `SharedSet of T` or `SharedSequence of T` is a CRDT that keeps every
page's concurrent writes. The store's actions see the plain value it converges to — an `Int`, a
`Set of T`, a `Seq of T` — so `Increase`, `Add`, `Remove` and `Push` read as they do anywhere, and the
page's operations are read off how that value moved. A `Tally` counts both ways; a
`ConvergentCount` only up, and an action that lowers one fails. A set add concurrent with a remove of
the same element wins; concurrent appends to a sequence all survive, in one order every page
derives. A convergent type outside a `shared on` store is refused, since nothing there is shared.

## Triggers

An `Actions:` sentence that starts with `When`, `Every` or `After` runs when the thing it names
happens, and at no other time. Each run is a turn like a click.

```logos-dom
## Page /

State:
    Let seconds be 0.
    Let query be "".
    Let searches be 0.
    Let online be true.

Actions:
    When the page loads:
        Set seconds to 0.

    Every 1 second as clock:
        Set seconds to seconds + 1.

    To pause:
        Stop the timer clock.

    When query changes:
        Set searches to searches + 1.

    When the app goes offline:
        Set online to false.

    When the app comes back online:
        Set online to true.

View:
    <p id="clock">{seconds}s</p>
    <button id="pause" on-click={pause}>pause</button>
    <input id="q" type="search" bind={query}>
    <p id="searches">{searches}</p>
    {if online:}<p id="status">online</p>{else:}<p id="status">offline</p>{end}

## Test: the clock ticks until it is stopped

Advance time by 3 seconds.
Expect that the text of "#clock" is equal to "3s".
Click "#pause".
Advance time by 5 seconds.
Expect that the text of "#clock" is equal to "3s".

## Test: a field change runs its sentence

Type "tea" into "#q".
Expect that the text of "#searches" is equal to "1".

## Test: the page hears the network go and come back

Raise "offline".
Expect that the text of "#status" is equal to "offline".
Raise "online".
Expect that the text of "#status" is equal to "online".
```

A timer belongs to its component: when the component leaves the page, its timers stop with it.
Only a named timer (`Every 1 second as clock:`) can be stopped by `Stop the timer clock.`, and
`Start the timer clock.` arms it again from now, reading its period afresh; the compiler refuses a
stop or a start that names no timer the component declares. `After 5 seconds:` runs once.

A component can react to arriving and leaving:

```logos-dom
## Component Toast (message: Text)

State:
    Let shown be 0.

Actions:
    When this component appears:
        Set shown to shown + 1.

View:
    <p class="toast">{message} ({shown})</p>

## Page /

State:
    Let open be true.
    Let closed be 0.

Actions:
    To dismiss:
        Toggle open.
        Set closed to closed + 1.

View:
    {if open:}<Toast message="Saved"/>{end}
    <button id="dismiss" on-click={dismiss}>dismiss</button>
    <p id="closed">{closed}</p>

## Test: a component leaves with its state

Expect that the text of ".toast" is equal to "Saved (1)".
Click "#dismiss".
Expect that ".toast" is hidden.
Expect that the text of "#closed" is equal to "1".
```

Two pages of an app — or two tabs — talk over a topic. `Publish message on "topic".` sends, and
every `When a message arrives on "topic" (m: T):` hears it exactly once, including the sender's
own page:

```logos-dom
## Page /

State:
    Let last be "none".

Actions:
    To ping:
        Publish "hello" on "chat".

    When a message arrives on "chat" (m: Text):
        Set last to m.

View:
    <button id="ping" on-click={ping}>ping</button>
    <p id="last">{last}</p>

## Test: a published message is heard

Click "#ping".
Expect that the text of "#last" is equal to "hello".
```

A drop hands its handler a `File`: `name`, `size` (its content's length, in bytes — a browser's own
`File.size`), `type` (its MIME type), and `bytes` (its content, a `Seq of Int` with one element per
byte). A browser reads the file before the drop's turn runs, so the turn always has the content in
hand, on every tier a real browser included. The handler's own parameter DECLARES what it wants:
`(file: File)` binds the whole record, `(name: Text)` binds just the name, for a handler that never
needed the rest.

```logos-dom
## Page /

State:
    Let name be "".
    Let size be 0.
    Let kind be "".

Actions:
    When a file is dropped on "#zone" (file: File):
        Set name to file's name.
        Set size to file's size.
        Set kind to file's type.

View:
    <div id="zone">drop a file here</div>
    <p id="name">{name}</p>
    <p id="size">{size}</p>
    <p id="kind">{kind}</p>

## Test: a dropped file's name, size and type arrive

Raise "filedropped" on "#zone" with a new File with name "photo.png" and type "image/png" and bytes [137, 80, 78, 71].
Expect that the text of "#name" is equal to "photo.png".
Expect that the text of "#size" is equal to "4".
Expect that the text of "#kind" is equal to "image/png".
```

The full set of trigger sentences is in the [reference](#triggers-reference) below.

## Fetching

A page asks the server for typed data. The answer arrives as a `Result`, decoded by the JSON codec
derived from the record type — the same codec the route that answers it encodes with:

```logos-dom
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /

State:
    Let todos be a new Seq of Todo.
    Let status be "idle".

Actions:
    To load:
        Set status to "loading".
        Get "/api/todos" as Seq of Todo, then loaded.

    To loaded (answer: Result of Seq of Todo and HttpError):
        Inspect answer:
            When Ok (list):
                Set todos to list.
                Set status to "loaded".
            When Err (e):
                Set status to "failed".

View:
    <button id="load" on-click={load}>Load</button>
    <p id="status">{status}</p>
    <ul>{for each t in todos, keyed by t's id:}<li>{t's text}</li>{end}</ul>

## Test: the list arrives and is shown

Given that GET "/api/todos" answers 200 with "[{{\"id\": 1, \"text\": \"milk\"}}]".
Click "#load".
Expect that the text of "#status" is equal to "loaded".
Expect that the text of "li" is equal to "milk".

## Test: a server error is a value, not a crash

Given that GET "/api/todos" answers 500 with "down".
Click "#load".
Expect that the text of "#status" is equal to "failed".
Expect that the count of "li" is equal to 0.
```

`Get URL as T, then action.` sends the request and returns; when the answer arrives, `action` runs
as its own turn with a `Result of T and HttpError`. A status outside 200–299, a body that does not
decode as `T`, and a request that never connected are all `Err`, with `e's message` saying which.
`Post body to URL as T, then action.` sends a body, and `Put`, `Patch` and `Delete` read the same
way; `Upload file to URL as T, then action.` sends `multipart/form-data`. A `Post`/`Put`/`Patch`/
`Delete` to the page's OWN server carries its visitor's CSRF token automatically, the way a `<form>`
submit does — nothing in the page adds it (see [Sessions and CSRF](web.md#csrf) on the server side).

In a `## Test`, `Given that METHOD "url" answers STATUS with BODY.` answers a request without a
network, matched on the method and the URL together. A brace in text opens a hole, so JSON in a
test doubles its braces.

`When a fetch fails (url: Text) and (error: Text):` hears every failed request a page makes, for a
page-wide error banner; the `Err` arm of each answer handles the specific one.

### The request functions underneath

The typed sentences are built on [`web.lg`](../crates/logicaffeine_compile/assets/std/web.lg).
`webGet`, `postJson`, `putJson` and `deleteAt` start a request and return its id; every part of
the answer is read by that id — `responseStatus`, `responseBody`, `responseHeader`,
`responseError` — so two requests in flight never deliver into each other. `fetchAbort` and
`fetchSetTimeout` control an in-flight request by the same id. `postJsonIdempotent` sends an
idempotency key so a retried create applies once, and `putJsonExpecting` sends an expected version
so a write against stale data is refused instead of overwriting a concurrent one. Neither retries on
its own.

## Pages, layouts and navigation

### A page's URL and parameters

`## Page /todos/:id (id: Int)` serves every URL of that shape and binds the segment, typed: the
page reads `id` like a field — `{id + 1}` is arithmetic, not text. A parameter the path has no
`:name` for is refused, because nothing could ever fill it.

### A page's head

`Head:` is the head HTML a page declares for its own document — the tab title, the search
description, the card a shared link becomes:

```logos-dom
## Page /about

Head:
    <title>About — Acme</title>
    <meta name="description" content="Who we are and why.">
    <meta property="og:title" content="About Acme">
    <link rel="canonical" href="https://acme.example/about">
    <html lang="en"></html>

View:
    <main><h1 id="title">About</h1></main>

## Test: the page renders under its head

Expect that the text of "#title" is equal to "About".
```

It is rendered into the document the server sends and merged over a default set — charset,
viewport, a light and dark `theme-color`, a favicon — tag by tag: a `<title>` of yours replaces the
default one and anything else is added. `<html lang="fr"></html>` sets the document's language,
which screen readers, translation and hyphenation all read. Only a `## Page` carries a `Head:`;
`<title>`, `<meta>`, `<link>` and `<base>` are the elements it accepts. A title that changes while
the page is open is `Set the title to "…".`, not a hole.

### Layouts

A layout is a component with `{block}` slots. A page fills them by extending it:

```logos-dom
## Component Site

View:
    <header><nav><a href="/">Home</a> <a href="/about">About</a></nav></header>
    <main>{block content}</main>
    <footer>{block footer:}© Acme{end}</footer>

## Page /about

State:
    Let team be ["Ada", "Grace"].

View:
    {extends Site}
    {block content:}<h1>About</h1><p id="team">{length of team} people</p>{end}

## Test: the page fills the layout

Expect that the text of "#team" is equal to "2 people".
Expect that the text of "footer" is equal to "© Acme".
```

`{extends Site}` comes first in the view. A `{block name:}` … `{end}` in the layout carries its
default; a page that does not fill it gets the default. Slot content renders in the scope it was
written in, so `{length of team}` reads the page's field. Pages share a layout by importing the file
that declares it: put `## Component Site` in `src/layout.lg` and start each page with
`Import Layout.`

### Navigation

A link is `<a href="/about">`; an action navigates with `Navigate to "/about".`;
`Replace the URL with "/about".` changes the address without a new history entry, and `Go back.`
steps back one. `When the route changes:` runs on every navigation that stays in the page,
including the visitor's Back button — but not every navigation does.

A move to a path THIS page's own `## Page` route serves stays in the page: the address changes with
no reload, and every live `When the route changes:` hears it. A move to a path a DIFFERENT page
serves is a real document load — this page and everything it holds is gone, replaced by whatever
that page renders. A clicked `<a href>` follows the identical rule with no code of yours: the page
checks whether it owns the target before the browser ever gets to navigate, so an in-page link and
a cross-page link need no different markup.

```logos-dom
## Page /photos/:n

State:
    Let moves be 0.

Actions:
    To next:
        Navigate to "/photos/2".

    When the route changes:
        Set moves to moves + 1.

View:
    <button id="next" on-click={next}>{moves}</button>

## Test: navigating within the page's own route

Click "#next".
Expect that the URL is equal to "/photos/2".
Expect that the text of "#next" is equal to "1".
```

`/photos/:n` is one compiled page serving every photo, so `Navigate to "/photos/2"` is still THIS
page under a different segment — the move is a pushState and the page's own route listener hears
it. A move to another page entirely leaves nothing here to assert but the address itself:

```logos-dom
## Page /

Actions:
    To leave:
        Navigate to "/about".

View:
    <button id="leave" on-click={leave}>About</button>

## Test: leaving for another page file

Click "#leave".
Expect that the URL is equal to "/about".
```

For reading a URL by hand, `routePath` and `routeQuery` answer the current path and query,
`navigate` and `onRoute` are the functions under the sentences, `matchRoute(pattern, path)` tests a
path against a pattern such as `/todos/:id`, `routeParam(pattern, name, path)` pulls one named
segment out, and `segmentMatches` is the single-segment comparison both are built on.

### When a page is not there

```logos-dom
## Error page 404

View:
    <main>
      <h1>Nothing here</h1>
      <a href="/">Back home</a>
    </main>

```

An error page is served, not mounted: the running server answers a URL nothing matches with it, as
HTML, in place of the bare status, and `largo build --static` publishes it as `404.html`, the file
every static host serves for a URL it cannot map. `## Error page 405` is the page for a method a
route does not take. Declare none and none is invented.

## Styling

A `Style:` block is real CSS scoped to its component: its rules cannot reach a child's elements or
leak out to the page, so `.card` means this component's cards.

```logos-dom
## Component Tag (label: Text) and (hot: Bool)

View:
    <span class="tag" class:hot={hot}>{label}</span>

Style:
    .tag { padding: 0 .4rem; border-radius: var(--radius); background: var(--paper); }
    .hot { background: var(--accent); color: white; }

## Page /

State:
    Let tone be "quiet".

View:
    <p class="line {tone}"><Tag label="new" hot={true}/> <Tag label="old" hot={false}/></p>

## Test: the toggle and the interpolated class compose

Expect that the count of ".hot" is equal to 1.
Expect that the count of ".line.quiet" is equal to 1.
```

`class:hot={hot}` adds the word `hot` while the expression is true and composes with a plain
`class="…"`. A hole inside `class="line {tone}"` interpolates a word. An action can also change a
class on an element directly: `Add class "open" to "#menu".` and `Remove class "open" from
"#menu".` work on whole words, never substrings — removing `act` from `active act` leaves `active`.
The word logic is compiled LOGOS — `classListWith` and `classListWithout` in `web.lg`, which the
compiler calls for these two sentences — so it is the same on every tier.

Site-wide styling lives in `static/`. Every top-level `.css` there is linked into every page;
`largo new --web` and `largo new --site` write `theme.css`, holding the design tokens (`--accent`,
`--ink`, `--paper`, `--muted`, `--border`, `--radius`, `--shadow`) and their dark-mode values, and
`style.css` for the site's own rules. The theme is linked
first, so a rule of yours overrides it, and overriding a token is one line:

```css
:root { --accent: #cc785c; }
@media (prefers-color-scheme: dark) { :root { --ink: #f2f0e8; } }
```

A `var(--name)` with no definition and no fallback silently invalidates the whole declaration, so a
colour that is mysteriously missing is usually a misspelt token.

Every page ships a built-in theme: design tokens on `:root` (`--paper`, `--ink`, `--accent`,
`--muted`, `--border`, `--radius`, `--shadow`, `--surface`, `--accent-hover`, `--accent-contrast`,
`--font-sans`, `--font-mono`, `--measure`, `--space`), element styles (headings, buttons, inputs,
tables, `.card`), and a dark mode. The scaffold's `static/theme.css` uses the same token names, so
changing one there reskins everything; a component's `Style:` rules win over the theme's element
rules. A site that brings its whole design opts out in `Largo.toml`: `[web]` then `theme = false`.

## Directives

A `## Directive` gives an attribute a behaviour: it receives the element it sits on and the
attribute's value, runs when the element appears and whenever the value changes, and its
`When removed:` block runs when the element leaves.

```logos-dom
## Directive tooltip (el: Element) and (text: Text):
    Call setAttribute with el and "title" and text.

## Page /

State:
    Let hint be "delete this row".

View:
    <button id="delete" tooltip={hint}>×</button>

## Test: the directive ran against its element

Expect that the text of "#delete" is equal to "×".
```

A directive is how an attribute reaches a browser API the framework does not wrap; the element it
receives is the real one.

## Testing a page

A `## Test` in a page file drives the page the way a visitor would and asserts on what is on
screen. Each test starts from a freshly mounted page, and a test that asserts nothing fails.

**Drive** — `Click "#sel".` · `Type "text" into "#sel".` · `Submit "#form".` ·
`Choose "value" in "#select".` · `Press "Enter" in "#sel".` (or `Press "Escape".` for the page) ·
`Focus "#sel".` · `Blur "#sel".` · `Hover "#sel".` · `Raise "offline".` ·
`Reload the page.` · `Advance time by 5 seconds.` ·
`Given that GET "/api/x" answers 200 with "…".` · `Serve fetches with routes.` ·
`Replay these actions: add, remove.`

`Raise "<event>" [on "<sel>"] [with <value>].` drives anything the page hears from the browser
itself rather than from an element a visitor touched: `Raise "online".` / `Raise "offline".` /
`Raise "resize".` for `When the app comes back online:` / `goes offline:` / `When the window
resizes:`; `Raise "scrolledtobottom".` (or `Raise "scrolledtobottom" on "#feed".` for `When the user
scrolls to the bottom of "#feed":`) for `When the user scrolls to the bottom[ of "#sel"]:`;
`Raise "filedropped" on "#zone" with "notes.txt".` for `When a file is dropped on "#zone" (…):` —
the target selector is the element the trigger listens on, and `with` carries the value a trigger
that binds a parameter reads. `Publish "hello" on "room".` drives `When a message arrives on
"room" (…):` the same way a real `Publish` action would, including the page's own topic.

`Reload the page.` loads the page again on every tier: every instance, listener and mounted root is
torn down and the page mounts afresh, its `State:` back at its initializers, while browser storage —
and with it every store `persisted as` a key or `preserved across reloads` — stays.

`Replay these actions: add, remove.` runs the named actions in order, each with no arguments and
each a real turn: journalled, rolled back if it fails, with the replay carrying on past a failure.
A bug report's sequence of actions becomes a test by naming them. An action the page does not
declare is refused by name, and so is a replay before anything is mounted.

**Ask** — `the text of "#sel"` · `the value of "#input"` · `the count of ".row"` ·
`"#sel" is visible` / `is hidden` / `is disabled` / `is checked` / `is focused` · `the title` ·
`the URL` · `the journal` (the actions that ran, in order) · `the journal shows add then remove`

A selector is a descendant chain of compounds, each an optional tag with any number of `#id`,
`.class`, `[attr]` and `[attr=value]` qualifiers. A child or sibling combinator, a comma, a
pseudo-class and `*` are refused by name rather than read: a selector the engine cannot evaluate
would match nothing, and `Expect that the count of "ul > li" is equal to 0.` would pass with the
list full.

A component can be tested on its own by mounting it in the test — the one place `Mount` belongs in
a page file:

```logos-dom
## Component Stepper

State:
    Let n be 1.

Actions:
    To increment:
        Set n to n + 1.

    To decrement:
        Set n to n - 1.

View:
    <button id="less" on-click={decrement}>-</button>
    <output id="n">{n}</output>
    <button id="more" on-click={increment}>+</button>

## Test: the stepper counts both ways

Mount Stepper.
Click "#more".
Click "#more".
Click "#less".
Expect that the text of "#n" is equal to "2".
Expect that the journal shows increment then decrement.

## Test: a recorded sequence replays as turns

Mount Stepper.
Replay these actions: increment, increment, decrement.
Expect that the text of "#n" is equal to "2".
Expect that the journal shows increment then decrement.
```

`largo test` runs every test in the project, `pages/` included; `largo check` compiles every page
for the browser and reports accessibility warnings; `largo fmt` formats them.

A test that works a page — a `## Page`'s own test, or one that mounts a component — runs on more than
one tier. The reference is the page's lowered LOGOS on the tree-walker; the same test runs again on
that LOGOS compiled to the bytecode VM, the program a browser's WebAssembly is built from, and the
two must agree statement by statement: a divergence fails the test and names the first statement
where the documents differ. `largo test --browser` adds a real headless Chrome as a third tier
(`CHROME_BIN`, else the first Chrome or Chromium on `PATH`); no Chrome found is a refused run, not a
skipped one. The `LOGOS_TIERS_RAN:` line the run ends with names the page tiers that ran.

## Building and deploying

`largo serve` is the development loop. For production there are two builds.

```bash
largo build --static
```

writes `dist/`: every page pre-rendered to HTML, each page's content-addressed module and shim,
everything in `static/`, `robots.txt`, `sitemap.xml` (when `Largo.toml`'s `[package]` names the
address the site is served at, `site = "https://acme.example"`) and a `_headers` file carrying the
cache rules. Any static host serves it — GitHub Pages, Netlify, Cloudflare Pages, S3. A page at
`/about` is written as `about/index.html`. A site with no backend has no program to compile, so a
plain `largo build` exports it the same way.

```bash
largo build --release
PORT=8080 ./target/release/mysite
```

packs the pages, the static files and the backend into one native binary; [Web](web.md) covers
running it in production.

A built page ships with its markup already in it, so a visitor sees the content before the module
loads and a crawler that runs no scripts reads the page. The pre-render writes elements, text,
`{if}`, `{for each}`, `{let}`, `{inspect}`, placed components with their children, `bind`, `class:`,
handlers and scoped `Style:`. It does not write `{raw}`, a directive, `{extends}` or `{block}`: a
page using one ships its shell and fills in when its module mounts.

## Reference

### The HTML a view accepts

A `View:` is strict HTML — closer to XHTML than to what a browser forgives. A browser repairs
`<p>one<p>two` by inferring the first `</p>`; a view refuses it and names the element left open,
because markup whose meaning depends on a repair algorithm cannot compile to one tree every tier
agrees on.

- **Every element closes itself.** `</p>` closing a `<div>` is an error naming the innermost open
  element.
- **Void elements never close.** `area base br col embed hr img input link meta param source track
  wbr` take no end tag and no children.
- **Elements and template tags nest; they do not cross.**
- **Attributes** are quoted, unquoted or valueless. A boolean attribute's state is its presence.
- **Character references** follow HTML: the full named list and the numeric forms. Text that is not
  a complete reference is kept as typed, so `Tom & Jerry` needs no escaping.
- **Comments** `<!-- … -->` are dropped.
- **`<script>`, `<style>` and a doctype are refused**: a view is a fragment, styles belong in
  `Style:`, and the document shell belongs to a layout.
- **Whitespace** that is only whitespace and contains a newline is formatting and is dropped; other
  runs collapse to one space. `<pre>` and `<textarea>` are kept verbatim.
- **A capitalised tag is a component** and its attributes are that component's props.

### Template tags

| Tag | Meaning |
|-----|---------|
| `{expr}` | A hole: the value, escaped |
| `{expr \| f a b}` | A filter: `f(expr, a, b)`; filters chain left to right |
| `{if c:}` … `{else if d:}` … `{else:}` … `{end}` | Conditional |
| `{for each x in xs[, keyed by k][, at index i]:}` … `{else:}` … `{end}` | Repetition, with an empty arm |
| `{inspect v:}{when Pattern:}` … `{end}` | Match on a variant |
| `{let name be expr}` | A local binding for the rest of the enclosing scope |
| `{children}` | Where a caller's markup lands |
| `{extends Layout}` · `{block name:}` … `{end}` · `{block name}` | Layout slots |
| `{raw expr}` | Unescaped markup — for text you have made safe yourself |
| `{# comment #}` | Dropped |

### Directives on elements

| Attribute | Meaning |
|-----------|---------|
| `on-<event>={action}` · `on-<event>={action(args)}` | Handle a DOM event with an action |
| `on-<event>.prevent={…}` | Also cancel the browser's default behaviour |
| `on-keydown.enter={…}` | Only for that key |
| `bind={field}` | Two-way binding on `input`, `textarea`, `select`, checkbox and radio |
| `class:name={condition}` | Add the class `name` while the condition holds |
| `name={expr}` · `name="text {expr}"` | A computed attribute; escaped |
| `<Component prop={expr}>` | A component placement; attributes are props |

### Triggers reference

| Sentence | Runs |
|----------|------|
| `When the page loads:` | Once, before the first paint |
| `When this component appears:` · `When this component disappears:` | When the component enters or leaves the page |
| `When field changes:` · `When Store's field changes:` | After a turn changes that field |
| `When "#sel" is clicked:` (`submitted`, `focused`, `blurred`, `hovered`, `changed`, `typed`, `keyed`) | On that event at that element |
| `When the user types in "#sel":` | On each input event |
| `When the user presses Escape:` · `When the user presses Enter in "#sel":` | On that key, on the page or at an element |
| `Every 5 seconds:` · `Every 5 seconds as name:` | Repeatedly; a named timer can be stopped |
| `After 5 seconds:` · `After 5 seconds as name:` | Once |
| `When an action fails (error: Text):` | After a turn fails and rolls back |
| `When a fetch fails (url: Text) and (error: Text):` | After any request fails |
| `When a message arrives on "topic" (m: T):` | On each message published on the topic |
| `When the route changes:` | After every navigation |
| `When the app goes offline:` · `When the app comes back online:` | On the network changing |
| `When the window resizes:` | On a resize |
| `When the user scrolls to the bottom:` · `… of "#list":` | On reaching the end of the page or an element |
| `When a file is dropped on "#zone" (file: File):` · `… (name: Text):` | On a drop; the parameter picks the whole `File` or its name |

### Action verbs

| Sentence | Does |
|----------|------|
| `Set field to value.` · `Toggle field.` | Change state |
| `Call Store's action [with a and b].` | Change a store |
| `Get url as T, then action.` · `Post body to url as T, then action.` (`Put`, `Patch`, `Delete`, `Upload`) | A typed request |
| `Navigate to url.` · `Replace the URL with url.` · `Go back.` | History |
| `Reload the page.` | Load the page again; browser storage stays |
| `Set the title to text.` | The document title |
| `Save value as "key".` · `Load "key" into field.` · `Forget "key".` | Browser storage |
| `Focus "#sel".` · `Blur "#sel".` · `Scroll to "#sel".` · `Select the text in "#sel".` | An element |
| `Add class "c" to "#sel".` · `Remove class "c" from "#sel".` | One class word |
| `Copy text to the clipboard.` · `Download bytes as "name.ext".` · `Notify message [as success].` (or `as error`) | The browser's own surfaces |
| `Publish message on "topic".` | Send on a topic |
| `Stop the timer name.` · `Start the timer name.` | Cancel a named timer, or arm it again from now |

### Filters

A filter is any function whose first parameter takes the hole's value: `{title | truncate 20}` is
`truncate(title, 20)`. [`ui.lg`](../crates/logicaffeine_compile/assets/std/ui.lg) holds the ones
a list view needs:

```logos-dom
## Page /

State:
    Let items be 41.
    Let title be "A very long headline that will not fit".

View:
    <p id="count">{items | plural "item" "items"}</p>
    <h2 id="title">{title | truncate 12}</h2>
    <p id="pages">page 1 of {pageCount(items, 10)}</p>
    <p id="who">{"Ada Lovelace" | initials}</p>

## Test: the list helpers format the view

Expect that the text of "#count" is equal to "41 items".
Expect that the text of "#title" is equal to "A very long…".
Expect that the text of "#pages" is equal to "page 1 of 5".
Expect that the text of "#who" is equal to "AL".
```

| Function | What it answers |
|----------|-----------------|
| `plural(n, one, many)` | `1 item`, `2 items` |
| `truncate(text, n)` | At most `n` characters, ending in `…` when cut |
| `initials(name)` | The first letter of each word |
| `pageCount(total, perPage)` | Pages needed, never 0 |
| `clampPage(page, total, perPage)` | A page number brought into range |
| `pageStart(page, perPage)` · `pageEnd(page, perPage, total)` | The 1-based item range of a page |

`upper` and `lower` change case: `{name | upper}`.

## Where this is tested

Every page in this guide is mounted, rendered on the tree-walker and the VM, compiled for the
browser, and has its tests run by `canonical_teaching_lock`. The behaviour the guide describes is
pinned by `page_renders_lock` (a page mounts itself), `lgx_constructs_lock` (every template
construct and the page it renders), `lgx_loop_index_lock` (positions count from 1),
`component_test_verbs_lock` (the drive verbs and page queries), `compile_replay_lock`,
`page_reload_lock`, `selector_subset_lock`, `form_verbs_lock` and `forms_lock`, `store_lock`,
`shared_store_lock` and `shared_store_crdt_lock` (stores shared on a topic),
`event_vocabulary_lock` and `host_delivered_triggers_lock` (every trigger sentence),
`typed_fetch_lock`, `layout_lock`, `directive_lock`, `style_scope_lock`, `testrun_page_tiers_lock`
and `browser_tier_lock` (a page test's tiers), and `lgx_vm_parity_lock`, which holds the tree-walker
and the VM to the same document.

---
[Docs index](README.md) · [Web](web.md) · [The largo CLI](cli.md)
