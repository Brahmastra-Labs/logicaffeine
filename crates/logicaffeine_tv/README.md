# logicaffeine-tv

SMT translation validation for the LOGOS compiler.

Proves that the Rust emitted by the compiler is *observationally equivalent* to the
LOGOS source it was compiled from, per compile, by symbolically executing both into
the shared [`logicaffeine-verify`] semantic domain and discharging the equivalence
with Z3.

`check_encoder_sound` is the meta-soundness anchor: it cross-validates the LOGOS
encoder against the tree-walking interpreter (the de-facto semantics), so a buggy
encoder is caught rather than masked by a downstream equivalence that "proves" two
wrong things equal.

This is rung 3–4 (translation validation), not rung 5 (machine-checked proof): the
trust boundary is the encoders + Z3 + rustc, not a mechanized meta-theorem.

## License

Licensed under BUSL-1.1. See the workspace root for details.
