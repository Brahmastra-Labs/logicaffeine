# Sourced physical constants for the SKY130 / FPAA analog non-ideality model

Deep-research pass, 2026-08-30. Every figure carries a primary-source citation and an exact
quote; DERIVED figures show their arithmetic; NOT-FOUND items are declared rather than invented.
This file is the provenance source of truth for `codegen_ana::nonideal` (the Phase-6
`NonIdealityModel` constants).

## 1. SKY130 MOSFET mismatch (Pelgrom) coefficients

No published SILICON-MEASURED Pelgrom characterization of sky130 exists (searched IEEE/arXiv/
Open-MPW; the community Slack confirms coefficients are unpublished outside the model files).
What is authoritative is the **mismatch model shipped in the PDK itself** (Apache-2.0):

| Device | Parameter | Value | Units | Source |
|---|---|---|---|---|
| nfet_01v8 | `vth0_slope` (60/63 W/L bins) | 3.356e-03 | V·µm (per-device σ = slope/√(W·L·mult), W,L in µm) | sky130_fd_pr nfet_01v8 mismatch corner file (github.com/google/skywater-pdk-libs-sky130_fd_pr, `cells/nfet_01v8/sky130_fd_pr__nfet_01v8__mismatch.corner.spice`): `.param sky130_fd_pr__nfet_01v8__vth0_slope=3.356e-03` |
| nfet_01v8 | `vth0_slope1` (3 bins) | 7.356e-03 | V·µm | same file |
| pfet_01v8 | `vth0_slope` / `slope1` / `slope2` | 5.856e-03 / 7.356e-03 / 4.356e-03 | V·µm | `cells/pfet_01v8/sky130_fd_pr__pfet_01v8__mismatch.corner.spice` |

Model application (quoted from the open_pdks-compiled ngspice model, sky130A):
`vth0 = {0.49439+...+MC_MM_SWITCH*AGAUSS(0,1.0,1)*(sky130_fd_pr__nfet_01v8__vth0_slope/sqrt(l*w*mult))}`

**DERIVED — Pelgrom-convention A_VT (pair difference = √2 × per-device σ):**
nfet_01v8 A_VT ≈ √2 × 3.356 = **4.75 mV·µm** (slope1 bins: 10.4); pfet_01v8 ≈ **8.28 mV·µm**
(slope1: 10.40). These are MODEL parameters, not measured silicon — flag accordingly.

## 2. Input-referred offset sigma, simple OTA/5T in SKY130

**NOT FOUND** as a published figure (the one sky130 OTA paper, SBMicro 2023, defers mismatch
"for future works"; the 3.41 mV 5T-OTA figure at arXiv:2407.00817 is TSMC 40nm — excluded).
**DERIVED substitute** from the cited PDK model, assumptions explicit (5T-OTA, nfet pair
10µ/0.5µ, pfet load 10µ/0.5µ, gm_p/gm_n = 0.5): σ(V_os) ≈ **2.8 mV (1σ)** — model-derived
example: input-pair ΔV_T σ = √2·(3.356/√5) = 2.12 mV; load 3.70 mV × 0.5 = 1.85 mV; RSS 2.8 mV.

## 3. FPAA floating-gate programmed-parameter accuracy

- **< 1% over 6 decades** (30 pA–30 µA): Hasler, "Large-Scale Field-Programmable Analog
  Arrays," Proc. IEEE 108(8) 2020 (hasler.ece.gatech.edu/FPAA_IEEEXPlore_2020.pdf):
  "Programmable subthreshold and above-threshold current sources are routinely programmed over
  six orders of magnitude (e.g., 30 pA–30 µA) with better than 1% accuracy at all values".
- "tighter than 1%" subthreshold; routing-fabric FG switches 0.2%–0.76% uncorrected: George
  et al., SoC FPAA, IEEE TVLSI 2016 (hasler.ece.gatech.edu/SoCFPAA/RASP30_SoCFPAA.pdf).
- 9.5-bit thermal-noise-limited programming: Basu et al., RASP 2.8, IEEE JSSC 2010.

## 4. Analog MAC / analog-compute energy per operation

- **68 fJ/MAC** (68 nW @ 1 MMAC/s), 350 nm FPAA source-coupled VMM: Schlottmann & Hasler,
  IEEE JETCAS 1(3) 2011.
- **≈1.15 fJ per 1-b op** (derived 1/866e12): Valavi et al. charge-domain in-memory CNN, 65 nm,
  "866 1-b TOP/s/W" — Verma et al., IEEE SSCS Magazine 2019.
- **≈0.32 pJ/op** (derived 1/3.12e12): Gonugondla et al., 65 nm SRAM in-memory classifier,
  "3.12TOPS/W" (ISSCC 2018, via SSCS Magazine 2019).
- System-level: < 1 µJ/inference @ < 23 µW command-word classifier (Hasler Proc. IEEE 2020).

## 5. Thermal noise floor (kT/C)

Formula V_rms = √(k_B·T/C) (Johnson–Nyquist noise, kTC section; en.wikipedia.org/wiki/
Johnson–Nyquist_noise, table: 1 pF → 64 µV, 10 pF → 20 µV).
**DERIVED** at T = 300 K (kT = 4.142e-21 J): 1 pF → **64.4 µV rms**; 10 pF → **20.4 µV rms**.

## 6. SKY130 nominal supply voltages

1.8 V internal/core + 5.0 V I/O: SkyWater PDK README (github.com/google/skywater-pdk):
"Support for internal 1.8V with 5.0V I/Os (operable at 2.5V)". 1.8 V device family `*_01v8`
(V_DS/V_GS 0–1.95 V) + 5.0 V and 3.3 V-native families: skywater-pdk.readthedocs.io
Device Details.

## 7. SKY130 wire parasitics (met2 / met3)

From skywater-pdk.readthedocs.io rcx.html (Tables 91/93/94/95) + periphery rules:
- met2 sheet R **125 mΩ/sq**, min width 0.140 µm → **≈0.89 Ω/µm** at min width.
- met3 sheet R **47 mΩ/sq**, min width 0.300 µm → **≈0.157 Ω/µm**.
- Plate met1↔met2 133.861 aF/µm²; met2↔met3 86.186 aF/µm²; fringe met2 67.045(down)/44.432(up)
  aF/µm; met3 69.846/42.643 aF/µm.
- DERIVED min-width wire-over-plane: met2 **≈0.15 fF/µm**; met3 **≈0.17 fF/µm**
  (configuration-dependent; tables give the primitives).

## Status ledger

SOLID: PDK mismatch model coefficients; FG accuracy; energy datapoints; kT/C; supplies;
parasitic primitives. DERIVED (arithmetic shown): A_VT convention conversion; 5T-OTA offset
example; fJ/op conversions; kT/C values; per-µm parasitics. NOT FOUND (never fabricate):
silicon-measured sky130 A_VT; published sky130 OTA offset sigma.

## Translinear anchors (O6)

- **Thermal voltage U_T = kT/q at 300 K = 25.85 mV — DERIVED from CODATA exact constants**:
  k = 1.380649e-23 J/K (exact, 2019 SI), q = 1.602176634e-19 C (exact, 2019 SI):
  U_T = 1.380649e-23 × 300 / 1.602176634e-19 = 2.585e-2 V. Arithmetic shown; nothing measured.
- **Subthreshold exponential law** — SOURCED: Mead, *Analog VLSI and Neural Systems* (1989);
  Vittoz & Fellrath, "CMOS analog integrated circuits based on weak inversion operation",
  IEEE JSSC 12(3), 1977. The EKV model is the modern statement of the same region.
- **Translinear principle (products/ratios/roots from junction loops)** — SOURCED: Gilbert,
  "Translinear circuits: a proposed classification", Electronics Letters 11(1), 1975.
- **Subthreshold slope factor n** — MODELED: process-dependent (typically quoted 1.2–1.6 for
  bulk CMOS); OUR fabric carries it as a design parameter until characterization. The tiles'
  ideal math semantics stand on the NORMALIZED law (the bias network realizes the U_T/n
  scaling); the normalization is a characterization-ladder concern, stated, not hidden.

