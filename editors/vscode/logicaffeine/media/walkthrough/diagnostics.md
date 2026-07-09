## Socratic errors + rustc's borrow checker

Errors explain instead of scold. Try using a value after giving it away:

```logos
## Main
Let x be 5.
Let y be 0.
Give x to y.
Show x.
```

> Cannot use 'x' after giving it away.
> You transferred ownership of 'x' with Give.
> Tip: Show 'x' to lend it without giving up ownership,
> or give 'a copy of x' to keep the original.

The diagnostic links to the exact `Give` that moved the value, and a quick
fix offers `a copy of x`.

**On save**, the extension additionally runs Rust's borrow checker over the
compiled form of your program (when cargo is installed) — findings arrive
translated into English, under the `logicaffeine (rustc)` source.
