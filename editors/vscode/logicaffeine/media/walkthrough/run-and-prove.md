## Run code, prove theorems

Open `src/main.lg` and click the **Run** lens above `## Main` (or press
`Ctrl+Alt+R`). The default mode is the interpreter — sub-second feedback;
switch to compiled runs with the `logicaffeine.run.mode` setting.

Theorems prove from the same file:

```logos
## Theorem: Socrates

Given: All men are mortal. Socrates is a man.
Prove: Socrates is mortal.
Proof: Auto.
```

Click **Prove** above the theorem — the derivation is kernel-certified, and
`--trace` (the `logicaffeine.prove.trace` setting) renders the proof tree.
