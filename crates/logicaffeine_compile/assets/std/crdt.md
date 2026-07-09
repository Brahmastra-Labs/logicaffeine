# CRDT Standard Library

Vocabulary for conflict-free replicated data — the convergent state programs
replicate. The merge semantics live in the runtime; this is the naming surface.

## Note
A counter delta from one replica — the unit CRDT merges exchange.

## Definition

A Delta has:
    a replica, which is Int.
    an amount, which is Int.
