# The web standard library — forms, ui, session

Three modules of the standard library exist for web programs: `forms.lg` checks what a visitor
typed and says what was wrong, `ui.lg` does the page arithmetic and the wording a list view needs,
and `session.lg` signs the cookie a signed-in visitor carries. A program does not import them:
calling one of their functions is enough, and a `## Session` block calls `session.lg` for you.

Each section below builds a page on the pattern of the example project that uses the module, and
ends with the module's definitions — that table is generated from the module's own `## Note`s, so
it says what hover says.

## Forms — `forms.lg`

[`examples/signup_form`](../examples/signup_form) is the whole pattern. Every complaint about a
submission is a `FieldError` — the `field` it concerns and the `message` to show — and a
validation is a `Seq of FieldError` threaded through the checks, starting from `noErrors()`.

Three rules make the pattern safe to write:

- **The list is a sequence, not a map.** `errorFor(errors, "email")` answers `""` for a field with
  no complaint, so a template can write it under every input without asking first. A map would
  trap on the field that validated cleanly.
- **The first check to fail is the one shown.** `addError` leaves a field that already has a
  complaint alone, so order the checks from most fundamental to most particular: `checkRequired`
  before `checkMinLength`, or an empty box is told it is too short.
- **Validate on submit, not on every keystroke.** The complaints live in a plain `State:` field set
  by the submit action, so nothing scolds the visitor before they have tried once.

```logos-dom
## To validate (email: Text) and (password: Text) -> Seq of FieldError:
    Let mutable e be noErrors().
    Set e to checkRequired(e, "email", email).
    Set e to checkEmail(e, "email", email).
    Set e to checkRequired(e, "password", password).
    Set e to checkMinLength(e, "password", password, 8).
    Return e.

## Page /join

State:
    Let email be "".
    Let password be "".
    Let errors be a new Seq of FieldError.
    Let joined be false.

Actions:
    To submit:
        Set errors to validate(email, password).
        If isValid(errors):
            Set joined to true.

View:
    <main>
      <form id="join" on-submit.prevent={submit}>
        <input id="email" type="email" aria-label="Email" bind={email}>
        <p id="email-error">{errorFor(errors, "email")}</p>
        <input id="password" type="password" aria-label="Password" bind={password}>
        <p id="password-error">{errorFor(errors, "password")}</p>
        <button id="go" type="submit">Join</button>
      </form>
      {if joined:}<p id="welcome">Welcome.</p>{end}
    </main>

## Test: an empty form names each field's first problem

Submit "#join".
Expect that the text of "#email-error" is equal to "This is required.".
Expect that the text of "#password-error" is equal to "This is required.".
Expect that "#welcome" is hidden.

## Test: a field that passes says nothing

Type "ada@example.com" into "#email".
Type "short" into "#password".
Submit "#join".
Expect that the text of "#email-error" is equal to "".
Expect that the text of "#password-error" is equal to "Use at least 8 characters.".

## Test: a clean submission is accepted

Type "ada@example.com" into "#email".
Type "correct horse" into "#password".
Submit "#join".
Expect that the text of "#welcome" is equal to "Welcome.".
```

The same functions work anywhere, not only in a page. `errorSummary` joins every message, one per
line, for the summary a form puts above its fields:

```logos
## Main
Let mutable e be noErrors().
Set e to checkRequired(e, "name", "   ").
Set e to checkRange(e, "age", 130, 0, 120).
Set e to checkSame(e, "confirm", "tulip", "tulips", "The two passwords differ.").
Set e to checkRequired(e, "name", "").
Show errorSummary(e).
Show length of e.
Show errorFor(e, "age").
Show length of errorFor(e, "email").
Show isValid(noErrors()).
```

```output
This is required.
Choose a number from 0 to 120.
The two passwords differ.
3
Choose a number from 0 to 120.
0
true
```

`checkEmail` checks the shape of an address — one `@`, something before it, a dotted something
after it — and leaves an empty value to `checkRequired`, so an optional email field may stay blank.
Only a message arriving proves an address works.

<!-- generated from forms.lg by lgx_stdlib_guide_lock: ./scripts/release/lgx/gen-reference.sh -->

| Definition | What it is |
|---|---|
| `A FieldError has` | One complaint about a submission: the `field` it concerns and the `message` to show beside it. |
| `noErrors() -> Seq of FieldError` | An empty complaint list — the start of every validation. |
| `isValid(errors: Seq of FieldError) -> Bool` | Whether `errors` records no complaint at all. |
| `hasError(errors: Seq of FieldError, field: Text) -> Bool` | Whether `field` has a complaint recorded against it. |
| `errorFor(errors: Seq of FieldError, field: Text) -> Text` | The complaint against `field`, or `""` when there is none. |
| `addError(errors: Seq of FieldError, field: Text, message: Text) -> Seq of FieldError` | `errors` with `message` recorded against `field`. |
| `isBlank(t: Text) -> Bool` | Whether `t` is empty or nothing but whitespace. |
| `checkRequired(errors: Seq of FieldError, field: Text, value: Text) -> Seq of FieldError` | Complain when `value` is empty or all whitespace. |
| `checkMinLength(errors: Seq of FieldError, field: Text, value: Text, n: Int) -> Seq of FieldError` | Complain when `value` is shorter than `n` characters, counting after trimming. |
| `checkMaxLength(errors: Seq of FieldError, field: Text, value: Text, n: Int) -> Seq of FieldError` | Complain when `value` is longer than `n` characters, counting after trimming. |
| `checkRange(errors: Seq of FieldError, field: Text, value: Int, low: Int, high: Int) -> Seq of FieldError` | Complain when `value` is outside `low` to `high`, both included. |
| `checkSame(errors: Seq of FieldError, field: Text, a: Text, b: Text, message: Text) -> Seq of FieldError` | Complain when `a` and `b` differ — the password-confirmation check. |
| `looksLikeEmail(value: Text) -> Bool` | Whether `value`, trimmed, has the shape of an email address: one `@`, something before it, and a dot with text on both sides after it. |
| `checkEmail(errors: Seq of FieldError, field: Text, value: Text) -> Seq of FieldError` | Complain when `value` does not look like an email address. An empty value is left to `checkRequired`, so an optional field may be blank without complaint. |
| `errorSummary(errors: Seq of FieldError) -> Text` | Every complaint, one per line — the summary a form puts above its fields. |

<!-- end generated -->

## UI — `ui.lg`

[`examples/data_table`](../examples/data_table) pages a filtered, sorted table with this module.
Pagination is where the off-by-one errors live, and every function here answers in the language's
own terms: positions are 1-based, a page count is never 0, and the last page is short.

- `clampPage(page, total, perPage)` brings a requested page inside the table, so Next on the last
  page stays there.
- `pageStart(page, perPage)` to `pageEnd(page, perPage, total)` is the run of positions a page
  shows, ready for `item i of xs`. For an empty list the run is empty: the end is 0 and the start
  is 1.
- `pageCount(total, perPage)` is at least 1: an empty list still has a page, the one that says
  nothing is there.

The text helpers are filters: `{n | plural "person" "people"}` is `plural(n, "person", "people")`.

```logos-dom
## To everyone () -> Seq of Text:
    Let mutable out be a new Seq of Text.
    Push "Ada Lovelace" to out.
    Push "Grace Hopper" to out.
    Push "Katherine Johnson" to out.
    Push "Margaret Hamilton" to out.
    Push "Radia Perlman" to out.
    Return out.

## To pageOf (names: Seq of Text) and (page: Int) and (perPage: Int) -> Seq of Text:
    Let total be length of names.
    Let mutable out be a new Seq of Text.
    Let mutable i be pageStart(page, perPage).
    While i is at most pageEnd(page, perPage, total):
        Push item i of names to out.
        Set i to i + 1.
    Return out.

## Page /people

State:
    Let names be a new Seq of Text.
    Let shown be a new Seq of Text.
    Let page be 1.

Actions:
    When the page loads:
        Set names to everyone().
        Set shown to pageOf(names, page, 2).

    To next:
        Set page to clampPage(page + 1, length of names, 2).
        Set shown to pageOf(names, page, 2).

    To previous:
        Set page to clampPage(page - 1, length of names, 2).
        Set shown to pageOf(names, page, 2).

View:
    <main>
      <p id="count">{length of names | plural "person" "people"}</p>
      <ul>
        {for each n in shown, at index i:}
          <li id="row-{i}" class="person"><span class="initials">{n | initials}</span> <span class="name">{n | truncate 8}</span></li>
        {end}
      </ul>
      <p id="where">Page {page} of {pageCount(length of names, 2)}</p>
      <button id="previous" on-click={previous}>Previous</button>
      <button id="next" on-click={next}>Next</button>
    </main>

## Test: the first page shows the first two people

Expect that the text of "#count" is equal to "5 people".
Expect that the count of ".person" is equal to 2.
Expect that the text of "#row-1 .initials" is equal to "AL".
Expect that the text of "#row-2 .name" is equal to "Grace H…".
Expect that the text of "#where" is equal to "Page 1 of 3".

## Test: the last page is short and Next stays on it

Click "#next".
Click "#next".
Click "#next".
Expect that the text of "#where" is equal to "Page 3 of 3".
Expect that the count of ".person" is equal to 1.
Expect that the text of "#row-1 .initials" is equal to "RP".

## Test: Previous on the first page stays on it

Click "#previous".
Expect that the text of "#where" is equal to "Page 1 of 3".
Expect that the count of ".person" is equal to 2.
```

`truncate` marks a cut with `…` and counts the mark inside its limit, so the answer is never longer
than asked; `initials` treats any run of spaces as one separator.

<!-- generated from ui.lg by lgx_stdlib_guide_lock: ./scripts/release/lgx/gen-reference.sh -->

| Definition | What it is |
|---|---|
| `pageCount(total: Int, perPage: Int) -> Int` | How many pages `total` items make at `perPage` each. |
| `clampPage(page: Int, total: Int, perPage: Int) -> Int` | `page` brought inside `1` to `pageCount(total, perPage)`. |
| `pageStart(page: Int, perPage: Int) -> Int` | The 1-based index of the first item on `page`. |
| `pageEnd(page: Int, perPage: Int, total: Int) -> Int` | The 1-based index of the last item on `page`, never past `total`. |
| `truncate(t: Text, n: Int) -> Text` | `t` cut to at most `n` characters, marked with `…` when anything was removed. |
| `plural(n: Int, one: Text, many: Text) -> Text` | `n` with the word that agrees with it: `1 item`, `2 items`, `0 items`. |
| `initials(name: Text) -> Text` | The first letter of each word of `name`, run together: `Ada Lovelace` is `AL`. |

<!-- end generated -->

## Sessions — `session.lg`

[`examples/login`](../examples/login) is a signed-in site that never calls this module by name.
A `## Session` block declares what a session holds, and the compiler writes the rest: the
cookie is the session's fields, hex-encoded, followed by a signature (`signSession`); every
request's cookie is checked (`verifySession`) and a forged or edited one reads as a new, empty
session; `csrf()` is this session's token (`csrfToken`) for a form to carry; and a request that
changes state without the right token is refused with 403 before its route runs (`csrfValid`).

```logos
## Session
    a user, which is Text.

## Route POST /login (name: Text) -> Response:
    Set the session's user to name.
    Return redirectTo("/me").

## Route GET /me -> View:
    <main>
      {if the session's user is equal to "":}
        <p id="who">Nobody is signed in.</p>
      {else:}
        <p id="who">Signed in as {the session's user}.</p>
      {end}
    </main>

## Main

Serve on port 8080.

## Test: a visitor with no session is nobody

Expect that route("GET", "/me") contains "Nobody is signed in.".

## Test: a form post without its CSRF token is refused before the route runs

Expect that routeStatus("POST", "/login") is equal to 403.
```

The key the cookies are signed with is `sessionSecret()`: `LOGOS_SESSION_SECRET` when it is set,
otherwise a random secret made once per process — sessions then end when the server restarts, and
nobody can forge one.

The functions are ordinary LOGOS, and they are what the sentences above become. A signature is
checked against the value it signs, so a cookie carrying someone else's name with a real signature
is refused, and a CSRF token is bound to one session:

```logos
## Main
Let secret be "a secret for this example".
Let cookie be signSession(secret, "ada").
Inspect verifySession(secret, cookie):
    When Some (who):
        Show "signed in: " + who.
    When None:
        Show "refused".
Inspect verifySession(secret, "mallory|" + sessionMac(cookie)):
    When Some (who):
        Show "signed in: " + who.
    When None:
        Show "refused".
Show csrfValid(secret, cookie, csrfToken(secret, cookie)).
Show csrfValid(secret, cookie, csrfToken(secret, "another session")).
```

```output
signed in: ada
refused
true
false
```

<!-- generated from session.lg by lgx_stdlib_guide_lock: ./scripts/release/lgx/gen-reference.sh -->

| Definition | What it is |
|---|---|
| `textUpTo(t: Text, n: Int) -> Text` | `t` up to and including its `n`th character; `""` when `n` is not positive. |
| `textFrom(t: Text, n: Int) -> Text` | `t` after its `n`th character; the whole of `t` when `n` is 0. |
| `lastSeparator(t: Text) -> Int` | The 1-based position of the LAST `\|` in `t`, or 0 when there is none. |
| `macHex(secret: Text, msg: Text) -> Text` | `msg` authenticated under `secret`, as lowercase hex. |
| `sameTextInConstantTime(a: Text, b: Text) -> Bool` | Whether `a` and `b` are the same text, reading every byte of two texts of one length before it answers. |
| `signSession(secret: Text, value: Text) -> Text` | `value`, followed by its signature — the cookie a server hands out. |
| `sessionMac(cookie: Text) -> Text` | The signature half of a cookie, for a caller that wants to inspect it. |
| `verifySession(secret: Text, cookie: Text) -> Option of Text` | The value a cookie carries, or `none` when its signature does not check. |
| `csrfToken(secret: Text, session: Text) -> Text` | A CSRF token BOUND to one session. |
| `csrfValid(secret: Text, session: Text, token: Text) -> Bool` | Whether `token` is the CSRF token for this `session`. |
| `hexText(t: Text) -> Text` | The UTF-8 bytes of `t` as lowercase hexadecimal; `unhexText` is the way back. |
| `unhexText(h: Text) -> Text` | The text a `hexText` came from. Invalid hex answers `""` rather than trapping, because a cookie is attacker-controlled input. |
| `sessionField(payload: Text, i: Int) -> Text` | Field `i` of a session payload — counting from 1, as LOGOS indices do — or `""` when the payload does not carry one. |
| `sessionInt(payload: Text, i: Int) -> Int` | Field `i` of a session payload as a whole number, or `0` when it is absent or not one. |
| `sessionBool(payload: Text, i: Int) -> Bool` | Field `i` of a session payload as a truth value. Anything that is not `true` is false, which is the safe direction for a flag a visitor holds a cookie for. |
| `sessionSecret() -> Text` | The secret every session cookie is signed with. |
| `randomSecret() -> Text` | A fresh unguessable secret — 8 random 31-bit numbers in hex. |

<!-- end generated -->

## Where this is tested

Every page here is mounted and rendered on the tree-walker and the VM, compiled for the browser,
and has its tests run by `canonical_teaching_lock`; every program compiles, prints what its
`output` block shows and passes its tests under `doc_examples`. The definition tables are held
equal to the modules' own notes, and each section's example project is held to exist and to use
its module, by `lgx_stdlib_guide_lock`. The modules themselves are pinned by `forms_lock`,
`ui_lock`, `session_lock`, `session_block_lock` and `csrf_lock`.

---
[Docs index](README.md) · [Frontend](frontend.md) · [Web](web.md) · [LGX reference](lgx-reference.md)
