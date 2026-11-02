# Task board — components that wrap other components

The smallest complete demonstration of the component half of LGX, the LOGOS web framework.
The other frontend examples are each one `## Page`; this one is a page built out of parts,
which is how a real interface is assembled.

```bash
largo test           # completes and resets tasks, and checks what the board shows
largo serve          # http://localhost:8080
largo build --static # dist/ — upload it anywhere
```

## What it shows

**A wrapper: a component given children.** `<Card title="This week">…</Card>` draws a
frame around markup the *caller* wrote. The card has a prop of its own, `title`, and the
children still read the names they were written against: `{todo}` inside the card means
the loop variable in the page, not anything of the card's. A card, a panel, a modal and a
layout are all this shape, which makes it the shape most pages are built from.

**Scoped CSS, per component.** Each `Style:` block is rewritten so its selectors match
only that component's own elements. Two components can both style `.row` without
colliding, and neither leaks into the page around them.

**A conditional class.** `class:done={done}` adds one word to the row's class list while
the condition holds and leaves it out otherwise. The server walk composes that list exactly
as the browser does, so the markup the server sends is byte-for-byte what the browser would
have painted.

**A derived value.** `Let remaining always be (length of todos) - doneCount.` recomputes
whenever either part moves. Nothing subscribes by hand.

**A loop that knows where it is.** `{for each todo in todos, at index i:}` numbers the rows
from 1, like every position in LOGOS, so `done={i is at most doneCount}` strikes exactly
the first `doneCount` rows — the same count the `left` line subtracts.

## The part worth looking at

Run `largo build --static` and read `dist/index.html`. The whole page is in there (the
card's frame, its scoped `<style>`, every row, the count) before a single byte of
WebAssembly has loaded. That is what a reader with JavaScript disabled sees, and what a
crawler indexes.
