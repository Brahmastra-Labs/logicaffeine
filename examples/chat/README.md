# Chat — a one-page chat room

A chat room needs two kinds of shared fact: messages, which every page should hear once, and a
presence count, which every page should agree on. This example uses one mechanism for each:
`Publish`/`When a message arrives` for the messages, and a `## Store … shared on` store for the
count.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**`Publish`/`When a message arrives`.** Submitting the composer publishes `username + ": " + draft`
on `"chat-room"`, and the page's own `When a message arrives on "chat-room" (m: Text):` hears it.
The sender's page is on the topic too, so it hears its own message, exactly as the ping example in
`docs/frontend.md`'s Triggers section documents.

**A count that adds up across pages.** `## Store Presence shared on "chat-presence"` keeps
`onlineCount` as a `Tally`, and every page calls `Presence's join` when it loads. A `Tally` is a
CRDT: every replica's increments and decrements add up, so two visitors arriving at the same moment
are both counted, where a plain field would keep whichever write it saw last. `PresenceCount` reads
the store from inside the page's own `View:`.

**A component tested on its own.** The first test `Mount`s `PresenceCount` alone, the way
`docs/frontend.md`'s "Testing a page" section tests a bare `## Component`. A `## Page` mounts itself
before any test statement runs, so the page's `When the page loads:` join has already happened: the
standalone component reads the store as that arrival left it, one visitor counted.
