# Web — the server half

A LOGOS backend is a set of typed routes: one `## Route` per path, each declaring what it takes and
what it answers, and `Serve on port 8080.` to bind the port. The frontend is a set of pages that
compile to WebAssembly and call those routes — the [Frontend](frontend.md) guide covers it. One
project holds both, `largo serve` runs them together with hot reload, and `largo build --release`
packs pages, assets and backend into one native binary.

Nothing here is a framework binding. The HTTP helpers, the static-file server, the JSON reader, the
session cookies and the entire PostgreSQL wire protocol are written in LOGOS, in
[`assets/std/`](../crates/logicaffeine_compile/assets/std/), over a small set of socket primitives.
All of it can be read, and another protocol can be written the same way.

## The shape of a site

`largo new mysite --web` scaffolds three directories, by convention and with no extra manifest keys:

| Path | What it is |
|------|-----------|
| `src/main.lg` | The backend and its `Serve` statement |
| `pages/` | Pages, one WebAssembly bundle each; `app.lg` is served at `/` |
| `static/` | Files served verbatim at root-relative paths |

`largo new mysite --site` scaffolds `pages/` and `static/` alone: no backend, no `entry` in
`Largo.toml`, and a plain `largo build` exports it to `dist/` for a static host.

**One routing rule holds everywhere.** An exact match against a built page or a static asset wins;
anything else goes to the backend, whose route table matches it or answers `404`. The rule is the
same in development and inside the release binary.

```text
GET /style.css   ->  static/style.css             (asset, exact match)
GET /            ->  pages/app.lg                 (page, by convention)
GET /posts/7     ->  the route GET /posts/:id     (route, matched and bound)
GET /nope        ->  404                          (nothing matched)
```

## Routes

A site is a set of paths, so write one `## Route` per path. Each declares its method, its path, the
parameters it takes and the type it answers with; the router matches, binds and dispatches:

```logos
## Route GET / -> Text:
    Return "Hello from LOGOS".

## Route GET /posts/:id (id: Int) -> Text:
    Return "post {id}".

## Main
Serve on port 8080.
```

`Serve on port 8080.` binds the port and answers every request from the route table, once per
request and concurrently: a slow client cannot block another, and a failure in one request is a
`500` for that request rather than the end of the server. The port can be any expression, and
`LOGOS_PORT` or `PORT` in the environment overrides it, so the binary that serves 8080 on a laptop
takes the port a hosting platform assigns.

A route's parameters come from its own path. `:id` is a slot, `(id: Int)` names and types it, and a
segment that is not an `Int` is answered `400` rather than reaching the route. A parameter the path
has no slot for is bound from the query string, or from the body of a `POST` form. A route that
needs the whole request declares a `Request` parameter.

The answer type decides the content type, so a route says what it means rather than assembling a
header: `Text` is served as text, `View` is rendered as HTML, a record is encoded as JSON through
the codec derived from its declaration. A route that sets the status or a header answers
`Response` — which is also how a route sets a cookie, and so what makes [sessions](#sessions)
possible.

### Asking the route table

The same table answers questions without a socket, which is how a server is tested:

```logos
## Route GET /posts/:id (id: Int) -> Text:
    Return "post {id}".

## Route POST /echo (text: Text) -> Text:
    Return "you said {text}".

## Main
Show route("GET", "/posts/7").
Show routeStatus("GET", "/posts/seven").
Show routeWith("POST", "/echo", "text=hi").
Show routeContentType("GET", "/posts/7").
```

```output
post 7
400
you said hi
text/plain; charset=utf-8
```

`route(method, url)` answers the body a request would receive, `routeStatus` its status and
`routeContentType` its content type; `routeHeader(method, url, name)` reads one response header.
Each has a `…With(method, url, body)` form that sends a body. In a `## Test` they are ordinary
expressions:

```logos
## Route GET /health -> Text:
    Return "ok".

## Test: the health check answers

Expect that route("GET", "/health") is equal to "ok".
Expect that routeStatus("GET", "/missing") is equal to 404.
```

### Guards and redirects

`## Before each request (r: Request) -> Option of Response:` is a guard, and `## Redirect /old -> /new.`
moves a URL. Both belong to the route table:

```logos
## Before each request (r: Request) -> Option of Response:
    If r's path is equal to "/admin":
        Return some(unauthorized()).
    Return none.

## Redirect /old -> /new.

## Route GET /new -> Text:
    Return "here".

## Route GET /admin -> Text:
    Return "secret".

## Main
Show route("GET", "/admin").
Show routeStatus("GET", "/admin").
Show route("GET", "/old").
Show routeStatus("POST", "/old").
Show route("GET", "/new").
```

```output
Unauthorized
401
302 /new
302
here
```

A guard runs once per request, before the route table matches anything. `none` lets the request
through; `some(response)` is sent in the route's place — its own status, content type and headers —
and the route never runs. Guards run in the order they are declared, and the first to answer wins.
Because a guard runs before matching, it also answers a path no route declares and a path a redirect
moves, so an authorization check cannot be sidestepped by aiming at a URL that does not exist. The
guard's request carries the real headers and query; its `path` is the path without the query string,
so `/admin?x=1` is still `/admin` to it.

A redirect answers `302` with `Location` set, for every method, and a query string on the old URL
does not stop it matching. It never shadows a route: a path a `## Route` declares is served, not
redirected. Asked through the route table, a redirect's body is `302 /new`; the served response
carries the `Location` header.

### Taking the whole request by hand

`## Route` covers a site whose paths are known. A program that wants the dispatch itself — a proxy,
a catch-all, a path shape no pattern describes — writes one function from `Request` to `Response`
and serves with it:

```logos
## To handle (request: Request) -> Response:
    Let path be request's path.
    If path is equal to "/":
        Return html("<h1>Hello from LOGOS</h1>").
    If path is equal to "/api/health":
        Return json("{{\"ok\": true}}").
    Return notFound().

## Main
Serve on port 8080 with handle.
```

`Serve on port <expr> with <handler>` sends every request to that one function; the route table is
not consulted, no guard or redirect runs, and a `## Session` is not loaded or saved around it,
because all of them belong to the router's dispatch.

## The request

| Field | Meaning |
|-------|---------|
| `method` | `GET`, `POST`, and the rest, uppercase |
| `path` | The path, already percent-decoded, without the query string |
| `body` | The request body as text |
| `headers` | A map of header name to value |
| `query` | A map of query parameter to value |

Read headers with `headerOf`, which is case-insensitive because HTTP header names are, and query
parameters with `queryOf`, which is case-sensitive because query keys are. Both answer `""` for
something absent; `hasHeader` and `hasQuery` tell an absent key apart from an empty value.

`headerIn(headers, name)` is the same case-insensitive lookup against a header map rather than a
request. A request's headers and a response's headers are the same shape, so it reads a header off a
reply as well — `headerIn(reply's headers, "set-cookie")` — and `headerOf` is it applied to a
request.

A route that needs a query parameter declares it, and the router binds it; a route that needs the
headers or the raw body declares a `Request` parameter and reads them:

```logos
## Route GET /hello (name: Text) -> Text:
    Return "hello {name}".

## Route GET /agent (request: Request) -> Text:
    If hasHeader(request, "user-agent"):
        Return "agent: " + headerOf(request, "user-agent").
    Return "no agent".

## Main
Show route("GET", "/hello?name=ada").
```

```output
hello ada
```

## The response

A `Response` is a record of `status`, `contentType`, `body`, `bytes` and `headers`. `plain`, `html`
and `json` build a `200` with the matching content type. `notFound` builds a `404`.
`bytesResponse` carries raw bytes for binary content, which is how a compiled page or an image is
served.

Two refusals mean different things. `unauthorized` is a `401` — *I do not know who you are*, so
signing in will help. `forbidden` is a `403` — *I know who you are, and no*, so it will not. Sending
the wrong one leaves a signed-in visitor bouncing off a login page they are already past.
`redirectTo` sends a `302` with `Location` set; `permanentRedirectTo` sends a `301`, which asks every
cache on the way to remember the move — close to irreversible, so it is asked for by name.

Extra headers go on with `withHeader`, which returns a `Result` because four kinds of header are
refused rather than written: a name that is not a valid token, a value containing a control byte, a
hop-by-hop header the connection itself owns, and `content-type`, which belongs in the response's
own field. The refusal of control bytes is what stops response splitting at the source.

```logos
## Route GET /tagged -> Response:
    Inspect withHeader(plain("ok"), "x-app", "demo"):
        When Ok (r):
            Return r.
        When Err (message):
            Return plain(message).

## Main
Show routeHeader("GET", "/tagged", "x-app").
```

```output
demo
```

The rules it applies are callable on their own: `isHeaderName` for a whole name,
`isHeaderTokenByte` for one byte of one, `isHeaderValue` for a value with no control bytes, and
`isHopByHop` for the names the connection owns.

## Cookies

A cookie is how a session survives a request, which makes it the most security-sensitive header a
site emits. `cookie(name, value)` writes the safe attributes, and a program opts out rather than in:

```text
sid=abc123; Path=/; HttpOnly; SameSite=Lax; Secure
```

`HttpOnly` keeps a script from reading it, which is what makes an XSS a nuisance rather than an
account takeover. `SameSite=Lax` is what CSRF defence rests on: a cookie the browser will not attach
to a cross-site request cannot be replayed by another site's form. `Secure` keeps it off plaintext
HTTP.

A `Secure` cookie is never sent back over plain HTTP. `localhost` is exempt — browsers treat it as
trustworthy — but a LAN address is not, so a page opened on a phone at `http://192.168.1.20:8080`
signs in and then does not stay signed in. `cookieFor(name, value, secure)` says which it wants, and
`cookiesAreSecure()` is the answer `cookie` gives it: yes, unless `LOGOS_COOKIE_INSECURE` is set. The
default stays secure because the two mistakes differ in size: a missing `Secure` in production is a
session token in clear text on every request; an unwanted one in development is a sign-in that does
not stick. Dropping it drops only `Secure` — `HttpOnly` and `SameSite` defend against other attacks.

`withCookie(response, name, value)` sets one on a reply, `cookieOf(request, name)` reads one back
(the `Cookie` header is a list, so a page with several cookies works), and `expiredCookie(name)` is
how a sign-out deletes one: a zero lifetime is the only way to say *forget this*.

```logos
## Route POST /login -> Response:
    Return withCookie(plain("ok"), "sid", "ada").

## Route GET /me (request: Request) -> Text:
    Return cookieOf(request, "sid").

## Main
Show route("POST", "/login").
Show isCookieSafe("ada").
Show isCookieSafe("a;b").
```

```output
ok
true
false
```

A name or value containing `;`, a comma, a quote, a space or a control byte is **refused**: those
end the cookie, or the whole header, and everything after them becomes something nobody wrote —
response splitting, aimed at the session cookie. `isCookieSafe` is the check, callable first.
Refusing beats trimming: a silently truncated session id is a bug nobody can see.

## Sessions

A cookie is the mechanism; a session is what a program wants. Declare its fields and the router
does the rest:

```logos
## Session
    a user, which is Text.

## Route POST /login (name: Text) -> Response:
    Set the session's user to name.
    Return redirectTo("/").

## Route GET /me -> Text:
    Return the session's user.
```

`the session's user` is an ordinary possessive and reads and writes state that belongs to the
visitor and outlives the request. The router loads it from a signed cookie before anything
dispatches and saves it back **only when a request changed it**, so a read-only page sets no cookie
and a route that sets a cookie of its own is not overwritten.

Fields are `Text`, `Int` or `Bool`. A session travels in the cookie the browser sends with *every*
request, so it holds an id and the program looks the rest up from that.

A cookie is attacker-controlled input, so one whose signature does not check is not an error and
not a crash: it is a visitor with no session. `LOGOS_SESSION_SECRET` sets the signing key; with
nothing set the key is random per process, so sessions do not survive a restart, and a program
nobody configured still cannot have its sessions forged.

### CSRF

With a `## Session` declared, the router **checks the token itself** before a state-changing route
runs. A `POST`, `PUT`, `PATCH` or `DELETE` without a valid token is answered `403` and the route
never executes. A form carries the token in a `_csrf` field with `csrf()`:

```logos
## Session
    a user, which is Text.

## Route GET / -> View:
    <form method="post" action="/save">
      <input type="hidden" name="_csrf" value={csrf()}>
      <input name="title">
      <button>Save</button>
    </form>

## Route POST /save (title: Text) -> Text:
    Return "saved " + title.

## Main
Serve on port 8080.
```

The form is rendered by the server, the only place a per-visitor token can come from. A scripted
request sends the token as an `X-CSRF-Token` header instead of a form field.

The token is bound to a per-visitor nonce minted the first time `csrf()` is called on a visit — the
first moment that visitor needs a cookie, so one who never sees a form is never given one. A token
bound to the session's *contents* would hand every anonymous visitor the same token, and a token
every visitor shares is one an attacker reads off their own page.

Forgetting the field is a compile error: a `<form>` whose method changes state must carry a `_csrf`
field or the page does not build.

A page's own `POST`/`PUT`/`PATCH`/`DELETE` fetches to its own server carry the same token
automatically — no header to add by hand. The composing server writes the visitor's token into the
document as `<meta name="csrf-token">`, and every state-changing request the page's own code makes
to its own origin reads it from there and sends it as `x-csrf-token`; a request to another origin,
or a `GET`, is untouched. This is the wire the form field above rides for a plain `<form>` submit,
and the ONLY thing a scripted `fetch`-style request needs to add for the same protection is nothing.

### The primitives underneath

[`session.lg`](../crates/logicaffeine_compile/assets/std/session.lg) is what `## Session` is built
on, and a program that manages its own cookie calls it directly. `signSession(secret, value)` answers
the value followed by its HMAC-SHA-256 signature; `verifySession(secret, cookie)` answers
`some value` when the signature checks and `none` for anything else, never trapping on a malformed
cookie. `sessionMac(cookie)` is the signature half, for a program that inspects it. The signature is split at the *last* separator, so a value may itself contain `|` without
letting an attacker move the boundary. `csrfToken(secret, session)` and `csrfValid(secret, session,
token)` bind a CSRF token to one session under a key distinct from the cookie's. `sessionSecret()`
reads `LOGOS_SESSION_SECRET` and falls back to `randomSecret()`. A payload of several fields is
hex-encoded per field and joined with `.` — `hexText` and `unhexText` convert, and
`sessionField(payload, i)`, `sessionInt` and `sessionBool` read field `i`, counting from 1, answering
the empty value for a field an older cookie does not carry.

## Static files

`largo serve` and the release binary serve `static/` without a line of code. A program that serves
files itself — downloads under their own prefix, say — uses `staticUnder`, which refuses any path
containing a `..` segment before a byte reaches the disk and answers `404` for a file that is not
there. `fileResponse` serves one known path.

```logos
## Route GET /downloads/:name (name: Text) -> Response:
    Return staticUnder("downloads", name).

## Main
Show routeStatus("GET", "/downloads/..%2F..%2Fetc%2Fpasswd").
Show hasTraversal("../../etc/passwd").
```

```output
404
true
```

A refused traversal and an absent file give the same `404`, so a prober learns nothing from the
difference. `hasTraversal` is the guard itself, for a path built from parts.

`contentTypeFor` maps an extension to a MIME type, and it is **the** MIME table: `largo serve` and
`largo build` read this function rather than keeping their own, so a file is typed identically in
development and production. Text types name their charset (`text/css; charset=utf-8`), because a
browser guessing an encoding renders a visitor's name as mojibake; `text/javascript` is the exception
its own specification asks for. An unknown extension is `application/octet-stream` — a download,
never a guess, since guessing `text/html` would let an uploaded file be served as a page.

## Reading JSON

A route that answers a record is encoded for you, and a route parameter of a record type is decoded
for you. For JSON read by hand, `jsonTryParse` is lenient the way a browser is — a duplicate object
key keeps the last value — and `jsonParseStrict` rejects duplicate keys with a positioned message,
which is what anything deciding money or access wants. Both return a `JsonResult`.

A brace in a text literal opens an interpolation hole, so a hand-written JSON object doubles its
braces and escapes its quotes: `"{{\"ok\": true}}"` is the text `{"ok": true}`. Text that arrives in a
request body needs no escaping.

```logos
## Main
Let parsed be jsonParseStrict("{{\"ok\": true}}").
Inspect parsed:
    When JOk (value):
        Show "ok".
    When JErr (message):
        Show message.
```

```output
ok
```

Pull typed values out with `jsonField`, `jsonString`, `jsonInt`, `jsonBool` and `jsonArray`. Each
returns a `Result`, so an authorization decision is never made on a field that was the wrong type or
missing. `jsonField` distinguishes an absent key from one stored as null. `jsonRequireObject` insists
the value is an object, and `jsonNoExtraKeys` closes the shape so an extra field cannot be smuggled
past the decoder.

## Rendering HTML on the server

A route answering `View` is a template, rendered on the server with every hole escaped, and a
component renders to text with `html of`:

```logos
## Component Greeting
State:
    Let name be "ada".
View:
    <p>Hi {name}</p>

## Main
Show html of Greeting.
```

```output
<p>Hi ada</p>
```

A server-rendered page is the page a browser would paint, serialized: every hole in an attribute
is escaped as `htmlEscape` escapes it and every hole in text as `htmlEscapeText` does, and a URL
attribute a browser follows goes through `safeUrl`. The two escapers are needed by hand only when
building markup as text.

`htmlEscape` replaces the four characters that mean something to an HTML parser — `&` first, or the
entities it adds would be re-escaped, then `<`, `>` and `"`. The quote is there because the same
function guards attribute values, where a bare quote ends the attribute. `htmlEscapeText` replaces
three: between tags a `"` ends nothing, and escaping it would make the bytes a server sends differ
from the bytes a browser paints, which every cross-tier comparison in this codebase reads.

```logos
## Main
Show htmlEscape("<b>hi</b>").
Show htmlEscape("a\"b").
Show htmlEscapeText("a\"b").
```

```output
&lt;b&gt;hi&lt;/b&gt;
a&quot;b
a"b
```

`safeUrl` guards a URL that becomes an `href`, a `src`, an `action` or any attribute a browser
follows. Escaping does nothing to `javascript:alert(1)` — it holds none of the characters
`htmlEscape` replaces — so `safeUrl` answers `"about:blank#blocked"` for a URL whose scheme executes,
and the URL itself otherwise, after normalising the case, whitespace and control characters a
browser ignores when it reads a scheme.

```logos
## Main
Show safeUrl("/todos/1").
Show safeUrl("java\tscript:alert(1)").
```

```output
/todos/1
about:blank#blocked
```

## URLs, query strings and form bodies

A route binds its parameters from the path, the query and the body. These are the functions it
uses, for the times a URL is read by hand.

`isIntText` asks whether text reads as a whole number. `parseInt` traps on text that is not one,
which is right for a program reading its own data and wrong for a server reading a stranger's URL;
the router asks first, which is why `/posts/abc` is a `400`. `""` and a lone `"-"` are rejected as
firmly as `"abc"`.

`pathOf` is everything before the first `?` — what a route matches on — and `queryString` everything
after it. `queryParam` reads one named parameter out of a query, and `urlDecode` undoes the encoding
a browser applies (`+` is a space, `%XX` is a byte; `hexDigitValue` is the digit reader underneath).
Decoding works over bytes, because percent-encoding encodes bytes: a check mark arrives as
`%E2%9C%93`. A `%` not followed by two hex digits is left alone — `50%` is text somebody typed.

A `POST` form puts its parameters in the body, and a scripted post puts JSON there. `bodyParam`
reads a named value out of either, deciding by the first character. Going the other way, `urlEncode`
escapes a value for a query string or a form body: it leaves the unreserved characters alone
(`isUnreservedByte` is its test, per RFC 3986) and percent-escapes every other byte, with
`hexByteText` writing one as `%XX`. `charAtText(s, i)` reads the character at a **1-based** byte
offset — it is the digit-table lookup under `hexByteText`, where the offset is a digit's value plus
one — and answers `""` past the end.

```logos
## Main
Show isIntText("42").
Show isIntText("4a").
Show pathOf("/search?q=hi").
Show queryString("/search?q=hi").
Show queryParam("q=hello+world&page=3", "q").
Show urlDecode("a%2Fb").
Show bodyParam("text=buy%20milk", "text").
Show urlEncode("buy milk").
Show charAtText("abc", 3).
```

```output
true
false
/search
q=hi
hello world
a/b
buy milk
buy%20milk
c
```

An upload is not a plain `POST`: it is `multipart/form-data`, and three functions build it.
`multipartBoundary` derives a delimiter that cannot occur inside the content it delimits — a random
one would make every recorded request unreproducible, and a colliding one would truncate the upload
at that byte. `multipartBody` frames the part with CRLF line endings, and `multipartContentType`
names the boundary in the header; they are produced in different places, so a mismatch fails every
upload on a real server while passing a test that reads only one of them.

## Storing data

[`postgres.lg`](../crates/logicaffeine_compile/assets/std/postgres.lg) speaks the PostgreSQL version 3
wire protocol in LOGOS over the socket primitives: cleartext, MD5 and SCRAM-SHA-256 authentication,
the simple and extended query protocols, transactions, statement timeouts and query cancellation.

`pgConnect` opens a connection from a URL and `pgClose` ends it. `pgConnectSecure` takes the same URL
and negotiates TLS first — a server that declines is refused rather than spoken to in the clear.
Hosted databases (RDS, Neon, Supabase, Cloud SQL) reject an unencrypted connection, so `pgConnect`
reaches a database on your own machine and `pgConnectSecure` reaches a managed one; everything after
the connection is identical. `pgQuery` runs a statement and returns rows; `pgExecute` runs one that
returns none and answers the number of rows affected. For a statement run many times, `pgPrepare`
parses it on the server and `pgExecPrepared` runs it with fresh arguments.

**Put user data in parameters, never in SQL text.** `pgQueryParams` sends the statement and its
arguments as separate wire fields, so nothing a user types is read as SQL. The only SQL the module
composes itself comes from a closed, validated set.

```logos
## To insertNote (pgc: PgConn) and (body: Text) -> Bool:
    Let params be a new Seq of Text.
    Push body to params.
    Let done be pgQueryParams(pgc, "INSERT INTO notes (body) VALUES ($1)", params).
    Inspect done:
        When Ok (result):
            Return true.
        When Err (message):
            Return false.
```

Read cells with `pgTextAt`, `pgIntAt`, `pgBoolAt`, `pgRealAt`, and the typed `pgDecimalAt`,
`pgUuidAt` and `pgEnumAt`. NULL is honest: the option-returning variants (`pgTextAtOpt` and its
siblings) answer `none`, and `pgIsNullAt` asks directly, while the plain getters surface NULL as an
empty string. Use the option getters wherever NULL is meaningful.

A connection is one socket holding one strictly ordered conversation. Run one statement at a time on
it and open more connections for parallelism; there is no pool in this version.

## Raw sockets

Everything above stands on [`tcp.lg`](../crates/logicaffeine_compile/assets/std/tcp.lg):
`tcpConnect`, `tcpSend`, `tcpRecv`, `tcpRecvExact` and `tcpClose` on the client side, `tcpListen`,
`tcpAccept` and `tcpListenerPort` on the server side, plus `tcpShutdownWrite` and `tcpSetTimeout`. A
connection is an opaque integer handle, bytes travel as a sequence of integers, and every operation
returns a `Result`, so a network failure is a value rather than a trap. A clean close reads as an
empty result from `tcpRecv`, while `tcpRecvExact` treats an early close as an error, which is what a
framed protocol needs.

`tcpConnectTls` dials a host and completes a TLS handshake before returning, and `tcpUpgradeTls`
turns an established plain connection into an encrypted one in place — what a protocol that
negotiates first requires: PostgreSQL sends an eight-byte request, reads a one-byte answer, and only
then hands the socket to TLS. Both return the same handle, so `tcpSend`, `tcpRecv` and `tcpClose`
work unchanged. Certificates are checked against the Mozilla root set bundled into the binary, so a
build behaves the same wherever it runs; `LOGOS_TLS_EXTRA_ROOT` names a PEM file to trust in
addition, for a private certificate authority or a test server.

## Hashing and passwords

[`crypto.lg`](../crates/logicaffeine_compile/assets/std/crypto.lg) carries `sha256`, `hmacSha256` and
`pbkdf2Sha256` as LOGOS source, pinned to the FIPS 180-4, RFC 4231 and RFC 7677 vectors. They take and
return sequences of bytes. The interpreted tiers accelerate the key derivation natively because it is
deliberately expensive; the compiled tiers build the LOGOS source; both are locked to the same
vectors.

## Running and deploying

```bash
largo new mysite --web
cd mysite
largo serve
```

`largo serve` owns the public port, serves the pages and assets from memory, and proxies the rest to
the backend running as a child process. It watches all three directories: editing a page or an asset
rebuilds it and reloads the browser, and editing the backend restarts the child. A site with no
backend is published with `largo build --static`, covered in the [Frontend](frontend.md#building-and-deploying)
guide, as is `## Error page`.

`largo build --release` produces one native binary with the pages, the assets and the backend
embedded:

```bash
largo build --release
PORT=8080 ./target/release/mysite
```

That binds `127.0.0.1`, so running a program does not publish it to the network. Inside a container,
behind a load balancer, or anywhere traffic arrives from another host, name the interface:

```bash
LOGOS_HOST=0.0.0.0 PORT=8080 ./mysite
```

`LOGOS_HOST` takes an IPv4 address, an IPv6 address (bracketed for you) or a hostname. The server
speaks HTTP; terminate TLS in front of it — nginx, Caddy, or whatever the platform puts in front of a
container.

### In production

A server built this way refuses a request rather than queueing it when it is already serving as many
as it will, bounds how long a handler may run and how long a client may take to send, and on a stop
signal stops accepting, finishes the work it has, and releases the port. Every bound is read from the
environment in one place, so the same names work on every tier:

| Variable | What it bounds |
|----------|----------------|
| `LOGOS_HOST` | The interface to bind. Defaults to `127.0.0.1` |
| `LOGOS_MAX_INFLIGHT` | How many requests may be served at once before the rest are refused |
| `LOGOS_REQUEST_TIMEOUT_MS` | How long one handler may run |
| `LOGOS_READ_DEADLINE_MS` | How long a client may take to finish sending |
| `LOGOS_DRAIN_MS` | How long a graceful stop waits for in-flight work |
| `LOGOS_MAX_BODY` | The largest request body accepted; a larger one is answered `413` |
| `LOGOS_READY_PATH` | A path answering a readiness probe. Unset by default |
| `LOGOS_HEALTH_PATH` | A path answering a liveness probe. Unset by default |
| `LOGOS_CORRELATION_ID` | `1`, `true`, `yes` or `on`: every response carries an `X-Correlation-Id` — the client's own, else one the server assigns |

Readiness reports whether the server wants traffic and turns `503` the moment it starts draining.
Liveness reports only that the process is answering and stays `200` through a drain, so an
orchestrator stops sending work without killing a server that is still finishing some. With neither
named no path is reserved, so the route table keeps every path it declares.

On the interpreted tier the request budget is checked between statements, so it bounds a runaway loop
rather than a single long-running builtin. The compiled tier races a real timer.

A request body is framed by `Content-Length` alone: a request sent with `Transfer-Encoding: chunked`
is answered `400`, as is any request whose bytes do not parse.

A compiled binary serves on one of two engines, and says which on stderr when it starts. On Linux
it defaults to a thread-per-core io_uring engine wherever the kernel can set up a ring, and to tokio
everywhere else; the interpreted tier always serves on tokio. The io_uring engine runs a handler
inline on its shard's thread, so it cannot bound a handler's time, shed by admission or answer a
probe path: setting `LOGOS_REQUEST_TIMEOUT_MS`, `LOGOS_MAX_INFLIGHT`, `LOGOS_READY_PATH`,
`LOGOS_HEALTH_PATH` or `LOGOS_CORRELATION_ID` moves the binary onto tokio, naming the variable that
did it, so the guarantee asked for holds. `LOGOS_MAX_BODY` and `LOGOS_READ_DEADLINE_MS` are enforced
by both engines.

| Variable | What it selects |
|----------|-----------------|
| `LOGOS_HTTP_ENGINE` | `uring` or `tokio`, overriding the default; `uring` where no ring can be set up falls back to tokio and says why |
| `LOGOS_HTTP_SHARDS` | The io_uring engine's shard count. Defaults to one per core |
| `LOGOS_HTTP_ACCESS_LOG` | Any value: the tokio engine prints one `logos-http method=… path=… status=… correlation_id=… dur_ms=…` line per request to stderr, never a header value |

## Where this is tested

The examples above run as part of the documentation suite, and each output shown is checked against
the program's real output. The behaviour they describe is pinned by `route_request_response_lock`,
`server_surface_lock` for guards, redirects and error pages, `session_lock`, `csrf_automatic_lock`
for the router's token check, `http_static_lock`, `json_conformance_lock` and `json_typed_decode_lock`,
`http_lifecycle_lock` and `http_lifecycle_bounds_lock` for the production bounds, `http_engine_fallback`
for the engine choice,
`postgres_wire` and `postgres_protocol` against recorded wire transcripts, `postgres_live` against a
real PostgreSQL server, `tcp_natives` and `tcp_cross_tier`, `http_workers` for concurrent serving,
`crypto_sha2_lock` for the digest vectors, and `http_differential`, which asserts the interpreted and
compiled servers answer the same request with byte-identical bytes.

---
[Docs index](README.md) · [Frontend](frontend.md) · [Imperative mode](imperative-mode.md) · [The largo CLI](cli.md)
