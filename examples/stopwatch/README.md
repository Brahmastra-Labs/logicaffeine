# Stopwatch and pomodoro

Two timers, one pattern: a component-owned `Every 1 second as tick:` that
never itself stops, guarded by a `running` field. Pause and resume are that
field flipping, not a timer torn down and recreated.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**Pause without stopping the timer.** The tick keeps firing once a second;
`If running:` decides whether it counts. Pause and resume flip `running`, as
many times as you like, and the timer itself is never touched. The other
route is `Stop the timer tick.` to pause and `Start the timer tick.` to
resume, which arms the timer again from the moment of the resume, reading its
period afresh (`docs/frontend.md`'s Triggers section).

**`Advance time by N seconds.`** drives both timers deterministically, so
every test finishes instantly regardless of the durations involved. That is
also why the pomodoro here counts down from 5 seconds rather than 25
minutes: the mechanics under test (start, pause, reach zero, the `done`
flag) do not depend on the number.

**A shared formatter, not a copy per component.** `formatTime`/`pad2` are
plain `## To` functions at the top of the file, called from both
`Stopwatch` and `Pomodoro`.

## Limits

No known gap. `lowered_vm_tier_lock` and `browser_tier_lock`'s example-corpus
sweep run this page's scenarios on the tree-walker, the VM and a real Chrome,
and `crates/logicaffeine_tests/tests/lgx_known_divergences.txt` lists no
divergence for any of them.
