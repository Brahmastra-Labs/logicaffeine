# Thermal constants dossier — Track S

Every figure here is SOURCED (a citation to a standard reference), DERIVED (arithmetic shown
from sourced inputs), or MODELED (a design parameter of OUR stack, loudly labeled, awaiting
characterization). An invented physical constant is none of these and ships never.
`stack_target_lock` pins the values the ADL carries against this file.

## Bulk thermal conductivities (300 K)

| Material | k (W/(m·K)) | Status | Source |
|---|---|---|---|
| Silicon (bulk, undoped) | 148 | SOURCED | Incropera & DeWitt, *Fundamentals of Heat and Mass Transfer*, property tables (silicon at 300 K ≈ 148 W/(m·K)); CRC Handbook of Chemistry and Physics quotes 149–156 for pure Si — we carry the common textbook 148 |
| Copper (pure) | 401 | SOURCED | CRC Handbook of Chemistry and Physics, thermal conductivity of copper at 300 K = 401 W/(m·K); Incropera property tables agree (401) |
| Silicon dioxide (thermal oxide) | 1.4 | SOURCED | Incropera property tables, fused silica ≈ 1.38 W/(m·K) at 300 K, rounded to the 1.4 the microelectronics literature standardly uses for thermal SiO₂ |
| Organic interposer / underfill class | 0.5 | MODELED | design parameter for the V1 stack's bond-layer background conductance; filled underfills are quoted 0.3–3 W/(m·K) across vendor datasheets — no single citable constant, so this is OUR stack's declared class figure |

## Convection film coefficients (h, W/(m²·K))

| Cooling class | h range | Carried value | Status | Source |
|---|---|---|---|---|
| Natural convection (air) | 2–25 | 10 | SOURCED (range) / MODELED (point) | Incropera & DeWitt, Table 1.1: free convection of gases 2–25 W/(m²·K); the carried point value is OUR conservative middle |
| Forced air | 25–250 | 100 | SOURCED (range) / MODELED (point) | Incropera & DeWitt, Table 1.1: forced convection of gases 25–250 W/(m²·K) |
| Cold plate (forced liquid) | 100–20,000 | 2,000 | SOURCED (range) / MODELED (point) | Incropera & DeWitt, Table 1.1: forced convection of liquids 100–20,000 W/(m²·K) |

## Vertical interconnect classes (V1 stack — all MODELED design parameters)

No public, citable per-via figures exist for our exact geometry; these parameterize the
placement/thermal machinery until extraction/characterization replaces them. The DERIVED
row shows its arithmetic from the sourced k_Cu above.

| Class | pitch (µm) | R (Ω) | C (fF) | G_th per via (W/K) | Status |
|---|---|---|---|---|---|
| TSV (Cu, 10 µm ⌀ × 100 µm) | 40 | 0.05 | 40 | 3.15e-4 | G_th DERIVED: k_Cu·A/L = 401 × π·(5e-6)² / 100e-6 = 401 × 7.854e-11 / 1e-4 ≈ 3.15e-4 W/K; R/C/pitch MODELED |
| Microbump (solder) | 50 | 0.02 | 25 | 1.0e-4 | MODELED (class figures) |
| Hybrid bond (Cu–Cu) | 10 | 0.005 | 5 | 5.0e-5 | MODELED (class figures) |

## Digital tier power injection

`benchmarks/silicon/sky130_power.json`: soft_core_scalar, 0.0266 W total (OpenLane Sky130
signoff, report_power on the post-route RC-extracted netlist the signoff ledger ships, at its
15 ns every-corner signoff clock, typical corner, propagated default switching activity — a
signoff EDA number, not bench-measured; `scripts/record-silicon-power.py` writes it). The watts
and the `_provenance` string are carried VERBATIM into the stack descriptor, and
`stack_target_lock` fails the moment they differ from the ledger.

## Thermal-model knobs (all MODELED — fidelity statements of OUR model, never physics claims)

| Knob | Carried value | Note |
|---|---|---|
| Analog-tier coarsening grid | 4×4 | thermal nodes per analog tier; finer grids are a fidelity dial, not a correctness one (the exact-ℚ solve certifies whatever network is stated) |
| thermal cell edge | 500 µm | the coarse cell's lateral dimension; all model figures are stated on an exact 1e-12 grid (the snap is part of the model's statement) |
| Bond-layer thickness | 10 µm | gap dielectric/underfill span the background conductance crosses |
| Per-tile bias power | 1e-4 W | quiescent bias of an occupied analog tile — V1 class figure awaiting characterization |

## NOT FOUND (honest gaps)

- Measured thermal boundary resistance of a production hybrid-bond interface (vendor NDA
  territory) — the bond-layer background stays MODELED.
- A citable per-TSV electrical R/C for our unextracted geometry — MODELED until extraction.
