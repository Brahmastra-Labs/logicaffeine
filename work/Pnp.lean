import Mathlib

set_option linter.style.header false
set_option linter.unusedSimpArgs false

/-!
# pnp — P vs NP via hypercube subcube-cover SAT

The main object is **not** an arbitrary Turing machine. It is a compact
3-SAT / hypercube-cover instance, together with algorithms and proof systems over it.
The P-vs-NP-shaped target is `ThreeSATInP`: a worst-case polynomial-time decider for
hypercube-cover SAT.

Sections:
* §A Basic        — vertices, blockers, `CoverSAT`.
* §B Clause3      — ordinary 3-SAT clauses and the bridge to blockers.
* §C Complexity   — algorithms, worst-case polynomial time, `ThreeSATInP`.
* §D BruteForce   — a proven exponential decider and a linear-time verifier.
* §E Patterns     — partial patterns, width, the polynomial/exponential line.
* §F Normalized   — unique cover normalized to `0ⁿ`; blocker types.
* §G Invariants   — type counts, global balance, lower bounds.
* §H DNF          — exact-width DNF equivalence with `ORₙ`.
-/

namespace HypercubeSAT

/-! ## §A Basic: hypercube-cover SAT -/

/-- A Boolean vertex of the `n`-dimensional hypercube. -/
abbrev Vertex (n : Nat) := Fin n → Bool

/-- The finite set of all hypercube vertices. -/
def allVertices (n : Nat) : Finset (Vertex n) := Finset.univ

/-- The all-zero vertex. -/
def zeroVertex (n : Nat) : Vertex n := fun _ => false

/-- Flip one coordinate of a vertex. These are the edges of the Boolean hypercube. -/
def flipVertex {n : Nat} (v : Vertex n) (i : Fin n) : Vertex n :=
  fun j => if j = i then !(v j) else v j

@[simp] theorem flipVertex_at {n : Nat} (v : Vertex n) (i : Fin n) :
    flipVertex v i i = !(v i) := by
  simp [flipVertex]

@[simp] theorem flipVertex_ne {n : Nat} (v : Vertex n) {i j : Fin n} (hji : j ≠ i) :
    flipVertex v i j = v j := by
  simp [flipVertex, hji]

/-- Hypercube adjacency: two vertices differ by flipping one coordinate. -/
def Adjacent {n : Nat} (v w : Vertex n) : Prop :=
  ∃ i : Fin n, w = flipVertex v i

/-- Coordinates on which two vertices differ. -/
def differingCoords {n : Nat} (v w : Vertex n) : Finset (Fin n) :=
  Finset.univ.filter fun i => v i ≠ w i

@[simp] theorem mem_differingCoords {n : Nat} (v w : Vertex n) (i : Fin n) :
    i ∈ differingCoords v w ↔ v i ≠ w i := by
  simp [differingCoords]

/-- Hamming distance on the Boolean hypercube. -/
def hammingDistance {n : Nat} (v w : Vertex n) : Nat :=
  (differingCoords v w).card

/-- Hamming distance is at most the dimension. -/
theorem hammingDistance_le_dimension {n : Nat} (v w : Vertex n) :
    hammingDistance v w ≤ n := by
  unfold hammingDistance differingCoords
  have hsub : (Finset.univ.filter fun i : Fin n => v i ≠ w i) ⊆ Finset.univ := by
    intro i hi
    exact Finset.mem_univ i
  have hle := Finset.card_le_card hsub
  simpa [Fintype.card_fin] using hle

/-- A vertex has no differing coordinates from itself. -/
@[simp] theorem differingCoords_self {n : Nat} (v : Vertex n) :
    differingCoords v v = ∅ := by
  ext i
  simp [differingCoords]

/-- Hamming distance from a vertex to itself is zero. -/
@[simp] theorem hammingDistance_self {n : Nat} (v : Vertex n) :
    hammingDistance v v = 0 := by
  simp [hammingDistance]

/-- Flipping coordinate `i` changes exactly coordinate `i`. -/
theorem differingCoords_flipVertex {n : Nat} (v : Vertex n) (i : Fin n) :
    differingCoords v (flipVertex v i) = {i} := by
  ext j
  by_cases hji : j = i
  · subst j
    cases v i <;> simp [differingCoords]
  · have hsame : flipVertex v i j = v j := by
      exact flipVertex_ne v hji
    simp [differingCoords, hji, hsame]

/-- One coordinate flip has Hamming distance one. -/
@[simp] theorem hammingDistance_flipVertex {n : Nat} (v : Vertex n) (i : Fin n) :
    hammingDistance v (flipVertex v i) = 1 := by
  simp [hammingDistance, differingCoords_flipVertex]

/-- Adjacency forces Hamming distance one. -/
theorem Adjacent.hammingDistance_eq_one {n : Nat} {v w : Vertex n}
    (h : Adjacent v w) :
    hammingDistance v w = 1 := by
  rcases h with ⟨i, rfl⟩
  simp

/-- The Hamming sphere of radius `r` around `center`. -/
def HammingSphere {n : Nat} (center : Vertex n) (r : Nat) : Finset (Vertex n) :=
  (allVertices n).filter fun v => hammingDistance center v = r

/-- The Hamming ball of radius `r` around `center`. -/
def HammingBall {n : Nat} (center : Vertex n) (r : Nat) : Finset (Vertex n) :=
  (allVertices n).filter fun v => hammingDistance center v ≤ r

@[simp] theorem mem_HammingSphere {n r : Nat} (center v : Vertex n) :
    v ∈ HammingSphere center r ↔ hammingDistance center v = r := by
  simp [HammingSphere, allVertices]

@[simp] theorem mem_HammingBall {n r : Nat} (center v : Vertex n) :
    v ∈ HammingBall center r ↔ hammingDistance center v ≤ r := by
  simp [HammingBall, allVertices]

/-- The local neighborhood of a vertex: all one-flip neighbors. -/
def vertexNeighborhood {n : Nat} (v : Vertex n) : Finset (Vertex n) :=
  (Finset.univ : Finset (Fin n)).image fun i => flipVertex v i

/-- Every explicit coordinate flip is in the local neighborhood. -/
theorem flipVertex_mem_neighborhood {n : Nat} (v : Vertex n) (i : Fin n) :
    flipVertex v i ∈ vertexNeighborhood v := by
  exact Finset.mem_image.mpr ⟨i, Finset.mem_univ i, rfl⟩

/-- Neighborhood membership gives an adjacent vertex. -/
theorem adjacent_of_mem_neighborhood {n : Nat} {v w : Vertex n}
    (h : w ∈ vertexNeighborhood v) :
    Adjacent v w := by
  rw [vertexNeighborhood] at h
  rcases Finset.mem_image.mp h with ⟨i, -, rfl⟩
  exact ⟨i, rfl⟩

/-- Neighborhood vertices are exactly Hamming-distance-one vertices, in the forward direction. -/
theorem hammingDistance_eq_one_of_mem_neighborhood {n : Nat} {v w : Vertex n}
    (h : w ∈ vertexNeighborhood v) :
    hammingDistance v w = 1 :=
  (adjacent_of_mem_neighborhood h).hammingDistance_eq_one

/-- The vertex boundary of a set of vertices: points outside the set that touch it by an edge. -/
def VertexBoundary {n : Nat} (S : Finset (Vertex n)) : Finset (Vertex n) :=
  (allVertices n).filter fun w => w ∉ S ∧ ∃ v ∈ S, hammingDistance v w = 1

@[simp] theorem mem_VertexBoundary {n : Nat} (S : Finset (Vertex n)) (w : Vertex n) :
    w ∈ VertexBoundary S ↔ w ∉ S ∧ ∃ v ∈ S, hammingDistance v w = 1 := by
  simp [VertexBoundary, allVertices]

@[simp] theorem mem_allVertices {n : Nat} (v : Vertex n) :
    v ∈ allVertices n := by
  simp [allVertices]

/-- The `n`-cube has `2^n` vertices. -/
theorem allVertices_card (n : Nat) :
    (allVertices n).card = 2 ^ n := by
  simp [allVertices, Vertex]

/-- A clean 3-SAT blocker: it fixes three distinct coordinates of the hypercube.
A vertex is blocked iff it matches all three fixed bits (one falsifying 3-clause). -/
structure Blocker (n : Nat) where
  i : Fin n
  j : Fin n
  k : Fin n
  hij : i ≠ j
  hik : i ≠ k
  hjk : j ≠ k
  bi : Bool
  bj : Bool
  bk : Bool
  deriving DecidableEq, Fintype

/-- A blocker covers a vertex when the vertex matches all three fixed bits. -/
def Blocker.Covers {n : Nat} (B : Blocker n) (v : Vertex n) : Prop :=
  v B.i = B.bi ∧ v B.j = B.bj ∧ v B.k = B.bk

instance {n : Nat} (B : Blocker n) (v : Vertex n) : Decidable (B.Covers v) := by
  unfold Blocker.Covers; infer_instance

/-- The three fixed coordinates of a clean blocker. -/
def Blocker.support {n : Nat} (B : Blocker n) : Finset (Fin n) :=
  {B.i, B.j, B.k}

@[simp] theorem Blocker.mem_support {n : Nat} (B : Blocker n) (x : Fin n) :
    x ∈ B.support ↔ x = B.i ∨ x = B.j ∨ x = B.k := by
  simp [Blocker.support]

theorem Blocker.support_card {n : Nat} (B : Blocker n) :
    B.support.card = 3 := by
  simp [Blocker.support, B.hij, B.hik, B.hjk]

/-- Coordinates free inside a blocker subcube. -/
def Blocker.freeCoordinates {n : Nat} (B : Blocker n) : Finset (Fin n) :=
  Finset.univ \ B.support

@[simp] theorem Blocker.mem_freeCoordinates {n : Nat} (B : Blocker n) (x : Fin n) :
    x ∈ B.freeCoordinates ↔ x ∉ B.support := by
  simp [Blocker.freeCoordinates]

/-- A clean blocker has `n - 3` free coordinates. -/
theorem Blocker.freeCoordinates_card {n : Nat} (B : Blocker n) :
    B.freeCoordinates.card = n - 3 := by
  have hsub : B.support ⊆ (Finset.univ : Finset (Fin n)) := by
    intro x hx
    exact Finset.mem_univ x
  rw [Blocker.freeCoordinates, Finset.card_sdiff_of_subset hsub]
  rw [Finset.card_univ, Fintype.card_fin, B.support_card]

/-- The exact vertex footprint of one blocker. -/
def Blocker.footprint {n : Nat} (B : Blocker n) : Finset (Vertex n) :=
  (allVertices n).filter fun v => B.Covers v

@[simp] theorem Blocker.mem_footprint {n : Nat} (B : Blocker n) (v : Vertex n) :
    v ∈ B.footprint ↔ B.Covers v := by
  simp [Blocker.footprint, allVertices]

/-- Projection of a vertex onto the free coordinates of a blocker. -/
def Blocker.freeProjection {n : Nat} (B : Blocker n) (v : Vertex n) :
    ({x : Fin n // x ∈ B.freeCoordinates} → Bool) :=
  fun x => v x

/-- A vertex covered by a blocker is determined by its free-coordinate projection. -/
theorem Blocker.eq_of_covers_and_freeProjection_eq {n : Nat} (B : Blocker n)
    {v w : Vertex n} (hv : B.Covers v) (hw : B.Covers w)
    (hfree : B.freeProjection v = B.freeProjection w) :
    v = w := by
  funext x
  by_cases hx : x ∈ B.support
  · rw [Blocker.mem_support] at hx
    rcases hx with rfl | rfl | rfl
    · exact hv.1.trans hw.1.symm
    · exact hv.2.1.trans hw.2.1.symm
    · exact hv.2.2.trans hw.2.2.symm
  · have hxfree : x ∈ B.freeCoordinates := by
      simpa using hx
    have hpoint := congrFun hfree ⟨x, hxfree⟩
    exact hpoint

/-- Capacity bound for one clean blocker: its footprint injects into assignments on its
`n - 3` free coordinates. -/
theorem Blocker.footprint_card_le_freeAssignments {n : Nat} (B : Blocker n) :
    B.footprint.card ≤ Fintype.card ({x : Fin n // x ∈ B.freeCoordinates} → Bool) := by
  classical
  rw [← Fintype.card_coe B.footprint]
  apply Fintype.card_le_of_injective
    (fun v : {v : Vertex n // v ∈ B.footprint} => B.freeProjection v.val)
  intro v w h
  apply Subtype.ext
  exact B.eq_of_covers_and_freeProjection_eq
    ((B.mem_footprint v.val).mp v.property)
    ((B.mem_footprint w.val).mp w.property)
    h

/-- One clean 3-blocker covers at most `2^(n-3)` vertices. -/
theorem Blocker.footprint_card_le {n : Nat} (B : Blocker n) :
    B.footprint.card ≤ 2 ^ (n - 3) := by
  calc
    B.footprint.card ≤ Fintype.card ({x : Fin n // x ∈ B.freeCoordinates} → Bool) :=
      B.footprint_card_le_freeAssignments
    _ = 2 ^ Fintype.card {x : Fin n // x ∈ B.freeCoordinates} := by
      rw [Fintype.card_fun, Fintype.card_bool]
    _ = 2 ^ B.freeCoordinates.card := by rw [Fintype.card_coe]
    _ = 2 ^ (n - 3) := by rw [B.freeCoordinates_card]

theorem Blocker.not_covers_flip_i {n : Nat} (B : Blocker n) {v : Vertex n}
    (hcover : B.Covers v) :
    ¬ B.Covers (flipVertex v B.i) := by
  intro hflip
  have hi : flipVertex v B.i B.i = B.bi := hflip.1
  rw [flipVertex_at, hcover.1] at hi
  cases B.bi <;> simp at hi

theorem Blocker.not_covers_flip_j {n : Nat} (B : Blocker n) {v : Vertex n}
    (hcover : B.Covers v) :
    ¬ B.Covers (flipVertex v B.j) := by
  intro hflip
  have hj : flipVertex v B.j B.j = B.bj := hflip.2.1
  rw [flipVertex_at, hcover.2.1] at hj
  cases B.bj <;> simp at hj

theorem Blocker.not_covers_flip_k {n : Nat} (B : Blocker n) {v : Vertex n}
    (hcover : B.Covers v) :
    ¬ B.Covers (flipVertex v B.k) := by
  intro hflip
  have hk : flipVertex v B.k B.k = B.bk := hflip.2.2
  rw [flipVertex_at, hcover.2.2] at hk
  cases B.bk <;> simp at hk

/-- Flipping any coordinate fixed by a blocker escapes that blocker. -/
theorem Blocker.not_covers_flip_of_mem_support {n : Nat} (B : Blocker n)
    {v : Vertex n} {x : Fin n} (hcover : B.Covers v) (hx : x ∈ B.support) :
    ¬ B.Covers (flipVertex v x) := by
  rw [Blocker.mem_support] at hx
  rcases hx with rfl | rfl | rfl
  · exact B.not_covers_flip_i hcover
  · exact B.not_covers_flip_j hcover
  · exact B.not_covers_flip_k hcover

/-- Flipping a coordinate not fixed by a blocker stays inside that blocker. -/
theorem Blocker.covers_flip_of_not_mem_support {n : Nat} (B : Blocker n)
    {v : Vertex n} {x : Fin n} (hcover : B.Covers v) (hx : x ∉ B.support) :
    B.Covers (flipVertex v x) := by
  rw [Blocker.mem_support] at hx
  have hxi : B.i ≠ x := by
    intro h
    exact hx (Or.inl h.symm)
  have hxj : B.j ≠ x := by
    intro h
    exact hx (Or.inr (Or.inl h.symm))
  have hxk : B.k ≠ x := by
    intro h
    exact hx (Or.inr (Or.inr h.symm))
  constructor
  · simpa [flipVertex_ne v hxi] using hcover.1
  · constructor
    · simpa [flipVertex_ne v hxj] using hcover.2.1
    · simpa [flipVertex_ne v hxk] using hcover.2.2

/-- Flipping a coordinate not fixed by a blocker preserves membership in that blocker's
forbidden subcube, in both directions. -/
theorem Blocker.covers_flip_iff_of_not_mem_support {n : Nat} (B : Blocker n)
    {v : Vertex n} {x : Fin n} (hx : x ∉ B.support) :
    B.Covers (flipVertex v x) ↔ B.Covers v := by
  constructor
  · intro hcover
    rw [Blocker.mem_support] at hx
    have hxi : B.i ≠ x := by
      intro h
      exact hx (Or.inl h.symm)
    have hxj : B.j ≠ x := by
      intro h
      exact hx (Or.inr (Or.inl h.symm))
    have hxk : B.k ≠ x := by
      intro h
      exact hx (Or.inr (Or.inr h.symm))
    constructor
    · simpa [flipVertex_ne v hxi] using hcover.1
    · constructor
      · simpa [flipVertex_ne v hxj] using hcover.2.1
      · simpa [flipVertex_ne v hxk] using hcover.2.2
  · intro hcover
    exact B.covers_flip_of_not_mem_support hcover hx

/-- A blocker footprint is closed under every free-coordinate edge. -/
theorem Blocker.footprint_closed_under_free_flip {n : Nat} (B : Blocker n)
    {v : Vertex n} {x : Fin n} (hv : v ∈ B.footprint) (hx : x ∈ B.freeCoordinates) :
    flipVertex v x ∈ B.footprint := by
  rw [Blocker.mem_footprint] at hv ⊢
  exact B.covers_flip_of_not_mem_support hv ((B.mem_freeCoordinates x).mp hx)

/-- A fixed-coordinate edge leaves the blocker footprint. -/
theorem Blocker.flip_fixed_not_mem_footprint {n : Nat} (B : Blocker n)
    {v : Vertex n} {x : Fin n} (hv : v ∈ B.footprint) (hx : x ∈ B.support) :
    flipVertex v x ∉ B.footprint := by
  rw [Blocker.mem_footprint] at hv ⊢
  exact B.not_covers_flip_of_mem_support hv hx

/-- A clean 3-blocker can exist only in dimension at least `3`. -/
theorem Blocker.dimension_ge_three {n : Nat} (B : Blocker n) : 3 ≤ n := by
  have hsub : ({B.i, B.j, B.k} : Finset (Fin n)) ⊆ Finset.univ := by
    intro x hx
    exact Finset.mem_univ x
  have hle := Finset.card_le_card hsub
  have hcard : ({B.i, B.j, B.k} : Finset (Fin n)).card = 3 := by
    simp [B.hij, B.hik, B.hjk]
  rw [hcard, Finset.card_univ, Fintype.card_fin] at hle
  exact hle

/-- A compact hypercube-cover input. -/
structure CoverInput where
  n : Nat
  blockers : List (Blocker n)

/-- A blocker list is duplicate-free when it contains no repeated clean blocker. -/
def BlockerListNodup {n : Nat} (Bs : List (Blocker n)) : Prop :=
  Bs.Nodup

/-- Duplicate-free blocker lists are bounded by the finite universe of clean 3-blockers. -/
theorem blockerList_length_le_total_of_nodup {n : Nat} {Bs : List (Blocker n)}
    (hnd : BlockerListNodup Bs) :
    Bs.length ≤ Fintype.card (Blocker n) := by
  have hcard : Bs.toFinset.card = Bs.length := List.toFinset_card_of_nodup hnd
  rw [← hcard]
  exact Finset.card_le_univ Bs.toFinset

/-- A vertex is uncovered if no blocker covers it. -/
def IsUncovered {n : Nat} (Bs : List (Blocker n)) (v : Vertex n) : Prop :=
  ∀ B ∈ Bs, ¬ B.Covers v

instance {n : Nat} (Bs : List (Blocker n)) (v : Vertex n) : Decidable (IsUncovered Bs v) := by
  unfold IsUncovered; infer_instance

/-- Removing blockers cannot destroy an uncovered vertex. -/
theorem IsUncovered.of_subset {n : Nat} {Bs Cs : List (Blocker n)} {v : Vertex n}
    (hsub : ∀ B, B ∈ Cs → B ∈ Bs) (h : IsUncovered Bs v) :
    IsUncovered Cs v := by
  intro B hB
  exact h B (hsub B hB)

/-- List-level satisfiability for a fixed dimension. -/
def CoverSATList {n : Nat} (Bs : List (Blocker n)) : Prop :=
  ∃ v : Vertex n, IsUncovered Bs v

/-- List-level unsatisfiability/full coverage for a fixed dimension. -/
def CoverUNSATList {n : Nat} (Bs : List (Blocker n)) : Prop :=
  ¬ CoverSATList Bs

/-- With no blockers, every vertex is uncovered. -/
theorem isUncovered_nil {n : Nat} (v : Vertex n) :
    IsUncovered ([] : List (Blocker n)) v := by
  intro B hB
  simp at hB

/-- The empty blocker list is satisfiable. -/
theorem coverSATList_nil (n : Nat) :
    CoverSATList ([] : List (Blocker n)) := by
  exact ⟨zeroVertex n, isUncovered_nil _⟩

/-- A single blocker never covers the whole cube: flip one of its fixed coordinates. -/
theorem Blocker.exists_not_covers {n : Nat} (B : Blocker n) :
    ∃ v : Vertex n, ¬ B.Covers v := by
  let v : Vertex n := fun x => if x = B.i then !B.bi else false
  refine ⟨v, ?_⟩
  intro h
  have hi : v B.i = B.bi := h.1
  have hflip : v B.i = !B.bi := by
    simp [v]
  rw [hflip] at hi
  cases B.bi <;> simp at hi

/-- Every singleton blocker list is satisfiable. -/
theorem coverSATList_singleton {n : Nat} (B : Blocker n) :
    CoverSATList [B] := by
  obtain ⟨v, hv⟩ := B.exists_not_covers
  refine ⟨v, ?_⟩
  intro C hC
  rcases List.mem_singleton.mp hC with rfl
  exact hv

/-- Removing blockers preserves satisfiability. -/
theorem CoverSATList.of_subset {n : Nat} {Bs Cs : List (Blocker n)}
    (hsub : ∀ B, B ∈ Cs → B ∈ Bs) (h : CoverSATList Bs) :
    CoverSATList Cs := by
  obtain ⟨v, hv⟩ := h
  exact ⟨v, hv.of_subset hsub⟩

/-- Adding blockers preserves unsatisfiability/full coverage. -/
theorem CoverUNSATList.of_superset {n : Nat} {Bs Cs : List (Blocker n)}
    (hsub : ∀ B, B ∈ Bs → B ∈ Cs) (h : CoverUNSATList Bs) :
    CoverUNSATList Cs := by
  intro hsat
  obtain ⟨v, hv⟩ := hsat
  exact h ⟨v, hv.of_subset hsub⟩

/-- Adding one blocker preserves unsatisfiability/full coverage. -/
theorem CoverUNSATList.cons {n : Nat} (B : Blocker n) {Bs : List (Blocker n)}
    (h : CoverUNSATList Bs) :
    CoverUNSATList (B :: Bs) := by
  apply CoverUNSATList.of_superset (Bs := Bs)
  · intro C hC
    exact List.mem_cons_of_mem B hC
  · exact h

/-- Any unsatisfiable/full-cover list has at least one blocker. -/
theorem CoverUNSATList.length_pos {n : Nat} {Bs : List (Blocker n)}
    (h : CoverUNSATList Bs) :
    0 < Bs.length := by
  cases Bs with
  | nil =>
      exact False.elim (h (coverSATList_nil n))
  | cons B Bs =>
      simp

/-- Any unsatisfiable/full-cover list needs at least two blockers. -/
theorem CoverUNSATList.length_two_le {n : Nat} {Bs : List (Blocker n)}
    (h : CoverUNSATList Bs) :
    2 ≤ Bs.length := by
  cases Bs with
  | nil =>
      exact False.elim (h (coverSATList_nil n))
  | cons B Bs =>
      cases Bs with
      | nil =>
          exact False.elim (h (coverSATList_singleton B))
      | cons C Bs =>
          simp

/-- Because list inputs allow duplicates/redundant blockers, full-cover lists can be padded
arbitrarily once one full cover exists. -/
theorem CoverUNSATList.pad_left {n : Nat} (extra : List (Blocker n)) {Bs : List (Blocker n)}
    (h : CoverUNSATList Bs) :
    CoverUNSATList (extra ++ Bs) := by
  apply CoverUNSATList.of_superset (Bs := Bs)
  · intro B hB
    exact List.mem_append_right extra hB
  · exact h

/-! ### §A1 Vertex energy, solution count, and full-cover cores -/

/-- The energy of a vertex is the number of blockers covering it. Zero-energy vertices are
exactly satisfying assignments. -/
def vertexEnergy {n : Nat} (Bs : List (Blocker n)) (v : Vertex n) : Nat :=
  (Bs.filter fun B => B.Covers v).length

theorem vertexEnergy_le_length {n : Nat} (Bs : List (Blocker n)) (v : Vertex n) :
    vertexEnergy Bs v ≤ Bs.length := by
  unfold vertexEnergy
  exact List.length_filter_le _ _

theorem vertexEnergy_eq_zero_iff_isUncovered {n : Nat} (Bs : List (Blocker n))
    (v : Vertex n) :
    vertexEnergy Bs v = 0 ↔ IsUncovered Bs v := by
  constructor
  · intro hzero B hB hcover
    have hmem : B ∈ Bs.filter fun C => C.Covers v := by
      exact List.mem_filter.mpr ⟨hB, decide_eq_true hcover⟩
    have hpos : 0 < (Bs.filter fun C => C.Covers v).length :=
      List.length_pos_of_mem hmem
    unfold vertexEnergy at hzero
    omega
  · intro hun
    unfold vertexEnergy
    rw [List.length_eq_zero_iff]
    apply List.eq_nil_iff_forall_not_mem.mpr
    intro B hB
    have hparts := List.mem_filter.mp hB
    have hBbase : B ∈ Bs := hparts.1
    have hcover : B.Covers v := of_decide_eq_true hparts.2
    exact hun B hBbase hcover

/-- The finite set of satisfying vertices for a blocker list. -/
def uncoveredVertices {n : Nat} (Bs : List (Blocker n)) : Finset (Vertex n) :=
  (allVertices n).filter fun v => IsUncovered Bs v

@[simp] theorem mem_uncoveredVertices {n : Nat} (Bs : List (Blocker n)) (v : Vertex n) :
    v ∈ uncoveredVertices Bs ↔ IsUncovered Bs v := by
  simp [uncoveredVertices]

/-- Number of satisfying assignments. -/
def solutionCount {n : Nat} (Bs : List (Blocker n)) : Nat :=
  (uncoveredVertices Bs).card

/-- UniqueSAT in the hypercube list model: exactly one zero-energy vertex. -/
def UniqueSATList {n : Nat} (Bs : List (Blocker n)) : Prop :=
  solutionCount Bs = 1

theorem coverSATList_iff_solutionCount_pos {n : Nat} (Bs : List (Blocker n)) :
    CoverSATList Bs ↔ 0 < solutionCount Bs := by
  constructor
  · rintro ⟨v, hv⟩
    unfold solutionCount
    exact Finset.card_pos.mpr ⟨v, (mem_uncoveredVertices Bs v).mpr hv⟩
  · intro hpos
    unfold solutionCount at hpos
    obtain ⟨v, hv⟩ := Finset.card_pos.mp hpos
    exact ⟨v, (mem_uncoveredVertices Bs v).mp hv⟩

theorem coverUNSATList_iff_solutionCount_eq_zero {n : Nat} (Bs : List (Blocker n)) :
    CoverUNSATList Bs ↔ solutionCount Bs = 0 := by
  unfold CoverUNSATList
  rw [coverSATList_iff_solutionCount_pos]
  omega

theorem coverSATList_iff_exists_energy_zero {n : Nat} (Bs : List (Blocker n)) :
    CoverSATList Bs ↔ ∃ v : Vertex n, vertexEnergy Bs v = 0 := by
  constructor
  · rintro ⟨v, hv⟩
    exact ⟨v, (vertexEnergy_eq_zero_iff_isUncovered Bs v).mpr hv⟩
  · rintro ⟨v, hv⟩
    exact ⟨v, (vertexEnergy_eq_zero_iff_isUncovered Bs v).mp hv⟩

theorem coverUNSATList_iff_forall_energy_pos {n : Nat} (Bs : List (Blocker n)) :
    CoverUNSATList Bs ↔ ∀ v : Vertex n, 0 < vertexEnergy Bs v := by
  unfold CoverUNSATList
  rw [coverSATList_iff_exists_energy_zero]
  constructor
  · intro h v
    have hv : vertexEnergy Bs v ≠ 0 := by
      intro hz
      exact h ⟨v, hz⟩
    omega
  · intro h hsat
    obtain ⟨v, hv⟩ := hsat
    have hpos := h v
    omega

theorem uniqueSATList_iff_exists_unique_uncovered {n : Nat} (Bs : List (Blocker n)) :
    UniqueSATList Bs ↔
      ∃ v : Vertex n, IsUncovered Bs v ∧ ∀ w : Vertex n, IsUncovered Bs w → w = v := by
  unfold UniqueSATList solutionCount
  constructor
  · intro hcard
    obtain ⟨v, hvset⟩ := Finset.card_eq_one.mp hcard
    refine ⟨v, ?_, ?_⟩
    · have hv : v ∈ uncoveredVertices Bs := by rw [hvset]; simp
      exact (mem_uncoveredVertices Bs v).mp hv
    · intro w hw
      have hwmem : w ∈ uncoveredVertices Bs := (mem_uncoveredVertices Bs w).mpr hw
      rw [hvset] at hwmem
      exact Finset.mem_singleton.mp hwmem
  · rintro ⟨v, hv, huniq⟩
    apply Finset.card_eq_one.mpr
    refine ⟨v, ?_⟩
    apply Finset.ext
    intro w
    constructor
    · intro hw
      exact Finset.mem_singleton.mpr (huniq w ((mem_uncoveredVertices Bs w).mp hw))
    · intro hw
      rw [Finset.mem_singleton] at hw
      subst w
      exact (mem_uncoveredVertices Bs v).mpr hv

theorem uniqueSATList_iff_exists_unique_energy_zero {n : Nat} (Bs : List (Blocker n)) :
    UniqueSATList Bs ↔
      ∃ v : Vertex n, vertexEnergy Bs v = 0 ∧
        ∀ w : Vertex n, vertexEnergy Bs w = 0 → w = v := by
  rw [uniqueSATList_iff_exists_unique_uncovered]
  constructor
  · rintro ⟨v, hv, huniq⟩
    refine ⟨v, (vertexEnergy_eq_zero_iff_isUncovered Bs v).mpr hv, ?_⟩
    intro w hw
    exact huniq w ((vertexEnergy_eq_zero_iff_isUncovered Bs w).mp hw)
  · rintro ⟨v, hv, huniq⟩
    refine ⟨v, (vertexEnergy_eq_zero_iff_isUncovered Bs v).mp hv, ?_⟩
    intro w hw
    exact huniq w ((vertexEnergy_eq_zero_iff_isUncovered Bs w).mpr hw)

/-- Total blocker-vertex incidence mass over the whole cube. -/
def totalVertexEnergy {n : Nat} (Bs : List (Blocker n)) : Nat :=
  ∑ v ∈ allVertices n, vertexEnergy Bs v

/-- Total energy is bounded above by the full blocker-by-vertex scan rectangle. -/
theorem totalVertexEnergy_le_length_mul_vertices {n : Nat} (Bs : List (Blocker n)) :
    totalVertexEnergy Bs ≤ Bs.length * 2 ^ n := by
  unfold totalVertexEnergy
  calc
    (∑ v ∈ allVertices n, vertexEnergy Bs v)
        ≤ ∑ v ∈ allVertices n, Bs.length := by
          apply Finset.sum_le_sum
          intro v hv
          exact vertexEnergy_le_length Bs v
    _ = Bs.length * 2 ^ n := by
          rw [Finset.sum_const, smul_eq_mul, allVertices_card]
          ring

/-- Full coverage forces at least one unit of energy on every vertex. -/
theorem totalVertexEnergy_ge_vertices_of_unsat {n : Nat} {Bs : List (Blocker n)}
    (hunsat : CoverUNSATList Bs) :
    2 ^ n ≤ totalVertexEnergy Bs := by
  have hpos := (coverUNSATList_iff_forall_energy_pos Bs).mp hunsat
  unfold totalVertexEnergy
  calc
    2 ^ n = (allVertices n).card := by rw [allVertices_card]
    _ = ∑ v ∈ allVertices n, (1 : Nat) := by
          rw [Finset.sum_const, smul_eq_mul, mul_one]
    _ ≤ ∑ v ∈ allVertices n, vertexEnergy Bs v := by
          apply Finset.sum_le_sum
          intro v hv
          exact hpos v

/-- Full coverage by a finite set of clean blockers. -/
def FullCoverBlockers {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  ∀ v : Vertex n, ∃ B ∈ Bs, B.Covers v

/-- Vertices covered by at least one blocker in a finite family. -/
noncomputable def coveredVertices {n : Nat} (Bs : Finset (Blocker n)) : Finset (Vertex n) := by
  classical
  exact (allVertices n).filter fun v => ∃ B ∈ Bs, B.Covers v

@[simp] theorem mem_coveredVertices {n : Nat} {Bs : Finset (Blocker n)}
    {v : Vertex n} :
    v ∈ coveredVertices Bs ↔ ∃ B ∈ Bs, B.Covers v := by
  classical
  simp [coveredVertices]

/-- Covered vertices are contained in the union of the individual blocker footprints. -/
theorem coveredVertices_subset_biUnion_footprint {n : Nat} (Bs : Finset (Blocker n)) :
    coveredVertices Bs ⊆ Bs.biUnion (fun B => B.footprint) := by
  intro v hv
  obtain ⟨B, hB, hcover⟩ := mem_coveredVertices.mp hv
  exact Finset.mem_biUnion.mpr ⟨B, hB, (B.mem_footprint v).mpr hcover⟩

/-- Union bound: the number of covered vertices is at most the sum of blocker footprint sizes. -/
theorem coveredVertices_card_le_sum_footprints {n : Nat} (Bs : Finset (Blocker n)) :
    (coveredVertices Bs).card ≤ ∑ B ∈ Bs, B.footprint.card := by
  have hcard : (coveredVertices Bs).card ≤
      (Bs.biUnion (fun B => B.footprint)).card :=
    Finset.card_le_card (coveredVertices_subset_biUnion_footprint Bs)
  have hunion : (Bs.biUnion (fun B => B.footprint)).card ≤
      ∑ B ∈ Bs, B.footprint.card :=
    Finset.card_biUnion_le
  exact hcard.trans hunion

/-- Density/volume upper bound: `m` clean 3-blockers cover at most
`m * 2^(n-3)` vertices. -/
theorem coveredVertices_card_le_card_mul_capacity {n : Nat} (Bs : Finset (Blocker n)) :
    (coveredVertices Bs).card ≤ Bs.card * 2 ^ (n - 3) := by
  calc
    (coveredVertices Bs).card ≤ ∑ B ∈ Bs, B.footprint.card :=
      coveredVertices_card_le_sum_footprints Bs
    _ ≤ ∑ B ∈ Bs, 2 ^ (n - 3) := by
      exact Finset.sum_le_sum (fun B hB => B.footprint_card_le)
    _ = Bs.card * 2 ^ (n - 3) := by
      rw [Finset.sum_const, nsmul_eq_mul]
      rfl

/-- Coordinate support of a clean-blocker family. -/
def BlockerFamily.support {n : Nat} (Bs : Finset (Blocker n)) : Finset (Fin n) :=
  Bs.biUnion Blocker.support

theorem BlockerFamily.mem_support_iff {n : Nat} (Bs : Finset (Blocker n)) (x : Fin n) :
    x ∈ BlockerFamily.support Bs ↔ ∃ B ∈ Bs, x ∈ B.support := by
  unfold BlockerFamily.support
  simp

/-- A coordinate is essential to a family when some blocker fixes it. -/
def VariableUsed {n : Nat} (Bs : Finset (Blocker n)) (x : Fin n) : Prop :=
  x ∈ BlockerFamily.support Bs

/-- A family uses every variable. Worst-case candidates should usually be in this envelope;
otherwise there is an unused product direction. -/
def UsesAllVariables {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  BlockerFamily.support Bs = Finset.univ

/-- Blocker adjacency through a shared coordinate. -/
def BlockerTouches {n : Nat} (B C : Blocker n) : Prop :=
  ∃ x : Fin n, x ∈ B.support ∧ x ∈ C.support

/-- A blocker family is pairwise variable-disjoint when no two distinct blockers touch.
Such families are structurally decomposed, not circular. -/
def PairwiseDisjointBlockers {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  ∀ B ∈ Bs, ∀ C ∈ Bs, B ≠ C → ¬ BlockerTouches B C

theorem PairwiseDisjointBlockers.support_card_eq_three_mul_card {n : Nat}
    {Bs : Finset (Blocker n)} (hpair : PairwiseDisjointBlockers Bs) :
    (BlockerFamily.support Bs).card = 3 * Bs.card := by
  classical
  revert hpair
  unfold BlockerFamily.support
  refine Finset.induction_on Bs ?base ?step
  · simp
  · intro B Bs hBnot hih hpair
    have hpairRest : PairwiseDisjointBlockers Bs := by
      intro C hC D hD hCD
      exact hpair C (Finset.mem_insert_of_mem hC) D (Finset.mem_insert_of_mem hD) hCD
    have hdisj : Disjoint B.support (Bs.biUnion Blocker.support) := by
      rw [Finset.disjoint_left]
      intro x hxB hxUnion
      rw [Finset.mem_biUnion] at hxUnion
      obtain ⟨C, hC, hxC⟩ := hxUnion
      exact hpair B (Finset.mem_insert_self B Bs) C (Finset.mem_insert_of_mem hC)
        (by intro h; subst C; exact hBnot hC) ⟨x, hxB, hxC⟩
    rw [Finset.biUnion_insert, Finset.card_union_of_disjoint hdisj, B.support_card, hih hpairRest]
    rw [Finset.card_insert_of_notMem hBnot]
    ring

theorem PairwiseDisjointBlockers.card_le_n_div_three {n : Nat}
    {Bs : Finset (Blocker n)} (hpair : PairwiseDisjointBlockers Bs) :
    3 * Bs.card ≤ n := by
  have hcard := hpair.support_card_eq_three_mul_card
  have hle : (BlockerFamily.support Bs).card ≤ n := by
    simpa [Fintype.card_fin] using Finset.card_le_univ (BlockerFamily.support Bs)
  omega

/-- Any clean-blocker full cover contains at least one blocker. -/
theorem FullCoverBlockers.card_pos {n : Nat} {Bs : Finset (Blocker n)}
    (hcover : FullCoverBlockers Bs) :
    0 < Bs.card := by
  obtain ⟨B, hB, -⟩ := hcover (zeroVertex n)
  exact Finset.card_pos.mpr ⟨B, hB⟩

/-- Any clean-blocker full cover lives in dimension at least three. -/
theorem FullCoverBlockers.dimension_ge_three {n : Nat} {Bs : Finset (Blocker n)}
    (hcover : FullCoverBlockers Bs) :
    3 ≤ n := by
  obtain ⟨B, -, -⟩ := hcover (zeroVertex n)
  exact B.dimension_ge_three

/-- A single clean blocker cannot be an UNSAT/full-cover core. -/
theorem FullCoverBlockers.card_two_le {n : Nat} {Bs : Finset (Blocker n)}
    (hcover : FullCoverBlockers Bs) :
    2 ≤ Bs.card := by
  by_contra hnot
  have hle : Bs.card ≤ 1 := by omega
  rcases Nat.le_one_iff_eq_zero_or_eq_one.mp hle with hzero | hone
  · rw [Finset.card_eq_zero] at hzero
    obtain ⟨B, hB, -⟩ := hcover (zeroVertex n)
    rw [hzero] at hB
    simp at hB
  · obtain ⟨B, hBs⟩ := Finset.card_eq_one.mp hone
    obtain ⟨v, hv⟩ := B.exists_not_covers
    obtain ⟨C, hC, hCv⟩ := hcover v
    rw [hBs] at hC
    have hCB : C = B := Finset.mem_singleton.mp hC
    subst C
    exact hv hCv

/-- A full cover must have enough blocker volume to cover all `2^n` vertices. -/
theorem FullCoverBlockers.vertices_le_card_mul_capacity {n : Nat} {Bs : Finset (Blocker n)}
    (hcover : FullCoverBlockers Bs) :
    2 ^ n ≤ Bs.card * 2 ^ (n - 3) := by
  have hsub : allVertices n ⊆ coveredVertices Bs := by
    intro v hv
    exact mem_coveredVertices.mpr (hcover v)
  calc
    2 ^ n = (allVertices n).card := by rw [allVertices_card]
    _ ≤ (coveredVertices Bs).card := Finset.card_le_card hsub
    _ ≤ Bs.card * 2 ^ (n - 3) := coveredVertices_card_le_card_mul_capacity Bs

/-- Sparse-cover rule: every full cover by clean 3-blockers uses at least eight blockers.
This is the basic density obstruction: each blocker covers at most one eighth of the cube. -/
theorem FullCoverBlockers.card_ge_eight {n : Nat} {Bs : Finset (Blocker n)}
    (hcover : FullCoverBlockers Bs) :
    8 ≤ Bs.card := by
  have hn : 3 ≤ n := hcover.dimension_ge_three
  have hvol := hcover.vertices_le_card_mul_capacity
  let cap := 2 ^ (n - 3)
  have hcapPos : 0 < cap := Nat.two_pow_pos (n - 3)
  have hpow : 2 ^ n = 8 * cap := by
    have hn' : n = (n - 3) + 3 := by omega
    have hrewrite : 2 ^ n = 2 ^ ((n - 3) + 3) := congrArg (fun t => 2 ^ t) hn'
    calc
      2 ^ n = 2 ^ ((n - 3) + 3) := hrewrite
      _ = 2 ^ (n - 3) * 2 ^ 3 := by rw [Nat.pow_add]
      _ = 8 * cap := by norm_num [cap, Nat.mul_comm]
  have hmul : 8 * cap ≤ Bs.card * cap := by
    simpa [hpow] using hvol
  exact Nat.le_of_mul_le_mul_right hmul hcapPos

/-- Contrapositive sparse rule: fewer than eight clean blockers always leave a hole. -/
theorem not_fullCoverBlockers_of_card_lt_eight {n : Nat} {Bs : Finset (Blocker n)}
    (hcard : Bs.card < 8) :
    ¬ FullCoverBlockers Bs := by
  intro hcover
  have h8 := hcover.card_ge_eight
  omega

/-- List/finset bridge: a blocker list is UNSAT exactly when its duplicate-free finset image is
a full cover. Duplicates do not change the covered region. -/
theorem coverUNSATList_iff_fullCover_toFinset {n : Nat} (Bs : List (Blocker n)) :
    CoverUNSATList Bs ↔ FullCoverBlockers Bs.toFinset := by
  constructor
  · intro hunsat v
    by_contra hnone
    have hun : IsUncovered Bs v := by
      intro B hB hcover
      exact hnone ⟨B, by simpa using hB, hcover⟩
    exact hunsat ⟨v, hun⟩
  · intro hcover hsat
    obtain ⟨v, hv⟩ := hsat
    obtain ⟨B, hB, hBv⟩ := hcover v
    exact hv B (by simpa using hB) hBv

/-- Sparse SAT rule for lists: fewer than eight clean 3-blockers cannot cover the cube. -/
theorem coverSATList_of_length_lt_eight {n : Nat} {Bs : List (Blocker n)}
    (hlen : Bs.length < 8) :
    CoverSATList Bs := by
  classical
  by_contra hnot
  have hcover : FullCoverBlockers Bs.toFinset :=
    (coverUNSATList_iff_fullCover_toFinset Bs).mp hnot
  have h8 : 8 ≤ Bs.toFinset.card := hcover.card_ge_eight
  have hle : Bs.toFinset.card ≤ Bs.length := by
    exact List.toFinset_card_le Bs
  omega

/-- Full coverage is monotone under adding clean blockers. -/
theorem FullCoverBlockers.mono {n : Nat} {Bs Cs : Finset (Blocker n)}
    (hsub : Bs ⊆ Cs) (hcover : FullCoverBlockers Bs) :
    FullCoverBlockers Cs := by
  intro v
  obtain ⟨B, hB, hBv⟩ := hcover v
  exact ⟨B, hsub hB, hBv⟩

/-- Minimal full cover by clean blockers. -/
def MinimalFullCoverBlockers {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  FullCoverBlockers Bs ∧ ∀ B ∈ Bs, ¬ FullCoverBlockers (Bs.erase B)

/-- A private blocked vertex is covered by `B` and by no other blocker in the full-cover core. -/
def PrivateBlockedVertex {n : Nat} (Bs : Finset (Blocker n)) (B : Blocker n)
    (v : Vertex n) : Prop :=
  B ∈ Bs ∧ B.Covers v ∧ ∀ C ∈ Bs, C.Covers v → C = B

/-- Finset version of vertex energy for a clean-blocker core. -/
def finsetVertexEnergy {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n) : Nat :=
  (Bs.filter fun B => B.Covers v).card

def VertexUncoveredBy {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n) : Prop :=
  finsetVertexEnergy Bs v = 0

def VertexSinglyCoveredBy {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n) : Prop :=
  finsetVertexEnergy Bs v = 1

def VertexOverlappedBy {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n) : Prop :=
  2 ≤ finsetVertexEnergy Bs v

def VertexTightFor {n : Nat} (Bs : Finset (Blocker n)) (B : Blocker n) (v : Vertex n) :
    Prop :=
  PrivateBlockedVertex Bs B v

/-- A vertex is difficult for a full cover when it is tight: exactly one blocker covers it.
Deleting that blocker exposes the vertex. -/
def VertexTight {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n) : Prop :=
  ∃ B ∈ Bs, PrivateBlockedVertex Bs B v

/-- A covered vertex is redundant/overlapped when at least two blockers cover it. Such vertices
are locally robust under deleting one covering blocker. -/
def VertexRedundant {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n) : Prop :=
  VertexOverlappedBy Bs v

/-- A vertex-border step: `v` is covered by `B`, but flipping coordinate `x` is not covered by
`B`. For clean blockers this can only happen along one of the three fixed coordinates. -/
def VertexExitFor {n : Nat} (B : Blocker n) (v : Vertex n) (x : Fin n) : Prop :=
  B.Covers v ∧ ¬ B.Covers (flipVertex v x)

/-- A cover handoff at the vertex level: `B` covers `v`, and after flipping coordinate `x`,
responsibility can move to `C`. -/
def VertexHandoff {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n)
    (x : Fin n) (B C : Blocker n) : Prop :=
  B ∈ Bs ∧ C ∈ Bs ∧ B.Covers v ∧ C.Covers (flipVertex v x) ∧ C ≠ B

theorem VertexUncoveredBy_iff_no_cover {n : Nat} (Bs : Finset (Blocker n)) (v : Vertex n) :
    VertexUncoveredBy Bs v ↔ ∀ B ∈ Bs, ¬ B.Covers v := by
  unfold VertexUncoveredBy
  unfold finsetVertexEnergy
  constructor
  · intro hzero B hB hcover
    have hmem : B ∈ Bs.filter fun C => C.Covers v := by
      exact Finset.mem_filter.mpr ⟨hB, hcover⟩
    have hpos : 0 < (Bs.filter fun C => C.Covers v).card :=
      Finset.card_pos.mpr ⟨B, hmem⟩
    omega
  · intro hnone
    rw [Finset.card_eq_zero]
    ext B
    simp only [Finset.mem_filter, Finset.notMem_empty, iff_false, not_and]
    intro hB
    exact hnone B hB

/-- If a coordinate is unused by the whole blocker family, flipping that coordinate preserves
uncoveredness. This is the basic free-bit information rule. -/
theorem VertexUncoveredBy.flip_of_not_mem_familySupport {n : Nat}
    {Bs : Finset (Blocker n)} {v : Vertex n} {x : Fin n}
    (hun : VertexUncoveredBy Bs v) (hx : x ∉ BlockerFamily.support Bs) :
    VertexUncoveredBy Bs (flipVertex v x) := by
  rw [VertexUncoveredBy_iff_no_cover] at hun ⊢
  intro B hB hcover
  have hxB : x ∉ B.support := by
    intro hxB
    exact hx ((BlockerFamily.mem_support_iff Bs x).mpr ⟨B, hB, hxB⟩)
  exact hun B hB ((B.covers_flip_iff_of_not_mem_support hxB).mp hcover)

theorem VertexTight.energy_eq_one {n : Nat} {Bs : Finset (Blocker n)} {v : Vertex n}
    (h : VertexTight Bs v) :
    finsetVertexEnergy Bs v = 1 := by
  rcases h with ⟨B, hB, hpriv⟩
  rcases hpriv with ⟨-, hBcover, huniq⟩
  unfold finsetVertexEnergy
  have hfilter : Bs.filter (fun C => C.Covers v) = {B} := by
    ext C
    constructor
    · intro hC
      have hparts := Finset.mem_filter.mp hC
      exact Finset.mem_singleton.mpr (huniq C hparts.1 hparts.2)
    · intro hC
      have hCB : C = B := Finset.mem_singleton.mp hC
      subst C
      exact Finset.mem_filter.mpr ⟨hB, hBcover⟩
  rw [hfilter, Finset.card_singleton]

theorem VertexTight.singlyCovered {n : Nat} {Bs : Finset (Blocker n)} {v : Vertex n}
    (h : VertexTight Bs v) :
    VertexSinglyCoveredBy Bs v :=
  h.energy_eq_one

theorem VertexTight.of_singlyCovered_full {n : Nat} {Bs : Finset (Blocker n)} {v : Vertex n}
    (_hcover : FullCoverBlockers Bs) (h : VertexSinglyCoveredBy Bs v) :
    VertexTight Bs v := by
  unfold VertexSinglyCoveredBy finsetVertexEnergy at h
  have hcard : (Bs.filter fun B => B.Covers v).card = 1 := h
  obtain ⟨B, hBset⟩ := Finset.card_eq_one.mp hcard
  have hBfilter : B ∈ Bs.filter fun B => B.Covers v := by
    rw [hBset]
    simp
  have hB : B ∈ Bs := (Finset.mem_filter.mp hBfilter).1
  have hBcover : B.Covers v := (Finset.mem_filter.mp hBfilter).2
  refine ⟨B, hB, ⟨hB, hBcover, ?_⟩⟩
  intro C hC hCcover
  have hCfilter : C ∈ Bs.filter fun B => B.Covers v := Finset.mem_filter.mpr ⟨hC, hCcover⟩
  rw [hBset] at hCfilter
  exact Finset.mem_singleton.mp hCfilter

theorem VertexTight_iff_singlyCovered_of_full {n : Nat}
    {Bs : Finset (Blocker n)} (hcover : FullCoverBlockers Bs) (v : Vertex n) :
    VertexTight Bs v ↔ VertexSinglyCoveredBy Bs v := by
  constructor
  · exact VertexTight.singlyCovered
  · exact VertexTight.of_singlyCovered_full hcover

theorem VertexRedundant_iff_two_le_energy {n : Nat}
    (Bs : Finset (Blocker n)) (v : Vertex n) :
    VertexRedundant Bs v ↔ 2 ≤ finsetVertexEnergy Bs v := by
  rfl

theorem VertexExitFor.support_mem {n : Nat} {B : Blocker n} {v : Vertex n} {x : Fin n}
    (hexit : VertexExitFor B v x) :
    x ∈ B.support := by
  by_contra hx
  exact hexit.2 (B.covers_flip_of_not_mem_support hexit.1 hx)

theorem VertexExitFor.of_mem_support {n : Nat} {B : Blocker n} {v : Vertex n} {x : Fin n}
    (hcover : B.Covers v) (hx : x ∈ B.support) :
    VertexExitFor B v x :=
  ⟨hcover, B.not_covers_flip_of_mem_support hcover hx⟩

theorem VertexExitFor_iff_mem_support {n : Nat} {B : Blocker n} {v : Vertex n} {x : Fin n}
    (hcover : B.Covers v) :
    VertexExitFor B v x ↔ x ∈ B.support := by
  constructor
  · exact VertexExitFor.support_mem
  · exact VertexExitFor.of_mem_support hcover

theorem finsetVertexEnergy_eq_zero_iff_no_cover {n : Nat}
    (Bs : Finset (Blocker n)) (v : Vertex n) :
    finsetVertexEnergy Bs v = 0 ↔ ∀ B ∈ Bs, ¬ B.Covers v := by
  constructor
  · intro hzero B hB hcover
    have hmem : B ∈ Bs.filter fun C => C.Covers v := by
      exact Finset.mem_filter.mpr ⟨hB, hcover⟩
    have hpos : 0 < (Bs.filter fun C => C.Covers v).card :=
      Finset.card_pos.mpr ⟨B, hmem⟩
    unfold finsetVertexEnergy at hzero
    omega
  · intro hnone
    unfold finsetVertexEnergy
    rw [Finset.card_eq_zero]
    ext B
    rw [Finset.mem_filter]
    simp only [Finset.notMem_empty, iff_false, not_and]
    exact fun hB hcover => hnone B hB hcover

theorem FullCoverBlockers.iff_forall_energy_pos {n : Nat} (Bs : Finset (Blocker n)) :
    FullCoverBlockers Bs ↔ ∀ v : Vertex n, 0 < finsetVertexEnergy Bs v := by
  constructor
  · intro hcover v
    obtain ⟨B, hB, hBv⟩ := hcover v
    have hmem : B ∈ Bs.filter fun C => C.Covers v :=
      Finset.mem_filter.mpr ⟨hB, hBv⟩
    exact Finset.card_pos.mpr ⟨B, hmem⟩
  · intro hpos v
    by_contra hnone
    have hzero : finsetVertexEnergy Bs v = 0 := by
      rw [finsetVertexEnergy_eq_zero_iff_no_cover]
      intro B hB hBv
      exact hnone ⟨B, hB, hBv⟩
    have hvpos := hpos v
    omega

theorem PrivateBlockedVertex.energy_eq_one {n : Nat} {Bs : Finset (Blocker n)}
    {B : Blocker n} {v : Vertex n} (hpriv : PrivateBlockedVertex Bs B v) :
    finsetVertexEnergy Bs v = 1 := by
  unfold finsetVertexEnergy
  rw [Finset.card_eq_one]
  refine ⟨B, ?_⟩
  apply Finset.ext
  intro C
  constructor
  · intro hC
    rw [Finset.mem_filter] at hC
    rw [hpriv.2.2 C hC.1 hC.2]
    exact Finset.mem_singleton.mpr rfl
  · intro hC
    rw [Finset.mem_singleton] at hC
    subst C
    exact Finset.mem_filter.mpr ⟨hpriv.1, hpriv.2.1⟩

/-- In a full cover, if `B` privately owns `v`, then flipping any fixed coordinate of `B`
must hand the neighboring vertex to a different blocker. This is the local circularity rule
for hard UNSAT cores. -/
theorem PrivateBlockedVertex.exists_handoff_on_fixed_flip {n : Nat}
    {Bs : Finset (Blocker n)} {B : Blocker n} {v : Vertex n}
    (hcover : FullCoverBlockers Bs) (hpriv : PrivateBlockedVertex Bs B v)
    {x : Fin n} (hx : x ∈ B.support) :
    ∃ C ∈ Bs, C ≠ B ∧ C.Covers (flipVertex v x) := by
  obtain ⟨C, hC, hCcover⟩ := hcover (flipVertex v x)
  refine ⟨C, hC, ?_, hCcover⟩
  intro hCB
  subst C
  exact B.not_covers_flip_of_mem_support hpriv.2.1 hx hCcover

/-- The three forced handoff neighbors of a private vertex, one for each fixed coordinate. -/
def HandoffRequired {n : Nat} (Bs : Finset (Blocker n)) (B : Blocker n)
    (v : Vertex n) (x : Fin n) : Prop :=
  PrivateBlockedVertex Bs B v ∧ x ∈ B.support ∧
    ∃ C ∈ Bs, C ≠ B ∧ C.Covers (flipVertex v x)

/-- A directed forced-handoff edge between blockers: from `B` to `C` when some private
vertex of `B`, after flipping a coordinate fixed by `B`, is caught by `C`. -/
def HandoffEdge {n : Nat} (Bs : Finset (Blocker n)) (B C : Blocker n) : Prop :=
  ∃ v : Vertex n, ∃ x : Fin n,
    PrivateBlockedVertex Bs B v ∧ x ∈ B.support ∧ C ∈ Bs ∧ C ≠ B ∧
      C.Covers (flipVertex v x)

/-- Handoff edges stay inside the blocker family on both ends. -/
theorem HandoffEdge.mem_left {n : Nat} {Bs : Finset (Blocker n)} {B C : Blocker n}
    (h : HandoffEdge Bs B C) :
    B ∈ Bs := by
  rcases h with ⟨v, x, hpriv, -⟩
  exact hpriv.1

theorem HandoffEdge.mem_right {n : Nat} {Bs : Finset (Blocker n)} {B C : Blocker n}
    (h : HandoffEdge Bs B C) :
    C ∈ Bs := by
  rcases h with ⟨v, x, hpriv, hx, hC, hne, hcover⟩
  exact hC

theorem HandoffEdge.ne {n : Nat} {Bs : Finset (Blocker n)} {B C : Blocker n}
    (h : HandoffEdge Bs B C) :
    C ≠ B := by
  rcases h with ⟨v, x, hpriv, hx, hC, hne, hcover⟩
  exact hne

/-- A blocker has an outgoing handoff if one of its private exits is caught by another blocker. -/
def HasOutgoingHandoff {n : Nat} (Bs : Finset (Blocker n)) (B : Blocker n) : Prop :=
  ∃ C ∈ Bs, HandoffEdge Bs B C

/-- Minimal UNSAT cores have a forced handoff edge out of every essential blocker. -/
def HandoffNoSinks {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  ∀ B ∈ Bs, HasOutgoingHandoff Bs B

/-- One step of forced blocker motion. -/
inductive HandoffReachable {n : Nat} (Bs : Finset (Blocker n)) :
    Blocker n → Blocker n → Prop where
  | refl {B : Blocker n} (hB : B ∈ Bs) : HandoffReachable Bs B B
  | step {B C D : Blocker n} :
      HandoffEdge Bs B C →
      HandoffReachable Bs C D →
      HandoffReachable Bs B D

theorem HandoffReachable.mem_left {n : Nat} {Bs : Finset (Blocker n)}
    {B C : Blocker n} (h : HandoffReachable Bs B C) :
    B ∈ Bs := by
  induction h with
  | refl hB => exact hB
  | step hedge _ _ => exact hedge.mem_left

theorem HandoffReachable.mem_right {n : Nat} {Bs : Finset (Blocker n)}
    {B C : Blocker n} (h : HandoffReachable Bs B C) :
    C ∈ Bs := by
  induction h with
  | refl hB => exact hB
  | step _ _ ih => exact ih

/-- Handoff reachability is transitive: chains collapse to one reachability fact. -/
theorem HandoffReachable.trans {n : Nat} {Bs : Finset (Blocker n)}
    {A B C : Blocker n} (hAB : HandoffReachable Bs A B)
    (hBC : HandoffReachable Bs B C) :
    HandoffReachable Bs A C := by
  induction hAB with
  | refl _ => exact hBC
  | step hedge _ ih => exact HandoffReachable.step hedge (ih hBC)

/-- One handoff edge is a reachability chain of length one. -/
theorem HandoffEdge.reachable {n : Nat} {Bs : Finset (Blocker n)}
    {B C : Blocker n} (h : HandoffEdge Bs B C) :
    HandoffReachable Bs B C :=
  HandoffReachable.step h (HandoffReachable.refl h.mem_right)

/-- A handoff-closed subfamily contains all forced handoff successors of its blockers. -/
def HandoffClosed {n : Nat} (Bs S : Finset (Blocker n)) : Prop :=
  S ⊆ Bs ∧ ∀ B ∈ S, ∀ C, HandoffEdge Bs B C → C ∈ S

/-- If a closed subfamily contains the start of a handoff chain, it contains the end. -/
theorem HandoffClosed.mem_of_reachable {n : Nat} {Bs S : Finset (Blocker n)}
    (hclosed : HandoffClosed Bs S) {B C : Blocker n}
    (hB : B ∈ S) (hreach : HandoffReachable Bs B C) :
    C ∈ S := by
  induction hreach with
  | refl _ => exact hB
  | step hedge _ ih =>
      exact ih (hclosed.2 _ hB _ hedge)

/-- The full family is closed under its own handoff edges. -/
theorem HandoffClosed.univ_family {n : Nat} (Bs : Finset (Blocker n)) :
    HandoffClosed Bs Bs := by
  constructor
  · intro B hB
    exact hB
  · intro B hB C hedge
    exact hedge.mem_right

/-- A finite explicit handoff chain, useful for accounting path length. -/
structure HandoffTrace {n : Nat} (Bs : Finset (Blocker n)) where
  start : Blocker n
  finish : Blocker n
  nodes : List (Blocker n)
  valid : HandoffReachable Bs start finish

/-- Trace length as an accountant column. This is deliberately list-based so later we can
replace it with an executable walk. -/
def HandoffTrace.length {n : Nat} {Bs : Finset (Blocker n)}
    (τ : HandoffTrace Bs) : Nat :=
  τ.nodes.length

/-- A one-edge handoff trace. -/
def HandoffTrace.ofEdge {n : Nat} {Bs : Finset (Blocker n)}
    {B C : Blocker n} (h : HandoffEdge Bs B C) : HandoffTrace Bs where
  start := B
  finish := C
  nodes := [B, C]
  valid := h.reachable

/-- Concatenating traces corresponds to transitivity of reachability. -/
def HandoffTrace.append {n : Nat} {Bs : Finset (Blocker n)}
    (τ σ : HandoffTrace Bs) (hjoin : τ.finish = σ.start) : HandoffTrace Bs where
  start := τ.start
  finish := σ.finish
  nodes := τ.nodes ++ σ.nodes
  valid := by
    have hσ : HandoffReachable Bs τ.finish σ.finish := by
      simpa [hjoin] using σ.valid
    exact τ.valid.trans hσ

theorem HandoffTrace.append_length {n : Nat} {Bs : Finset (Blocker n)}
    (τ σ : HandoffTrace Bs) (hjoin : τ.finish = σ.start) :
    (τ.append σ hjoin).length = τ.length + σ.length := by
  simp [HandoffTrace.append, HandoffTrace.length]

/-- In a no-sink handoff graph, a nonempty closed subfamily has an internal outgoing edge. -/
theorem HandoffClosed.exists_internal_edge {n : Nat} {Bs S : Finset (Blocker n)}
    (hnosink : HandoffNoSinks Bs) (hclosed : HandoffClosed Bs S) (hS : S.Nonempty) :
    ∃ B ∈ S, ∃ C ∈ S, HandoffEdge Bs B C := by
  obtain ⟨B, hB⟩ := hS
  obtain ⟨C, hC, hedge⟩ := hnosink B (hclosed.1 hB)
  exact ⟨B, hB, C, hclosed.2 B hB C hedge, hedge⟩

/-- Forward closure under handoff reachability: the blockers reachable from `S` by any finite
chain of forced private-vertex handoffs. This is the "rule closure" operator for UNSAT cores. -/
noncomputable def HandoffForwardClosure {n : Nat}
    (Bs S : Finset (Blocker n)) : Finset (Blocker n) :=
  by
    classical
    exact Bs.filter fun C => ∃ B ∈ S, HandoffReachable Bs B C

@[simp] theorem mem_HandoffForwardClosure {n : Nat} {Bs S : Finset (Blocker n)}
    {C : Blocker n} :
    C ∈ HandoffForwardClosure Bs S ↔
      C ∈ Bs ∧ ∃ B ∈ S, HandoffReachable Bs B C := by
  classical
  simp [HandoffForwardClosure]

/-- The closure never leaves the original blocker family. -/
theorem HandoffForwardClosure.subset_family {n : Nat} (Bs S : Finset (Blocker n)) :
    HandoffForwardClosure Bs S ⊆ Bs := by
  intro C hC
  exact (mem_HandoffForwardClosure.mp hC).1

/-- Every starting blocker that lies in the family lies in its own forward closure. -/
theorem HandoffForwardClosure.mem_of_start {n : Nat} {Bs S : Finset (Blocker n)}
    {B : Blocker n} (hB : B ∈ Bs) (hS : B ∈ S) :
    B ∈ HandoffForwardClosure Bs S := by
  rw [mem_HandoffForwardClosure]
  exact ⟨hB, B, hS, HandoffReachable.refl hB⟩

/-- Forward closure is handoff-closed: once a chain can reach a blocker, it can also reach any
one-step forced successor of that blocker. -/
theorem HandoffForwardClosure.closed {n : Nat} (Bs S : Finset (Blocker n)) :
    HandoffClosed Bs (HandoffForwardClosure Bs S) := by
  constructor
  · exact HandoffForwardClosure.subset_family Bs S
  · intro C hC D hedge
    rw [mem_HandoffForwardClosure] at hC ⊢
    rcases hC with ⟨-, B, hB, hreach⟩
    exact ⟨hedge.mem_right, B, hB, hreach.trans hedge.reachable⟩

/-- Minimality of forward closure: every handoff-closed region containing the starts contains
their whole reachable closure. -/
theorem HandoffForwardClosure.subset_of_closed {n : Nat} {Bs S T : Finset (Blocker n)}
    (hclosed : HandoffClosed Bs T) (hST : S ⊆ T) :
    HandoffForwardClosure Bs S ⊆ T := by
  intro C hC
  rw [mem_HandoffForwardClosure] at hC
  rcases hC with ⟨-, B, hB, hreach⟩
  exact hclosed.mem_of_reachable (hST hB) hreach

/-- If the starting region is already closed, closing it again gives the same region. -/
theorem HandoffForwardClosure.eq_of_closed {n : Nat} {Bs S : Finset (Blocker n)}
    (hclosed : HandoffClosed Bs S) :
    HandoffForwardClosure Bs S = S := by
  apply Finset.Subset.antisymm
  · exact HandoffForwardClosure.subset_of_closed hclosed (by intro B hB; exact hB)
  · intro B hB
    exact HandoffForwardClosure.mem_of_start (hclosed.1 hB) hB

/-- A nonempty closure inside a no-sink handoff graph contains an internal edge. In words:
once a worst-case UNSAT core has no terminal blocker, any nonempty reachable envelope keeps
forcing another local handoff. -/
theorem HandoffForwardClosure.exists_internal_edge {n : Nat} {Bs S : Finset (Blocker n)}
    (hnosink : HandoffNoSinks Bs) (hS : (HandoffForwardClosure Bs S).Nonempty) :
    ∃ B ∈ HandoffForwardClosure Bs S, ∃ C ∈ HandoffForwardClosure Bs S,
      HandoffEdge Bs B C :=
  HandoffClosed.exists_internal_edge hnosink (HandoffForwardClosure.closed Bs S) hS

/-- Handoff strong connectivity: every blocker can force-reach every other blocker. This is the
formal version of a single circular UNSAT responsibility component. -/
def HandoffStronglyConnected {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  ∀ B ∈ Bs, ∀ C ∈ Bs, HandoffReachable Bs B C

/-- In a strongly connected handoff core, the forward closure of any nonempty set is the whole
core. So there are no proper nonempty handoff-closed envelopes to peel away. -/
theorem HandoffStronglyConnected.forwardClosure_eq_univ_of_nonempty {n : Nat}
    {Bs S : Finset (Blocker n)} (hconn : HandoffStronglyConnected Bs)
    (hS : S.Nonempty) (hSBs : S ⊆ Bs) :
    HandoffForwardClosure Bs S = Bs := by
  apply Finset.Subset.antisymm
  · exact HandoffForwardClosure.subset_family Bs S
  · intro C hC
    obtain ⟨B, hB⟩ := hS
    rw [mem_HandoffForwardClosure]
    exact ⟨hC, B, hB, hconn B (hSBs hB) C hC⟩

/-- Strong connectivity forbids proper nonempty closed handoff subfamilies. -/
theorem HandoffStronglyConnected.closed_eq_univ_of_nonempty {n : Nat}
    {Bs S : Finset (Blocker n)} (hconn : HandoffStronglyConnected Bs)
    (hclosed : HandoffClosed Bs S) (hS : S.Nonempty) :
    S = Bs := by
  rw [← HandoffForwardClosure.eq_of_closed hclosed]
  exact HandoffStronglyConnected.forwardClosure_eq_univ_of_nonempty hconn hS hclosed.1

/-- Equivalently, a proper closed handoff subfamily of a strongly connected core must be empty. -/
theorem HandoffStronglyConnected.closed_empty_of_proper {n : Nat}
    {Bs S : Finset (Blocker n)} (hconn : HandoffStronglyConnected Bs)
    (hclosed : HandoffClosed Bs S) (hproper : S ≠ Bs) :
    S = ∅ := by
  by_contra hne
  have hS : S.Nonempty := Finset.nonempty_iff_ne_empty.mpr hne
  exact hproper (HandoffStronglyConnected.closed_eq_univ_of_nonempty hconn hclosed hS)

/-- Choose one forced outgoing handoff from each blocker in a no-sink family. This turns the
possibly branching rule graph into one accountable deterministic walk. -/
noncomputable def handoffSuccessor {n : Nat} {Bs : Finset (Blocker n)}
    (hnosink : HandoffNoSinks Bs) (B : {B : Blocker n // B ∈ Bs}) :
    {B : Blocker n // B ∈ Bs} := by
  classical
  exact ⟨(hnosink B.val B.property).choose,
    (hnosink B.val B.property).choose_spec.1⟩

/-- The chosen successor is a genuine handoff edge. -/
theorem handoffSuccessor_edge {n : Nat} {Bs : Finset (Blocker n)}
    (hnosink : HandoffNoSinks Bs) (B : {B : Blocker n // B ∈ Bs}) :
    HandoffEdge Bs B.val (handoffSuccessor hnosink B).val := by
  classical
  exact (hnosink B.val B.property).choose_spec.2

/-- The deterministic orbit produced by repeatedly applying the chosen handoff successor. -/
noncomputable def HandoffOrbit {n : Nat} {Bs : Finset (Blocker n)}
    (hnosink : HandoffNoSinks Bs) (start : {B : Blocker n // B ∈ Bs}) (t : Nat) :
    {B : Blocker n // B ∈ Bs} :=
  (handoffSuccessor hnosink)^[t] start

/-- A no-sink finite handoff walk must repeat a blocker within `|Bs| + 1` observed states.
This is the first formal circularity accountant: terminal chains are impossible in a finite
minimal UNSAT core, so a deterministic chain eventually loops. -/
theorem HandoffNoSinks.exists_repeated_orbit_state {n : Nat} {Bs : Finset (Blocker n)}
    (hnosink : HandoffNoSinks Bs) (hBs : Bs.Nonempty) :
    ∃ start : {B : Blocker n // B ∈ Bs},
      ∃ a b : Fin (Bs.card + 1), a ≠ b ∧
        HandoffOrbit hnosink start a.val = HandoffOrbit hnosink start b.val := by
  classical
  obtain ⟨B, hB⟩ := hBs
  let start : {B : Blocker n // B ∈ Bs} := ⟨B, hB⟩
  have hcard :
      Fintype.card {B : Blocker n // B ∈ Bs} < Fintype.card (Fin (Bs.card + 1)) := by
    simp
  obtain ⟨a, b, hne, heq⟩ :=
    Fintype.exists_ne_map_eq_of_card_lt
      (fun t : Fin (Bs.card + 1) => HandoffOrbit hnosink start t.val) hcard
  exact ⟨start, a, b, hne, heq⟩

/-- A blocker is redundant in a full-cover family when erasing it still leaves a full cover. -/
def RedundantBlocker {n : Nat} (Bs : Finset (Blocker n)) (B : Blocker n) : Prop :=
  B ∈ Bs ∧ FullCoverBlockers (Bs.erase B)

/-- A blocker is essential when erasing it destroys full coverage. -/
def EssentialBlocker {n : Nat} (Bs : Finset (Blocker n)) (B : Blocker n) : Prop :=
  B ∈ Bs ∧ ¬ FullCoverBlockers (Bs.erase B)

theorem minimalFullCoverBlockers_iff_fullCover_and_all_essential {n : Nat}
    (Bs : Finset (Blocker n)) :
    MinimalFullCoverBlockers Bs ↔
      FullCoverBlockers Bs ∧ ∀ B ∈ Bs, EssentialBlocker Bs B := by
  constructor
  · rintro ⟨hcover, hmin⟩
    refine ⟨hcover, ?_⟩
    intro B hB
    exact ⟨hB, hmin B hB⟩
  · rintro ⟨hcover, hall⟩
    refine ⟨hcover, ?_⟩
    intro B hB
    exact (hall B hB).2

theorem not_redundant_of_essential {n : Nat} {Bs : Finset (Blocker n)} {B : Blocker n}
    (h : EssentialBlocker Bs B) :
    ¬ RedundantBlocker Bs B := by
  intro hred
  exact h.2 hred.2

/-- Irredundant clean UNSAT covers are exactly minimal full covers: every blocker is essential.
This is the finite object whose largest examples are our core-size worst cases. -/
def IrredundantFullCoverBlockers {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  MinimalFullCoverBlockers Bs

/-- For a fixed dimension, all irredundant/minimal clean UNSAT cores. This is finite and
therefore gives a constructive search space for "all worst cases" in that dimension. -/
noncomputable def allIrredundantFullCovers (n : Nat) : Finset (Finset (Blocker n)) :=
  by
    classical
    exact Finset.univ.filter (fun Bs : Finset (Blocker n) => IrredundantFullCoverBlockers Bs)

@[simp] theorem mem_allIrredundantFullCovers {n : Nat} {Bs : Finset (Blocker n)} :
    Bs ∈ allIrredundantFullCovers n ↔ IrredundantFullCoverBlockers Bs := by
  classical
  simp [allIrredundantFullCovers]

/-- Generic finite worst-case selector: among all irredundant UNSAT cores in dimension `n`,
keep exactly those maximizing a chosen natural-valued score. Different scores encode different
meanings of "worst": core size, local radius, proof-width lower bound, handoff tangle, etc. -/
noncomputable def allWorstIrredundantFullCoversBy (n : Nat)
    (score : Finset (Blocker n) → Nat) : Finset (Finset (Blocker n)) :=
  by
    classical
    exact (allIrredundantFullCovers n).filter
      (fun Bs => ∀ Cs ∈ allIrredundantFullCovers n, score Cs ≤ score Bs)

@[simp] theorem mem_allWorstIrredundantFullCoversBy {n : Nat}
    {score : Finset (Blocker n) → Nat} {Bs : Finset (Blocker n)} :
    Bs ∈ allWorstIrredundantFullCoversBy n score ↔
      IrredundantFullCoverBlockers Bs ∧
        ∀ Cs, IrredundantFullCoverBlockers Cs → score Cs ≤ score Bs := by
  classical
  simp [allWorstIrredundantFullCoversBy]

/-- Core-size worst cases: all maximum-size irredundant clean UNSAT cores in dimension `n`. -/
noncomputable def allMaximalMinimalUNSATCores (n : Nat) : Finset (Finset (Blocker n)) :=
  allWorstIrredundantFullCoversBy n Finset.card

@[simp] theorem mem_allMaximalMinimalUNSATCores {n : Nat} {Bs : Finset (Blocker n)} :
    Bs ∈ allMaximalMinimalUNSATCores n ↔
      MinimalFullCoverBlockers Bs ∧
        ∀ Cs : Finset (Blocker n), MinimalFullCoverBlockers Cs → Cs.card ≤ Bs.card := by
  classical
  simp [allMaximalMinimalUNSATCores, IrredundantFullCoverBlockers]

/-- A combined worst-case score made from several accountant columns. For example, columns can
be core size, local-radius lower bound, handoff edge count, proof-width lower bound, or any
other finite structural statistic we later define. -/
def combinedScore {n : Nat} (scores : List (Finset (Blocker n) → Nat))
    (Bs : Finset (Blocker n)) : Nat :=
  (scores.map fun score => score Bs).sum

@[simp] theorem combinedScore_nil {n : Nat} (Bs : Finset (Blocker n)) :
    combinedScore ([] : List (Finset (Blocker n) → Nat)) Bs = 0 := by
  simp [combinedScore]

@[simp] theorem combinedScore_cons {n : Nat} (score : Finset (Blocker n) → Nat)
    (scores : List (Finset (Blocker n) → Nat)) (Bs : Finset (Blocker n)) :
    combinedScore (score :: scores) Bs = score Bs + combinedScore scores Bs := by
  simp [combinedScore]

/-- If every score column increases from `A` to `B`, then the combined score also increases.
This is the basic accountant monotonicity rule. -/
theorem combinedScore_mono_columns {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {A B : Finset (Blocker n)}
    (h : ∀ score ∈ scores, score A ≤ score B) :
    combinedScore scores A ≤ combinedScore scores B := by
  induction scores with
  | nil => simp
  | cons score scores ih =>
      rw [combinedScore_cons, combinedScore_cons]
      exact Nat.add_le_add (h score (by simp)) (ih (by intro s hs; exact h s (by simp [hs])))

/-- If every score column is tied between `A` and `B`, then the combined score is tied. -/
theorem combinedScore_eq_of_columns_eq {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {A B : Finset (Blocker n)}
    (h : ∀ score ∈ scores, score A = score B) :
    combinedScore scores A = combinedScore scores B := by
  apply le_antisymm
  · exact combinedScore_mono_columns (by intro score hs; exact (h score hs).le)
  · exact combinedScore_mono_columns (by intro score hs; exact (h score hs).symm.le)

/-- If all score columns weakly increase and one listed column strictly increases, then the
combined score strictly increases. -/
theorem combinedScore_lt_of_column_lt {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {A B : Finset (Blocker n)}
    {score : Finset (Blocker n) → Nat}
    (hmono : ∀ score ∈ scores, score A ≤ score B) (hmem : score ∈ scores)
    (hlt : score A < score B) :
    combinedScore scores A < combinedScore scores B := by
  induction scores with
  | nil => simp at hmem
  | cons head tail ih =>
      rw [combinedScore_cons, combinedScore_cons]
      rw [List.mem_cons] at hmem
      by_cases hhead : head = score
      · subst head
        have htail : combinedScore tail A ≤ combinedScore tail B :=
          combinedScore_mono_columns (by intro s hs; exact hmono s (by simp [hs]))
        omega
      · have hmemTail : score ∈ tail := by
          rcases hmem with hsame | htail
          · exact False.elim (hhead hsame.symm)
          · exact htail
        have hheadle : head A ≤ head B := hmono head (by simp)
        have htaillt : combinedScore tail A < combinedScore tail B :=
          ih (by intro s hs; exact hmono s (by simp [hs])) hmemTail
        omega

/-- All worst irredundant UNSAT cores under a sum of several scores. This is the first
multi-axis constructor: it lets us ask for worst cases under a chosen bundle of structural
accountants rather than committing to just size. -/
noncomputable def allWorstIrredundantFullCoversBySum (n : Nat)
    (scores : List (Finset (Blocker n) → Nat)) : Finset (Finset (Blocker n)) :=
  allWorstIrredundantFullCoversBy n (combinedScore scores)

@[simp] theorem mem_allWorstIrredundantFullCoversBySum {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)} :
    Bs ∈ allWorstIrredundantFullCoversBySum n scores ↔
      IrredundantFullCoverBlockers Bs ∧
        ∀ Cs, IrredundantFullCoverBlockers Cs →
          combinedScore scores Cs ≤ combinedScore scores Bs := by
  simp [allWorstIrredundantFullCoversBySum]

/-- Any core that is worst for a combined score is one of the irredundant full covers. -/
theorem IrredundantFullCoverBlockers.of_mem_worstBySum {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)}
    (h : Bs ∈ allWorstIrredundantFullCoversBySum n scores) :
    IrredundantFullCoverBlockers Bs :=
  (mem_allWorstIrredundantFullCoversBySum.mp h).1

/-- Pareto-worst irredundant UNSAT cores: no other irredundant core is at least as good in every
chosen score and strictly better in one. This captures "worst can be a combo" without choosing
weights between incomparable structural axes. -/
noncomputable def allParetoWorstIrredundantFullCovers (n : Nat)
    (scores : List (Finset (Blocker n) → Nat)) : Finset (Finset (Blocker n)) :=
  by
    classical
    exact (allIrredundantFullCovers n).filter fun Bs =>
      ∀ Cs ∈ allIrredundantFullCovers n,
        (∀ score ∈ scores, score Bs ≤ score Cs) →
          (∀ score ∈ scores, score Cs ≤ score Bs)

@[simp] theorem mem_allParetoWorstIrredundantFullCovers {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)} :
    Bs ∈ allParetoWorstIrredundantFullCovers n scores ↔
      IrredundantFullCoverBlockers Bs ∧
        ∀ Cs, IrredundantFullCoverBlockers Cs →
          (∀ score ∈ scores, score Bs ≤ score Cs) →
            (∀ score ∈ scores, score Cs ≤ score Bs) := by
  classical
  simp [allParetoWorstIrredundantFullCovers]

/-- Pareto-worst cores are irredundant full covers. -/
theorem IrredundantFullCoverBlockers.of_mem_paretoWorst {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)}
    (h : Bs ∈ allParetoWorstIrredundantFullCovers n scores) :
    IrredundantFullCoverBlockers Bs :=
  (mem_allParetoWorstIrredundantFullCovers.mp h).1

/-- A combined-score winner is automatically Pareto-worst for the same score columns: if another
core were at least as large in every column, the combined score would be no smaller; maximality
then forces equality of the combined score, which rules out strict aggregate improvement. -/
theorem mem_paretoWorst_of_mem_worstBySum {n : Nat}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)}
    (h : Bs ∈ allWorstIrredundantFullCoversBySum n scores) :
    Bs ∈ allParetoWorstIrredundantFullCovers n scores := by
  classical
  rw [mem_allParetoWorstIrredundantFullCovers]
  rw [mem_allWorstIrredundantFullCoversBySum] at h
  constructor
  · exact h.1
  · intro Cs hCs hdom score hscore
    have hsumCB : combinedScore scores Cs ≤ combinedScore scores Bs := h.2 Cs hCs
    have hsumBC : combinedScore scores Bs ≤ combinedScore scores Cs :=
      combinedScore_mono_columns hdom
    have hsumEq : combinedScore scores Cs = combinedScore scores Bs := by omega
    by_contra hnot
    have hlt : score Bs < score Cs := Nat.lt_of_not_ge hnot
    have hsumStrict : combinedScore scores Bs < combinedScore scores Cs :=
      combinedScore_lt_of_column_lt hdom hscore hlt
    omega

/-- A constrained family of irredundant UNSAT cores. Constraints let us carve out what we mean by
"difficult": no-small-core, handoff strong connectivity, balance, high proof width, or any other
structural predicate we later add. -/
noncomputable def allConstrainedIrredundantFullCovers (n : Nat)
    (constraints : List (Finset (Blocker n) → Prop)) : Finset (Finset (Blocker n)) :=
  by
    classical
    exact (allIrredundantFullCovers n).filter
      (fun Bs => ∀ constraint ∈ constraints, constraint Bs)

@[simp] theorem mem_allConstrainedIrredundantFullCovers {n : Nat}
    {constraints : List (Finset (Blocker n) → Prop)} {Bs : Finset (Blocker n)} :
    Bs ∈ allConstrainedIrredundantFullCovers n constraints ↔
      IrredundantFullCoverBlockers Bs ∧
        ∀ constraint ∈ constraints, constraint Bs := by
  classical
  simp [allConstrainedIrredundantFullCovers]

/-- Difficult cores by weighted construction: among the cores satisfying the selected structural
constraints, keep those maximizing the combined score. -/
noncomputable def allDifficultIrredundantFullCoversBySum (n : Nat)
    (constraints : List (Finset (Blocker n) → Prop))
    (scores : List (Finset (Blocker n) → Nat)) : Finset (Finset (Blocker n)) :=
  by
    classical
    exact (allConstrainedIrredundantFullCovers n constraints).filter
      (fun Bs => ∀ Cs ∈ allConstrainedIrredundantFullCovers n constraints,
        combinedScore scores Cs ≤ combinedScore scores Bs)

@[simp] theorem mem_allDifficultIrredundantFullCoversBySum {n : Nat}
    {constraints : List (Finset (Blocker n) → Prop)}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)} :
    Bs ∈ allDifficultIrredundantFullCoversBySum n constraints scores ↔
      IrredundantFullCoverBlockers Bs ∧
        (∀ constraint ∈ constraints, constraint Bs) ∧
        ∀ Cs, IrredundantFullCoverBlockers Cs →
          (∀ constraint ∈ constraints, constraint Cs) →
            combinedScore scores Cs ≤ combinedScore scores Bs := by
  classical
  constructor
  · intro h
    rw [allDifficultIrredundantFullCoversBySum] at h
    simp only [Finset.mem_filter] at h
    rcases h with ⟨hBase, hmax⟩
    rw [mem_allConstrainedIrredundantFullCovers] at hBase
    exact ⟨hBase.1, hBase.2, by
      intro Cs hCs hConstraints
      exact hmax Cs ((mem_allConstrainedIrredundantFullCovers.mpr ⟨hCs, hConstraints⟩))⟩
  · rintro ⟨hIrr, hConstraints, hmax⟩
    rw [allDifficultIrredundantFullCoversBySum]
    simp only [Finset.mem_filter]
    exact ⟨mem_allConstrainedIrredundantFullCovers.mpr ⟨hIrr, hConstraints⟩, by
      intro Cs hCs
      rw [mem_allConstrainedIrredundantFullCovers] at hCs
      exact hmax Cs hCs.1 hCs.2⟩

/-- Difficult cores by Pareto construction: among the cores satisfying the selected structural
constraints, keep every core not dominated across the selected score columns. This is the broad
"all difficult possibilities" constructor. -/
noncomputable def allParetoDifficultIrredundantFullCovers (n : Nat)
    (constraints : List (Finset (Blocker n) → Prop))
    (scores : List (Finset (Blocker n) → Nat)) : Finset (Finset (Blocker n)) :=
  by
    classical
    exact (allConstrainedIrredundantFullCovers n constraints).filter fun Bs =>
      ∀ Cs ∈ allConstrainedIrredundantFullCovers n constraints,
        (∀ score ∈ scores, score Bs ≤ score Cs) →
          (∀ score ∈ scores, score Cs ≤ score Bs)

@[simp] theorem mem_allParetoDifficultIrredundantFullCovers {n : Nat}
    {constraints : List (Finset (Blocker n) → Prop)}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)} :
    Bs ∈ allParetoDifficultIrredundantFullCovers n constraints scores ↔
      IrredundantFullCoverBlockers Bs ∧
        (∀ constraint ∈ constraints, constraint Bs) ∧
        ∀ Cs, IrredundantFullCoverBlockers Cs →
          (∀ constraint ∈ constraints, constraint Cs) →
          (∀ score ∈ scores, score Bs ≤ score Cs) →
            (∀ score ∈ scores, score Cs ≤ score Bs) := by
  classical
  constructor
  · intro h
    rw [allParetoDifficultIrredundantFullCovers] at h
    simp only [Finset.mem_filter] at h
    rcases h with ⟨hBase, hpareto⟩
    rw [mem_allConstrainedIrredundantFullCovers] at hBase
    exact ⟨hBase.1, hBase.2, by
      intro Cs hCs hConstraints hdom
      exact hpareto Cs
        (mem_allConstrainedIrredundantFullCovers.mpr ⟨hCs, hConstraints⟩) hdom⟩
  · rintro ⟨hIrr, hConstraints, hpareto⟩
    rw [allParetoDifficultIrredundantFullCovers]
    simp only [Finset.mem_filter]
    exact ⟨mem_allConstrainedIrredundantFullCovers.mpr ⟨hIrr, hConstraints⟩, by
      intro Cs hCs hdom
      rw [mem_allConstrainedIrredundantFullCovers] at hCs
      exact hpareto Cs hCs.1 hCs.2 hdom⟩

/-- Weighted difficult cores are Pareto-difficult for the same constraints and score columns. -/
theorem mem_paretoDifficult_of_mem_difficultBySum {n : Nat}
    {constraints : List (Finset (Blocker n) → Prop)}
    {scores : List (Finset (Blocker n) → Nat)} {Bs : Finset (Blocker n)}
    (h : Bs ∈ allDifficultIrredundantFullCoversBySum n constraints scores) :
    Bs ∈ allParetoDifficultIrredundantFullCovers n constraints scores := by
  classical
  rw [mem_allDifficultIrredundantFullCoversBySum] at h
  rw [mem_allParetoDifficultIrredundantFullCovers]
  refine ⟨h.1, h.2.1, ?_⟩
  intro Cs hCs hConstraints hdom score hscore
  have hsumCB : combinedScore scores Cs ≤ combinedScore scores Bs :=
    h.2.2 Cs hCs hConstraints
  have hsumBC : combinedScore scores Bs ≤ combinedScore scores Cs :=
    combinedScore_mono_columns hdom
  have hsumEq : combinedScore scores Cs = combinedScore scores Bs := by omega
  by_contra hnot
  have hlt : score Bs < score Cs := Nat.lt_of_not_ge hnot
  have hsumStrict : combinedScore scores Bs < combinedScore scores Cs :=
    combinedScore_lt_of_column_lt hdom hscore hlt
  omega

/-- Clean-blocker local satisfiability radius: no subfamily of size at most `r` is already
UNSAT/full-covering. This is the UNSAT-only "no small core" condition. -/
def LocallySatisfiableBlockersUpTo {n : Nat} (Bs : Finset (Blocker n)) (r : Nat) : Prop :=
  ∀ S : Finset (Blocker n), S ⊆ Bs → S.card ≤ r → ¬ FullCoverBlockers S

/-- Downward monotonicity of the local satisfiability radius. -/
theorem LocallySatisfiableBlockersUpTo.mono {n : Nat} {Bs : Finset (Blocker n)}
    {r s : Nat} (h : LocallySatisfiableBlockersUpTo Bs r) (hsr : s ≤ r) :
    LocallySatisfiableBlockersUpTo Bs s := by
  intro S hS hcard
  exact h S hS (hcard.trans hsr)

/-- Every full-cover subcore has size at least `k`. -/
def UNSATCoreSizeAtLeast {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  ∀ S : Finset (Blocker n), S ⊆ Bs → FullCoverBlockers S → k ≤ S.card

/-- Local satisfiability up to `r` says every full-cover subcore has size at least `r + 1`. -/
theorem coreSizeAtLeast_succ_of_locallySatisfiable {n : Nat}
    {Bs : Finset (Blocker n)} {r : Nat}
    (hlocal : LocallySatisfiableBlockersUpTo Bs r) :
    UNSATCoreSizeAtLeast Bs (r + 1) := by
  intro S hS hcover
  by_contra hnot
  have hcard : S.card ≤ r := by omega
  exact hlocal S hS hcard hcover

/-- A minimum core-size lower bound implies local satisfiability below that bound. -/
theorem locallySatisfiable_of_coreSizeAtLeast_succ {n : Nat}
    {Bs : Finset (Blocker n)} {r : Nat}
    (hcore : UNSATCoreSizeAtLeast Bs (r + 1)) :
    LocallySatisfiableBlockersUpTo Bs r := by
  intro S hS hcard hcover
  have hmin := hcore S hS hcover
  omega

/-- A minimal full cover has no strictly smaller full-cover subfamily. -/
theorem MinimalFullCoverBlockers.no_full_subcover_of_card_lt {n : Nat}
    {Bs S : Finset (Blocker n)} (hmin : MinimalFullCoverBlockers Bs)
    (hsub : S ⊆ Bs) (hcard : S.card < Bs.card) :
    ¬ FullCoverBlockers S := by
  intro hScover
  rcases hmin with ⟨-, hminimal⟩
  have hnsub : ∃ B, B ∈ Bs ∧ B ∉ S := by
    by_contra hnone
    have hBsS : Bs ⊆ S := by
      intro B hB
      by_contra hBnotS
      exact hnone ⟨B, hB, hBnotS⟩
    have hle : Bs.card ≤ S.card := Finset.card_le_card hBsS
    omega
  obtain ⟨B, hB, hBnotS⟩ := hnsub
  have hSerase : S ⊆ Bs.erase B := by
    intro C hC
    have hCB : C ≠ B := by
      intro h
      subst C
      exact hBnotS hC
    simp [hsub hC, hCB]
  have heraseCover : FullCoverBlockers (Bs.erase B) :=
    FullCoverBlockers.mono hSerase hScover
  exact hminimal B hB heraseCover

/-- Therefore a minimal full cover is locally satisfiable up to one less than its own size. -/
theorem MinimalFullCoverBlockers.locallySatisfiable_pred {n : Nat}
    {Bs : Finset (Blocker n)} (hmin : MinimalFullCoverBlockers Bs) :
    LocallySatisfiableBlockersUpTo Bs (Bs.card - 1) := by
  intro S hS hcard hcover
  have hlt : S.card < Bs.card := by
    by_cases hzero : Bs.card = 0
    · have hSzero : S.card = 0 := by omega
      rw [Finset.card_eq_zero] at hSzero
      subst S
      rw [Finset.card_eq_zero] at hzero
      obtain ⟨B, hB, -⟩ := hmin.1 (zeroVertex n)
      rw [hzero] at hB
      simp at hB
    · omega
  exact hmin.no_full_subcover_of_card_lt hS hlt hcover

/-- A clean-blocker UNSAT candidate with no small core up to radius `r`.
This is the UNSAT-only envelope: full cover, every blocker essential, and every
subfamily of size at most `r` still satisfiable. -/
def HardUNSATBlockerEnvelope {n : Nat} (Bs : Finset (Blocker n)) (r : Nat) : Prop :=
  MinimalFullCoverBlockers Bs ∧ LocallySatisfiableBlockersUpTo Bs r

/-- Minimal covers automatically satisfy their maximal internal no-small-core envelope. -/
theorem HardUNSATBlockerEnvelope.of_minimal {n : Nat} {Bs : Finset (Blocker n)}
    (hmin : MinimalFullCoverBlockers Bs) :
    HardUNSATBlockerEnvelope Bs (Bs.card - 1) :=
  ⟨hmin, hmin.locallySatisfiable_pred⟩

/-- Minimal full covers by clean blockers have private blocked vertices. -/
theorem minimal_full_cover_blockers_private_vertex {n : Nat} (Bs : Finset (Blocker n))
    (hmin : MinimalFullCoverBlockers Bs) {B : Blocker n} (hB : B ∈ Bs) :
    ∃ v : Vertex n, PrivateBlockedVertex Bs B v := by
  rcases hmin with ⟨hcover, hminimal⟩
  have hnot : ¬ FullCoverBlockers (Bs.erase B) := hminimal B hB
  unfold FullCoverBlockers at hnot
  push Not at hnot
  rcases hnot with ⟨v, hvnot⟩
  obtain ⟨C, hC, hCv⟩ := hcover v
  have hCB : C = B := by
    by_contra hneq
    have hCerase : C ∈ Bs.erase B := by
      simp [hC, hneq]
    exact hvnot C hCerase hCv
  refine ⟨v, hB, ?_, ?_⟩
  · simpa [hCB] using hCv
  · intro D hD hDv
    by_contra hneq
    have hDerase : D ∈ Bs.erase B := by
      simp [hD, hneq]
    exact hvnot D hDerase hDv

/-- Minimal UNSAT cores have no sink blockers in the handoff graph. -/
theorem MinimalFullCoverBlockers.hasOutgoingHandoff {n : Nat}
    {Bs : Finset (Blocker n)} (hmin : MinimalFullCoverBlockers Bs)
    {B : Blocker n} (hB : B ∈ Bs) :
    HasOutgoingHandoff Bs B := by
  obtain ⟨v, hpriv⟩ := minimal_full_cover_blockers_private_vertex Bs hmin hB
  have hx : B.i ∈ B.support := by simp [Blocker.support]
  obtain ⟨C, hC, hne, hCcover⟩ :=
    hpriv.exists_handoff_on_fixed_flip hmin.1 hx
  refine ⟨C, hC, ?_⟩
  exact ⟨v, B.i, hpriv, hx, hC, hne, hCcover⟩

theorem MinimalFullCoverBlockers.handoffNoSinks {n : Nat}
    {Bs : Finset (Blocker n)} (hmin : MinimalFullCoverBlockers Bs) :
    HandoffNoSinks Bs := by
  intro B hB
  exact hmin.hasOutgoingHandoff hB

/-- Every minimal UNSAT core has a repeated deterministic handoff orbit. This is the formal
finite-circularity rule: private vertices force exits, full coverage catches those exits, and
finiteness forces some blocker to recur along a chosen handoff chain. -/
theorem MinimalFullCoverBlockers.exists_repeated_handoff_orbit {n : Nat}
    {Bs : Finset (Blocker n)} (hmin : MinimalFullCoverBlockers Bs) :
    ∃ start : {B : Blocker n // B ∈ Bs},
      ∃ a b : Fin (Bs.card + 1), a ≠ b ∧
        HandoffOrbit hmin.handoffNoSinks start a.val =
          HandoffOrbit hmin.handoffNoSinks start b.val := by
  have hnonempty : Bs.Nonempty := Finset.card_pos.mp hmin.1.card_pos
  exact HandoffNoSinks.exists_repeated_orbit_state hmin.handoffNoSinks hnonempty

/-- Choose the private vertex owned by a blocker in a minimal UNSAT core. -/
noncomputable def privateVertexOfBlocker {n : Nat} (Bs : Finset (Blocker n))
    (hmin : MinimalFullCoverBlockers Bs) (B : {B : Blocker n // B ∈ Bs}) : Vertex n :=
  (minimal_full_cover_blockers_private_vertex Bs hmin B.property).choose

theorem privateVertexOfBlocker_spec {n : Nat} (Bs : Finset (Blocker n))
    (hmin : MinimalFullCoverBlockers Bs) (B : {B : Blocker n // B ∈ Bs}) :
    PrivateBlockedVertex Bs B.val (privateVertexOfBlocker Bs hmin B) :=
  (minimal_full_cover_blockers_private_vertex Bs hmin B.property).choose_spec

/-- Distinct blockers in a minimal UNSAT core have distinct chosen private vertices. -/
theorem privateVertexOfBlocker_injective {n : Nat} (Bs : Finset (Blocker n))
    (hmin : MinimalFullCoverBlockers Bs) :
    Function.Injective (privateVertexOfBlocker Bs hmin) := by
  intro B C hvertex
  apply Subtype.ext
  have hBpriv := privateVertexOfBlocker_spec Bs hmin B
  have hCpriv := privateVertexOfBlocker_spec Bs hmin C
  have hCcoversAtB : C.val.Covers (privateVertexOfBlocker Bs hmin B) := by
    rw [hvertex]
    exact hCpriv.2.1
  exact (hBpriv.2.2 C.val C.property hCcoversAtB).symm

/-- Absolute vertex ceiling: a minimal clean UNSAT core has at most one blocker per vertex. -/
theorem MinimalFullCoverBlockers.card_le_vertices {n : Nat} {Bs : Finset (Blocker n)}
    (hmin : MinimalFullCoverBlockers Bs) :
    Bs.card ≤ 2 ^ n := by
  have hcard :=
    Fintype.card_le_of_injective (privateVertexOfBlocker Bs hmin)
      (privateVertexOfBlocker_injective Bs hmin)
  simpa [allVertices_card, Vertex] using hcard

/-- An extremal minimal UNSAT core in dimension `n`: no other minimal clean full cover has
more blockers. This is a candidate notion for "absolute worst" by core size. -/
def MaximalMinimalUNSATCore (n : Nat) (Bs : Finset (Blocker n)) : Prop :=
  MinimalFullCoverBlockers Bs ∧
  ∀ Cs : Finset (Blocker n), MinimalFullCoverBlockers Cs → Cs.card ≤ Bs.card

theorem MaximalMinimalUNSATCore.card_le_vertices {n : Nat} {Bs : Finset (Blocker n)}
    (hmax : MaximalMinimalUNSATCore n Bs) :
    Bs.card ≤ 2 ^ n :=
  hmax.1.card_le_vertices

/-- Any maximal minimal UNSAT core still has the universal nontrivial lower edge. -/
theorem MaximalMinimalUNSATCore.card_two_le {n : Nat} {Bs : Finset (Blocker n)}
    (hmax : MaximalMinimalUNSATCore n Bs) :
    2 ≤ Bs.card :=
  hmax.1.1.card_two_le

/-- Any maximal minimal UNSAT core can only exist in dimensions with clean 3-blockers. -/
theorem MaximalMinimalUNSATCore.dimension_ge_three {n : Nat} {Bs : Finset (Blocker n)}
    (hmax : MaximalMinimalUNSATCore n Bs) :
    3 ≤ n :=
  hmax.1.1.dimension_ge_three

/-- Local-radius ceiling: a hard UNSAT envelope cannot have radius reaching its own size. -/
theorem HardUNSATBlockerEnvelope.radius_lt_card {n r : Nat} {Bs : Finset (Blocker n)}
    (hhard : HardUNSATBlockerEnvelope Bs r) :
    r < Bs.card := by
  by_contra hnot
  have hcard : Bs.card ≤ r := by omega
  exact hhard.2 Bs (by intro B hB; exact hB) hcard hhard.1.1

/-- Combining private-vertex accounting with local-radius accounting bounds the radius by
the number of cube vertices. -/
theorem HardUNSATBlockerEnvelope.radius_lt_vertices {n r : Nat} {Bs : Finset (Blocker n)}
    (hhard : HardUNSATBlockerEnvelope Bs r) :
    r < 2 ^ n := by
  have hlt := hhard.radius_lt_card
  have hcard := hhard.1.card_le_vertices
  exact lt_of_lt_of_le hlt hcard

/-- Therefore no hard UNSAT core can demand a local-satisfiability radius at or above
the number of vertices. -/
theorem not_HardUNSATBlockerEnvelope_radius_ge_vertices {n r : Nat}
    {Bs : Finset (Blocker n)} (hr : 2 ^ n ≤ r) :
    ¬ HardUNSATBlockerEnvelope Bs r := by
  intro hhard
  have hlt := hhard.radius_lt_vertices
  omega

/-- The SAT decision problem in hypercube-cover form. -/
def CoverSAT (I : CoverInput) : Prop :=
  ∃ v : Vertex I.n, IsUncovered I.blockers v

def CoverUNSAT (I : CoverInput) : Prop :=
  ¬ CoverSAT I

/-- Sparse SAT rule for compact inputs: fewer than eight clean 3-blockers cannot cover the
hypercube. -/
theorem CoverInput.sat_of_blockers_length_lt_eight (I : CoverInput)
    (hlen : I.blockers.length < 8) :
    CoverSAT I :=
  coverSATList_of_length_lt_eight hlen

/-- Satisfiability is decidable: the vertex set is finite and `IsUncovered` is decidable. -/
instance (I : CoverInput) : Decidable (CoverSAT I) := by
  unfold CoverSAT; infer_instance

/-- Compact input size (variables + blockers). -/
def CoverInput.size (I : CoverInput) : Nat :=
  I.n + I.blockers.length

/-! ### Difficulty classes for blocker hypercubes

These predicates name the main geometric axes used later. They are intentionally modest:
they classify the kind of structure present in a blocker system without asserting that any
one axis is the unique notion of algorithmic hardness. -/

/-- Search-easy SAT evidence: at least `k` vertices remain uncovered. -/
def HasAtLeastHoles {n : Nat} (Bs : List (Blocker n)) (k : Nat) : Prop :=
  k ≤ solutionCount Bs

/-- Search-critical SAT evidence: exactly one hypercube vertex remains uncovered. -/
def HasUniqueHole {n : Nat} (Bs : List (Blocker n)) : Prop :=
  UniqueSATList Bs

/-- Refutation side: the blockers cover the whole hypercube. -/
def HasNoHole {n : Nat} (Bs : List (Blocker n)) : Prop :=
  CoverUNSATList Bs

/-- A full cover whose contradiction is invisible to all subfamilies of size at most `r`. -/
def LocallyInvisibleFullCover {n : Nat} (Bs : Finset (Blocker n)) (r : Nat) : Prop :=
  FullCoverBlockers Bs ∧ LocallySatisfiableBlockersUpTo Bs r

/-- A minimal full cover is locally invisible up to one less than its own size. -/
theorem locallyInvisibleFullCover_of_minimal {n : Nat} {Bs : Finset (Blocker n)}
    (hmin : MinimalFullCoverBlockers Bs) :
    LocallyInvisibleFullCover Bs (Bs.card - 1) :=
  ⟨hmin.1, hmin.locallySatisfiable_pred⟩

/-- No blocker crosses the coordinate cut `U`: each blocker lives entirely inside `U` or
entirely outside it. This is a hypercube version of a decomposition separator. -/
def BlockerFamily.SeparatedBy {n : Nat} (Bs : Finset (Blocker n)) (U : Finset (Fin n)) :
    Prop :=
  ∀ B ∈ Bs, B.support ⊆ U ∨ Disjoint B.support U

/-- The empty coordinate cut separates a family exactly when every blocker is outside it. -/
theorem BlockerFamily.separatedBy_empty {n : Nat} (Bs : Finset (Blocker n)) :
    BlockerFamily.SeparatedBy Bs ∅ := by
  intro B hB
  right
  simp [Disjoint]

/-- The full coordinate cut trivially separates every family. -/
theorem BlockerFamily.separatedBy_univ {n : Nat} (Bs : Finset (Blocker n)) :
    BlockerFamily.SeparatedBy Bs Finset.univ := by
  intro B hB
  left
  intro i hi
  simp

/-- Two coordinates interact when some blocker mentions both. This is the primal-graph
shadow of the hypercube cover. -/
def BlockerFamily.VariableInteraction {n : Nat} (Bs : Finset (Blocker n))
    (i j : Fin n) : Prop :=
  i ≠ j ∧ ∃ B ∈ Bs, i ∈ B.support ∧ j ∈ B.support

/-- Variable interaction is symmetric. -/
theorem BlockerFamily.VariableInteraction.symm {n : Nat} {Bs : Finset (Blocker n)}
    {i j : Fin n} (h : BlockerFamily.VariableInteraction Bs i j) :
    BlockerFamily.VariableInteraction Bs j i := by
  rcases h with ⟨hij, B, hB, hi, hj⟩
  exact ⟨hij.symm, B, hB, hj, hi⟩

/-- A coordinate separator has no blocker crossing it. This is the decomposition hook used
by bounded-treewidth and separator-style reasoning. -/
def HasCoordinateSeparator {n : Nat} (Bs : Finset (Blocker n)) (U : Finset (Fin n)) : Prop :=
  BlockerFamily.SeparatedBy Bs U

/-- A small coordinate separator is an explicit bounded-size decomposition witness. -/
def HasCoordinateSeparatorAtMost {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  ∃ U : Finset (Fin n), U.card ≤ k ∧ HasCoordinateSeparator Bs U

/-- If a family is separated by `U`, then no interaction edge crosses from `U` to its
complement. -/
theorem not_variableInteraction_cross_of_separatedBy {n : Nat}
    {Bs : Finset (Blocker n)} {U : Finset (Fin n)} (hsep : BlockerFamily.SeparatedBy Bs U)
    {i j : Fin n} (hi : i ∈ U) (hj : j ∉ U) :
    ¬ BlockerFamily.VariableInteraction Bs i j := by
  rintro ⟨-, B, hB, hiB, hjB⟩
  rcases hsep B hB with hinside | houtside
  · exact hj (hinside hjB)
  · exact (Finset.disjoint_left.mp houtside) hiB hi

/-- A coordinate cut separates the blocker family exactly when no primal-graph interaction edge
crosses the cut. This is the bridge from blocker geometry to graph-separator reasoning. -/
theorem separatedBy_iff_no_variableInteraction_cross {n : Nat}
    (Bs : Finset (Blocker n)) (U : Finset (Fin n)) :
    BlockerFamily.SeparatedBy Bs U ↔
      ∀ i j : Fin n, i ∈ U → j ∉ U → ¬ BlockerFamily.VariableInteraction Bs i j := by
  constructor
  · intro hsep i j hi hj
    exact not_variableInteraction_cross_of_separatedBy hsep hi hj
  · intro hno B hB
    by_cases hsub : B.support ⊆ U
    · exact Or.inl hsub
    · right
      rw [Finset.disjoint_left]
      intro x hxB hxU
      apply hsub
      intro y hyB
      by_contra hyU
      have hxy : x ≠ y := by
        intro h
        subst y
        exact hyU hxU
      exact (hno x y hxU hyU) ⟨hxy, B, hB, hxB, hyB⟩

/-- Generic backdoor schema: a small coordinate set `U` is a backdoor when every slice
assigned on `U` lands in a chosen easy class. The residual operation is left abstract so
we can instantiate it later for Horn, 2-SAT, affine, bounded-width, or our own rules. -/
def HasBackdoorTo {n : Nat} {Residual : Type} (Easy : Residual → Prop)
    (slice : (Fin n → Bool) → Residual) (k : Nat) : Prop :=
  ∃ U : Finset (Fin n), U.card ≤ k ∧ ∀ assignment : Fin n → Bool, Easy (slice assignment)

/-- A slice system is the formal object behind backdoor reasoning: choose a coordinate set,
assign the cube, and inspect the residual instance in some target representation. -/
structure SliceSystem (n : Nat) (Residual : Type) where
  slice : Finset (Fin n) → (Fin n → Bool) → Residual

/-- Strong backdoor schema: one small coordinate set makes every slice land in an easy
residual class. -/
def HasStrongBackdoorTo {n : Nat} {Residual : Type} (Easy : Residual → Prop)
    (S : SliceSystem n Residual) (k : Nat) : Prop :=
  ∃ U : Finset (Fin n), U.card ≤ k ∧ ∀ assignment : Fin n → Bool, Easy (S.slice U assignment)

/-- Named buckets for the main serious SAT views we want to track. -/
inductive SATViewKind where
  | fasterExponential
  | tractableSubclass
  | structuralDecomposition
  | backdoor
  | proofComplexity
  | randomPhaseTransition
  | algebraicEncoding
  | circuitComplexity
  | solverTrace
  | proofBarrier
  deriving DecidableEq, Repr

/-- A project view packages a SAT perspective as a predicate on compact cover inputs. -/
structure SATView where
  kind : SATViewKind
  holds : CoverInput → Prop

/-- A random/parametric instance ensemble, kept abstract so probability can be layered on
later without changing the geometric vocabulary. -/
structure CoverEnsemble where
  Sample : Nat → Type
  instanceOf : {n : Nat} → Sample n → CoverInput
  density : {n : Nat} → Sample n → Nat

/-- A phase window says that samples whose density lies between two curves satisfy a target
property. Later the target can be "typically hard", "usually SAT", or "usually UNSAT". -/
def CoverEnsemble.PhaseWindow (E : CoverEnsemble) (lo hi : Nat → Nat)
    (Target : CoverInput → Prop) : Prop :=
  ∀ n : Nat, ∀ s : E.Sample n,
    lo n ≤ E.density s → E.density s ≤ hi n → Target (E.instanceOf s)

/-- Algebraic encodings of cover instances: an object, a certificate relation, and a soundness
bridge for UNSAT certificates. This captures polynomial-calculus/Nullstellensatz-style views
without choosing a particular polynomial language yet. -/
structure AlgebraicEncoding (Object Certificate : Type) where
  encode : CoverInput → Object
  certifiesUNSAT : Certificate → Object → Prop
  sound_unsat : ∀ I cert, certifiesUNSAT cert (encode I) → CoverUNSAT I

/-- A generic circuit-family interface for the decision function on compact inputs. -/
structure CircuitFamily (Input : Type) where
  eval : Nat → Input → Bool
  gateCount : Nat → Nat

/-- A circuit family decides a language when the circuit indexed by input size is correct. -/
def CircuitFamily.Decides {Input : Type} (C : CircuitFamily Input) (size : Input → Nat)
    (L : Input → Prop) : Prop :=
  ∀ x : Input, C.eval (size x) x = true ↔ L x

/-- Abstract profile for known proof-strategy barriers. A serious route should eventually
explain which boxes it avoids rather than merely asserting a strong conclusion. -/
structure BarrierProfile where
  relativizing : Prop
  naturalizing : Prop
  algebrizing : Prop

/-- The clean target profile: none of the standard broad barriers apply to the strategy. -/
def BarrierProfile.AvoidsKnownBarriers (B : BarrierProfile) : Prop :=
  ¬ B.relativizing ∧ ¬ B.naturalizing ∧ ¬ B.algebrizing

/-- Solver actions as high-level moves in hypercube language. These are deliberately
implementation-neutral: concrete DPLL/CDCL traces can refine them later. -/
inductive SolverActionKind where
  | branch
  | propagate
  | learn
  | restart
  | checkVertex
  | derivePattern
  deriving DecidableEq, Repr

/-- A solver trace records a compact input, a sequence of high-level actions, and the final
Boolean answer. -/
structure SolverTrace where
  input : CoverInput
  actions : List SolverActionKind
  result : Bool

/-- Sound SAT traces may only answer true when a hole really exists. -/
def SolverTrace.SoundSAT (τ : SolverTrace) : Prop :=
  τ.result = true → CoverSAT τ.input

/-- Sound UNSAT traces may only answer false when the cover is complete. -/
def SolverTrace.SoundUNSAT (τ : SolverTrace) : Prop :=
  τ.result = false → CoverUNSAT τ.input

/-- A geometric hard-UNSAT candidate combines full cover, minimality, local invisibility,
and no-sink handoff dynamics. -/
def GeometricHardUNSATCandidate {n : Nat} (Bs : Finset (Blocker n)) (r : Nat) : Prop :=
  MinimalFullCoverBlockers Bs ∧ LocallySatisfiableBlockersUpTo Bs r ∧ HandoffNoSinks Bs

/-- Minimality plus a local radius automatically gives the no-sink handoff part. -/
theorem geometricHardUNSATCandidate_of_minimal_local {n r : Nat}
    {Bs : Finset (Blocker n)} (hmin : MinimalFullCoverBlockers Bs)
    (hlocal : LocallySatisfiableBlockersUpTo Bs r) :
    GeometricHardUNSATCandidate Bs r :=
  ⟨hmin, hlocal, hmin.handoffNoSinks⟩

/-! ### Finite structural classification and counting -/

/-- Finset-level uncovered vertices for a blocker family. These are the holes in the
hypercube cover. -/
noncomputable def finsetUncoveredVertices {n : Nat}
    (Bs : Finset (Blocker n)) : Finset (Vertex n) := by
  classical
  exact (allVertices n).filter fun v => VertexUncoveredBy Bs v

@[simp] theorem mem_finsetUncoveredVertices {n : Nat} {Bs : Finset (Blocker n)}
    {v : Vertex n} :
    v ∈ finsetUncoveredVertices Bs ↔ VertexUncoveredBy Bs v := by
  classical
  simp [finsetUncoveredVertices]

/-- Finset-level solution count. -/
noncomputable def finsetSolutionCount {n : Nat} (Bs : Finset (Blocker n)) : Nat :=
  (finsetUncoveredVertices Bs).card

/-- Adding blockers can only remove uncovered vertices. -/
theorem finsetUncoveredVertices_antitone {n : Nat}
    {Bs Cs : Finset (Blocker n)} (hsub : Bs ⊆ Cs) :
    finsetUncoveredVertices Cs ⊆ finsetUncoveredVertices Bs := by
  intro v hv
  rw [mem_finsetUncoveredVertices] at hv ⊢
  rw [VertexUncoveredBy_iff_no_cover] at hv ⊢
  intro B hB
  exact hv B (hsub hB)

/-- Adding blockers can only decrease the number of holes. -/
theorem finsetSolutionCount_antitone {n : Nat}
    {Bs Cs : Finset (Blocker n)} (hsub : Bs ⊆ Cs) :
    finsetSolutionCount Cs ≤ finsetSolutionCount Bs := by
  unfold finsetSolutionCount
  exact Finset.card_le_card (finsetUncoveredVertices_antitone hsub)

theorem finsetSolutionCount_le_vertexCount {n : Nat} (Bs : Finset (Blocker n)) :
    finsetSolutionCount Bs ≤ 2 ^ n := by
  classical
  unfold finsetSolutionCount finsetUncoveredVertices
  simpa [allVertices_card] using
    Finset.card_filter_le (allVertices n) (fun v => VertexUncoveredBy Bs v)

theorem coveredVertices_eq_sdiff_uncoveredVertices {n : Nat} (Bs : Finset (Blocker n)) :
    coveredVertices Bs = allVertices n \ finsetUncoveredVertices Bs := by
  classical
  ext v
  rw [mem_coveredVertices]
  simp only [Finset.mem_sdiff, mem_allVertices, true_and]
  rw [mem_finsetUncoveredVertices, VertexUncoveredBy_iff_no_cover]
  constructor
  · intro hcover hun
    rcases hcover with ⟨B, hB, hBcover⟩
    exact hun B hB hBcover
  · intro hnot
    push Not at hnot
    exact hnot

theorem coveredVertices_card_add_finsetSolutionCount {n : Nat} (Bs : Finset (Blocker n)) :
    (coveredVertices Bs).card + finsetSolutionCount Bs = 2 ^ n := by
  classical
  have hsub : finsetUncoveredVertices Bs ⊆ allVertices n := by
    intro v hv
    exact mem_allVertices v
  unfold finsetSolutionCount
  rw [coveredVertices_eq_sdiff_uncoveredVertices Bs]
  simpa [allVertices_card] using Finset.card_sdiff_add_card_eq_card hsub

/-- Number of coordinates actually mentioned by a finite blocker family. This is the variable
footprint of the instance, and unused coordinates contribute free hypercube bits. -/
def familySupportSize {n : Nat} (Bs : Finset (Blocker n)) : Nat :=
  (BlockerFamily.support Bs).card

theorem familySupportSize_le_dimension {n : Nat} (Bs : Finset (Blocker n)) :
    familySupportSize Bs ≤ n := by
  unfold familySupportSize
  simpa [Fintype.card_fin] using
    Finset.card_le_univ (BlockerFamily.support Bs)

/-- Number of unused coordinates. These are product directions not constrained by any blocker. -/
def unusedVariableCount {n : Nat} (Bs : Finset (Blocker n)) : Nat :=
  n - familySupportSize Bs

/-- Families mentioning at most `k` coordinates. -/
def FinsetUsesAtMostVariables {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  familySupportSize Bs ≤ k

/-- Families mentioning at least `k` coordinates. -/
def FinsetUsesAtLeastVariables {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  k ≤ familySupportSize Bs

/-- Families with exact coordinate footprint `k`. -/
def FinsetUsesExactlyVariables {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  familySupportSize Bs = k

/-- SAT-side bucket: at least `k` holes remain. -/
def FinsetHasAtLeastHoles {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  k ≤ finsetSolutionCount Bs

/-- Exact hole-count bucket. This is the solution-count spectrum of the hypercube cover. -/
def FinsetHasExactlyHoles {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  finsetSolutionCount Bs = k

/-- Critical SAT-side bucket: exactly one hole remains. -/
def FinsetHasUniqueHole {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  finsetSolutionCount Bs = 1

/-- Full cover is exactly zero uncovered vertices. -/
theorem fullCoverBlockers_iff_finsetSolutionCount_eq_zero {n : Nat}
    (Bs : Finset (Blocker n)) :
    FullCoverBlockers Bs ↔ finsetSolutionCount Bs = 0 := by
  constructor
  · intro hcover
    unfold finsetSolutionCount
    rw [Finset.card_eq_zero]
    ext v
    constructor
    · intro hv
      have hun : VertexUncoveredBy Bs v := mem_finsetUncoveredVertices.mp hv
      obtain ⟨B, hB, hcoverv⟩ := hcover v
      exact False.elim ((VertexUncoveredBy_iff_no_cover Bs v).mp hun B hB hcoverv)
    · intro hv
      simp at hv
  · intro hzero v
    unfold finsetSolutionCount at hzero
    rw [Finset.card_eq_zero] at hzero
    by_contra hnot
    have hun : VertexUncoveredBy Bs v := by
      rw [VertexUncoveredBy_iff_no_cover]
      intro B hB hcover
      exact hnot ⟨B, hB, hcover⟩
    have hv : v ∈ finsetUncoveredVertices Bs := mem_finsetUncoveredVertices.mpr hun
    rw [hzero] at hv
    simp at hv

/-- SAT/UNSAT pointwise complement: not being a full cover is exactly having at least one
uncovered vertex. -/
theorem not_fullCoverBlockers_iff_hasAtLeastOneHole {n : Nat}
    (Bs : Finset (Blocker n)) :
    ¬ FullCoverBlockers Bs ↔ FinsetHasAtLeastHoles Bs 1 := by
  rw [fullCoverBlockers_iff_finsetSolutionCount_eq_zero]
  unfold FinsetHasAtLeastHoles
  omega

/-- Sparse SAT-side density rule: fewer than eight clean 3-blockers always leave at least one
uncovered vertex. -/
theorem finsetHasAtLeastOneHole_of_card_lt_eight {n : Nat}
    {Bs : Finset (Blocker n)} (hcard : Bs.card < 8) :
    FinsetHasAtLeastHoles Bs 1 := by
  have hnot : ¬ FullCoverBlockers Bs := not_fullCoverBlockers_of_card_lt_eight hcard
  unfold FinsetHasAtLeastHoles
  by_contra hzeroish
  have hzero : finsetSolutionCount Bs = 0 := by omega
  exact hnot ((fullCoverBlockers_iff_finsetSolutionCount_eq_zero Bs).mpr hzero)

/-- Unique-hole SAT instances are also dense: in dimension at least three, covering all but
one vertex requires at least seven clean 3-blockers. -/
theorem FinsetHasUniqueHole.card_ge_seven {n : Nat} {Bs : Finset (Blocker n)}
    (hn : 3 ≤ n) (hunique : FinsetHasUniqueHole Bs) :
    7 ≤ Bs.card := by
  let cap := 2 ^ (n - 3)
  have hcapPos : 0 < cap := Nat.two_pow_pos (n - 3)
  have hpow : 2 ^ n = 8 * cap := by
    have hn' : n = (n - 3) + 3 := by omega
    have hrewrite : 2 ^ n = 2 ^ ((n - 3) + 3) := congrArg (fun t => 2 ^ t) hn'
    calc
      2 ^ n = 2 ^ ((n - 3) + 3) := hrewrite
      _ = 2 ^ (n - 3) * 2 ^ 3 := by rw [Nat.pow_add]
      _ = 8 * cap := by norm_num [cap, Nat.mul_comm]
  have hsum := coveredVertices_card_add_finsetSolutionCount Bs
  have hsumOne : (coveredVertices Bs).card + 1 = 2 ^ n := by
    unfold FinsetHasUniqueHole at hunique
    simpa [hunique] using hsum
  have hcovered : (coveredVertices Bs).card ≤ Bs.card * cap := by
    simpa [cap] using coveredVertices_card_le_card_mul_capacity Bs
  have hle : 8 * cap ≤ Bs.card * cap + 1 := by
    calc
      8 * cap = 2 ^ n := hpow.symm
      _ = (coveredVertices Bs).card + 1 := hsumOne.symm
      _ ≤ Bs.card * cap + 1 := Nat.add_le_add_right hcovered 1
  by_contra hnot
  have hsmall : Bs.card ≤ 6 := by omega
  have hsmallMul : Bs.card * cap + 1 ≤ 6 * cap + 1 := by
    exact Nat.add_le_add_right (Nat.mul_le_mul_right cap hsmall) 1
  have hbad : 8 * cap ≤ 6 * cap + 1 := hle.trans hsmallMul
  omega

/-- One uncovered vertex plus one unused coordinate yields at least two uncovered vertices:
the original hole and its free-coordinate flip. -/
theorem finsetSolutionCount_ge_two_of_uncovered_and_unused_coord {n : Nat}
    {Bs : Finset (Blocker n)} {v : Vertex n} {x : Fin n}
    (hun : VertexUncoveredBy Bs v) (hx : x ∉ BlockerFamily.support Bs) :
    2 ≤ finsetSolutionCount Bs := by
  have hflip : VertexUncoveredBy Bs (flipVertex v x) :=
    hun.flip_of_not_mem_familySupport hx
  have hne : v ≠ flipVertex v x := by
    intro h
    have hxcoord := congrFun h x
    simp at hxcoord
  have hsub :
      ({v, flipVertex v x} : Finset (Vertex n)) ⊆ finsetUncoveredVertices Bs := by
    intro w hw
    rw [Finset.mem_insert] at hw
    rcases hw with rfl | hw
    · exact mem_finsetUncoveredVertices.mpr hun
    · rw [Finset.mem_singleton] at hw
      rw [hw]
      exact mem_finsetUncoveredVertices.mpr hflip
  have hcard := Finset.card_le_card hsub
  unfold finsetSolutionCount
  simpa [hne] using hcard

/-- Unique-hole SAT instances must use every coordinate. Otherwise the hole can be flipped along
an unused coordinate, producing a second hole. -/
theorem usesAllVariables_of_finsetHasUniqueHole {n : Nat} {Bs : Finset (Blocker n)}
    (hunique : FinsetHasUniqueHole Bs) :
    UsesAllVariables Bs := by
  apply Finset.eq_univ_iff_forall.mpr
  intro x
  by_contra hx
  have hposCount : 0 < finsetSolutionCount Bs := by
    unfold FinsetHasUniqueHole at hunique
    omega
  unfold finsetSolutionCount at hposCount
  have hpos : 0 < (finsetUncoveredVertices Bs).card := hposCount
  obtain ⟨v, hv⟩ := Finset.card_pos.mp hpos
  have hun : VertexUncoveredBy Bs v := mem_finsetUncoveredVertices.mp hv
  have htwo := finsetSolutionCount_ge_two_of_uncovered_and_unused_coord
    (Bs := Bs) (v := v) (x := x) hun hx
  unfold FinsetHasUniqueHole at hunique
  omega

theorem unusedVariableCount_eq_zero_of_usesAllVariables {n : Nat}
    {Bs : Finset (Blocker n)} (hall : UsesAllVariables Bs) :
    unusedVariableCount Bs = 0 := by
  unfold unusedVariableCount familySupportSize
  rw [hall]
  simp

theorem usesAllVariables_of_unusedVariableCount_eq_zero {n : Nat}
    {Bs : Finset (Blocker n)} (hzero : unusedVariableCount Bs = 0) :
    UsesAllVariables Bs := by
  unfold UsesAllVariables
  apply Finset.eq_of_subset_of_card_le
  · intro x _hx
    simp
  · have hle : familySupportSize Bs ≤ n := familySupportSize_le_dimension Bs
    have hnle : n ≤ familySupportSize Bs := by
      unfold unusedVariableCount at hzero
      omega
    simpa [familySupportSize, Fintype.card_fin] using hnle

theorem unusedVariableCount_eq_zero_iff_usesAllVariables {n : Nat}
    (Bs : Finset (Blocker n)) :
    unusedVariableCount Bs = 0 ↔ UsesAllVariables Bs := by
  constructor
  · exact usesAllVariables_of_unusedVariableCount_eq_zero
  · exact unusedVariableCount_eq_zero_of_usesAllVariables

/-- Unique-hole instances have no unused product directions. -/
theorem unusedVariableCount_eq_zero_of_finsetHasUniqueHole {n : Nat}
    {Bs : Finset (Blocker n)} (hunique : FinsetHasUniqueHole Bs) :
    unusedVariableCount Bs = 0 :=
  unusedVariableCount_eq_zero_of_usesAllVariables
    (usesAllVariables_of_finsetHasUniqueHole hunique)

/-- A nontrivial coordinate cut is neither empty nor the whole coordinate set. -/
def NontrivialCoordinateCut {n : Nat} (U : Finset (Fin n)) : Prop :=
  U.Nonempty ∧ U.card < n

/-- A nontrivial small separator: a genuine decomposition witness rather than the empty/full
trivial cuts. -/
def HasNontrivialCoordinateSeparatorAtMost {n : Nat}
    (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  ∃ U : Finset (Fin n),
    U.card ≤ k ∧ NontrivialCoordinateCut U ∧ HasCoordinateSeparator Bs U

/-- Entanglement bucket: no nontrivial separator of size at most `k`. -/
def NoNontrivialCoordinateSeparatorAtMost {n : Nat}
    (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  ¬ HasNontrivialCoordinateSeparatorAtMost Bs k

/-- Finite universe of blocker families in dimension `n`. -/
noncomputable def allBlockerFamilies (n : Nat) : Finset (Finset (Blocker n)) := by
  classical
  exact Finset.univ

/-- Finite universe of blocker families using at most `m` blockers. -/
noncomputable def allBlockerFamiliesAtMost (n m : Nat) :
    Finset (Finset (Blocker n)) := by
  classical
  exact (allBlockerFamilies n).filter fun Bs => Bs.card ≤ m

@[simp] theorem mem_allBlockerFamiliesAtMost {n m : Nat} {Bs : Finset (Blocker n)} :
    Bs ∈ allBlockerFamiliesAtMost n m ↔ Bs.card ≤ m := by
  classical
  simp [allBlockerFamiliesAtMost, allBlockerFamilies]

/-- A structural class is any predicate on finite blocker families. -/
abbrev BlockerFamilyClass (n : Nat) := Finset (Blocker n) → Prop

/-- Families with exactly `k` blockers. This is the blocker-budget shell/ring. -/
def FinsetHasExactlyBlockers {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  Bs.card = k

/-- Count families in a bounded finite search space satisfying a structural class. -/
noncomputable def classCount (n m : Nat) (Class : BlockerFamilyClass n) : Nat := by
  classical
  exact ((allBlockerFamiliesAtMost n m).filter Class).card

/-- The total number of blocker families under a blocker budget. -/
noncomputable def totalFamilyCountAtMost (n m : Nat) : Nat :=
  (allBlockerFamiliesAtMost n m).card

theorem classCount_le_total (n m : Nat) (Class : BlockerFamilyClass n) :
    classCount n m Class ≤ totalFamilyCountAtMost n m := by
  classical
  unfold classCount totalFamilyCountAtMost
  exact Finset.card_filter_le _ _

/-- If class `A` implies class `B`, then its bounded count is no larger. -/
theorem classCount_mono {n m : Nat} {A B : BlockerFamilyClass n}
    (hAB : ∀ Bs, A Bs → B Bs) :
    classCount n m A ≤ classCount n m B := by
  classical
  unfold classCount
  apply Finset.card_le_card
  intro Bs hBs
  rw [Finset.mem_filter] at hBs ⊢
  exact ⟨hBs.1, hAB Bs hBs.2⟩

/-- Enlarging the blocker budget can only enlarge a class count. -/
theorem classCount_budget_mono {n m l : Nat} (hml : m ≤ l)
    (Class : BlockerFamilyClass n) :
    classCount n m Class ≤ classCount n l Class := by
  classical
  unfold classCount
  apply Finset.card_le_card
  intro Bs hBs
  rw [Finset.mem_filter] at hBs ⊢
  rw [mem_allBlockerFamiliesAtMost] at hBs ⊢
  exact ⟨hBs.1.trans hml, hBs.2⟩

theorem totalFamilyCountAtMost_mono {n m l : Nat} (hml : m ≤ l) :
    totalFamilyCountAtMost n m ≤ totalFamilyCountAtMost n l := by
  unfold totalFamilyCountAtMost
  apply Finset.card_le_card
  intro Bs hBs
  rw [mem_allBlockerFamiliesAtMost] at hBs ⊢
  exact hBs.trans hml

/-- Count of bounded families with at least `k` holes. -/
noncomputable def manyHoleFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => FinsetHasAtLeastHoles Bs k

/-- Count of bounded families with exactly `k` blockers. -/
noncomputable def exactBlockerFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => FinsetHasExactlyBlockers Bs k

/-- Count of bounded families with exactly `k` holes. -/
noncomputable def exactHoleFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => FinsetHasExactlyHoles Bs k

/-- Count of bounded families with a unique hole. -/
noncomputable def uniqueHoleFamilyCount (n m : Nat) : Nat :=
  classCount n m fun Bs => FinsetHasUniqueHole Bs

/-- Count of bounded families whose variable footprint is at most `k`. -/
noncomputable def supportAtMostFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => FinsetUsesAtMostVariables Bs k

/-- Count of bounded families whose variable footprint is at least `k`. -/
noncomputable def supportAtLeastFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => FinsetUsesAtLeastVariables Bs k

/-- Count of bounded families whose variable footprint is exactly `k`. -/
noncomputable def supportExactFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => FinsetUsesExactlyVariables Bs k

/-- Count of bounded full-cover/UNSAT families. -/
noncomputable def fullCoverFamilyCount (n m : Nat) : Nat :=
  classCount n m fun Bs => FullCoverBlockers Bs

/-- Count of bounded locally invisible full covers. -/
noncomputable def locallyInvisibleFamilyCount (n m r : Nat) : Nat :=
  classCount n m fun Bs => LocallyInvisibleFullCover Bs r

/-- Count of bounded families admitting a nontrivial coordinate separator of size at most `k`. -/
noncomputable def decomposableFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => HasNontrivialCoordinateSeparatorAtMost Bs k

/-- Count of bounded families satisfying the current geometric hard-UNSAT candidate envelope. -/
noncomputable def geometricHardUNSATCandidateCount (n m r : Nat) : Nat :=
  classCount n m fun Bs => GeometricHardUNSATCandidate Bs r

/-- Unique-hole families are included among families with at least one hole. -/
theorem uniqueHole_count_le_sat_count (n m : Nat) :
    uniqueHoleFamilyCount n m ≤ manyHoleFamilyCount n m 1 := by
  unfold uniqueHoleFamilyCount manyHoleFamilyCount
  apply classCount_mono
  intro Bs hunique
  unfold FinsetHasUniqueHole at hunique
  unfold FinsetHasAtLeastHoles
  omega

theorem manyHoleFamilyCount_zero_eq_total (n m : Nat) :
    manyHoleFamilyCount n m 0 = totalFamilyCountAtMost n m := by
  classical
  unfold manyHoleFamilyCount classCount totalFamilyCountAtMost
  have hset :
      (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FinsetHasAtLeastHoles Bs 0) =
        allBlockerFamiliesAtMost n m := by
    ext Bs
    constructor
    · intro hmem
      rw [Finset.mem_filter] at hmem
      exact hmem.1
    · intro hmem
      rw [Finset.mem_filter]
      exact ⟨hmem, Nat.zero_le _⟩
  rw [hset]

theorem manyHoleFamilyCount_antitone {n m k l : Nat} (hkl : k ≤ l) :
    manyHoleFamilyCount n m l ≤ manyHoleFamilyCount n m k := by
  unfold manyHoleFamilyCount
  apply classCount_mono
  intro Bs h
  unfold FinsetHasAtLeastHoles at h ⊢
  omega

theorem manyHoleFamilyCount_budget_mono {n m l k : Nat} (hml : m ≤ l) :
    manyHoleFamilyCount n m k ≤ manyHoleFamilyCount n l k := by
  unfold manyHoleFamilyCount
  exact classCount_budget_mono hml _

theorem exactBlockerFamilyCount_budget_mono {n m l k : Nat} (hml : m ≤ l) :
    exactBlockerFamilyCount n m k ≤ exactBlockerFamilyCount n l k := by
  unfold exactBlockerFamilyCount
  exact classCount_budget_mono hml _

theorem exactBlockerFamilyCount_eq_zero_of_budget_lt {n m k : Nat} (hmk : m < k) :
    exactBlockerFamilyCount n m k = 0 := by
  classical
  unfold exactBlockerFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    have hbudget : Bs.card ≤ m := mem_allBlockerFamiliesAtMost.mp hmem.1
    unfold FinsetHasExactlyBlockers at hmem
    omega
  · intro hmem
    simp at hmem

theorem FinsetHasExactlyBlockers.le_budget {n m k : Nat} {Bs : Finset (Blocker n)}
    (hmem : Bs ∈ allBlockerFamiliesAtMost n m) (hexact : FinsetHasExactlyBlockers Bs k) :
    k ≤ m := by
  have hbudget : Bs.card ≤ m := mem_allBlockerFamiliesAtMost.mp hmem
  unfold FinsetHasExactlyBlockers at hexact
  omega

theorem FinsetHasExactlyBlockers.eq_of {n k l : Nat} {Bs : Finset (Blocker n)}
    (hk : FinsetHasExactlyBlockers Bs k) (hl : FinsetHasExactlyBlockers Bs l) :
    k = l := by
  unfold FinsetHasExactlyBlockers at hk hl
  omega

theorem FinsetHasExactlyBlockers.not_two_rings {n k l : Nat} {Bs : Finset (Blocker n)}
    (hkl : k ≠ l) :
    ¬ (FinsetHasExactlyBlockers Bs k ∧ FinsetHasExactlyBlockers Bs l) := by
  intro h
  exact hkl (FinsetHasExactlyBlockers.eq_of h.1 h.2)

theorem exactBlockerFamilyCount_eq_zero_of_blockerUniverse_lt {n m k : Nat}
    (hk : Fintype.card (Blocker n) < k) :
    exactBlockerFamilyCount n m k = 0 := by
  classical
  unfold exactBlockerFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    have hle : Bs.card ≤ Fintype.card (Blocker n) := Finset.card_le_univ Bs
    unfold FinsetHasExactlyBlockers at hmem
    omega
  · intro hmem
    simp at hmem

theorem exactBlockerFamilyCount_self_budget_eq_budget {n m k : Nat} (hkm : k ≤ m) :
    exactBlockerFamilyCount n m k = exactBlockerFamilyCount n k k := by
  classical
  unfold exactBlockerFamilyCount classCount
  congr 1
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem ⊢
    have hcard : Bs.card = k := hmem.2
    exact ⟨mem_allBlockerFamiliesAtMost.mpr (by omega), hcard⟩
  · intro hmem
    rw [Finset.mem_filter] at hmem ⊢
    have hcard : Bs.card = k := hmem.2
    exact ⟨mem_allBlockerFamiliesAtMost.mpr (by omega), hcard⟩

theorem exactHoleFamilyCount_budget_mono {n m l k : Nat} (hml : m ≤ l) :
    exactHoleFamilyCount n m k ≤ exactHoleFamilyCount n l k := by
  unfold exactHoleFamilyCount
  exact classCount_budget_mono hml _

theorem uniqueHoleFamilyCount_budget_mono {n m l : Nat} (hml : m ≤ l) :
    uniqueHoleFamilyCount n m ≤ uniqueHoleFamilyCount n l := by
  unfold uniqueHoleFamilyCount
  exact classCount_budget_mono hml _

theorem fullCoverFamilyCount_budget_mono {n m l : Nat} (hml : m ≤ l) :
    fullCoverFamilyCount n m ≤ fullCoverFamilyCount n l := by
  unfold fullCoverFamilyCount
  exact classCount_budget_mono hml _

theorem exactHoleFamilyCount_le_manyHoleFamilyCount (n m k : Nat) :
    exactHoleFamilyCount n m k ≤ manyHoleFamilyCount n m k := by
  unfold exactHoleFamilyCount manyHoleFamilyCount
  apply classCount_mono
  intro Bs h
  unfold FinsetHasExactlyHoles at h
  unfold FinsetHasAtLeastHoles
  omega

theorem uniqueHoleFamilyCount_eq_exactHoleFamilyCount_one (n m : Nat) :
    uniqueHoleFamilyCount n m = exactHoleFamilyCount n m 1 := by
  rfl

theorem exactHoleFamilyCount_eq_zero_of_vertexCount_lt {n m k : Nat}
    (hk : 2 ^ n < k) :
    exactHoleFamilyCount n m k = 0 := by
  classical
  unfold exactHoleFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    unfold FinsetHasExactlyHoles at hmem
    have hle := finsetSolutionCount_le_vertexCount Bs
    omega
  · intro hmem
    simp at hmem

theorem manyHoleFamilyCount_eq_zero_of_vertexCount_lt {n m k : Nat}
    (hk : 2 ^ n < k) :
    manyHoleFamilyCount n m k = 0 := by
  classical
  unfold manyHoleFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    unfold FinsetHasAtLeastHoles at hmem
    have hle := finsetSolutionCount_le_vertexCount Bs
    omega
  · intro hmem
    simp at hmem

/-- Below the eight-blocker density floor, every bounded family is SAT with at least one hole. -/
theorem manyHoleFamilyCount_one_eq_total_of_budget_lt_eight {n m : Nat}
    (hm : m < 8) :
    manyHoleFamilyCount n m 1 = totalFamilyCountAtMost n m := by
  classical
  unfold manyHoleFamilyCount classCount totalFamilyCountAtMost
  have hset :
      (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FinsetHasAtLeastHoles Bs 1) =
        allBlockerFamiliesAtMost n m := by
    ext Bs
    constructor
    · intro hmem
      rw [Finset.mem_filter] at hmem
      exact hmem.1
    · intro hmem
      rw [Finset.mem_filter]
      have hcard : Bs.card < 8 := lt_of_le_of_lt (mem_allBlockerFamiliesAtMost.mp hmem) hm
      exact ⟨hmem, finsetHasAtLeastOneHole_of_card_lt_eight hcard⟩
  rw [hset]

/-- Unique-hole families lie in the full-variable-footprint bucket. This is the counted version
of the free-bit rule: a missing coordinate would duplicate the hole. -/
theorem uniqueHole_count_le_supportAtLeast_dimension_count (n m : Nat) :
    uniqueHoleFamilyCount n m ≤ supportAtLeastFamilyCount n m n := by
  unfold uniqueHoleFamilyCount supportAtLeastFamilyCount
  apply classCount_mono
  intro Bs hunique
  have hall : UsesAllVariables Bs := usesAllVariables_of_finsetHasUniqueHole hunique
  unfold FinsetUsesAtLeastVariables familySupportSize
  rw [hall]
  simp

/-- Unique-hole families land in the exact top support ring. This sharpens the previous
full-footprint rule from "at least `n`" to "exactly `n`". -/
theorem uniqueHole_count_le_supportExact_dimension_count (n m : Nat) :
    uniqueHoleFamilyCount n m ≤ supportExactFamilyCount n m n := by
  unfold uniqueHoleFamilyCount supportExactFamilyCount
  apply classCount_mono
  intro Bs hunique
  have hall : UsesAllVariables Bs := usesAllVariables_of_finsetHasUniqueHole hunique
  unfold FinsetUsesExactlyVariables familySupportSize
  rw [hall]
  simp

theorem supportExact_count_le_supportAtMost_count (n m k : Nat) :
    supportExactFamilyCount n m k ≤ supportAtMostFamilyCount n m k := by
  unfold supportExactFamilyCount supportAtMostFamilyCount
  apply classCount_mono
  intro Bs h
  unfold FinsetUsesExactlyVariables at h
  unfold FinsetUsesAtMostVariables
  omega

theorem supportExact_count_le_supportAtLeast_count (n m k : Nat) :
    supportExactFamilyCount n m k ≤ supportAtLeastFamilyCount n m k := by
  unfold supportExactFamilyCount supportAtLeastFamilyCount
  apply classCount_mono
  intro Bs h
  unfold FinsetUsesExactlyVariables at h
  unfold FinsetUsesAtLeastVariables
  omega

theorem supportAtMostFamilyCount_mono {n m k l : Nat} (hkl : k ≤ l) :
    supportAtMostFamilyCount n m k ≤ supportAtMostFamilyCount n m l := by
  unfold supportAtMostFamilyCount
  apply classCount_mono
  intro Bs h
  unfold FinsetUsesAtMostVariables at h ⊢
  omega

theorem supportAtLeastFamilyCount_antitone {n m k l : Nat} (hkl : k ≤ l) :
    supportAtLeastFamilyCount n m l ≤ supportAtLeastFamilyCount n m k := by
  unfold supportAtLeastFamilyCount
  apply classCount_mono
  intro Bs h
  unfold FinsetUsesAtLeastVariables at h ⊢
  omega

theorem supportAtMostFamilyCount_at_dimension_eq_total (n m : Nat) :
    supportAtMostFamilyCount n m n = totalFamilyCountAtMost n m := by
  classical
  unfold supportAtMostFamilyCount classCount totalFamilyCountAtMost
  have hset :
      (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FinsetUsesAtMostVariables Bs n) =
        allBlockerFamiliesAtMost n m := by
    ext Bs
    constructor
    · intro h
      rw [Finset.mem_filter] at h
      exact h.1
    · intro hmem
      rw [Finset.mem_filter]
      exact ⟨hmem, familySupportSize_le_dimension Bs⟩
  rw [hset]

theorem supportAtLeastFamilyCount_zero_eq_total (n m : Nat) :
    supportAtLeastFamilyCount n m 0 = totalFamilyCountAtMost n m := by
  classical
  unfold supportAtLeastFamilyCount classCount totalFamilyCountAtMost
  have hset :
      (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FinsetUsesAtLeastVariables Bs 0) =
        allBlockerFamiliesAtMost n m := by
    ext Bs
    constructor
    · intro h
      rw [Finset.mem_filter] at h
      exact h.1
    · intro hmem
      rw [Finset.mem_filter]
      exact ⟨hmem, Nat.zero_le _⟩
  rw [hset]

theorem supportExactFamilyCount_eq_zero_of_dimension_lt {n m k : Nat}
    (hnk : n < k) :
    supportExactFamilyCount n m k = 0 := by
  classical
  unfold supportExactFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    unfold FinsetUsesExactlyVariables at hmem
    have hle := familySupportSize_le_dimension Bs
    omega
  · intro hmem
    simp at hmem

theorem supportAtLeastFamilyCount_eq_zero_of_dimension_lt {n m k : Nat}
    (hnk : n < k) :
    supportAtLeastFamilyCount n m k = 0 := by
  classical
  unfold supportAtLeastFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    unfold FinsetUsesAtLeastVariables at hmem
    have hle := familySupportSize_le_dimension Bs
    omega
  · intro hmem
    simp at hmem

theorem supportAtLeastFamilyCount_at_dimension_eq_supportExact (n m : Nat) :
    supportAtLeastFamilyCount n m n = supportExactFamilyCount n m n := by
  classical
  unfold supportAtLeastFamilyCount supportExactFamilyCount classCount
  have hset :
      (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FinsetUsesAtLeastVariables Bs n) =
        (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FinsetUsesExactlyVariables Bs n) := by
    ext Bs
    constructor
    · intro hmem
      rw [Finset.mem_filter] at hmem ⊢
      unfold FinsetUsesAtLeastVariables at hmem
      unfold FinsetUsesExactlyVariables
      have hle := familySupportSize_le_dimension Bs
      exact ⟨hmem.1, by omega⟩
    · intro hmem
      rw [Finset.mem_filter] at hmem ⊢
      unfold FinsetUsesExactlyVariables at hmem
      unfold FinsetUsesAtLeastVariables
      exact ⟨hmem.1, by omega⟩
  rw [hset]

/-- Sparse density collapse: with fewer than eight clean 3-blockers, no bounded family can be
a full cover. -/
theorem fullCoverFamilyCount_eq_zero_of_budget_lt_eight {n m : Nat}
    (hm : m < 8) :
    fullCoverFamilyCount n m = 0 := by
  classical
  unfold fullCoverFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    have hcard : Bs.card < 8 := lt_of_le_of_lt (mem_allBlockerFamiliesAtMost.mp hmem.1) hm
    exact False.elim (not_fullCoverBlockers_of_card_lt_eight hcard hmem.2)
  · intro hmem
    simp at hmem

/-- In dimension at least three, fewer than seven clean blockers cannot leave a unique hole. -/
theorem uniqueHoleFamilyCount_eq_zero_of_budget_lt_seven {n m : Nat}
    (hn : 3 ≤ n) (hm : m < 7) :
    uniqueHoleFamilyCount n m = 0 := by
  classical
  unfold uniqueHoleFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    have hbudget : Bs.card ≤ m := mem_allBlockerFamiliesAtMost.mp hmem.1
    have hge : 7 ≤ Bs.card := FinsetHasUniqueHole.card_ge_seven hn hmem.2
    omega
  · intro hmem
    simp at hmem

/-- Exact zero-hole families and one-or-more-hole families partition the bounded universe. This
is the SAT/UNSAT split stated purely as hole-count accounting. -/
theorem exactHoleFamilyCount_zero_add_manyHoleFamilyCount_one_eq_total (n m : Nat) :
    exactHoleFamilyCount n m 0 + manyHoleFamilyCount n m 1 =
      totalFamilyCountAtMost n m := by
  classical
  unfold exactHoleFamilyCount manyHoleFamilyCount classCount totalFamilyCountAtMost
  have hpart := Finset.card_filter_add_card_filter_not
    (s := allBlockerFamiliesAtMost n m)
    (p := fun Bs : Finset (Blocker n) => FinsetHasExactlyHoles Bs 0)
  have hnot :
      (Finset.filter
          (fun Bs : Finset (Blocker n) => ¬ FinsetHasExactlyHoles Bs 0)
          (allBlockerFamiliesAtMost n m)) =
        (Finset.filter
          (fun Bs : Finset (Blocker n) => FinsetHasAtLeastHoles Bs 1)
          (allBlockerFamiliesAtMost n m)) := by
    ext Bs
    constructor
    · intro hmem
      rw [Finset.mem_filter] at hmem ⊢
      unfold FinsetHasExactlyHoles at hmem
      unfold FinsetHasAtLeastHoles
      exact ⟨hmem.1, by omega⟩
    · intro hmem
      rw [Finset.mem_filter] at hmem ⊢
      unfold FinsetHasAtLeastHoles at hmem
      unfold FinsetHasExactlyHoles
      exact ⟨hmem.1, by omega⟩
  rw [hnot] at hpart
  simpa using hpart

theorem finsetHasAtLeastHoles_of_subset {n k : Nat}
    {Bs Cs : Finset (Blocker n)} (hsub : Bs ⊆ Cs)
    (hholes : FinsetHasAtLeastHoles Cs k) :
    FinsetHasAtLeastHoles Bs k := by
  unfold FinsetHasAtLeastHoles at hholes ⊢
  exact hholes.trans (finsetSolutionCount_antitone hsub)

theorem finsetHasAtLeastOneHole_of_subset_of_unique_superset {n : Nat}
    {Bs Cs : Finset (Blocker n)} (hsub : Bs ⊆ Cs)
    (hunique : FinsetHasUniqueHole Cs) :
    FinsetHasAtLeastHoles Bs 1 := by
  apply finsetHasAtLeastHoles_of_subset hsub
  unfold FinsetHasAtLeastHoles FinsetHasUniqueHole at *
  omega

theorem fullCoverFamilyCount_eq_exactHoleFamilyCount_zero (n m : Nat) :
    fullCoverFamilyCount n m = exactHoleFamilyCount n m 0 := by
  classical
  unfold fullCoverFamilyCount exactHoleFamilyCount classCount
  have hset :
      (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FullCoverBlockers Bs) =
        (allBlockerFamiliesAtMost n m).filter
          (fun Bs => FinsetHasExactlyHoles Bs 0) := by
    ext Bs
    constructor
    · intro hmem
      rw [Finset.mem_filter] at hmem ⊢
      unfold FinsetHasExactlyHoles
      exact ⟨hmem.1, (fullCoverBlockers_iff_finsetSolutionCount_eq_zero Bs).mp hmem.2⟩
    · intro hmem
      rw [Finset.mem_filter] at hmem ⊢
      unfold FinsetHasExactlyHoles at hmem
      exact ⟨hmem.1, (fullCoverBlockers_iff_finsetSolutionCount_eq_zero Bs).mpr hmem.2⟩
  rw [hset]

/-- The named SAT/UNSAT counting partition: full covers and families with at least one hole
split the bounded family universe. -/
theorem fullCoverFamilyCount_add_manyHoleFamilyCount_one_eq_total (n m : Nat) :
    fullCoverFamilyCount n m + manyHoleFamilyCount n m 1 =
      totalFamilyCountAtMost n m := by
  rw [fullCoverFamilyCount_eq_exactHoleFamilyCount_zero]
  exact exactHoleFamilyCount_zero_add_manyHoleFamilyCount_one_eq_total n m

/-- Locally invisible full-cover families also vanish below the eight-blocker density floor. -/
theorem locallyInvisibleFamilyCount_eq_zero_of_budget_lt_eight {n m r : Nat}
    (hm : m < 8) :
    locallyInvisibleFamilyCount n m r = 0 := by
  classical
  unfold locallyInvisibleFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    have hcard : Bs.card < 8 := lt_of_le_of_lt (mem_allBlockerFamiliesAtMost.mp hmem.1) hm
    exact False.elim (not_fullCoverBlockers_of_card_lt_eight hcard hmem.2.1)
  · intro hmem
    simp at hmem

/-- The current geometric hard-UNSAT candidate envelope is empty below the eight-blocker
density floor, since it includes full cover. -/
theorem geometricHardUNSATCandidateCount_eq_zero_of_budget_lt_eight {n m r : Nat}
    (hm : m < 8) :
    geometricHardUNSATCandidateCount n m r = 0 := by
  classical
  unfold geometricHardUNSATCandidateCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    rw [Finset.mem_filter] at hmem
    have hcard : Bs.card < 8 := lt_of_le_of_lt (mem_allBlockerFamiliesAtMost.mp hmem.1) hm
    exact False.elim (not_fullCoverBlockers_of_card_lt_eight hcard hmem.2.1.1)
  · intro hmem
    simp at hmem

/-- Locally invisible full covers are included among full covers. -/
theorem locallyInvisible_count_le_fullCover_count (n m r : Nat) :
    locallyInvisibleFamilyCount n m r ≤ fullCoverFamilyCount n m := by
  unfold locallyInvisibleFamilyCount fullCoverFamilyCount
  apply classCount_mono
  intro Bs h
  exact h.1

/-- Geometric hard-UNSAT candidates are full covers. -/
theorem geometricHardUNSATCandidate_count_le_fullCover_count (n m r : Nat) :
    geometricHardUNSATCandidateCount n m r ≤ fullCoverFamilyCount n m := by
  unfold geometricHardUNSATCandidateCount fullCoverFamilyCount
  apply classCount_mono
  intro Bs h
  exact h.1.1

/-! ### Hypercube symmetries -/

/-- A hypercube symmetry: permute coordinates and optionally flip each coordinate. These are
the automorphisms that preserve clean 3-bit blocker shape. -/
structure CubeSymmetry (n : Nat) where
  perm : Equiv.Perm (Fin n)
  flip : Fin n → Bool

namespace CubeSymmetry

/-- The identity hypercube symmetry. -/
def id (n : Nat) : CubeSymmetry n where
  perm := Equiv.refl (Fin n)
  flip := fun _ => false

/-- Transform one bit on coordinate `i`. -/
def mapBit {n : Nat} (S : CubeSymmetry n) (i : Fin n) (b : Bool) : Bool :=
  if S.flip i then !b else b

@[simp] theorem mapBit_involutive {n : Nat} (S : CubeSymmetry n) (i : Fin n)
    (b : Bool) :
    S.mapBit i (S.mapBit i b) = b := by
  unfold mapBit
  by_cases h : S.flip i <;> simp [h]

theorem mapBit_eq_iff {n : Nat} (S : CubeSymmetry n) (i : Fin n)
    {a b : Bool} :
    S.mapBit i a = S.mapBit i b ↔ a = b := by
  unfold mapBit
  by_cases h : S.flip i <;> simp [h]

/-- Push a vertex forward through the symmetry. -/
def mapVertex {n : Nat} (S : CubeSymmetry n) (v : Vertex n) : Vertex n :=
  fun i => S.mapBit (S.perm.symm i) (v (S.perm.symm i))

/-- Pull a vertex back through the symmetry. -/
def invVertex {n : Nat} (S : CubeSymmetry n) (v : Vertex n) : Vertex n :=
  fun i => S.mapBit i (v (S.perm i))

@[simp] theorem mapVertex_perm {n : Nat} (S : CubeSymmetry n)
    (v : Vertex n) (i : Fin n) :
    S.mapVertex v (S.perm i) = S.mapBit i (v i) := by
  simp [mapVertex, mapBit]

@[simp] theorem mapVertex_invVertex {n : Nat} (S : CubeSymmetry n)
    (v : Vertex n) :
    S.mapVertex (S.invVertex v) = v := by
  funext i
  simp [mapVertex, invVertex]

@[simp] theorem invVertex_mapVertex {n : Nat} (S : CubeSymmetry n)
    (v : Vertex n) :
    S.invVertex (S.mapVertex v) = v := by
  funext i
  simp [mapVertex, invVertex]

theorem mapVertex_injective {n : Nat} (S : CubeSymmetry n) :
    Function.Injective S.mapVertex := by
  intro v w h
  have h' : S.invVertex (S.mapVertex v) = S.invVertex (S.mapVertex w) := congrArg S.invVertex h
  simpa using h'

/-- Push a clean blocker through the symmetry. -/
def mapBlocker {n : Nat} (S : CubeSymmetry n) (B : Blocker n) : Blocker n :=
  { i := S.perm B.i
    j := S.perm B.j
    k := S.perm B.k
    hij := by intro h; exact B.hij (S.perm.injective h)
    hik := by intro h; exact B.hik (S.perm.injective h)
    hjk := by intro h; exact B.hjk (S.perm.injective h)
    bi := S.mapBit B.i B.bi
    bj := S.mapBit B.j B.bj
    bk := S.mapBit B.k B.bk }

/-- Push a finite blocker family through the symmetry. -/
def mapFamily {n : Nat} (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    Finset (Blocker n) :=
  Bs.image S.mapBlocker

theorem mapBlocker_injective {n : Nat} (S : CubeSymmetry n) :
    Function.Injective S.mapBlocker := by
  intro B C h
  cases B with
  | mk i j k hij hik hjk bi bj bk =>
    cases C with
    | mk i' j' k' hij' hik' hjk' bi' bj' bk' =>
      simp only [mapBlocker, Blocker.mk.injEq] at h
      rcases h with ⟨hi, hj, hk, hbi, hbj, hbk⟩
      have hi_eq : i = i' := S.perm.injective hi
      have hj_eq : j = j' := S.perm.injective hj
      have hk_eq : k = k' := S.perm.injective hk
      cases hi_eq
      cases hj_eq
      cases hk_eq
      have hbi' : bi = bi' := (S.mapBit_eq_iff i).mp hbi
      have hbj' : bj = bj' := (S.mapBit_eq_iff j).mp hbj
      have hbk' : bk = bk' := (S.mapBit_eq_iff k).mp hbk
      cases hbi'
      cases hbj'
      cases hbk'
      simp

theorem mapFamily_card {n : Nat} (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    (S.mapFamily Bs).card = Bs.card := by
  unfold mapFamily
  exact Finset.card_image_of_injective _ S.mapBlocker_injective

@[simp] theorem id_mapBlocker {n : Nat} (B : Blocker n) :
    (CubeSymmetry.id n).mapBlocker B = B := by
  cases B
  simp [CubeSymmetry.id, mapBlocker, mapBit]

@[simp] theorem id_mapFamily {n : Nat} (Bs : Finset (Blocker n)) :
    (CubeSymmetry.id n).mapFamily Bs = Bs := by
  unfold mapFamily
  ext B
  simp

/-- Symmetries carry a blocker support exactly by the coordinate permutation. -/
theorem mapBlocker_support {n : Nat} (S : CubeSymmetry n) (B : Blocker n) :
    (S.mapBlocker B).support = B.support.image S.perm := by
  ext x
  simp [Blocker.support, mapBlocker]

/-- Symmetries carry the family support exactly by the coordinate permutation. -/
theorem mapFamily_support {n : Nat} (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    BlockerFamily.support (S.mapFamily Bs) =
      (BlockerFamily.support Bs).image S.perm := by
  ext x
  constructor
  · intro hx
    rw [BlockerFamily.mem_support_iff] at hx
    obtain ⟨C, hC, hxC⟩ := hx
    obtain ⟨B, hB, rfl⟩ := Finset.mem_image.mp hC
    rw [S.mapBlocker_support B] at hxC
    obtain ⟨y, hy, hxy⟩ := Finset.mem_image.mp hxC
    exact Finset.mem_image.mpr
      ⟨y, (BlockerFamily.mem_support_iff Bs y).mpr ⟨B, hB, hy⟩, hxy⟩
  · intro hx
    obtain ⟨y, hy, hxy⟩ := Finset.mem_image.mp hx
    rw [BlockerFamily.mem_support_iff] at hy
    obtain ⟨B, hB, hyB⟩ := hy
    rw [BlockerFamily.mem_support_iff]
    refine ⟨S.mapBlocker B, Finset.mem_image.mpr ⟨B, hB, rfl⟩, ?_⟩
    rw [S.mapBlocker_support B]
    exact Finset.mem_image.mpr ⟨y, hyB, hxy⟩

/-- Using every variable is a symmetry-invariant geometric envelope. -/
theorem usesAllVariables_mapFamily_iff {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    UsesAllVariables (S.mapFamily Bs) ↔ UsesAllVariables Bs := by
  unfold UsesAllVariables
  constructor
  · intro h
    apply Finset.eq_univ_iff_forall.mpr
    intro x
    have hx : S.perm x ∈ (BlockerFamily.support Bs).image S.perm := by
      rw [← S.mapFamily_support Bs, h]
      simp
    obtain ⟨y, hy, hxy⟩ := Finset.mem_image.mp hx
    have hyx : y = x := S.perm.injective hxy
    simpa [hyx] using hy
  · intro h
    rw [S.mapFamily_support Bs, h]
    ext x
    simp

/-- Blocker adjacency through a shared coordinate is preserved by cube symmetries. -/
theorem mapBlocker_touches_iff {n : Nat} (S : CubeSymmetry n) (B C : Blocker n) :
    BlockerTouches (S.mapBlocker B) (S.mapBlocker C) ↔ BlockerTouches B C := by
  unfold BlockerTouches
  constructor
  · rintro ⟨x, hxB, hxC⟩
    rw [S.mapBlocker_support B] at hxB
    rw [S.mapBlocker_support C] at hxC
    obtain ⟨y, hyB, hyx⟩ := Finset.mem_image.mp hxB
    obtain ⟨z, hzC, hzx⟩ := Finset.mem_image.mp hxC
    have hyz : y = z := S.perm.injective (hyx.trans hzx.symm)
    exact ⟨y, hyB, by simpa [hyz] using hzC⟩
  · rintro ⟨x, hxB, hxC⟩
    refine ⟨S.perm x, ?_, ?_⟩
    · rw [S.mapBlocker_support B]
      exact Finset.mem_image.mpr ⟨x, hxB, rfl⟩
    · rw [S.mapBlocker_support C]
      exact Finset.mem_image.mpr ⟨x, hxC, rfl⟩

/-- Pairwise-disjoint blocker decompositions are preserved by cube symmetries. -/
theorem pairwiseDisjointBlockers_mapFamily_of_pairwiseDisjoint {n : Nat}
    (S : CubeSymmetry n) {Bs : Finset (Blocker n)}
    (hpair : PairwiseDisjointBlockers Bs) :
    PairwiseDisjointBlockers (S.mapFamily Bs) := by
  intro C hC D hD hCD htouch
  obtain ⟨B, hB, rfl⟩ := Finset.mem_image.mp hC
  obtain ⟨E, hE, rfl⟩ := Finset.mem_image.mp hD
  have hBE : B ≠ E := by
    intro h
    subst E
    exact hCD rfl
  exact hpair B hB E hE hBE ((S.mapBlocker_touches_iff B E).mp htouch)

/-- Membership in a permuted coordinate set is exactly membership before permutation. -/
theorem mem_perm_image_iff {n : Nat} (S : CubeSymmetry n) (U : Finset (Fin n))
    (x : Fin n) :
    S.perm x ∈ U.image S.perm ↔ x ∈ U := by
  constructor
  · intro hx
    obtain ⟨y, hy, hyx⟩ := Finset.mem_image.mp hx
    have hyx' : y = x := S.perm.injective hyx
    simpa [hyx'] using hy
  · intro hx
    exact Finset.mem_image.mpr ⟨x, hx, rfl⟩

/-- Subset relations between coordinate sets are preserved and reflected by permutation. -/
theorem image_subset_image_perm_iff {n : Nat} (S : CubeSymmetry n)
    (A U : Finset (Fin n)) :
    A.image S.perm ⊆ U.image S.perm ↔ A ⊆ U := by
  constructor
  · intro h x hx
    exact (S.mem_perm_image_iff U x).mp (h (Finset.mem_image.mpr ⟨x, hx, rfl⟩))
  · intro h y hy
    obtain ⟨x, hx, rfl⟩ := Finset.mem_image.mp hy
    exact Finset.mem_image.mpr ⟨x, h hx, rfl⟩

/-- Disjointness of coordinate sets is preserved and reflected by permutation. -/
theorem disjoint_image_perm_iff {n : Nat} (S : CubeSymmetry n)
    (A U : Finset (Fin n)) :
    Disjoint (A.image S.perm) (U.image S.perm) ↔ Disjoint A U := by
  constructor
  · intro h
    rw [Finset.disjoint_left]
    intro x hxA hxU
    exact (Finset.disjoint_left.mp h) (Finset.mem_image.mpr ⟨x, hxA, rfl⟩)
      (Finset.mem_image.mpr ⟨x, hxU, rfl⟩)
  · intro h
    rw [Finset.disjoint_left]
    intro y hyA hyU
    obtain ⟨x, hxA, hxy⟩ := Finset.mem_image.mp hyA
    obtain ⟨z, hzU, hzy⟩ := Finset.mem_image.mp hyU
    have hxz : x = z := S.perm.injective (hxy.trans hzy.symm)
    exact (Finset.disjoint_left.mp h) hxA (by simpa [hxz] using hzU)

/-- Variable-interaction edges are preserved and reflected by cube symmetries. -/
theorem variableInteraction_mapFamily_perm_iff {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) (i j : Fin n) :
    BlockerFamily.VariableInteraction (S.mapFamily Bs) (S.perm i) (S.perm j) ↔
      BlockerFamily.VariableInteraction Bs i j := by
  unfold BlockerFamily.VariableInteraction
  constructor
  · rintro ⟨hij, C, hC, hiC, hjC⟩
    obtain ⟨B, hB, rfl⟩ := Finset.mem_image.mp hC
    rw [S.mapBlocker_support B] at hiC hjC
    exact ⟨by intro h; exact hij (by rw [h]),
      B, hB, (S.mem_perm_image_iff B.support i).mp hiC,
      (S.mem_perm_image_iff B.support j).mp hjC⟩
  · rintro ⟨hij, B, hB, hiB, hjB⟩
    exact ⟨by intro h; exact hij (S.perm.injective h),
      S.mapBlocker B, Finset.mem_image.mpr ⟨B, hB, rfl⟩,
      by rw [S.mapBlocker_support B]; exact Finset.mem_image.mpr ⟨i, hiB, rfl⟩,
      by rw [S.mapBlocker_support B]; exact Finset.mem_image.mpr ⟨j, hjB, rfl⟩⟩

/-- Coordinate separators are preserved and reflected by cube symmetries. -/
theorem separatedBy_mapFamily_image_iff {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) (U : Finset (Fin n)) :
    BlockerFamily.SeparatedBy (S.mapFamily Bs) (U.image S.perm) ↔
      BlockerFamily.SeparatedBy Bs U := by
  unfold BlockerFamily.SeparatedBy
  constructor
  · intro hsep B hB
    have h := hsep (S.mapBlocker B) (Finset.mem_image.mpr ⟨B, hB, rfl⟩)
    rw [S.mapBlocker_support B] at h
    rcases h with hsub | hdisj
    · exact Or.inl ((S.image_subset_image_perm_iff B.support U).mp hsub)
    · exact Or.inr ((S.disjoint_image_perm_iff B.support U).mp hdisj)
  · intro hsep C hC
    obtain ⟨B, hB, rfl⟩ := Finset.mem_image.mp hC
    have h := hsep B hB
    rw [S.mapBlocker_support B]
    rcases h with hsub | hdisj
    · exact Or.inl ((S.image_subset_image_perm_iff B.support U).mpr hsub)
    · exact Or.inr ((S.disjoint_image_perm_iff B.support U).mpr hdisj)

/-- Small coordinate-separator witnesses push forward through cube symmetries. -/
theorem hasCoordinateSeparatorAtMost_mapFamily_of {n k : Nat}
    (S : CubeSymmetry n) {Bs : Finset (Blocker n)}
    (hsep : HasCoordinateSeparatorAtMost Bs k) :
    HasCoordinateSeparatorAtMost (S.mapFamily Bs) k := by
  obtain ⟨U, hcard, hU⟩ := hsep
  refine ⟨U.image S.perm, ?_, ?_⟩
  · rw [Finset.card_image_of_injective _ S.perm.injective]
    exact hcard
  · exact (S.separatedBy_mapFamily_image_iff Bs U).mpr hU

/-- Imaging by the inverse coordinate permutation and then by the permutation returns the set. -/
theorem image_symm_image_perm {n : Nat} (S : CubeSymmetry n) (U : Finset (Fin n)) :
    (U.image S.perm.symm).image S.perm = U := by
  ext x
  constructor
  · intro hx
    obtain ⟨y, hy, hyx⟩ := Finset.mem_image.mp hx
    obtain ⟨z, hz, hzy⟩ := Finset.mem_image.mp hy
    have hxz : x = z := by
      rw [← hyx, ← hzy]
      simp
    simpa [hxz] using hz
  · intro hx
    exact Finset.mem_image.mpr
      ⟨S.perm.symm x, Finset.mem_image.mpr ⟨x, hx, rfl⟩, by simp⟩

/-- Having a small coordinate separator is symmetry-invariant. -/
theorem hasCoordinateSeparatorAtMost_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    HasCoordinateSeparatorAtMost (S.mapFamily Bs) k ↔ HasCoordinateSeparatorAtMost Bs k := by
  constructor
  · intro hsep
    obtain ⟨V, hcard, hV⟩ := hsep
    refine ⟨V.image S.perm.symm, ?_, ?_⟩
    · rw [Finset.card_image_of_injective _ S.perm.symm.injective]
      exact hcard
    · have hV' : BlockerFamily.SeparatedBy (S.mapFamily Bs)
          ((V.image S.perm.symm).image S.perm) := by
        simpa [S.image_symm_image_perm V] using hV
      exact (S.separatedBy_mapFamily_image_iff Bs (V.image S.perm.symm)).mp hV'
  · intro hsep
    exact S.hasCoordinateSeparatorAtMost_mapFamily_of hsep

/-- Nontriviality of a coordinate cut is preserved and reflected by permutation. -/
theorem nontrivialCoordinateCut_image_perm_iff {n : Nat}
    (S : CubeSymmetry n) (U : Finset (Fin n)) :
    NontrivialCoordinateCut (U.image S.perm) ↔ NontrivialCoordinateCut U := by
  unfold NontrivialCoordinateCut
  constructor
  · rintro ⟨hnonempty, hcard⟩
    obtain ⟨x, hx⟩ := hnonempty
    obtain ⟨y, hy, -⟩ := Finset.mem_image.mp hx
    refine ⟨⟨y, hy⟩, ?_⟩
    rwa [Finset.card_image_of_injective _ S.perm.injective] at hcard
  · rintro ⟨hnonempty, hcard⟩
    obtain ⟨x, hx⟩ := hnonempty
    refine ⟨⟨S.perm x, Finset.mem_image.mpr ⟨x, hx, rfl⟩⟩, ?_⟩
    rwa [Finset.card_image_of_injective _ S.perm.injective]

/-- Nontrivial small separator witnesses push forward through cube symmetries. -/
theorem hasNontrivialCoordinateSeparatorAtMost_mapFamily_of {n k : Nat}
    (S : CubeSymmetry n) {Bs : Finset (Blocker n)}
    (hsep : HasNontrivialCoordinateSeparatorAtMost Bs k) :
    HasNontrivialCoordinateSeparatorAtMost (S.mapFamily Bs) k := by
  obtain ⟨U, hcard, hnontrivial, hU⟩ := hsep
  refine ⟨U.image S.perm, ?_, ?_, ?_⟩
  · rw [Finset.card_image_of_injective _ S.perm.injective]
    exact hcard
  · exact (S.nontrivialCoordinateCut_image_perm_iff U).mpr hnontrivial
  · exact (S.separatedBy_mapFamily_image_iff Bs U).mpr hU

/-- Having a nontrivial small coordinate separator is symmetry-invariant. -/
theorem hasNontrivialCoordinateSeparatorAtMost_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    HasNontrivialCoordinateSeparatorAtMost (S.mapFamily Bs) k ↔
      HasNontrivialCoordinateSeparatorAtMost Bs k := by
  constructor
  · intro hsep
    obtain ⟨V, hcard, hnontrivial, hV⟩ := hsep
    refine ⟨V.image S.perm.symm, ?_, ?_, ?_⟩
    · rw [Finset.card_image_of_injective _ S.perm.symm.injective]
      exact hcard
    · have hnontrivial' :
          NontrivialCoordinateCut ((V.image S.perm.symm).image S.perm) := by
        simpa [S.image_symm_image_perm V] using hnontrivial
      exact (S.nontrivialCoordinateCut_image_perm_iff (V.image S.perm.symm)).mp hnontrivial'
    · have hV' : BlockerFamily.SeparatedBy (S.mapFamily Bs)
          ((V.image S.perm.symm).image S.perm) := by
        simpa [S.image_symm_image_perm V] using hV
      exact (S.separatedBy_mapFamily_image_iff Bs (V.image S.perm.symm)).mp hV'
  · intro hsep
    exact S.hasNontrivialCoordinateSeparatorAtMost_mapFamily_of hsep

/-- The entanglement bucket "no nontrivial separator up to size `k`" is symmetry-invariant. -/
theorem noNontrivialCoordinateSeparatorAtMost_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    NoNontrivialCoordinateSeparatorAtMost (S.mapFamily Bs) k ↔
      NoNontrivialCoordinateSeparatorAtMost Bs k := by
  unfold NoNontrivialCoordinateSeparatorAtMost
  rw [S.hasNontrivialCoordinateSeparatorAtMost_mapFamily_iff Bs]

/-- Symmetries preserve blocker coverage. -/
theorem mapBlocker_covers_mapVertex_iff {n : Nat} (S : CubeSymmetry n)
    (B : Blocker n) (v : Vertex n) :
    (S.mapBlocker B).Covers (S.mapVertex v) ↔ B.Covers v := by
  unfold Blocker.Covers mapBlocker
  simp only [mapVertex_perm]
  constructor
  · intro h
    exact ⟨(S.mapBit_eq_iff B.i).mp h.1,
      (S.mapBit_eq_iff B.j).mp h.2.1,
      (S.mapBit_eq_iff B.k).mp h.2.2⟩
  · intro h
    exact ⟨by rw [h.1], by rw [h.2.1], by rw [h.2.2]⟩

theorem vertexUncovered_mapFamily_mapVertex_iff {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) (v : Vertex n) :
    VertexUncoveredBy (S.mapFamily Bs) (S.mapVertex v) ↔ VertexUncoveredBy Bs v := by
  rw [VertexUncoveredBy_iff_no_cover, VertexUncoveredBy_iff_no_cover]
  constructor
  · intro h B hB hcover
    exact h (S.mapBlocker B) (Finset.mem_image.mpr ⟨B, hB, rfl⟩)
      ((S.mapBlocker_covers_mapVertex_iff B v).mpr hcover)
  · intro h C hC hcover
    obtain ⟨B, hB, rfl⟩ := Finset.mem_image.mp hC
    exact h B hB ((S.mapBlocker_covers_mapVertex_iff B v).mp hcover)

theorem finsetUncoveredVertices_mapFamily {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    finsetUncoveredVertices (S.mapFamily Bs) =
      (finsetUncoveredVertices Bs).image S.mapVertex := by
  ext w
  constructor
  · intro hw
    let v := S.invVertex w
    have hv : v ∈ finsetUncoveredVertices Bs := by
      rw [mem_finsetUncoveredVertices]
      have hw' : VertexUncoveredBy (S.mapFamily Bs) (S.mapVertex v) := by
        simpa [v] using hw
      exact (S.vertexUncovered_mapFamily_mapVertex_iff Bs v).mp hw'
    exact Finset.mem_image.mpr ⟨v, hv, by simp [v]⟩
  · intro hw
    obtain ⟨v, hv, rfl⟩ := Finset.mem_image.mp hw
    rw [mem_finsetUncoveredVertices] at hv ⊢
    exact (S.vertexUncovered_mapFamily_mapVertex_iff Bs v).mpr hv

theorem finsetSolutionCount_mapFamily {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    finsetSolutionCount (S.mapFamily Bs) = finsetSolutionCount Bs := by
  unfold finsetSolutionCount
  rw [S.finsetUncoveredVertices_mapFamily Bs]
  exact Finset.card_image_of_injective _ S.mapVertex_injective

theorem familySupportSize_mapFamily {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    familySupportSize (S.mapFamily Bs) = familySupportSize Bs := by
  unfold familySupportSize
  rw [S.mapFamily_support Bs]
  exact Finset.card_image_of_injective _ S.perm.injective

theorem unusedVariableCount_mapFamily {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    unusedVariableCount (S.mapFamily Bs) = unusedVariableCount Bs := by
  unfold unusedVariableCount
  rw [S.familySupportSize_mapFamily Bs]

theorem finsetHasExactlyBlockers_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    FinsetHasExactlyBlockers (S.mapFamily Bs) k ↔ FinsetHasExactlyBlockers Bs k := by
  unfold FinsetHasExactlyBlockers
  rw [S.mapFamily_card Bs]

theorem finsetHasAtLeastHoles_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    FinsetHasAtLeastHoles (S.mapFamily Bs) k ↔ FinsetHasAtLeastHoles Bs k := by
  unfold FinsetHasAtLeastHoles
  rw [S.finsetSolutionCount_mapFamily Bs]

theorem finsetUsesAtMostVariables_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    FinsetUsesAtMostVariables (S.mapFamily Bs) k ↔ FinsetUsesAtMostVariables Bs k := by
  unfold FinsetUsesAtMostVariables
  rw [S.familySupportSize_mapFamily Bs]

theorem finsetUsesAtLeastVariables_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    FinsetUsesAtLeastVariables (S.mapFamily Bs) k ↔ FinsetUsesAtLeastVariables Bs k := by
  unfold FinsetUsesAtLeastVariables
  rw [S.familySupportSize_mapFamily Bs]

theorem finsetUsesExactlyVariables_mapFamily_iff {n k : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    FinsetUsesExactlyVariables (S.mapFamily Bs) k ↔ FinsetUsesExactlyVariables Bs k := by
  unfold FinsetUsesExactlyVariables
  rw [S.familySupportSize_mapFamily Bs]

theorem finsetHasUniqueHole_mapFamily_iff {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    FinsetHasUniqueHole (S.mapFamily Bs) ↔ FinsetHasUniqueHole Bs := by
  unfold FinsetHasUniqueHole
  rw [S.finsetSolutionCount_mapFamily Bs]

theorem fullCoverBlockers_mapFamily_iff {n : Nat}
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    FullCoverBlockers (S.mapFamily Bs) ↔ FullCoverBlockers Bs := by
  rw [fullCoverBlockers_iff_finsetSolutionCount_eq_zero,
    fullCoverBlockers_iff_finsetSolutionCount_eq_zero,
    S.finsetSolutionCount_mapFamily Bs]

/-- A structural class is symmetry-invariant when every hypercube automorphism preserves it.
These are the classes where we may reason orbit-by-orbit instead of instance-by-instance. -/
def InvariantClass {n : Nat} (Class : BlockerFamilyClass n) : Prop :=
  ∀ S : CubeSymmetry n, ∀ Bs : Finset (Blocker n),
    Class (S.mapFamily Bs) ↔ Class Bs

/-- Two blocker families are the same geometric instance when one is obtained from the other by
a hypercube symmetry: coordinate permutation plus independent bit flips. -/
def SymmetryEquivalent {n : Nat} (Bs Cs : Finset (Blocker n)) : Prop :=
  ∃ S : CubeSymmetry n, S.mapFamily Bs = Cs

theorem SymmetryEquivalent.refl {n : Nat} (Bs : Finset (Blocker n)) :
    SymmetryEquivalent Bs Bs := by
  exact ⟨CubeSymmetry.id n, by simp⟩

theorem SymmetryEquivalent.card_eq {n : Nat} {Bs Cs : Finset (Blocker n)}
    (h : SymmetryEquivalent Bs Cs) :
    Cs.card = Bs.card := by
  rcases h with ⟨S, rfl⟩
  exact S.mapFamily_card Bs

theorem SymmetryEquivalent.solutionCount_eq {n : Nat} {Bs Cs : Finset (Blocker n)}
    (h : SymmetryEquivalent Bs Cs) :
    finsetSolutionCount Cs = finsetSolutionCount Bs := by
  rcases h with ⟨S, rfl⟩
  exact S.finsetSolutionCount_mapFamily Bs

theorem SymmetryEquivalent.supportSize_eq {n : Nat} {Bs Cs : Finset (Blocker n)}
    (h : SymmetryEquivalent Bs Cs) :
    familySupportSize Cs = familySupportSize Bs := by
  rcases h with ⟨S, rfl⟩
  exact S.familySupportSize_mapFamily Bs

theorem InvariantClass.of_symmetryEquivalent {n : Nat} {Class : BlockerFamilyClass n}
    (hClass : InvariantClass Class) {Bs Cs : Finset (Blocker n)}
    (h : SymmetryEquivalent Bs Cs) :
    Class Cs ↔ Class Bs := by
  rcases h with ⟨S, rfl⟩
  exact hClass S Bs

/-- Any class depending only on the number of holes is symmetry-invariant. -/
theorem invariantClass_of_solutionCountPredicate {n : Nat} (Q : Nat → Prop) :
    InvariantClass (fun Bs : Finset (Blocker n) => Q (finsetSolutionCount Bs)) := by
  intro S Bs
  change Q (finsetSolutionCount (S.mapFamily Bs)) ↔ Q (finsetSolutionCount Bs)
  rw [S.finsetSolutionCount_mapFamily Bs]

/-- Any class depending only on the variable footprint is symmetry-invariant. -/
theorem invariantClass_of_supportSizePredicate {n : Nat} (Q : Nat → Prop) :
    InvariantClass (fun Bs : Finset (Blocker n) => Q (familySupportSize Bs)) := by
  intro S Bs
  change Q (familySupportSize (S.mapFamily Bs)) ↔ Q (familySupportSize Bs)
  rw [S.familySupportSize_mapFamily Bs]

/-- Any class depending only on the number of unused coordinates is symmetry-invariant. -/
theorem invariantClass_of_unusedVariableCountPredicate {n : Nat} (Q : Nat → Prop) :
    InvariantClass (fun Bs : Finset (Blocker n) => Q (unusedVariableCount Bs)) := by
  intro S Bs
  change Q (unusedVariableCount (S.mapFamily Bs)) ↔ Q (unusedVariableCount Bs)
  rw [S.unusedVariableCount_mapFamily Bs]

/-- Exact blocker-budget rings are symmetry-invariant. -/
theorem invariantClass_hasExactlyBlockers {n k : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FinsetHasExactlyBlockers Bs k) := by
  intro S Bs
  exact S.finsetHasExactlyBlockers_mapFamily_iff Bs

/-- The many-hole buckets are symmetry-invariant. -/
theorem invariantClass_hasAtLeastHoles {n k : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FinsetHasAtLeastHoles Bs k) :=
  invariantClass_of_solutionCountPredicate (fun holes => k ≤ holes)

/-- Exact hole-count rings are symmetry-invariant. -/
theorem invariantClass_hasExactlyHoles {n k : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FinsetHasExactlyHoles Bs k) :=
  invariantClass_of_solutionCountPredicate (fun holes => holes = k)

/-- The unique-hole bucket is symmetry-invariant. -/
theorem invariantClass_uniqueHole {n : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FinsetHasUniqueHole Bs) :=
  invariantClass_of_solutionCountPredicate (fun holes => holes = 1)

/-- The bounded variable-footprint buckets are symmetry-invariant. -/
theorem invariantClass_usesAtMostVariables {n k : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FinsetUsesAtMostVariables Bs k) :=
  invariantClass_of_supportSizePredicate (fun used => used ≤ k)

/-- The large variable-footprint buckets are symmetry-invariant. -/
theorem invariantClass_usesAtLeastVariables {n k : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FinsetUsesAtLeastVariables Bs k) :=
  invariantClass_of_supportSizePredicate (fun used => k ≤ used)

/-- Exact variable-footprint rings are symmetry-invariant. -/
theorem invariantClass_usesExactlyVariables {n k : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FinsetUsesExactlyVariables Bs k) :=
  invariantClass_of_supportSizePredicate (fun used => used = k)

/-- The full-cover/UNSAT bucket is symmetry-invariant. -/
theorem invariantClass_fullCover {n : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => FullCoverBlockers Bs) := by
  intro S Bs
  exact S.fullCoverBlockers_mapFamily_iff Bs

/-- The "all coordinates are genuinely used" envelope is symmetry-invariant. -/
theorem invariantClass_usesAllVariables {n : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => UsesAllVariables Bs) := by
  intro S Bs
  exact S.usesAllVariables_mapFamily_iff Bs

/-- The existence of a small coordinate separator is symmetry-invariant. -/
theorem invariantClass_hasCoordinateSeparatorAtMost {n k : Nat} :
    InvariantClass (fun Bs : Finset (Blocker n) => HasCoordinateSeparatorAtMost Bs k) := by
  intro S Bs
  exact S.hasCoordinateSeparatorAtMost_mapFamily_iff Bs

/-- The existence of a genuine small coordinate separator is symmetry-invariant. -/
theorem invariantClass_hasNontrivialCoordinateSeparatorAtMost {n k : Nat} :
    InvariantClass
      (fun Bs : Finset (Blocker n) => HasNontrivialCoordinateSeparatorAtMost Bs k) := by
  intro S Bs
  exact S.hasNontrivialCoordinateSeparatorAtMost_mapFamily_iff Bs

/-- The hard-side entanglement bucket "no genuine separator up to size `k`" is
symmetry-invariant. -/
theorem invariantClass_noNontrivialCoordinateSeparatorAtMost {n k : Nat} :
    InvariantClass
      (fun Bs : Finset (Blocker n) => NoNontrivialCoordinateSeparatorAtMost Bs k) := by
  intro S Bs
  exact S.noNontrivialCoordinateSeparatorAtMost_mapFamily_iff Bs

/-- A rule is a formal implication between structural classes. -/
def StructuralRule {n : Nat} (Hyp Conclusion : BlockerFamilyClass n) : Prop :=
  ∀ Bs : Finset (Blocker n), Hyp Bs → Conclusion Bs

/-- Conjunction of two structural classes. -/
def ClassAnd {n : Nat} (A B : BlockerFamilyClass n) : BlockerFamilyClass n :=
  fun Bs => A Bs ∧ B Bs

/-- Disjunction of two structural classes. -/
def ClassOr {n : Nat} (A B : BlockerFamilyClass n) : BlockerFamilyClass n :=
  fun Bs => A Bs ∨ B Bs

/-- Complement of a structural class. -/
def ClassNot {n : Nat} (A : BlockerFamilyClass n) : BlockerFamilyClass n :=
  fun Bs => ¬ A Bs

/-- Symmetry-invariant classes are closed under conjunction. -/
theorem InvariantClass.and {n : Nat} {A B : BlockerFamilyClass n}
    (hA : InvariantClass A) (hB : InvariantClass B) :
    InvariantClass (ClassAnd A B) := by
  intro S Bs
  unfold ClassAnd
  rw [hA S Bs, hB S Bs]

/-- Symmetry-invariant classes are closed under disjunction. -/
theorem InvariantClass.or {n : Nat} {A B : BlockerFamilyClass n}
    (hA : InvariantClass A) (hB : InvariantClass B) :
    InvariantClass (ClassOr A B) := by
  intro S Bs
  unfold ClassOr
  rw [hA S Bs, hB S Bs]

/-- Symmetry-invariant classes are closed under complement. -/
theorem InvariantClass.not {n : Nat} {A : BlockerFamilyClass n}
    (hA : InvariantClass A) :
    InvariantClass (ClassNot A) := by
  intro S Bs
  unfold ClassNot
  rw [hA S Bs]

/-- A structural rule can always be restricted by adding another hypothesis. -/
theorem StructuralRule.with_extra_hypothesis {n : Nat}
    {Hyp Extra Conclusion : BlockerFamilyClass n}
    (hRule : StructuralRule Hyp Conclusion) :
    StructuralRule (ClassAnd Hyp Extra) Conclusion := by
  intro Bs h
  exact hRule Bs h.1

/-- A structural rule can always be weakened by adding another possible conclusion. -/
theorem StructuralRule.weaken_conclusion_left {n : Nat}
    {Hyp Conclusion Extra : BlockerFamilyClass n}
    (hRule : StructuralRule Hyp Conclusion) :
    StructuralRule Hyp (ClassOr Conclusion Extra) := by
  intro Bs h
  exact Or.inl (hRule Bs h)

/-- Structural rules compose transitively. -/
theorem StructuralRule.trans {n : Nat} {A B C : BlockerFamilyClass n}
    (hAB : StructuralRule A B) (hBC : StructuralRule B C) :
    StructuralRule A C := by
  intro Bs hA
  exact hBC Bs (hAB Bs hA)

/-- Symmetry-invariant conclusions can be transported across symmetric copies whenever the
hypothesis is also symmetry-invariant. This is the formal "prove it once per orbit" rule. -/
theorem StructuralRule.transport_by_symmetry {n : Nat}
    {Hyp Conclusion : BlockerFamilyClass n}
    (hRule : StructuralRule Hyp Conclusion)
    (hHyp : InvariantClass Hyp) (hConclusion : InvariantClass Conclusion)
    (S : CubeSymmetry n) (Bs : Finset (Blocker n)) :
    Hyp (S.mapFamily Bs) → Conclusion (S.mapFamily Bs) := by
  intro hBs
  have hOrig : Hyp Bs := (hHyp S Bs).mp hBs
  exact (hConclusion S Bs).mpr (hRule Bs hOrig)

end CubeSymmetry

/-- Exhaustively checking every blocker against every vertex has this exact abstract cost. -/
def fullVertexScanCost (I : CoverInput) : Nat :=
  I.blockers.length * (allVertices I.n).card

theorem fullVertexScanCost_eq (I : CoverInput) :
    fullVertexScanCost I = I.blockers.length * 2 ^ I.n := by
  simp [fullVertexScanCost, allVertices_card]

/-! ## §B Clause3: bridge from ordinary 3-SAT -/

/-- A literal over variables `Fin n`. `pos = true ↦ xᵢ`, `pos = false ↦ ¬xᵢ`. -/
structure Lit (n : Nat) where
  var : Fin n
  pos : Bool

/-- A literal is satisfied by `v` when its variable takes its satisfying value. -/
def Lit.eval {n : Nat} (l : Lit n) (v : Vertex n) : Bool :=
  if l.pos then v l.var else !(v l.var)

/-- The bit that makes this literal false (the bit recorded by a blocker). -/
def Lit.falseBit {n : Nat} (l : Lit n) : Bool := !l.pos

/-- A clean 3-clause with three distinct variables. -/
structure Clause3 (n : Nat) where
  a : Lit n
  b : Lit n
  c : Lit n
  hab : a.var ≠ b.var
  hac : a.var ≠ c.var
  hbc : b.var ≠ c.var

/-- A clause is satisfied when at least one literal is satisfied. -/
def Clause3.eval {n : Nat} (C : Clause3 n) (v : Vertex n) : Bool :=
  C.a.eval v || C.b.eval v || C.c.eval v

/-- Convert a clean 3-clause into the blocker where all three literals are false. -/
def Clause3.toBlocker {n : Nat} (C : Clause3 n) : Blocker n :=
  { i := C.a.var, j := C.b.var, k := C.c.var
    hij := C.hab, hik := C.hac, hjk := C.hbc
    bi := C.a.falseBit, bj := C.b.falseBit, bk := C.c.falseBit }

/-- A formula in 3-CNF. -/
abbrev Formula3 (n : Nat) := List (Clause3 n)

/-- A formula is satisfied when every clause is satisfied. -/
def Formula3.eval {n : Nat} (F : Formula3 n) (v : Vertex n) : Bool :=
  F.all fun C => C.eval v

/-- Satisfiability of a 3-CNF formula. -/
def Formula3.Satisfiable {n : Nat} (F : Formula3 n) : Prop :=
  ∃ v : Vertex n, F.eval v = true

instance {n : Nat} (F : Formula3 n) : Decidable F.Satisfiable := by
  unfold Formula3.Satisfiable
  infer_instance

/-- Translate a formula into a hypercube-cover instance. -/
def Formula3.toCoverInput {n : Nat} (F : Formula3 n) : CoverInput :=
  { n := n, blockers := F.map Clause3.toBlocker }

/-- A literal is false iff its variable equals its false bit. -/
theorem Lit.eval_eq_false_iff {n : Nat} (l : Lit n) (v : Vertex n) :
    l.eval v = false ↔ v l.var = l.falseBit := by
  simp only [Lit.eval, Lit.falseBit]
  cases l.pos <;> cases v l.var <;> simp

/-- A clause is satisfied by `v` iff its blocker does **not** cover `v`. -/
theorem clause_eval_true_iff_not_blocked {n : Nat} (C : Clause3 n) (v : Vertex n) :
    C.eval v = true ↔ ¬ C.toBlocker.Covers v := by
  simp only [Clause3.eval, Blocker.Covers, Clause3.toBlocker, Lit.eval, Lit.falseBit]
  cases C.a.pos <;> cases C.b.pos <;> cases C.c.pos <;>
    cases v C.a.var <;> cases v C.b.var <;> cases v C.c.var <;> simp

/-- Pointwise: a vertex satisfies the formula iff it is uncovered in the translation. -/
theorem formula_eval_iff_uncovered {n : Nat} (F : Formula3 n) (v : Vertex n) :
    F.eval v = true ↔ IsUncovered (F.map Clause3.toBlocker) v := by
  constructor
  · intro h B hB
    obtain ⟨C, hCF, rfl⟩ := List.mem_map.mp hB
    exact (clause_eval_true_iff_not_blocked C v).mp (List.all_eq_true.mp h C hCF)
  · intro h
    unfold Formula3.eval
    rw [List.all_eq_true]
    intro C hCF
    exact (clause_eval_true_iff_not_blocked C v).mpr
      (h C.toBlocker (List.mem_map.mpr ⟨C, hCF, rfl⟩))

/-- The finite set of satisfying Boolean vertices of a formula. -/
def Formula3.satisfyingVertices {n : Nat} (F : Formula3 n) : Finset (Vertex n) :=
  (allVertices n).filter fun v => F.eval v = true

@[simp] theorem Formula3.mem_satisfyingVertices {n : Nat} (F : Formula3 n)
    (v : Vertex n) :
    v ∈ F.satisfyingVertices ↔ F.eval v = true := by
  simp [Formula3.satisfyingVertices]

/-- Number of satisfying Boolean vertices of a formula. -/
def Formula3.solutionCount {n : Nat} (F : Formula3 n) : Nat :=
  F.satisfyingVertices.card

/-- The formula satisfying set is exactly the uncovered set of the blocker translation. -/
theorem Formula3.satisfyingVertices_eq_uncoveredVertices {n : Nat} (F : Formula3 n) :
    F.satisfyingVertices = uncoveredVertices (F.map Clause3.toBlocker) := by
  ext v
  rw [Formula3.mem_satisfyingVertices, mem_uncoveredVertices]
  exact formula_eval_iff_uncovered F v

/-- The SAT-to-cover translation preserves the exact number of candidate witnesses. -/
theorem Formula3.solutionCount_eq_cover_solutionCount {n : Nat} (F : Formula3 n) :
    F.solutionCount = HypercubeSAT.solutionCount (F.map Clause3.toBlocker) := by
  unfold Formula3.solutionCount HypercubeSAT.solutionCount
  rw [Formula3.satisfyingVertices_eq_uncoveredVertices]

/-- The bridge: formula satisfiability coincides with `CoverSAT` of the translation. -/
theorem formula_sat_iff_cover_sat {n : Nat} (F : Formula3 n) :
    F.Satisfiable ↔ CoverSAT F.toCoverInput := by
  constructor
  · rintro ⟨v, hv⟩
    exact ⟨v, (formula_eval_iff_uncovered F v).mp hv⟩
  · rintro ⟨v, hv⟩
    exact ⟨v, (formula_eval_iff_uncovered F v).mpr hv⟩

/-! ### §B0 Assignment-cube and projection rules for 3-SAT -/

/-- The local verifier relation for a fixed 3-CNF formula and an assignment. -/
def Formula3.Check {n : Nat} (F : Formula3 n) (v : Vertex n) : Bool :=
  F.eval v

/-- Formula satisfiability is exactly existential projection over assignments. -/
theorem Formula3.satisfiable_iff_exists_check {n : Nat} (F : Formula3 n) :
    F.Satisfiable ↔ ∃ v : Vertex n, F.Check v = true := by
  rfl

/-- Abstract verifier cost: checking a candidate assignment scans one clause at a time. -/
def Formula3.verifierCost {n : Nat} (F : Formula3 n) : Nat :=
  F.length

@[simp] theorem Formula3.verifierCost_eq_length {n : Nat} (F : Formula3 n) :
    F.verifierCost = F.length := rfl

/-- The fixed-formula verifier has linear cost in the number of clauses. -/
theorem Formula3.verifierCost_le_length {n : Nat} (F : Formula3 n) :
    F.verifierCost ≤ F.length :=
  le_rfl

/-- The forbidden region of a clause is the blocker's codimension-3 subcube. -/
def Clause3.ForbiddenRegion {n : Nat} (C : Clause3 n) (v : Vertex n) : Prop :=
  C.toBlocker.Covers v

/-- The allowed region of a clause is the complement of its forbidden region. -/
def Clause3.AllowedRegion {n : Nat} (C : Clause3 n) (v : Vertex n) : Prop :=
  ¬ C.ForbiddenRegion v

/-- Clause truth is exactly membership in its allowed assignment-cube region. -/
theorem Clause3.eval_true_iff_allowed {n : Nat} (C : Clause3 n) (v : Vertex n) :
    C.eval v = true ↔ C.AllowedRegion v := by
  exact clause_eval_true_iff_not_blocked C v

/-- Clause falsity is exactly membership in its forbidden codimension-3 subcube. -/
theorem Clause3.eval_false_iff_forbidden {n : Nat} (C : Clause3 n) (v : Vertex n) :
    C.eval v = false ↔ C.ForbiddenRegion v := by
  constructor
  · intro hfalse
    by_contra hnot
    have htrue : C.eval v = true := (Clause3.eval_true_iff_allowed C v).mpr hnot
    rw [hfalse] at htrue
    contradiction
  · intro hforbid
    have hnottrue : C.eval v ≠ true := fun htrue =>
      (Clause3.eval_true_iff_allowed C v).mp htrue hforbid
    exact Bool.eq_false_of_not_eq_true hnottrue

/-- A formula is true at `v` iff `v` lies in every clause's allowed region. -/
theorem Formula3.eval_true_iff_all_allowed {n : Nat} (F : Formula3 n) (v : Vertex n) :
    F.eval v = true ↔ ∀ C ∈ F, C.AllowedRegion v := by
  constructor
  · intro h C hC
    exact (Clause3.eval_true_iff_allowed C v).mp (List.all_eq_true.mp h C hC)
  · intro h
    rw [Formula3.eval, List.all_eq_true]
    intro C hC
    exact (Clause3.eval_true_iff_allowed C v).mpr (h C hC)

/-- SAT is nonempty intersection of all clause-allowed regions. -/
theorem Formula3.sat_iff_allowed_regions_intersect {n : Nat} (F : Formula3 n) :
    F.Satisfiable ↔ ∃ v : Vertex n, ∀ C ∈ F, C.AllowedRegion v := by
  constructor
  · rintro ⟨v, hv⟩
    exact ⟨v, (F.eval_true_iff_all_allowed v).mp hv⟩
  · rintro ⟨v, hv⟩
    exact ⟨v, (F.eval_true_iff_all_allowed v).mpr hv⟩

/-- UNSAT is complete coverage of the assignment cube by forbidden clause regions. -/
theorem Formula3.unsat_iff_forall_exists_forbidden_clause {n : Nat} (F : Formula3 n) :
    ¬ F.Satisfiable ↔
      ∀ v : Vertex n, ∃ C ∈ F, C.ForbiddenRegion v := by
  constructor
  · intro hunsat v
    have hnotun : ¬ IsUncovered (F.map Clause3.toBlocker) v := by
      intro hun
      exact hunsat ⟨v, (formula_eval_iff_uncovered F v).mpr hun⟩
    unfold IsUncovered at hnotun
    push Not at hnotun
    obtain ⟨B, hB, hcover⟩ := hnotun
    obtain ⟨C, hC, rfl⟩ := List.mem_map.mp hB
    exact ⟨C, hC, hcover⟩
  · intro hcover hsat
    obtain ⟨v, hv⟩ := hsat
    obtain ⟨C, hC, hforbid⟩ := hcover v
    exact (Clause3.eval_true_iff_allowed C v).mp
      (List.all_eq_true.mp hv C hC) hforbid

/-- A formula's translated blockers fully cover the assignment cube exactly when the formula is
UNSAT. This is the assignment-hypercube cover version of 3-SAT. -/
theorem Formula3.unsat_iff_fullCoverBlockers {n : Nat} (F : Formula3 n) :
    ¬ F.Satisfiable ↔ FullCoverBlockers (F.map Clause3.toBlocker).toFinset := by
  rw [formula_sat_iff_cover_sat]
  exact coverUNSATList_iff_fullCover_toFinset (F.map Clause3.toBlocker)

/-- Formula energy at an assignment: the number of translated clause-blockers covering that
assignment, i.e. the number of violated clauses in the geometric cover view. -/
def Formula3.energy {n : Nat} (F : Formula3 n) (v : Vertex n) : Nat :=
  vertexEnergy (F.map Clause3.toBlocker) v

theorem Formula3.energy_le_length {n : Nat} (F : Formula3 n) (v : Vertex n) :
    F.energy v ≤ F.length := by
  unfold Formula3.energy
  simpa using vertexEnergy_le_length (F.map Clause3.toBlocker) v

/-- Zero energy is exactly formula satisfaction at that assignment. -/
theorem Formula3.energy_eq_zero_iff_eval_true {n : Nat} (F : Formula3 n) (v : Vertex n) :
    F.energy v = 0 ↔ F.eval v = true := by
  unfold Formula3.energy
  rw [vertexEnergy_eq_zero_iff_isUncovered, ← formula_eval_iff_uncovered]

/-- SAT means some assignment has zero geometric energy. -/
theorem Formula3.sat_iff_exists_energy_zero {n : Nat} (F : Formula3 n) :
    F.Satisfiable ↔ ∃ v : Vertex n, F.energy v = 0 := by
  constructor
  · rintro ⟨v, hv⟩
    exact ⟨v, (F.energy_eq_zero_iff_eval_true v).mpr hv⟩
  · rintro ⟨v, hv⟩
    exact ⟨v, (F.energy_eq_zero_iff_eval_true v).mp hv⟩

/-- UNSAT means every assignment has positive geometric energy. -/
theorem Formula3.unsat_iff_forall_energy_pos {n : Nat} (F : Formula3 n) :
    ¬ F.Satisfiable ↔ ∀ v : Vertex n, 0 < F.energy v := by
  rw [F.sat_iff_exists_energy_zero]
  constructor
  · intro h v
    exact Nat.pos_of_ne_zero (fun hzero => h ⟨v, hzero⟩)
  · intro h hsat
    obtain ⟨v, hv⟩ := hsat
    exact Nat.lt_irrefl 0 (hv ▸ h v)

/-- Formula-native projected SAT language: accept `F` iff some assignment passes the local
verifier `Formula3.Check`. This is the structured-object version of the formula-hypercube
projection story; an explicit bit-level encoding can refine this later. -/
def Formula3.ProjectedSAT {n : Nat} (F : Formula3 n) : Prop :=
  ∃ v : Vertex n, F.Check v = true

theorem Formula3.projectedSAT_iff_satisfiable {n : Nat} (F : Formula3 n) :
    F.ProjectedSAT ↔ F.Satisfiable := by
  rfl

/-- Minimal UNSAT for a formula, expressed through minimality of the translated full cover. -/
def Formula3.MinimalUNSATCover {n : Nat} (F : Formula3 n) : Prop :=
  MinimalFullCoverBlockers (F.map Clause3.toBlocker).toFinset

/-- Minimal UNSAT covers have private assignments: every essential translated blocker owns at
least one assignment that it alone forbids. -/
theorem Formula3.minimalUNSATCover_private_assignment_per_blocker {n : Nat}
    (F : Formula3 n) (hmin : F.MinimalUNSATCover)
    {B : Blocker n} (hB : B ∈ (F.map Clause3.toBlocker).toFinset) :
    ∃ v : Vertex n, PrivateBlockedVertex (F.map Clause3.toBlocker).toFinset B v :=
  minimal_full_cover_blockers_private_vertex (F.map Clause3.toBlocker).toFinset hmin hB

/-- SAT as a Boolean projection predicate on the formula object: the formula is accepted iff
some assignment satisfies the verifier relation. -/
noncomputable def Formula3.projectedSATPredicate {n : Nat} (F : Formula3 n) : Bool :=
  decide (∃ v : Vertex n, F.Check v = true)

theorem Formula3.projectedSATPredicate_eq_decide_satisfiable {n : Nat} (F : Formula3 n) :
    F.projectedSATPredicate = decide F.Satisfiable := by
  simp [Formula3.projectedSATPredicate, Formula3.Check, Formula3.Satisfiable]

/-! ### §B1 Algebraic encoding: clauses as polynomial equations -/

/-- A small polynomial-expression language over variables `Fin n`.

This is intentionally lighter than `MVPolynomial`: it gives us a first formal algebraic
rule layer with clean evaluation semantics, and can later be compiled to `MVPolynomial`. -/
inductive PolyExpr (n : Nat) where
  | zero : PolyExpr n
  | one : PolyExpr n
  | var : Fin n → PolyExpr n
  | add : PolyExpr n → PolyExpr n → PolyExpr n
  | sub : PolyExpr n → PolyExpr n → PolyExpr n
  | mul : PolyExpr n → PolyExpr n → PolyExpr n
  deriving DecidableEq, Repr

namespace PolyExpr

/-- Evaluate a polynomial expression under an assignment into a ring. -/
def eval {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R) : PolyExpr n → R
  | zero => 0
  | one => 1
  | var i => ρ i
  | add p q => p.eval ρ + q.eval ρ
  | sub p q => p.eval ρ - q.eval ρ
  | mul p q => p.eval ρ * q.eval ρ

/-- Boolean field/ring value: false is `0`, true is `1`. -/
def boolVal {R : Type} [Zero R] [One R] (b : Bool) : R :=
  if b then 1 else 0

/-- Interpret a Boolean vertex as a ring-valued assignment. -/
def boolAssignment {n : Nat} {R : Type} [Zero R] [One R] (v : Vertex n) : Fin n → R :=
  fun i => boolVal (v i)

/-- Algebraic negation of a Boolean variable/literal value: `1 - p`. -/
def oneMinus {n : Nat} (p : PolyExpr n) : PolyExpr n :=
  sub one p

/-- Additive negation as an expression: `0 - p`. -/
def neg {n : Nat} (p : PolyExpr n) : PolyExpr n :=
  sub zero p

/-- Boolean constraint forcing variable `i` to satisfy `xᵢ² - xᵢ = 0`. -/
def booleanConstraint {n : Nat} (i : Fin n) : PolyExpr n :=
  sub (mul (var i) (var i)) (var i)

/-- Syntactic square, used to state Boolean reduction rules. -/
def square {n : Nat} (p : PolyExpr n) : PolyExpr n :=
  mul p p

/-- Boolean reduction line: under `xᵢ² = xᵢ`, the expression `p * xᵢ² - p * xᵢ`
must vanish. This records the rule as an equation, not as a rewriting quotient. -/
def booleanSquareReductionLeft {n : Nat} (p : PolyExpr n) (i : Fin n) : PolyExpr n :=
  sub (mul p (square (var i))) (mul p (var i))

/-- Right-sided Boolean reduction line: `xᵢ² * p - xᵢ * p = 0`. -/
def booleanSquareReductionRight {n : Nat} (p : PolyExpr n) (i : Fin n) : PolyExpr n :=
  sub (mul (square (var i)) p) (mul (var i) p)

/-- Sum a list of polynomial expressions. -/
def sumList {n : Nat} : List (PolyExpr n) → PolyExpr n
  | [] => zero
  | p :: ps => add p (sumList ps)

/-- Sum of two expressions as a named algebraic rule constructor. -/
def addRule {n : Nat} (p q : PolyExpr n) : PolyExpr n :=
  add p q

/-- Difference of two expressions as a named algebraic rule constructor. -/
def subRule {n : Nat} (p q : PolyExpr n) : PolyExpr n :=
  sub p q

/-- Multiply a known equation by any expression. -/
def mulRule {n : Nat} (p q : PolyExpr n) : PolyExpr n :=
  mul p q

@[simp] theorem eval_zero {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R) :
    (zero : PolyExpr n).eval ρ = 0 := rfl

@[simp] theorem eval_one {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R) :
    (one : PolyExpr n).eval ρ = 1 := rfl

@[simp] theorem eval_var {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R) (i : Fin n) :
    (var i).eval ρ = ρ i := rfl

@[simp] theorem eval_add {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R)
    (p q : PolyExpr n) :
    (add p q).eval ρ = p.eval ρ + q.eval ρ := rfl

@[simp] theorem eval_sub {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R)
    (p q : PolyExpr n) :
    (sub p q).eval ρ = p.eval ρ - q.eval ρ := rfl

@[simp] theorem eval_mul {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R)
    (p q : PolyExpr n) :
    (mul p q).eval ρ = p.eval ρ * q.eval ρ := rfl

@[simp] theorem eval_oneMinus {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R)
    (p : PolyExpr n) :
    (oneMinus p).eval ρ = 1 - p.eval ρ := rfl

@[simp] theorem eval_neg {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R)
    (p : PolyExpr n) :
    (neg p).eval ρ = 0 - p.eval ρ := rfl

@[simp] theorem eval_square {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R)
    (p : PolyExpr n) :
    (square p).eval ρ = p.eval ρ * p.eval ρ := rfl

@[simp] theorem eval_sumList {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R)
    (ps : List (PolyExpr n)) :
    (sumList ps).eval ρ = (ps.map fun p => p.eval ρ).sum := by
  induction ps with
  | nil =>
      rfl
  | cons p ps ih =>
      simp [sumList, ih]

/-! #### Information accountants for algebraic expressions -/

/-- Syntax-tree size of a polynomial expression. This is a proof-size/information budget
proxy before quotienting by algebraic identities. -/
def size {n : Nat} : PolyExpr n → Nat
  | zero => 1
  | one => 1
  | var _ => 1
  | add p q => p.size + q.size + 1
  | sub p q => p.size + q.size + 1
  | mul p q => p.size + q.size + 1

/-- Total degree upper bound read from syntax. Addition/subtraction take max; multiplication
adds degrees. -/
def degree {n : Nat} : PolyExpr n → Nat
  | zero => 0
  | one => 0
  | var _ => 1
  | add p q => max p.degree q.degree
  | sub p q => max p.degree q.degree
  | mul p q => p.degree + q.degree

/-- Variables that syntactically occur in a polynomial expression. -/
def supportVars {n : Nat} : PolyExpr n → Finset (Fin n)
  | zero => ∅
  | one => ∅
  | var i => {i}
  | add p q => p.supportVars ∪ q.supportVars
  | sub p q => p.supportVars ∪ q.supportVars
  | mul p q => p.supportVars ∪ q.supportVars

/-- Number of variables mentioned by the expression: an information footprint. -/
def supportSize {n : Nat} (p : PolyExpr n) : Nat :=
  p.supportVars.card

/-- A compact multi-axis algebraic information profile. -/
structure InfoProfile (n : Nat) where
  size : Nat
  degree : Nat
  supportSize : Nat
  deriving DecidableEq, Repr

/-- Information profile of one polynomial expression. -/
def infoProfile {n : Nat} (p : PolyExpr n) : InfoProfile n where
  size := p.size
  degree := p.degree
  supportSize := p.supportSize

@[simp] theorem size_pos {n : Nat} (p : PolyExpr n) : 0 < p.size := by
  induction p <;> simp [size, *]

theorem supportSize_le_dimension {n : Nat} (p : PolyExpr n) :
    p.supportSize ≤ n := by
  unfold supportSize
  have hsub : p.supportVars ⊆ Finset.univ := by
    intro i hi
    exact Finset.mem_univ i
  have hle := Finset.card_le_card hsub
  simpa [Fintype.card_fin] using hle

@[simp] theorem degree_oneMinus {n : Nat} (p : PolyExpr n) :
    (oneMinus p).degree = p.degree := by
  simp [oneMinus, degree]

@[simp] theorem supportVars_oneMinus {n : Nat} (p : PolyExpr n) :
    (oneMinus p).supportVars = p.supportVars := by
  simp [oneMinus, supportVars]

@[simp] theorem degree_neg {n : Nat} (p : PolyExpr n) :
    (neg p).degree = p.degree := by
  simp [neg, degree]

@[simp] theorem supportVars_neg {n : Nat} (p : PolyExpr n) :
    (neg p).supportVars = p.supportVars := by
  simp [neg, supportVars]

@[simp] theorem size_neg {n : Nat} (p : PolyExpr n) :
    (neg p).size = p.size + 2 := by
  change 1 + p.size + 1 = p.size + 2
  ring

theorem supportSize_add_le {n : Nat} (p q : PolyExpr n) :
    (add p q).supportSize ≤ p.supportSize + q.supportSize := by
  simpa [supportSize, supportVars] using Finset.card_union_le p.supportVars q.supportVars

theorem supportSize_sub_le {n : Nat} (p q : PolyExpr n) :
    (sub p q).supportSize ≤ p.supportSize + q.supportSize := by
  simpa [supportSize, supportVars] using Finset.card_union_le p.supportVars q.supportVars

theorem supportSize_mul_le {n : Nat} (p q : PolyExpr n) :
    (mul p q).supportSize ≤ p.supportSize + q.supportSize := by
  simpa [supportSize, supportVars] using Finset.card_union_le p.supportVars q.supportVars

/-- Locality principle: a polynomial expression only reads coordinates in its support.
This makes `supportVars` a semantic information boundary, not just a syntactic counter. -/
theorem eval_eq_of_eq_on_support {n : Nat} {R : Type} [Ring R]
    {ρ σ : Fin n → R} :
    ∀ p : PolyExpr n,
      (∀ i, i ∈ p.supportVars → ρ i = σ i) → p.eval ρ = p.eval σ
  | zero, _ => rfl
  | one, _ => rfl
  | var i, h => h i (by simp [supportVars])
  | add p q, h => by
      have hp : p.eval ρ = p.eval σ := eval_eq_of_eq_on_support p (by
        intro i hi
        exact h i (by simp [supportVars, hi]))
      have hq : q.eval ρ = q.eval σ := eval_eq_of_eq_on_support q (by
        intro i hi
        exact h i (by simp [supportVars, hi]))
      simp [eval, hp, hq]
  | sub p q, h => by
      have hp : p.eval ρ = p.eval σ := eval_eq_of_eq_on_support p (by
        intro i hi
        exact h i (by simp [supportVars, hi]))
      have hq : q.eval ρ = q.eval σ := eval_eq_of_eq_on_support q (by
        intro i hi
        exact h i (by simp [supportVars, hi]))
      simp [eval, hp, hq]
  | mul p q, h => by
      have hp : p.eval ρ = p.eval σ := eval_eq_of_eq_on_support p (by
        intro i hi
        exact h i (by simp [supportVars, hi]))
      have hq : q.eval ρ = q.eval σ := eval_eq_of_eq_on_support q (by
        intro i hi
        exact h i (by simp [supportVars, hi]))
      simp [eval, hp, hq]

/-- If no variables occur in an expression, its value is independent of the assignment. -/
theorem eval_eq_of_supportVars_eq_empty {n : Nat} {R : Type} [Ring R]
    {ρ σ : Fin n → R} {p : PolyExpr n} (hsupp : p.supportVars = ∅) :
    p.eval ρ = p.eval σ := by
  exact eval_eq_of_eq_on_support p (by
    intro i hi
    rw [hsupp] at hi
    simp at hi)

theorem degree_add_le {n : Nat} (p q : PolyExpr n) :
    (add p q).degree ≤ max p.degree q.degree := by
  rfl

theorem degree_sub_le {n : Nat} (p q : PolyExpr n) :
    (sub p q).degree ≤ max p.degree q.degree := by
  rfl

theorem degree_mul_eq {n : Nat} (p q : PolyExpr n) :
    (mul p q).degree = p.degree + q.degree := by
  rfl

theorem size_add_eq {n : Nat} (p q : PolyExpr n) :
    (add p q).size = p.size + q.size + 1 := by
  rfl

theorem size_sub_eq {n : Nat} (p q : PolyExpr n) :
    (sub p q).size = p.size + q.size + 1 := by
  rfl

theorem size_mul_eq {n : Nat} (p q : PolyExpr n) :
    (mul p q).size = p.size + q.size + 1 := by
  rfl

@[simp] theorem degree_square {n : Nat} (p : PolyExpr n) :
    (square p).degree = 2 * p.degree := by
  simp [square, degree, Nat.two_mul]

@[simp] theorem size_square {n : Nat} (p : PolyExpr n) :
    (square p).size = 2 * p.size + 1 := by
  simp [square, size, Nat.two_mul]

theorem booleanSquareReductionLeft_degree_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanSquareReductionLeft p i).degree ≤ p.degree + 2 := by
  simp [booleanSquareReductionLeft, square, degree, Nat.add_comm, Nat.add_left_comm, Nat.add_assoc]

theorem booleanSquareReductionRight_degree_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanSquareReductionRight p i).degree ≤ p.degree + 2 := by
  simp [booleanSquareReductionRight, square, degree, Nat.add_comm, Nat.add_left_comm, Nat.add_assoc]

@[simp] theorem booleanSquareReductionLeft_size {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanSquareReductionLeft p i).size = 2 * p.size + 7 := by
  simp [booleanSquareReductionLeft, square, size]
  omega

@[simp] theorem booleanSquareReductionRight_size {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanSquareReductionRight p i).size = 2 * p.size + 7 := by
  simp [booleanSquareReductionRight, square, size]
  omega

@[simp] theorem supportVars_booleanSquareReductionLeft {n : Nat} (p : PolyExpr n)
    (i : Fin n) :
    (booleanSquareReductionLeft p i).supportVars = p.supportVars ∪ {i} := by
  simp [booleanSquareReductionLeft, square, supportVars, Finset.union_assoc,
    Finset.union_left_comm, Finset.union_comm]

@[simp] theorem supportVars_booleanSquareReductionRight {n : Nat} (p : PolyExpr n)
    (i : Fin n) :
    (booleanSquareReductionRight p i).supportVars = p.supportVars ∪ {i} := by
  simp [booleanSquareReductionRight, square, supportVars, Finset.union_assoc,
    Finset.union_left_comm, Finset.union_comm]

theorem booleanSquareReductionLeft_supportSize_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanSquareReductionLeft p i).supportSize ≤ p.supportSize + 1 := by
  simpa [supportSize] using Finset.card_union_le p.supportVars ({i} : Finset (Fin n))

theorem booleanSquareReductionRight_supportSize_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanSquareReductionRight p i).supportSize ≤ p.supportSize + 1 := by
  simpa [supportSize] using Finset.card_union_le p.supportVars ({i} : Finset (Fin n))

/-- Boolean assignments satisfy `x² - x = 0`. -/
theorem booleanConstraint_eval_boolAssignment {n : Nat} {R : Type} [Ring R]
    (v : Vertex n) (i : Fin n) :
    (booleanConstraint i).eval (boolAssignment (R := R) v) = 0 := by
  cases hv : v i <;>
    simp [booleanConstraint, boolAssignment, boolVal, eval, hv]

/-- Over an integral domain, `x² - x = 0` forces `x = 0` or `x = 1`. -/
theorem eq_zero_or_eq_one_of_booleanConstraint {R : Type} [CommRing R] [IsDomain R]
    {x : R} (h : x * x - x = 0) :
    x = 0 ∨ x = 1 := by
  have hmul : x * (x - 1) = 0 := by
    rw [← h]
    ring
  rcases mul_eq_zero.mp hmul with hx | hx
  · exact Or.inl hx
  · exact Or.inr (sub_eq_zero.mp hx)

end PolyExpr

/-- Algebraic falsity indicator for a literal: it evaluates to `1` when the literal is false
on a Boolean assignment, and `0` when the literal is true. -/
def Lit.falsePoly {n : Nat} (l : Lit n) : PolyExpr n :=
  if l.pos then PolyExpr.oneMinus (PolyExpr.var l.var) else PolyExpr.var l.var

/-- Algebraic clause polynomial: product of the three literal-falsity indicators. -/
def Clause3.falsePoly {n : Nat} (C : Clause3 n) : PolyExpr n :=
  PolyExpr.mul (PolyExpr.mul C.a.falsePoly C.b.falsePoly) C.c.falsePoly

/-- Boolean constraints for all variables, represented as an equation predicate. -/
def BooleanPolynomialConstraints {n : Nat} {R : Type} [Ring R] (ρ : Fin n → R) : Prop :=
  ∀ i : Fin n, (PolyExpr.booleanConstraint i).eval ρ = 0

/-- A Boolean polynomial constraint is exactly the idempotence law `xᵢ² = xᵢ`. -/
theorem boolean_square_of_constraints {n : Nat} {R : Type} [Ring R]
    {ρ : Fin n → R} (hbool : BooleanPolynomialConstraints ρ) (i : Fin n) :
    ρ i * ρ i = ρ i := by
  exact sub_eq_zero.mp (hbool i)

/-- Boolean vertices are idempotent in every target ring. -/
theorem boolAssignment_square {n : Nat} {R : Type} [Ring R] (v : Vertex n) (i : Fin n) :
    PolyExpr.boolAssignment (R := R) v i * PolyExpr.boolAssignment (R := R) v i =
      PolyExpr.boolAssignment (R := R) v i := by
  cases hv : v i <;>
    simp [PolyExpr.boolAssignment, PolyExpr.boolVal, hv]

/-- Left Boolean reduction is sound under Boolean polynomial constraints. -/
theorem booleanSquareReductionLeft_eval_of_constraints {n : Nat} {R : Type} [Ring R]
    {ρ : Fin n → R} (hbool : BooleanPolynomialConstraints ρ)
    (p : PolyExpr n) (i : Fin n) :
    (PolyExpr.booleanSquareReductionLeft p i).eval ρ = 0 := by
  have hsq : ρ i * ρ i = ρ i := boolean_square_of_constraints hbool i
  simp [PolyExpr.booleanSquareReductionLeft, PolyExpr.square, PolyExpr.eval, hsq]

/-- Right Boolean reduction is sound under Boolean polynomial constraints. -/
theorem booleanSquareReductionRight_eval_of_constraints {n : Nat} {R : Type} [Ring R]
    {ρ : Fin n → R} (hbool : BooleanPolynomialConstraints ρ)
    (p : PolyExpr n) (i : Fin n) :
    (PolyExpr.booleanSquareReductionRight p i).eval ρ = 0 := by
  have hsq : ρ i * ρ i = ρ i := boolean_square_of_constraints hbool i
  simp [PolyExpr.booleanSquareReductionRight, PolyExpr.square, PolyExpr.eval, hsq]

/-- Clause equations for all clauses of a 3-CNF formula. -/
def Formula3.ClausePolynomialConstraints {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (ρ : Fin n → R) : Prop :=
  ∀ C ∈ F, C.falsePoly.eval ρ = 0

/-- Full algebraic solution relation: Boolean equations plus all clause equations. -/
def Formula3.AlgebraicSolution {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (ρ : Fin n → R) : Prop :=
  BooleanPolynomialConstraints ρ ∧ F.ClausePolynomialConstraints ρ

/-- Boolean-vertex algebraic solution: variables are explicitly supplied by a cube vertex. -/
def Formula3.BooleanAlgebraicSolution {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (v : Vertex n) : Prop :=
  F.AlgebraicSolution (PolyExpr.boolAssignment (R := R) v)

@[simp] theorem Lit.falsePoly_degree {n : Nat} (l : Lit n) :
    l.falsePoly.degree = 1 := by
  cases hp : l.pos <;>
    simp [Lit.falsePoly, hp, PolyExpr.oneMinus, PolyExpr.degree]

@[simp] theorem Lit.falsePoly_supportVars {n : Nat} (l : Lit n) :
    l.falsePoly.supportVars = {l.var} := by
  cases hp : l.pos <;>
    simp [Lit.falsePoly, hp, PolyExpr.oneMinus, PolyExpr.supportVars]

theorem Lit.falsePoly_supportSize_le_one {n : Nat} (l : Lit n) :
    l.falsePoly.supportSize ≤ 1 := by
  simp [PolyExpr.supportSize]

theorem Lit.falsePoly_size_le_three {n : Nat} (l : Lit n) :
    l.falsePoly.size ≤ 3 := by
  cases hp : l.pos <;>
    simp [Lit.falsePoly, hp, PolyExpr.oneMinus, PolyExpr.size]

@[simp] theorem Clause3.falsePoly_degree {n : Nat} (C : Clause3 n) :
    C.falsePoly.degree = 3 := by
  simp [Clause3.falsePoly, PolyExpr.degree]

theorem Clause3.falsePoly_supportSize_le_three {n : Nat} (C : Clause3 n) :
    C.falsePoly.supportSize ≤ 3 := by
  have hmul :
      (PolyExpr.mul C.a.falsePoly C.b.falsePoly).supportSize ≤
        C.a.falsePoly.supportSize + C.b.falsePoly.supportSize :=
    PolyExpr.supportSize_mul_le C.a.falsePoly C.b.falsePoly
  have hclause :
      C.falsePoly.supportSize ≤
        (PolyExpr.mul C.a.falsePoly C.b.falsePoly).supportSize +
          C.c.falsePoly.supportSize := by
    simpa [Clause3.falsePoly] using
      PolyExpr.supportSize_mul_le (PolyExpr.mul C.a.falsePoly C.b.falsePoly) C.c.falsePoly
  have ha : C.a.falsePoly.supportSize ≤ 1 := C.a.falsePoly_supportSize_le_one
  have hb : C.b.falsePoly.supportSize ≤ 1 := C.b.falsePoly_supportSize_le_one
  have hc : C.c.falsePoly.supportSize ≤ 1 := C.c.falsePoly_supportSize_le_one
  calc
    C.falsePoly.supportSize
        ≤ (PolyExpr.mul C.a.falsePoly C.b.falsePoly).supportSize +
            C.c.falsePoly.supportSize := hclause
    _ ≤ (C.a.falsePoly.supportSize + C.b.falsePoly.supportSize) +
            C.c.falsePoly.supportSize := Nat.add_le_add_right hmul _
    _ ≤ (1 + 1) + 1 := Nat.add_le_add (Nat.add_le_add ha hb) hc
    _ = 3 := rfl

theorem Clause3.falsePoly_size_le_eleven {n : Nat} (C : Clause3 n) :
    C.falsePoly.size ≤ 11 := by
  have ha : C.a.falsePoly.size ≤ 3 := C.a.falsePoly_size_le_three
  have hb : C.b.falsePoly.size ≤ 3 := C.b.falsePoly_size_le_three
  have hc : C.c.falsePoly.size ≤ 3 := C.c.falsePoly_size_le_three
  simp [Clause3.falsePoly, PolyExpr.size]
  omega

@[simp] theorem PolyExpr.booleanConstraint_degree {n : Nat} (i : Fin n) :
    (PolyExpr.booleanConstraint i).degree = 2 := by
  simp [PolyExpr.booleanConstraint, PolyExpr.degree]

@[simp] theorem PolyExpr.booleanConstraint_supportVars {n : Nat} (i : Fin n) :
    (PolyExpr.booleanConstraint i).supportVars = {i} := by
  simp [PolyExpr.booleanConstraint, PolyExpr.supportVars]

theorem PolyExpr.booleanConstraint_supportSize_le_one {n : Nat} (i : Fin n) :
    (PolyExpr.booleanConstraint i).supportSize ≤ 1 := by
  simp [PolyExpr.supportSize]

@[simp] theorem PolyExpr.booleanConstraint_size {n : Nat} (i : Fin n) :
    (PolyExpr.booleanConstraint i).size = 5 := by
  simp [PolyExpr.booleanConstraint, PolyExpr.size]

/-- Integer-valued Boolean evaluation of a literal falsity polynomial vanishes exactly when
the literal is true. Integer coefficients are the stable bridge; finite-field versions can be
added by changing the target ring. -/
theorem lit_falsePoly_eval_boolInt_eq_zero_iff {n : Nat}
    (l : Lit n) (v : Vertex n) :
    l.falsePoly.eval (PolyExpr.boolAssignment (R := Int) v) = 0 ↔ l.eval v = true := by
  cases hp : l.pos <;> cases hv : v l.var <;>
    simp [Lit.falsePoly, Lit.eval, PolyExpr.boolAssignment, PolyExpr.boolVal,
      PolyExpr.oneMinus, PolyExpr.eval, hp, hv]

/-- Integer-valued Boolean evaluation of a literal falsity polynomial is one exactly when
the literal is false. -/
theorem lit_falsePoly_eval_boolInt_eq_one_iff {n : Nat}
    (l : Lit n) (v : Vertex n) :
    l.falsePoly.eval (PolyExpr.boolAssignment (R := Int) v) = 1 ↔ l.eval v = false := by
  cases hp : l.pos <;> cases hv : v l.var <;>
    simp [Lit.falsePoly, Lit.eval, PolyExpr.boolAssignment, PolyExpr.boolVal,
      PolyExpr.oneMinus, PolyExpr.eval, hp, hv]

/-- The clause polynomial vanishes exactly when the clause is satisfied, on Boolean vertices. -/
theorem clause_falsePoly_eval_boolInt_eq_zero_iff {n : Nat}
    (C : Clause3 n) (v : Vertex n) :
    C.falsePoly.eval (PolyExpr.boolAssignment (R := Int) v) = 0 ↔ C.eval v = true := by
  cases ha : C.a.pos <;> cases hb : C.b.pos <;> cases hc : C.c.pos <;>
    cases hva : v C.a.var <;> cases hvb : v C.b.var <;> cases hvc : v C.c.var <;>
      simp [Clause3.falsePoly, Clause3.eval, Lit.falsePoly, Lit.eval,
        PolyExpr.boolAssignment, PolyExpr.boolVal, PolyExpr.oneMinus, PolyExpr.eval,
        ha, hb, hc, hva, hvb, hvc]

/-- Clause-polynomial equation form of the blocker bridge. -/
theorem clause_falsePoly_eval_bool_eq_zero_iff_not_blocked {n : Nat}
    (C : Clause3 n) (v : Vertex n) :
    C.falsePoly.eval (PolyExpr.boolAssignment (R := Int) v) = 0 ↔
      ¬ C.toBlocker.Covers v := by
  rw [clause_falsePoly_eval_boolInt_eq_zero_iff, clause_eval_true_iff_not_blocked]

/-- A Boolean vertex satisfies all polynomial equations of a formula exactly when it satisfies
the original 3-CNF formula. -/
theorem formula_booleanAlgebraicSolution_iff_eval {n : Nat}
    (F : Formula3 n) (v : Vertex n) :
    F.BooleanAlgebraicSolution (R := Int) v ↔ F.eval v = true := by
  unfold Formula3.BooleanAlgebraicSolution Formula3.AlgebraicSolution
    BooleanPolynomialConstraints Formula3.ClausePolynomialConstraints
  constructor
  · intro h
    unfold Formula3.eval
    rw [List.all_eq_true]
    intro C hC
    exact (clause_falsePoly_eval_boolInt_eq_zero_iff C v).mp (h.2 C hC)
  · intro h
    constructor
    · intro i
      exact PolyExpr.booleanConstraint_eval_boolAssignment (R := Int) v i
    · intro C hC
      exact (clause_falsePoly_eval_boolInt_eq_zero_iff C v).mpr
        (List.all_eq_true.mp h C hC)

/-- 3-SAT satisfiability iff the algebraic encoding has a Boolean-vertex solution. -/
theorem formula_sat_iff_exists_boolean_algebraic_solution {n : Nat}
    (F : Formula3 n) :
    F.Satisfiable ↔ ∃ v : Vertex n, F.BooleanAlgebraicSolution (R := Int) v := by
  constructor
  · rintro ⟨v, hv⟩
    exact ⟨v, (formula_booleanAlgebraicSolution_iff_eval F v).mpr hv⟩
  · rintro ⟨v, hv⟩
    exact ⟨v, (formula_booleanAlgebraicSolution_iff_eval F v).mp hv⟩

/-- The finite set of Boolean vertices solving the algebraic encoding over `Int`. -/
noncomputable def Formula3.booleanAlgebraicSolutionVertices {n : Nat}
    (F : Formula3 n) : Finset (Vertex n) := by
  classical
  exact (allVertices n).filter fun v => F.BooleanAlgebraicSolution (R := Int) v

@[simp] theorem Formula3.mem_booleanAlgebraicSolutionVertices {n : Nat}
    (F : Formula3 n) (v : Vertex n) :
    v ∈ F.booleanAlgebraicSolutionVertices ↔ F.BooleanAlgebraicSolution (R := Int) v := by
  classical
  simp [Formula3.booleanAlgebraicSolutionVertices]

/-- Algebraic encoding preserves the exact Boolean solution set, not only existence. -/
theorem Formula3.booleanAlgebraicSolutionVertices_eq_satisfyingVertices {n : Nat}
    (F : Formula3 n) :
    F.booleanAlgebraicSolutionVertices = F.satisfyingVertices := by
  ext v
  rw [Formula3.mem_booleanAlgebraicSolutionVertices, Formula3.mem_satisfyingVertices]
  exact formula_booleanAlgebraicSolution_iff_eval F v

/-- Count preservation for the Boolean algebraic encoding. -/
theorem Formula3.booleanAlgebraicSolutionVertices_card {n : Nat} (F : Formula3 n) :
    F.booleanAlgebraicSolutionVertices.card = F.solutionCount := by
  unfold Formula3.solutionCount
  rw [Formula3.booleanAlgebraicSolutionVertices_eq_satisfyingVertices]

/-- A polynomial system is a semantic list of equations `p = 0`. -/
structure PolySystem (n : Nat) where
  equations : List (PolyExpr n)

/-- The full polynomial system generated by a 3-CNF formula: all Boolean axioms followed by
all clause falsity-product equations. -/
noncomputable def Formula3.toPolySystem {n : Nat} (F : Formula3 n) : PolySystem n := by
  classical
  exact
    { equations :=
        ((Finset.univ : Finset (Fin n)).toList.map PolyExpr.booleanConstraint) ++
          F.map Clause3.falsePoly }

theorem Formula3.booleanConstraint_mem_toPolySystem {n : Nat} (F : Formula3 n)
    (i : Fin n) :
    PolyExpr.booleanConstraint i ∈ F.toPolySystem.equations := by
  classical
  change PolyExpr.booleanConstraint i ∈
    ((Finset.univ : Finset (Fin n)).toList.map PolyExpr.booleanConstraint) ++
      F.map Clause3.falsePoly
  rw [List.mem_append]
  left
  exact List.mem_map.mpr ⟨i, by simp, rfl⟩

theorem Formula3.clauseFalsePoly_mem_toPolySystem {n : Nat} {F : Formula3 n}
    {C : Clause3 n} (hC : C ∈ F) :
    C.falsePoly ∈ F.toPolySystem.equations := by
  classical
  change C.falsePoly ∈
    ((Finset.univ : Finset (Fin n)).toList.map PolyExpr.booleanConstraint) ++
      F.map Clause3.falsePoly
  rw [List.mem_append]
  right
  exact List.mem_map.mpr ⟨C, hC, rfl⟩

namespace PolySystem

/-- List helper for maximum-style degree accountants. -/
theorem list_foldl_max_le {xs : List Nat} {acc d : Nat}
    (hacc : acc ≤ d) (hxs : ∀ x ∈ xs, x ≤ d) :
    xs.foldl max acc ≤ d := by
  induction xs generalizing acc with
  | nil =>
      simpa using hacc
  | cons x xs ih =>
      apply ih
      · exact max_le hacc (hxs x (by simp))
      · intro y hy
        exact hxs y (by simp [hy])

/-- If every listed measurement is bounded by `c`, the sum is bounded by length times `c`. -/
theorem list_sum_map_le_length_mul_of_forall {α : Type} {xs : List α} {f : α → Nat}
    {c : Nat} (h : ∀ x ∈ xs, f x ≤ c) :
    (xs.map f).sum ≤ xs.length * c := by
  induction xs with
  | nil =>
      simp
  | cons x xs ih =>
      have hx : f x ≤ c := h x (by simp)
      have hxs : ∀ y ∈ xs, f y ≤ c := by
        intro y hy
        exact h y (by simp [hy])
      have htail := ih hxs
      simp [Nat.succ_mul]
      omega

/-- Exact list-sum form when every listed measurement is the same constant. -/
theorem list_sum_map_eq_length_mul_of_forall {α : Type} {xs : List α} {f : α → Nat}
    {c : Nat} (h : ∀ x ∈ xs, f x = c) :
    (xs.map f).sum = xs.length * c := by
  induction xs with
  | nil =>
      simp
  | cons x xs ih =>
      have hx : f x = c := h x (by simp)
      have hxs : ∀ y ∈ xs, f y = c := by
        intro y hy
        exact h y (by simp [hy])
      simp [hx, ih hxs, Nat.succ_mul]
      omega

/-- Number of equations: a raw information/certificate length counter. -/
def equationCount {n : Nat} (S : PolySystem n) : Nat :=
  S.equations.length

@[simp] theorem formula_toPolySystem_equationCount {n : Nat} (F : Formula3 n) :
    F.toPolySystem.equationCount = n + F.length := by
  classical
  simp [Formula3.toPolySystem, equationCount]

/-- Sum of expression-tree sizes across the system. -/
def totalSize {n : Nat} (S : PolySystem n) : Nat :=
  (S.equations.map PolyExpr.size).sum

/-- Maximum syntactic degree among equations. -/
def maxDegree {n : Nat} (S : PolySystem n) : Nat :=
  (S.equations.map PolyExpr.degree).foldl max 0

theorem maxDegree_le_of_forall {n : Nat} (S : PolySystem n) {d : Nat}
    (h : ∀ p ∈ S.equations, p.degree ≤ d) :
    S.maxDegree ≤ d := by
  unfold maxDegree
  apply list_foldl_max_le (Nat.zero_le d)
  intro deg hdeg
  rcases List.mem_map.mp hdeg with ⟨p, hp, rfl⟩
  exact h p hp

theorem totalSize_le_of_forall {n : Nat} (S : PolySystem n) {c : Nat}
    (h : ∀ p ∈ S.equations, p.size ≤ c) :
    S.totalSize ≤ S.equationCount * c := by
  unfold totalSize equationCount
  exact list_sum_map_le_length_mul_of_forall h

/-- The SAT-to-algebra translation has degree at most three everywhere. -/
theorem formula_toPolySystem_maxDegree_le_three {n : Nat} (F : Formula3 n) :
    F.toPolySystem.maxDegree ≤ 3 := by
  classical
  apply maxDegree_le_of_forall
  intro p hp
  change p ∈
    ((Finset.univ : Finset (Fin n)).toList.map PolyExpr.booleanConstraint) ++
      F.map Clause3.falsePoly at hp
  rw [List.mem_append] at hp
  rcases hp with hp | hp
  · rcases List.mem_map.mp hp with ⟨i, _hi, rfl⟩
    simp
  · rcases List.mem_map.mp hp with ⟨C, _hC, rfl⟩
    simp

/-- Static information budget: Boolean axioms cost size `5` each and clean 3-clause
equations cost at most `11` symbols each in the current expression language. -/
theorem formula_toPolySystem_totalSize_le {n : Nat} (F : Formula3 n) :
    F.toPolySystem.totalSize ≤ 5 * n + 11 * F.length := by
  classical
  unfold totalSize Formula3.toPolySystem
  simp only [List.map_append, List.sum_append, List.map_map]
  have hbool :
      (((Finset.univ : Finset (Fin n)).toList.map
          (fun i => (PolyExpr.booleanConstraint i).size)).sum) =
        ((Finset.univ : Finset (Fin n)).toList.length) * 5 := by
    exact list_sum_map_eq_length_mul_of_forall (xs := (Finset.univ : Finset (Fin n)).toList)
      (f := fun i => (PolyExpr.booleanConstraint i).size) (c := 5) (by intro i _; simp)
  have hclauses :
      ((F.map fun C => C.falsePoly.size).sum) ≤ F.length * 11 := by
    exact list_sum_map_le_length_mul_of_forall (xs := F)
      (f := fun C => C.falsePoly.size) (c := 11)
      (by intro C _; exact C.falsePoly_size_le_eleven)
  change
    (((Finset.univ : Finset (Fin n)).toList.map
        (fun i => (PolyExpr.booleanConstraint i).size)).sum) +
      (F.map fun C => C.falsePoly.size).sum ≤ 5 * n + 11 * F.length
  rw [hbool]
  have hlen : ((Finset.univ : Finset (Fin n)).toList.length) = n := by
    simp
  rw [hlen]
  omega

/-- Variables mentioned anywhere in the system. -/
def supportVars {n : Nat} (S : PolySystem n) : Finset (Fin n) :=
  S.equations.foldl (fun acc p => acc ∪ p.supportVars) ∅

/-- Information footprint of a polynomial system, measured by variable support. -/
def supportSize {n : Nat} (S : PolySystem n) : Nat :=
  S.supportVars.card

/-- Multi-axis information profile of a polynomial system. -/
structure InfoProfile (n : Nat) where
  equations : Nat
  totalSize : Nat
  maxDegree : Nat
  supportSize : Nat
  deriving DecidableEq, Repr

/-- Compute all basic information counters of a system. -/
def infoProfile {n : Nat} (S : PolySystem n) : InfoProfile n where
  equations := S.equationCount
  totalSize := S.totalSize
  maxDegree := S.maxDegree
  supportSize := S.supportSize

theorem supportSize_le_dimension {n : Nat} (S : PolySystem n) :
    S.supportSize ≤ n := by
  unfold supportSize
  have hsub : S.supportVars ⊆ Finset.univ := by
    intro i hi
    exact Finset.mem_univ i
  have hle := Finset.card_le_card hsub
  simpa [Fintype.card_fin] using hle

/-- An assignment solves every equation in a polynomial system. -/
def Solution {n : Nat} {R : Type} [Ring R] (S : PolySystem n) (ρ : Fin n → R) : Prop :=
  ∀ p ∈ S.equations, p.eval ρ = 0

/-- System locality: to preserve being a solution, it is enough to preserve every equation
on the coordinates that equation actually mentions. -/
theorem solution_of_eq_on_equation_supports {n : Nat} {R : Type} [Ring R]
    {S : PolySystem n} {ρ σ : Fin n → R} (hσ : S.Solution σ)
    (heq : ∀ p ∈ S.equations, ∀ i, i ∈ p.supportVars → ρ i = σ i) :
    S.Solution ρ := by
  intro p hp
  have hval : p.eval ρ = p.eval σ := PolyExpr.eval_eq_of_eq_on_support p (heq p hp)
  rw [hval]
  exact hσ p hp

/-- A polynomial is a semantic consequence of a system if it vanishes on every solution. -/
def Consequence {n : Nat} {R : Type} [Ring R] (S : PolySystem n) (p : PolyExpr n) : Prop :=
  ∀ ρ : Fin n → R, S.Solution ρ → p.eval ρ = 0

/-- A semantic algebraic refutation: the constant `1` is forced to vanish. -/
def Refutes {n : Nat} {R : Type} [Ring R] (S : PolySystem n) : Prop :=
  S.Consequence (R := R) PolyExpr.one

/-! #### Zero loci: the algebraic-geometry view -/

/-- The common-zero set of a polynomial system over a target ring. -/
def ZeroLocus {n : Nat} {R : Type} [Ring R] (S : PolySystem n) : Set (Fin n → R) :=
  {ρ | S.Solution ρ}

@[simp] theorem mem_zeroLocus {n : Nat} {R : Type} [Ring R]
    (S : PolySystem n) (ρ : Fin n → R) :
    ρ ∈ S.ZeroLocus ↔ S.Solution ρ := by
  rfl

/-- A polynomial vanishes on a set of assignments. -/
def VanishesOn {n : Nat} {R : Type} [Ring R]
    (X : Set (Fin n → R)) (p : PolyExpr n) : Prop :=
  ∀ ρ ∈ X, p.eval ρ = 0

/-- Adding equations reverses inclusion of zero loci. -/
theorem zeroLocus_anti_mono {n : Nat} {R : Type} [Ring R]
    {S T : PolySystem n}
    (hsub : ∀ p, p ∈ S.equations → p ∈ T.equations) :
    T.ZeroLocus (R := R) ⊆ S.ZeroLocus := by
  intro ρ hT p hp
  exact hT p (hsub p hp)

/-- The zero locus is empty exactly when there is no solution. -/
theorem zeroLocus_empty_iff_no_solution {n : Nat} {R : Type} [Ring R]
    (S : PolySystem n) :
    S.ZeroLocus (R := R) = ∅ ↔ ¬ ∃ ρ : Fin n → R, S.Solution ρ := by
  constructor
  · intro hempty
    rintro ⟨ρ, hρ⟩
    have hmem : ρ ∈ S.ZeroLocus := hρ
    rw [hempty] at hmem
    simp at hmem
  · intro hno
    ext ρ
    constructor
    · intro hρ
      exact False.elim (hno ⟨ρ, hρ⟩)
    · intro hρ
      simp at hρ

/-- Any solution of the generated polynomial system satisfies the formula-level algebraic
constraints. This is the reusable bridge from raw equation lists back to SAT structure. -/
theorem formula_algebraicSolution_of_toPolySystem_solution {n : Nat} {R : Type} [Ring R]
    {F : Formula3 n} {ρ : Fin n → R} (hsol : F.toPolySystem.Solution ρ) :
    F.AlgebraicSolution ρ := by
  constructor
  · intro i
    exact hsol (PolyExpr.booleanConstraint i) (F.booleanConstraint_mem_toPolySystem i)
  · intro C hC
    exact hsol C.falsePoly (Formula3.clauseFalsePoly_mem_toPolySystem hC)

/-- Formula-level algebraic constraints solve every equation in the generated system. -/
theorem formula_toPolySystem_solution_of_algebraicSolution {n : Nat} {R : Type} [Ring R]
    {F : Formula3 n} {ρ : Fin n → R} (h : F.AlgebraicSolution ρ) :
    F.toPolySystem.Solution ρ := by
  classical
  intro p hp
  change p ∈
    ((Finset.univ : Finset (Fin n)).toList.map PolyExpr.booleanConstraint) ++
      F.map Clause3.falsePoly at hp
  rw [List.mem_append] at hp
  rcases hp with hp | hp
  · rcases List.mem_map.mp hp with ⟨i, _hi, rfl⟩
    exact h.1 i
  · rcases List.mem_map.mp hp with ⟨C, hC, rfl⟩
    exact h.2 C hC

/-- The generated polynomial system is semantically the same as the formula-level
algebraic constraints. -/
theorem formula_toPolySystem_solution_iff_algebraicSolution {n : Nat} {R : Type} [Ring R]
    {F : Formula3 n} {ρ : Fin n → R} :
    F.toPolySystem.Solution ρ ↔ F.AlgebraicSolution ρ := by
  constructor
  · exact formula_algebraicSolution_of_toPolySystem_solution
  · exact formula_toPolySystem_solution_of_algebraicSolution

/-- If an equation is explicitly in the system, it is a consequence. -/
theorem consequence_of_mem {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {p : PolyExpr n} (hp : p ∈ S.equations) :
    S.Consequence (R := R) p := by
  intro ρ hsol
  exact hsol p hp

/-- The zero polynomial is always a semantic consequence. -/
theorem consequence_zero {n : Nat} {R : Type} [Ring R] (S : PolySystem n) :
    S.Consequence (R := R) PolyExpr.zero := by
  intro ρ hsol
  rfl

/-- Polynomial-calculus addition rule: consequences are closed under addition. -/
theorem Consequence.add {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {p q : PolyExpr n} (hp : S.Consequence (R := R) p)
    (hq : S.Consequence (R := R) q) :
    S.Consequence (R := R) (PolyExpr.add p q) := by
  intro ρ hsol
  simp [PolyExpr.eval, hp ρ hsol, hq ρ hsol]

/-- Polynomial-calculus subtraction rule: consequences are closed under subtraction. -/
theorem Consequence.sub {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {p q : PolyExpr n} (hp : S.Consequence (R := R) p)
    (hq : S.Consequence (R := R) q) :
    S.Consequence (R := R) (PolyExpr.sub p q) := by
  intro ρ hsol
  simp [PolyExpr.eval, hp ρ hsol, hq ρ hsol]

/-- Polynomial-calculus negation rule: consequences are closed under additive inverse. -/
theorem Consequence.neg {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {p : PolyExpr n} (hp : S.Consequence (R := R) p) :
    S.Consequence (R := R) (PolyExpr.neg p) := by
  intro ρ hsol
  simp [PolyExpr.neg, PolyExpr.eval, hp ρ hsol]

/-- Polynomial-calculus multiplication rule: multiply a known consequence by any polynomial. -/
theorem Consequence.mul_left {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {p q : PolyExpr n} (hp : S.Consequence (R := R) p) :
    S.Consequence (R := R) (PolyExpr.mul q p) := by
  intro ρ hsol
  simp [PolyExpr.eval, hp ρ hsol]

/-- Polynomial-calculus multiplication rule, right-sided version. -/
theorem Consequence.mul_right {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {p q : PolyExpr n} (hp : S.Consequence (R := R) p) :
    S.Consequence (R := R) (PolyExpr.mul p q) := by
  intro ρ hsol
  simp [PolyExpr.eval, hp ρ hsol]

/-- Polynomial-calculus linear combination with polynomial multipliers. -/
theorem Consequence.polyCombination {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {a b p q : PolyExpr n} (hp : S.Consequence (R := R) p)
    (hq : S.Consequence (R := R) q) :
    S.Consequence (R := R)
      (PolyExpr.add (PolyExpr.mul a p) (PolyExpr.mul b q)) := by
  exact (hp.mul_left (q := a)).add (hq.mul_left (q := b))

/-- Finite summation rule: the sum of any list of consequences is a consequence. -/
theorem Consequence.sumList {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    {ps : List (PolyExpr n)}
    (hps : ∀ p ∈ ps, S.Consequence (R := R) p) :
    S.Consequence (R := R) (PolyExpr.sumList ps) := by
  induction ps with
  | nil =>
      exact consequence_zero S
  | cons p ps ih =>
      exact (hps p (by simp)).add (ih (by
        intro q hq
        exact hps q (by simp [hq])))

/-- Boolean axioms of a generated SAT polynomial system are immediate consequences. -/
theorem consequence_booleanConstraint_of_formula {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (i : Fin n) :
    F.toPolySystem.Consequence (R := R) (PolyExpr.booleanConstraint i) :=
  consequence_of_mem (F.booleanConstraint_mem_toPolySystem i)

/-- Clause equations of a generated SAT polynomial system are immediate consequences. -/
theorem consequence_clauseFalsePoly_of_formula {n : Nat} {R : Type} [Ring R]
    {F : Formula3 n} {C : Clause3 n} (hC : C ∈ F) :
    F.toPolySystem.Consequence (R := R) C.falsePoly :=
  consequence_of_mem (Formula3.clauseFalsePoly_mem_toPolySystem hC)

/-- Boolean reduction rule for generated SAT systems: `p * xᵢ² - p * xᵢ = 0`. -/
theorem consequence_booleanSquareReductionLeft_of_formula {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (p : PolyExpr n) (i : Fin n) :
    F.toPolySystem.Consequence (R := R) (PolyExpr.booleanSquareReductionLeft p i) := by
  intro ρ hsol
  have halg := formula_algebraicSolution_of_toPolySystem_solution (F := F) hsol
  exact booleanSquareReductionLeft_eval_of_constraints halg.1 p i

/-- Right-sided Boolean reduction rule for generated SAT systems: `xᵢ² * p - xᵢ * p = 0`. -/
theorem consequence_booleanSquareReductionRight_of_formula {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (p : PolyExpr n) (i : Fin n) :
    F.toPolySystem.Consequence (R := R) (PolyExpr.booleanSquareReductionRight p i) := by
  intro ρ hsol
  have halg := formula_algebraicSolution_of_toPolySystem_solution (F := F) hsol
  exact booleanSquareReductionRight_eval_of_constraints halg.1 p i

/-- Adding equations preserves consequences from the smaller system. -/
theorem Consequence.of_subset {n : Nat} {R : Type} [Ring R]
    {S T : PolySystem n} {p : PolyExpr n}
    (hsub : ∀ q, q ∈ S.equations → q ∈ T.equations)
    (hp : S.Consequence (R := R) p) :
    T.Consequence (R := R) p := by
  intro ρ hT
  exact hp ρ (by intro q hq; exact hT q (hsub q hq))

/-- If the constant `1` is a consequence over a nontrivial ring, the system has no solution. -/
theorem no_solution_of_refutes {n : Nat} {R : Type} [Ring R] [Nontrivial R]
    {S : PolySystem n} (hrefute : S.Refutes (R := R)) :
    ¬ ∃ ρ : Fin n → R, S.Solution ρ := by
  rintro ⟨ρ, hsol⟩
  change S.Consequence (R := R) PolyExpr.one at hrefute
  have h1 : (1 : R) = 0 := hrefute ρ hsol
  exact one_ne_zero h1

/-- Semantic consequence is exactly vanishing on the zero locus. -/
theorem consequence_iff_vanishesOn_zeroLocus {n : Nat} {R : Type} [Ring R]
    (S : PolySystem n) (p : PolyExpr n) :
    S.Consequence (R := R) p ↔
      VanishesOn (R := R) (S.ZeroLocus (R := R)) p := by
  rfl

/-- If the zero locus shrinks, all vanishing consequences of the larger locus still vanish. -/
theorem Consequence.of_zeroLocus_subset {n : Nat} {R : Type} [Ring R]
    {S T : PolySystem n} {p : PolyExpr n}
    (hsub : T.ZeroLocus (R := R) ⊆ S.ZeroLocus (R := R))
    (hp : S.Consequence (R := R) p) :
    T.Consequence (R := R) p := by
  intro ρ hT
  exact hp ρ (hsub hT)

/-- An empty zero locus makes every polynomial a semantic consequence. -/
theorem consequence_of_zeroLocus_empty {n : Nat} {R : Type} [Ring R]
    {S : PolySystem n} (hempty : S.ZeroLocus (R := R) = ∅) (p : PolyExpr n) :
    S.Consequence (R := R) p := by
  intro ρ hsol
  have hmem : ρ ∈ S.ZeroLocus := hsol
  rw [hempty] at hmem
  simp at hmem

/-- Empty zero locus implies algebraic refutation. -/
theorem refutes_of_zeroLocus_empty {n : Nat} {R : Type} [Ring R]
    {S : PolySystem n} (hempty : S.ZeroLocus (R := R) = ∅) :
    S.Refutes (R := R) :=
  consequence_of_zeroLocus_empty hempty PolyExpr.one

/-- Over a nontrivial ring, refuting by `1 = 0` empties the zero locus. -/
theorem zeroLocus_empty_of_refutes {n : Nat} {R : Type} [Ring R] [Nontrivial R]
    {S : PolySystem n} (hrefute : S.Refutes (R := R)) :
    S.ZeroLocus (R := R) = ∅ := by
  rw [zeroLocus_empty_iff_no_solution]
  exact no_solution_of_refutes hrefute

/-- Over a nontrivial ring, refutation is equivalent to empty zero locus. -/
theorem refutes_iff_zeroLocus_empty {n : Nat} {R : Type} [Ring R] [Nontrivial R]
    (S : PolySystem n) :
    S.Refutes (R := R) ↔ S.ZeroLocus (R := R) = ∅ := by
  constructor
  · exact zeroLocus_empty_of_refutes
  · exact refutes_of_zeroLocus_empty

/-- A polynomial-system refutation of the SAT encoding rules out Boolean SAT witnesses. -/
theorem formula_not_satisfiable_of_toPolySystem_refutes_int {n : Nat}
    {F : Formula3 n} (hrefute : F.toPolySystem.Refutes (R := Int)) :
    ¬ F.Satisfiable := by
  intro hsat
  rcases hsat with ⟨v, hv⟩
  have halg : F.BooleanAlgebraicSolution (R := Int) v :=
    (formula_booleanAlgebraicSolution_iff_eval F v).mpr hv
  have hsol : F.toPolySystem.Solution (PolyExpr.boolAssignment (R := Int) v) :=
    formula_toPolySystem_solution_of_algebraicSolution halg
  exact no_solution_of_refutes (S := F.toPolySystem) (R := Int) hrefute
    ⟨PolyExpr.boolAssignment (R := Int) v, hsol⟩

/-- Refutations are monotone under adding equations. -/
theorem Refutes.of_subset {n : Nat} {R : Type} [Ring R]
    {S T : PolySystem n}
    (hsub : ∀ q, q ∈ S.equations → q ∈ T.equations)
    (hrefute : S.Refutes (R := R)) :
    T.Refutes (R := R) := by
  exact Consequence.of_subset hsub hrefute

end PolySystem

/-! The algebraic rule names mirror the SAT taxonomy. -/

/-- Named algebraic rule families used by polynomial-calculus and Nullstellensatz views. -/
inductive AlgebraicRuleKind where
  | booleanAxiom
  | clauseAxiom
  | linearCombination
  | multiplyByPolynomial
  | booleanReduction
  | deriveOne
  | degreeBound
  | monomialSupport
  | nullstellensatzCertificate
  deriving DecidableEq, Repr

/-- Information budget for an algebraic derivation or certificate.
The philosophy is: a proof is not just true/false; it has an information channel width. -/
structure AlgebraicInformationBudget where
  equationCount : Nat
  totalSymbolSize : Nat
  maxDegree : Nat
  supportVariables : Nat
  derivedLines : Nat
  deriving DecidableEq, Repr

namespace AlgebraicInformationBudget

/-- Componentwise domination of information budgets. -/
def Le (A B : AlgebraicInformationBudget) : Prop :=
  A.equationCount ≤ B.equationCount ∧
  A.totalSymbolSize ≤ B.totalSymbolSize ∧
  A.maxDegree ≤ B.maxDegree ∧
  A.supportVariables ≤ B.supportVariables ∧
  A.derivedLines ≤ B.derivedLines

theorem refl (A : AlgebraicInformationBudget) : A.Le A := by
  exact ⟨le_rfl, le_rfl, le_rfl, le_rfl, le_rfl⟩

theorem trans {A B C : AlgebraicInformationBudget} (hAB : A.Le B) (hBC : B.Le C) :
    A.Le C := by
  rcases hAB with ⟨h1, h2, h3, h4, h5⟩
  rcases hBC with ⟨g1, g2, g3, g4, g5⟩
  exact ⟨h1.trans g1, h2.trans g2, h3.trans g3, h4.trans g4, h5.trans g5⟩

/-- The empty information budget. -/
def zero : AlgebraicInformationBudget where
  equationCount := 0
  totalSymbolSize := 0
  maxDegree := 0
  supportVariables := 0
  derivedLines := 0

/-- Combine two budgets. Counts and sizes add; width-like fields take maxima. -/
def combine (A B : AlgebraicInformationBudget) : AlgebraicInformationBudget where
  equationCount := A.equationCount + B.equationCount
  totalSymbolSize := A.totalSymbolSize + B.totalSymbolSize
  maxDegree := max A.maxDegree B.maxDegree
  supportVariables := max A.supportVariables B.supportVariables
  derivedLines := A.derivedLines + B.derivedLines

theorem le_combine_left (A B : AlgebraicInformationBudget) : A.Le (combine A B) := by
  unfold Le combine
  exact ⟨Nat.le_add_right _ _, Nat.le_add_right _ _, Nat.le_max_left _ _,
    Nat.le_max_left _ _, Nat.le_add_right _ _⟩

theorem le_combine_right (A B : AlgebraicInformationBudget) : B.Le (combine A B) := by
  unfold Le combine
  exact ⟨Nat.le_add_left _ _, Nat.le_add_left _ _, Nat.le_max_right _ _,
    Nat.le_max_right _ _, Nat.le_add_left _ _⟩

end AlgebraicInformationBudget

/-- One derived algebraic line, classified by rule family and carrying its conclusion. -/
structure AlgebraicDerivedLine (n : Nat) where
  kind : AlgebraicRuleKind
  conclusion : PolyExpr n
  premises : Nat
  deriving DecidableEq, Repr

namespace AlgebraicDerivedLine

/-- Expression-level information profile of a derived line. -/
def infoProfile {n : Nat} (L : AlgebraicDerivedLine n) : PolyExpr.InfoProfile n :=
  L.conclusion.infoProfile

/-- Turn a derived line into a one-line information budget. -/
def toInformationBudget {n : Nat} (L : AlgebraicDerivedLine n) : AlgebraicInformationBudget where
  equationCount := 0
  totalSymbolSize := L.conclusion.size
  maxDegree := L.conclusion.degree
  supportVariables := L.conclusion.supportSize
  derivedLines := 1

/-- The line support footprint is always bounded by the ambient hypercube dimension. -/
theorem supportVariables_le_dimension {n : Nat} (L : AlgebraicDerivedLine n) :
    L.toInformationBudget.supportVariables ≤ n := by
  exact L.conclusion.supportSize_le_dimension

/-- Derived line constructor for left Boolean reduction. -/
def booleanReductionLeft {n : Nat} (p : PolyExpr n) (i : Fin n) :
    AlgebraicDerivedLine n where
  kind := AlgebraicRuleKind.booleanReduction
  conclusion := PolyExpr.booleanSquareReductionLeft p i
  premises := 1

/-- Derived line constructor for right Boolean reduction. -/
def booleanReductionRight {n : Nat} (p : PolyExpr n) (i : Fin n) :
    AlgebraicDerivedLine n where
  kind := AlgebraicRuleKind.booleanReduction
  conclusion := PolyExpr.booleanSquareReductionRight p i
  premises := 1

/-- Derived line constructor for a Boolean axiom `xᵢ² - xᵢ = 0`. -/
def booleanAxiom {n : Nat} (i : Fin n) : AlgebraicDerivedLine n where
  kind := AlgebraicRuleKind.booleanAxiom
  conclusion := PolyExpr.booleanConstraint i
  premises := 0

/-- Derived line constructor for a clause axiom. -/
def clauseAxiom {n : Nat} (C : Clause3 n) : AlgebraicDerivedLine n where
  kind := AlgebraicRuleKind.clauseAxiom
  conclusion := C.falsePoly
  premises := 0

/-- Derived line constructor for multiplying a known equation by a polynomial. -/
def multiplyByPolynomial {n : Nat} (a p : PolyExpr n) : AlgebraicDerivedLine n where
  kind := AlgebraicRuleKind.multiplyByPolynomial
  conclusion := PolyExpr.mul a p
  premises := 1

/-- Derived line constructor for a polynomial-calculus linear combination
`a*p + b*q`. -/
def linearCombination {n : Nat} (a b p q : PolyExpr n) : AlgebraicDerivedLine n where
  kind := AlgebraicRuleKind.linearCombination
  conclusion := PolyExpr.add (PolyExpr.mul a p) (PolyExpr.mul b q)
  premises := 2

@[simp] theorem booleanAxiom_totalSymbolSize {n : Nat} (i : Fin n) :
    (booleanAxiom i).toInformationBudget.totalSymbolSize = 5 := by
  simp [toInformationBudget, booleanAxiom]

@[simp] theorem booleanAxiom_maxDegree {n : Nat} (i : Fin n) :
    (booleanAxiom i).toInformationBudget.maxDegree = 2 := by
  simp [toInformationBudget, booleanAxiom]

theorem booleanAxiom_supportVariables_le_one {n : Nat} (i : Fin n) :
    (booleanAxiom i).toInformationBudget.supportVariables ≤ 1 :=
  PolyExpr.booleanConstraint_supportSize_le_one i

@[simp] theorem clauseAxiom_maxDegree {n : Nat} (C : Clause3 n) :
    (clauseAxiom C).toInformationBudget.maxDegree = 3 := by
  simp [toInformationBudget, clauseAxiom]

theorem clauseAxiom_totalSymbolSize_le_eleven {n : Nat} (C : Clause3 n) :
    (clauseAxiom C).toInformationBudget.totalSymbolSize ≤ 11 :=
  C.falsePoly_size_le_eleven

theorem clauseAxiom_supportVariables_le_three {n : Nat} (C : Clause3 n) :
    (clauseAxiom C).toInformationBudget.supportVariables ≤ 3 :=
  C.falsePoly_supportSize_le_three

@[simp] theorem booleanReductionLeft_totalSymbolSize {n : Nat}
    (p : PolyExpr n) (i : Fin n) :
    (booleanReductionLeft p i).toInformationBudget.totalSymbolSize = 2 * p.size + 7 := by
  simp [toInformationBudget, booleanReductionLeft, PolyExpr.booleanSquareReductionLeft,
    PolyExpr.square, PolyExpr.size]
  omega

@[simp] theorem booleanReductionRight_totalSymbolSize {n : Nat}
    (p : PolyExpr n) (i : Fin n) :
    (booleanReductionRight p i).toInformationBudget.totalSymbolSize = 2 * p.size + 7 := by
  simp [toInformationBudget, booleanReductionRight, PolyExpr.booleanSquareReductionRight,
    PolyExpr.square, PolyExpr.size]
  omega

theorem booleanReductionLeft_maxDegree_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanReductionLeft p i).toInformationBudget.maxDegree ≤ p.degree + 2 :=
  PolyExpr.booleanSquareReductionLeft_degree_le p i

theorem booleanReductionRight_maxDegree_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanReductionRight p i).toInformationBudget.maxDegree ≤ p.degree + 2 :=
  PolyExpr.booleanSquareReductionRight_degree_le p i

theorem booleanReductionLeft_supportVariables_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanReductionLeft p i).toInformationBudget.supportVariables ≤ p.supportSize + 1 :=
  PolyExpr.booleanSquareReductionLeft_supportSize_le p i

theorem booleanReductionRight_supportVariables_le {n : Nat} (p : PolyExpr n) (i : Fin n) :
    (booleanReductionRight p i).toInformationBudget.supportVariables ≤ p.supportSize + 1 :=
  PolyExpr.booleanSquareReductionRight_supportSize_le p i

@[simp] theorem multiplyByPolynomial_totalSymbolSize {n : Nat}
    (a p : PolyExpr n) :
    (multiplyByPolynomial a p).toInformationBudget.totalSymbolSize = a.size + p.size + 1 := by
  rfl

@[simp] theorem multiplyByPolynomial_maxDegree {n : Nat} (a p : PolyExpr n) :
    (multiplyByPolynomial a p).toInformationBudget.maxDegree = a.degree + p.degree := by
  rfl

theorem multiplyByPolynomial_supportVariables_le {n : Nat} (a p : PolyExpr n) :
    (multiplyByPolynomial a p).toInformationBudget.supportVariables ≤
      a.supportSize + p.supportSize :=
  PolyExpr.supportSize_mul_le a p

@[simp] theorem linearCombination_totalSymbolSize {n : Nat}
    (a b p q : PolyExpr n) :
    (linearCombination a b p q).toInformationBudget.totalSymbolSize =
      a.size + p.size + b.size + q.size + 3 := by
  simp [toInformationBudget, linearCombination, PolyExpr.size]
  omega

@[simp] theorem linearCombination_maxDegree {n : Nat} (a b p q : PolyExpr n) :
    (linearCombination a b p q).toInformationBudget.maxDegree =
      max (a.degree + p.degree) (b.degree + q.degree) := by
  rfl

theorem linearCombination_supportVariables_le {n : Nat} (a b p q : PolyExpr n) :
    (linearCombination a b p q).toInformationBudget.supportVariables ≤
      a.supportSize + p.supportSize + b.supportSize + q.supportSize := by
  have hap := PolyExpr.supportSize_mul_le a p
  have hbq := PolyExpr.supportSize_mul_le b q
  have hadd := PolyExpr.supportSize_add_le (PolyExpr.mul a p) (PolyExpr.mul b q)
  calc
    (linearCombination a b p q).toInformationBudget.supportVariables
        ≤ (PolyExpr.mul a p).supportSize + (PolyExpr.mul b q).supportSize := hadd
    _ ≤ (a.supportSize + p.supportSize) + (b.supportSize + q.supportSize) :=
      Nat.add_le_add hap hbq
    _ ≤ a.supportSize + p.supportSize + b.supportSize + q.supportSize := by omega

/-- Soundness of a Boolean-reduction line over the formula-generated system. -/
theorem booleanReductionLeft_sound {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (p : PolyExpr n) (i : Fin n) :
    F.toPolySystem.Consequence (R := R) (booleanReductionLeft p i).conclusion :=
  PolySystem.consequence_booleanSquareReductionLeft_of_formula F p i

/-- Soundness of a right Boolean-reduction line over the formula-generated system. -/
theorem booleanReductionRight_sound {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (p : PolyExpr n) (i : Fin n) :
    F.toPolySystem.Consequence (R := R) (booleanReductionRight p i).conclusion :=
  PolySystem.consequence_booleanSquareReductionRight_of_formula F p i

/-- Soundness of a Boolean-axiom line over the generated SAT system. -/
theorem booleanAxiom_sound {n : Nat} {R : Type} [Ring R]
    (F : Formula3 n) (i : Fin n) :
    F.toPolySystem.Consequence (R := R) (booleanAxiom i).conclusion :=
  PolySystem.consequence_booleanConstraint_of_formula F i

/-- Soundness of a clause-axiom line over the generated SAT system. -/
theorem clauseAxiom_sound {n : Nat} {R : Type} [Ring R]
    {F : Formula3 n} {C : Clause3 n} (hC : C ∈ F) :
    F.toPolySystem.Consequence (R := R) (clauseAxiom C).conclusion :=
  PolySystem.consequence_clauseFalsePoly_of_formula hC

/-- Soundness of a multiply-by-polynomial derived line. -/
theorem multiplyByPolynomial_sound {n : Nat} {R : Type} [Ring R]
    {S : PolySystem n} {a p : PolyExpr n}
    (hp : S.Consequence (R := R) p) :
    S.Consequence (R := R) (multiplyByPolynomial a p).conclusion :=
  hp.mul_left (q := a)

/-- Soundness of a linear-combination derived line. -/
theorem linearCombination_sound {n : Nat} {R : Type} [Ring R]
    {S : PolySystem n} {a b p q : PolyExpr n}
    (hp : S.Consequence (R := R) p) (hq : S.Consequence (R := R) q) :
    S.Consequence (R := R) (linearCombination a b p q).conclusion :=
  hp.polyCombination hq

/-! #### Information budgets for lists of derived lines -/

/-- Total symbol size of a list of derived algebraic lines. -/
def listTotalSymbolSize {n : Nat} (Ls : List (AlgebraicDerivedLine n)) : Nat :=
  (Ls.map fun L => L.conclusion.size).sum

/-- Maximum degree of conclusions in a list of derived algebraic lines. -/
def listMaxDegree {n : Nat} (Ls : List (AlgebraicDerivedLine n)) : Nat :=
  (Ls.map fun L => L.conclusion.degree).foldl max 0

/-- Variables mentioned by any conclusion in a list of derived algebraic lines. -/
def listSupportVars {n : Nat} (Ls : List (AlgebraicDerivedLine n)) : Finset (Fin n) :=
  Ls.foldl (fun acc L => acc ∪ L.conclusion.supportVars) ∅

/-- Number of variables mentioned by a derived-line list. -/
def listSupportVariables {n : Nat} (Ls : List (AlgebraicDerivedLine n)) : Nat :=
  (listSupportVars Ls).card

/-- Aggregate information budget for a derived-line list. -/
def listInformationBudget {n : Nat} (Ls : List (AlgebraicDerivedLine n)) :
    AlgebraicInformationBudget where
  equationCount := 0
  totalSymbolSize := listTotalSymbolSize Ls
  maxDegree := listMaxDegree Ls
  supportVariables := listSupportVariables Ls
  derivedLines := Ls.length

@[simp] theorem listInformationBudget_derivedLines {n : Nat}
    (Ls : List (AlgebraicDerivedLine n)) :
    (listInformationBudget Ls).derivedLines = Ls.length := by
  rfl

@[simp] theorem listInformationBudget_equationCount {n : Nat}
    (Ls : List (AlgebraicDerivedLine n)) :
    (listInformationBudget Ls).equationCount = 0 := by
  rfl

theorem listSupportVariables_le_dimension {n : Nat}
    (Ls : List (AlgebraicDerivedLine n)) :
    (listInformationBudget Ls).supportVariables ≤ n := by
  unfold listInformationBudget listSupportVariables
  have hsub : listSupportVars Ls ⊆ Finset.univ := by
    intro i hi
    exact Finset.mem_univ i
  have hle := Finset.card_le_card hsub
  simpa [Fintype.card_fin] using hle

theorem listMaxDegree_le_of_forall {n : Nat} {Ls : List (AlgebraicDerivedLine n)}
    {d : Nat} (h : ∀ L ∈ Ls, L.conclusion.degree ≤ d) :
    (listInformationBudget Ls).maxDegree ≤ d := by
  unfold listInformationBudget listMaxDegree
  apply PolySystem.list_foldl_max_le (Nat.zero_le d)
  intro deg hdeg
  rcases List.mem_map.mp hdeg with ⟨L, hL, rfl⟩
  exact h L hL

theorem listTotalSymbolSize_le_of_forall {n : Nat} {Ls : List (AlgebraicDerivedLine n)}
    {c : Nat} (h : ∀ L ∈ Ls, L.conclusion.size ≤ c) :
    (listInformationBudget Ls).totalSymbolSize ≤ Ls.length * c := by
  unfold listInformationBudget listTotalSymbolSize
  exact PolySystem.list_sum_map_le_length_mul_of_forall h

end AlgebraicDerivedLine

/-- A compact target for algebraic proof complexity: refute a polynomial system using
consequences whose degree/size is bounded by chosen accountant functions. -/
structure AlgebraicProofProfile (n : Nat) where
  system : PolySystem n
  degreeBound : Nat
  sizeBound : Nat
  refutesOver : (R : Type) → [Ring R] → Prop

/-- Convert a polynomial system profile into the static part of an information budget. -/
def PolySystem.toInformationBudget {n : Nat} (S : PolySystem n) : AlgebraicInformationBudget where
  equationCount := S.equationCount
  totalSymbolSize := S.totalSize
  maxDegree := S.maxDegree
  supportVariables := S.supportSize
  derivedLines := 0

/-- A semantic algebraic certificate is a list of derived equations ending in a conclusion,
with an information budget attached. A Nullstellensatz or polynomial-calculus certificate can
later refine `sound` into a syntactic derivation. -/
structure AlgebraicCertificate (n : Nat) where
  system : PolySystem n
  derived : List (PolyExpr n)
  conclusion : PolyExpr n
  budget : AlgebraicInformationBudget
  sound : ∀ {R : Type} [Ring R], system.Consequence (R := R) conclusion

/-- A certificate refutes over a nontrivial ring when its conclusion is the constant one. -/
def AlgebraicCertificate.Refutes {n : Nat} (C : AlgebraicCertificate n) : Prop :=
  C.conclusion = PolyExpr.one

theorem AlgebraicCertificate.no_solution_of_refutes {n : Nat} {R : Type} [Ring R] [Nontrivial R]
    {C : AlgebraicCertificate n} (hrefute : C.Refutes) :
    ¬ ∃ ρ : Fin n → R, C.system.Solution ρ := by
  apply PolySystem.no_solution_of_refutes (S := C.system) (R := R)
  rw [AlgebraicCertificate.Refutes] at hrefute
  intro ρ hsol
  simpa [hrefute] using (C.sound (R := R) ρ hsol)

/-! #### Nullstellensatz-style ideal membership certificates -/

/-- One ideal-membership term: a multiplier times one generator equation of `S`. -/
structure NullstellensatzTerm {n : Nat} (S : PolySystem n) where
  multiplier : PolyExpr n
  equation : PolyExpr n
  equation_mem : equation ∈ S.equations

namespace NullstellensatzTerm

/-- The polynomial expression contributed by one ideal-membership term. -/
def expr {n : Nat} {S : PolySystem n} (T : NullstellensatzTerm S) : PolyExpr n :=
  PolyExpr.mul T.multiplier T.equation

/-- View one ideal-membership term as a derived multiply-by-polynomial line. -/
def toDerivedLine {n : Nat} {S : PolySystem n} (T : NullstellensatzTerm S) :
    AlgebraicDerivedLine n :=
  AlgebraicDerivedLine.multiplyByPolynomial T.multiplier T.equation

/-- Soundness of one ideal-membership term: it vanishes on every solution of `S`. -/
theorem consequence {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    (T : NullstellensatzTerm S) :
    S.Consequence (R := R) T.expr := by
  exact (PolySystem.consequence_of_mem (S := S) (R := R) T.equation_mem).mul_left
    (q := T.multiplier)

@[simp] theorem toDerivedLine_conclusion {n : Nat} {S : PolySystem n}
    (T : NullstellensatzTerm S) :
    T.toDerivedLine.conclusion = T.expr := by
  rfl

@[simp] theorem toDerivedLine_totalSymbolSize {n : Nat} {S : PolySystem n}
    (T : NullstellensatzTerm S) :
    T.toDerivedLine.toInformationBudget.totalSymbolSize =
      T.multiplier.size + T.equation.size + 1 := by
  rfl

@[simp] theorem toDerivedLine_maxDegree {n : Nat} {S : PolySystem n}
    (T : NullstellensatzTerm S) :
    T.toDerivedLine.toInformationBudget.maxDegree =
      T.multiplier.degree + T.equation.degree := by
  rfl

theorem toDerivedLine_supportVariables_le {n : Nat} {S : PolySystem n}
    (T : NullstellensatzTerm S) :
    T.toDerivedLine.toInformationBudget.supportVariables ≤
      T.multiplier.supportSize + T.equation.supportSize :=
  AlgebraicDerivedLine.multiplyByPolynomial_supportVariables_le T.multiplier T.equation

end NullstellensatzTerm

/-- A Nullstellensatz-style certificate: a finite linear combination of generator equations.
When the `conclusion` is `1`, this is the algebraic-geometry certificate that the polynomial
system has no common zero over any nontrivial target ring. -/
structure NullstellensatzCertificate {n : Nat} (S : PolySystem n) where
  terms : List (NullstellensatzTerm S)

namespace NullstellensatzCertificate

/-- The polynomial produced by a Nullstellensatz-style certificate. -/
def conclusion {n : Nat} {S : PolySystem n} (C : NullstellensatzCertificate S) : PolyExpr n :=
  PolyExpr.sumList (C.terms.map fun T => T.expr)

/-- Derived multiply-lines corresponding to the certificate terms. -/
def derivedLines {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) : List (AlgebraicDerivedLine n) :=
  C.terms.map fun T => T.toDerivedLine

/-- Information budget of the multiplier-times-equation lines alone. -/
def lineBudget {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) : AlgebraicInformationBudget :=
  AlgebraicDerivedLine.listInformationBudget C.derivedLines

/-- Total budget: original system plus the ideal-membership terms. -/
def totalBudget {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) : AlgebraicInformationBudget :=
  AlgebraicInformationBudget.combine S.toInformationBudget C.lineBudget

/-- Semantic soundness: every ideal-membership certificate conclusion is a consequence. -/
theorem sound {n : Nat} {R : Type} [Ring R] {S : PolySystem n}
    (C : NullstellensatzCertificate S) :
    S.Consequence (R := R) C.conclusion := by
  unfold conclusion
  apply PolySystem.Consequence.sumList
  intro p hp
  rcases List.mem_map.mp hp with ⟨T, hT, rfl⟩
  exact T.consequence

/-- Convert a Nullstellensatz-style certificate into the generic semantic certificate. -/
def toAlgebraicCertificate {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) : AlgebraicCertificate n where
  system := S
  derived := C.derivedLines.map fun L => L.conclusion
  conclusion := C.conclusion
  budget := C.totalBudget
  sound := by
    intro R inst
    exact C.sound

/-- A Nullstellensatz certificate refutes when its ideal-combination conclusion is `1`. -/
def Refutes {n : Nat} {S : PolySystem n} (C : NullstellensatzCertificate S) : Prop :=
  C.conclusion = PolyExpr.one

/-- A Nullstellensatz refutation rules out common zeros over nontrivial rings. -/
theorem no_solution_of_refutes {n : Nat} {R : Type} [Ring R] [Nontrivial R]
    {S : PolySystem n} {C : NullstellensatzCertificate S} (hrefute : C.Refutes) :
    ¬ ∃ ρ : Fin n → R, S.Solution ρ := by
  have hrefute' : C.toAlgebraicCertificate.Refutes := by
    simpa [AlgebraicCertificate.Refutes, toAlgebraicCertificate, Refutes] using hrefute
  exact AlgebraicCertificate.no_solution_of_refutes (R := R) hrefute'

@[simp] theorem derivedLines_length {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) :
    C.derivedLines.length = C.terms.length := by
  simp [derivedLines]

@[simp] theorem lineBudget_derivedLines {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) :
    C.lineBudget.derivedLines = C.terms.length := by
  simp [lineBudget]

@[simp] theorem totalBudget_equationCount {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) :
    C.totalBudget.equationCount = S.equationCount := by
  simp [totalBudget, lineBudget, AlgebraicInformationBudget.combine,
    AlgebraicDerivedLine.listInformationBudget, PolySystem.toInformationBudget]

@[simp] theorem totalBudget_derivedLines {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) :
    C.totalBudget.derivedLines = C.terms.length := by
  simp [totalBudget, lineBudget, AlgebraicInformationBudget.combine,
    PolySystem.toInformationBudget]

/-- If all multipliers and generators in the certificate have bounded size, so does the
derived-line channel. -/
theorem lineBudget_totalSymbolSize_le {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) {M E : Nat}
    (hM : ∀ T ∈ C.terms, T.multiplier.size ≤ M)
    (hE : ∀ T ∈ C.terms, T.equation.size ≤ E) :
    C.lineBudget.totalSymbolSize ≤ C.terms.length * (M + E + 1) := by
  unfold lineBudget derivedLines
  have hbound :
      (AlgebraicDerivedLine.listInformationBudget
          (C.terms.map fun T => T.toDerivedLine)).totalSymbolSize ≤
        (C.terms.map fun T => T.toDerivedLine).length * (M + E + 1) := by
    apply AlgebraicDerivedLine.listTotalSymbolSize_le_of_forall
    intro L hL
    rcases List.mem_map.mp hL with ⟨T, hT, rfl⟩
    have hm := hM T hT
    have he := hE T hT
    change T.multiplier.size + T.equation.size + 1 ≤ M + E + 1
    omega
  simpa using hbound

/-- Degree width bound for a Nullstellensatz-style certificate. -/
theorem lineBudget_maxDegree_le {n : Nat} {S : PolySystem n}
    (C : NullstellensatzCertificate S) {D E : Nat}
    (hD : ∀ T ∈ C.terms, T.multiplier.degree ≤ D)
    (hE : ∀ T ∈ C.terms, T.equation.degree ≤ E) :
    C.lineBudget.maxDegree ≤ D + E := by
  unfold lineBudget
  apply AlgebraicDerivedLine.listMaxDegree_le_of_forall
  intro L hL
  unfold derivedLines at hL
  rcases List.mem_map.mp hL with ⟨T, hT, rfl⟩
  change (PolyExpr.mul T.multiplier T.equation).degree ≤ D + E
  rw [PolyExpr.degree_mul_eq]
  have hd := hD T hT
  have he := hE T hT
  omega

end NullstellensatzCertificate

/-- A semantic algebraic derivation: every listed derived line is certified as a consequence
of the same polynomial system. This is the proof-complexity object whose information channel
we can measure line-by-line. -/
structure AlgebraicDerivation (n : Nat) where
  system : PolySystem n
  lines : List (AlgebraicDerivedLine n)
  sound : ∀ L ∈ lines, ∀ {R : Type} [Ring R], system.Consequence (R := R) L.conclusion

namespace AlgebraicDerivation

/-- Conclusions carried by a derivation. -/
def conclusions {n : Nat} (D : AlgebraicDerivation n) : List (PolyExpr n) :=
  D.lines.map fun L => L.conclusion

/-- Derived-line information only, excluding the input system. -/
def lineBudget {n : Nat} (D : AlgebraicDerivation n) : AlgebraicInformationBudget :=
  AlgebraicDerivedLine.listInformationBudget D.lines

/-- Total information budget: original equation channel plus derived-line channel. -/
def totalBudget {n : Nat} (D : AlgebraicDerivation n) : AlgebraicInformationBudget :=
  AlgebraicInformationBudget.combine D.system.toInformationBudget D.lineBudget

@[simp] theorem lineBudget_derivedLines {n : Nat} (D : AlgebraicDerivation n) :
    D.lineBudget.derivedLines = D.lines.length := by
  rfl

@[simp] theorem totalBudget_equationCount {n : Nat} (D : AlgebraicDerivation n) :
    D.totalBudget.equationCount = D.system.equationCount := by
  simp [totalBudget, lineBudget, PolySystem.toInformationBudget,
    AlgebraicInformationBudget.combine, AlgebraicDerivedLine.listInformationBudget]

@[simp] theorem totalBudget_derivedLines {n : Nat} (D : AlgebraicDerivation n) :
    D.totalBudget.derivedLines = D.lines.length := by
  simp [totalBudget, lineBudget, PolySystem.toInformationBudget,
    AlgebraicInformationBudget.combine]

theorem lineBudget_supportVariables_le_dimension {n : Nat} (D : AlgebraicDerivation n) :
    D.lineBudget.supportVariables ≤ n :=
  AlgebraicDerivedLine.listSupportVariables_le_dimension D.lines

theorem systemBudget_le_totalBudget {n : Nat} (D : AlgebraicDerivation n) :
    D.system.toInformationBudget.Le D.totalBudget :=
  AlgebraicInformationBudget.le_combine_left _ _

theorem lineBudget_le_totalBudget {n : Nat} (D : AlgebraicDerivation n) :
    D.lineBudget.Le D.totalBudget :=
  AlgebraicInformationBudget.le_combine_right _ _

/-- A listed conclusion is a semantic consequence of the derivation's system. -/
theorem consequence_of_line {n : Nat} {D : AlgebraicDerivation n}
    {L : AlgebraicDerivedLine n} (hL : L ∈ D.lines) {R : Type} [Ring R] :
    D.system.Consequence (R := R) L.conclusion :=
  D.sound L hL

/-- Turn any derived line of a derivation into the older certificate interface. -/
def toCertificateOfLine {n : Nat} (D : AlgebraicDerivation n)
    {L : AlgebraicDerivedLine n} (hL : L ∈ D.lines) : AlgebraicCertificate n where
  system := D.system
  derived := D.conclusions
  conclusion := L.conclusion
  budget := D.totalBudget
  sound := by
    intro R inst
    exact D.sound L hL

/-- If a derivation contains the constant-one conclusion, it refutes over nontrivial rings. -/
theorem no_solution_of_one_line {n : Nat} {R : Type} [Ring R] [Nontrivial R]
    {D : AlgebraicDerivation n} {L : AlgebraicDerivedLine n}
    (hL : L ∈ D.lines) (hone : L.conclusion = PolyExpr.one) :
    ¬ ∃ ρ : Fin n → R, D.system.Solution ρ := by
  have hcertRefutes : (D.toCertificateOfLine hL).Refutes := by
    simpa [AlgebraicCertificate.Refutes, toCertificateOfLine] using hone
  exact AlgebraicCertificate.no_solution_of_refutes (R := R) hcertRefutes

end AlgebraicDerivation

/-- The canonical list of algebraic axiom lines generated by a 3-CNF formula. -/
noncomputable def Formula3.algebraicAxiomLines {n : Nat} (F : Formula3 n) :
    List (AlgebraicDerivedLine n) := by
  classical
  exact
    ((Finset.univ : Finset (Fin n)).toList.map AlgebraicDerivedLine.booleanAxiom) ++
      F.map AlgebraicDerivedLine.clauseAxiom

@[simp] theorem Formula3.algebraicAxiomLines_length {n : Nat} (F : Formula3 n) :
    F.algebraicAxiomLines.length = n + F.length := by
  classical
  simp [Formula3.algebraicAxiomLines]

/-- The canonical axiom derivation of the SAT polynomial system. It contains exactly the
Boolean equations and clause equations as typed, measured derived lines. -/
noncomputable def Formula3.algebraicAxiomDerivation {n : Nat} (F : Formula3 n) :
    AlgebraicDerivation n := by
  classical
  exact
    { system := F.toPolySystem
      lines := F.algebraicAxiomLines
      sound := by
        intro L hL R inst
        unfold Formula3.algebraicAxiomLines at hL
        rw [List.mem_append] at hL
        rcases hL with hbool | hclause
        · rcases List.mem_map.mp hbool with ⟨i, _hi, rfl⟩
          exact AlgebraicDerivedLine.booleanAxiom_sound F i
        · rcases List.mem_map.mp hclause with ⟨C, hC, rfl⟩
          exact AlgebraicDerivedLine.clauseAxiom_sound hC }

@[simp] theorem Formula3.algebraicAxiomDerivation_derivedLines {n : Nat}
    (F : Formula3 n) :
    F.algebraicAxiomDerivation.totalBudget.derivedLines = n + F.length := by
  classical
  simp [Formula3.algebraicAxiomDerivation]

@[simp] theorem Formula3.algebraicAxiomDerivation_equationCount {n : Nat}
    (F : Formula3 n) :
    F.algebraicAxiomDerivation.totalBudget.equationCount = n + F.length := by
  classical
  simp [Formula3.algebraicAxiomDerivation]

theorem Formula3.algebraicAxiomLines_maxDegree_le_three {n : Nat} (F : Formula3 n) :
    (AlgebraicDerivedLine.listInformationBudget F.algebraicAxiomLines).maxDegree ≤ 3 := by
  classical
  apply AlgebraicDerivedLine.listMaxDegree_le_of_forall
  intro L hL
  unfold Formula3.algebraicAxiomLines at hL
  rw [List.mem_append] at hL
  rcases hL with hbool | hclause
  · rcases List.mem_map.mp hbool with ⟨i, _hi, rfl⟩
    have hdeg : (PolyExpr.booleanConstraint i).degree ≤ 3 := by
      rw [PolyExpr.booleanConstraint_degree]
      omega
    simp [AlgebraicDerivedLine.booleanAxiom, hdeg]
  · rcases List.mem_map.mp hclause with ⟨C, _hC, rfl⟩
    have hdeg : C.falsePoly.degree ≤ 3 := by
      rw [C.falsePoly_degree]
    simp [AlgebraicDerivedLine.clauseAxiom, hdeg]

theorem Formula3.algebraicAxiomLines_totalSymbolSize_le {n : Nat} (F : Formula3 n) :
    (AlgebraicDerivedLine.listInformationBudget F.algebraicAxiomLines).totalSymbolSize ≤
      5 * n + 11 * F.length := by
  classical
  unfold AlgebraicDerivedLine.listInformationBudget
    AlgebraicDerivedLine.listTotalSymbolSize Formula3.algebraicAxiomLines
  simp only [List.map_append, List.sum_append, List.map_map]
  have hbool :
      (((Finset.univ : Finset (Fin n)).toList.map
          (fun i => (AlgebraicDerivedLine.booleanAxiom i).conclusion.size)).sum) =
        ((Finset.univ : Finset (Fin n)).toList.length) * 5 := by
    exact PolySystem.list_sum_map_eq_length_mul_of_forall
      (xs := (Finset.univ : Finset (Fin n)).toList)
      (f := fun i => (AlgebraicDerivedLine.booleanAxiom i).conclusion.size)
      (c := 5) (by intro i _; simp [AlgebraicDerivedLine.booleanAxiom])
  have hclauses :
      ((F.map fun C => (AlgebraicDerivedLine.clauseAxiom C).conclusion.size).sum) ≤
        F.length * 11 := by
    exact PolySystem.list_sum_map_le_length_mul_of_forall (xs := F)
      (f := fun C => (AlgebraicDerivedLine.clauseAxiom C).conclusion.size) (c := 11)
      (by intro C _; exact C.falsePoly_size_le_eleven)
  change
    (((Finset.univ : Finset (Fin n)).toList.map
        (fun i => (AlgebraicDerivedLine.booleanAxiom i).conclusion.size)).sum) +
      (F.map fun C => (AlgebraicDerivedLine.clauseAxiom C).conclusion.size).sum ≤
        5 * n + 11 * F.length
  rw [hbool]
  have hlen : ((Finset.univ : Finset (Fin n)).toList.length) = n := by
    simp
  rw [hlen]
  omega

theorem Formula3.algebraicAxiomDerivation_totalBudget_maxDegree_le_three {n : Nat}
    (F : Formula3 n) :
    F.algebraicAxiomDerivation.totalBudget.maxDegree ≤ 3 := by
  classical
  unfold AlgebraicDerivation.totalBudget AlgebraicDerivation.lineBudget
    AlgebraicInformationBudget.combine Formula3.algebraicAxiomDerivation
  exact max_le (PolySystem.formula_toPolySystem_maxDegree_le_three F)
    (Formula3.algebraicAxiomLines_maxDegree_le_three F)

theorem Formula3.algebraicAxiomDerivation_totalBudget_totalSymbolSize_le {n : Nat}
    (F : Formula3 n) :
    F.algebraicAxiomDerivation.totalBudget.totalSymbolSize ≤
      2 * (5 * n + 11 * F.length) := by
  classical
  have hsys := PolySystem.formula_toPolySystem_totalSize_le F
  have hlines := Formula3.algebraicAxiomLines_totalSymbolSize_le F
  unfold AlgebraicDerivation.totalBudget AlgebraicDerivation.lineBudget
    Formula3.algebraicAxiomDerivation AlgebraicInformationBudget.combine
    PolySystem.toInformationBudget
  change F.toPolySystem.totalSize +
      (AlgebraicDerivedLine.listInformationBudget F.algebraicAxiomLines).totalSymbolSize ≤
    2 * (5 * n + 11 * F.length)
  omega

theorem Formula3.algebraicAxiomDerivation_lineBudget_support_le_dimension {n : Nat}
    (F : Formula3 n) :
    F.algebraicAxiomDerivation.lineBudget.supportVariables ≤ n := by
  classical
  simpa [Formula3.algebraicAxiomDerivation] using
    (AlgebraicDerivation.lineBudget_supportVariables_le_dimension
      F.algebraicAxiomDerivation)

/-- A Nullstellensatz-style refutation of the generated polynomial system proves the
original 3-CNF formula unsatisfiable. This is the algebraic-geometry bridge:
`1` in the generated ideal means there is no Boolean common zero. -/
theorem Formula3.not_satisfiable_of_nullstellensatz_refutes {n : Nat} {F : Formula3 n}
    {C : NullstellensatzCertificate F.toPolySystem} (hrefute : C.Refutes) :
    ¬ F.Satisfiable := by
  have hrefuteSystem : F.toPolySystem.Refutes (R := Int) := by
    intro ρ hsol
    have hC := C.sound (R := Int) ρ hsol
    rw [NullstellensatzCertificate.Refutes] at hrefute
    rw [hrefute] at hC
    exact hC
  exact PolySystem.formula_not_satisfiable_of_toPolySystem_refutes_int hrefuteSystem

/-! ## §C Complexity: algorithms and worst-case polynomial time -/

/-- An algorithm as a first interface: a result function plus a step counter.

`steps` is a *problem-specific structural operation count* — at this abstraction level one
operation is one primitive structural action on the compact hypercube-cover representation
(e.g. checking one blocker against one vertex, adding one derived pattern, applying one merge
rule). This interface only lets us *state* bounds; it does not by itself force `steps` to match
`run` (one could cheat with `steps := fun _ => 0`). For concrete algorithms we therefore prove
separate correctness and step-bound theorems where `steps` is defined transparently from the
implementation (see `bruteForce`). A fully operational model (`runFuel (steps x) x = some (run x)`)
can be layered on later. -/
structure Algorithm (Input Output : Type) where
  run : Input → Output
  steps : Input → Nat

/-- Eventual domination of natural-valued cost functions. -/
def EventuallyLE (f g : Nat → Nat) : Prop :=
  ∃ N0 : Nat, ∀ N, N0 ≤ N → f N ≤ g N

/-- Big-O for natural-valued cost functions: eventually `f N ≤ C * g N`. -/
def BigO (f g : Nat → Nat) : Prop :=
  ∃ C N0 : Nat, ∀ N, N0 ≤ N → f N ≤ C * g N

/-- Big-Omega: `f` is eventually at least `g` up to a constant factor. -/
def BigOmega (f g : Nat → Nat) : Prop :=
  BigO g f

/-- Big-Theta: asymptotic equivalence up to constant factors. -/
def BigTheta (f g : Nat → Nat) : Prop :=
  BigO f g ∧ BigO g f

/-- A coarse resource profile: time and space as functions of a single size parameter. -/
structure ResourceProfile where
  time : Nat → Nat
  space : Nat → Nat

/-- A more explicit operational model interface: result, time, and space columns.
This is still abstract, but it is the right slot for a later Turing-machine or RAM model. -/
structure OperationalModel (Input Output : Type) where
  run : Input → Output
  time : Input → Nat
  space : Input → Nat

/-- The resource profile induced by an operational model and an input-size function, using
worst-case exact fibers: `timeAt N` is the supremum over inputs of size exactly `N` when supplied
as a separate bound function. This file keeps the bound function explicit instead of computing
a finite maximum for arbitrary input types. -/
structure WorstCaseResourceBound (Input Output : Type) (size : Input → Nat)
    (M : OperationalModel Input Output) where
  timeAt : Nat → Nat
  spaceAt : Nat → Nat
  time_bound : ∀ x, M.time x ≤ timeAt (size x)
  space_bound : ∀ x, M.space x ≤ spaceAt (size x)

/-- A worst-case resource bound as a `ResourceProfile`. -/
def WorstCaseResourceBound.profile {Input Output : Type} {size : Input → Nat}
    {M : OperationalModel Input Output} (B : WorstCaseResourceBound Input Output size M) :
    ResourceProfile where
  time := B.timeAt
  space := B.spaceAt

/-- An operational model runs in big-O time relative to a size function. -/
def ModelTimeBigO {Input Output : Type} (size : Input → Nat)
    (M : OperationalModel Input Output) (scale : Nat → Nat) : Prop :=
  ∃ C N0 : Nat, ∀ x : Input, N0 ≤ size x → M.time x ≤ C * scale (size x)

/-- An operational model runs in big-O space relative to a size function. -/
def ModelSpaceBigO {Input Output : Type} (size : Input → Nat)
    (M : OperationalModel Input Output) (scale : Nat → Nat) : Prop :=
  ∃ C N0 : Nat, ∀ x : Input, N0 ≤ size x → M.space x ≤ C * scale (size x)

/-- Combined time/space big-O for an operational model. -/
def ModelResourceBigO {Input Output : Type} (size : Input → Nat)
    (M : OperationalModel Input Output) (timeScale spaceScale : Nat → Nat) : Prop :=
  ModelTimeBigO size M timeScale ∧ ModelSpaceBigO size M spaceScale

/-- A minimal measured-machine interface. Later this can be refined into a genuine
single-tape or multi-tape Turing machine; for now it gives the accountant a concrete target:
configurations, transition, halting, decoding, time, and space. -/
structure MeasuredMachine (Input Output Config : Type) where
  init : Input → Config
  step : Config → Config
  halted : Config → Bool
  output : Config → Output
  run : Input → Output
  time : Input → Nat
  space : Input → Nat
  run_correct : ∀ x, run x = output ((step^[time x]) (init x))
  halts_at_time : ∀ x, halted ((step^[time x]) (init x)) = true

/-- Every measured machine induces an operational model. -/
def MeasuredMachine.toOperationalModel {Input Output Config : Type}
    (M : MeasuredMachine Input Output Config) : OperationalModel Input Output where
  run := M.run
  time := M.time
  space := M.space

/-- A measured machine satisfies a time big-O envelope exactly when its induced model does. -/
theorem ModelTimeBigO_of_machine {Input Output Config : Type} {size : Input → Nat}
    {M : MeasuredMachine Input Output Config} {scale : Nat → Nat}
    (h : ModelTimeBigO size M.toOperationalModel scale) :
    ModelTimeBigO size M.toOperationalModel scale := h

/-- Any worst-case profile big-O bound transfers back to every concrete input. -/
theorem ModelTimeBigO_of_worstCaseProfile {Input Output : Type} {size : Input → Nat}
    {M : OperationalModel Input Output} (B : WorstCaseResourceBound Input Output size M)
    {scale : Nat → Nat} (hO : BigO B.timeAt scale) :
    ModelTimeBigO size M scale := by
  obtain ⟨C, N0, hO⟩ := hO
  refine ⟨C, N0, fun x hx => ?_⟩
  exact (B.time_bound x).trans (hO (size x) hx)

/-- Any worst-case profile space big-O bound transfers back to every concrete input. -/
theorem ModelSpaceBigO_of_worstCaseProfile {Input Output : Type} {size : Input → Nat}
    {M : OperationalModel Input Output} (B : WorstCaseResourceBound Input Output size M)
    {scale : Nat → Nat} (hO : BigO B.spaceAt scale) :
    ModelSpaceBigO size M scale := by
  obtain ⟨C, N0, hO⟩ := hO
  refine ⟨C, N0, fun x hx => ?_⟩
  exact (B.space_bound x).trans (hO (size x) hx)

/-- View a step-counting algorithm as an operational model by supplying a space counter. -/
def Algorithm.toOperationalModel {Input Output : Type} (A : Algorithm Input Output)
    (space : Input → Nat) : OperationalModel Input Output where
  run := A.run
  time := A.steps
  space := space

@[simp] theorem Algorithm.toOperationalModel_time {Input Output : Type}
    (A : Algorithm Input Output) (space : Input → Nat) (x : Input) :
    (A.toOperationalModel space).time x = A.steps x := rfl

@[simp] theorem Algorithm.toOperationalModel_run {Input Output : Type}
    (A : Algorithm Input Output) (space : Input → Nat) (x : Input) :
    (A.toOperationalModel space).run x = A.run x := rfl

/-- The algorithm decides a predicate. -/
def Decides {Input : Type} (A : Algorithm Input Bool) (L : Input → Prop) : Prop :=
  ∀ x : Input, A.run x = true ↔ L x

/-- A worst-case step bound: it holds on *every* input. -/
def StepsBoundedBy {Input Output : Type} (size : Input → Nat)
    (A : Algorithm Input Output) (bound : Nat → Nat) : Prop :=
  ∀ x : Input, A.steps x ≤ bound (size x)

/-- Worst-case polynomial time. -/
def PolyTimeAlgorithm {Input Output : Type} (size : Input → Nat)
    (A : Algorithm Input Output) : Prop :=
  ∃ C k : Nat, StepsBoundedBy size A (fun N => C * (N + 1) ^ k)

/-- Worst-case big-O step bound for an algorithm, measured as a function of input size. -/
def StepsBigOBy {Input Output : Type} (size : Input → Nat)
    (A : Algorithm Input Output) (scale : Nat → Nat) : Prop :=
  ∃ C N0 : Nat, ∀ x : Input, N0 ≤ size x → A.steps x ≤ C * scale (size x)

/-- Big-O step bounds are exactly big-O time bounds for the induced operational model. -/
theorem ModelTimeBigO_of_StepsBigOBy {Input Output : Type} {size : Input → Nat}
    {A : Algorithm Input Output} {scale : Nat → Nat} (h : StepsBigOBy size A scale)
    (space : Input → Nat) :
    ModelTimeBigO size (A.toOperationalModel space) scale := by
  exact h

/-- A size→cost function is polynomially bounded. -/
def PolyBound (f : Nat → Nat) : Prop :=
  ∃ C k : Nat, ∀ N, f N ≤ C * (N + 1) ^ k

/-- A function is in `O(1)`. -/
def ConstO (f : Nat → Nat) : Prop :=
  BigO f (fun _ => 1)

/-- A function is in `O(N)`. -/
def LinearO (f : Nat → Nat) : Prop :=
  BigO f (fun N => N + 1)

/-- A function is in `O(N^k)` for some fixed `k`. -/
def PolynomialO (f : Nat → Nat) : Prop :=
  ∃ k : Nat, BigO f (fun N => (N + 1) ^ k)

/-- A function is in singly-exponential time with polynomial prefactor. -/
def ExponentialO (f : Nat → Nat) : Prop :=
  ∃ k : Nat, BigO f (fun N => (N + 1) ^ k * 2 ^ N)

/-! ### Algorithmic SAT views

These definitions make the common algorithmic approaches first-class: faster exponential
envelopes, tractable subclasses, parameterized/FPT reasoning, and circuit-size views. -/

/-- A SAT algorithm with an explicit asymptotic step envelope. -/
def SATAlgorithmWithEnvelope (A : Algorithm CoverInput Bool) (scale : Nat → Nat) : Prop :=
  Decides A CoverSAT ∧ StepsBigOBy CoverInput.size A scale

/-- The coarse full vertex-scan envelope in terms of compact input size. -/
def FullVertexScanEnvelope (N : Nat) : Nat :=
  (N + 1) * 2 ^ N

/-- An algorithmic envelope that is at most full vertex scan up to constants. -/
def AtMostFullVertexScan (scale : Nat → Nat) : Prop :=
  BigO scale FullVertexScanEnvelope

/-- A faster-exponential research candidate: a correct SAT algorithm with a supplied
singly-exponential envelope that is no worse than the full vertex-scan envelope. This does
not assert a concrete base such as `1.3^n`; that can be modeled by choosing `scale`. -/
def FasterExponentialSATCandidate (A : Algorithm CoverInput Bool) (scale : Nat → Nat) : Prop :=
  SATAlgorithmWithEnvelope A scale ∧ ExponentialO scale ∧ AtMostFullVertexScan scale

/-- A semantic subclass of compact SAT-cover inputs. Examples later: Horn-like, 2-SAT-like,
affine/XOR-like, bounded-treewidth, or normalized-gadget families. -/
structure SATSubclass where
  member : CoverInput → Prop

/-- An algorithm decides a language on a subclass. Outside the subclass it may behave
arbitrarily. -/
def DecidesOn {Input : Type} (A : Algorithm Input Bool) (L C : Input → Prop) : Prop :=
  ∀ x : Input, C x → (A.run x = true ↔ L x)

/-- A tractable SAT subclass has a polynomial-time algorithm that is correct on that class. -/
def TractableSATSubclass (C : SATSubclass) : Prop :=
  ∃ A : Algorithm CoverInput Bool, PolyTimeAlgorithm CoverInput.size A ∧
    DecidesOn A CoverSAT C.member

/-- Tractable-class correctness is available whenever the input is in the class. -/
theorem TractableSATSubclass.correct_on {C : SATSubclass} (hC : TractableSATSubclass C)
    {I : CoverInput} (hI : C.member I) :
    ∃ A : Algorithm CoverInput Bool, A.run I = true ↔ CoverSAT I := by
  obtain ⟨A, -, hdec⟩ := hC
  exact ⟨A, hdec I hI⟩

/-- Fixed-parameter tractability schema: time is `f(parameter) * poly(size)`. -/
def FPTAlgorithm {Input Output : Type} (size param : Input → Nat)
    (A : Algorithm Input Output) : Prop :=
  ∃ f : Nat → Nat, ∃ C k : Nat,
    ∀ x : Input, A.steps x ≤ f (param x) * (C * (size x + 1) ^ k)

/-- A parameterized SAT class gives each input a structural parameter such as backdoor size,
treewidth, proof width, or separator size. -/
structure ParameterizedSATClass where
  member : CoverInput → Prop
  parameter : CoverInput → Nat

/-- A parameterized class is fixed-parameter tractable when one algorithm is correct on the
class and FPT in the chosen parameter. -/
def FPTSATSubclass (C : ParameterizedSATClass) : Prop :=
  ∃ A : Algorithm CoverInput Bool, FPTAlgorithm CoverInput.size C.parameter A ∧
    DecidesOn A CoverSAT C.member

/-- Polynomial-size circuit-family route for compact SAT-cover inputs. -/
def PolynomialSizeCircuitFamily (C : CircuitFamily CoverInput) : Prop :=
  PolynomialO C.gateCount

/-- A circuit-family solution to a subclass: polynomial-size circuits decide SAT correctly
on inputs in that class. -/
def CircuitSolvesSATSubclass (C : SATSubclass) (F : CircuitFamily CoverInput) : Prop :=
  PolynomialSizeCircuitFamily F ∧
    ∀ I : CoverInput, C.member I → (F.eval (CoverInput.size I) I = true ↔ CoverSAT I)

/-- Time usage is big-O of a scale. -/
def TimeO (R : ResourceProfile) (scale : Nat → Nat) : Prop :=
  BigO R.time scale

/-- Space usage is big-O of a scale. -/
def SpaceO (R : ResourceProfile) (scale : Nat → Nat) : Prop :=
  BigO R.space scale

/-- A combined time/space envelope. -/
def ResourceO (R : ResourceProfile) (timeScale spaceScale : Nat → Nat) : Prop :=
  TimeO R timeScale ∧ SpaceO R spaceScale

/-- A linear-time, constant-space resource profile. -/
def LinearTimeConstSpace (R : ResourceProfile) : Prop :=
  TimeO R (fun N => N + 1) ∧ SpaceO R (fun _ => 1)

/-- A polynomial-time, polynomial-space resource profile. -/
def PolyTimePolySpaceProfile (R : ResourceProfile) : Prop :=
  PolynomialO R.time ∧ PolynomialO R.space

/-- Pointwise domination by a constant multiple implies big-O. -/
theorem BigO.of_forall_le_mul {f g : Nat → Nat} {C : Nat}
    (h : ∀ N, f N ≤ C * g N) : BigO f g := by
  exact ⟨C, 0, fun N _ => h N⟩

/-- Big-O is reflexive. -/
theorem BigO.refl (f : Nat → Nat) : BigO f f := by
  refine ⟨1, 0, fun N _ => ?_⟩
  simp

/-- Big-O is transitive. -/
theorem BigO.trans {f g h : Nat → Nat} (hfg : BigO f g) (hgh : BigO g h) :
    BigO f h := by
  obtain ⟨C1, N1, h1⟩ := hfg
  obtain ⟨C2, N2, h2⟩ := hgh
  refine ⟨C1 * C2, max N1 N2, fun N hN => ?_⟩
  have hN1 : N1 ≤ N := le_trans (Nat.le_max_left _ _) hN
  have hN2 : N2 ≤ N := le_trans (Nat.le_max_right _ _) hN
  calc
    f N ≤ C1 * g N := h1 N hN1
    _ ≤ C1 * (C2 * h N) := Nat.mul_le_mul_left C1 (h2 N hN2)
    _ = (C1 * C2) * h N := by ring

/-- Big-O is closed under adding two costs with the same envelope. This is the first
accounting algebra rule: sequential composition adds operation ledgers. -/
theorem BigO.add {f g h : Nat → Nat} (hf : BigO f h) (hg : BigO g h) :
    BigO (fun N => f N + g N) h := by
  obtain ⟨C1, N1, h1⟩ := hf
  obtain ⟨C2, N2, h2⟩ := hg
  refine ⟨C1 + C2, max N1 N2, fun N hN => ?_⟩
  have hN1 : N1 ≤ N := le_trans (Nat.le_max_left _ _) hN
  have hN2 : N2 ≤ N := le_trans (Nat.le_max_right _ _) hN
  calc
    f N + g N ≤ C1 * h N + C2 * h N :=
      Nat.add_le_add (h1 N hN1) (h2 N hN2)
    _ = (C1 + C2) * h N := by ring

/-- A dominated sub-cost may be added to a larger accounted cost without leaving the envelope. -/
theorem BigO.add_left {f g h : Nat → Nat} (hf : BigO f h) (hg : BigO g h) :
    BigO (fun N => g N + f N) h := by
  simpa [Nat.add_comm] using BigO.add hg hf

/-- Reflexivity for lower-bound asymptotics. -/
theorem BigOmega.refl (f : Nat → Nat) : BigOmega f f :=
  BigO.refl f

/-- Reflexivity for tight asymptotics. -/
theorem BigTheta.refl (f : Nat → Nat) : BigTheta f f :=
  ⟨BigO.refl f, BigO.refl f⟩

/-- Tight asymptotics are symmetric. -/
theorem BigTheta.symm {f g : Nat → Nat} (h : BigTheta f g) : BigTheta g f :=
  ⟨h.2, h.1⟩

/-- Tight asymptotics are transitive. -/
theorem BigTheta.trans {f g h : Nat → Nat} (hfg : BigTheta f g) (hgh : BigTheta g h) :
    BigTheta f h :=
  ⟨BigO.trans hfg.1 hgh.1, BigO.trans hgh.2 hfg.2⟩

/-- Polynomial bounds imply polynomial big-O. -/
theorem PolynomialO_of_PolyBound {f : Nat → Nat} (h : PolyBound f) :
    PolynomialO f := by
  obtain ⟨C, k, hk⟩ := h
  exact ⟨k, BigO.of_forall_le_mul hk⟩

/-- A polynomial-time algorithm has a big-O polynomial step bound. -/
theorem StepsBigOBy_of_polyTime {Input Output : Type} (size : Input → Nat)
    (A : Algorithm Input Output) (h : PolyTimeAlgorithm size A) :
    ∃ k : Nat, StepsBigOBy size A (fun N => (N + 1) ^ k) := by
  obtain ⟨C, k, hb⟩ := h
  exact ⟨k, C, 0, fun x _ => hb x⟩

/-- A size→cost function is at most exponential: a (polynomial) coefficient times `2 ^ N`.

Note: the naïve `∃ C, ∀ N, f N ≤ C * 2 ^ (N + 1)` is too tight — it excludes even
`fun N => N * 2 ^ N`, since the linear factor `N` cannot be absorbed into a constant `C`.
Allowing a polynomial coefficient is the standard "at most singly-exponential" shape and keeps
`fun N => N * 2 ^ N` (brute force) on the exponential side. -/
def ExpTimeBound (f : Nat → Nat) : Prop :=
  ∃ C k : Nat, ∀ N, f N ≤ C * (N + 1) ^ k * 2 ^ N

/-- The older exponential-bound shape implies the big-O exponential predicate. -/
theorem ExponentialO_of_ExpTimeBound {f : Nat → Nat} (h : ExpTimeBound f) :
    ExponentialO f := by
  obtain ⟨C, k, hk⟩ := h
  refine ⟨k, C, 0, fun N _ => ?_⟩
  calc
    f N ≤ C * (N + 1) ^ k * 2 ^ N := hk N
    _ = C * ((N + 1) ^ k * 2 ^ N) := by ring

/-- A polynomial-time algorithm has a polynomially-bounded composed cost `size ↦ steps`. -/
theorem PolyBound_of_polyTime {Input Output : Type} (size : Input → Nat)
    (A : Algorithm Input Output) (h : PolyTimeAlgorithm size A) :
    ∀ x, ∃ C k : Nat, A.steps x ≤ C * (size x + 1) ^ k := by
  obtain ⟨C, k, hb⟩ := h
  exact fun x => ⟨C, k, hb x⟩

/-- A correct decider for hypercube-cover SAT. -/
def CorrectSATDecider (A : Algorithm CoverInput Bool) : Prop :=
  Decides A CoverSAT

/-- The 3-SAT / hypercube-cover form of "3-SAT ∈ P" (i.e. P = NP via NP-completeness). -/
def ThreeSATInP : Prop :=
  ∃ A : Algorithm CoverInput Bool,
    CorrectSATDecider A ∧ PolyTimeAlgorithm CoverInput.size A

/-- The 3-SAT / hypercube-cover form of "3-SAT ∉ P" (the 3-SAT route to P ≠ NP). -/
def ThreeSATNotInP : Prop := ¬ ThreeSATInP

/-! ### Project-level P-vs-NP-shaped targets -/

/-- Internal project form of the 3-SAT route to P ≠ NP. This is deliberately the uniform
algorithmic statement, not the stronger nonuniform circuit statement. -/
def PvsNP_3SAT_Form : Prop :=
  ThreeSATNotInP

/-- Fully expanded version of the project-level target. -/
def PvsNP_3SAT_Form_Expanded : Prop :=
  ¬ ∃ A : Algorithm CoverInput Bool,
    (∀ I : CoverInput, A.run I = true ↔ CoverSAT I) ∧
    ∃ C k : Nat,
      ∀ I : CoverInput, A.steps I ≤ C * (CoverInput.size I + 1) ^ k

/-- The compact and expanded project-level statements are equivalent. -/
theorem PvsNP_3SAT_Form_iff_expanded :
    PvsNP_3SAT_Form ↔ PvsNP_3SAT_Form_Expanded := by
  unfold PvsNP_3SAT_Form PvsNP_3SAT_Form_Expanded ThreeSATNotInP ThreeSATInP
    CorrectSATDecider Decides PolyTimeAlgorithm StepsBoundedBy
  rfl

/-! ## §D Brute force and the verifier (proven) -/

/-- Brute-force decider: search every vertex of the cube for an uncovered one. -/
def bruteForce : Algorithm CoverInput Bool where
  run I := decide (CoverSAT I)
  steps I := I.blockers.length * 2 ^ I.n

/-- The brute-force decider is correct. -/
theorem bruteForce_correct : CorrectSATDecider bruteForce := by
  intro I; simp [bruteForce]

/-- Exponential upper bound on the brute-force step count. -/
theorem bruteForce_bound (I : CoverInput) :
    bruteForce.steps I ≤ I.blockers.length * 2 ^ I.n :=
  le_refl _

/-- The brute-force step counter is exactly the full blocker-by-vertex scan rectangle. -/
theorem bruteForce_steps_eq_fullVertexScanCost (I : CoverInput) :
    bruteForce.steps I = fullVertexScanCost I := by
  simp [bruteForce, fullVertexScanCost_eq]

/-- The NP verifier: given a candidate vertex, check that it is uncovered. -/
def verifyUncovered {n : Nat} (Bs : List (Blocker n)) (v : Vertex n) : Bool :=
  decide (IsUncovered Bs v)

/-- The verifier is correct. -/
theorem verifyUncovered_correct {n : Nat} (Bs : List (Blocker n)) (v : Vertex n) :
    verifyUncovered Bs v = true ↔ IsUncovered Bs v := by
  simp [verifyUncovered]

/-- A produced uncovered vertex witnesses satisfiability (the NP side). -/
theorem cover_sat_of_uncovered {I : CoverInput} (v : Vertex I.n)
    (h : verifyUncovered I.blockers v = true) : CoverSAT I :=
  ⟨v, (verifyUncovered_correct _ _).mp h⟩

/-- Verification cost: one check per blocker, i.e. `O(m)`. -/
def verifySteps {n : Nat} (Bs : List (Blocker n)) : Nat := Bs.length

theorem verify_poly {n : Nat} (Bs : List (Blocker n)) :
    verifySteps Bs ≤ Bs.length :=
  le_refl _

/-- Operation-boundary fact: the verifier's cost `N ↦ N` is polynomial. -/
theorem verify_steps_poly : PolyBound (fun N => N) := by
  refine ⟨1, 1, fun N => ?_⟩
  simp only [pow_one, one_mul]
  omega

/-- In big-O notation, verifier cost is linear. -/
theorem verify_steps_linearO : LinearO (fun N => N) := by
  unfold LinearO
  refine ⟨1, 0, fun N _ => ?_⟩
  simp

/-- The verifier has a linear-time, constant-space abstract resource profile. -/
def verifierProfile : ResourceProfile where
  time := fun N => N
  space := fun _ => 1

theorem verifierProfile_linear_time_const_space :
    LinearTimeConstSpace verifierProfile := by
  constructor
  · exact verify_steps_linearO
  · unfold SpaceO verifierProfile
    exact BigO.refl (fun _ => 1)

/-- Operation-boundary fact: brute force's cost `N ↦ N · 2ᴺ` is at most exponential. -/
theorem brute_force_steps_exp : ExpTimeBound (fun N => N * 2 ^ N) := by
  refine ⟨1, 1, fun N => ?_⟩
  calc N * 2 ^ N ≤ (N + 1) * 2 ^ N := by gcongr; omega
    _ = 1 * (N + 1) ^ 1 * 2 ^ N := by ring

/-- In big-O notation, brute force is singly exponential with linear prefactor. -/
theorem brute_force_steps_exponentialO : ExponentialO (fun N => N * 2 ^ N) :=
  ExponentialO_of_ExpTimeBound brute_force_steps_exp

/-- The brute-force decider's abstract resource profile. -/
def bruteForceProfile : ResourceProfile where
  time := fun N => N * 2 ^ N
  space := fun N => N

theorem bruteForceProfile_exponential_time_linear_space :
    TimeO bruteForceProfile (fun N => (N + 1) ^ 1 * 2 ^ N) ∧
      SpaceO bruteForceProfile (fun N => N + 1) := by
  constructor
  · unfold TimeO bruteForceProfile
    refine ⟨1, 0, fun N _ => ?_⟩
    calc
      N * 2 ^ N ≤ (N + 1) * 2 ^ N := by gcongr; omega
      _ = 1 * ((N + 1) ^ 1 * 2 ^ N) := by ring
  · unfold SpaceO bruteForceProfile
    exact verify_steps_linearO

/-! ## §E Patterns: width and the polynomial/exponential boundary -/

/-- A partial pattern: a consistent finite set of fixed coordinate/bit pairs. -/
structure Pattern (n : Nat) where
  fixed : Finset (Fin n × Bool)
  consistent : ∀ i : Fin n, ¬ (((i, true) ∈ fixed) ∧ ((i, false) ∈ fixed))
  deriving DecidableEq

/-- The width of a pattern is the number of fixed coordinate/bit pairs. -/
def Pattern.width {n : Nat} (P : Pattern n) : Nat := P.fixed.card

/-- A vertex is covered by a pattern when it agrees on every fixed pair. -/
def Pattern.Covers {n : Nat} (P : Pattern n) (v : Vertex n) : Prop :=
  ∀ ib ∈ P.fixed, v ib.1 = ib.2

instance {n : Nat} (P : Pattern n) (v : Vertex n) : Decidable (P.Covers v) := by
  unfold Pattern.Covers
  infer_instance

/-- A pattern fixes coordinate `i` to bit `b`. -/
def Pattern.Fixes {n : Nat} (P : Pattern n) (i : Fin n) (b : Bool) : Prop :=
  (i, b) ∈ P.fixed

/-- A coordinate is free in a pattern when neither Boolean value is fixed there. -/
def Pattern.Free {n : Nat} (P : Pattern n) (i : Fin n) : Prop :=
  (i, true) ∉ P.fixed ∧ (i, false) ∉ P.fixed

instance {n : Nat} (P : Pattern n) (i : Fin n) : Decidable (P.Free i) := by
  unfold Pattern.Free
  infer_instance

/-- Assigning a free coordinate intersects a face with one Boolean half. -/
def Pattern.assign {n : Nat} (P : Pattern n) (i : Fin n) (b : Bool)
    (hfree : P.Free i) : Pattern n where
  fixed := insert (i, b) P.fixed
  consistent := by
    intro j hbad
    rcases hbad with ⟨ht, hf⟩
    rw [Finset.mem_insert] at ht hf
    rcases ht with htnew | htold
    · rcases hf with hfnew | hfold
      · cases b <;> simp at htnew hfnew
      · rcases Prod.ext_iff.mp htnew with ⟨hji, hb⟩
        change j = i at hji
        change true = b at hb
        subst j
        subst b
        exact hfree.2 hfold
    · rcases hf with hfnew | hfold
      · rcases Prod.ext_iff.mp hfnew with ⟨hji, hb⟩
        change j = i at hji
        change false = b at hb
        subst j
        subst b
        exact hfree.1 htold
      · exact P.consistent j ⟨htold, hfold⟩

/-- Semantic meaning of assigning a free coordinate: cover the old pattern and have
the assigned bit. -/
theorem Pattern.assign_covers_iff {n : Nat} (P : Pattern n) (i : Fin n) (b : Bool)
    (hfree : P.Free i) (v : Vertex n) :
    (P.assign i b hfree).Covers v ↔ P.Covers v ∧ v i = b := by
  constructor
  · intro hcov
    constructor
    · intro ib hib
      exact hcov ib (by simp [Pattern.assign, hib])
    · exact hcov (i, b) (by simp [Pattern.assign])
  · rintro ⟨hP, hvb⟩ ib hib
    change ib ∈ insert (i, b) P.fixed at hib
    rw [Finset.mem_insert] at hib
    rcases hib with hibnew | hibold
    · subst ib
      exact hvb
    · exact hP ib hibold

@[simp] theorem Pattern.assign_fixes_self {n : Nat} (P : Pattern n) (i : Fin n)
    (b : Bool) (hfree : P.Free i) :
    (P.assign i b hfree).Fixes i b := by
  simp [Pattern.Fixes, Pattern.assign]

theorem Pattern.assign_fixes_of_fixes {n : Nat} {P : Pattern n} {i j : Fin n}
    {b c : Bool} {hfree : P.Free i} (hfix : P.Fixes j c) :
    (P.assign i b hfree).Fixes j c := by
  change (j, c) ∈ insert (i, b) P.fixed
  exact Finset.mem_insert_of_mem hfix

/-- A free coordinate splits a face into its false and true halves. -/
theorem Pattern.split_by_free {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) (v : Vertex n) :
    P.Covers v →
      (P.assign i false hfree).Covers v ∨ (P.assign i true hfree).Covers v := by
  intro hP
  cases hv : v i
  · left
    exact (P.assign_covers_iff i false hfree v).mpr ⟨hP, hv⟩
  · right
    exact (P.assign_covers_iff i true hfree v).mpr ⟨hP, hv⟩

/-- The coordinate support of a pattern. -/
def Pattern.support {n : Nat} (P : Pattern n) : Finset (Fin n) :=
  P.fixed.image Prod.fst

theorem Pattern.mem_support_iff {n : Nat} (P : Pattern n) (i : Fin n) :
    i ∈ P.support ↔ ∃ b : Bool, (i, b) ∈ P.fixed := by
  unfold Pattern.support
  constructor
  · intro hi
    rcases Finset.mem_image.mp hi with ⟨ib, hib, hfst⟩
    rcases ib with ⟨j, b⟩
    change j = i at hfst
    subst j
    exact ⟨b, hib⟩
  · rintro ⟨b, hib⟩
    exact Finset.mem_image.mpr ⟨(i, b), hib, rfl⟩

theorem Pattern.support_card_le_width {n : Nat} (P : Pattern n) :
    P.support.card ≤ P.width := by
  unfold Pattern.support Pattern.width
  exact Finset.card_image_le

theorem Pattern.width_le_two_mul_support_card {n : Nat} (P : Pattern n) :
    P.width ≤ 2 * P.support.card := by
  unfold Pattern.width Pattern.support
  -- Each support coordinate can contribute at most the two Boolean values.
  have hfib :
      ∀ i ∈ P.fixed.image Prod.fst,
        ((P.fixed.filter fun ib => ib.1 = i).card) ≤ 2 := by
    intro i hi
    have hsub :
        (P.fixed.filter fun ib => ib.1 = i) ⊆
          ({(i, false), (i, true)} : Finset (Fin n × Bool)) := by
      intro ib hib
      rw [Finset.mem_filter] at hib
      rcases ib with ⟨j, b⟩
      rcases hib with ⟨_, hji⟩
      change j = i at hji
      subst j
      cases b <;> simp
    have hle := Finset.card_le_card hsub
    simpa using hle
  calc
    P.fixed.card
        = ∑ i ∈ P.fixed.image Prod.fst, (P.fixed.filter fun ib => ib.1 = i).card := by
          rw [Finset.card_eq_sum_card_fiberwise
            (s := P.fixed) (t := P.fixed.image Prod.fst)
            (f := fun ib : Fin n × Bool => ib.1)]
          intro ib hib
          exact Finset.mem_image.mpr ⟨ib, hib, rfl⟩
    _ ≤ ∑ i ∈ P.fixed.image Prod.fst, (2 : Nat) := by
          exact Finset.sum_le_sum hfib
    _ = 2 * (P.fixed.image Prod.fst).card := by
          rw [Finset.sum_const, smul_eq_mul, Nat.mul_comm]

/-- No consistent pattern can have width larger than two bit-assignments per coordinate. -/
theorem Pattern.width_le_two_mul_n {n : Nat} (P : Pattern n) :
    P.width ≤ 2 * n := by
  have hsupp : P.support.card ≤ n := by
    simpa [Fintype.card_fin] using Finset.card_le_univ P.support
  calc
    P.width ≤ 2 * P.support.card := P.width_le_two_mul_support_card
    _ ≤ 2 * n := Nat.mul_le_mul_left 2 hsupp

/-- The empty pattern covers the whole cube. -/
def Pattern.empty (n : Nat) : Pattern n where
  fixed := ∅
  consistent := by simp

/-- The slice pattern determined by fixing every coordinate in `U` according to assignment `a`.
This is the normal-form slab object behind backdoors and coordinate branching. -/
def Pattern.slice {n : Nat} (U : Finset (Fin n)) (a : Fin n → Bool) : Pattern n where
  fixed := U.image fun i => (i, a i)
  consistent := by
    intro i hbad
    rcases hbad with ⟨ht, hf⟩
    obtain ⟨x, hxU, hx⟩ := Finset.mem_image.mp ht
    obtain ⟨y, hyU, hy⟩ := Finset.mem_image.mp hf
    have hxi : x = i := by
      exact congrArg Prod.fst hx
    have hyi : y = i := by
      exact congrArg Prod.fst hy
    subst x
    subst y
    have htrue : a i = true := by
      exact congrArg Prod.snd hx
    have hfalse : a i = false := by
      exact congrArg Prod.snd hy
    rw [htrue] at hfalse
    simp at hfalse

@[simp] theorem Pattern.mem_slice_fixed {n : Nat} (U : Finset (Fin n))
    (a : Fin n → Bool) (i : Fin n) (b : Bool) :
    (i, b) ∈ (Pattern.slice U a).fixed ↔ i ∈ U ∧ a i = b := by
  constructor
  · intro h
    obtain ⟨j, hjU, hj⟩ := Finset.mem_image.mp h
    have hji : j = i := congrArg Prod.fst hj
    subst j
    exact ⟨hjU, congrArg Prod.snd hj⟩
  · rintro ⟨hiU, hai⟩
    exact Finset.mem_image.mpr ⟨i, hiU, by simp [hai]⟩

/-- Slice coverage is exactly agreement with the chosen partial assignment on `U`. -/
theorem Pattern.slice_covers_iff {n : Nat} (U : Finset (Fin n)) (a : Fin n → Bool)
    (v : Vertex n) :
    (Pattern.slice U a).Covers v ↔ ∀ i : Fin n, i ∈ U → v i = a i := by
  constructor
  · intro hcov i hiU
    exact hcov (i, a i) ((Pattern.mem_slice_fixed U a i (a i)).mpr ⟨hiU, rfl⟩)
  · intro hagree ib hib
    rcases ib with ⟨i, b⟩
    rw [Pattern.mem_slice_fixed] at hib
    exact (hagree i hib.1).trans hib.2

/-- The width of a coordinate slice is exactly the number of fixed coordinates. -/
@[simp] theorem Pattern.slice_width {n : Nat} (U : Finset (Fin n)) (a : Fin n → Bool) :
    (Pattern.slice U a).width = U.card := by
  unfold Pattern.width Pattern.slice
  exact Finset.card_image_of_injective _ (by
    intro i j hij
    exact congrArg Prod.fst hij)

/-- The coordinate support of a slice is exactly its coordinate set. -/
@[simp] theorem Pattern.slice_support {n : Nat} (U : Finset (Fin n)) (a : Fin n → Bool) :
    (Pattern.slice U a).support = U := by
  ext i
  rw [Pattern.mem_support_iff]
  constructor
  · rintro ⟨b, hb⟩
    rw [Pattern.mem_slice_fixed] at hb
    exact hb.1
  · intro hi
    exact ⟨a i, (Pattern.mem_slice_fixed U a i (a i)).mpr ⟨hi, rfl⟩⟩

/-- A non-duplicating assignment for a coordinate slice: only coordinates in `U` are assigned. -/
abbrev SliceAssignment {n : Nat} (U : Finset (Fin n)) := ({i : Fin n // i ∈ U} → Bool)

/-- The number of genuine assignments to a coordinate slice is exactly `2^|U|`. -/
theorem sliceAssignment_card {n : Nat} (U : Finset (Fin n)) :
    Fintype.card (SliceAssignment U) = 2 ^ U.card := by
  classical
  simp [SliceAssignment]

/-- Information cost of branching on a coordinate set: one Boolean slab per assignment on `U`. -/
def sliceBranchCount {n : Nat} (U : Finset (Fin n)) : Nat :=
  2 ^ U.card

/-- Non-duplicating slice pattern from an assignment only on `U`. -/
def Pattern.sliceOn {n : Nat} (U : Finset (Fin n)) (a : SliceAssignment U) : Pattern n where
  fixed := U.attach.image fun i => (i.1, a i)
  consistent := by
    intro i hbad
    rcases hbad with ⟨ht, hf⟩
    obtain ⟨x, _hxmem, hx⟩ := Finset.mem_image.mp ht
    obtain ⟨y, _hymem, hy⟩ := Finset.mem_image.mp hf
    have hxi : x.1 = i := congrArg Prod.fst hx
    have hyi : y.1 = i := congrArg Prod.fst hy
    have hxy : x = y := Subtype.ext (hxi.trans hyi.symm)
    subst y
    have htrue : a x = true := congrArg Prod.snd hx
    have hfalse : a x = false := congrArg Prod.snd hy
    rw [htrue] at hfalse
    simp at hfalse

@[simp] theorem Pattern.mem_sliceOn_fixed {n : Nat} (U : Finset (Fin n))
    (a : SliceAssignment U) (i : Fin n) (b : Bool) :
    (i, b) ∈ (Pattern.sliceOn U a).fixed ↔ ∃ hi : i ∈ U, a ⟨i, hi⟩ = b := by
  constructor
  · intro h
    obtain ⟨x, _hxmem, hx⟩ := Finset.mem_image.mp h
    have hxi : x.1 = i := congrArg Prod.fst hx
    subst i
    exact ⟨x.2, congrArg Prod.snd hx⟩
  · rintro ⟨hi, hb⟩
    exact Finset.mem_image.mpr ⟨⟨i, hi⟩, by simp, by simp [hb]⟩

/-- Restricted slice coverage is agreement with the restricted assignment on `U`. -/
theorem Pattern.sliceOn_covers_iff {n : Nat} (U : Finset (Fin n)) (a : SliceAssignment U)
    (v : Vertex n) :
    (Pattern.sliceOn U a).Covers v ↔ ∀ i : Fin n, ∀ hi : i ∈ U, v i = a ⟨i, hi⟩ := by
  constructor
  · intro hcov i hi
    exact hcov (i, a ⟨i, hi⟩)
      ((Pattern.mem_sliceOn_fixed U a i (a ⟨i, hi⟩)).mpr ⟨hi, rfl⟩)
  · intro hagree ib hib
    rcases ib with ⟨i, b⟩
    rw [Pattern.mem_sliceOn_fixed] at hib
    obtain ⟨hi, hb⟩ := hib
    exact (hagree i hi).trans hb

/-- A restricted slice has width exactly `|U|`. -/
@[simp] theorem Pattern.sliceOn_width {n : Nat} (U : Finset (Fin n)) (a : SliceAssignment U) :
    (Pattern.sliceOn U a).width = U.card := by
  unfold Pattern.width Pattern.sliceOn
  rw [Finset.card_image_of_injective]
  · simp
  · intro x y hxy
    exact Subtype.ext (congrArg Prod.fst hxy)

/-- A restricted slice has support exactly `U`. -/
@[simp] theorem Pattern.sliceOn_support {n : Nat} (U : Finset (Fin n)) (a : SliceAssignment U) :
    (Pattern.sliceOn U a).support = U := by
  ext i
  rw [Pattern.mem_support_iff]
  constructor
  · rintro ⟨b, hb⟩
    rw [Pattern.mem_sliceOn_fixed] at hb
    exact hb.1
  · intro hi
    exact ⟨a ⟨i, hi⟩, (Pattern.mem_sliceOn_fixed U a i (a ⟨i, hi⟩)).mpr ⟨hi, rfl⟩⟩

/-- The branch-count notation agrees with the actual finite type of restricted slice assignments. -/
theorem sliceBranchCount_eq_sliceAssignment_card {n : Nat} (U : Finset (Fin n)) :
    sliceBranchCount U = Fintype.card (SliceAssignment U) := by
  rw [sliceBranchCount, sliceAssignment_card]

@[simp] theorem Pattern.empty_covers {n : Nat} (v : Vertex n) :
    (Pattern.empty n).Covers v := by
  intro ib hib
  simp [Pattern.empty] at hib

@[simp] theorem Pattern.empty_width {n : Nat} :
    (Pattern.empty n).width = 0 := by
  simp [Pattern.width, Pattern.empty]

/-- Syntactic pattern order: fewer fixed pairs means a larger subcube. -/
def Pattern.Le {n : Nat} (P Q : Pattern n) : Prop :=
  P.fixed ⊆ Q.fixed

/-- Assigning a coordinate refines the parent face. -/
theorem Pattern.le_assign {n : Nat} (P : Pattern n) (i : Fin n) (b : Bool)
    (hfree : P.Free i) :
    P.Le (P.assign i b hfree) := by
  intro ib hib
  exact Finset.mem_insert_of_mem hib

/-- Alias for `Pattern.Le`, matching the geometric language: `Q` refines `P` when
`P.fixed ⊆ Q.fixed`. -/
def Pattern.Refines {n : Nat} (P Q : Pattern n) : Prop :=
  P.Le Q

/-- Two patterns are compatible when they never force opposite bits on the same coordinate. -/
def Pattern.Compatible {n : Nat} (P Q : Pattern n) : Prop :=
  ∀ i : Fin n, ¬ (((i, true) ∈ P.fixed ∧ (i, false) ∈ Q.fixed) ∨
                 ((i, false) ∈ P.fixed ∧ (i, true) ∈ Q.fixed))

/-- Two patterns are incompatible when some coordinate is forced to opposite bits. -/
def Pattern.Incompatible {n : Nat} (P Q : Pattern n) : Prop :=
  ∃ i : Fin n, (((i, true) ∈ P.fixed ∧ (i, false) ∈ Q.fixed) ∨
                ((i, false) ∈ P.fixed ∧ (i, true) ∈ Q.fixed))

theorem Pattern.not_compatible_iff_incompatible {n : Nat} (P Q : Pattern n) :
    ¬ P.Compatible Q ↔ P.Incompatible Q := by
  unfold Pattern.Compatible Pattern.Incompatible
  push Not
  rfl

/-- Pattern compatibility is symmetric. -/
theorem Pattern.Compatible.symm {n : Nat} {P Q : Pattern n} (h : P.Compatible Q) :
    Q.Compatible P := by
  intro i hbad
  rcases hbad with htf | hft
  · exact h i (Or.inr ⟨htf.2, htf.1⟩)
  · exact h i (Or.inl ⟨hft.2, hft.1⟩)

/-- Pattern incompatibility is symmetric. -/
theorem Pattern.Incompatible.symm {n : Nat} {P Q : Pattern n} (h : P.Incompatible Q) :
    Q.Incompatible P := by
  rcases h with ⟨i, hbad⟩
  refine ⟨i, ?_⟩
  rcases hbad with htf | hft
  · exact Or.inr ⟨htf.2, htf.1⟩
  · exact Or.inl ⟨hft.2, hft.1⟩

/-- If `P` fixes a subset of `Q`'s coordinates/bits, then every vertex covered by `Q`
is covered by `P`. This is geometric subsumption. -/
theorem pattern_covers_of_subset {n : Nat} {P Q : Pattern n} (h : P.fixed ⊆ Q.fixed) :
    ∀ v, Q.Covers v → P.Covers v := by
  intro v hQ ib hib
  exact hQ ib (h hib)

/-- Pattern subsumption expressed with `Pattern.Le`. -/
theorem Pattern.covers_of_le {n : Nat} {P Q : Pattern n} (h : P.Le Q) :
    ∀ v, Q.Covers v → P.Covers v :=
  pattern_covers_of_subset h

/-- Every assigned child lies inside the parent face. -/
theorem Pattern.covers_of_assign {n : Nat} (P : Pattern n) (i : Fin n) (b : Bool)
    (hfree : P.Free i) :
    ∀ v : Vertex n, (P.assign i b hfree).Covers v → P.Covers v :=
  Pattern.covers_of_le (Pattern.le_assign P i b hfree)

theorem Pattern.covers_of_refines {n : Nat} {P Q : Pattern n} (h : P.Refines Q) :
    ∀ v, Q.Covers v → P.Covers v :=
  Pattern.covers_of_le h

/-! ### §E0 Information-theoretic face scores -/

/-- The finite set of vertices in a face/pattern. This is the remaining search mass if
we ignore blockers and look only at the geometric face. -/
def Pattern.vertices {n : Nat} (P : Pattern n) : Finset (Vertex n) :=
  (allVertices n).filter fun v => P.Covers v

@[simp] theorem Pattern.mem_vertices {n : Nat} (P : Pattern n) (v : Vertex n) :
    v ∈ P.vertices ↔ P.Covers v := by
  simp [Pattern.vertices]

/-- Cardinality mass of a face. Information heuristics try to shrink this mass quickly. -/
def Pattern.mass {n : Nat} (P : Pattern n) : Nat :=
  P.vertices.card

/-- Base-2 uncertainty proxy: the number of bits needed up to the natural-log floor model. -/
def uncertaintyBits (N : Nat) : Nat :=
  Nat.log 2 N

theorem uncertaintyBits_mono {a b : Nat} (h : a ≤ b) :
    uncertaintyBits a ≤ uncertaintyBits b :=
  Nat.log_mono_right h

@[simp] theorem uncertaintyBits_zero :
    uncertaintyBits 0 = 0 := by
  simp [uncertaintyBits]

/-- The mass of one child of a split. -/
def Pattern.childMass {n : Nat} (P : Pattern n) (i : Fin n) (b : Bool)
    (hfree : P.Free i) : Nat :=
  (P.assign i b hfree).mass

theorem Pattern.childMass_le_mass {n : Nat} (P : Pattern n) (i : Fin n) (b : Bool)
    (hfree : P.Free i) :
    P.childMass i b hfree ≤ P.mass := by
  unfold Pattern.childMass Pattern.mass
  apply Finset.card_le_card
  intro v hv
  rw [Pattern.mem_vertices] at hv ⊢
  exact P.covers_of_assign i b hfree v hv

/-- Worst remaining geometric mass after splitting a face on coordinate `i`. -/
def Pattern.splitWorstMass {n : Nat} (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Nat :=
  max (P.childMass i false hfree) (P.childMass i true hfree)

theorem Pattern.splitWorstMass_le_mass {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.splitWorstMass i hfree ≤ P.mass := by
  unfold Pattern.splitWorstMass
  exact max_le (P.childMass_le_mass i false hfree) (P.childMass_le_mass i true hfree)

/-- A free binary split partitions the parent face exactly into its two Boolean children. -/
theorem Pattern.mass_eq_childMass_add_childMass {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.mass = P.childMass i false hfree + P.childMass i true hfree := by
  let P0 := P.assign i false hfree
  let P1 := P.assign i true hfree
  have hvertices : P.vertices = P0.vertices ∪ P1.vertices := by
    apply Finset.ext
    intro v
    constructor
    · intro hv
      rw [Pattern.mem_vertices] at hv
      rcases P.split_by_free i hfree v hv with h0 | h1
      · rw [Finset.mem_union]
        left
        rw [Pattern.mem_vertices]
        exact h0
      · rw [Finset.mem_union]
        right
        rw [Pattern.mem_vertices]
        exact h1
    · intro hv
      rw [Finset.mem_union] at hv
      rw [Pattern.mem_vertices]
      rcases hv with h0 | h1
      · rw [Pattern.mem_vertices] at h0
        exact P.covers_of_assign i false hfree v h0
      · rw [Pattern.mem_vertices] at h1
        exact P.covers_of_assign i true hfree v h1
  have hdisjoint : Disjoint P0.vertices P1.vertices := by
    rw [Finset.disjoint_left]
    intro v hv0 hv1
    rw [Pattern.mem_vertices] at hv0 hv1
    have hfalse : v i = false := (P.assign_covers_iff i false hfree v).mp hv0 |>.2
    have htrue : v i = true := (P.assign_covers_iff i true hfree v).mp hv1 |>.2
    rw [hfalse] at htrue
    contradiction
  calc
    P.mass = (P0.vertices ∪ P1.vertices).card := by
      rw [Pattern.mass, hvertices]
    _ = P0.vertices.card + P1.vertices.card := Finset.card_union_of_disjoint hdisjoint
    _ = P.childMass i false hfree + P.childMass i true hfree := by
      rfl

/-- A binary split cannot make both children smaller than half of the parent face, in the
integer form `mass(parent) ≤ 2 * worstChildMass`. This is the local information-theoretic
lower bound behind branch accounting. -/
theorem Pattern.mass_le_two_mul_splitWorstMass {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.mass ≤ 2 * P.splitWorstMass i hfree := by
  have hsum := P.mass_eq_childMass_add_childMass i hfree
  rw [hsum]
  unfold Pattern.splitWorstMass
  calc
    P.childMass i false hfree + P.childMass i true hfree
        ≤ max (P.childMass i false hfree) (P.childMass i true hfree) +
          max (P.childMass i false hfree) (P.childMass i true hfree) := by
          exact add_le_add (le_max_left _ _) (le_max_right _ _)
    _ = 2 * max (P.childMass i false hfree) (P.childMass i true hfree) := by omega

/-- A free coordinate split partitions a face into its two Boolean children. This is the
exact counting statement behind the information view: one queried bit separates the current
mass into the `false` slab and the `true` slab. -/
theorem Pattern.vertices_split_eq_union {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.vertices =
      (P.assign i false hfree).vertices ∪ (P.assign i true hfree).vertices := by
  ext v
  constructor
  · intro hv
    rw [Pattern.mem_vertices] at hv
    rw [Finset.mem_union]
    rcases P.split_by_free i hfree v hv with hfalse | htrue
    · left
      rw [Pattern.mem_vertices]
      exact hfalse
    · right
      rw [Pattern.mem_vertices]
      exact htrue
  · intro hv
    rw [Finset.mem_union] at hv
    rw [Pattern.mem_vertices]
    rcases hv with hfalse | htrue
    · rw [Pattern.mem_vertices] at hfalse
      exact P.covers_of_assign i false hfree v hfalse
    · rw [Pattern.mem_vertices] at htrue
      exact P.covers_of_assign i true hfree v htrue

/-- The two children of a free-coordinate split are disjoint faces. -/
theorem Pattern.vertices_split_disjoint {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    Disjoint (P.assign i false hfree).vertices (P.assign i true hfree).vertices := by
  rw [Finset.disjoint_left]
  intro v hfalse htrue
  rw [Pattern.mem_vertices, P.assign_covers_iff i false hfree] at hfalse
  rw [Pattern.mem_vertices, P.assign_covers_iff i true hfree] at htrue
  have hf : v i = false := hfalse.2
  have ht : v i = true := htrue.2
  rw [hf] at ht
  simp at ht

/-- Exact mass conservation for a one-coordinate information split. -/
theorem Pattern.mass_eq_childMass_add_childMass_via_vertices {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.mass = P.childMass i false hfree + P.childMass i true hfree := by
  exact P.mass_eq_childMass_add_childMass i hfree

/-! #### Blocker-aware candidate-solution mass -/

/-- A pattern/subcube is bad for a blocker list when it contains no uncovered vertex.
This is the central invariant for a hypercube-native DPLL/CDCL calculus: learned
regions may prune search only when they are bad with respect to the original blockers. -/
def BadRegionFor {n : Nat} (Bs : List (Blocker n)) (P : Pattern n) : Prop :=
  ∀ v : Vertex n, P.Covers v → ¬ IsUncovered Bs v

/-- Vertices inside a face that are still uncovered by the current original blockers.
This is the solver's real candidate set for SAT search inside the face. -/
def Pattern.solutionVertices {n : Nat} (Bs : List (Blocker n)) (P : Pattern n) :
    Finset (Vertex n) :=
  P.vertices.filter fun v => IsUncovered Bs v

@[simp] theorem Pattern.mem_solutionVertices {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (v : Vertex n) :
    v ∈ P.solutionVertices Bs ↔ P.Covers v ∧ IsUncovered Bs v := by
  simp [Pattern.solutionVertices]

/-- Blocker-aware remaining candidate mass inside a face. -/
def Pattern.solutionMass {n : Nat} (Bs : List (Blocker n)) (P : Pattern n) : Nat :=
  (P.solutionVertices Bs).card

theorem Pattern.solutionMass_le_mass {n : Nat} (Bs : List (Blocker n)) (P : Pattern n) :
    P.solutionMass Bs ≤ P.mass := by
  unfold Pattern.solutionMass Pattern.mass Pattern.solutionVertices
  exact Finset.card_filter_le _ _

theorem Pattern.solutionMass_eq_zero_iff_no_solution_in_face {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) :
    P.solutionMass Bs = 0 ↔ ∀ v : Vertex n, P.Covers v → ¬ IsUncovered Bs v := by
  constructor
  · intro hzero v hPv hun
    have hv : v ∈ P.solutionVertices Bs := by
      rw [Pattern.mem_solutionVertices]
      exact ⟨hPv, hun⟩
    have hempty : P.solutionVertices Bs = ∅ := Finset.card_eq_zero.mp hzero
    rw [hempty] at hv
    simp at hv
  · intro hbad
    have hempty : P.solutionVertices Bs = ∅ := by
      ext v
      constructor
      · intro hv
        rw [Pattern.mem_solutionVertices] at hv
        exact False.elim (hbad v hv.1 hv.2)
      · intro hv
        simp at hv
    simp [Pattern.solutionMass, hempty]

theorem Pattern.exists_solution_of_solutionMass_pos {n : Nat} {Bs : List (Blocker n)}
    {P : Pattern n} (hpos : 0 < P.solutionMass Bs) :
    ∃ v : Vertex n, P.Covers v ∧ IsUncovered Bs v := by
  unfold Pattern.solutionMass at hpos
  obtain ⟨v, hv⟩ := Finset.card_pos.mp hpos
  rw [Pattern.mem_solutionVertices] at hv
  exact ⟨v, hv⟩

theorem Pattern.solutionMass_pos_iff_exists_solution {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) :
    0 < P.solutionMass Bs ↔ ∃ v : Vertex n, P.Covers v ∧ IsUncovered Bs v := by
  constructor
  · exact Pattern.exists_solution_of_solutionMass_pos
  · rintro ⟨v, hPv, hun⟩
    unfold Pattern.solutionMass
    apply Finset.card_pos.mpr
    exact ⟨v, by rw [Pattern.mem_solutionVertices]; exact ⟨hPv, hun⟩⟩

/-- SAT inside one coordinate slice. A backdoor branches over these predicates. -/
def SliceSATList {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n))
    (a : Fin n → Bool) : Prop :=
  ∃ v : Vertex n, (Pattern.slice U a).Covers v ∧ IsUncovered Bs v

/-- UNSAT inside one coordinate slice. -/
def SliceUNSATList {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n))
    (a : Fin n → Bool) : Prop :=
  ¬ SliceSATList Bs U a

/-- Candidate-witness mass contained in a coordinate slice. -/
def sliceSolutionMass {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n))
    (a : Fin n → Bool) : Nat :=
  (Pattern.slice U a).solutionMass Bs

/-- A slice is SAT exactly when its candidate-solution mass is positive. -/
theorem sliceSATList_iff_solutionMass_pos {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) (a : Fin n → Bool) :
    SliceSATList Bs U a ↔ 0 < sliceSolutionMass Bs U a := by
  unfold SliceSATList sliceSolutionMass
  rw [Pattern.solutionMass_pos_iff_exists_solution]

/-- A slice is UNSAT exactly when its candidate-solution mass is zero. -/
theorem sliceUNSATList_iff_solutionMass_eq_zero {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) (a : Fin n → Bool) :
    SliceUNSATList Bs U a ↔ sliceSolutionMass Bs U a = 0 := by
  unfold SliceUNSATList
  rw [sliceSATList_iff_solutionMass_pos]
  omega

/-- A bad region is exactly a face with zero candidate-solution mass. -/
theorem BadRegionFor_iff_solutionMass_eq_zero {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) :
    BadRegionFor Bs P ↔ P.solutionMass Bs = 0 := by
  rw [Pattern.solutionMass_eq_zero_iff_no_solution_in_face]
  rfl

/-- A coordinate slice is bad exactly when its candidate-solution mass is zero. -/
theorem slice_bad_iff_solutionMass_eq_zero {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) (a : Fin n → Bool) :
    BadRegionFor Bs (Pattern.slice U a) ↔ sliceSolutionMass Bs U a = 0 := by
  unfold sliceSolutionMass
  exact BadRegionFor_iff_solutionMass_eq_zero Bs (Pattern.slice U a)

/-- Global SAT is equivalent to at least one SAT slice for any chosen coordinate set. -/
theorem coverSATList_iff_exists_sliceSAT {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) :
    CoverSATList Bs ↔ ∃ a : Fin n → Bool, SliceSATList Bs U a := by
  constructor
  · rintro ⟨v, hun⟩
    exact ⟨v, v, (Pattern.slice_covers_iff U v v).mpr (by intro i _; rfl), hun⟩
  · rintro ⟨a, v, _hPv, hun⟩
    exact ⟨v, hun⟩

/-- Global UNSAT is equivalent to every coordinate slice being UNSAT. -/
theorem coverUNSATList_iff_forall_sliceUNSAT {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) :
    CoverUNSATList Bs ↔ ∀ a : Fin n → Bool, SliceUNSATList Bs U a := by
  unfold CoverUNSATList
  rw [coverSATList_iff_exists_sliceSAT Bs U]
  constructor
  · intro h a hsat
    exact h ⟨a, hsat⟩
  · intro h hsat
    obtain ⟨a, ha⟩ := hsat
    exact h a ha

/-- Information form of slice decomposition: UNSAT means every slice has zero solution mass. -/
theorem coverUNSATList_iff_forall_sliceSolutionMass_zero {n : Nat}
    (Bs : List (Blocker n)) (U : Finset (Fin n)) :
    CoverUNSATList Bs ↔ ∀ a : Fin n → Bool, sliceSolutionMass Bs U a = 0 := by
  rw [coverUNSATList_iff_forall_sliceUNSAT Bs U]
  constructor
  · intro h a
    exact (sliceUNSATList_iff_solutionMass_eq_zero Bs U a).mp (h a)
  · intro h a
    exact (sliceUNSATList_iff_solutionMass_eq_zero Bs U a).mpr (h a)

/-- There is at least one live slice for the coordinate set `U`. -/
def HasLiveSliceList {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n)) : Prop :=
  ∃ a : Fin n → Bool, SliceSATList Bs U a

/-- Every slice induced by `U` is a bad region. This is a slice/backdoor UNSAT certificate. -/
def AllSlicesBadList {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n)) : Prop :=
  ∀ a : Fin n → Bool, BadRegionFor Bs (Pattern.slice U a)

/-- Every slice induced by `U` has zero candidate-solution mass. -/
def AllSlicesZeroMassList {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n)) : Prop :=
  ∀ a : Fin n → Bool, sliceSolutionMass Bs U a = 0

/-- Live-slice existence is exactly global satisfiability. -/
theorem hasLiveSliceList_iff_coverSATList {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) :
    HasLiveSliceList Bs U ↔ CoverSATList Bs := by
  unfold HasLiveSliceList
  rw [coverSATList_iff_exists_sliceSAT Bs U]

/-- All-slices-bad is exactly global UNSAT. -/
theorem allSlicesBadList_iff_coverUNSATList {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) :
    AllSlicesBadList Bs U ↔ CoverUNSATList Bs := by
  unfold AllSlicesBadList
  rw [coverUNSATList_iff_forall_sliceUNSAT Bs U]
  constructor
  · intro h a hsat
    obtain ⟨v, hslice, hun⟩ := hsat
    exact h a v hslice hun
  · intro h a v hslice hun
    exact h a ⟨v, hslice, hun⟩

/-- Zero-mass slices are exactly global UNSAT. -/
theorem allSlicesZeroMassList_iff_coverUNSATList {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) :
    AllSlicesZeroMassList Bs U ↔ CoverUNSATList Bs := by
  unfold AllSlicesZeroMassList
  rw [coverUNSATList_iff_forall_sliceSolutionMass_zero Bs U]

/-- All-slices-bad and all-slices-zero-mass are the same certificate. -/
theorem allSlicesBadList_iff_allSlicesZeroMassList {n : Nat}
    (Bs : List (Blocker n)) (U : Finset (Fin n)) :
    AllSlicesBadList Bs U ↔ AllSlicesZeroMassList Bs U := by
  rw [allSlicesBadList_iff_coverUNSATList, allSlicesZeroMassList_iff_coverUNSATList]

/-- Branching on all coordinates gives the full vertex count. -/
@[simp] theorem sliceBranchCount_univ (n : Nat) :
    sliceBranchCount (Finset.univ : Finset (Fin n)) = 2 ^ n := by
  simp [sliceBranchCount]

/-- Any coordinate slice family has at most as many slabs as the full hypercube has vertices. -/
theorem sliceBranchCount_le_vertexCount {n : Nat} (U : Finset (Fin n)) :
    sliceBranchCount U ≤ 2 ^ n := by
  unfold sliceBranchCount
  have hcard : U.card ≤ n := by
    simpa [Fintype.card_fin] using Finset.card_le_univ U
  exact Nat.pow_le_pow_right (by decide : 0 < 2) hcard

/-- A small backdoor of size `k` has at most `2^k` slices to inspect. -/
theorem sliceBranchCount_le_of_card_le {n k : Nat} {U : Finset (Fin n)}
    (hcard : U.card ≤ k) :
    sliceBranchCount U ≤ 2 ^ k := by
  unfold sliceBranchCount
  exact Nat.pow_le_pow_right (by decide : 0 < 2) hcard

/-- Cover-input wrapper for live slices. -/
def CoverInput.HasLiveSlice (I : CoverInput) (U : Finset (Fin I.n)) : Prop :=
  HasLiveSliceList I.blockers U

/-- Cover-input wrapper for all-slices-bad certificates. -/
def CoverInput.AllSlicesBad (I : CoverInput) (U : Finset (Fin I.n)) : Prop :=
  AllSlicesBadList I.blockers U

/-- Cover-input wrapper for all-slices-zero-mass certificates. -/
def CoverInput.AllSlicesZeroMass (I : CoverInput) (U : Finset (Fin I.n)) : Prop :=
  AllSlicesZeroMassList I.blockers U

theorem CoverInput.hasLiveSlice_iff_sat (I : CoverInput) (U : Finset (Fin I.n)) :
    I.HasLiveSlice U ↔ CoverSAT I := by
  unfold CoverInput.HasLiveSlice CoverSAT
  exact hasLiveSliceList_iff_coverSATList I.blockers U

theorem CoverInput.allSlicesBad_iff_unsat (I : CoverInput) (U : Finset (Fin I.n)) :
    I.AllSlicesBad U ↔ CoverUNSAT I := by
  unfold CoverInput.AllSlicesBad CoverUNSAT CoverSAT
  exact allSlicesBadList_iff_coverUNSATList I.blockers U

theorem CoverInput.allSlicesZeroMass_iff_unsat (I : CoverInput) (U : Finset (Fin I.n)) :
    I.AllSlicesZeroMass U ↔ CoverUNSAT I := by
  unfold CoverInput.AllSlicesZeroMass CoverUNSAT CoverSAT
  exact allSlicesZeroMassList_iff_coverUNSATList I.blockers U

/-- SAT inside a non-duplicating restricted slice assignment. -/
def SliceOnSATList {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n))
    (a : SliceAssignment U) : Prop :=
  ∃ v : Vertex n, (Pattern.sliceOn U a).Covers v ∧ IsUncovered Bs v

/-- UNSAT inside a non-duplicating restricted slice assignment. -/
def SliceOnUNSATList {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n))
    (a : SliceAssignment U) : Prop :=
  ¬ SliceOnSATList Bs U a

/-- Candidate-witness mass inside a non-duplicating restricted slice. -/
def sliceOnSolutionMass {n : Nat} (Bs : List (Blocker n)) (U : Finset (Fin n))
    (a : SliceAssignment U) : Nat :=
  (Pattern.sliceOn U a).solutionMass Bs

theorem sliceOnSATList_iff_solutionMass_pos {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) (a : SliceAssignment U) :
    SliceOnSATList Bs U a ↔ 0 < sliceOnSolutionMass Bs U a := by
  unfold SliceOnSATList sliceOnSolutionMass
  rw [Pattern.solutionMass_pos_iff_exists_solution]

theorem sliceOnUNSATList_iff_solutionMass_eq_zero {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) (a : SliceAssignment U) :
    SliceOnUNSATList Bs U a ↔ sliceOnSolutionMass Bs U a = 0 := by
  unfold SliceOnUNSATList
  rw [sliceOnSATList_iff_solutionMass_pos]
  omega

/-- Global SAT is equivalent to at least one genuine restricted slice being SAT. -/
theorem coverSATList_iff_exists_sliceOnSAT {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) :
    CoverSATList Bs ↔ ∃ a : SliceAssignment U, SliceOnSATList Bs U a := by
  constructor
  · rintro ⟨v, hun⟩
    let a : SliceAssignment U := fun i => v i.1
    exact ⟨a, v, (Pattern.sliceOn_covers_iff U a v).mpr (by intro i hi; rfl), hun⟩
  · rintro ⟨a, v, _hPv, hun⟩
    exact ⟨v, hun⟩

/-- Global UNSAT is equivalent to every genuine restricted slice being UNSAT. -/
theorem coverUNSATList_iff_forall_sliceOnUNSAT {n : Nat} (Bs : List (Blocker n))
    (U : Finset (Fin n)) :
    CoverUNSATList Bs ↔ ∀ a : SliceAssignment U, SliceOnUNSATList Bs U a := by
  unfold CoverUNSATList
  rw [coverSATList_iff_exists_sliceOnSAT Bs U]
  constructor
  · intro h a hsat
    exact h ⟨a, hsat⟩
  · intro h hsat
    obtain ⟨a, ha⟩ := hsat
    exact h a ha

/-- Information form over the exact `2^|U|` restricted slice assignments. -/
theorem coverUNSATList_iff_forall_sliceOnSolutionMass_zero {n : Nat}
    (Bs : List (Blocker n)) (U : Finset (Fin n)) :
    CoverUNSATList Bs ↔ ∀ a : SliceAssignment U, sliceOnSolutionMass Bs U a = 0 := by
  rw [coverUNSATList_iff_forall_sliceOnUNSAT Bs U]
  constructor
  · intro h a
    exact (sliceOnUNSATList_iff_solutionMass_eq_zero Bs U a).mp (h a)
  · intro h a
    exact (sliceOnUNSATList_iff_solutionMass_eq_zero Bs U a).mpr (h a)

/-- A SAT certificate localized to one restricted coordinate slice. -/
structure SliceOnSATCertificateList {n : Nat} (Bs : List (Blocker n)) where
  U : Finset (Fin n)
  assignment : SliceAssignment U
  witness : Vertex n
  inSlice : (Pattern.sliceOn U assignment).Covers witness
  uncovered : IsUncovered Bs witness

/-- An UNSAT certificate obtained by closing every restricted slice over `U`. -/
structure SliceOnUNSATCertificateList {n : Nat} (Bs : List (Blocker n)) where
  U : Finset (Fin n)
  allZero : ∀ a : SliceAssignment U, sliceOnSolutionMass Bs U a = 0

/-- A bounded-slice UNSAT certificate: close every slab after branching on at most `k`
coordinates. Its information/search cost is bounded by `2^k`. -/
structure BoundedSliceOnUNSATCertificateList {n : Nat} (Bs : List (Blocker n)) (k : Nat) where
  U : Finset (Fin n)
  card_le : U.card ≤ k
  allZero : ∀ a : SliceAssignment U, sliceOnSolutionMass Bs U a = 0

namespace SliceOnSATCertificateList

theorem coverSAT {n : Nat} {Bs : List (Blocker n)}
    (cert : SliceOnSATCertificateList Bs) :
    CoverSATList Bs :=
  ⟨cert.witness, cert.uncovered⟩

theorem sliceSAT {n : Nat} {Bs : List (Blocker n)}
    (cert : SliceOnSATCertificateList Bs) :
    SliceOnSATList Bs cert.U cert.assignment :=
  ⟨cert.witness, cert.inSlice, cert.uncovered⟩

end SliceOnSATCertificateList

namespace SliceOnUNSATCertificateList

/-- The exact number of restricted slices inspected by this certificate. -/
def branchCount {n : Nat} {Bs : List (Blocker n)}
    (cert : SliceOnUNSATCertificateList Bs) : Nat :=
  sliceBranchCount cert.U

theorem coverUNSAT {n : Nat} {Bs : List (Blocker n)}
    (cert : SliceOnUNSATCertificateList Bs) :
    CoverUNSATList Bs :=
  (coverUNSATList_iff_forall_sliceOnSolutionMass_zero Bs cert.U).mpr cert.allZero

theorem allSlicesUNSAT {n : Nat} {Bs : List (Blocker n)}
    (cert : SliceOnUNSATCertificateList Bs) :
    ∀ a : SliceAssignment cert.U, SliceOnUNSATList Bs cert.U a := by
  intro a
  exact (sliceOnUNSATList_iff_solutionMass_eq_zero Bs cert.U a).mpr (cert.allZero a)

end SliceOnUNSATCertificateList

namespace BoundedSliceOnUNSATCertificateList

def toCertificate {n k : Nat} {Bs : List (Blocker n)}
    (cert : BoundedSliceOnUNSATCertificateList Bs k) :
    SliceOnUNSATCertificateList Bs where
  U := cert.U
  allZero := cert.allZero

/-- The exact branch count paid by the bounded certificate. -/
def branchCount {n k : Nat} {Bs : List (Blocker n)}
    (cert : BoundedSliceOnUNSATCertificateList Bs k) : Nat :=
  sliceBranchCount cert.U

theorem branchCount_le {n k : Nat} {Bs : List (Blocker n)}
    (cert : BoundedSliceOnUNSATCertificateList Bs k) :
    cert.branchCount ≤ 2 ^ k :=
  sliceBranchCount_le_of_card_le cert.card_le

theorem coverUNSAT {n k : Nat} {Bs : List (Blocker n)}
    (cert : BoundedSliceOnUNSATCertificateList Bs k) :
    CoverUNSATList Bs :=
  cert.toCertificate.coverUNSAT

end BoundedSliceOnUNSATCertificateList

/-- `I` has a SAT witness visible after branching on at most `k` coordinates. This is always
sound, but naming it lets us compare SAT-side backdoor certificates by branch budget. -/
def CoverInput.HasBoundedSliceSATCertificate (I : CoverInput) (k : Nat) : Prop :=
  ∃ cert : SliceOnSATCertificateList I.blockers, cert.U.card ≤ k

/-- `I` has an UNSAT certificate obtained by closing all restricted slices after branching on
at most `k` coordinates. -/
def CoverInput.HasBoundedSliceUNSATCertificate (I : CoverInput) (k : Nat) : Prop :=
  Nonempty (BoundedSliceOnUNSATCertificateList I.blockers k)

/-- Promise class: the instance is decided by a bounded slice certificate of radius `k`. -/
def CoverInput.HasBoundedSliceDecisionCertificate (I : CoverInput) (k : Nat) : Prop :=
  I.HasBoundedSliceSATCertificate k ∨ I.HasBoundedSliceUNSATCertificate k

theorem CoverInput.sat_of_boundedSliceSATCertificate {I : CoverInput} {k : Nat}
    (h : I.HasBoundedSliceSATCertificate k) :
    CoverSAT I := by
  obtain ⟨cert, _hcard⟩ := h
  exact cert.coverSAT

theorem CoverInput.unsat_of_boundedSliceUNSATCertificate {I : CoverInput} {k : Nat}
    (h : I.HasBoundedSliceUNSATCertificate k) :
    CoverUNSAT I := by
  obtain ⟨cert⟩ := h
  exact cert.coverUNSAT

/-- Any bounded UNSAT slice certificate pays at most `2^k` branches. -/
theorem CoverInput.boundedSliceUNSAT_branchCount_le {I : CoverInput} {k : Nat}
    (cert : BoundedSliceOnUNSATCertificateList I.blockers k) :
    cert.branchCount ≤ 2 ^ k :=
  cert.branchCount_le

theorem CoverInput.boundedSliceSATCertificate_mono {I : CoverInput} {k l : Nat}
    (hkl : k ≤ l) (h : I.HasBoundedSliceSATCertificate k) :
    I.HasBoundedSliceSATCertificate l := by
  obtain ⟨cert, hcard⟩ := h
  exact ⟨cert, hcard.trans hkl⟩

theorem CoverInput.boundedSliceUNSATCertificate_mono {I : CoverInput} {k l : Nat}
    (hkl : k ≤ l) (h : I.HasBoundedSliceUNSATCertificate k) :
    I.HasBoundedSliceUNSATCertificate l := by
  obtain ⟨cert⟩ := h
  exact ⟨{ U := cert.U, card_le := cert.card_le.trans hkl, allZero := cert.allZero }⟩

theorem CoverInput.boundedSliceDecisionCertificate_mono {I : CoverInput} {k l : Nat}
    (hkl : k ≤ l) (h : I.HasBoundedSliceDecisionCertificate k) :
    I.HasBoundedSliceDecisionCertificate l := by
  rcases h with hsat | hunsat
  · exact Or.inl (I.boundedSliceSATCertificate_mono hkl hsat)
  · exact Or.inr (I.boundedSliceUNSATCertificate_mono hkl hunsat)

/-- The bounded-slice decision envelope is semantically complete at `k = n`, by branching on all
coordinates. This is a certificate-level version of brute force, useful as the baseline that
smaller backdoors improve on. -/
theorem CoverInput.hasBoundedSliceDecisionCertificate_full (I : CoverInput) :
    I.HasBoundedSliceDecisionCertificate I.n := by
  by_cases hsat : CoverSAT I
  · left
    obtain ⟨v, hun⟩ := hsat
    let U : Finset (Fin I.n) := Finset.univ
    let a : SliceAssignment U := fun i => v i.1
    refine ⟨{ U := U, assignment := a, witness := v, inSlice := ?_, uncovered := hun }, ?_⟩
    · exact (Pattern.sliceOn_covers_iff U a v).mpr (by intro i hi; rfl)
    · simp [U]
  · right
    refine ⟨{ U := Finset.univ, card_le := ?_, allZero := ?_ }⟩
    · simp
    · have hunsat : CoverUNSATList I.blockers := by
        unfold CoverUNSAT CoverSAT at hsat
        exact hsat
      exact (coverUNSATList_iff_forall_sliceOnSolutionMass_zero I.blockers Finset.univ).mp hunsat

/-- Least certified slice/backdoor width deciding the instance. It is an information invariant:
how many coordinates suffice before the cube decomposes into closed/live slabs. -/
noncomputable def CoverInput.sliceDecisionNumber (I : CoverInput) : Nat :=
  by
    classical
    exact Nat.find ⟨I.n, I.hasBoundedSliceDecisionCertificate_full⟩

theorem CoverInput.hasBoundedSliceDecisionCertificate_sliceDecisionNumber (I : CoverInput) :
    I.HasBoundedSliceDecisionCertificate I.sliceDecisionNumber := by
  classical
  exact Nat.find_spec ⟨I.n, I.hasBoundedSliceDecisionCertificate_full⟩

theorem CoverInput.sliceDecisionNumber_le_of_hasBoundedSliceDecisionCertificate
    {I : CoverInput} {k : Nat} (h : I.HasBoundedSliceDecisionCertificate k) :
    I.sliceDecisionNumber ≤ k := by
  classical
  exact Nat.find_min' ⟨I.n, I.hasBoundedSliceDecisionCertificate_full⟩ h

theorem CoverInput.sliceDecisionNumber_le_dimension (I : CoverInput) :
    I.sliceDecisionNumber ≤ I.n :=
  I.sliceDecisionNumber_le_of_hasBoundedSliceDecisionCertificate
    I.hasBoundedSliceDecisionCertificate_full

theorem CoverInput.hasBoundedSliceDecisionCertificate_of_sliceDecisionNumber_le
    {I : CoverInput} {k : Nat} (h : I.sliceDecisionNumber ≤ k) :
    I.HasBoundedSliceDecisionCertificate k :=
  I.boundedSliceDecisionCertificate_mono h
    I.hasBoundedSliceDecisionCertificate_sliceDecisionNumber

theorem CoverInput.hasBoundedSliceDecisionCertificate_iff_sliceDecisionNumber_le
    (I : CoverInput) (k : Nat) :
    I.HasBoundedSliceDecisionCertificate k ↔ I.sliceDecisionNumber ≤ k := by
  constructor
  · exact I.sliceDecisionNumber_le_of_hasBoundedSliceDecisionCertificate
  · exact I.hasBoundedSliceDecisionCertificate_of_sliceDecisionNumber_le

/-- Hard-side threshold: failure of all certificates of width `k` is exactly saying `k` lies
below the least slice-decision number. -/
theorem CoverInput.not_hasBoundedSliceDecisionCertificate_iff_lt_sliceDecisionNumber
    (I : CoverInput) (k : Nat) :
    ¬ I.HasBoundedSliceDecisionCertificate k ↔ k < I.sliceDecisionNumber := by
  rw [I.hasBoundedSliceDecisionCertificate_iff_sliceDecisionNumber_le k]
  omega

/-- Hard-side bounded-slice class: no decision certificate exists using at most `k` branch
coordinates. Equivalently, the slice-decision number is larger than `k`. -/
def CoverInput.SliceHardAbove (I : CoverInput) (k : Nat) : Prop :=
  k < I.sliceDecisionNumber

theorem CoverInput.sliceHardAbove_iff_no_boundedSliceDecisionCertificate
    (I : CoverInput) (k : Nat) :
    I.SliceHardAbove k ↔ ¬ I.HasBoundedSliceDecisionCertificate k := by
  unfold CoverInput.SliceHardAbove
  rw [I.not_hasBoundedSliceDecisionCertificate_iff_lt_sliceDecisionNumber]

theorem CoverInput.not_sliceHardAbove_dimension (I : CoverInput) :
    ¬ I.SliceHardAbove I.n := by
  unfold CoverInput.SliceHardAbove
  have hle := I.sliceDecisionNumber_le_dimension
  omega

/-- The brute-force branch bound stated through the least slice-decision invariant. -/
theorem CoverInput.sliceDecisionBranchCount_le_vertexCount (I : CoverInput) :
    2 ^ I.sliceDecisionNumber ≤ 2 ^ I.n :=
  Nat.pow_le_pow_right (by decide : 0 < 2) I.sliceDecisionNumber_le_dimension

/-- View a finite blocker family as a compact cover input. This bridges the finite classifier
world (`Finset`) with the operational/certificate world (`CoverInput`). -/
noncomputable def BlockerFamily.toCoverInput {n : Nat} (Bs : Finset (Blocker n)) :
    CoverInput where
  n := n
  blockers := Bs.toList

/-- Slice-decision number of a finite blocker family. -/
noncomputable def BlockerFamily.sliceDecisionNumber {n : Nat}
    (Bs : Finset (Blocker n)) : Nat :=
  (BlockerFamily.toCoverInput Bs).sliceDecisionNumber

/-- Finset-level easy bucket: bounded slice decision certificate of width at most `k`. -/
def BlockerFamily.HasBoundedSliceDecisionCertificate {n : Nat}
    (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  (BlockerFamily.toCoverInput Bs).HasBoundedSliceDecisionCertificate k

/-- Finset-level hard bucket: no bounded slice decision certificate of width at most `k`. -/
def BlockerFamily.SliceHardAbove {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) : Prop :=
  (BlockerFamily.toCoverInput Bs).SliceHardAbove k

theorem BlockerFamily.sliceHardAbove_iff_lt_sliceDecisionNumber {n : Nat}
    (Bs : Finset (Blocker n)) (k : Nat) :
    BlockerFamily.SliceHardAbove Bs k ↔ k < BlockerFamily.sliceDecisionNumber Bs := by
  rfl

theorem BlockerFamily.sliceDecisionNumber_le_dimension {n : Nat}
    (Bs : Finset (Blocker n)) :
    BlockerFamily.sliceDecisionNumber Bs ≤ n :=
  (BlockerFamily.toCoverInput Bs).sliceDecisionNumber_le_dimension

theorem BlockerFamily.hasBoundedSliceDecisionCertificate_iff_sliceDecisionNumber_le
    {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) :
    BlockerFamily.HasBoundedSliceDecisionCertificate Bs k ↔
      BlockerFamily.sliceDecisionNumber Bs ≤ k :=
  (BlockerFamily.toCoverInput Bs).hasBoundedSliceDecisionCertificate_iff_sliceDecisionNumber_le k

theorem BlockerFamily.sliceHardAbove_iff_no_boundedSliceDecisionCertificate
    {n : Nat} (Bs : Finset (Blocker n)) (k : Nat) :
    BlockerFamily.SliceHardAbove Bs k ↔
      ¬ BlockerFamily.HasBoundedSliceDecisionCertificate Bs k :=
  (BlockerFamily.toCoverInput Bs).sliceHardAbove_iff_no_boundedSliceDecisionCertificate k

theorem BlockerFamily.not_sliceHardAbove_dimension {n : Nat}
    (Bs : Finset (Blocker n)) :
    ¬ BlockerFamily.SliceHardAbove Bs n :=
  (BlockerFamily.toCoverInput Bs).not_sliceHardAbove_dimension

/-- Count bounded blocker families decidable by slice certificates of width at most `k`. -/
noncomputable def sliceDecidableFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => BlockerFamily.HasBoundedSliceDecisionCertificate Bs k

/-- Count bounded blocker families not decidable by any slice certificate of width at most `k`. -/
noncomputable def sliceHardFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => BlockerFamily.SliceHardAbove Bs k

/-- Count bounded blocker families whose least slice-decision width is exactly `k`. These are
the exact difficulty rings for the bounded-slice/backdoor normal form. -/
noncomputable def exactSliceDecisionFamilyCount (n m k : Nat) : Nat :=
  classCount n m fun Bs => BlockerFamily.sliceDecisionNumber Bs = k

theorem sliceDecidableFamilyCount_le_total (n m k : Nat) :
    sliceDecidableFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold sliceDecidableFamilyCount
  exact classCount_le_total n m _

theorem sliceHardFamilyCount_le_total (n m k : Nat) :
    sliceHardFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold sliceHardFamilyCount
  exact classCount_le_total n m _

theorem exactSliceDecisionFamilyCount_le_total (n m k : Nat) :
    exactSliceDecisionFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold exactSliceDecisionFamilyCount
  exact classCount_le_total n m _

theorem sliceDecidableFamilyCount_mono {n m k l : Nat} (hkl : k ≤ l) :
    sliceDecidableFamilyCount n m k ≤ sliceDecidableFamilyCount n m l := by
  unfold sliceDecidableFamilyCount
  apply classCount_mono
  intro Bs h
  rw [BlockerFamily.hasBoundedSliceDecisionCertificate_iff_sliceDecisionNumber_le] at h ⊢
  exact h.trans hkl

theorem sliceHardFamilyCount_antitone {n m k l : Nat} (hkl : k ≤ l) :
    sliceHardFamilyCount n m l ≤ sliceHardFamilyCount n m k := by
  unfold sliceHardFamilyCount
  apply classCount_mono
  intro Bs h
  unfold BlockerFamily.SliceHardAbove at h ⊢
  unfold CoverInput.SliceHardAbove at h ⊢
  omega

theorem sliceHardFamilyCount_at_dimension_eq_zero (n m : Nat) :
    sliceHardFamilyCount n m n = 0 := by
  classical
  unfold sliceHardFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  simp [BlockerFamily.not_sliceHardAbove_dimension Bs]

/-- There are no exact slice-decision rings above the ambient dimension. -/
theorem exactSliceDecisionFamilyCount_eq_zero_of_dimension_lt {n m k : Nat}
    (hnk : n < k) :
    exactSliceDecisionFamilyCount n m k = 0 := by
  classical
  unfold exactSliceDecisionFamilyCount classCount
  rw [Finset.card_eq_zero]
  ext Bs
  constructor
  · intro hmem
    simp only [Finset.mem_filter] at hmem
    have hle := BlockerFamily.sliceDecisionNumber_le_dimension Bs
    omega
  · intro hmem
    simp at hmem

/-- At full coordinate width, the slice-decision bucket is the entire bounded family space.
This is the formal baseline: branching on all dimensions always decides the cover question. -/
theorem sliceDecidableFamilyCount_at_dimension_eq_total (n m : Nat) :
    sliceDecidableFamilyCount n m n = totalFamilyCountAtMost n m := by
  classical
  unfold sliceDecidableFamilyCount classCount totalFamilyCountAtMost
  have hset :
      (allBlockerFamiliesAtMost n m).filter
          (fun Bs => BlockerFamily.HasBoundedSliceDecisionCertificate Bs n) =
        allBlockerFamiliesAtMost n m := by
    ext Bs
    constructor
    · intro h
      rw [Finset.mem_filter] at h
      exact h.1
    · intro hmem
      rw [Finset.mem_filter]
      exact ⟨hmem, (BlockerFamily.toCoverInput Bs).hasBoundedSliceDecisionCertificate_full⟩
  rw [hset]

/-- Slice-decidable and slice-hard are complementary buckets at each fixed width. This turns the
backdoor-width rule into an accounting identity over the bounded family universe. -/
theorem sliceDecidableFamilyCount_add_sliceHardFamilyCount_eq_total (n m k : Nat) :
    sliceDecidableFamilyCount n m k + sliceHardFamilyCount n m k =
      totalFamilyCountAtMost n m := by
  classical
  unfold sliceDecidableFamilyCount sliceHardFamilyCount classCount totalFamilyCountAtMost
  have hpart := Finset.card_filter_add_card_filter_not
    (s := allBlockerFamiliesAtMost n m)
    (p := fun Bs : Finset (Blocker n) =>
      BlockerFamily.HasBoundedSliceDecisionCertificate Bs k)
  have hhard :
      (Finset.filter
          (fun Bs : Finset (Blocker n) =>
            ¬ BlockerFamily.HasBoundedSliceDecisionCertificate Bs k)
          (allBlockerFamiliesAtMost n m)) =
        (Finset.filter
          (fun Bs : Finset (Blocker n) => BlockerFamily.SliceHardAbove Bs k)
          (allBlockerFamiliesAtMost n m)) := by
    ext Bs
    constructor
    · intro h
      rw [Finset.mem_filter] at h ⊢
      exact ⟨h.1, (BlockerFamily.sliceHardAbove_iff_no_boundedSliceDecisionCertificate
        Bs k).mpr h.2⟩
    · intro h
      rw [Finset.mem_filter] at h ⊢
      exact ⟨h.1, (BlockerFamily.sliceHardAbove_iff_no_boundedSliceDecisionCertificate
        Bs k).mp h.2⟩
  rw [hhard] at hpart
  simpa using hpart

/-- The branch cost attached to the least slice-decision certificate of a blocker family. This is
the information/search price of the best normal-form coordinate slicing certificate. -/
noncomputable def BlockerFamily.sliceDecisionBranchCount {n : Nat}
    (Bs : Finset (Blocker n)) : Nat :=
  2 ^ BlockerFamily.sliceDecisionNumber Bs

theorem BlockerFamily.sliceDecisionBranchCount_le_vertexCount {n : Nat}
    (Bs : Finset (Blocker n)) :
    BlockerFamily.sliceDecisionBranchCount Bs ≤ 2 ^ n := by
  unfold BlockerFamily.sliceDecisionBranchCount
  exact Nat.pow_le_pow_right (by decide : 0 < 2)
    (BlockerFamily.sliceDecisionNumber_le_dimension Bs)

theorem BlockerFamily.sliceDecisionBranchCount_eq_of_exact {n k : Nat}
    {Bs : Finset (Blocker n)}
    (h : BlockerFamily.sliceDecisionNumber Bs = k) :
    BlockerFamily.sliceDecisionBranchCount Bs = 2 ^ k := by
  simp [BlockerFamily.sliceDecisionBranchCount, h]

/-- Exact ring `k` is included in the cumulative slice-decidable bucket of width `k`. -/
theorem exactSliceDecisionFamilyCount_le_sliceDecidableFamilyCount (n m k : Nat) :
    exactSliceDecisionFamilyCount n m k ≤ sliceDecidableFamilyCount n m k := by
  unfold exactSliceDecisionFamilyCount sliceDecidableFamilyCount
  apply classCount_mono
  intro Bs h
  rw [BlockerFamily.hasBoundedSliceDecisionCertificate_iff_sliceDecisionNumber_le]
  omega

/-- Exact ring `k+1` is still hard above width `k`. This is the formal "next ring has not
collapsed yet" rule. -/
theorem exactSliceDecisionFamilyCount_succ_le_sliceHardFamilyCount (n m k : Nat) :
    exactSliceDecisionFamilyCount n m (k + 1) ≤ sliceHardFamilyCount n m k := by
  unfold exactSliceDecisionFamilyCount sliceHardFamilyCount
  apply classCount_mono
  intro Bs h
  unfold BlockerFamily.SliceHardAbove CoverInput.SliceHardAbove
  unfold BlockerFamily.sliceDecisionNumber at h
  omega

/-- Newly classified mass when climbing from width `k` to width `k+1`: exactly the next
slice-decision ring. This is the lightweight information-gain counter for the ring ladder. -/
noncomputable def sliceRingGainCount (n m k : Nat) : Nat :=
  exactSliceDecisionFamilyCount n m (k + 1)

theorem sliceRingGainCount_le_sliceHardFamilyCount (n m k : Nat) :
    sliceRingGainCount n m k ≤ sliceHardFamilyCount n m k := by
  unfold sliceRingGainCount
  exact exactSliceDecisionFamilyCount_succ_le_sliceHardFamilyCount n m k

theorem sliceRingGainCount_le_sliceDecidableFamilyCount_succ (n m k : Nat) :
    sliceRingGainCount n m k ≤ sliceDecidableFamilyCount n m (k + 1) := by
  unfold sliceRingGainCount
  exact exactSliceDecisionFamilyCount_le_sliceDecidableFamilyCount n m (k + 1)

/-- Uncertainty of the cumulative easy slice bucket. -/
noncomputable def sliceDecidableFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (sliceDecidableFamilyCount n m k)

/-- Uncertainty of the still-hard-above-`k` slice bucket. -/
noncomputable def sliceHardFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (sliceHardFamilyCount n m k)

/-- Uncertainty of one exact slice-decision ring. -/
noncomputable def exactSliceDecisionFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (exactSliceDecisionFamilyCount n m k)

/-- Uncertainty of the newly classified ring when climbing from width `k` to `k+1`. -/
noncomputable def sliceRingGainUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (sliceRingGainCount n m k)

/-- Total uncertainty of the bounded blocker-family universe. -/
noncomputable def totalFamilyUncertaintyAtMost (n m : Nat) : Nat :=
  uncertaintyBits (totalFamilyCountAtMost n m)

/-- Uncertainty of the bounded-variable-footprint bucket. -/
noncomputable def supportAtMostFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (supportAtMostFamilyCount n m k)

/-- Uncertainty of the large-variable-footprint bucket. -/
noncomputable def supportAtLeastFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (supportAtLeastFamilyCount n m k)

/-- Uncertainty of one exact support-footprint ring. -/
noncomputable def supportExactFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (supportExactFamilyCount n m k)

/-- Uncertainty of the unique-hole bucket. -/
noncomputable def uniqueHoleFamilyUncertainty (n m : Nat) : Nat :=
  uncertaintyBits (uniqueHoleFamilyCount n m)

/-- Uncertainty of the bucket with at least `k` holes. -/
noncomputable def manyHoleFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (manyHoleFamilyCount n m k)

/-- Uncertainty of one exact blocker-budget ring. -/
noncomputable def exactBlockerFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (exactBlockerFamilyCount n m k)

/-- Uncertainty of one exact hole-count ring. -/
noncomputable def exactHoleFamilyUncertainty (n m k : Nat) : Nat :=
  uncertaintyBits (exactHoleFamilyCount n m k)

/-- Uncertainty of the full-cover bucket. -/
noncomputable def fullCoverFamilyUncertainty (n m : Nat) : Nat :=
  uncertaintyBits (fullCoverFamilyCount n m)

/-- Uncertainty of the locally invisible full-cover bucket. -/
noncomputable def locallyInvisibleFamilyUncertainty (n m r : Nat) : Nat :=
  uncertaintyBits (locallyInvisibleFamilyCount n m r)

/-- Uncertainty of the geometric hard-UNSAT candidate bucket. -/
noncomputable def geometricHardUNSATCandidateUncertainty (n m r : Nat) : Nat :=
  uncertaintyBits (geometricHardUNSATCandidateCount n m r)

theorem sliceDecidableFamilyUncertainty_le_total (n m k : Nat) :
    sliceDecidableFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold sliceDecidableFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (sliceDecidableFamilyCount_le_total n m k)

theorem supportAtMostFamilyCount_le_total (n m k : Nat) :
    supportAtMostFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold supportAtMostFamilyCount
  exact classCount_le_total n m _

theorem supportAtLeastFamilyCount_le_total (n m k : Nat) :
    supportAtLeastFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold supportAtLeastFamilyCount
  exact classCount_le_total n m _

theorem supportExactFamilyCount_le_total (n m k : Nat) :
    supportExactFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold supportExactFamilyCount
  exact classCount_le_total n m _

theorem manyHoleFamilyCount_le_total (n m k : Nat) :
    manyHoleFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold manyHoleFamilyCount
  exact classCount_le_total n m _

theorem exactBlockerFamilyCount_le_total (n m k : Nat) :
    exactBlockerFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold exactBlockerFamilyCount
  exact classCount_le_total n m _

theorem exactHoleFamilyCount_le_total (n m k : Nat) :
    exactHoleFamilyCount n m k ≤ totalFamilyCountAtMost n m := by
  unfold exactHoleFamilyCount
  exact classCount_le_total n m _

theorem fullCoverFamilyCount_le_total (n m : Nat) :
    fullCoverFamilyCount n m ≤ totalFamilyCountAtMost n m := by
  unfold fullCoverFamilyCount
  exact classCount_le_total n m _

theorem locallyInvisibleFamilyCount_le_total (n m r : Nat) :
    locallyInvisibleFamilyCount n m r ≤ totalFamilyCountAtMost n m := by
  unfold locallyInvisibleFamilyCount
  exact classCount_le_total n m _

theorem geometricHardUNSATCandidateCount_le_total (n m r : Nat) :
    geometricHardUNSATCandidateCount n m r ≤ totalFamilyCountAtMost n m := by
  unfold geometricHardUNSATCandidateCount
  exact classCount_le_total n m _

theorem supportAtMostFamilyUncertainty_le_total (n m k : Nat) :
    supportAtMostFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold supportAtMostFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (supportAtMostFamilyCount_le_total n m k)

theorem supportAtLeastFamilyUncertainty_le_total (n m k : Nat) :
    supportAtLeastFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold supportAtLeastFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (supportAtLeastFamilyCount_le_total n m k)

theorem supportExactFamilyUncertainty_le_total (n m k : Nat) :
    supportExactFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold supportExactFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (supportExactFamilyCount_le_total n m k)

theorem manyHoleFamilyUncertainty_le_total (n m k : Nat) :
    manyHoleFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold manyHoleFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (manyHoleFamilyCount_le_total n m k)

theorem exactBlockerFamilyUncertainty_le_total (n m k : Nat) :
    exactBlockerFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold exactBlockerFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (exactBlockerFamilyCount_le_total n m k)

theorem exactHoleFamilyUncertainty_le_total (n m k : Nat) :
    exactHoleFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold exactHoleFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (exactHoleFamilyCount_le_total n m k)

theorem fullCoverFamilyUncertainty_le_total (n m : Nat) :
    fullCoverFamilyUncertainty n m ≤ totalFamilyUncertaintyAtMost n m := by
  unfold fullCoverFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (fullCoverFamilyCount_le_total n m)

theorem locallyInvisibleFamilyUncertainty_le_total (n m r : Nat) :
    locallyInvisibleFamilyUncertainty n m r ≤ totalFamilyUncertaintyAtMost n m := by
  unfold locallyInvisibleFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (locallyInvisibleFamilyCount_le_total n m r)

theorem geometricHardUNSATCandidateUncertainty_le_total (n m r : Nat) :
    geometricHardUNSATCandidateUncertainty n m r ≤ totalFamilyUncertaintyAtMost n m := by
  unfold geometricHardUNSATCandidateUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (geometricHardUNSATCandidateCount_le_total n m r)

theorem sliceHardFamilyUncertainty_le_total (n m k : Nat) :
    sliceHardFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold sliceHardFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (sliceHardFamilyCount_le_total n m k)

theorem exactSliceDecisionFamilyUncertainty_le_total (n m k : Nat) :
    exactSliceDecisionFamilyUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold exactSliceDecisionFamilyUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (exactSliceDecisionFamilyCount_le_total n m k)

theorem sliceRingGainUncertainty_le_total (n m k : Nat) :
    sliceRingGainUncertainty n m k ≤ totalFamilyUncertaintyAtMost n m := by
  unfold sliceRingGainUncertainty totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono
    ((sliceRingGainCount_le_sliceHardFamilyCount n m k).trans
      (sliceHardFamilyCount_le_total n m k))

theorem sliceDecidableFamilyUncertainty_mono {n m k l : Nat} (hkl : k ≤ l) :
    sliceDecidableFamilyUncertainty n m k ≤ sliceDecidableFamilyUncertainty n m l := by
  unfold sliceDecidableFamilyUncertainty
  exact uncertaintyBits_mono (sliceDecidableFamilyCount_mono hkl)

theorem sliceHardFamilyUncertainty_antitone {n m k l : Nat} (hkl : k ≤ l) :
    sliceHardFamilyUncertainty n m l ≤ sliceHardFamilyUncertainty n m k := by
  unfold sliceHardFamilyUncertainty
  exact uncertaintyBits_mono (sliceHardFamilyCount_antitone hkl)

theorem manyHoleFamilyUncertainty_antitone {n m k l : Nat} (hkl : k ≤ l) :
    manyHoleFamilyUncertainty n m l ≤ manyHoleFamilyUncertainty n m k := by
  unfold manyHoleFamilyUncertainty
  exact uncertaintyBits_mono (manyHoleFamilyCount_antitone hkl)

theorem totalFamilyUncertaintyAtMost_mono {n m l : Nat} (hml : m ≤ l) :
    totalFamilyUncertaintyAtMost n m ≤ totalFamilyUncertaintyAtMost n l := by
  unfold totalFamilyUncertaintyAtMost
  exact uncertaintyBits_mono (totalFamilyCountAtMost_mono hml)

theorem manyHoleFamilyUncertainty_budget_mono {n m l k : Nat} (hml : m ≤ l) :
    manyHoleFamilyUncertainty n m k ≤ manyHoleFamilyUncertainty n l k := by
  unfold manyHoleFamilyUncertainty
  exact uncertaintyBits_mono (manyHoleFamilyCount_budget_mono hml)

theorem exactBlockerFamilyUncertainty_budget_mono {n m l k : Nat} (hml : m ≤ l) :
    exactBlockerFamilyUncertainty n m k ≤ exactBlockerFamilyUncertainty n l k := by
  unfold exactBlockerFamilyUncertainty
  exact uncertaintyBits_mono (exactBlockerFamilyCount_budget_mono hml)

theorem exactHoleFamilyUncertainty_budget_mono {n m l k : Nat} (hml : m ≤ l) :
    exactHoleFamilyUncertainty n m k ≤ exactHoleFamilyUncertainty n l k := by
  unfold exactHoleFamilyUncertainty
  exact uncertaintyBits_mono (exactHoleFamilyCount_budget_mono hml)

theorem uniqueHoleFamilyUncertainty_budget_mono {n m l : Nat} (hml : m ≤ l) :
    uniqueHoleFamilyUncertainty n m ≤ uniqueHoleFamilyUncertainty n l := by
  unfold uniqueHoleFamilyUncertainty
  exact uncertaintyBits_mono (uniqueHoleFamilyCount_budget_mono hml)

theorem fullCoverFamilyUncertainty_budget_mono {n m l : Nat} (hml : m ≤ l) :
    fullCoverFamilyUncertainty n m ≤ fullCoverFamilyUncertainty n l := by
  unfold fullCoverFamilyUncertainty
  exact uncertaintyBits_mono (fullCoverFamilyCount_budget_mono hml)

theorem supportAtMostFamilyUncertainty_mono {n m k l : Nat} (hkl : k ≤ l) :
    supportAtMostFamilyUncertainty n m k ≤ supportAtMostFamilyUncertainty n m l := by
  unfold supportAtMostFamilyUncertainty
  exact uncertaintyBits_mono (supportAtMostFamilyCount_mono hkl)

theorem supportAtLeastFamilyUncertainty_antitone {n m k l : Nat} (hkl : k ≤ l) :
    supportAtLeastFamilyUncertainty n m l ≤ supportAtLeastFamilyUncertainty n m k := by
  unfold supportAtLeastFamilyUncertainty
  exact uncertaintyBits_mono (supportAtLeastFamilyCount_antitone hkl)

theorem supportExactFamilyUncertainty_eq_zero_of_dimension_lt {n m k : Nat}
    (hnk : n < k) :
    supportExactFamilyUncertainty n m k = 0 := by
  simp [supportExactFamilyUncertainty, supportExactFamilyCount_eq_zero_of_dimension_lt hnk]

theorem supportAtLeastFamilyUncertainty_eq_zero_of_dimension_lt {n m k : Nat}
    (hnk : n < k) :
    supportAtLeastFamilyUncertainty n m k = 0 := by
  simp [supportAtLeastFamilyUncertainty, supportAtLeastFamilyCount_eq_zero_of_dimension_lt hnk]

theorem supportAtMostFamilyUncertainty_at_dimension_eq_total (n m : Nat) :
    supportAtMostFamilyUncertainty n m n = totalFamilyUncertaintyAtMost n m := by
  simp [supportAtMostFamilyUncertainty, totalFamilyUncertaintyAtMost,
    supportAtMostFamilyCount_at_dimension_eq_total]

theorem supportAtLeastFamilyUncertainty_zero_eq_total (n m : Nat) :
    supportAtLeastFamilyUncertainty n m 0 = totalFamilyUncertaintyAtMost n m := by
  simp [supportAtLeastFamilyUncertainty, totalFamilyUncertaintyAtMost,
    supportAtLeastFamilyCount_zero_eq_total]

theorem supportAtLeastFamilyUncertainty_at_dimension_eq_supportExact (n m : Nat) :
    supportAtLeastFamilyUncertainty n m n = supportExactFamilyUncertainty n m n := by
  simp [supportAtLeastFamilyUncertainty, supportExactFamilyUncertainty,
    supportAtLeastFamilyCount_at_dimension_eq_supportExact]

theorem uniqueHoleFamilyUncertainty_le_supportExact_dimension (n m : Nat) :
    uniqueHoleFamilyUncertainty n m ≤ supportExactFamilyUncertainty n m n := by
  unfold uniqueHoleFamilyUncertainty supportExactFamilyUncertainty
  exact uncertaintyBits_mono (uniqueHole_count_le_supportExact_dimension_count n m)

theorem manyHoleFamilyUncertainty_one_eq_total_of_budget_lt_eight {n m : Nat}
    (hm : m < 8) :
    manyHoleFamilyUncertainty n m 1 = totalFamilyUncertaintyAtMost n m := by
  simp [manyHoleFamilyUncertainty, totalFamilyUncertaintyAtMost,
    manyHoleFamilyCount_one_eq_total_of_budget_lt_eight hm]

theorem uniqueHoleFamilyUncertainty_eq_zero_of_budget_lt_seven {n m : Nat}
    (hn : 3 ≤ n) (hm : m < 7) :
    uniqueHoleFamilyUncertainty n m = 0 := by
  simp [uniqueHoleFamilyUncertainty,
    uniqueHoleFamilyCount_eq_zero_of_budget_lt_seven hn hm]

theorem manyHoleFamilyUncertainty_zero_eq_total (n m : Nat) :
    manyHoleFamilyUncertainty n m 0 = totalFamilyUncertaintyAtMost n m := by
  simp [manyHoleFamilyUncertainty, totalFamilyUncertaintyAtMost,
    manyHoleFamilyCount_zero_eq_total]

theorem exactHoleFamilyUncertainty_eq_zero_of_vertexCount_lt {n m k : Nat}
    (hk : 2 ^ n < k) :
    exactHoleFamilyUncertainty n m k = 0 := by
  simp [exactHoleFamilyUncertainty, exactHoleFamilyCount_eq_zero_of_vertexCount_lt hk]

theorem manyHoleFamilyUncertainty_eq_zero_of_vertexCount_lt {n m k : Nat}
    (hk : 2 ^ n < k) :
    manyHoleFamilyUncertainty n m k = 0 := by
  simp [manyHoleFamilyUncertainty, manyHoleFamilyCount_eq_zero_of_vertexCount_lt hk]

theorem exactBlockerFamilyUncertainty_eq_zero_of_budget_lt {n m k : Nat}
    (hmk : m < k) :
    exactBlockerFamilyUncertainty n m k = 0 := by
  simp [exactBlockerFamilyUncertainty, exactBlockerFamilyCount_eq_zero_of_budget_lt hmk]

theorem exactBlockerFamilyUncertainty_eq_zero_of_blockerUniverse_lt {n m k : Nat}
    (hk : Fintype.card (Blocker n) < k) :
    exactBlockerFamilyUncertainty n m k = 0 := by
  simp [exactBlockerFamilyUncertainty, exactBlockerFamilyCount_eq_zero_of_blockerUniverse_lt hk]

theorem exactBlockerFamilyUncertainty_self_budget_eq_budget {n m k : Nat} (hkm : k ≤ m) :
    exactBlockerFamilyUncertainty n m k = exactBlockerFamilyUncertainty n k k := by
  simp [exactBlockerFamilyUncertainty, exactBlockerFamilyCount_self_budget_eq_budget hkm]

theorem uniqueHoleFamilyUncertainty_eq_exactHole_one (n m : Nat) :
    uniqueHoleFamilyUncertainty n m = exactHoleFamilyUncertainty n m 1 := by
  simp [uniqueHoleFamilyUncertainty, exactHoleFamilyUncertainty,
    uniqueHoleFamilyCount_eq_exactHoleFamilyCount_one]

theorem fullCoverFamilyUncertainty_eq_exactHole_zero (n m : Nat) :
    fullCoverFamilyUncertainty n m = exactHoleFamilyUncertainty n m 0 := by
  simp [fullCoverFamilyUncertainty, exactHoleFamilyUncertainty,
    fullCoverFamilyCount_eq_exactHoleFamilyCount_zero]

theorem fullCoverFamilyUncertainty_eq_zero_of_budget_lt_eight {n m : Nat}
    (hm : m < 8) :
    fullCoverFamilyUncertainty n m = 0 := by
  simp [fullCoverFamilyUncertainty, fullCoverFamilyCount_eq_zero_of_budget_lt_eight hm]

theorem locallyInvisibleFamilyUncertainty_eq_zero_of_budget_lt_eight {n m r : Nat}
    (hm : m < 8) :
    locallyInvisibleFamilyUncertainty n m r = 0 := by
  simp [locallyInvisibleFamilyUncertainty,
    locallyInvisibleFamilyCount_eq_zero_of_budget_lt_eight hm]

theorem geometricHardUNSATCandidateUncertainty_eq_zero_of_budget_lt_eight {n m r : Nat}
    (hm : m < 8) :
    geometricHardUNSATCandidateUncertainty n m r = 0 := by
  simp [geometricHardUNSATCandidateUncertainty,
    geometricHardUNSATCandidateCount_eq_zero_of_budget_lt_eight hm]

theorem sliceRingGainUncertainty_le_hardUncertainty (n m k : Nat) :
    sliceRingGainUncertainty n m k ≤ sliceHardFamilyUncertainty n m k := by
  unfold sliceRingGainUncertainty sliceHardFamilyUncertainty
  exact uncertaintyBits_mono (sliceRingGainCount_le_sliceHardFamilyCount n m k)

theorem sliceRingGainUncertainty_le_decidableSuccUncertainty (n m k : Nat) :
    sliceRingGainUncertainty n m k ≤ sliceDecidableFamilyUncertainty n m (k + 1) := by
  unfold sliceRingGainUncertainty sliceDecidableFamilyUncertainty
  exact uncertaintyBits_mono (sliceRingGainCount_le_sliceDecidableFamilyCount_succ n m k)

@[simp] theorem sliceHardFamilyUncertainty_at_dimension_eq_zero (n m : Nat) :
    sliceHardFamilyUncertainty n m n = 0 := by
  simp [sliceHardFamilyUncertainty, sliceHardFamilyCount_at_dimension_eq_zero]

theorem exactSliceDecisionFamilyUncertainty_eq_zero_of_dimension_lt {n m k : Nat}
    (hnk : n < k) :
    exactSliceDecisionFamilyUncertainty n m k = 0 := by
  simp [exactSliceDecisionFamilyUncertainty,
    exactSliceDecisionFamilyCount_eq_zero_of_dimension_lt hnk]

theorem sliceDecidableFamilyUncertainty_at_dimension_eq_total (n m : Nat) :
    sliceDecidableFamilyUncertainty n m n = totalFamilyUncertaintyAtMost n m := by
  simp [sliceDecidableFamilyUncertainty, totalFamilyUncertaintyAtMost,
    sliceDecidableFamilyCount_at_dimension_eq_total]

/-- Blocker-aware mass of one child. -/
def Pattern.childSolutionMass {n : Nat} (Bs : List (Blocker n)) (P : Pattern n)
    (i : Fin n) (b : Bool) (hfree : P.Free i) : Nat :=
  (P.assign i b hfree).solutionMass Bs

theorem Pattern.childSolutionMass_le_solutionMass {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (b : Bool) (hfree : P.Free i) :
    P.childSolutionMass Bs i b hfree ≤ P.solutionMass Bs := by
  unfold Pattern.childSolutionMass Pattern.solutionMass
  apply Finset.card_le_card
  intro v hv
  rw [Pattern.mem_solutionVertices] at hv ⊢
  exact ⟨P.covers_of_assign i b hfree v hv.1, hv.2⟩

/-- Worst remaining candidate-solution mass after splitting a face on coordinate `i`. -/
def Pattern.splitWorstSolutionMass {n : Nat} (Bs : List (Blocker n)) (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) : Nat :=
  max (P.childSolutionMass Bs i false hfree) (P.childSolutionMass Bs i true hfree)

theorem Pattern.splitWorstSolutionMass_le_solutionMass {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.splitWorstSolutionMass Bs i hfree ≤ P.solutionMass Bs := by
  unfold Pattern.splitWorstSolutionMass
  exact max_le (P.childSolutionMass_le_solutionMass Bs i false hfree)
    (P.childSolutionMass_le_solutionMass Bs i true hfree)

/-- A free split exactly partitions the still-uncovered candidate vertices. -/
theorem Pattern.solutionVertices_split_eq_union {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.solutionVertices Bs =
      Pattern.solutionVertices Bs (P.assign i false hfree) ∪
        Pattern.solutionVertices Bs (P.assign i true hfree) := by
  ext v
  constructor
  · intro hv
    rw [Pattern.mem_solutionVertices] at hv
    rw [Finset.mem_union]
    rcases P.split_by_free i hfree v hv.1 with hfalse | htrue
    · left
      rw [Pattern.mem_solutionVertices]
      exact ⟨hfalse, hv.2⟩
    · right
      rw [Pattern.mem_solutionVertices]
      exact ⟨htrue, hv.2⟩
  · intro hv
    rw [Finset.mem_union] at hv
    rw [Pattern.mem_solutionVertices]
    rcases hv with hfalse | htrue
    · rw [Pattern.mem_solutionVertices] at hfalse
      exact ⟨P.covers_of_assign i false hfree v hfalse.1, hfalse.2⟩
    · rw [Pattern.mem_solutionVertices] at htrue
      exact ⟨P.covers_of_assign i true hfree v htrue.1, htrue.2⟩

/-- The two still-uncovered child candidate sets are disjoint. -/
theorem Pattern.solutionVertices_split_disjoint {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    Disjoint (Pattern.solutionVertices Bs (P.assign i false hfree))
      (Pattern.solutionVertices Bs (P.assign i true hfree)) := by
  rw [Finset.disjoint_left]
  intro v hfalse htrue
  rw [Pattern.mem_solutionVertices] at hfalse htrue
  have hf : v i = false := (P.assign_covers_iff i false hfree v).mp hfalse.1 |>.2
  have ht : v i = true := (P.assign_covers_iff i true hfree v).mp htrue.1 |>.2
  rw [hf] at ht
  simp at ht

/-- Exact conservation of blocker-aware candidate mass under a binary branch. -/
theorem Pattern.solutionMass_eq_childSolutionMass_add_childSolutionMass {n : Nat}
    (Bs : List (Blocker n)) (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.solutionMass Bs =
      P.childSolutionMass Bs i false hfree + P.childSolutionMass Bs i true hfree := by
  unfold Pattern.solutionMass Pattern.childSolutionMass
  rw [P.solutionVertices_split_eq_union Bs i hfree]
  exact Finset.card_union_of_disjoint (P.solutionVertices_split_disjoint Bs i hfree)

/-- A blocker-aware binary split also obeys the one-bit accounting lower bound. -/
theorem Pattern.solutionMass_le_two_mul_splitWorstSolutionMass {n : Nat}
    (Bs : List (Blocker n)) (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.solutionMass Bs ≤ 2 * P.splitWorstSolutionMass Bs i hfree := by
  rw [P.solutionMass_eq_childSolutionMass_add_childSolutionMass Bs i hfree]
  unfold Pattern.splitWorstSolutionMass
  calc
    P.childSolutionMass Bs i false hfree + P.childSolutionMass Bs i true hfree
        ≤ max (P.childSolutionMass Bs i false hfree) (P.childSolutionMass Bs i true hfree) +
          max (P.childSolutionMass Bs i false hfree) (P.childSolutionMass Bs i true hfree) := by
          exact add_le_add (le_max_left _ _) (le_max_right _ _)
    _ = 2 * max (P.childSolutionMass Bs i false hfree)
          (P.childSolutionMass Bs i true hfree) := by omega

/-- Imbalance of a split: zero is perfectly balanced, larger means one answer is much worse. -/
def Pattern.splitImbalance {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) : Nat :=
  max (P.childMass i false hfree) (P.childMass i true hfree) -
    min (P.childMass i false hfree) (P.childMass i true hfree)

/-- A geometric split is perfectly balanced when both child faces have the same vertex mass. -/
def Pattern.BalancedSplit {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) : Prop :=
  P.childMass i false hfree = P.childMass i true hfree

theorem Pattern.splitImbalance_le_splitWorstMass {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.splitImbalance i hfree ≤ P.splitWorstMass i hfree := by
  unfold Pattern.splitImbalance Pattern.splitWorstMass
  exact Nat.sub_le _ _

theorem Pattern.splitImbalance_eq_zero_iff_balanced {n : Nat} (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) :
    P.splitImbalance i hfree = 0 ↔ P.BalancedSplit i hfree := by
  unfold Pattern.splitImbalance Pattern.BalancedSplit
  constructor
  · intro hzero
    have hmaxlemin :
        max (P.childMass i false hfree) (P.childMass i true hfree) ≤
          min (P.childMass i false hfree) (P.childMass i true hfree) := by
      omega
    have hfalseletrue :
        P.childMass i false hfree ≤ P.childMass i true hfree := by
      exact (le_max_left _ _).trans hmaxlemin |>.trans (min_le_right _ _)
    have htruelefalse :
        P.childMass i true hfree ≤ P.childMass i false hfree := by
      exact (le_max_right _ _).trans hmaxlemin |>.trans (min_le_left _ _)
    exact Nat.le_antisymm hfalseletrue htruelefalse
  · intro hbal
    rw [hbal]
    simp

theorem Pattern.balancedSplit_mass_eq_two_mul_child {n : Nat} (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) (hbal : P.BalancedSplit i hfree) :
    P.mass = 2 * P.childMass i false hfree := by
  rw [P.mass_eq_childMass_add_childMass i hfree, hbal]
  omega

theorem Pattern.balancedSplit_splitWorstMass {n : Nat} (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) (hbal : P.BalancedSplit i hfree) :
    P.splitWorstMass i hfree = P.childMass i false hfree := by
  unfold Pattern.splitWorstMass
  rw [hbal]
  simp

/-- Blocker-aware imbalance: the split quality seen only through still-uncovered candidates. -/
def Pattern.splitSolutionImbalance {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Nat :=
  max (P.childSolutionMass Bs i false hfree) (P.childSolutionMass Bs i true hfree) -
    min (P.childSolutionMass Bs i false hfree) (P.childSolutionMass Bs i true hfree)

/-- A split is candidate-balanced when both children contain the same number of still-uncovered
vertices. This is the SAT-aware version of a Shannon-balanced question. -/
def Pattern.SolutionBalancedSplit {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Prop :=
  P.childSolutionMass Bs i false hfree = P.childSolutionMass Bs i true hfree

theorem Pattern.splitSolutionImbalance_le_splitWorstSolutionMass {n : Nat}
    (Bs : List (Blocker n)) (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.splitSolutionImbalance Bs i hfree ≤ P.splitWorstSolutionMass Bs i hfree := by
  unfold Pattern.splitSolutionImbalance Pattern.splitWorstSolutionMass
  exact Nat.sub_le _ _

theorem Pattern.splitSolutionImbalance_eq_zero_iff_balanced {n : Nat}
    (Bs : List (Blocker n)) (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.splitSolutionImbalance Bs i hfree = 0 ↔ P.SolutionBalancedSplit Bs i hfree := by
  unfold Pattern.splitSolutionImbalance Pattern.SolutionBalancedSplit
  constructor
  · intro hzero
    have hmaxlemin :
        max (P.childSolutionMass Bs i false hfree) (P.childSolutionMass Bs i true hfree) ≤
          min (P.childSolutionMass Bs i false hfree) (P.childSolutionMass Bs i true hfree) := by
      omega
    have hfalseletrue :
        P.childSolutionMass Bs i false hfree ≤ P.childSolutionMass Bs i true hfree := by
      exact (le_max_left _ _).trans hmaxlemin |>.trans (min_le_right _ _)
    have htruelefalse :
        P.childSolutionMass Bs i true hfree ≤ P.childSolutionMass Bs i false hfree := by
      exact (le_max_right _ _).trans hmaxlemin |>.trans (min_le_left _ _)
    exact Nat.le_antisymm hfalseletrue htruelefalse
  · intro hbal
    rw [hbal]
    simp

theorem Pattern.solutionBalancedSplit_mass_eq_two_mul_child {n : Nat}
    (Bs : List (Blocker n)) (P : Pattern n) (i : Fin n) (hfree : P.Free i)
    (hbal : P.SolutionBalancedSplit Bs i hfree) :
    P.solutionMass Bs = 2 * P.childSolutionMass Bs i false hfree := by
  rw [P.solutionMass_eq_childSolutionMass_add_childSolutionMass Bs i hfree, hbal]
  omega

theorem Pattern.solutionBalancedSplit_splitWorstSolutionMass {n : Nat}
    (Bs : List (Blocker n)) (P : Pattern n) (i : Fin n) (hfree : P.Free i)
    (hbal : P.SolutionBalancedSplit Bs i hfree) :
    P.splitWorstSolutionMass Bs i hfree = P.childSolutionMass Bs i false hfree := by
  unfold Pattern.splitWorstSolutionMass
  rw [hbal]
  simp

/-- Pareto dominance for geometric split quality. A split is better when it has no worse
worst-child mass and no worse imbalance. This avoids hiding tradeoffs inside one magic scalar. -/
def Pattern.SplitParetoDominates {n : Nat} (P : Pattern n)
    (i : Fin n) (hfreei : P.Free i) (j : Fin n) (hfreej : P.Free j) : Prop :=
  P.splitWorstMass i hfreei ≤ P.splitWorstMass j hfreej ∧
    P.splitImbalance i hfreei ≤ P.splitImbalance j hfreej

theorem Pattern.SplitParetoDominates.refl {n : Nat} (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) :
    P.SplitParetoDominates i hfree i hfree :=
  ⟨le_rfl, le_rfl⟩

theorem Pattern.SplitParetoDominates.trans {n : Nat} {P : Pattern n}
    {i j k : Fin n} {hfreei : P.Free i} {hfreej : P.Free j} {hfreek : P.Free k}
    (hij : P.SplitParetoDominates i hfreei j hfreej)
    (hjk : P.SplitParetoDominates j hfreej k hfreek) :
    P.SplitParetoDominates i hfreei k hfreek :=
  ⟨hij.1.trans hjk.1, hij.2.trans hjk.2⟩

/-- Pareto dominance for blocker-aware candidate-solution split quality. -/
def Pattern.SolutionSplitParetoDominates {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfreei : P.Free i)
    (j : Fin n) (hfreej : P.Free j) : Prop :=
  P.splitWorstSolutionMass Bs i hfreei ≤ P.splitWorstSolutionMass Bs j hfreej ∧
    P.splitSolutionImbalance Bs i hfreei ≤ P.splitSolutionImbalance Bs j hfreej

theorem Pattern.SolutionSplitParetoDominates.refl {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.SolutionSplitParetoDominates Bs i hfree i hfree :=
  ⟨le_rfl, le_rfl⟩

theorem Pattern.SolutionSplitParetoDominates.trans {n : Nat} {Bs : List (Blocker n)}
    {P : Pattern n} {i j k : Fin n}
    {hfreei : P.Free i} {hfreej : P.Free j} {hfreek : P.Free k}
    (hij : P.SolutionSplitParetoDominates Bs i hfreei j hfreej)
    (hjk : P.SolutionSplitParetoDominates Bs j hfreej k hfreek) :
    P.SolutionSplitParetoDominates Bs i hfreei k hfreek :=
  ⟨hij.1.trans hjk.1, hij.2.trans hjk.2⟩

/-- Worst-case remaining uncertainty after a split. -/
def Pattern.splitWorstUncertainty {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) : Nat :=
  uncertaintyBits (P.splitWorstMass i hfree)

theorem Pattern.splitWorstUncertainty_le_uncertainty {n : Nat} (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) :
    P.splitWorstUncertainty i hfree ≤ uncertaintyBits P.mass :=
  uncertaintyBits_mono (P.splitWorstMass_le_mass i hfree)

/-- Worst-case remaining candidate-solution uncertainty after a split. -/
def Pattern.splitWorstSolutionUncertainty {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Nat :=
  uncertaintyBits (P.splitWorstSolutionMass Bs i hfree)

theorem Pattern.splitWorstSolutionUncertainty_le_solutionUncertainty {n : Nat}
    (Bs : List (Blocker n)) (P : Pattern n) (i : Fin n) (hfree : P.Free i) :
    P.splitWorstSolutionUncertainty Bs i hfree ≤ uncertaintyBits (P.solutionMass Bs) :=
  uncertaintyBits_mono (P.splitWorstSolutionMass_le_solutionMass Bs i hfree)

/-- A coordinate is an information-optimal split among a candidate set when it minimizes
the worst remaining child mass. This is "provably best" relative to this score and class
of candidate coordinates. -/
def Pattern.InfoOptimalSplit {n : Nat} (P : Pattern n) (Candidates : Finset (Fin n))
    (i : Fin n) (hfree : P.Free i) : Prop :=
  i ∈ Candidates ∧
    ∀ j ∈ Candidates, ∀ hfreej : P.Free j,
      P.splitWorstMass i hfree ≤ P.splitWorstMass j hfreej

theorem Pattern.InfoOptimalSplit.best {n : Nat} {P : Pattern n}
    {Candidates : Finset (Fin n)} {i j : Fin n}
    {hfreei : P.Free i} (hopt : P.InfoOptimalSplit Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.splitWorstMass i hfreei ≤ P.splitWorstMass j hfreej :=
  hopt.2 j hj hfreej

theorem Pattern.InfoOptimalSplit.best_uncertainty {n : Nat} {P : Pattern n}
    {Candidates : Finset (Fin n)} {i j : Fin n}
    {hfreei : P.Free i} (hopt : P.InfoOptimalSplit Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.splitWorstUncertainty i hfreei ≤ P.splitWorstUncertainty j hfreej :=
  uncertaintyBits_mono (hopt.best hj hfreej)

/-- Guaranteed mass removed by a split in the worst branch. Maximizing this is equivalent
to minimizing worst-child mass. -/
def Pattern.guaranteedMassPruned {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) : Nat :=
  P.mass - P.splitWorstMass i hfree

/-- Guaranteed uncertainty removed by a split in the worst branch. This is the entropy-style
branch score: how many base-2 uncertainty units are certified gone no matter which child wins. -/
def Pattern.guaranteedUncertaintyDrop {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) : Nat :=
  uncertaintyBits P.mass - P.splitWorstUncertainty i hfree

/-- Guaranteed blocker-aware candidate mass removed by a split in the worst branch. -/
def Pattern.guaranteedSolutionMassPruned {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Nat :=
  P.solutionMass Bs - P.splitWorstSolutionMass Bs i hfree

/-- Guaranteed blocker-aware uncertainty removed by a split in the worst branch. -/
def Pattern.guaranteedSolutionUncertaintyDrop {n : Nat} (Bs : List (Blocker n))
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Nat :=
  uncertaintyBits (P.solutionMass Bs) - P.splitWorstSolutionUncertainty Bs i hfree

theorem Pattern.InfoOptimalSplit.maximizes_guaranteedMassPruned {n : Nat}
    {P : Pattern n} {Candidates : Finset (Fin n)} {i j : Fin n}
    {hfreei : P.Free i} (hopt : P.InfoOptimalSplit Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.guaranteedMassPruned j hfreej ≤ P.guaranteedMassPruned i hfreei := by
  unfold Pattern.guaranteedMassPruned
  exact Nat.sub_le_sub_left (hopt.best hj hfreej) P.mass

theorem Pattern.InfoOptimalSplit.maximizes_guaranteedUncertaintyDrop {n : Nat}
    {P : Pattern n} {Candidates : Finset (Fin n)} {i j : Fin n}
    {hfreei : P.Free i} (hopt : P.InfoOptimalSplit Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.guaranteedUncertaintyDrop j hfreej ≤ P.guaranteedUncertaintyDrop i hfreei := by
  unfold Pattern.guaranteedUncertaintyDrop
  exact Nat.sub_le_sub_left (hopt.best_uncertainty hj hfreej) (uncertaintyBits P.mass)

/-- Blocker-aware information optimality: minimize worst remaining uncovered-candidate mass. -/
def Pattern.InfoOptimalSolutionSplit {n : Nat} (Bs : List (Blocker n)) (P : Pattern n)
    (Candidates : Finset (Fin n)) (i : Fin n) (hfree : P.Free i) : Prop :=
  i ∈ Candidates ∧
    ∀ j ∈ Candidates, ∀ hfreej : P.Free j,
      P.splitWorstSolutionMass Bs i hfree ≤ P.splitWorstSolutionMass Bs j hfreej

theorem Pattern.InfoOptimalSolutionSplit.best {n : Nat} {Bs : List (Blocker n)}
    {P : Pattern n} {Candidates : Finset (Fin n)} {i j : Fin n}
    {hfreei : P.Free i} (hopt : P.InfoOptimalSolutionSplit Bs Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.splitWorstSolutionMass Bs i hfreei ≤ P.splitWorstSolutionMass Bs j hfreej :=
  hopt.2 j hj hfreej

theorem Pattern.InfoOptimalSolutionSplit.best_uncertainty {n : Nat}
    {Bs : List (Blocker n)} {P : Pattern n} {Candidates : Finset (Fin n)}
    {i j : Fin n} {hfreei : P.Free i}
    (hopt : P.InfoOptimalSolutionSplit Bs Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.splitWorstSolutionUncertainty Bs i hfreei ≤
      P.splitWorstSolutionUncertainty Bs j hfreej :=
  uncertaintyBits_mono (hopt.best hj hfreej)

theorem Pattern.InfoOptimalSolutionSplit.maximizes_guaranteedSolutionMassPruned {n : Nat}
    {Bs : List (Blocker n)} {P : Pattern n} {Candidates : Finset (Fin n)}
    {i j : Fin n} {hfreei : P.Free i}
    (hopt : P.InfoOptimalSolutionSplit Bs Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.guaranteedSolutionMassPruned Bs j hfreej ≤
      P.guaranteedSolutionMassPruned Bs i hfreei := by
  unfold Pattern.guaranteedSolutionMassPruned
  exact Nat.sub_le_sub_left (hopt.best hj hfreej) (P.solutionMass Bs)

theorem Pattern.InfoOptimalSolutionSplit.maximizes_guaranteedSolutionUncertaintyDrop {n : Nat}
    {Bs : List (Blocker n)} {P : Pattern n} {Candidates : Finset (Fin n)}
    {i j : Fin n} {hfreei : P.Free i}
    (hopt : P.InfoOptimalSolutionSplit Bs Candidates i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.guaranteedSolutionUncertaintyDrop Bs j hfreej ≤
      P.guaranteedSolutionUncertaintyDrop Bs i hfreei := by
  unfold Pattern.guaranteedSolutionUncertaintyDrop
  exact Nat.sub_le_sub_left (hopt.best_uncertainty hj hfreej)
    (uncertaintyBits (P.solutionMass Bs))

/-- If an information-optimal split is also perfectly balanced, then it Pareto-dominates every
candidate free split: no worse worst child, and minimum possible imbalance. -/
theorem Pattern.InfoOptimalSplit.paretoDominates_of_balanced {n : Nat}
    {P : Pattern n} {Candidates : Finset (Fin n)} {i j : Fin n}
    {hfreei : P.Free i} (hopt : P.InfoOptimalSplit Candidates i hfreei)
    (hbal : P.BalancedSplit i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.SplitParetoDominates i hfreei j hfreej := by
  constructor
  · exact hopt.best hj hfreej
  · have hzero : P.splitImbalance i hfreei = 0 :=
      (P.splitImbalance_eq_zero_iff_balanced i hfreei).mpr hbal
    rw [hzero]
    exact Nat.zero_le _

/-- Blocker-aware version: an optimal split that also balances the remaining SAT-candidate
mass Pareto-dominates every candidate free split. -/
theorem Pattern.InfoOptimalSolutionSplit.paretoDominates_of_balanced {n : Nat}
    {Bs : List (Blocker n)} {P : Pattern n} {Candidates : Finset (Fin n)}
    {i j : Fin n} {hfreei : P.Free i}
    (hopt : P.InfoOptimalSolutionSplit Bs Candidates i hfreei)
    (hbal : P.SolutionBalancedSplit Bs i hfreei)
    (hj : j ∈ Candidates) (hfreej : P.Free j) :
    P.SolutionSplitParetoDominates Bs i hfreei j hfreej := by
  constructor
  · exact hopt.best hj hfreej
  · have hzero : P.splitSolutionImbalance Bs i hfreei = 0 :=
      (P.splitSolutionImbalance_eq_zero_iff_balanced Bs i hfreei).mpr hbal
    rw [hzero]
    exact Nat.zero_le _

/-- A generic coordinate score for branching inside a face. Lower is better. -/
abbrev BranchScore (n : Nat) := Pattern n → Fin n → Nat

/-- A coordinate is score-optimal among candidates when it minimizes the supplied score. -/
def ScoreOptimalSplit {n : Nat} (score : BranchScore n) (P : Pattern n)
    (Candidates : Finset (Fin n)) (i : Fin n) : Prop :=
  i ∈ Candidates ∧ ∀ j ∈ Candidates, score P i ≤ score P j

theorem ScoreOptimalSplit.best {n : Nat} {score : BranchScore n} {P : Pattern n}
    {Candidates : Finset (Fin n)} {i j : Fin n}
    (hopt : ScoreOptimalSplit score P Candidates i) (hj : j ∈ Candidates) :
    score P i ≤ score P j :=
  hopt.2 j hj

/-- Information score with an explicit proof that the candidate coordinate is free. -/
def infoBranchScore {n : Nat} (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Nat :=
  P.splitWorstMass i hfree

/-- Executable information score. Free coordinates are scored by worst-child mass; non-free
coordinates are penalized above the parent mass so a minimizer avoids them whenever possible. -/
def Pattern.totalInfoScore {n : Nat} (P : Pattern n) (i : Fin n) : Nat :=
  if hfree : P.Free i then P.splitWorstMass i hfree else P.mass + 1

theorem Pattern.totalInfoScore_le_mass_of_free {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.totalInfoScore i ≤ P.mass := by
  rw [Pattern.totalInfoScore]
  simp [hfree, P.splitWorstMass_le_mass i hfree]

theorem Pattern.mass_lt_totalInfoScore_of_not_free {n : Nat} (P : Pattern n) (i : Fin n)
    (hnot : ¬ P.Free i) :
    P.mass < P.totalInfoScore i := by
  rw [Pattern.totalInfoScore]
  simp [hnot]

/-- If an executable information-score optimum has any free candidate available, it must
choose a free coordinate. This is the first "score contract" for implementable heuristics. -/
theorem ScoreOptimalSplit.free_of_exists_free_candidate {n : Nat}
    {P : Pattern n} {Candidates : Finset (Fin n)} {i : Fin n}
    (hopt : ScoreOptimalSplit (fun P i => P.totalInfoScore i) P Candidates i)
    (hexists : ∃ j ∈ Candidates, P.Free j) :
    P.Free i := by
  by_contra hnot
  obtain ⟨j, hj, hfreej⟩ := hexists
  have hgt : P.mass < P.totalInfoScore i :=
    P.mass_lt_totalInfoScore_of_not_free i hnot
  have hle_i_j : P.totalInfoScore i ≤ P.totalInfoScore j := hopt.best hj
  have hle_j : P.totalInfoScore j ≤ P.mass :=
    P.totalInfoScore_le_mass_of_free j hfreej
  omega

/-- Information-optimality is exactly score-optimality for `splitWorstMass`, with the
free-coordinate proofs made explicit. -/
theorem infoOptimal_iff_scoreOptimal {n : Nat} (P : Pattern n)
    (Candidates : Finset (Fin n)) (i : Fin n) (hfree : P.Free i) :
    P.InfoOptimalSplit Candidates i hfree ↔
      i ∈ Candidates ∧
        ∀ j ∈ Candidates, ∀ hfreej : P.Free j,
          infoBranchScore P i hfree ≤ infoBranchScore P j hfreej := by
  rfl

/-- If a vertex covers both patterns, then the patterns are compatible. -/
theorem Pattern.compatible_of_common_vertex {n : Nat} {P Q : Pattern n} {v : Vertex n}
    (hP : P.Covers v) (hQ : Q.Covers v) :
    P.Compatible Q := by
  intro i hbad
  rcases hbad with htf | hft
  · have ht : v i = true := hP (i, true) htf.1
    have hf : v i = false := hQ (i, false) htf.2
    rw [ht] at hf
    contradiction
  · have hf : v i = false := hP (i, false) hft.1
    have ht : v i = true := hQ (i, true) hft.2
    rw [ht] at hf
    contradiction

/-- Incompatible patterns have empty semantic intersection. -/
theorem Pattern.no_common_vertex_of_incompatible {n : Nat} {P Q : Pattern n}
    (hbad : P.Incompatible Q) :
    ¬ ∃ v : Vertex n, P.Covers v ∧ Q.Covers v := by
  rintro ⟨v, hP, hQ⟩
  rcases hbad with ⟨i, hi⟩
  exact (Pattern.compatible_of_common_vertex hP hQ) i hi

/-- The syntactic intersection of two compatible patterns: fix every bit fixed by either. -/
def Pattern.unionOfCompatible {n : Nat} (P Q : Pattern n) (hcompat : P.Compatible Q) :
    Pattern n where
  fixed := P.fixed ∪ Q.fixed
  consistent := by
    intro i hbad
    rcases hbad with ⟨htrue, hfalse⟩
    rw [Finset.mem_union] at htrue hfalse
    rcases htrue with htP | htQ
    · rcases hfalse with hfP | hfQ
      · exact P.consistent i ⟨htP, hfP⟩
      · exact hcompat i (Or.inl ⟨htP, hfQ⟩)
    · rcases hfalse with hfP | hfQ
      · exact hcompat i (Or.inr ⟨hfP, htQ⟩)
      · exact Q.consistent i ⟨htQ, hfQ⟩

/-- Compatible pattern intersection is represented by unioning their fixed assignments. -/
theorem Pattern.covers_unionOfCompatible_iff {n : Nat} (P Q : Pattern n)
    (hcompat : P.Compatible Q) (v : Vertex n) :
    (P.unionOfCompatible Q hcompat).Covers v ↔ P.Covers v ∧ Q.Covers v := by
  constructor
  · intro hcov
    constructor
    · intro ib hib
      exact hcov ib (by simp [Pattern.unionOfCompatible, hib])
    · intro ib hib
      exact hcov ib (by simp [Pattern.unionOfCompatible, hib])
  · rintro ⟨hP, hQ⟩ ib hib
    change ib ∈ P.fixed ∪ Q.fixed at hib
    rw [Finset.mem_union] at hib
    rcases hib with hPi | hQi
    · exact hP ib hPi
    · exact hQ ib hQi

/-- Semantic merge soundness: if two derived patterns cover the two Boolean halves of
`A` along coordinate `i`, then together they cover all of `A`.

This is the geometric content of resolution, without committing yet to a syntactic
merge constructor for patterns. -/
theorem pattern_merge_sound_semantic {n : Nat} (A P0 P1 : Pattern n) (i : Fin n)
    (h0 : ∀ v, A.Covers v ∧ v i = false → P0.Covers v)
    (h1 : ∀ v, A.Covers v ∧ v i = true → P1.Covers v) :
    ∀ v, A.Covers v → P0.Covers v ∨ P1.Covers v := by
  intro v hA
  cases hv : v i
  · left
    exact h0 v ⟨hA, hv⟩
  · right
    exact h1 v ⟨hA, hv⟩

/-! ### §E0 Solver-accounting semantics: bad regions -/

/-- UNSAT is exactly the statement that the whole cube, represented by the empty
pattern, is a bad region. -/
theorem coverUNSATList_iff_emptyPattern_bad {n : Nat} (Bs : List (Blocker n)) :
    CoverUNSATList Bs ↔ BadRegionFor Bs (Pattern.empty n) := by
  unfold CoverUNSATList CoverSATList BadRegionFor
  constructor
  · intro hunsat v _ hun
    exact hunsat ⟨v, hun⟩
  · intro hbad hsat
    obtain ⟨v, hun⟩ := hsat
    exact hbad v (Pattern.empty_covers v) hun

/-- A pattern implies a primitive blocker when every vertex in the pattern is blocked
by that blocker. This is the geometric conflict test. -/
def Pattern.ImpliesBlocker {n : Nat} (P : Pattern n) (B : Blocker n) : Prop :=
  ∀ v : Vertex n, P.Covers v → B.Covers v

/-- If a search face lies inside one listed blocker, the whole face is bad. -/
theorem badRegion_of_implies_blocker {n : Nat} {Bs : List (Blocker n)}
    {P : Pattern n} {B : Blocker n} (hB : B ∈ Bs)
    (himp : P.ImpliesBlocker B) :
    BadRegionFor Bs P := by
  intro v hPv hun
  exact hun B hB (himp v hPv)

/-- Concrete conflict test for a clean 3-blocker: if a pattern fixes all three blocker
coordinates to their bad bits, then the whole pattern lies inside that blocker. -/
theorem Pattern.impliesBlocker_of_fixes {n : Nat} {P : Pattern n} {B : Blocker n}
    (hi : P.Fixes B.i B.bi) (hj : P.Fixes B.j B.bj) (hk : P.Fixes B.k B.bk) :
    P.ImpliesBlocker B := by
  intro v hPv
  exact ⟨hPv (B.i, B.bi) hi, hPv (B.j, B.bj) hj, hPv (B.k, B.bk) hk⟩

/-- A conflict at a face means the face is contained in one primitive blocker. -/
def ConflictAt {n : Nat} (Bs : List (Blocker n)) (P : Pattern n) : Prop :=
  ∃ B ∈ Bs, P.ImpliesBlocker B

/-- A conflict face is a bad region. -/
theorem ConflictAt.badRegion {n : Nat} {Bs : List (Blocker n)} {P : Pattern n}
    (h : ConflictAt Bs P) :
    BadRegionFor Bs P := by
  rcases h with ⟨B, hB, himp⟩
  exact badRegion_of_implies_blocker hB himp

/-! ### §E0.1 Classical SAT rules as hypercube rules -/

/-- A blocker semantically fixes coordinate `x` to bit `b`: every vertex inside the blocked
subcube has that bit. This is the hypercube-native literal occurrence. -/
def Blocker.FixesBit {n : Nat} (B : Blocker n) (x : Fin n) (b : Bool) : Prop :=
  ∀ v : Vertex n, B.Covers v → v x = b

/-- Syntactic fixed-bit occurrence inside a blocker. This is the finite occurrence relation
used for polarity counts, degrees, purity, and incidence-style rules. -/
def Blocker.HasFixedBit {n : Nat} (B : Blocker n) (x : Fin n) (b : Bool) : Prop :=
  (x = B.i ∧ b = B.bi) ∨ (x = B.j ∧ b = B.bj) ∨ (x = B.k ∧ b = B.bk)

instance {n : Nat} (B : Blocker n) (x : Fin n) (b : Bool) :
    Decidable (B.HasFixedBit x b) := by
  unfold Blocker.HasFixedBit
  infer_instance

instance {n : Nat} (x : Fin n) (b : Bool) :
    DecidablePred (fun B : Blocker n => B.HasFixedBit x b) := by
  intro B
  infer_instance

/-- A blocker fixes its first listed coordinate to its first listed bit. -/
theorem Blocker.fixesBit_i {n : Nat} (B : Blocker n) :
    B.FixesBit B.i B.bi := by
  intro v hcover
  exact hcover.1

/-- A blocker fixes its second listed coordinate to its second listed bit. -/
theorem Blocker.fixesBit_j {n : Nat} (B : Blocker n) :
    B.FixesBit B.j B.bj := by
  intro v hcover
  exact hcover.2.1

/-- A blocker fixes its third listed coordinate to its third listed bit. -/
theorem Blocker.fixesBit_k {n : Nat} (B : Blocker n) :
    B.FixesBit B.k B.bk := by
  intro v hcover
  exact hcover.2.2

/-- Every support coordinate of a clean blocker has a corresponding fixed bit. -/
theorem Blocker.exists_fixesBit_of_mem_support {n : Nat} (B : Blocker n)
    {x : Fin n} (hx : x ∈ B.support) :
    ∃ b : Bool, B.FixesBit x b := by
  rw [Blocker.mem_support] at hx
  rcases hx with rfl | rfl | rfl
  · exact ⟨B.bi, B.fixesBit_i⟩
  · exact ⟨B.bj, B.fixesBit_j⟩
  · exact ⟨B.bk, B.fixesBit_k⟩

/-- Syntactic occurrence implies semantic fixation. -/
theorem Blocker.fixesBit_of_hasFixedBit {n : Nat} {B : Blocker n}
    {x : Fin n} {b : Bool} (h : B.HasFixedBit x b) :
    B.FixesBit x b := by
  rcases h with h | h | h
  · rcases h with ⟨rfl, rfl⟩
    exact B.fixesBit_i
  · rcases h with ⟨rfl, rfl⟩
    exact B.fixesBit_j
  · rcases h with ⟨rfl, rfl⟩
    exact B.fixesBit_k

/-- A coordinate is in the blocker support exactly when it has some fixed-bit occurrence. -/
theorem Blocker.mem_support_iff_exists_hasFixedBit {n : Nat} (B : Blocker n) (x : Fin n) :
    x ∈ B.support ↔ ∃ b : Bool, B.HasFixedBit x b := by
  constructor
  · intro hx
    rw [Blocker.mem_support] at hx
    rcases hx with rfl | rfl | rfl
    · exact ⟨B.bi, Or.inl ⟨rfl, rfl⟩⟩
    · exact ⟨B.bj, Or.inr (Or.inl ⟨rfl, rfl⟩)⟩
    · exact ⟨B.bk, Or.inr (Or.inr ⟨rfl, rfl⟩)⟩
  · rintro ⟨b, hbit⟩
    rcases hbit with h | h | h
    · exact (Blocker.mem_support B x).mpr (Or.inl h.1)
    · exact (Blocker.mem_support B x).mpr (Or.inr (Or.inl h.1))
    · exact (Blocker.mem_support B x).mpr (Or.inr (Or.inr h.1))

/-- Count blockers in a family that syntactically fix coordinate `x` to bit `b`. -/
noncomputable def BlockerFamily.bitOccurrenceCount {n : Nat} (Bs : Finset (Blocker n))
    (x : Fin n) (b : Bool) : Nat := by
  classical
  exact (Bs.filter fun B => B.HasFixedBit x b).card

/-- Number of blockers mentioning a coordinate, ignoring which bit they fix. -/
noncomputable def BlockerFamily.variableDegree {n : Nat} (Bs : Finset (Blocker n))
    (x : Fin n) : Nat := by
  classical
  exact (Bs.filter fun B => x ∈ B.support).card

/-- A coordinate is syntactically pure when all its occurrences, if any, use one bit. -/
def BlockerFamily.PureCoordinate {n : Nat} (Bs : Finset (Blocker n)) (x : Fin n) : Prop :=
  BlockerFamily.bitOccurrenceCount Bs x true = 0 ∨
    BlockerFamily.bitOccurrenceCount Bs x false = 0

/-- A coordinate is balanced when both blocker-bit polarities occur equally often. -/
def BlockerFamily.BalancedCoordinate {n : Nat} (Bs : Finset (Blocker n)) (x : Fin n) : Prop :=
  BlockerFamily.bitOccurrenceCount Bs x true =
    BlockerFamily.bitOccurrenceCount Bs x false

/-- Bit occurrence count is bounded by the number of blockers. -/
theorem BlockerFamily.bitOccurrenceCount_le_card {n : Nat}
    (Bs : Finset (Blocker n)) (x : Fin n) (b : Bool) :
    BlockerFamily.bitOccurrenceCount Bs x b ≤ Bs.card := by
  unfold BlockerFamily.bitOccurrenceCount
  exact Finset.card_filter_le _ _

/-- Variable degree is bounded by the number of blockers. -/
theorem BlockerFamily.variableDegree_le_card {n : Nat}
    (Bs : Finset (Blocker n)) (x : Fin n) :
    BlockerFamily.variableDegree Bs x ≤ Bs.card := by
  unfold BlockerFamily.variableDegree
  exact Finset.card_filter_le _ _

/-- If there are no occurrences of bit `b`, then every occurrence of the coordinate is
purely the opposite bit. -/
theorem BlockerFamily.hasFixedBit_not_of_bitOccurrenceCount_eq_zero {n : Nat}
    {Bs : Finset (Blocker n)} {x : Fin n} {b : Bool}
    (hzero : BlockerFamily.bitOccurrenceCount Bs x b = 0)
    {B : Blocker n} (hB : B ∈ Bs) (hx : x ∈ B.support) :
    B.HasFixedBit x (!b) := by
  obtain ⟨c, hc⟩ := (B.mem_support_iff_exists_hasFixedBit x).mp hx
  by_cases hcb : c = b
  · subst c
    have hmem : B ∈ Bs.filter fun C => C.HasFixedBit x b := by
      exact Finset.mem_filter.mpr ⟨hB, hc⟩
    have hpos : 0 < (Bs.filter fun C => C.HasFixedBit x b).card :=
      Finset.card_pos.mpr ⟨B, hmem⟩
    unfold BlockerFamily.bitOccurrenceCount at hzero
    omega
  · have hcnot : c = !b := by
      cases b <;> cases c <;> simp at hcb ⊢
    rw [← hcnot]
    exact hc

/-- If a blocker fixes `x` to `b`, then no vertex with `x = !b` is covered by it. -/
theorem Blocker.not_covers_of_fixesBit_not {n : Nat} {B : Blocker n}
    {x : Fin n} {b : Bool} {v : Vertex n}
    (hfix : B.FixesBit x b) (hv : v x = !b) :
    ¬ B.Covers v := by
  intro hcover
  have hxb := hfix v hcover
  rw [hv] at hxb
  cases b <;> simp at hxb

/-- Blocker lists with every blocker mentioning `x` deleted. This is the residual instance
used by the pure-literal/autarky view. -/
def blockersWithoutCoord {n : Nat} (Bs : List (Blocker n)) (x : Fin n) : List (Blocker n) :=
  Bs.filter fun B => x ∉ B.support

/-- Coordinate `x` is pure with blocker bit `b` when every blocker that mentions `x`
fixes it to `b`. In ordinary clause language, assigning `x = !b` satisfies all clauses
where `x` occurs. -/
def PureBlockerBit {n : Nat} (Bs : List (Blocker n)) (x : Fin n) (b : Bool) : Prop :=
  ∀ B ∈ Bs, x ∈ B.support → B.FixesBit x b

/-- Pure-literal elimination is sound: if the residual instance not mentioning `x` has a
hole, and all deleted blockers are killed by setting `x = !b`, then the original instance
has a hole. -/
theorem pureBlockerBit_lift_sat {n : Nat} {Bs : List (Blocker n)}
    {x : Fin n} {b : Bool} (hpure : PureBlockerBit Bs x b)
    (hsat : CoverSATList (blockersWithoutCoord Bs x)) :
    CoverSATList Bs := by
  rcases hsat with ⟨v, hv⟩
  let w : Vertex n := fun y => if y = x then !b else v y
  refine ⟨w, ?_⟩
  intro B hB hcover
  by_cases hx : x ∈ B.support
  · have hfix := hpure B hB hx
    have hwx : w x = !b := by simp [w]
    exact B.not_covers_of_fixesBit_not hfix hwx hcover
  · have hBres : B ∈ blockersWithoutCoord Bs x := by
      exact List.mem_filter.mpr
        ⟨hB, (show decide (x ∉ B.support) = true from decide_eq_true hx)⟩
    have hvi : v B.i = B.bi := by
      have hne : B.i ≠ x := by
        intro hix
        exact hx (by rw [Blocker.mem_support]; exact Or.inl hix.symm)
      have hwi : w B.i = v B.i := by simp [w, hne]
      rw [← hwi]
      exact hcover.1
    have hvj : v B.j = B.bj := by
      have hne : B.j ≠ x := by
        intro hjx
        exact hx (by rw [Blocker.mem_support]; exact Or.inr (Or.inl hjx.symm))
      have hwj : w B.j = v B.j := by simp [w, hne]
      rw [← hwj]
      exact hcover.2.1
    have hvk : v B.k = B.bk := by
      have hne : B.k ≠ x := by
        intro hkx
        exact hx (by rw [Blocker.mem_support]; exact Or.inr (Or.inr hkx.symm))
      have hwk : w B.k = v B.k := by simp [w, hne]
      rw [← hwk]
      exact hcover.2.2
    exact hv B hBres ⟨hvi, hvj, hvk⟩

/-- Pure-literal reduction on the UNSAT side: if a coordinate is pure, then an UNSAT
certificate for the original instance must already live in the residual blockers that do not
mention that coordinate. Equivalently, pure occurrences carry no refutation information. -/
theorem pureBlockerBit_residual_unsat {n : Nat} {Bs : List (Blocker n)}
    {x : Fin n} {b : Bool} (hpure : PureBlockerBit Bs x b)
    (hunsat : CoverUNSATList Bs) :
    CoverUNSATList (blockersWithoutCoord Bs x) := by
  intro hsat
  exact hunsat (pureBlockerBit_lift_sat hpure hsat)

/-- Removing all blockers mentioning one coordinate never increases the blocker count. -/
theorem blockersWithoutCoord_length_le {n : Nat} (Bs : List (Blocker n)) (x : Fin n) :
    (blockersWithoutCoord Bs x).length ≤ Bs.length := by
  unfold blockersWithoutCoord
  exact List.length_filter_le _ _

/-- If some blocker mentions the removed coordinate, the pure/residual reduction strictly
shrinks the list. This is an information-reduction rule: the residual refutation has fewer
primitive blockers to explain. -/
theorem blockersWithoutCoord_length_lt_of_exists_mem_support {n : Nat}
    {Bs : List (Blocker n)} {x : Fin n}
    (hexists : ∃ B ∈ Bs, x ∈ B.support) :
    (blockersWithoutCoord Bs x).length < Bs.length := by
  unfold blockersWithoutCoord
  have hle : (Bs.filter fun B => x ∉ B.support).length ≤ Bs.length :=
    List.length_filter_le _ _
  by_contra hnot
  have heq : (Bs.filter fun B => x ∉ B.support).length = Bs.length := by
    omega
  have hall :=
    (List.length_filter_eq_length_iff
      (p := fun B : Blocker n => decide (x ∉ B.support)) (l := Bs)).mp heq
  obtain ⟨B, hB, hx⟩ := hexists
  have hkeep := hall B hB
  simp [hx] at hkeep

/-- Combined pure-coordinate UNSAT reduction: under purity, an UNSAT instance reduces to a
smaller residual instance whenever the coordinate actually occurs. -/
theorem pureBlockerBit_residual_unsat_and_smaller {n : Nat} {Bs : List (Blocker n)}
    {x : Fin n} {b : Bool} (hpure : PureBlockerBit Bs x b)
    (hunsat : CoverUNSATList Bs) (hoccurs : ∃ B ∈ Bs, x ∈ B.support) :
    CoverUNSATList (blockersWithoutCoord Bs x) ∧
      (blockersWithoutCoord Bs x).length < Bs.length :=
  ⟨pureBlockerBit_residual_unsat hpure hunsat,
    blockersWithoutCoord_length_lt_of_exists_mem_support hoccurs⟩

/-- Pure-slab rule: if every blocker mentions coordinate `x`, and every occurrence fixes
the same bit `b`, then the entire slice `x = !b` is uncovered. -/
theorem pureBlockerBit_allMention_uncovered_slab {n : Nat} {Bs : List (Blocker n)}
    {x : Fin n} {b : Bool} (hpure : PureBlockerBit Bs x b)
    (hall : ∀ B ∈ Bs, x ∈ B.support) :
    ∀ v : Vertex n, v x = !b → IsUncovered Bs v := by
  intro v hv B hB hcover
  have hfix : B.FixesBit x b := hpure B hB (hall B hB)
  exact B.not_covers_of_fixesBit_not hfix hv hcover

/-- If all blockers are killed by one pure coordinate slice, the instance is satisfiable. -/
theorem pureBlockerBit_allMention_sat {n : Nat} {Bs : List (Blocker n)}
    {x : Fin n} {b : Bool} (hpure : PureBlockerBit Bs x b)
    (hall : ∀ B ∈ Bs, x ∈ B.support) :
    CoverSATList Bs := by
  let v : Vertex n := fun y => if y = x then !b else false
  refine ⟨v, ?_⟩
  apply pureBlockerBit_allMention_uncovered_slab hpure hall
  simp [v]

/-- Pure-slab SAT rule for compact inputs. If every blocker mentions `x` and fixes that
coordinate to the same bit `b`, then the opposite slice contains satisfying assignments. -/
theorem CoverInput.sat_of_pure_slab (I : CoverInput) (x : Fin I.n) (b : Bool)
    (hpure : PureBlockerBit I.blockers x b)
    (hall : ∀ B ∈ I.blockers, x ∈ B.support) :
    CoverSAT I :=
  pureBlockerBit_allMention_sat hpure hall

/-- Necessary condition for a full cover: no coordinate can be both present in every blocker
and pure with one fixed blocker-bit. Otherwise the opposite slab is an uncovered subcube. -/
theorem coverUNSATList_no_pure_allMention {n : Nat} {Bs : List (Blocker n)}
    {x : Fin n} {b : Bool} (hunsat : CoverUNSATList Bs)
    (hpure : PureBlockerBit Bs x b) :
    ¬ ∀ B ∈ Bs, x ∈ B.support := by
  intro hall
  exact hunsat (pureBlockerBit_allMention_sat hpure hall)

/-- Compact-input version of `coverUNSATList_no_pure_allMention`. -/
theorem CoverInput.unsat_no_pure_slab (I : CoverInput) (x : Fin I.n) (b : Bool)
    (hunsat : CoverUNSAT I) (hpure : PureBlockerBit I.blockers x b) :
    ¬ ∀ B ∈ I.blockers, x ∈ B.support :=
  coverUNSATList_no_pure_allMention hunsat hpure

/-- Semantic unit propagation: if assigning `i = bad` inside `P` would place the face
inside a listed blocker, then any uncovered vertex in `P` must take `i = !bad`. -/
theorem unit_propagation_sound {n : Nat} {Bs : List (Blocker n)} {P : Pattern n}
    {B : Blocker n} {i : Fin n} {bad : Bool} (hfree : P.Free i)
    (hB : B ∈ Bs) (himp : (P.assign i bad hfree).ImpliesBlocker B) :
    ∀ v : Vertex n, P.Covers v → IsUncovered Bs v → v i = !bad := by
  intro v hP hun
  by_cases hbadbit : v i = bad
  · have hassign : (P.assign i bad hfree).Covers v :=
      (P.assign_covers_iff i bad hfree v).mpr ⟨hP, hbadbit⟩
    exact False.elim (hun B hB (himp v hassign))
  · cases bad
    · cases hv : v i
      · exact False.elim (hbadbit hv)
      · rfl
    · cases hv : v i
      · rfl
      · exact False.elim (hbadbit hv)

/-- A forced bit in a search face: every still-possible satisfying vertex in `P`
sets coordinate `i` to `b`. Unit propagation, failed literals, and backbones all land here. -/
def ForcedBitInFace {n : Nat} (Bs : List (Blocker n)) (P : Pattern n)
    (i : Fin n) (b : Bool) : Prop :=
  ∀ v : Vertex n, P.Covers v → IsUncovered Bs v → v i = b

/-- Failed-literal rule: if the `bad` branch of a free coordinate is already a certified
bad region, every solution in the parent face must take the opposite bit. -/
theorem failed_literal_sound {n : Nat} {Bs : List (Blocker n)} {P : Pattern n}
    {i : Fin n} {bad : Bool} (hfree : P.Free i)
    (hbad : BadRegionFor Bs (P.assign i bad hfree)) :
    ForcedBitInFace Bs P i (!bad) := by
  intro v hP hun
  by_cases hbadbit : v i = bad
  · have hassign : (P.assign i bad hfree).Covers v :=
      (P.assign_covers_iff i bad hfree v).mpr ⟨hP, hbadbit⟩
    exact False.elim (hbad v hassign hun)
  · cases bad
    · cases hv : v i
      · exact False.elim (hbadbit hv)
      · rfl
    · cases hv : v i
      · rfl
      · exact False.elim (hbadbit hv)

/-- A backbone bit is forced over the whole instance, not merely inside a search face. -/
def BackboneBit {n : Nat} (Bs : List (Blocker n)) (i : Fin n) (b : Bool) : Prop :=
  ∀ v : Vertex n, IsUncovered Bs v → v i = b

/-- Root-level failed literal gives a backbone assignment. -/
theorem backboneBit_of_failed_literal {n : Nat} {Bs : List (Blocker n)}
    {i : Fin n} {bad : Bool} (hfree : (Pattern.empty n).Free i)
    (hbad : BadRegionFor Bs ((Pattern.empty n).assign i bad hfree)) :
    BackboneBit Bs i (!bad) := by
  intro v hun
  exact failed_literal_sound hfree hbad v (Pattern.empty_covers v) hun

/-- Badness is monotone toward smaller faces: if `P` is bad and `Q ⊆ P`, then `Q`
is bad. -/
theorem BadRegionFor.mono_pattern {n : Nat} {Bs : List (Blocker n)}
    {P Q : Pattern n} (hbad : BadRegionFor Bs P)
    (hQP : ∀ v, Q.Covers v → P.Covers v) :
    BadRegionFor Bs Q := by
  intro v hQv hun
  exact hbad v (hQP v hQv) hun

/-- Syntactic refinement form of bad-region monotonicity. -/
theorem BadRegionFor.of_le {n : Nat} {Bs : List (Blocker n)}
    {P Q : Pattern n} (hbad : BadRegionFor Bs P) (hPQ : P.Le Q) :
    BadRegionFor Bs Q :=
  hbad.mono_pattern (Pattern.covers_of_le hPQ)

/-- Bad-region merge/split rule: if both Boolean halves of `A` along coordinate `i`
are bad, then `A` is bad. This is the certified DPLL accounting step. -/
theorem BadRegionFor.merge_split {n : Nat} {Bs : List (Blocker n)}
    {A P0 P1 : Pattern n} (i : Fin n)
    (h0bad : BadRegionFor Bs P0) (h1bad : BadRegionFor Bs P1)
    (h0 : ∀ v, A.Covers v ∧ v i = false → P0.Covers v)
    (h1 : ∀ v, A.Covers v ∧ v i = true → P1.Covers v) :
    BadRegionFor Bs A := by
  intro v hA hun
  cases hv : v i
  · exact h0bad v (h0 v ⟨hA, hv⟩) hun
  · exact h1bad v (h1 v ⟨hA, hv⟩) hun

/-- DPLL branch accounting in its direct face form: if both assigned children of a free
coordinate are bad, then the parent face is bad. -/
theorem BadRegionFor.merge_assign {n : Nat} {Bs : List (Blocker n)}
    {P : Pattern n} {i : Fin n} (hfree : P.Free i)
    (hfalse : BadRegionFor Bs (P.assign i false hfree))
    (htrue : BadRegionFor Bs (P.assign i true hfree)) :
    BadRegionFor Bs P := by
  apply BadRegionFor.merge_split (i := i) hfalse htrue
  · intro v hv
    exact (P.assign_covers_iff i false hfree v).mpr hv
  · intro v hv
    exact (P.assign_covers_iff i true hfree v).mpr hv

/-- Semantic resolution rule: a learned region obtained by merging the two Boolean
children of a face is sound exactly when both children are sound. -/
def ResolutionMergeRule {n : Nat} (Bs : List (Blocker n)) (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) : Prop :=
  BadRegionFor Bs (P.assign i false hfree) →
  BadRegionFor Bs (P.assign i true hfree) →
  BadRegionFor Bs P

theorem resolutionMergeRule_sound {n : Nat} (Bs : List (Blocker n)) (P : Pattern n)
    (i : Fin n) (hfree : P.Free i) :
    ResolutionMergeRule Bs P i hfree :=
  BadRegionFor.merge_assign hfree

/-- Subsumption/deletion rule for learned regions: a bad larger face subsumes every
smaller face refining it. -/
def PatternSubsumes {n : Nat} (P Q : Pattern n) : Prop :=
  P.Le Q

theorem patternSubsumption_sound {n : Nat} {Bs : List (Blocker n)}
    {P Q : Pattern n} (hbad : BadRegionFor Bs P) (hsub : PatternSubsumes P Q) :
    BadRegionFor Bs Q :=
  BadRegionFor.of_le hbad hsub

/-- A bad-region derivation is a proof object for CDCL/resolution-style reasoning:
all listed learned regions are semantically bad, and the conclusion is semantically bad. -/
structure BadRegionDerivation {n : Nat} (Bs : List (Blocker n)) where
  lines : List (Pattern n)
  conclusion : Pattern n
  lineSound : ∀ P ∈ lines, BadRegionFor Bs P
  conclusionSound : BadRegionFor Bs conclusion

namespace BadRegionDerivation

/-- Number of learned/derived regions in the derivation. -/
def lineCount {n : Nat} {Bs : List (Blocker n)} (π : BadRegionDerivation Bs) : Nat :=
  π.lines.length

/-- Width accountant for a derivation: the largest width among the conclusion and all lines. -/
def width {n : Nat} {Bs : List (Blocker n)} (π : BadRegionDerivation Bs) : Nat :=
  (π.conclusion :: π.lines).foldl (fun acc P => max acc P.width) 0

/-- A derivation whose conclusion is the whole cube is an UNSAT refutation. -/
def Refutes {n : Nat} {Bs : List (Blocker n)} (π : BadRegionDerivation Bs) : Prop :=
  π.conclusion = Pattern.empty n

theorem coverUNSATList_of_refutes {n : Nat} {Bs : List (Blocker n)}
    {π : BadRegionDerivation Bs} (hrefute : π.Refutes) :
    CoverUNSATList Bs := by
  rw [coverUNSATList_iff_emptyPattern_bad]
  simpa [Refutes] using (hrefute ▸ π.conclusionSound)

end BadRegionDerivation

/-- Two blockers clash when they force opposite bits on some shared coordinate. In ordinary
CNF terms, the corresponding clauses are a hitting pair. -/
def Blocker.Clashes {n : Nat} (B C : Blocker n) : Prop :=
  ∃ x : Fin n, ∃ b : Bool, B.FixesBit x b ∧ C.FixesBit x (!b)

theorem Blocker.no_common_vertex_of_clashes {n : Nat} {B C : Blocker n}
    (hclash : B.Clashes C) :
    ¬ ∃ v : Vertex n, B.Covers v ∧ C.Covers v := by
  rintro ⟨v, hBv, hCv⟩
  rcases hclash with ⟨x, b, hBfix, hCfix⟩
  have hxb : v x = b := hBfix v hBv
  have hxnb : v x = !b := hCfix v hCv
  rw [hxb] at hxnb
  cases b <;> simp at hxnb

/-- Hitting clean 3-SAT/blocker family: distinct blockers have disjoint falsifying subcubes. -/
def HittingBlockerFamily {n : Nat} (Bs : Finset (Blocker n)) : Prop :=
  ∀ B ∈ Bs, ∀ C ∈ Bs, B ≠ C → B.Clashes C

theorem HittingBlockerFamily.pairwise_no_common_vertex {n : Nat}
    {Bs : Finset (Blocker n)} (hhit : HittingBlockerFamily Bs)
    {B C : Blocker n} (hB : B ∈ Bs) (hC : C ∈ Bs) (hne : B ≠ C) :
    ¬ ∃ v : Vertex n, B.Covers v ∧ C.Covers v :=
  HypercubeSAT.Blocker.no_common_vertex_of_clashes (hhit B hB C hC hne)

/-- Deficiency of a blocker family: clauses minus used variables, truncated at zero.
This is the standard SAT deficiency accountant in the hypercube-cover vocabulary. -/
def BlockerFamily.deficiency {n : Nat} (Bs : Finset (Blocker n)) : Nat :=
  Bs.card - (BlockerFamily.support Bs).card

/-- Excess of variables over clauses, the dual truncated accountant. -/
def BlockerFamily.variableExcess {n : Nat} (Bs : Finset (Blocker n)) : Nat :=
  (BlockerFamily.support Bs).card - Bs.card

/-- Classical families and solver/proof rules that we track against the hypercube model.
The constructors are labels; the semantic content is supplied by the definitions and
soundness lemmas above and elsewhere in the file. -/
inductive SATRuleKind where
  | unitPropagation
  | failedLiteral
  | pureLiteral
  | backbone
  | subsumption
  | resolution
  | selfSubsumingResolution
  | blockedClause
  | variableElimination
  | autarky
  | clauseLearning
  | restart
  | hitting
  | deficiency
  | separator
  | backdoor
  | symmetryBreaking
  | xorAffineReasoning
  | proofWidth
  | proofSize
  | localSearch
  | randomPhaseTransition
  deriving DecidableEq, Repr

/-- Structural features of the hypercube/blocker system that a rule may inspect or preserve. -/
inductive CubeStructureKind where
  | vertex
  | edge
  | face
  | blockerFootprint
  | support
  | occurrence
  | energy
  | ring
  | boundary
  | separator
  | backdoor
  | privateVertex
  | handoff
  | deficiency
  | proofWidth
  | proofSize
  | symmetry
  deriving DecidableEq, Repr

/-- The scale at which a rule acts. -/
inductive RuleScale where
  | vertexLocal
  | edgeLocal
  | faceLocal
  | coordinateLocal
  | familyGlobal
  | proofGlobal
  | ensembleGlobal
  deriving DecidableEq, Repr

/-- The semantic direction of a rule. SAT-preserving rules lift holes forward; UNSAT-preserving
rules lift full-cover certificates forward; equivalence gives both directions. -/
inductive RuleDirection where
  | satPreserving
  | unsatPreserving
  | equisatisfiable
  | derivesForcedBit
  | derivesBadRegion
  | derivesRefutation
  | classifiesInstance
  | heuristicOnly
  deriving DecidableEq, Repr

/-- The kind of evidence a rule produces or consumes. -/
inductive RuleEvidenceKind where
  | vertexWitness
  | blockerWitness
  | forcedBit
  | badRegion
  | derivation
  | countingInequality
  | decomposition
  | transform
  | taxonomyLabel
  deriving DecidableEq, Repr

/-- Metadata for a SAT rule in hypercube language. This lets us ask structured questions
about rules themselves: what scale do they act on, what do they preserve, and which
hypercube features do they touch? -/
structure SATRuleTaxonomy where
  kind : SATRuleKind
  scale : RuleScale
  direction : RuleDirection
  evidence : RuleEvidenceKind
  structures : List CubeStructureKind
  deriving Repr

/-- A first-pass classification of the standard rule names in this file. -/
def SATRuleKind.defaultTaxonomy : SATRuleKind → SATRuleTaxonomy
  | unitPropagation =>
      ⟨unitPropagation, .faceLocal, .derivesForcedBit, .forcedBit,
        [.face, .blockerFootprint, .support]⟩
  | failedLiteral =>
      ⟨failedLiteral, .faceLocal, .derivesForcedBit, .badRegion,
        [.face, .boundary, .energy]⟩
  | pureLiteral =>
      ⟨pureLiteral, .coordinateLocal, .satPreserving, .transform,
        [.occurrence, .support]⟩
  | backbone =>
      ⟨backbone, .coordinateLocal, .derivesForcedBit, .forcedBit,
        [.vertex, .energy]⟩
  | subsumption =>
      ⟨subsumption, .faceLocal, .derivesBadRegion, .badRegion,
        [.face, .proofWidth]⟩
  | resolution =>
      ⟨resolution, .faceLocal, .derivesBadRegion, .derivation,
        [.face, .boundary, .proofWidth, .proofSize]⟩
  | selfSubsumingResolution =>
      ⟨selfSubsumingResolution, .faceLocal, .derivesBadRegion, .derivation,
        [.face, .proofWidth]⟩
  | blockedClause =>
      ⟨blockedClause, .familyGlobal, .equisatisfiable, .transform,
        [.occurrence, .blockerFootprint]⟩
  | variableElimination =>
      ⟨variableElimination, .coordinateLocal, .equisatisfiable, .transform,
        [.occurrence, .proofSize]⟩
  | autarky =>
      ⟨autarky, .faceLocal, .satPreserving, .transform,
        [.face, .occurrence, .support]⟩
  | clauseLearning =>
      ⟨clauseLearning, .proofGlobal, .unsatPreserving, .badRegion,
        [.face, .proofWidth, .proofSize]⟩
  | restart =>
      ⟨restart, .proofGlobal, .heuristicOnly, .taxonomyLabel,
        [.proofSize]⟩
  | hitting =>
      ⟨hitting, .familyGlobal, .classifiesInstance, .taxonomyLabel,
        [.blockerFootprint, .occurrence, .deficiency]⟩
  | deficiency =>
      ⟨deficiency, .familyGlobal, .classifiesInstance, .countingInequality,
        [.support, .occurrence, .deficiency]⟩
  | separator =>
      ⟨separator, .familyGlobal, .classifiesInstance, .decomposition,
        [.separator, .support]⟩
  | backdoor =>
      ⟨backdoor, .familyGlobal, .classifiesInstance, .decomposition,
        [.backdoor, .support]⟩
  | symmetryBreaking =>
      ⟨symmetryBreaking, .familyGlobal, .equisatisfiable, .transform,
        [.symmetry, .vertex]⟩
  | xorAffineReasoning =>
      ⟨xorAffineReasoning, .familyGlobal, .derivesForcedBit, .countingInequality,
        [.occurrence, .proofWidth]⟩
  | proofWidth =>
      ⟨proofWidth, .proofGlobal, .classifiesInstance, .countingInequality,
        [.proofWidth, .face]⟩
  | proofSize =>
      ⟨proofSize, .proofGlobal, .classifiesInstance, .countingInequality,
        [.proofSize, .face]⟩
  | localSearch =>
      ⟨localSearch, .edgeLocal, .heuristicOnly, .vertexWitness,
        [.vertex, .edge, .energy, .boundary]⟩
  | randomPhaseTransition =>
      ⟨randomPhaseTransition, .ensembleGlobal, .classifiesInstance, .countingInequality,
        [.energy, .deficiency, .ring]⟩

@[simp] theorem SATRuleKind.defaultTaxonomy_kind (k : SATRuleKind) :
    k.defaultTaxonomy.kind = k := by
  cases k <;> rfl

/-- Structured questions we can ask of any rule while building the theory. -/
inductive RuleQuestionKind where
  | isSound
  | isComplete
  | preservesSAT
  | preservesUNSAT
  | isEquisatisfiable
  | hasPolynomialCheck
  | lowersWidth
  | lowersSize
  | exposesBackbone
  | detectsDecomposition
  | interactsWithHandoffs
  deriving DecidableEq, Repr

/-- A formal question about a named rule, ready to be attached to future theorem targets. -/
structure RuleQuestion where
  rule : SATRuleKind
  asks : RuleQuestionKind
  deriving Repr

/-- A transformation of compact cover inputs. -/
structure CoverTransform where
  run : CoverInput → CoverInput

/-- A transform is SAT-sound when holes in the transformed instance lift to holes in the
original instance. Simplification rules usually need at least this direction. -/
def CoverTransform.SATSound (T : CoverTransform) : Prop :=
  ∀ I : CoverInput, CoverSAT (T.run I) → CoverSAT I

/-- A transform is UNSAT-sound when full coverage of the original implies full coverage of
the transformed instance. -/
def CoverTransform.UNSATSound (T : CoverTransform) : Prop :=
  ∀ I : CoverInput, CoverUNSAT I → CoverUNSAT (T.run I)

/-- Equisatisfiability packages both SAT directions. -/
def CoverTransform.Equisatisfiable (T : CoverTransform) : Prop :=
  ∀ I : CoverInput, CoverSAT (T.run I) ↔ CoverSAT I

/-- The identity transform. -/
def CoverTransform.id : CoverTransform where
  run := fun I => I

theorem CoverTransform.id_equisatisfiable :
    CoverTransform.id.Equisatisfiable := by
  intro I
  simp [CoverTransform.Equisatisfiable, CoverTransform.id]

theorem CoverTransform.SATSound.of_equisatisfiable {T : CoverTransform}
    (h : T.Equisatisfiable) :
    T.SATSound := by
  intro I hsat
  exact (h I).mp hsat

theorem CoverTransform.UNSATSound.of_equisatisfiable {T : CoverTransform}
    (h : T.Equisatisfiable) :
    T.UNSATSound := by
  intro I hunsat hsat
  exact hunsat ((h I).mp hsat)

/-- A learned region is sound exactly when it is bad for the original blockers. -/
def LearnedRegionSound {n : Nat} (Bs : List (Blocker n)) (P : Pattern n) : Prop :=
  BadRegionFor Bs P

/-- A minimal solver trail entry. `reason = none` marks a decision assignment; `some P`
marks an assignment justified by a learned/original bad region. Later layers can refine
which regions are allowed as propagation reasons. -/
structure TrailEntry (n : Nat) where
  var : Fin n
  val : Bool
  level : Nat
  reason : Option (Pattern n)

abbrev Trail (n : Nat) := List (TrailEntry n)

namespace TrailEntry

/-- The fixed coordinate/bit pair represented by a trail entry. -/
def toPair {n : Nat} (e : TrailEntry n) : Fin n × Bool :=
  (e.var, e.val)

end TrailEntry

namespace Trail

/-- The finite set of coordinate/bit assignments currently recorded by the trail. -/
def fixed {n : Nat} (τ : Trail n) : Finset (Fin n × Bool) :=
  (τ.map TrailEntry.toPair).toFinset

/-- A vertex agrees with every assignment in a trail. -/
def Agrees {n : Nat} (τ : Trail n) (v : Vertex n) : Prop :=
  ∀ e ∈ τ, v e.var = e.val

/-- A trail is consistent when it never assigns both Boolean values to one coordinate. -/
def Consistent {n : Nat} (τ : Trail n) : Prop :=
  ∀ i : Fin n, ¬ (((i, true) ∈ τ.fixed) ∧ ((i, false) ∈ τ.fixed))

/-- Convert a consistent trail into the corresponding hypercube face. -/
def toPattern {n : Nat} (τ : Trail n) (hτ : τ.Consistent) : Pattern n where
  fixed := τ.fixed
  consistent := hτ

theorem mem_fixed_iff {n : Nat} (τ : Trail n) (ib : Fin n × Bool) :
    ib ∈ τ.fixed ↔ ∃ e ∈ τ, e.toPair = ib := by
  unfold fixed
  rw [List.mem_toFinset]
  constructor
  · intro h
    obtain ⟨e, he, hpair⟩ := List.mem_map.mp h
    exact ⟨e, he, hpair⟩
  · rintro ⟨e, he, hpair⟩
    exact List.mem_map.mpr ⟨e, he, hpair⟩

theorem toPattern_covers_iff_agrees {n : Nat} (τ : Trail n) (hτ : τ.Consistent)
    (v : Vertex n) :
    (τ.toPattern hτ).Covers v ↔ τ.Agrees v := by
  constructor
  · intro hcov e he
    exact hcov (e.var, e.val)
      ((Trail.mem_fixed_iff τ (e.var, e.val)).mpr ⟨e, he, rfl⟩)
  · intro hagree ib hib
    obtain ⟨e, he, hpair⟩ := (Trail.mem_fixed_iff τ ib).mp hib
    rw [← hpair]
    exact hagree e he

end Trail

/-- A hypercube proof-search state: original clean 3-SAT blockers, learned generalized
bad regions, frontier faces still under investigation, and the current trail. -/
structure SolverState (n : Nat) where
  original : List (Blocker n)
  learned : List (Pattern n)
  frontier : List (Pattern n)
  trail : Trail n

/-- Solver-state soundness: every learned region is semantically bad with respect to
the original instance. This is the invariant every operation must preserve. -/
def SolverState.Sound {n : Nat} (S : SolverState n) : Prop :=
  ∀ P ∈ S.learned, BadRegionFor S.original P

/-- The empty learned database is sound. -/
theorem SolverState.sound_no_learned {n : Nat} (Bs : List (Blocker n))
    (frontier : List (Pattern n)) (τ : Trail n) :
    SolverState.Sound
      ({ original := Bs, learned := [], frontier := frontier, trail := τ } : SolverState n) := by
  intro P hP
  simp at hP

/-- Initial abstract search state: no learned regions, the whole cube as the only frontier
face, and an empty trail. -/
def SolverState.initial {n : Nat} (Bs : List (Blocker n)) : SolverState n where
  original := Bs
  learned := []
  frontier := [Pattern.empty n]
  trail := []

theorem SolverState.initial_sound {n : Nat} (Bs : List (Blocker n)) :
    (SolverState.initial Bs).Sound :=
  SolverState.sound_no_learned Bs [Pattern.empty n] []

/-! #### Coordinate information exposure scores -/

/-- List-based blocker occurrence degree. Unlike the finset version, this keeps duplicate
blockers because duplicate constraints still cost scans in an operational solver model. -/
def BlockerList.variableDegree {n : Nat} (Bs : List (Blocker n)) (x : Fin n) : Nat :=
  (Bs.filter fun B => x ∈ B.support).length

theorem BlockerList.variableDegree_le_length {n : Nat} (Bs : List (Blocker n)) (x : Fin n) :
    BlockerList.variableDegree Bs x ≤ Bs.length := by
  unfold BlockerList.variableDegree
  exact List.length_filter_le _ _

/-- List-based frontier coordinate degree: how many active faces already mention coordinate `x`.
This is an activity signal for face-based proof search. -/
def PatternList.coordinateDegree {n : Nat} (Ps : List (Pattern n)) (x : Fin n) : Nat :=
  (Ps.filter fun P => x ∈ P.support).length

theorem PatternList.coordinateDegree_le_length {n : Nat} (Ps : List (Pattern n)) (x : Fin n) :
    PatternList.coordinateDegree Ps x ≤ Ps.length := by
  unfold PatternList.coordinateDegree
  exact List.length_filter_le _ _

/-- Original-instance occurrence activity for one coordinate. -/
def SolverState.originalVariableDegree {n : Nat} (S : SolverState n) (x : Fin n) : Nat :=
  BlockerList.variableDegree S.original x

/-- Active-frontier occurrence activity for one coordinate. -/
def SolverState.frontierCoordinateDegree {n : Nat} (S : SolverState n) (x : Fin n) : Nat :=
  PatternList.coordinateDegree S.frontier x

/-- A coordinate is active in the information sense when it appears both in original blockers
and in the current frontier representation. -/
def SolverState.InformationActiveCoordinate {n : Nat} (S : SolverState n) (x : Fin n) : Prop :=
  0 < S.originalVariableDegree x ∧ 0 < S.frontierCoordinateDegree x

/-- Product exposure score: a cheap VSIDS-like signal measuring how much original blocker
activity can meet current frontier activity at coordinate `x`. -/
def SolverState.coordinateExposure {n : Nat} (S : SolverState n) (x : Fin n) : Nat :=
  S.originalVariableDegree x * S.frontierCoordinateDegree x

theorem SolverState.originalVariableDegree_le_length {n : Nat} (S : SolverState n) (x : Fin n) :
    S.originalVariableDegree x ≤ S.original.length :=
  BlockerList.variableDegree_le_length S.original x

theorem SolverState.frontierCoordinateDegree_le_length {n : Nat} (S : SolverState n) (x : Fin n) :
    S.frontierCoordinateDegree x ≤ S.frontier.length :=
  PatternList.coordinateDegree_le_length S.frontier x

theorem SolverState.coordinateExposure_eq_zero_of_originalDegree_eq_zero {n : Nat}
    (S : SolverState n) (x : Fin n) (hzero : S.originalVariableDegree x = 0) :
    S.coordinateExposure x = 0 := by
  simp [SolverState.coordinateExposure, hzero]

theorem SolverState.coordinateExposure_eq_zero_of_frontierDegree_eq_zero {n : Nat}
    (S : SolverState n) (x : Fin n) (hzero : S.frontierCoordinateDegree x = 0) :
    S.coordinateExposure x = 0 := by
  simp [SolverState.coordinateExposure, hzero]

theorem SolverState.coordinateExposure_pos_iff_active {n : Nat}
    (S : SolverState n) (x : Fin n) :
    0 < S.coordinateExposure x ↔ S.InformationActiveCoordinate x := by
  unfold SolverState.coordinateExposure SolverState.InformationActiveCoordinate
  constructor
  · intro hpos
    constructor
    · by_contra hnot
      have hz : S.originalVariableDegree x = 0 := by omega
      simp [hz] at hpos
    · by_contra hnot
      have hz : S.frontierCoordinateDegree x = 0 := by omega
      simp [hz] at hpos
  · intro h
    exact Nat.mul_pos h.1 h.2

theorem SolverState.coordinateExposure_le_length_product {n : Nat}
    (S : SolverState n) (x : Fin n) :
    S.coordinateExposure x ≤ S.original.length * S.frontier.length := by
  unfold SolverState.coordinateExposure
  exact Nat.mul_le_mul
    (S.originalVariableDegree_le_length x)
    (S.frontierCoordinateDegree_le_length x)

/-- Exposure-optimal coordinate: among a candidate set, maximize blocker/frontier interaction.
This is a VSIDS-like information heuristic stated as a proof obligation rather than an
implementation trick. -/
def SolverState.ExposureOptimalCoordinate {n : Nat}
    (S : SolverState n) (Candidates : Finset (Fin n)) (x : Fin n) : Prop :=
  x ∈ Candidates ∧ ∀ y ∈ Candidates, S.coordinateExposure y ≤ S.coordinateExposure x

theorem SolverState.ExposureOptimalCoordinate.best {n : Nat}
    {S : SolverState n} {Candidates : Finset (Fin n)} {x y : Fin n}
    (hopt : S.ExposureOptimalCoordinate Candidates x) (hy : y ∈ Candidates) :
    S.coordinateExposure y ≤ S.coordinateExposure x :=
  hopt.2 y hy

theorem SolverState.ExposureOptimalCoordinate.active_of_positive_candidate {n : Nat}
    {S : SolverState n} {Candidates : Finset (Fin n)} {x y : Fin n}
    (hopt : S.ExposureOptimalCoordinate Candidates x)
    (hy : y ∈ Candidates) (hpos : 0 < S.coordinateExposure y) :
    S.InformationActiveCoordinate x := by
  have hxpos : 0 < S.coordinateExposure x := by
    exact lt_of_lt_of_le hpos (hopt.best hy)
  exact (S.coordinateExposure_pos_iff_active x).mp hxpos

/-- Adding a certified bad region to the learned database preserves solver soundness. -/
def SolverState.addLearned {n : Nat} (S : SolverState n) (P : Pattern n) : SolverState n :=
  { S with learned := P :: S.learned }

theorem SolverState.addLearned_sound {n : Nat} {S : SolverState n} {P : Pattern n}
    (hS : S.Sound) (hP : BadRegionFor S.original P) :
    (S.addLearned P).Sound := by
  intro Q hQ
  have hmem : Q ∈ P :: S.learned := by
    simpa [SolverState.addLearned] using hQ
  rw [List.mem_cons] at hmem
  rcases hmem with hQP | hQold
  · simpa [hQP] using hP
  · exact hS Q hQold

/-- Restart clears the trail but keeps every learned bad region. -/
def SolverState.restart {n : Nat} (S : SolverState n) : SolverState n :=
  { S with trail := [] }

theorem SolverState.restart_sound {n : Nat} {S : SolverState n}
    (hS : S.Sound) :
    S.restart.Sound := by
  intro P hP
  exact hS P hP

/-- Sharing learned regions between workers is sound when every shared region is bad
for the common original blocker list. -/
def SolverState.addLearnedMany {n : Nat} (S : SolverState n)
    (Ps : List (Pattern n)) : SolverState n :=
  { S with learned := Ps ++ S.learned }

theorem SolverState.addLearnedMany_sound {n : Nat} {S : SolverState n}
    {Ps : List (Pattern n)} (hS : S.Sound)
    (hPs : ∀ P ∈ Ps, BadRegionFor S.original P) :
    (S.addLearnedMany Ps).Sound := by
  intro P hP
  have hmem : P ∈ Ps ++ S.learned := by
    simpa [SolverState.addLearnedMany] using hP
  rw [List.mem_append] at hmem
  rcases hmem with hnew | hold
  · exact hPs P hnew
  · exact hS P hold

/-- Push one face onto the frontier. This changes search order only, so it preserves
learned-region soundness. -/
def SolverState.pushFrontier {n : Nat} (S : SolverState n) (P : Pattern n) :
    SolverState n :=
  { S with frontier := P :: S.frontier }

theorem SolverState.pushFrontier_sound {n : Nat} {S : SolverState n} {P : Pattern n}
    (hS : S.Sound) :
    (S.pushFrontier P).Sound := by
  intro Q hQ
  exact hS Q hQ

/-- Push a list of faces onto the frontier. -/
def SolverState.pushFrontierMany {n : Nat} (S : SolverState n) (Ps : List (Pattern n)) :
    SolverState n :=
  { S with frontier := Ps ++ S.frontier }

theorem SolverState.pushFrontierMany_sound {n : Nat} {S : SolverState n}
    {Ps : List (Pattern n)} (hS : S.Sound) :
    (S.pushFrontierMany Ps).Sound := by
  intro Q hQ
  exact hS Q hQ

/-- Branch a free coordinate of a face by adding its two Boolean children to the frontier. -/
def SolverState.branchFace {n : Nat} (S : SolverState n) (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) : SolverState n :=
  S.pushFrontierMany [P.assign i false hfree, P.assign i true hfree]

theorem SolverState.branchFace_sound {n : Nat} {S : SolverState n} {P : Pattern n}
    {i : Fin n} {hfree : P.Free i} (hS : S.Sound) :
    (S.branchFace P i hfree).Sound :=
  SolverState.pushFrontierMany_sound hS

/-- Candidate coordinates for branching in a face. We keep it explicit because different
solver classes may restrict candidates by activity, trail level, component, or watched data. -/
abbrev BranchCandidates (n : Nat) := Pattern n → Finset (Fin n)

/-- The unrestricted branch class: every coordinate is a candidate. -/
def allCoordinateCandidates {n : Nat} : BranchCandidates n :=
  fun _ => Finset.univ

@[simp] theorem mem_allCoordinateCandidates {n : Nat} (P : Pattern n) (i : Fin n) :
    i ∈ (allCoordinateCandidates (n := n)) P := by
  simp [allCoordinateCandidates]

/-- The geometric branch class: only coordinates still free in the current face are candidates. -/
noncomputable def freeCoordinateCandidates {n : Nat} : BranchCandidates n := by
  classical
  exact fun P => Finset.univ.filter fun i => P.Free i

@[simp] theorem mem_freeCoordinateCandidates {n : Nat} (P : Pattern n) (i : Fin n) :
    i ∈ (freeCoordinateCandidates (n := n)) P ↔ P.Free i := by
  classical
  simp [freeCoordinateCandidates]

/-- Candidate coordinates restricted to variables that occur in the original clean blockers. -/
def blockerSupportCandidates {n : Nat} (Bs : Finset (Blocker n)) : BranchCandidates n :=
  fun _ => BlockerFamily.support Bs

@[simp] theorem mem_blockerSupportCandidates {n : Nat} (Bs : Finset (Blocker n))
    (P : Pattern n) (i : Fin n) :
    i ∈ blockerSupportCandidates Bs P ↔ ∃ B ∈ Bs, i ∈ B.support := by
  simp [blockerSupportCandidates, BlockerFamily.mem_support_iff]

/-- Active geometric candidates: variables used by some blocker and still free in the face. -/
noncomputable def activeFreeSupportCandidates {n : Nat} (Bs : Finset (Blocker n)) :
    BranchCandidates n := by
  classical
  exact fun P => (BlockerFamily.support Bs).filter fun i => P.Free i

@[simp] theorem mem_activeFreeSupportCandidates {n : Nat} (Bs : Finset (Blocker n))
    (P : Pattern n) (i : Fin n) :
    i ∈ activeFreeSupportCandidates Bs P ↔
      (∃ B ∈ Bs, i ∈ B.support) ∧ P.Free i := by
  classical
  simp [activeFreeSupportCandidates, BlockerFamily.mem_support_iff]

theorem activeFreeSupportCandidates_subset_support {n : Nat} (Bs : Finset (Blocker n))
    (P : Pattern n) :
    activeFreeSupportCandidates Bs P ⊆ blockerSupportCandidates Bs P := by
  intro i hi
  exact (mem_blockerSupportCandidates Bs P i).mpr
    ((mem_activeFreeSupportCandidates Bs P i).mp hi).1

theorem activeFreeSupportCandidates_subset_free {n : Nat} (Bs : Finset (Blocker n))
    (P : Pattern n) :
    activeFreeSupportCandidates Bs P ⊆ freeCoordinateCandidates P := by
  intro i hi
  exact (mem_freeCoordinateCandidates P i).mpr
    ((mem_activeFreeSupportCandidates Bs P i).mp hi).2

/-- Candidate coordinates supplied by the original blocker list in a solver state. -/
def SolverState.originalSupportCandidates {n : Nat} (S : SolverState n) :
    BranchCandidates n :=
  blockerSupportCandidates S.original.toFinset

/-- Free active candidates supplied by the original blocker list in a solver state. -/
noncomputable def SolverState.activeOriginalFreeCandidates {n : Nat} (S : SolverState n) :
    BranchCandidates n :=
  activeFreeSupportCandidates S.original.toFinset

/-- A branch choice is information-optimal for a candidate generator when it minimizes
worst-child face mass among all free candidate coordinates. -/
def InfoOptimalBranchChoice {n : Nat} (C : BranchCandidates n)
    (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Prop :=
  P.InfoOptimalSplit (C P) i hfree

/-- Blocker-aware branch choice: optimal for shrinking the current original instance's
remaining uncovered-candidate set inside the face. -/
def InfoOptimalSolutionBranchChoice {n : Nat} (Bs : List (Blocker n))
    (C : BranchCandidates n) (P : Pattern n) (i : Fin n) (hfree : P.Free i) : Prop :=
  P.InfoOptimalSolutionSplit Bs (C P) i hfree

theorem InfoOptimalBranchChoice.best {n : Nat} {C : BranchCandidates n}
    {P : Pattern n} {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice C P i hfreei)
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstMass i hfreei ≤ P.splitWorstMass j hfreej :=
  Pattern.InfoOptimalSplit.best hopt hj hfreej

theorem InfoOptimalBranchChoice.best_uncertainty {n : Nat} {C : BranchCandidates n}
    {P : Pattern n} {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice C P i hfreei)
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstUncertainty i hfreei ≤ P.splitWorstUncertainty j hfreej :=
  Pattern.InfoOptimalSplit.best_uncertainty hopt hj hfreej

theorem InfoOptimalBranchChoice.best_of_subset {n : Nat} {C D : BranchCandidates n}
    {P : Pattern n} {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice C P i hfreei)
    (hsub : D P ⊆ C P) (hj : j ∈ D P) (hfreej : P.Free j) :
    P.splitWorstMass i hfreei ≤ P.splitWorstMass j hfreej :=
  hopt.best (hsub hj) hfreej

theorem InfoOptimalBranchChoice.best_uncertainty_of_subset {n : Nat}
    {C D : BranchCandidates n} {P : Pattern n} {i j : Fin n}
    {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice C P i hfreei)
    (hsub : D P ⊆ C P) (hj : j ∈ D P) (hfreej : P.Free j) :
    P.splitWorstUncertainty i hfreei ≤ P.splitWorstUncertainty j hfreej :=
  hopt.best_uncertainty (hsub hj) hfreej

theorem InfoOptimalBranchChoice.best_among_all_free {n : Nat} {P : Pattern n}
    {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice (allCoordinateCandidates (n := n)) P i hfreei)
    (hfreej : P.Free j) :
    P.splitWorstMass i hfreei ≤ P.splitWorstMass j hfreej :=
  hopt.best (mem_allCoordinateCandidates P j) hfreej

theorem InfoOptimalBranchChoice.best_uncertainty_among_all_free {n : Nat}
    {P : Pattern n} {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice (allCoordinateCandidates (n := n)) P i hfreei)
    (hfreej : P.Free j) :
    P.splitWorstUncertainty i hfreei ≤ P.splitWorstUncertainty j hfreej :=
  hopt.best_uncertainty (mem_allCoordinateCandidates P j) hfreej

theorem InfoOptimalBranchChoice.best_among_free_candidates {n : Nat} {P : Pattern n}
    {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice (freeCoordinateCandidates (n := n)) P i hfreei)
    (hfreej : P.Free j) :
    P.splitWorstMass i hfreei ≤ P.splitWorstMass j hfreej :=
  hopt.best (by simpa using hfreej) hfreej

theorem InfoOptimalBranchChoice.best_uncertainty_among_free_candidates {n : Nat}
    {P : Pattern n} {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice (freeCoordinateCandidates (n := n)) P i hfreei)
    (hfreej : P.Free j) :
    P.splitWorstUncertainty i hfreei ≤ P.splitWorstUncertainty j hfreej :=
  hopt.best_uncertainty (by simpa using hfreej) hfreej

theorem InfoOptimalBranchChoice.best_among_active_support {n : Nat}
    {Bs : Finset (Blocker n)} {P : Pattern n} {i j : Fin n}
    {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice (activeFreeSupportCandidates Bs) P i hfreei)
    (hused : ∃ B ∈ Bs, j ∈ B.support) (hfreej : P.Free j) :
    P.splitWorstMass i hfreei ≤ P.splitWorstMass j hfreej :=
  hopt.best (by simpa using And.intro hused hfreej) hfreej

theorem InfoOptimalBranchChoice.best_uncertainty_among_active_support {n : Nat}
    {Bs : Finset (Blocker n)} {P : Pattern n} {i j : Fin n}
    {hfreei : P.Free i}
    (hopt : InfoOptimalBranchChoice (activeFreeSupportCandidates Bs) P i hfreei)
    (hused : ∃ B ∈ Bs, j ∈ B.support) (hfreej : P.Free j) :
    P.splitWorstUncertainty i hfreei ≤ P.splitWorstUncertainty j hfreej :=
  hopt.best_uncertainty (by simpa using And.intro hused hfreej) hfreej

theorem InfoOptimalSolutionBranchChoice.best {n : Nat} {Bs : List (Blocker n)}
    {C : BranchCandidates n} {P : Pattern n} {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalSolutionBranchChoice Bs C P i hfreei)
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstSolutionMass Bs i hfreei ≤ P.splitWorstSolutionMass Bs j hfreej :=
  Pattern.InfoOptimalSolutionSplit.best hopt hj hfreej

theorem InfoOptimalSolutionBranchChoice.best_uncertainty {n : Nat} {Bs : List (Blocker n)}
    {C : BranchCandidates n} {P : Pattern n} {i j : Fin n} {hfreei : P.Free i}
    (hopt : InfoOptimalSolutionBranchChoice Bs C P i hfreei)
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstSolutionUncertainty Bs i hfreei ≤
      P.splitWorstSolutionUncertainty Bs j hfreej :=
  Pattern.InfoOptimalSolutionSplit.best_uncertainty hopt hj hfreej

/-- Exact head-branching: replace the selected frontier head by its two Boolean children. -/
def SolverState.branchFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (hfree : P.Free i) :
    SolverState n :=
  { S with frontier := P.assign i false hfree :: P.assign i true hfree :: Rest }

/-- Number of active faces still under search. -/
def SolverState.frontierSize {n : Nat} (S : SolverState n) : Nat :=
  S.frontier.length

/-- Number of learned bad regions retained by the solver. -/
def SolverState.learnedSize {n : Nat} (S : SolverState n) : Nat :=
  S.learned.length

/-- Number of entries in the current trail. -/
def SolverState.trailSize {n : Nat} (S : SolverState n) : Nat :=
  S.trail.length

/-- Total geometric mass carried by the active frontier, counted with multiplicity if faces
overlap. This is the UNSAT/proof-search entropy ledger: how much cube volume remains active. -/
def SolverState.frontierMass {n : Nat} (S : SolverState n) : Nat :=
  (S.frontier.map fun P => P.mass).sum

/-- Base-2 uncertainty of the active geometric frontier volume. -/
def SolverState.frontierUncertainty {n : Nat} (S : SolverState n) : Nat :=
  uncertaintyBits S.frontierMass

/-- Total blocker-aware candidate-solution mass carried by the active frontier.
This is a state-level information ledger: how many possible SAT witnesses are still represented
by the frontier, counted with multiplicity if frontier faces overlap. -/
def SolverState.frontierSolutionMass {n : Nat} (S : SolverState n) : Nat :=
  (S.frontier.map fun P => P.solutionMass S.original).sum

/-- Base-2 uncertainty of the frontier's remaining candidate-solution mass. -/
def SolverState.frontierSolutionUncertainty {n : Nat} (S : SolverState n) : Nat :=
  uncertaintyBits S.frontierSolutionMass

@[simp] theorem SolverState.frontierSolutionMass_nil {n : Nat}
    (original : List (Blocker n)) (learned _frontier : List (Pattern n)) (trail : Trail n) :
    SolverState.frontierSolutionMass
      ({ original := original, learned := learned, frontier := [], trail := trail } :
        SolverState n) = 0 := by
  rfl

@[simp] theorem SolverState.frontierMass_nil {n : Nat}
    (original : List (Blocker n)) (learned _frontier : List (Pattern n)) (trail : Trail n) :
    SolverState.frontierMass
      ({ original := original, learned := learned, frontier := [], trail := trail } :
        SolverState n) = 0 := by
  rfl

/-- Candidate-solution mass is always bounded by the active geometric frontier volume. -/
theorem SolverState.frontierSolutionMass_le_frontierMass {n : Nat} (S : SolverState n) :
    S.frontierSolutionMass ≤ S.frontierMass := by
  unfold SolverState.frontierSolutionMass SolverState.frontierMass
  induction S.frontier with
  | nil =>
      simp
  | cons P Ps ih =>
      simp only [List.map_cons, List.sum_cons]
      exact Nat.add_le_add (Pattern.solutionMass_le_mass S.original P) ih

/-- Candidate-solution uncertainty never exceeds geometric frontier uncertainty. -/
theorem SolverState.frontierSolutionUncertainty_le_frontierUncertainty {n : Nat}
    (S : SolverState n) :
    S.frontierSolutionUncertainty ≤ S.frontierUncertainty := by
  exact uncertaintyBits_mono S.frontierSolutionMass_le_frontierMass

/-- If the active geometric frontier is empty, then it contains no candidate witnesses. -/
theorem SolverState.frontierSolutionMass_eq_zero_of_frontierMass_eq_zero {n : Nat}
    {S : SolverState n} (hzero : S.frontierMass = 0) :
    S.frontierSolutionMass = 0 := by
  have hle := S.frontierSolutionMass_le_frontierMass
  omega

/-- Positive candidate mass forces positive geometric frontier mass. -/
theorem SolverState.frontierMass_pos_of_frontierSolutionMass_pos {n : Nat}
    {S : SolverState n} (hpos : 0 < S.frontierSolutionMass) :
    0 < S.frontierMass := by
  have hle := S.frontierSolutionMass_le_frontierMass
  omega

/-- Initial frontier mass is exactly the number of satisfying vertices of the instance. -/
theorem SolverState.initial_frontierSolutionMass {n : Nat} (Bs : List (Blocker n)) :
    (SolverState.initial Bs).frontierSolutionMass = solutionCount Bs := by
  unfold SolverState.frontierSolutionMass SolverState.initial Pattern.solutionMass
    Pattern.solutionVertices Pattern.vertices solutionCount uncoveredVertices allVertices
  simp [Pattern.empty_covers]

/-- Initial geometric frontier mass is the whole cube, `2^n`. -/
theorem SolverState.initial_frontierMass {n : Nat} (Bs : List (Blocker n)) :
    (SolverState.initial Bs).frontierMass = 2 ^ n := by
  unfold SolverState.frontierMass SolverState.initial Pattern.mass Pattern.vertices
  simp [Pattern.empty_covers, allVertices_card]

/-- Record one trail entry without changing the frontier or learned database. -/
def SolverState.pushTrail {n : Nat} (S : SolverState n) (e : TrailEntry n) : SolverState n :=
  { S with trail := e :: S.trail }

@[simp] theorem SolverState.trailSize_pushTrail {n : Nat} (S : SolverState n)
    (e : TrailEntry n) :
    (S.pushTrail e).trailSize = S.trailSize + 1 := by
  simp [SolverState.trailSize, SolverState.pushTrail]

@[simp] theorem SolverState.frontierSize_pushTrail {n : Nat} (S : SolverState n)
    (e : TrailEntry n) :
    (S.pushTrail e).frontierSize = S.frontierSize := by
  simp [SolverState.frontierSize, SolverState.pushTrail]

@[simp] theorem SolverState.learnedSize_pushTrail {n : Nat} (S : SolverState n)
    (e : TrailEntry n) :
    (S.pushTrail e).learnedSize = S.learnedSize := by
  simp [SolverState.learnedSize, SolverState.pushTrail]

@[simp] theorem SolverState.frontierSize_branchFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (hfree : P.Free i) :
    SolverState.frontierSize (S.branchFrontierHead P Rest i hfree) = Rest.length + 2 := by
  rfl

@[simp] theorem SolverState.learnedSize_branchFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (hfree : P.Free i) :
    SolverState.learnedSize (S.branchFrontierHead P Rest i hfree) = SolverState.learnedSize S := by
  simp [SolverState.learnedSize, SolverState.branchFrontierHead]

/-- Branching a frontier head preserves total candidate-solution mass: it only partitions the
uncertainty into two children. -/
theorem SolverState.frontierSolutionMass_branchFrontierHead {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)} {i : Fin n} {hfree : P.Free i}
    (hfrontier : S.frontier = P :: Rest) :
    (S.branchFrontierHead P Rest i hfree).frontierSolutionMass =
      S.frontierSolutionMass := by
  unfold SolverState.frontierSolutionMass SolverState.branchFrontierHead
  rw [hfrontier]
  simp [Pattern.childSolutionMass,
    P.solutionMass_eq_childSolutionMass_add_childSolutionMass S.original i hfree,
    Nat.add_assoc]

/-- Branching a frontier head preserves total geometric mass: it partitions cube volume. -/
theorem SolverState.frontierMass_branchFrontierHead {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)} {i : Fin n} {hfree : P.Free i}
    (hfrontier : S.frontier = P :: Rest) :
    (S.branchFrontierHead P Rest i hfree).frontierMass = S.frontierMass := by
  unfold SolverState.frontierMass SolverState.branchFrontierHead
  rw [hfrontier]
  simp [Pattern.childMass, P.mass_eq_childMass_add_childMass i hfree, Nat.add_assoc]

/-- Branching preserves total frontier uncertainty when uncertainty is measured as total
candidate-solution mass: the branch redistributes possibilities rather than removing them. -/
theorem SolverState.frontierSolutionUncertainty_branchFrontierHead {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest) :
    (S.branchFrontierHead P Rest i hfree).frontierSolutionUncertainty =
      S.frontierSolutionUncertainty := by
  unfold SolverState.frontierSolutionUncertainty
  rw [SolverState.frontierSolutionMass_branchFrontierHead hfrontier]

/-- Branching preserves geometric frontier uncertainty: proof search has divided the region,
not removed cube volume. -/
theorem SolverState.frontierUncertainty_branchFrontierHead {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest) :
    (S.branchFrontierHead P Rest i hfree).frontierUncertainty = S.frontierUncertainty := by
  unfold SolverState.frontierUncertainty
  rw [SolverState.frontierMass_branchFrontierHead hfrontier]

/-- Candidate-solution mass removed by a branch move. Branching is a partition, so this will be
zero when the supplied `P :: Rest` really is the current frontier. -/
def SolverState.branchFrontierHeadSolutionLoss {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (hfree : P.Free i) : Nat :=
  S.frontierSolutionMass - (S.branchFrontierHead P Rest i hfree).frontierSolutionMass

/-- Exact branching loses no candidate-solution mass. -/
theorem SolverState.branchFrontierHeadSolutionLoss_eq_zero {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest) :
    S.branchFrontierHeadSolutionLoss P Rest i hfree = 0 := by
  unfold SolverState.branchFrontierHeadSolutionLoss
  rw [SolverState.frontierSolutionMass_branchFrontierHead hfrontier]
  exact Nat.sub_self _

/-- Geometric mass removed by a branch move. Branching partitions cube volume, so this is zero
for an exact frontier-head branch. -/
def SolverState.branchFrontierHeadMassLoss {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (hfree : P.Free i) : Nat :=
  S.frontierMass - (S.branchFrontierHead P Rest i hfree).frontierMass

/-- Exact branching loses no geometric mass. -/
theorem SolverState.branchFrontierHeadMassLoss_eq_zero {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest) :
    S.branchFrontierHeadMassLoss P Rest i hfree = 0 := by
  unfold SolverState.branchFrontierHeadMassLoss
  rw [SolverState.frontierMass_branchFrontierHead hfrontier]
  exact Nat.sub_self _

theorem list_sum_pos_of_mem_pos {xs : List Nat} {x : Nat}
    (hx : x ∈ xs) (hpos : 0 < x) : 0 < xs.sum := by
  induction xs with
  | nil =>
      simp at hx
  | cons y ys ih =>
      rw [List.mem_cons] at hx
      rcases hx with hxy | hxys
      · subst y
        simp
        omega
      · simp
        have hys : 0 < ys.sum := ih hxys
        omega

theorem list_exists_mem_pos_of_sum_pos {xs : List Nat}
    (hpos : 0 < xs.sum) : ∃ x ∈ xs, 0 < x := by
  induction xs with
  | nil =>
      simp at hpos
  | cons y ys ih =>
      by_cases hy : 0 < y
      · exact ⟨y, by simp, hy⟩
      · have hys : 0 < ys.sum := by
          simp at hpos
          omega
        obtain ⟨x, hx, hxpos⟩ := ih hys
        exact ⟨x, by simp [hx], hxpos⟩

theorem SolverState.frontierSolutionMass_pos_of_frontier_solution {n : Nat}
    {S : SolverState n} {P : Pattern n} {v : Vertex n}
    (hP : P ∈ S.frontier) (hPv : P.Covers v) (hun : IsUncovered S.original v) :
    0 < S.frontierSolutionMass := by
  unfold SolverState.frontierSolutionMass
  apply list_sum_pos_of_mem_pos
  · exact List.mem_map.mpr ⟨P, hP, rfl⟩
  · apply (Pattern.solutionMass_pos_iff_exists_solution S.original P).mpr
    exact ⟨v, hPv, hun⟩

theorem SolverState.exists_uncovered_of_frontierSolutionMass_pos {n : Nat}
    {S : SolverState n} (hpos : 0 < S.frontierSolutionMass) :
    ∃ v : Vertex n, IsUncovered S.original v := by
  unfold SolverState.frontierSolutionMass at hpos
  obtain ⟨m, hm, hmpos⟩ := list_exists_mem_pos_of_sum_pos hpos
  obtain ⟨P, hP, hmP⟩ := List.mem_map.mp hm
  subst m
  obtain ⟨v, _hPv, hun⟩ :=
    Pattern.exists_solution_of_solutionMass_pos (Bs := S.original) (P := P) hmpos
  exact ⟨v, hun⟩

theorem SolverState.branchFrontierHead_sound {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)} {i : Fin n} {hfree : P.Free i}
    (hS : S.Sound) :
    (S.branchFrontierHead P Rest i hfree).Sound := by
  intro Q hQ
  exact hS Q hQ

/-- A frontier covers a target face when every vertex in the target lies in some
frontier face. -/
def SolverState.FrontierCovers {n : Nat} (S : SolverState n) (Target : Pattern n) : Prop :=
  ∀ v : Vertex n, Target.Covers v → ∃ Q ∈ S.frontier, Q.Covers v

/-- The current frontier covers the whole cube. -/
def SolverState.FrontierCoversWhole {n : Nat} (S : SolverState n) : Prop :=
  S.FrontierCovers (Pattern.empty n)

/-- Search-completeness invariant: every genuinely uncovered vertex is still contained
in at least one frontier face. This is weaker than covering the whole cube and is the
right invariant after pruning faces known to be bad. -/
def SolverState.FrontierCoversUncovered {n : Nat} (S : SolverState n) : Prop :=
  ∀ v : Vertex n, IsUncovered S.original v → ∃ P ∈ S.frontier, P.Covers v

/-- The main abstract search invariant: learned regions are sound, and no possible SAT
witness has been lost from the frontier. -/
def SolverState.SearchInvariant {n : Nat} (S : SolverState n) : Prop :=
  S.Sound ∧ S.FrontierCoversUncovered

/-- Positive frontier candidate mass is a SAT witness certificate in disguise. -/
theorem SolverState.coverSATList_of_frontierSolutionMass_pos {n : Nat}
    {S : SolverState n} (hpos : 0 < S.frontierSolutionMass) :
    CoverSATList S.original := by
  obtain ⟨v, hun⟩ := S.exists_uncovered_of_frontierSolutionMass_pos hpos
  exact ⟨v, hun⟩

/-- Under the search invariant, zero frontier candidate mass proves UNSAT. This is the
information-theoretic stopping condition for a certified search state. -/
theorem SolverState.coverUNSATList_of_searchInvariant_frontierSolutionMass_zero {n : Nat}
    {S : SolverState n} (h : S.SearchInvariant) (hzero : S.frontierSolutionMass = 0) :
    CoverUNSATList S.original := by
  intro hsat
  obtain ⟨v, hun⟩ := hsat
  obtain ⟨P, hP, hPv⟩ := h.2 v hun
  have hpos : 0 < S.frontierSolutionMass :=
    S.frontierSolutionMass_pos_of_frontier_solution hP hPv hun
  omega

theorem SolverState.pushTrail_preserves_searchInvariant {n : Nat} {S : SolverState n}
    {e : TrailEntry n} (h : S.SearchInvariant) :
    (S.pushTrail e).SearchInvariant :=
  ⟨by
      intro P hP
      exact h.1 P hP,
    by
      intro v hv
      exact h.2 v hv⟩

theorem SolverState.initial_frontierCoversWhole {n : Nat} (Bs : List (Blocker n)) :
    (SolverState.initial Bs).FrontierCoversWhole := by
  intro v hv
  exact ⟨Pattern.empty n, by simp [SolverState.initial], Pattern.empty_covers v⟩

theorem SolverState.initial_frontierCoversUncovered {n : Nat} (Bs : List (Blocker n)) :
    (SolverState.initial Bs).FrontierCoversUncovered := by
  intro v hv
  exact ⟨Pattern.empty n, by simp [SolverState.initial], Pattern.empty_covers v⟩

theorem SolverState.initial_searchInvariant {n : Nat} (Bs : List (Blocker n)) :
    (SolverState.initial Bs).SearchInvariant :=
  ⟨SolverState.initial_sound Bs, SolverState.initial_frontierCoversUncovered Bs⟩

/-- Whole-frontier coverage implies uncovered-vertex frontier coverage. -/
theorem SolverState.frontierCoversUncovered_of_whole {n : Nat} {S : SolverState n}
    (h : S.FrontierCoversWhole) :
    S.FrontierCoversUncovered := by
  intro v hv
  exact h v (Pattern.empty_covers v)

/-- Replacing the frontier head by its two Boolean children preserves any semantic
coverage invariant held by the old frontier. -/
theorem SolverState.branchFrontierHead_preserves_frontierCovers {n : Nat}
    {S : SolverState n} {Target P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest)
    (hcov : S.FrontierCovers Target) :
    (S.branchFrontierHead P Rest i hfree).FrontierCovers Target := by
  unfold SolverState.FrontierCovers at hcov ⊢
  intro v hTv
  obtain ⟨Q, hQ, hQv⟩ := hcov v hTv
  rw [hfrontier] at hQ
  rw [List.mem_cons] at hQ
  rcases hQ with hQP | hQrest
  · subst Q
    rcases P.split_by_free i hfree v hQv with hfalse | htrue
    · exact ⟨P.assign i false hfree, by simp [SolverState.branchFrontierHead], hfalse⟩
    · exact ⟨P.assign i true hfree, by simp [SolverState.branchFrontierHead], htrue⟩
  · exact ⟨Q, by simp [SolverState.branchFrontierHead, hQrest], hQv⟩

/-- Exact head-branching preserves the invariant that all uncovered vertices remain in
the frontier. -/
theorem SolverState.branchFrontierHead_preserves_uncovered {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest)
    (hcov : S.FrontierCoversUncovered) :
    (S.branchFrontierHead P Rest i hfree).FrontierCoversUncovered := by
  intro v hun
  obtain ⟨Q, hQ, hQv⟩ := hcov v hun
  rw [hfrontier] at hQ
  rw [List.mem_cons] at hQ
  rcases hQ with hQP | hQrest
  · subst Q
    rcases P.split_by_free i hfree v hQv with hfalse | htrue
    · exact ⟨P.assign i false hfree, by simp [SolverState.branchFrontierHead], hfalse⟩
    · exact ⟨P.assign i true hfree, by simp [SolverState.branchFrontierHead], htrue⟩
  · exact ⟨Q, by simp [SolverState.branchFrontierHead, hQrest], hQv⟩

theorem SolverState.branchFrontierHead_preserves_searchInvariant {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest)
    (h : S.SearchInvariant) :
    (S.branchFrontierHead P Rest i hfree).SearchInvariant :=
  ⟨SolverState.branchFrontierHead_sound h.1,
    SolverState.branchFrontierHead_preserves_uncovered hfrontier h.2⟩

theorem SolverState.frontierSize_branchFrontierHead_eq_succ {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {hfree : P.Free i} (hfrontier : S.frontier = P :: Rest) :
    SolverState.frontierSize (S.branchFrontierHead P Rest i hfree) =
      SolverState.frontierSize S + 1 := by
  rw [SolverState.frontierSize_branchFrontierHead]
  simp [SolverState.frontierSize, hfrontier]

/-- Drop the frontier head. This is search-complete only when the dropped face is bad. -/
def SolverState.dropFrontierHead {n : Nat} (S : SolverState n)
    (_P : Pattern n) (Rest : List (Pattern n)) : SolverState n :=
  { S with frontier := Rest }

@[simp] theorem SolverState.frontierSize_dropFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) :
    SolverState.frontierSize (S.dropFrontierHead P Rest) = Rest.length := by
  simp [SolverState.frontierSize, SolverState.dropFrontierHead]

@[simp] theorem SolverState.learnedSize_dropFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) :
    SolverState.learnedSize (S.dropFrontierHead P Rest) = SolverState.learnedSize S := by
  simp [SolverState.learnedSize, SolverState.dropFrontierHead]

theorem SolverState.dropBadFrontierHead_sound {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)} (hS : S.Sound) :
    (S.dropFrontierHead P Rest).Sound := by
  intro Q hQ
  exact hS Q hQ

/-- Dropping a bad frontier head preserves every still-possible uncovered vertex. -/
theorem SolverState.dropBadFrontierHead_preserves_uncovered {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P)
    (hcov : S.FrontierCoversUncovered) :
    (S.dropFrontierHead P Rest).FrontierCoversUncovered := by
  intro v hun
  obtain ⟨Q, hQ, hQv⟩ := hcov v hun
  rw [hfrontier] at hQ
  rw [List.mem_cons] at hQ
  rcases hQ with hQP | hQrest
  · subst Q
    exact False.elim (hbad v hQv hun)
  · exact ⟨Q, by simp [SolverState.dropFrontierHead, hQrest], hQv⟩

theorem SolverState.dropBadFrontierHead_preserves_searchInvariant {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P)
    (h : S.SearchInvariant) :
    (S.dropFrontierHead P Rest).SearchInvariant :=
  ⟨SolverState.dropBadFrontierHead_sound h.1,
    SolverState.dropBadFrontierHead_preserves_uncovered hfrontier hbad h.2⟩

theorem SolverState.frontierSize_dropFrontierHead_add_one {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    SolverState.frontierSize (S.dropFrontierHead P Rest) + 1 =
      SolverState.frontierSize S := by
  rw [SolverState.frontierSize_dropFrontierHead]
  simp [SolverState.frontierSize, hfrontier]

/-- Dropping a frontier head removes exactly that face's geometric mass from the frontier
accountant. -/
theorem SolverState.frontierMass_dropFrontierHead_add {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    (S.dropFrontierHead P Rest).frontierMass + P.mass = S.frontierMass := by
  unfold SolverState.frontierMass SolverState.dropFrontierHead
  rw [hfrontier]
  simp [Nat.add_comm, Nat.add_left_comm, Nat.add_assoc]

/-- Dropping any frontier head cannot increase represented geometric mass. -/
theorem SolverState.frontierMass_dropFrontierHead_le {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    (S.dropFrontierHead P Rest).frontierMass ≤ S.frontierMass := by
  have h := SolverState.frontierMass_dropFrontierHead_add (S := S) (P := P)
    (Rest := Rest) hfrontier
  omega

/-- Dropping any frontier head cannot increase geometric frontier uncertainty. -/
theorem SolverState.frontierUncertainty_dropFrontierHead_le {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    (S.dropFrontierHead P Rest).frontierUncertainty ≤ S.frontierUncertainty := by
  unfold SolverState.frontierUncertainty
  exact uncertaintyBits_mono (SolverState.frontierMass_dropFrontierHead_le hfrontier)

/-- Geometric mass removed by dropping a frontier head. -/
def SolverState.dropFrontierHeadMassGain {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) : Nat :=
  S.frontierMass - (S.dropFrontierHead P Rest).frontierMass

/-- If `P :: Rest` is the frontier, the geometric mass gain of dropping the head is exactly
the head face's mass. -/
theorem SolverState.dropFrontierHeadMassGain_eq_head_mass {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    S.dropFrontierHeadMassGain P Rest = P.mass := by
  unfold SolverState.dropFrontierHeadMassGain SolverState.frontierMass SolverState.dropFrontierHead
  rw [hfrontier]
  simp only [List.map_cons, List.sum_cons]
  omega

/-- Dropping any frontier head cannot increase represented candidate-solution mass. It may be
search-unsound unless the dropped face is bad, but as an information operation it is monotone:
removing a region never adds candidate witnesses. -/
theorem SolverState.frontierSolutionMass_dropFrontierHead_le {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    (S.dropFrontierHead P Rest).frontierSolutionMass ≤ S.frontierSolutionMass := by
  unfold SolverState.frontierSolutionMass SolverState.dropFrontierHead
  rw [hfrontier]
  simp only [List.map_cons, List.sum_cons]
  exact Nat.le_add_left _ _

/-- Dropping any frontier head cannot increase the frontier's uncertainty proxy. Certified-bad
drops are the zero-loss case; arbitrary drops are information-losing but still monotone. -/
theorem SolverState.frontierSolutionUncertainty_dropFrontierHead_le_any {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    (S.dropFrontierHead P Rest).frontierSolutionUncertainty ≤
      S.frontierSolutionUncertainty := by
  unfold SolverState.frontierSolutionUncertainty
  exact uncertaintyBits_mono (SolverState.frontierSolutionMass_dropFrontierHead_le hfrontier)

/-- Candidate mass lost by dropping a frontier head. -/
def SolverState.dropFrontierHeadSolutionLoss {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) : Nat :=
  S.frontierSolutionMass - (S.dropFrontierHead P Rest).frontierSolutionMass

/-- If `P :: Rest` is the frontier, the mass lost by dropping the head is exactly the
candidate-solution mass of that head. -/
theorem SolverState.dropFrontierHeadSolutionLoss_eq_head_solutionMass {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    S.dropFrontierHeadSolutionLoss P Rest = P.solutionMass S.original := by
  unfold SolverState.dropFrontierHeadSolutionLoss SolverState.frontierSolutionMass
    SolverState.dropFrontierHead
  rw [hfrontier]
  simp only [List.map_cons, List.sum_cons]
  omega

/-- Certified-bad drops lose zero candidate-solution mass. -/
theorem SolverState.dropFrontierHeadSolutionLoss_eq_zero_of_bad {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P) :
    S.dropFrontierHeadSolutionLoss P Rest = 0 := by
  rw [SolverState.dropFrontierHeadSolutionLoss_eq_head_solutionMass hfrontier]
  exact (Pattern.solutionMass_eq_zero_iff_no_solution_in_face S.original P).mpr hbad

/-- Dropping a bad frontier head preserves total candidate-solution mass because the dropped
face has zero candidate mass. -/
theorem SolverState.frontierSolutionMass_dropBadFrontierHead {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P) :
    (S.dropFrontierHead P Rest).frontierSolutionMass = S.frontierSolutionMass := by
  have hzero : P.solutionMass S.original = 0 :=
    (Pattern.solutionMass_eq_zero_iff_no_solution_in_face S.original P).mpr hbad
  unfold SolverState.frontierSolutionMass SolverState.dropFrontierHead
  rw [hfrontier]
  simp [hzero]

/-- Dropping a certified-bad head also preserves the true frontier uncertainty, because the
dropped face represented no candidate SAT witnesses. -/
theorem SolverState.frontierSolutionUncertainty_dropBadFrontierHead {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P) :
    (S.dropFrontierHead P Rest).frontierSolutionUncertainty =
      S.frontierSolutionUncertainty := by
  unfold SolverState.frontierSolutionUncertainty
  rw [SolverState.frontierSolutionMass_dropBadFrontierHead hfrontier hbad]

/-- Dropping a bad frontier head cannot increase frontier uncertainty. -/
theorem SolverState.frontierSolutionUncertainty_dropBadFrontierHead_le {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P) :
    (S.dropFrontierHead P Rest).frontierSolutionUncertainty ≤
      S.frontierSolutionUncertainty := by
  rw [SolverState.frontierSolutionUncertainty_dropBadFrontierHead hfrontier hbad]

/-- Concrete primitive-conflict pruning: if the frontier head is contained in one
original blocker, it can be dropped. -/
theorem SolverState.dropConflictFrontierHead_preserves_searchInvariant {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)} {B : Blocker n}
    (hfrontier : S.frontier = P :: Rest) (hB : B ∈ S.original)
    (himp : P.ImpliesBlocker B) (h : S.SearchInvariant) :
    (S.dropFrontierHead P Rest).SearchInvariant :=
  SolverState.dropBadFrontierHead_preserves_searchInvariant hfrontier
    (badRegion_of_implies_blocker hB himp) h

/-- Unit propagation on the frontier head: if the `bad` child is contained in an original
blocker, then every uncovered vertex in the parent must lie in the opposite child. -/
def SolverState.propagateFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (bad : Bool)
    (hfree : P.Free i) : SolverState n :=
  { S with frontier := P.assign i (!bad) hfree :: Rest }

@[simp] theorem SolverState.frontierSize_propagateFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (bad : Bool)
    (hfree : P.Free i) :
    SolverState.frontierSize (S.propagateFrontierHead P Rest i bad hfree) = Rest.length + 1 := by
  simp [SolverState.frontierSize, SolverState.propagateFrontierHead]

@[simp] theorem SolverState.learnedSize_propagateFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (bad : Bool)
    (hfree : P.Free i) :
    SolverState.learnedSize (S.propagateFrontierHead P Rest i bad hfree) =
      SolverState.learnedSize S := by
  simp [SolverState.learnedSize, SolverState.propagateFrontierHead]

theorem SolverState.propagateFrontierHead_sound {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)} {i : Fin n} {bad : Bool}
    {hfree : P.Free i} (hS : S.Sound) :
    (S.propagateFrontierHead P Rest i bad hfree).Sound := by
  intro Q hQ
  exact hS Q hQ

theorem SolverState.propagateFrontierHead_preserves_uncovered {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)} {B : Blocker n}
    {i : Fin n} {bad : Bool} {hfree : P.Free i}
    (hfrontier : S.frontier = P :: Rest) (hB : B ∈ S.original)
    (himp : (P.assign i bad hfree).ImpliesBlocker B)
    (hcov : S.FrontierCoversUncovered) :
    (S.propagateFrontierHead P Rest i bad hfree).FrontierCoversUncovered := by
  intro v hun
  obtain ⟨Q, hQ, hQv⟩ := hcov v hun
  rw [hfrontier] at hQ
  rw [List.mem_cons] at hQ
  rcases hQ with hQP | hQrest
  · subst Q
    have hforced : v i = !bad := unit_propagation_sound hfree hB himp v hQv hun
    exact ⟨P.assign i (!bad) hfree, by simp [SolverState.propagateFrontierHead],
      (P.assign_covers_iff i (!bad) hfree v).mpr ⟨hQv, hforced⟩⟩
  · exact ⟨Q, by simp [SolverState.propagateFrontierHead, hQrest], hQv⟩

theorem SolverState.propagateFrontierHead_preserves_searchInvariant {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)} {B : Blocker n}
    {i : Fin n} {bad : Bool} {hfree : P.Free i}
    (hfrontier : S.frontier = P :: Rest) (hB : B ∈ S.original)
    (himp : (P.assign i bad hfree).ImpliesBlocker B) (h : S.SearchInvariant) :
    (S.propagateFrontierHead P Rest i bad hfree).SearchInvariant :=
  ⟨SolverState.propagateFrontierHead_sound h.1,
    SolverState.propagateFrontierHead_preserves_uncovered hfrontier hB himp h.2⟩

theorem SolverState.frontierSize_propagateFrontierHead_eq {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    {i : Fin n} {bad : Bool} {hfree : P.Free i}
    (hfrontier : S.frontier = P :: Rest) :
    SolverState.frontierSize (S.propagateFrontierHead P Rest i bad hfree) =
      SolverState.frontierSize S := by
  rw [SolverState.frontierSize_propagateFrontierHead]
  simp [SolverState.frontierSize, hfrontier]

/-- Learn the current bad frontier head and drop it in one CDCL-style conflict step. -/
def SolverState.learnAndDropFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) : SolverState n :=
  { S with learned := P :: S.learned, frontier := Rest }

@[simp] theorem SolverState.frontierSize_learnAndDropFrontierHead {n : Nat}
    (S : SolverState n) (P : Pattern n) (Rest : List (Pattern n)) :
    SolverState.frontierSize (S.learnAndDropFrontierHead P Rest) = Rest.length := by
  simp [SolverState.frontierSize, SolverState.learnAndDropFrontierHead]

@[simp] theorem SolverState.learnedSize_learnAndDropFrontierHead {n : Nat}
    (S : SolverState n) (P : Pattern n) (Rest : List (Pattern n)) :
    SolverState.learnedSize (S.learnAndDropFrontierHead P Rest) =
      SolverState.learnedSize S + 1 := by
  simp [SolverState.learnedSize, SolverState.learnAndDropFrontierHead]

theorem SolverState.learnAndDropFrontierHead_sound {n : Nat} {S : SolverState n}
    {P : Pattern n} {Rest : List (Pattern n)}
    (hbad : BadRegionFor S.original P) (hS : S.Sound) :
    (S.learnAndDropFrontierHead P Rest).Sound := by
  intro Q hQ
  have hmem : Q ∈ P :: S.learned := by
    simpa [SolverState.learnAndDropFrontierHead] using hQ
  rw [List.mem_cons] at hmem
  rcases hmem with hQP | hQold
  · simpa [hQP] using hbad
  · exact hS Q hQold

theorem SolverState.learnAndDropFrontierHead_preserves_uncovered {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P)
    (hcov : S.FrontierCoversUncovered) :
    (S.learnAndDropFrontierHead P Rest).FrontierCoversUncovered := by
  intro v hun
  obtain ⟨Q, hQ, hQv⟩ := hcov v hun
  rw [hfrontier] at hQ
  rw [List.mem_cons] at hQ
  rcases hQ with hQP | hQrest
  · subst Q
    exact False.elim (hbad v hQv hun)
  · exact ⟨Q, by simp [SolverState.learnAndDropFrontierHead, hQrest], hQv⟩

theorem SolverState.learnAndDropFrontierHead_preserves_searchInvariant {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) (hbad : BadRegionFor S.original P)
    (h : S.SearchInvariant) :
    (S.learnAndDropFrontierHead P Rest).SearchInvariant :=
  ⟨SolverState.learnAndDropFrontierHead_sound hbad h.1,
    SolverState.learnAndDropFrontierHead_preserves_uncovered hfrontier hbad h.2⟩

theorem SolverState.frontierSize_learnAndDropFrontierHead_add_one {n : Nat}
    {S : SolverState n} {P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest) :
    SolverState.frontierSize (S.learnAndDropFrontierHead P Rest) + 1 =
      SolverState.frontierSize S := by
  rw [SolverState.frontierSize_learnAndDropFrontierHead]
  simp [SolverState.frontierSize, hfrontier]

theorem SolverState.learnedSize_learnAndDropFrontierHead_eq_succ {n : Nat}
    (S : SolverState n) (P : Pattern n) (Rest : List (Pattern n)) :
    SolverState.learnedSize (S.learnAndDropFrontierHead P Rest) =
      SolverState.learnedSize S + 1 := by
  simp [SolverState.learnedSize, SolverState.learnAndDropFrontierHead]

/-- If the frontier is empty while it still covers every possible uncovered vertex, the
original blocker list is UNSAT. -/
theorem SolverState.coverUNSATList_of_empty_frontier {n : Nat} {S : SolverState n}
    (hcov : S.FrontierCoversUncovered) (hempty : S.frontier = []) :
    CoverUNSATList S.original := by
  intro hsat
  obtain ⟨v, hun⟩ := hsat
  obtain ⟨P, hP, -⟩ := hcov v hun
  rw [hempty] at hP
  simp at hP

/-- Empty-frontier search termination as a bad-region certificate for the whole cube. -/
theorem SolverState.emptyBad_of_empty_frontier {n : Nat} {S : SolverState n}
    (hcov : S.FrontierCoversUncovered) (hempty : S.frontier = []) :
    BadRegionFor S.original (Pattern.empty n) := by
  rw [← coverUNSATList_iff_emptyPattern_bad]
  exact S.coverUNSATList_of_empty_frontier hcov hempty

/-- If a frontier face contains a real uncovered vertex, the original blocker list is SAT. -/
theorem SolverState.coverSATList_of_frontier_hole {n : Nat} {S : SolverState n}
    {P : Pattern n} (_hP : P ∈ S.frontier)
    (hhole : ∃ v : Vertex n, P.Covers v ∧ IsUncovered S.original v) :
    CoverSATList S.original := by
  obtain ⟨v, -, hun⟩ := hhole
  exact ⟨v, hun⟩

/-- A frontier face inside a learned bad region is itself bad. -/
theorem SolverState.bad_frontier_of_learned_subsumes {n : Nat} {S : SolverState n}
    (hS : S.Sound) {Learned P : Pattern n}
    (hLearned : Learned ∈ S.learned) (hsub : Learned.Le P) :
    BadRegionFor S.original P :=
  BadRegionFor.of_le (hS Learned hLearned) hsub

/-- Pruning a frontier head that is subsumed by a learned bad region preserves all
still-possible uncovered vertices. -/
theorem SolverState.dropFrontierHead_of_learned_preserves_uncovered {n : Nat}
    {S : SolverState n} {Learned P : Pattern n} {Rest : List (Pattern n)}
    (hS : S.Sound) (hfrontier : S.frontier = P :: Rest)
    (hLearned : Learned ∈ S.learned) (hsub : Learned.Le P)
    (hcov : S.FrontierCoversUncovered) :
    (S.dropFrontierHead P Rest).FrontierCoversUncovered := by
  exact S.dropBadFrontierHead_preserves_uncovered hfrontier
    (S.bad_frontier_of_learned_subsumes hS hLearned hsub) hcov

theorem SolverState.dropFrontierHead_of_learned_preserves_searchInvariant {n : Nat}
    {S : SolverState n} {Learned P : Pattern n} {Rest : List (Pattern n)}
    (hfrontier : S.frontier = P :: Rest)
    (hLearned : Learned ∈ S.learned) (hsub : Learned.Le P)
    (h : S.SearchInvariant) :
    (S.dropFrontierHead P Rest).SearchInvariant :=
  SolverState.dropBadFrontierHead_preserves_searchInvariant hfrontier
    (S.bad_frontier_of_learned_subsumes h.1 hLearned hsub) h

/-- Delete learned regions by a Boolean keep predicate. Deletion can hurt performance, but
cannot hurt soundness because it only removes pruning information. -/
def SolverState.filterLearned {n : Nat} (S : SolverState n) (keep : Pattern n → Bool) :
    SolverState n :=
  { S with learned := S.learned.filter keep }

theorem SolverState.filterLearned_sound {n : Nat} {S : SolverState n}
    {keep : Pattern n → Bool} (hS : S.Sound) :
    (S.filterLearned keep).Sound := by
  intro P hP
  have hmem : P ∈ S.learned.filter keep := by
    simpa [SolverState.filterLearned] using hP
  exact hS P (List.mem_of_mem_filter hmem)

theorem SolverState.addLearned_preserves_searchInvariant {n : Nat} {S : SolverState n}
    {P : Pattern n} (hP : BadRegionFor S.original P) (h : S.SearchInvariant) :
    (S.addLearned P).SearchInvariant :=
  ⟨SolverState.addLearned_sound h.1 hP, h.2⟩

theorem SolverState.addLearnedMany_preserves_searchInvariant {n : Nat} {S : SolverState n}
    {Ps : List (Pattern n)} (hPs : ∀ P ∈ Ps, BadRegionFor S.original P)
    (h : S.SearchInvariant) :
    (S.addLearnedMany Ps).SearchInvariant :=
  ⟨SolverState.addLearnedMany_sound h.1 hPs, h.2⟩

theorem SolverState.restart_preserves_searchInvariant {n : Nat} {S : SolverState n}
    (h : S.SearchInvariant) :
    S.restart.SearchInvariant :=
  ⟨SolverState.restart_sound h.1, by simpa [SolverState.restart] using h.2⟩

theorem SolverState.filterLearned_preserves_searchInvariant {n : Nat} {S : SolverState n}
    {keep : Pattern n → Bool} (h : S.SearchInvariant) :
    (S.filterLearned keep).SearchInvariant :=
  ⟨SolverState.filterLearned_sound h.1, by simpa [SolverState.filterLearned] using h.2⟩

/-- Solver-facing operation ledger. This is deliberately multi-column: a later scalar
runtime model can choose weights, but the proof theory can still see whether cost came
from branching, propagation, conflict analysis, learning, restart, merge, scanning, or space. -/
structure SearchCostVector where
  decisions : Nat
  propagations : Nat
  blockerChecks : Nat
  conflicts : Nat
  learnedRegions : Nat
  restarts : Nat
  regionMerges : Nat
  vertexVisits : Nat
  memoryCells : Nat
  deriving DecidableEq, Repr

namespace SearchCostVector

/-- The zero search ledger. -/
def zero : SearchCostVector where
  decisions := 0
  propagations := 0
  blockerChecks := 0
  conflicts := 0
  learnedRegions := 0
  restarts := 0
  regionMerges := 0
  vertexVisits := 0
  memoryCells := 0

/-- Componentwise addition of search ledgers. -/
def add (a b : SearchCostVector) : SearchCostVector where
  decisions := a.decisions + b.decisions
  propagations := a.propagations + b.propagations
  blockerChecks := a.blockerChecks + b.blockerChecks
  conflicts := a.conflicts + b.conflicts
  learnedRegions := a.learnedRegions + b.learnedRegions
  restarts := a.restarts + b.restarts
  regionMerges := a.regionMerges + b.regionMerges
  vertexVisits := a.vertexVisits + b.vertexVisits
  memoryCells := a.memoryCells + b.memoryCells

/-- Componentwise domination of solver ledgers. -/
def Le (a b : SearchCostVector) : Prop :=
  a.decisions ≤ b.decisions ∧
  a.propagations ≤ b.propagations ∧
  a.blockerChecks ≤ b.blockerChecks ∧
  a.conflicts ≤ b.conflicts ∧
  a.learnedRegions ≤ b.learnedRegions ∧
  a.restarts ≤ b.restarts ∧
  a.regionMerges ≤ b.regionMerges ∧
  a.vertexVisits ≤ b.vertexVisits ∧
  a.memoryCells ≤ b.memoryCells

/-- Collapse a multi-column search ledger into a scalar operation count. -/
def weighted (weights a : SearchCostVector) : Nat :=
  weights.decisions * a.decisions +
  weights.propagations * a.propagations +
  weights.blockerChecks * a.blockerChecks +
  weights.conflicts * a.conflicts +
  weights.learnedRegions * a.learnedRegions +
  weights.restarts * a.restarts +
  weights.regionMerges * a.regionMerges +
  weights.vertexVisits * a.vertexVisits +
  weights.memoryCells * a.memoryCells

/-- Unit ledger for one decision branch. -/
def decisionUnit : SearchCostVector := { zero with decisions := 1 }

/-- Unit ledger for one propagation. -/
def propagationUnit : SearchCostVector := { zero with propagations := 1 }

/-- Unit ledger for one blocker check. -/
def blockerCheckUnit : SearchCostVector := { zero with blockerChecks := 1 }

/-- Unit ledger for one detected conflict. -/
def conflictUnit : SearchCostVector := { zero with conflicts := 1 }

/-- Unit ledger for one learned region. -/
def learnedRegionUnit : SearchCostVector := { zero with learnedRegions := 1 }

/-- Unit ledger for one restart. -/
def restartUnit : SearchCostVector := { zero with restarts := 1 }

/-- Unit ledger for one certified region merge. -/
def regionMergeUnit : SearchCostVector := { zero with regionMerges := 1 }

/-- Unit ledger for one visited vertex. -/
def vertexVisitUnit : SearchCostVector := { zero with vertexVisits := 1 }

/-- Unit ledger for one abstract memory cell. -/
def memoryCellUnit : SearchCostVector := { zero with memoryCells := 1 }

@[simp] theorem zero_add (a : SearchCostVector) : add zero a = a := by
  cases a
  simp [add, zero]

@[simp] theorem add_zero (a : SearchCostVector) : add a zero = a := by
  cases a
  simp [add, zero]

theorem add_comm (a b : SearchCostVector) : add a b = add b a := by
  cases a
  cases b
  simp [add, Nat.add_comm]

theorem add_assoc (a b c : SearchCostVector) :
    add (add a b) c = add a (add b c) := by
  cases a
  cases b
  cases c
  simp [add, Nat.add_assoc]

theorem weighted_add (weights a b : SearchCostVector) :
    weighted weights (add a b) = weighted weights a + weighted weights b := by
  cases weights
  cases a
  cases b
  simp [weighted, add]
  ring

/-- Componentwise big-O for search-ledger families. -/
def BigO (f g : Nat → SearchCostVector) : Prop :=
  _root_.HypercubeSAT.BigO (fun N => (f N).decisions) (fun N => (g N).decisions) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).propagations) (fun N => (g N).propagations) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).blockerChecks) (fun N => (g N).blockerChecks) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).conflicts) (fun N => (g N).conflicts) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).learnedRegions) (fun N => (g N).learnedRegions) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).restarts) (fun N => (g N).restarts) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).regionMerges) (fun N => (g N).regionMerges) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).vertexVisits) (fun N => (g N).vertexVisits) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).memoryCells) (fun N => (g N).memoryCells)

/-- A search-ledger family is polynomially bounded when every operation column is. -/
def PolynomialO (f : Nat → SearchCostVector) : Prop :=
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).decisions) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).propagations) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).blockerChecks) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).conflicts) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).learnedRegions) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).restarts) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).regionMerges) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).vertexVisits) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).memoryCells)

/-- Weighted scalar projection of a search-ledger family. -/
def weightedFamily (weights : SearchCostVector) (f : Nat → SearchCostVector) : Nat → Nat :=
  fun N => weighted weights (f N)

end SearchCostVector

/-- Primitive solver-accounting operations. These are not yet an implementation; they are
the finite alphabet of operations whose chains we can count. -/
inductive SearchOp where
  | decide
  | propagate
  | checkBlocker
  | conflict
  | learnRegion
  | restart
  | mergeRegion
  | visitVertex
  | allocateCell
  deriving DecidableEq, Repr

namespace SearchOp

/-- Ledger cost of one primitive search operation. -/
def cost : SearchOp → SearchCostVector
  | decide => SearchCostVector.decisionUnit
  | propagate => SearchCostVector.propagationUnit
  | checkBlocker => SearchCostVector.blockerCheckUnit
  | conflict => SearchCostVector.conflictUnit
  | learnRegion => SearchCostVector.learnedRegionUnit
  | restart => SearchCostVector.restartUnit
  | mergeRegion => SearchCostVector.regionMergeUnit
  | visitVertex => SearchCostVector.vertexVisitUnit
  | allocateCell => SearchCostVector.memoryCellUnit

end SearchOp

/-- Total ledger for a finite operation trace. This is the accountant for rule chains. -/
def SearchTraceCost : List SearchOp → SearchCostVector
  | [] => SearchCostVector.zero
  | op :: ops => SearchCostVector.add op.cost (SearchTraceCost ops)

@[simp] theorem SearchTraceCost_nil :
    SearchTraceCost [] = SearchCostVector.zero := rfl

@[simp] theorem SearchTraceCost_cons (op : SearchOp) (ops : List SearchOp) :
    SearchTraceCost (op :: ops) = SearchCostVector.add op.cost (SearchTraceCost ops) := rfl

/-- Sequential composition of operation traces adds their ledgers. -/
theorem SearchTraceCost_append (xs ys : List SearchOp) :
    SearchTraceCost (xs ++ ys) =
      SearchCostVector.add (SearchTraceCost xs) (SearchTraceCost ys) := by
  induction xs with
  | nil =>
      simp [SearchTraceCost]
  | cons op xs ih =>
      simp [SearchTraceCost, ih, SearchCostVector.add_assoc]

/-! ### §E0.2 Abstract solver calculus and strategy classification -/

/-- Coarse taxonomy of hypercube solver moves. A concrete implementation can refine these,
but this gives the strategy calculus a finite grammar of meaningful operation families. -/
inductive SolverMoveKind where
  | branch
  | propagate
  | conflict
  | learn
  | restart
  | delete
  | simplify
  | chooseFace
  | chooseVariable
  | checkVertex
  | share
  | merge
  deriving DecidableEq, Repr

namespace SolverMoveKind

/-- Default accounting vector for one move kind. More precise implementations may charge
larger or model-specific ledgers, but every move has a canonical unit cost column. -/
def defaultCost : SolverMoveKind → SearchCostVector
  | branch => SearchCostVector.decisionUnit
  | propagate => SearchCostVector.propagationUnit
  | conflict => SearchCostVector.conflictUnit
  | learn => SearchCostVector.learnedRegionUnit
  | restart => SearchCostVector.restartUnit
  | delete => SearchCostVector.memoryCellUnit
  | simplify => SearchCostVector.regionMergeUnit
  | chooseFace => SearchCostVector.memoryCellUnit
  | chooseVariable => SearchCostVector.memoryCellUnit
  | checkVertex => SearchCostVector.vertexVisitUnit
  | share => SearchCostVector.learnedRegionUnit
  | merge => SearchCostVector.regionMergeUnit

end SolverMoveKind

/-- A certified local move from one solver state. The move carries its after-state,
operation kind, operation ledger, and the proof that it preserves the learned-region
soundness invariant. -/
structure SolverMove {n : Nat} (S : SolverState n) where
  after : SolverState n
  kind : SolverMoveKind
  cost : SearchCostVector
  sound : S.Sound → after.Sound

namespace SolverMove

/-- A no-op move, useful for policies that only classify or inspect a state. -/
def noop {n : Nat} (S : SolverState n) : SolverMove S where
  after := S
  kind := SolverMoveKind.chooseFace
  cost := SearchCostVector.zero
  sound := fun hS => hS

/-- Restart as a certified local move. -/
def restart {n : Nat} (S : SolverState n) : SolverMove S where
  after := S.restart
  kind := SolverMoveKind.restart
  cost := SolverMoveKind.defaultCost SolverMoveKind.restart
  sound := SolverState.restart_sound

/-- Learning one already-certified bad region as a certified local move. -/
def learn {n : Nat} (S : SolverState n) (P : Pattern n)
    (hP : BadRegionFor S.original P) : SolverMove S where
  after := S.addLearned P
  kind := SolverMoveKind.learn
  cost := SolverMoveKind.defaultCost SolverMoveKind.learn
  sound := fun hS => SolverState.addLearned_sound hS hP

/-- Sharing/importing a batch of certified bad regions as a local move. -/
def share {n : Nat} (S : SolverState n) (Ps : List (Pattern n))
    (hPs : ∀ P ∈ Ps, BadRegionFor S.original P) : SolverMove S where
  after := S.addLearnedMany Ps
  kind := SolverMoveKind.share
  cost := SolverMoveKind.defaultCost SolverMoveKind.share
  sound := fun hS => SolverState.addLearnedMany_sound hS hPs

/-- Branching a face as a certified local move. Its semantic effect is search refinement:
it adds the two children to the frontier and leaves learned-region soundness unchanged. -/
def branchFace {n : Nat} (S : SolverState n) (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) : SolverMove S where
  after := S.branchFace P i hfree
  kind := SolverMoveKind.branch
  cost := SolverMoveKind.defaultCost SolverMoveKind.branch
  sound := fun hS => SolverState.branchFace_sound hS

/-- Exact head-branching as a certified move. -/
def branchFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (hfree : P.Free i) :
    SolverMove S where
  after := S.branchFrontierHead P Rest i hfree
  kind := SolverMoveKind.branch
  cost := SolverMoveKind.defaultCost SolverMoveKind.branch
  sound := fun hS => SolverState.branchFrontierHead_sound hS

/-- Pruning a frontier head that has already been certified bad. -/
def dropBadFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (_hbad : BadRegionFor S.original P) :
    SolverMove S where
  after := S.dropFrontierHead P Rest
  kind := SolverMoveKind.conflict
  cost := SolverMoveKind.defaultCost SolverMoveKind.conflict
  sound := fun hS => SolverState.dropBadFrontierHead_sound hS

/-- Learned-region deletion as a certified local move. -/
def deleteLearned {n : Nat} (S : SolverState n) (keep : Pattern n → Bool) :
    SolverMove S where
  after := S.filterLearned keep
  kind := SolverMoveKind.delete
  cost := SolverMoveKind.defaultCost SolverMoveKind.delete
  sound := fun hS => SolverState.filterLearned_sound hS

end SolverMove

/-- A stronger certified move: it preserves the full search invariant, not only learned
database soundness. Branching/pruning policies should use this interface. -/
structure SearchMove {n : Nat} (S : SolverState n) where
  after : SolverState n
  kind : SolverMoveKind
  cost : SearchCostVector
  same_original : after.original = S.original
  preserves : S.SearchInvariant → after.SearchInvariant

namespace SearchMove

def restart {n : Nat} (S : SolverState n) : SearchMove S where
  after := S.restart
  kind := SolverMoveKind.restart
  cost := SolverMoveKind.defaultCost SolverMoveKind.restart
  same_original := rfl
  preserves := SolverState.restart_preserves_searchInvariant

def learn {n : Nat} (S : SolverState n) (P : Pattern n)
    (hP : BadRegionFor S.original P) : SearchMove S where
  after := S.addLearned P
  kind := SolverMoveKind.learn
  cost := SolverMoveKind.defaultCost SolverMoveKind.learn
  same_original := rfl
  preserves := fun h => SolverState.addLearned_preserves_searchInvariant hP h

def share {n : Nat} (S : SolverState n) (Ps : List (Pattern n))
    (hPs : ∀ P ∈ Ps, BadRegionFor S.original P) : SearchMove S where
  after := S.addLearnedMany Ps
  kind := SolverMoveKind.share
  cost := SolverMoveKind.defaultCost SolverMoveKind.share
  same_original := rfl
  preserves := fun h => SolverState.addLearnedMany_preserves_searchInvariant hPs h

def deleteLearned {n : Nat} (S : SolverState n) (keep : Pattern n → Bool) :
    SearchMove S where
  after := S.filterLearned keep
  kind := SolverMoveKind.delete
  cost := SolverMoveKind.defaultCost SolverMoveKind.delete
  same_original := rfl
  preserves := SolverState.filterLearned_preserves_searchInvariant

def recordDecision {n : Nat} (S : SolverState n) (i : Fin n) (b : Bool) (level : Nat) :
    SearchMove S where
  after := S.pushTrail { var := i, val := b, level := level, reason := none }
  kind := SolverMoveKind.chooseVariable
  cost := SearchCostVector.decisionUnit
  same_original := rfl
  preserves := SolverState.pushTrail_preserves_searchInvariant

def recordPropagation {n : Nat} (S : SolverState n) (i : Fin n) (b : Bool)
    (level : Nat) (reason : Pattern n) : SearchMove S where
  after := S.pushTrail { var := i, val := b, level := level, reason := some reason }
  kind := SolverMoveKind.propagate
  cost := SearchCostVector.propagationUnit
  same_original := rfl
  preserves := SolverState.pushTrail_preserves_searchInvariant

def branchFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (i : Fin n) (hfree : P.Free i)
    (hfrontier : S.frontier = P :: Rest) : SearchMove S where
  after := S.branchFrontierHead P Rest i hfree
  kind := SolverMoveKind.branch
  cost := SolverMoveKind.defaultCost SolverMoveKind.branch
  same_original := rfl
  preserves := fun h => SolverState.branchFrontierHead_preserves_searchInvariant hfrontier h

def dropBadFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (hbad : BadRegionFor S.original P) : SearchMove S where
  after := S.dropFrontierHead P Rest
  kind := SolverMoveKind.conflict
  cost := SolverMoveKind.defaultCost SolverMoveKind.conflict
  same_original := rfl
  preserves := fun h => SolverState.dropBadFrontierHead_preserves_searchInvariant hfrontier hbad h

def dropConflictFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (B : Blocker n) (hB : B ∈ S.original) (himp : P.ImpliesBlocker B) : SearchMove S where
  after := S.dropFrontierHead P Rest
  kind := SolverMoveKind.conflict
  cost := SearchCostVector.add SearchCostVector.conflictUnit SearchCostVector.blockerCheckUnit
  same_original := rfl
  preserves := fun h =>
    SolverState.dropConflictFrontierHead_preserves_searchInvariant hfrontier hB himp h

def dropConflictFrontierHeadOfFixes {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (B : Blocker n) (hB : B ∈ S.original)
    (hi : P.Fixes B.i B.bi) (hj : P.Fixes B.j B.bj) (hk : P.Fixes B.k B.bk) :
    SearchMove S :=
  dropConflictFrontierHead S P Rest hfrontier B hB
    (Pattern.impliesBlocker_of_fixes hi hj hk)

def propagateFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (B : Blocker n) (i : Fin n) (bad : Bool) (hfree : P.Free i)
    (hB : B ∈ S.original) (himp : (P.assign i bad hfree).ImpliesBlocker B) :
    SearchMove S where
  after := S.propagateFrontierHead P Rest i bad hfree
  kind := SolverMoveKind.propagate
  cost := SearchCostVector.add SearchCostVector.propagationUnit SearchCostVector.blockerCheckUnit
  same_original := rfl
  preserves := fun h =>
    SolverState.propagateFrontierHead_preserves_searchInvariant hfrontier hB himp h

def propagateFrontierHeadOfFixes {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (B : Blocker n) (i : Fin n) (bad : Bool) (hfree : P.Free i)
    (hB : B ∈ S.original)
    (hi : (P.assign i bad hfree).Fixes B.i B.bi)
    (hj : (P.assign i bad hfree).Fixes B.j B.bj)
    (hk : (P.assign i bad hfree).Fixes B.k B.bk) :
    SearchMove S :=
  propagateFrontierHead S P Rest hfrontier B i bad hfree hB
    (Pattern.impliesBlocker_of_fixes hi hj hk)

def learnAndDropBadFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (hbad : BadRegionFor S.original P) : SearchMove S where
  after := S.learnAndDropFrontierHead P Rest
  kind := SolverMoveKind.learn
  cost := SearchCostVector.add SearchCostVector.learnedRegionUnit SearchCostVector.conflictUnit
  same_original := rfl
  preserves := fun h =>
    SolverState.learnAndDropFrontierHead_preserves_searchInvariant hfrontier hbad h

def learnAndDropConflictFrontierHead {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (B : Blocker n) (hB : B ∈ S.original) (himp : P.ImpliesBlocker B) :
    SearchMove S where
  after := S.learnAndDropFrontierHead P Rest
  kind := SolverMoveKind.learn
  cost :=
    SearchCostVector.add SearchCostVector.learnedRegionUnit
      (SearchCostVector.add SearchCostVector.conflictUnit SearchCostVector.blockerCheckUnit)
  same_original := rfl
  preserves := fun h =>
    SolverState.learnAndDropFrontierHead_preserves_searchInvariant hfrontier
      (badRegion_of_implies_blocker hB himp) h

def learnAndDropConflictFrontierHeadOfFixes {n : Nat} (S : SolverState n)
    (P : Pattern n) (Rest : List (Pattern n)) (hfrontier : S.frontier = P :: Rest)
    (B : Blocker n) (hB : B ∈ S.original)
    (hi : P.Fixes B.i B.bi) (hj : P.Fixes B.j B.bj) (hk : P.Fixes B.k B.bk) :
    SearchMove S :=
  learnAndDropConflictFrontierHead S P Rest hfrontier B hB
    (Pattern.impliesBlocker_of_fixes hi hj hk)

/-- Weighted scalar cost of a certified move under a chosen operation model. -/
def weightedCost {n : Nat} {S : SolverState n} (weights : SearchCostVector)
    (move : SearchMove S) : Nat :=
  SearchCostVector.weighted weights move.cost

/-- Geometric frontier mass removed by a move. Positive values mean the move shrank the
active proof-search region. -/
def frontierMassDrop {n : Nat} {S : SolverState n} (move : SearchMove S) : Nat :=
  S.frontierMass - move.after.frontierMass

/-- Geometric frontier uncertainty removed by a move. -/
def frontierUncertaintyDrop {n : Nat} {S : SolverState n} (move : SearchMove S) : Nat :=
  S.frontierUncertainty - move.after.frontierUncertainty

/-- Candidate-solution uncertainty removed by a move. Sound pruning normally preserves this;
it becomes useful for unsafe analyses, failed-literal tests, or bounded incomplete workers. -/
def solutionUncertaintyDrop {n : Nat} {S : SolverState n} (move : SearchMove S) : Nat :=
  S.frontierSolutionUncertainty - move.after.frontierSolutionUncertainty

/-- `a` is at least as information-efficient as `b` under `weights`, using cross-multiplication
to avoid rational numbers. The `+ 1` denominator makes zero-cost bookkeeping harmless. -/
def GeometricInfoEfficiencyDominates {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (a b : SearchMove S) : Prop :=
  b.frontierUncertaintyDrop * (a.weightedCost weights + 1) ≤
    a.frontierUncertaintyDrop * (b.weightedCost weights + 1)

/-- A move is cost-normalized information-optimal among a finite menu of certified moves. -/
def GeometricInfoOptimalAmong {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (choices : List (SearchMove S)) (move : SearchMove S) : Prop :=
  move ∈ choices ∧ ∀ other ∈ choices, GeometricInfoEfficiencyDominates weights move other

theorem GeometricInfoOptimalAmong.best {n : Nat} {S : SolverState n}
    {weights : SearchCostVector} {choices : List (SearchMove S)} {move other : SearchMove S}
    (hopt : GeometricInfoOptimalAmong weights choices move) (hother : other ∈ choices) :
    GeometricInfoEfficiencyDominates weights move other :=
  hopt.2 other hother

theorem GeometricInfoOptimalAmong.best_cross_mul {n : Nat} {S : SolverState n}
    {weights : SearchCostVector} {choices : List (SearchMove S)} {move other : SearchMove S}
    (hopt : GeometricInfoOptimalAmong weights choices move) (hother : other ∈ choices) :
    other.frontierUncertaintyDrop * (move.weightedCost weights + 1) ≤
      move.frontierUncertaintyDrop * (other.weightedCost weights + 1) :=
  hopt.best hother

/-- Candidate-solution version of cost-normalized information dominance. -/
def SolutionInfoEfficiencyDominates {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (a b : SearchMove S) : Prop :=
  b.solutionUncertaintyDrop * (a.weightedCost weights + 1) ≤
    a.solutionUncertaintyDrop * (b.weightedCost weights + 1)

def SolutionInfoOptimalAmong {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (choices : List (SearchMove S)) (move : SearchMove S) : Prop :=
  move ∈ choices ∧ ∀ other ∈ choices, SolutionInfoEfficiencyDominates weights move other

theorem SolutionInfoOptimalAmong.best {n : Nat} {S : SolverState n}
    {weights : SearchCostVector} {choices : List (SearchMove S)} {move other : SearchMove S}
    (hopt : SolutionInfoOptimalAmong weights choices move) (hother : other ∈ choices) :
    SolutionInfoEfficiencyDominates weights move other :=
  hopt.2 other hother

theorem SolutionInfoOptimalAmong.best_cross_mul {n : Nat} {S : SolverState n}
    {weights : SearchCostVector} {choices : List (SearchMove S)} {move other : SearchMove S}
    (hopt : SolutionInfoOptimalAmong weights choices move) (hother : other ∈ choices) :
    other.solutionUncertaintyDrop * (move.weightedCost weights + 1) ≤
      move.solutionUncertaintyDrop * (other.weightedCost weights + 1) :=
  hopt.best hother

end SearchMove

/-! ### §E0.2a Information-walk rule accounting -/

/-- Fine-grained cost decomposition for a recognized solver rule. This is the "accountant"
layer: a rule may be sound and useful, but it still has to pay to be recognized, applied,
certified, checked, and carried forward through the frontier. -/
structure RuleCostBreakdown where
  matchCost : SearchCostVector
  applicationCost : SearchCostVector
  certificateCost : SearchCostVector
  checkCost : SearchCostVector
  frontierCost : SearchCostVector
  deriving DecidableEq, Repr

namespace RuleCostBreakdown

/-- The zero rule-overhead ledger. -/
def zero : RuleCostBreakdown where
  matchCost := SearchCostVector.zero
  applicationCost := SearchCostVector.zero
  certificateCost := SearchCostVector.zero
  checkCost := SearchCostVector.zero
  frontierCost := SearchCostVector.zero

/-- Total overhead of a successful rule match, excluding the certified move's own operation
ledger. -/
def total (c : RuleCostBreakdown) : SearchCostVector :=
  SearchCostVector.add c.matchCost
    (SearchCostVector.add c.applicationCost
      (SearchCostVector.add c.certificateCost
        (SearchCostVector.add c.checkCost c.frontierCost)))

@[simp] theorem total_zero : total zero = SearchCostVector.zero := by
  simp [total, zero]

end RuleCostBreakdown

/-- A certified rule application is a sound search move plus the overhead paid to discover and
use that move. This is the formal version of
`match + apply + certificate + check + frontier + move`. -/
structure CertifiedRuleApplication {n : Nat} (S : SolverState n) where
  move : SearchMove S
  breakdown : RuleCostBreakdown

namespace CertifiedRuleApplication

/-- Total multi-column ledger for this rule application. -/
def totalCost {n : Nat} {S : SolverState n} (A : CertifiedRuleApplication S) :
    SearchCostVector :=
  SearchCostVector.add A.breakdown.total A.move.cost

/-- Weighted scalar cost of a certified rule application. -/
def weightedCost {n : Nat} {S : SolverState n} (weights : SearchCostVector)
    (A : CertifiedRuleApplication S) : Nat :=
  SearchCostVector.weighted weights A.totalCost

/-- Geometric frontier uncertainty removed by this rule application. -/
def frontierUncertaintyDrop {n : Nat} {S : SolverState n}
    (A : CertifiedRuleApplication S) : Nat :=
  A.move.frontierUncertaintyDrop

/-- Candidate-solution uncertainty removed by this rule application. -/
def solutionUncertaintyDrop {n : Nat} {S : SolverState n}
    (A : CertifiedRuleApplication S) : Nat :=
  A.move.solutionUncertaintyDrop

theorem preserves_searchInvariant {n : Nat} {S : SolverState n}
    (A : CertifiedRuleApplication S) (hS : S.SearchInvariant) :
    A.move.after.SearchInvariant :=
  A.move.preserves hS

theorem frontierUncertaintyDrop_le_before {n : Nat} {S : SolverState n}
    (A : CertifiedRuleApplication S) :
    A.frontierUncertaintyDrop ≤ S.frontierUncertainty := by
  unfold frontierUncertaintyDrop SearchMove.frontierUncertaintyDrop
  exact Nat.sub_le _ _

theorem solutionUncertaintyDrop_le_before {n : Nat} {S : SolverState n}
    (A : CertifiedRuleApplication S) :
    A.solutionUncertaintyDrop ≤ S.frontierSolutionUncertainty := by
  unfold solutionUncertaintyDrop SearchMove.solutionUncertaintyDrop
  exact Nat.sub_le _ _

/-- Geometric information-efficiency comparison for fully accounted rule applications. -/
def GeometricEfficiencyDominates {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (a b : CertifiedRuleApplication S) : Prop :=
  b.frontierUncertaintyDrop * (a.weightedCost weights + 1) ≤
    a.frontierUncertaintyDrop * (b.weightedCost weights + 1)

/-- Candidate-solution information-efficiency comparison for fully accounted rule applications. -/
def SolutionEfficiencyDominates {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (a b : CertifiedRuleApplication S) : Prop :=
  b.solutionUncertaintyDrop * (a.weightedCost weights + 1) ≤
    a.solutionUncertaintyDrop * (b.weightedCost weights + 1)

def GeometricOptimalAmong {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (choices : List (CertifiedRuleApplication S))
    (A : CertifiedRuleApplication S) : Prop :=
  A ∈ choices ∧ ∀ B ∈ choices, GeometricEfficiencyDominates weights A B

def SolutionOptimalAmong {n : Nat} {S : SolverState n}
    (weights : SearchCostVector) (choices : List (CertifiedRuleApplication S))
    (A : CertifiedRuleApplication S) : Prop :=
  A ∈ choices ∧ ∀ B ∈ choices, SolutionEfficiencyDominates weights A B

theorem GeometricOptimalAmong.best {n : Nat} {S : SolverState n}
    {weights : SearchCostVector} {choices : List (CertifiedRuleApplication S)}
    {A B : CertifiedRuleApplication S}
    (hA : GeometricOptimalAmong weights choices A) (hB : B ∈ choices) :
    GeometricEfficiencyDominates weights A B :=
  hA.2 B hB

theorem SolutionOptimalAmong.best {n : Nat} {S : SolverState n}
    {weights : SearchCostVector} {choices : List (CertifiedRuleApplication S)}
    {A B : CertifiedRuleApplication S}
    (hA : SolutionOptimalAmong weights choices A) (hB : B ∈ choices) :
    SolutionEfficiencyDominates weights A B :=
  hA.2 B hB

end CertifiedRuleApplication

/-- A pattern rule is a recognizer plus a certified application. A failed recognizer still has a
miss cost; a successful recognizer yields a fully accounted, sound state transition. -/
structure InformationPatternRule where
  MatchCert : Type
  recognize : {n : Nat} → (S : SolverState n) → Option MatchCert
  missCost : {n : Nat} → (S : SolverState n) → SearchCostVector
  onMatch : {n : Nat} → (S : SolverState n) → MatchCert → CertifiedRuleApplication S

namespace InformationPatternRule

/-- Total cost of trying a rule once in state `S`, charging miss cost if the recognizer fails and
full application cost if it succeeds. -/
def attemptCost (R : InformationPatternRule) {n : Nat} (S : SolverState n) :
    SearchCostVector :=
  match R.recognize S with
  | none => R.missCost S
  | some cert => (R.onMatch S cert).totalCost

/-- Geometric uncertainty removed by one rule attempt. Failed recognizers remove none. -/
def attemptFrontierUncertaintyDrop (R : InformationPatternRule)
    {n : Nat} (S : SolverState n) : Nat :=
  match R.recognize S with
  | none => 0
  | some cert => (R.onMatch S cert).frontierUncertaintyDrop

/-- Candidate-solution uncertainty removed by one rule attempt. Failed recognizers remove none. -/
def attemptSolutionUncertaintyDrop (R : InformationPatternRule)
    {n : Nat} (S : SolverState n) : Nat :=
  match R.recognize S with
  | none => 0
  | some cert => (R.onMatch S cert).solutionUncertaintyDrop

theorem attemptCost_none (R : InformationPatternRule)
    {n : Nat} (S : SolverState n) (h : R.recognize S = none) :
    R.attemptCost S = R.missCost S := by
  simp [attemptCost, h]

theorem attemptCost_some (R : InformationPatternRule)
    {n : Nat} (S : SolverState n) {cert : R.MatchCert}
    (h : R.recognize S = some cert) :
    R.attemptCost S = (R.onMatch S cert).totalCost := by
  simp [attemptCost, h]

theorem attemptFrontierUncertaintyDrop_some (R : InformationPatternRule)
    {n : Nat} (S : SolverState n) {cert : R.MatchCert}
    (h : R.recognize S = some cert) :
    R.attemptFrontierUncertaintyDrop S =
      (R.onMatch S cert).frontierUncertaintyDrop := by
  simp [attemptFrontierUncertaintyDrop, h]

theorem attempt_preserves_searchInvariant (R : InformationPatternRule)
    {n : Nat} (S : SolverState n) {cert : R.MatchCert}
    (_h : R.recognize S = some cert) (hS : S.SearchInvariant) :
    (R.onMatch S cert).move.after.SearchInvariant :=
  (R.onMatch S cert).preserves_searchInvariant hS

end InformationPatternRule

/-- A menu of fully accounted rule applications available at a state. A scheduler can optimize
over this menu by information gain per weighted cost. -/
abbrev InformationRuleMenu :=
  {n : Nat} → (S : SolverState n) → List (CertifiedRuleApplication S)

/-- A menu of certified search moves available at a state. Different solver classes get different
menus: DPLL menus, CDCL menus, restart menus, portfolio-sharing menus, and so on. -/
abbrev SearchMoveMenu :=
  {n : Nat} → (S : SolverState n) → List (SearchMove S)

/-- A state policy is locally best over an accounted rule menu when it chooses a rule application
with maximal geometric information gain per operation cost. -/
def LocallyBestAccountedRuleByGeometry (weights : SearchCostVector)
    (menu : InformationRuleMenu)
    (choose : {n : Nat} → (S : SolverState n) → Option (CertifiedRuleApplication S)) : Prop :=
  ∀ {n : Nat} (S : SolverState n) (A : CertifiedRuleApplication S),
    choose S = some A →
      CertifiedRuleApplication.GeometricOptimalAmong weights (menu S) A

/-- Candidate-solution analogue of local accounted-rule optimality. -/
def LocallyBestAccountedRuleBySolutions (weights : SearchCostVector)
    (menu : InformationRuleMenu)
    (choose : {n : Nat} → (S : SolverState n) → Option (CertifiedRuleApplication S)) : Prop :=
  ∀ {n : Nat} (S : SolverState n) (A : CertifiedRuleApplication S),
    choose S = some A →
      CertifiedRuleApplication.SolutionOptimalAmong weights (menu S) A

/-- Repackage an accounted rule application as an ordinary search move whose cost includes the
recognition/application/certificate/check/frontier overhead. -/
def CertifiedRuleApplication.toSearchMove {n : Nat} {S : SolverState n}
    (A : CertifiedRuleApplication S) : SearchMove S where
  after := A.move.after
  kind := A.move.kind
  cost := A.totalCost
  same_original := A.move.same_original
  preserves := A.move.preserves

/-- Attach an overhead breakdown to an already-certified search move. -/
def CertifiedRuleApplication.ofSearchMove {n : Nat} {S : SolverState n}
    (move : SearchMove S) (breakdown : RuleCostBreakdown) :
    CertifiedRuleApplication S where
  move := move
  breakdown := breakdown

/-- Treat a certified search move as an accounted rule with no extra recognition overhead. -/
def CertifiedRuleApplication.noOverhead {n : Nat} {S : SolverState n}
    (move : SearchMove S) : CertifiedRuleApplication S :=
  CertifiedRuleApplication.ofSearchMove move RuleCostBreakdown.zero

/-- Convert a concrete search-move menu into a fully accounted menu by charging a supplied
overhead breakdown to each move. This is the bridge from existing CDCL/DPLL menus to the new
information-walk accountant. -/
def accountSearchMoveMenu (menu : SearchMoveMenu)
    (overhead : {n : Nat} → (S : SolverState n) → SearchMove S → RuleCostBreakdown) :
    InformationRuleMenu :=
  fun S => (menu S).map fun move =>
    CertifiedRuleApplication.ofSearchMove move (overhead S move)

/-- Account an existing menu with zero overhead. Useful as a baseline before charging
recognizer/miss/certificate costs. -/
def accountSearchMoveMenuNoOverhead (menu : SearchMoveMenu) : InformationRuleMenu :=
  accountSearchMoveMenu menu (fun _ _ => RuleCostBreakdown.zero)

/-- Accounted strategy: a policy over certified rule applications, not merely over final moves.
This is the executable form of the information-walk abstraction. -/
structure AccountedSearchStrategy where
  choose : {n : Nat} → (S : SolverState n) → Option (CertifiedRuleApplication S)

namespace AccountedSearchStrategy

/-- Run an accounted strategy for bounded fuel. -/
def run (σ : AccountedSearchStrategy) {n : Nat} : SolverState n → Nat → SolverState n
  | S, 0 => S
  | S, fuel + 1 =>
      match σ.choose S with
      | none => S
      | some A => run σ A.move.after fuel

/-- Full accounted ledger of a bounded run. -/
def runCost (σ : AccountedSearchStrategy) {n : Nat} : SolverState n → Nat → SearchCostVector
  | _, 0 => SearchCostVector.zero
  | S, fuel + 1 =>
      match σ.choose S with
      | none => SearchCostVector.zero
      | some A => SearchCostVector.add A.totalCost (runCost σ A.move.after fuel)

/-- Weighted scalar cost of a bounded accounted run. -/
def weightedRunCost (σ : AccountedSearchStrategy) (weights : SearchCostVector)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Nat :=
  SearchCostVector.weighted weights (σ.runCost S fuel)

/-- Geometric frontier uncertainty removed by a bounded accounted run. -/
def runFrontierUncertaintyDrop (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Nat :=
  S.frontierUncertainty - (σ.run S fuel).frontierUncertainty

/-- Candidate-solution uncertainty removed by a bounded accounted run. -/
def runSolutionUncertaintyDrop (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Nat :=
  S.frontierSolutionUncertainty - (σ.run S fuel).frontierSolutionUncertainty

theorem run_preserves_searchInvariant (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) (hS : S.SearchInvariant) :
    (σ.run S fuel).SearchInvariant := by
  induction fuel generalizing S with
  | zero =>
      exact hS
  | succ fuel ih =>
      unfold run
      cases hchoose : σ.choose S with
      | none =>
          exact hS
      | some A =>
          exact ih A.move.after (A.preserves_searchInvariant hS)

theorem run_same_original (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) :
    (σ.run S fuel).original = S.original := by
  induction fuel generalizing S with
  | zero =>
      rfl
  | succ fuel ih =>
      unfold run
      cases hchoose : σ.choose S with
      | none =>
          rfl
      | some A =>
          exact (ih A.move.after).trans A.move.same_original

theorem coverUNSATList_of_empty_frontier_after_run (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hS : S.SearchInvariant) (hempty : (σ.run S fuel).frontier = []) :
    CoverUNSATList S.original := by
  have hrun := σ.run_preserves_searchInvariant S fuel hS
  have hunsat : CoverUNSATList (σ.run S fuel).original :=
    SolverState.coverUNSATList_of_empty_frontier hrun.2 hempty
  simpa [σ.run_same_original S fuel] using hunsat

theorem coverUNSATList_of_initial_empty_frontier (σ : AccountedSearchStrategy)
    {n : Nat} (Bs : List (Blocker n)) (fuel : Nat)
    (hempty : (σ.run (SolverState.initial Bs) fuel).frontier = []) :
    CoverUNSATList Bs := by
  exact σ.coverUNSATList_of_empty_frontier_after_run (SolverState.initial Bs) fuel
    (SolverState.initial_searchInvariant Bs) hempty

@[simp] theorem runCost_zero (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) :
    σ.runCost S 0 = SearchCostVector.zero := rfl

@[simp] theorem weightedRunCost_zero (σ : AccountedSearchStrategy)
    (weights : SearchCostVector) {n : Nat} (S : SolverState n) :
    σ.weightedRunCost weights S 0 = 0 := by
  simp [weightedRunCost, SearchCostVector.weighted, SearchCostVector.zero]

theorem runCost_succ_none (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hchoose : σ.choose S = none) :
    σ.runCost S (fuel + 1) = SearchCostVector.zero := by
  simp [runCost, hchoose]

theorem runCost_succ_some (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) {A : CertifiedRuleApplication S}
    (hchoose : σ.choose S = some A) :
    σ.runCost S (fuel + 1) =
      SearchCostVector.add A.totalCost (σ.runCost A.move.after fuel) := by
  simp [runCost, hchoose]

theorem weightedRunCost_succ_some (σ : AccountedSearchStrategy)
    (weights : SearchCostVector) {n : Nat} (S : SolverState n) (fuel : Nat)
    {A : CertifiedRuleApplication S} (hchoose : σ.choose S = some A) :
    σ.weightedRunCost weights S (fuel + 1) =
      A.weightedCost weights + σ.weightedRunCost weights A.move.after fuel := by
  rw [weightedRunCost, runCost_succ_some σ S fuel hchoose, SearchCostVector.weighted_add]
  rfl

theorem run_one_some (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) {A : CertifiedRuleApplication S}
    (hchoose : σ.choose S = some A) :
    σ.run S 1 = A.move.after := by
  simp [run, hchoose]

theorem runCost_one_some (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) {A : CertifiedRuleApplication S}
    (hchoose : σ.choose S = some A) :
    σ.runCost S 1 = A.totalCost := by
  rw [runCost_succ_some σ S 0 hchoose]
  simp

theorem weightedRunCost_one_some (σ : AccountedSearchStrategy)
    (weights : SearchCostVector) {n : Nat} (S : SolverState n)
    {A : CertifiedRuleApplication S} (hchoose : σ.choose S = some A) :
    σ.weightedRunCost weights S 1 = A.weightedCost weights := by
  simp [weightedRunCost, runCost_one_some σ S hchoose, CertifiedRuleApplication.weightedCost]

theorem runFrontierUncertaintyDrop_one_some (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) {A : CertifiedRuleApplication S}
    (hchoose : σ.choose S = some A) :
    σ.runFrontierUncertaintyDrop S 1 = A.frontierUncertaintyDrop := by
  simp [runFrontierUncertaintyDrop, run_one_some σ S hchoose,
    CertifiedRuleApplication.frontierUncertaintyDrop, SearchMove.frontierUncertaintyDrop]

theorem runSolutionUncertaintyDrop_one_some (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) {A : CertifiedRuleApplication S}
    (hchoose : σ.choose S = some A) :
    σ.runSolutionUncertaintyDrop S 1 = A.solutionUncertaintyDrop := by
  simp [runSolutionUncertaintyDrop, run_one_some σ S hchoose,
    CertifiedRuleApplication.solutionUncertaintyDrop, SearchMove.solutionUncertaintyDrop]

/-- Run-level geometric information-capacity contract for fully accounted strategies. -/
def GeometricRunCapacityBound (weights : SearchCostVector)
    (σ : AccountedSearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Prop :=
  σ.runFrontierUncertaintyDrop S fuel ≤
    bitsPerCost * (σ.weightedRunCost weights S fuel + 1)

/-- Candidate-solution information-capacity contract for fully accounted strategies. -/
def SolutionRunCapacityBound (weights : SearchCostVector)
    (σ : AccountedSearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Prop :=
  σ.runSolutionUncertaintyDrop S fuel ≤
    bitsPerCost * (σ.weightedRunCost weights S fuel + 1)

theorem runFrontierUncertaintyDrop_of_empty_frontier (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = []) :
    σ.runFrontierUncertaintyDrop S fuel = S.frontierUncertainty := by
  unfold runFrontierUncertaintyDrop SolverState.frontierUncertainty SolverState.frontierMass
  rw [hempty]
  simp

theorem runSolutionUncertaintyDrop_of_empty_frontier (σ : AccountedSearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = []) :
    σ.runSolutionUncertaintyDrop S fuel = S.frontierSolutionUncertainty := by
  unfold runSolutionUncertaintyDrop SolverState.frontierSolutionUncertainty
    SolverState.frontierSolutionMass
  rw [hempty]
  simp

theorem empty_frontier_geometric_information_envelope (weights : SearchCostVector)
    (σ : AccountedSearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = [])
    (hcap : GeometricRunCapacityBound weights σ bitsPerCost S fuel) :
    S.frontierUncertainty ≤
      bitsPerCost * (σ.weightedRunCost weights S fuel + 1) := by
  rw [← runFrontierUncertaintyDrop_of_empty_frontier σ S fuel hempty]
  exact hcap

theorem empty_frontier_solution_information_envelope (weights : SearchCostVector)
    (σ : AccountedSearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = [])
    (hcap : SolutionRunCapacityBound weights σ bitsPerCost S fuel) :
    S.frontierSolutionUncertainty ≤
      bitsPerCost * (σ.weightedRunCost weights S fuel + 1) := by
  rw [← runSolutionUncertaintyDrop_of_empty_frontier σ S fuel hempty]
  exact hcap

theorem initial_empty_frontier_geometric_information_envelope
    (weights : SearchCostVector) (σ : AccountedSearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (Bs : List (Blocker n)) (fuel : Nat)
    (hempty : (σ.run (SolverState.initial Bs) fuel).frontier = [])
    (hcap : GeometricRunCapacityBound weights σ bitsPerCost (SolverState.initial Bs) fuel) :
    uncertaintyBits (2 ^ n) ≤
      bitsPerCost * (σ.weightedRunCost weights (SolverState.initial Bs) fuel + 1) := by
  have h := empty_frontier_geometric_information_envelope weights σ bitsPerCost
    (SolverState.initial Bs) fuel hempty hcap
  simpa [SolverState.frontierUncertainty, SolverState.initial_frontierMass] using h

end AccountedSearchStrategy

/-- A strategy is a policy that may choose one certified move from the current state.
The policy can be deterministic, heuristic, or a placeholder for an implementation; the
returned move itself carries the soundness proof. -/
structure SolverStrategy where
  choose : {n : Nat} → (S : SolverState n) → Option (SolverMove S)

namespace SolverStrategy

/-- Execute a strategy for a bounded amount of fuel. If the strategy returns `none`,
the run stops. -/
def run (σ : SolverStrategy) {n : Nat} : SolverState n → Nat → SolverState n
  | S, 0 => S
  | S, fuel + 1 =>
      match σ.choose S with
      | none => S
      | some move => run σ move.after fuel

/-- Ledger accumulated by a bounded strategy run. This is the operation accountant for
policy-controlled chains. -/
def runCost (σ : SolverStrategy) {n : Nat} : SolverState n → Nat → SearchCostVector
  | _, 0 => SearchCostVector.zero
  | S, fuel + 1 =>
      match σ.choose S with
      | none => SearchCostVector.zero
      | some move => SearchCostVector.add move.cost (runCost σ move.after fuel)

/-- A strategy run preserves the learned-region soundness invariant for any fuel. -/
theorem run_sound (σ : SolverStrategy) {n : Nat} (S : SolverState n) (fuel : Nat)
    (hS : S.Sound) :
    (σ.run S fuel).Sound := by
  induction fuel generalizing S with
  | zero =>
      exact hS
  | succ fuel ih =>
      unfold run
      cases hmove : σ.choose S with
      | none =>
          exact hS
      | some move =>
          exact ih move.after (move.sound hS)

end SolverStrategy

/-- A search strategy chooses moves that preserve the full search invariant. -/
structure SearchStrategy where
  choose : {n : Nat} → (S : SolverState n) → Option (SearchMove S)

/-- Forget the accountant layer by pushing each accounted total into the ordinary move's cost
column. This lets existing search-strategy theorems consume accounted policies. -/
def AccountedSearchStrategy.toSearchStrategy (σ : AccountedSearchStrategy) : SearchStrategy where
  choose := fun S =>
    match σ.choose S with
    | none => none
    | some A => some A.toSearchMove

namespace SearchStrategy

def run (σ : SearchStrategy) {n : Nat} : SolverState n → Nat → SolverState n
  | S, 0 => S
  | S, fuel + 1 =>
      match σ.choose S with
      | none => S
      | some move => run σ move.after fuel

def runCost (σ : SearchStrategy) {n : Nat} : SolverState n → Nat → SearchCostVector
  | _, 0 => SearchCostVector.zero
  | S, fuel + 1 =>
      match σ.choose S with
      | none => SearchCostVector.zero
      | some move => SearchCostVector.add move.cost (runCost σ move.after fuel)

/-- Scalar weighted operation cost of a bounded search run. -/
def weightedRunCost (σ : SearchStrategy) (weights : SearchCostVector)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Nat :=
  SearchCostVector.weighted weights (σ.runCost S fuel)

/-- Geometric frontier mass removed by a bounded run. -/
def runFrontierMassDrop (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Nat :=
  S.frontierMass - (σ.run S fuel).frontierMass

/-- Geometric frontier uncertainty removed by a bounded run. -/
def runFrontierUncertaintyDrop (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Nat :=
  S.frontierUncertainty - (σ.run S fuel).frontierUncertainty

/-- Candidate-solution uncertainty removed by a bounded run. -/
def runSolutionUncertaintyDrop (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Nat :=
  S.frontierSolutionUncertainty - (σ.run S fuel).frontierSolutionUncertainty

@[simp] theorem runCost_zero (σ : SearchStrategy) {n : Nat} (S : SolverState n) :
    σ.runCost S 0 = SearchCostVector.zero := rfl

@[simp] theorem weightedRunCost_zero (σ : SearchStrategy) (weights : SearchCostVector)
    {n : Nat} (S : SolverState n) :
    σ.weightedRunCost weights S 0 = 0 := by
  simp [weightedRunCost, SearchCostVector.weighted, SearchCostVector.zero]

@[simp] theorem runFrontierMassDrop_zero (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) :
    σ.runFrontierMassDrop S 0 = 0 := by
  simp [runFrontierMassDrop, run]

@[simp] theorem runFrontierUncertaintyDrop_zero (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) :
    σ.runFrontierUncertaintyDrop S 0 = 0 := by
  simp [runFrontierUncertaintyDrop, run]

@[simp] theorem runSolutionUncertaintyDrop_zero (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) :
    σ.runSolutionUncertaintyDrop S 0 = 0 := by
  simp [runSolutionUncertaintyDrop, run]

theorem runCost_succ_none (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hchoose : σ.choose S = none) :
    σ.runCost S (fuel + 1) = SearchCostVector.zero := by
  simp [runCost, hchoose]

theorem runCost_succ_some (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) {move : SearchMove S}
    (hchoose : σ.choose S = some move) :
    σ.runCost S (fuel + 1) =
      SearchCostVector.add move.cost (σ.runCost move.after fuel) := by
  simp [runCost, hchoose]

theorem weightedRunCost_succ_some (σ : SearchStrategy) (weights : SearchCostVector)
    {n : Nat} (S : SolverState n) (fuel : Nat) {move : SearchMove S}
    (hchoose : σ.choose S = some move) :
    σ.weightedRunCost weights S (fuel + 1) =
      move.weightedCost weights + σ.weightedRunCost weights move.after fuel := by
  rw [weightedRunCost, runCost_succ_some σ S fuel hchoose, SearchCostVector.weighted_add]
  rfl

theorem run_one_some (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) {move : SearchMove S}
    (hchoose : σ.choose S = some move) :
    σ.run S 1 = move.after := by
  simp [run, hchoose]

theorem runCost_one_some (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) {move : SearchMove S}
    (hchoose : σ.choose S = some move) :
    σ.runCost S 1 = move.cost := by
  rw [runCost_succ_some σ S 0 hchoose]
  simp

theorem weightedRunCost_one_some (σ : SearchStrategy) (weights : SearchCostVector)
    {n : Nat} (S : SolverState n) {move : SearchMove S}
    (hchoose : σ.choose S = some move) :
    σ.weightedRunCost weights S 1 = move.weightedCost weights := by
  simp [weightedRunCost, runCost_one_some σ S hchoose, SearchMove.weightedCost]

theorem runFrontierUncertaintyDrop_one_some (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) {move : SearchMove S}
    (hchoose : σ.choose S = some move) :
    σ.runFrontierUncertaintyDrop S 1 = move.frontierUncertaintyDrop := by
  simp [runFrontierUncertaintyDrop, run_one_some σ S hchoose,
    SearchMove.frontierUncertaintyDrop]

theorem runSolutionUncertaintyDrop_one_some (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) {move : SearchMove S}
    (hchoose : σ.choose S = some move) :
    σ.runSolutionUncertaintyDrop S 1 = move.solutionUncertaintyDrop := by
  simp [runSolutionUncertaintyDrop, run_one_some σ S hchoose,
    SearchMove.solutionUncertaintyDrop]

/-- Run-level geometric information efficiency comparison, again using cross-multiplication to
avoid rational numbers. -/
def RunGeometricInfoEfficiencyDominates (weights : SearchCostVector)
    (A B : SearchStrategy) {n : Nat} (S : SolverState n) (fuel : Nat) : Prop :=
  B.runFrontierUncertaintyDrop S fuel * (A.weightedRunCost weights S fuel + 1) ≤
    A.runFrontierUncertaintyDrop S fuel * (B.weightedRunCost weights S fuel + 1)

/-- Run-level candidate-solution information efficiency comparison. -/
def RunSolutionInfoEfficiencyDominates (weights : SearchCostVector)
    (A B : SearchStrategy) {n : Nat} (S : SolverState n) (fuel : Nat) : Prop :=
  B.runSolutionUncertaintyDrop S fuel * (A.weightedRunCost weights S fuel + 1) ≤
    A.runSolutionUncertaintyDrop S fuel * (B.weightedRunCost weights S fuel + 1)

theorem RunGeometricInfoEfficiencyDominates.refl (weights : SearchCostVector)
    (σ : SearchStrategy) {n : Nat} (S : SolverState n) (fuel : Nat) :
    RunGeometricInfoEfficiencyDominates weights σ σ S fuel := by
  unfold RunGeometricInfoEfficiencyDominates
  exact le_rfl

theorem RunSolutionInfoEfficiencyDominates.refl (weights : SearchCostVector)
    (σ : SearchStrategy) {n : Nat} (S : SolverState n) (fuel : Nat) :
    RunSolutionInfoEfficiencyDominates weights σ σ S fuel := by
  unfold RunSolutionInfoEfficiencyDominates
  exact le_rfl

/-- A run-level information-capacity contract: this strategy/run extracts at most
`bitsPerCost` geometric uncertainty bits per weighted operation, up to the same `+1`
zero-cost guard used by the local efficiency ratios. This is the formal shape of an
information-theoretic lower-bound assumption. -/
def GeometricRunCapacityBound (weights : SearchCostVector)
    (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Prop :=
  σ.runFrontierUncertaintyDrop S fuel ≤
    bitsPerCost * (σ.weightedRunCost weights S fuel + 1)

/-- Candidate-solution analogue of the run-level information-capacity contract. -/
def SolutionRunCapacityBound (weights : SearchCostVector)
    (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat) : Prop :=
  σ.runSolutionUncertaintyDrop S fuel ≤
    bitsPerCost * (σ.weightedRunCost weights S fuel + 1)

theorem target_geometric_information_envelope (weights : SearchCostVector)
    (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel target : Nat)
    (hneed : target ≤ σ.runFrontierUncertaintyDrop S fuel)
    (hcap : GeometricRunCapacityBound weights σ bitsPerCost S fuel) :
    target ≤ bitsPerCost * (σ.weightedRunCost weights S fuel + 1) :=
  hneed.trans hcap

theorem target_solution_information_envelope (weights : SearchCostVector)
    (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel target : Nat)
    (hneed : target ≤ σ.runSolutionUncertaintyDrop S fuel)
    (hcap : SolutionRunCapacityBound weights σ bitsPerCost S fuel) :
    target ≤ bitsPerCost * (σ.weightedRunCost weights S fuel + 1) :=
  hneed.trans hcap

theorem runFrontierUncertaintyDrop_of_empty_frontier (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = []) :
    σ.runFrontierUncertaintyDrop S fuel = S.frontierUncertainty := by
  unfold runFrontierUncertaintyDrop SolverState.frontierUncertainty SolverState.frontierMass
  rw [hempty]
  simp

theorem runSolutionUncertaintyDrop_of_empty_frontier (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = []) :
    σ.runSolutionUncertaintyDrop S fuel = S.frontierSolutionUncertainty := by
  unfold runSolutionUncertaintyDrop SolverState.frontierSolutionUncertainty
    SolverState.frontierSolutionMass
  rw [hempty]
  simp

/-- If an UNSAT run empties the frontier and the rule class has a geometric information
capacity bound, then the initial frontier uncertainty must fit inside the run's weighted
operation budget. This is the accounting form of "you must pay enough operations to remove
enough hypercube uncertainty." -/
theorem empty_frontier_geometric_information_envelope (weights : SearchCostVector)
    (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = [])
    (hcap : GeometricRunCapacityBound weights σ bitsPerCost S fuel) :
    S.frontierUncertainty ≤
      bitsPerCost * (σ.weightedRunCost weights S fuel + 1) := by
  apply target_geometric_information_envelope weights σ bitsPerCost S fuel
  · rw [runFrontierUncertaintyDrop_of_empty_frontier σ S fuel hempty]
  · exact hcap

/-- Candidate-solution version of the empty-frontier information envelope. -/
theorem empty_frontier_solution_information_envelope (weights : SearchCostVector)
    (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = [])
    (hcap : SolutionRunCapacityBound weights σ bitsPerCost S fuel) :
    S.frontierSolutionUncertainty ≤
      bitsPerCost * (σ.weightedRunCost weights S fuel + 1) := by
  apply target_solution_information_envelope weights σ bitsPerCost S fuel
  · rw [runSolutionUncertaintyDrop_of_empty_frontier σ S fuel hempty]
  · exact hcap

/-- Initial-state geometric envelope: a full UNSAT refutation starting from the whole cube must
account for the uncertainty of all `2^n` vertices under the chosen information-capacity model. -/
theorem initial_empty_frontier_geometric_information_envelope
    (weights : SearchCostVector) (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (Bs : List (Blocker n)) (fuel : Nat)
    (hempty : (σ.run (SolverState.initial Bs) fuel).frontier = [])
    (hcap : GeometricRunCapacityBound weights σ bitsPerCost (SolverState.initial Bs) fuel) :
    uncertaintyBits (2 ^ n) ≤
      bitsPerCost * (σ.weightedRunCost weights (SolverState.initial Bs) fuel + 1) := by
  have h := empty_frontier_geometric_information_envelope weights σ bitsPerCost
    (SolverState.initial Bs) fuel hempty hcap
  simpa [SolverState.frontierUncertainty, SolverState.initial_frontierMass] using h

/-- Initial-state candidate-solution envelope. This one is useful for SAT-side bounded workers:
if a run eliminates the whole frontier, it must have accounted for the original candidate
solution mass. For UNSAT instances this mass is zero; for incomplete/unsafe analyses it exposes
exactly what would have to be justified. -/
theorem initial_empty_frontier_solution_information_envelope
    (weights : SearchCostVector) (σ : SearchStrategy) (bitsPerCost : Nat)
    {n : Nat} (Bs : List (Blocker n)) (fuel : Nat)
    (hempty : (σ.run (SolverState.initial Bs) fuel).frontier = [])
    (hcap : SolutionRunCapacityBound weights σ bitsPerCost (SolverState.initial Bs) fuel) :
    uncertaintyBits (solutionCount Bs) ≤
      bitsPerCost * (σ.weightedRunCost weights (SolverState.initial Bs) fuel + 1) := by
  have h := empty_frontier_solution_information_envelope weights σ bitsPerCost
    (SolverState.initial Bs) fuel hempty hcap
  simpa [SolverState.frontierSolutionUncertainty, SolverState.initial_frontierSolutionMass] using h

/-- A class of concrete search strategies. This is separate from `SolverClass`, which classifies
syntax trees; this classifies already-interpreted certified policies. -/
abbrev SearchStrategyClass := SearchStrategy → Prop

/-- Uniform geometric information-capacity bound for every strategy in a concrete class. -/
def SearchStrategyClassGeometricCapacityBound (weights : SearchCostVector)
    (Class : SearchStrategyClass) (bitsPerCost : Nat) : Prop :=
  ∀ σ : SearchStrategy, Class σ → ∀ {n : Nat} (S : SolverState n) (fuel : Nat),
    GeometricRunCapacityBound weights σ bitsPerCost S fuel

/-- Uniform candidate-solution information-capacity bound for every strategy in a concrete class. -/
def SearchStrategyClassSolutionCapacityBound (weights : SearchCostVector)
    (Class : SearchStrategyClass) (bitsPerCost : Nat) : Prop :=
  ∀ σ : SearchStrategy, Class σ → ∀ {n : Nat} (S : SolverState n) (fuel : Nat),
    SolutionRunCapacityBound weights σ bitsPerCost S fuel

theorem SearchStrategyClassGeometricCapacityBound.empty_frontier_envelope
    {weights : SearchCostVector} {Class : SearchStrategyClass} {bitsPerCost : Nat}
    (hClass : SearchStrategyClassGeometricCapacityBound weights Class bitsPerCost)
    {σ : SearchStrategy} (hσ : Class σ)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = []) :
    S.frontierUncertainty ≤
      bitsPerCost * (σ.weightedRunCost weights S fuel + 1) :=
  empty_frontier_geometric_information_envelope weights σ bitsPerCost S fuel hempty
    (hClass σ hσ S fuel)

theorem SearchStrategyClassSolutionCapacityBound.empty_frontier_envelope
    {weights : SearchCostVector} {Class : SearchStrategyClass} {bitsPerCost : Nat}
    (hClass : SearchStrategyClassSolutionCapacityBound weights Class bitsPerCost)
    {σ : SearchStrategy} (hσ : Class σ)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (hempty : (σ.run S fuel).frontier = []) :
    S.frontierSolutionUncertainty ≤
      bitsPerCost * (σ.weightedRunCost weights S fuel + 1) :=
  empty_frontier_solution_information_envelope weights σ bitsPerCost S fuel hempty
    (hClass σ hσ S fuel)

theorem run_preserves_searchInvariant (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) (h : S.SearchInvariant) :
    (σ.run S fuel).SearchInvariant := by
  induction fuel generalizing S with
  | zero =>
      exact h
  | succ fuel ih =>
      unfold run
      cases hmove : σ.choose S with
      | none =>
          exact h
      | some move =>
          exact ih move.after (move.preserves h)

theorem run_same_original (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat) :
    (σ.run S fuel).original = S.original := by
  induction fuel generalizing S with
  | zero =>
      rfl
  | succ fuel ih =>
      unfold run
      cases hmove : σ.choose S with
      | none =>
          rfl
      | some move =>
          exact (ih move.after).trans move.same_original

theorem coverUNSATList_of_empty_frontier_after_run (σ : SearchStrategy)
    {n : Nat} (S : SolverState n) (fuel : Nat)
    (h : S.SearchInvariant) (hempty : (σ.run S fuel).frontier = []) :
    CoverUNSATList S.original := by
  have hrun := σ.run_preserves_searchInvariant S fuel h
  have hunsat : CoverUNSATList (σ.run S fuel).original :=
    SolverState.coverUNSATList_of_empty_frontier hrun.2 hempty
  simpa [σ.run_same_original S fuel] using hunsat

theorem coverUNSATList_of_initial_empty_frontier (σ : SearchStrategy)
    {n : Nat} (Bs : List (Blocker n)) (fuel : Nat)
    (hempty : (σ.run (SolverState.initial Bs) fuel).frontier = []) :
    CoverUNSATList Bs := by
  simpa [SolverState.initial] using
    σ.coverUNSATList_of_empty_frontier_after_run
      (SolverState.initial Bs) fuel (SolverState.initial_searchInvariant Bs) hempty

end SearchStrategy

/-! ### §E0.2 Information-optimal strategy contracts -/

/-- A branch decision certificate: the chosen coordinate is free and information-optimal
for a candidate generator on the selected face. -/
structure InfoBranchCertificate {n : Nat} (C : BranchCandidates n) (P : Pattern n) where
  coord : Fin n
  free : P.Free coord
  optimal : InfoOptimalBranchChoice C P coord free

theorem InfoBranchCertificate.best {n : Nat} {C : BranchCandidates n} {P : Pattern n}
    (cert : InfoBranchCertificate C P) {j : Fin n}
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstMass cert.coord cert.free ≤ P.splitWorstMass j hfreej :=
  cert.optimal.best hj hfreej

theorem InfoBranchCertificate.best_uncertainty {n : Nat} {C : BranchCandidates n}
    {P : Pattern n} (cert : InfoBranchCertificate C P) {j : Fin n}
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstUncertainty cert.coord cert.free ≤ P.splitWorstUncertainty j hfreej :=
  cert.optimal.best_uncertainty hj hfreej

/-- Even the chosen information-optimal binary branch is constrained by the one-bit split
lower bound: its worst child carries enough mass to account for the parent. -/
theorem InfoBranchCertificate.parent_mass_le_two_mul_worst {n : Nat}
    {C : BranchCandidates n} {P : Pattern n} (cert : InfoBranchCertificate C P) :
    P.mass ≤ 2 * P.splitWorstMass cert.coord cert.free :=
  P.mass_le_two_mul_splitWorstMass cert.coord cert.free

/-- A state-level branch certificate: the frontier head is the selected face and the
coordinate is information-optimal for that head. -/
structure InfoFrontierBranchCertificate {n : Nat} (C : BranchCandidates n)
    (S : SolverState n) where
  head : Pattern n
  rest : List (Pattern n)
  frontier_eq : S.frontier = head :: rest
  branch : InfoBranchCertificate C head

/-- Execute an information-optimal branch certificate as a certified search move. -/
def InfoFrontierBranchCertificate.toSearchMove {n : Nat} {C : BranchCandidates n}
    {S : SolverState n} (cert : InfoFrontierBranchCertificate C S) : SearchMove S :=
  SearchMove.branchFrontierHead S cert.head cert.rest cert.branch.coord cert.branch.free
    cert.frontier_eq

/-- The chosen coordinate in an information branch certificate is no worse than any
candidate free coordinate, in worst-child mass. -/
theorem InfoFrontierBranchCertificate.best {n : Nat} {C : BranchCandidates n}
    {S : SolverState n} (cert : InfoFrontierBranchCertificate C S)
    {j : Fin n} (hj : j ∈ C cert.head) (hfreej : cert.head.Free j) :
    cert.head.splitWorstMass cert.branch.coord cert.branch.free ≤
      cert.head.splitWorstMass j hfreej :=
  cert.branch.best hj hfreej

/-- Same optimality statement after translating mass into base-2 uncertainty. -/
theorem InfoFrontierBranchCertificate.best_uncertainty {n : Nat} {C : BranchCandidates n}
    {S : SolverState n} (cert : InfoFrontierBranchCertificate C S)
    {j : Fin n} (hj : j ∈ C cert.head) (hfreej : cert.head.Free j) :
    cert.head.splitWorstUncertainty cert.branch.coord cert.branch.free ≤
      cert.head.splitWorstUncertainty j hfreej :=
  cert.branch.best_uncertainty hj hfreej

theorem InfoFrontierBranchCertificate.parent_mass_le_two_mul_worst {n : Nat}
    {C : BranchCandidates n} {S : SolverState n}
    (cert : InfoFrontierBranchCertificate C S) :
    cert.head.mass ≤ 2 * cert.head.splitWorstMass cert.branch.coord cert.branch.free :=
  cert.branch.parent_mass_le_two_mul_worst

/-- Blocker-aware branch decision certificate: the chosen coordinate is optimal for shrinking
the still-uncovered candidate set under the original blockers. -/
structure InfoSolutionBranchCertificate {n : Nat} (Bs : List (Blocker n))
    (C : BranchCandidates n) (P : Pattern n) where
  coord : Fin n
  free : P.Free coord
  optimal : InfoOptimalSolutionBranchChoice Bs C P coord free

theorem InfoSolutionBranchCertificate.best {n : Nat} {Bs : List (Blocker n)}
    {C : BranchCandidates n} {P : Pattern n}
    (cert : InfoSolutionBranchCertificate Bs C P) {j : Fin n}
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstSolutionMass Bs cert.coord cert.free ≤
      P.splitWorstSolutionMass Bs j hfreej :=
  cert.optimal.best hj hfreej

theorem InfoSolutionBranchCertificate.best_uncertainty {n : Nat} {Bs : List (Blocker n)}
    {C : BranchCandidates n} {P : Pattern n}
    (cert : InfoSolutionBranchCertificate Bs C P) {j : Fin n}
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.splitWorstSolutionUncertainty Bs cert.coord cert.free ≤
      P.splitWorstSolutionUncertainty Bs j hfreej :=
  cert.optimal.best_uncertainty hj hfreej

/-- The blocker-aware optimal branch still obeys the one-bit lower bound on possible
satisfying candidates. -/
theorem InfoSolutionBranchCertificate.parent_solutionMass_le_two_mul_worst {n : Nat}
    {Bs : List (Blocker n)} {C : BranchCandidates n} {P : Pattern n}
    (cert : InfoSolutionBranchCertificate Bs C P) :
    P.solutionMass Bs ≤
      2 * P.splitWorstSolutionMass Bs cert.coord cert.free :=
  P.solutionMass_le_two_mul_splitWorstSolutionMass Bs cert.coord cert.free

theorem InfoSolutionBranchCertificate.maximizes_guaranteedSolutionUncertaintyDrop {n : Nat}
    {Bs : List (Blocker n)} {C : BranchCandidates n} {P : Pattern n}
    (cert : InfoSolutionBranchCertificate Bs C P) {j : Fin n}
    (hj : j ∈ C P) (hfreej : P.Free j) :
    P.guaranteedSolutionUncertaintyDrop Bs j hfreej ≤
      P.guaranteedSolutionUncertaintyDrop Bs cert.coord cert.free :=
  cert.optimal.maximizes_guaranteedSolutionUncertaintyDrop hj hfreej

/-- State-level blocker-aware branch certificate. This is the information-theoretic CDCL
contract: the branch is certified optimal for the current original instance and frontier head. -/
structure InfoSolutionFrontierBranchCertificate {n : Nat} (C : BranchCandidates n)
    (S : SolverState n) where
  head : Pattern n
  rest : List (Pattern n)
  frontier_eq : S.frontier = head :: rest
  branch : InfoSolutionBranchCertificate S.original C head

def InfoSolutionFrontierBranchCertificate.toSearchMove {n : Nat} {C : BranchCandidates n}
    {S : SolverState n} (cert : InfoSolutionFrontierBranchCertificate C S) : SearchMove S :=
  SearchMove.branchFrontierHead S cert.head cert.rest cert.branch.coord cert.branch.free
    cert.frontier_eq

theorem InfoSolutionFrontierBranchCertificate.best_uncertainty {n : Nat}
    {C : BranchCandidates n} {S : SolverState n}
    (cert : InfoSolutionFrontierBranchCertificate C S) {j : Fin n}
    (hj : j ∈ C cert.head) (hfreej : cert.head.Free j) :
    cert.head.splitWorstSolutionUncertainty S.original cert.branch.coord cert.branch.free ≤
      cert.head.splitWorstSolutionUncertainty S.original j hfreej :=
  cert.branch.best_uncertainty hj hfreej

theorem InfoSolutionFrontierBranchCertificate.parent_solutionMass_le_two_mul_worst {n : Nat}
    {C : BranchCandidates n} {S : SolverState n}
    (cert : InfoSolutionFrontierBranchCertificate C S) :
    cert.head.solutionMass S.original ≤
      2 * cert.head.splitWorstSolutionMass S.original cert.branch.coord cert.branch.free :=
  cert.branch.parent_solutionMass_le_two_mul_worst

/-- A search strategy is locally information-greedy when every branch move it emits is
accompanied by an information-optimal branch certificate for the current frontier head.
Other move kinds may be conflict, propagation, restart, deletion, or sharing. -/
def LocallyInformationGreedy (C : {n : Nat} → BranchCandidates n)
    (σ : SearchStrategy) : Prop :=
  ∀ {n : Nat} (S : SolverState n) (move : SearchMove S),
    σ.choose S = some move → move.kind = SolverMoveKind.branch →
      ∃ cert : InfoFrontierBranchCertificate (C (n := n)) S,
        move.after = cert.toSearchMove.after

/-- Stronger information-greedy contract: every branch must be certified optimal for reducing
the blocker-aware candidate-solution uncertainty of the current original instance. -/
def LocallySolutionInformationGreedy (C : {n : Nat} → BranchCandidates n)
    (σ : SearchStrategy) : Prop :=
  ∀ {n : Nat} (S : SolverState n) (move : SearchMove S),
    σ.choose S = some move → move.kind = SolverMoveKind.branch →
      ∃ cert : InfoSolutionFrontierBranchCertificate (C (n := n)) S,
        move.after = cert.toSearchMove.after

/-! ### §E0.3 Cost-aware information-optimal strategy contracts -/

/-- The DPLL-style branch menu containing every legal binary split of the current frontier
head along a free coordinate. If there is no frontier head, there are no branch moves. -/
noncomputable def allFreeBranchMoves {n : Nat} (S : SolverState n) :
    List (SearchMove S) := by
  classical
  cases hfrontier : S.frontier with
  | nil =>
      exact []
  | cons P Rest =>
      exact (((freeCoordinateCandidates (n := n)) P).attach.toList.map fun i =>
        SearchMove.branchFrontierHead S P Rest i.1
          ((mem_freeCoordinateCandidates P i.1).mp i.2) hfrontier)

/-- The standard hypercube-DPLL menu: split the selected face on any still-free coordinate. -/
noncomputable def freeBranchMoveMenu : SearchMoveMenu :=
  fun {n} S => allFreeBranchMoves (n := n) S

/-- Baseline accounted DPLL menu: every legal free-coordinate branch, charging only the move's
own decision cost and no recognizer overhead. -/
noncomputable def freeBranchAccountedMenuNoOverhead : InformationRuleMenu :=
  accountSearchMoveMenuNoOverhead freeBranchMoveMenu

/-- A simple overhead model for branch-menu scanning: trying a branch candidate pays one memory
cell for face/coordinate bookkeeping and one frontier cell for carrying the produced children. -/
def branchScanBreakdown {n : Nat} (S : SolverState n) (_move : SearchMove S) :
    RuleCostBreakdown :=
  { RuleCostBreakdown.zero with
    matchCost := SearchCostVector.memoryCellUnit
    frontierCost := SearchCostVector.memoryCellUnit }

/-- Accounted DPLL menu with a minimal scan/frontier overhead charged to every branch candidate. -/
noncomputable def freeBranchAccountedMenuWithScan : InformationRuleMenu :=
  accountSearchMoveMenu freeBranchMoveMenu branchScanBreakdown

/-- A strategy is locally geometrically cost-optimal when its chosen move has maximal
frontier-uncertainty drop per weighted operation cost among the current menu. -/
def LocallyGeometricInfoCostOptimal (weights : SearchCostVector)
    (menu : SearchMoveMenu) (σ : SearchStrategy) : Prop :=
  ∀ {n : Nat} (S : SolverState n) (move : SearchMove S),
    σ.choose S = some move →
      SearchMove.GeometricInfoOptimalAmong weights (menu S) move

/-- A strategy is locally solution-information cost-optimal when its chosen move has maximal
candidate-solution uncertainty drop per weighted operation cost among the current menu. -/
def LocallySolutionInfoCostOptimal (weights : SearchCostVector)
    (menu : SearchMoveMenu) (σ : SearchStrategy) : Prop :=
  ∀ {n : Nat} (S : SolverState n) (move : SearchMove S),
    σ.choose S = some move →
      SearchMove.SolutionInfoOptimalAmong weights (menu S) move

theorem LocallyGeometricInfoCostOptimal.best {weights : SearchCostVector}
    {menu : SearchMoveMenu} {σ : SearchStrategy}
    (hσ : LocallyGeometricInfoCostOptimal weights menu σ)
    {n : Nat} (S : SolverState n) {move other : SearchMove S}
    (hchoose : σ.choose S = some move) (hother : other ∈ menu S) :
    SearchMove.GeometricInfoEfficiencyDominates weights move other :=
  (hσ S move hchoose).best hother

theorem LocallyGeometricInfoCostOptimal.best_cross_mul {weights : SearchCostVector}
    {menu : SearchMoveMenu} {σ : SearchStrategy}
    (hσ : LocallyGeometricInfoCostOptimal weights menu σ)
    {n : Nat} (S : SolverState n) {move other : SearchMove S}
    (hchoose : σ.choose S = some move) (hother : other ∈ menu S) :
    other.frontierUncertaintyDrop * (move.weightedCost weights + 1) ≤
      move.frontierUncertaintyDrop * (other.weightedCost weights + 1) :=
  (hσ S move hchoose).best_cross_mul hother

/-- Local menu optimality is exactly one-step run optimality after translating the chosen
move into its one-step run ledger. -/
theorem LocallyGeometricInfoCostOptimal.run_one_best_cross_mul {weights : SearchCostVector}
    {menu : SearchMoveMenu} {σ : SearchStrategy}
    (hσ : LocallyGeometricInfoCostOptimal weights menu σ)
    {n : Nat} (S : SolverState n) {move other : SearchMove S}
    (hchoose : σ.choose S = some move) (hother : other ∈ menu S) :
    other.frontierUncertaintyDrop * (σ.weightedRunCost weights S 1 + 1) ≤
      σ.runFrontierUncertaintyDrop S 1 * (other.weightedCost weights + 1) := by
  rw [SearchStrategy.weightedRunCost_one_some σ weights S hchoose,
    SearchStrategy.runFrontierUncertaintyDrop_one_some σ S hchoose]
  exact hσ.best_cross_mul S hchoose hother

theorem LocallySolutionInfoCostOptimal.best {weights : SearchCostVector}
    {menu : SearchMoveMenu} {σ : SearchStrategy}
    (hσ : LocallySolutionInfoCostOptimal weights menu σ)
    {n : Nat} (S : SolverState n) {move other : SearchMove S}
    (hchoose : σ.choose S = some move) (hother : other ∈ menu S) :
    SearchMove.SolutionInfoEfficiencyDominates weights move other :=
  (hσ S move hchoose).best hother

theorem LocallySolutionInfoCostOptimal.best_cross_mul {weights : SearchCostVector}
    {menu : SearchMoveMenu} {σ : SearchStrategy}
    (hσ : LocallySolutionInfoCostOptimal weights menu σ)
    {n : Nat} (S : SolverState n) {move other : SearchMove S}
    (hchoose : σ.choose S = some move) (hother : other ∈ menu S) :
    other.solutionUncertaintyDrop * (move.weightedCost weights + 1) ≤
      move.solutionUncertaintyDrop * (other.weightedCost weights + 1) :=
  (hσ S move hchoose).best_cross_mul hother

/-- Candidate-solution version of the one-step local-to-run optimality bridge. -/
theorem LocallySolutionInfoCostOptimal.run_one_best_cross_mul {weights : SearchCostVector}
    {menu : SearchMoveMenu} {σ : SearchStrategy}
    (hσ : LocallySolutionInfoCostOptimal weights menu σ)
    {n : Nat} (S : SolverState n) {move other : SearchMove S}
    (hchoose : σ.choose S = some move) (hother : other ∈ menu S) :
    other.solutionUncertaintyDrop * (σ.weightedRunCost weights S 1 + 1) ≤
      σ.runSolutionUncertaintyDrop S 1 * (other.weightedCost weights + 1) := by
  rw [SearchStrategy.weightedRunCost_one_some σ weights S hchoose,
    SearchStrategy.runSolutionUncertaintyDrop_one_some σ S hchoose]
  exact hσ.best_cross_mul S hchoose hother

/-- A combined local optimality contract for the research target: the strategy is only called
"best" relative to an explicit move menu and operation weighting, and it optimizes both geometry
and candidate-solution information ledgers. -/
def LocallyInformationCostOptimal (weights : SearchCostVector)
    (menu : SearchMoveMenu) (σ : SearchStrategy) : Prop :=
  LocallyGeometricInfoCostOptimal weights menu σ ∧
    LocallySolutionInfoCostOptimal weights menu σ

/-- A concrete strategy class whose members are locally optimal with respect to an explicit
move menu and weighted operation model. This is the "provably best heuristic" contract at the
local CDCL-decision layer. -/
def LocallyOptimalSearchStrategyClass (weights : SearchCostVector)
    (menu : SearchMoveMenu) : SearchStrategy.SearchStrategyClass :=
  fun σ => LocallyInformationCostOptimal weights menu σ

/-- A concrete strategy class whose members are both locally information-cost optimal and
globally subject to declared geometric and solution-information capacity limits. This is the
formal skeleton for information-theoretic solver lower/upper-bound comparisons. -/
def InformationAccountedSearchStrategyClass (weights : SearchCostVector)
    (menu : SearchMoveMenu) (geometricBitsPerCost solutionBitsPerCost : Nat) :
    SearchStrategy.SearchStrategyClass :=
  fun σ =>
    LocallyInformationCostOptimal weights menu σ ∧
      (∀ {n : Nat} (S : SolverState n) (fuel : Nat),
        SearchStrategy.GeometricRunCapacityBound weights σ geometricBitsPerCost S fuel) ∧
      (∀ {n : Nat} (S : SolverState n) (fuel : Nat),
        SearchStrategy.SolutionRunCapacityBound weights σ solutionBitsPerCost S fuel)

theorem InformationAccountedSearchStrategyClass.locallyOptimal
    {weights : SearchCostVector} {menu : SearchMoveMenu}
    {geometricBitsPerCost solutionBitsPerCost : Nat}
    {σ : SearchStrategy}
    (hσ : InformationAccountedSearchStrategyClass weights menu
      geometricBitsPerCost solutionBitsPerCost σ) :
    LocallyInformationCostOptimal weights menu σ :=
  hσ.1

theorem InformationAccountedSearchStrategyClass.geometricCapacity
    {weights : SearchCostVector} {menu : SearchMoveMenu}
    {geometricBitsPerCost solutionBitsPerCost : Nat}
    {σ : SearchStrategy}
    (hσ : InformationAccountedSearchStrategyClass weights menu
      geometricBitsPerCost solutionBitsPerCost σ)
    {n : Nat} (S : SolverState n) (fuel : Nat) :
    SearchStrategy.GeometricRunCapacityBound weights σ geometricBitsPerCost S fuel :=
  hσ.2.1 S fuel

theorem InformationAccountedSearchStrategyClass.solutionCapacity
    {weights : SearchCostVector} {menu : SearchMoveMenu}
    {geometricBitsPerCost solutionBitsPerCost : Nat}
    {σ : SearchStrategy}
    (hσ : InformationAccountedSearchStrategyClass weights menu
      geometricBitsPerCost solutionBitsPerCost σ)
    {n : Nat} (S : SolverState n) (fuel : Nat) :
    SearchStrategy.SolutionRunCapacityBound weights σ solutionBitsPerCost S fuel :=
  hσ.2.2 S fuel

/-- A complete local information-theoretic model for a hypercube CDCL-style solver.  The model
spells out the legal move menu, the operation weights, and the declared information bandwidth of
the strategy.  "Provably best" always means "best inside one of these explicit models." -/
structure InformationTheoreticCDCLModel where
  weights : SearchCostVector
  menu : SearchMoveMenu
  geometricBitsPerCost : Nat
  solutionBitsPerCost : Nat

namespace InformationTheoreticCDCLModel

/-- Strategies admissible in an information-theoretic CDCL model are locally optimal over the
model's menu and obey both geometric and candidate-solution information-capacity ledgers. -/
def Admissible (M : InformationTheoreticCDCLModel) (σ : SearchStrategy) : Prop :=
  InformationAccountedSearchStrategyClass M.weights M.menu
    M.geometricBitsPerCost M.solutionBitsPerCost σ

/-- The local best-move contract extracted from admissibility. -/
def LocallyBest (M : InformationTheoreticCDCLModel) (σ : SearchStrategy) : Prop :=
  LocallyInformationCostOptimal M.weights M.menu σ

/-- Geometric information bandwidth declared by the model. -/
def GeometricBandwidth (M : InformationTheoreticCDCLModel) : Nat :=
  M.geometricBitsPerCost

/-- Candidate-solution information bandwidth declared by the model. -/
def SolutionBandwidth (M : InformationTheoreticCDCLModel) : Nat :=
  M.solutionBitsPerCost

theorem admissible_locallyBest {M : InformationTheoreticCDCLModel} {σ : SearchStrategy}
    (hσ : M.Admissible σ) :
    M.LocallyBest σ :=
  hσ.1

theorem admissible_geometricCapacity {M : InformationTheoreticCDCLModel} {σ : SearchStrategy}
    (hσ : M.Admissible σ) {n : Nat} (S : SolverState n) (fuel : Nat) :
    SearchStrategy.GeometricRunCapacityBound M.weights σ M.GeometricBandwidth S fuel :=
  hσ.2.1 S fuel

theorem admissible_solutionCapacity {M : InformationTheoreticCDCLModel} {σ : SearchStrategy}
    (hσ : M.Admissible σ) {n : Nat} (S : SolverState n) (fuel : Nat) :
    SearchStrategy.SolutionRunCapacityBound M.weights σ M.SolutionBandwidth S fuel :=
  hσ.2.2 S fuel

/-- A strategy is information-theoretically unbeatable in a model when every admissible
competitor is bounded by the same declared information envelopes.  This is intentionally a
model-relative definition; outside the model, new operations or observables can change the game. -/
def UnbeatableEnvelope (M : InformationTheoreticCDCLModel) (σ : SearchStrategy) : Prop :=
  M.Admissible σ ∧
    (∀ τ : SearchStrategy, M.Admissible τ →
      ∀ {n : Nat} (S : SolverState n) (fuel : Nat),
        τ.runFrontierUncertaintyDrop S fuel ≤
          M.GeometricBandwidth * (τ.weightedRunCost M.weights S fuel + 1)) ∧
    (∀ τ : SearchStrategy, M.Admissible τ →
      ∀ {n : Nat} (S : SolverState n) (fuel : Nat),
        τ.runSolutionUncertaintyDrop S fuel ≤
          M.SolutionBandwidth * (τ.weightedRunCost M.weights S fuel + 1))

theorem unbeatable_admissible {M : InformationTheoreticCDCLModel} {σ : SearchStrategy}
    (hσ : M.UnbeatableEnvelope σ) :
    M.Admissible σ :=
  hσ.1

theorem unbeatable_geometricEnvelope {M : InformationTheoreticCDCLModel} {σ τ : SearchStrategy}
    (hσ : M.UnbeatableEnvelope σ) (hτ : M.Admissible τ)
    {n : Nat} (S : SolverState n) (fuel : Nat) :
    τ.runFrontierUncertaintyDrop S fuel ≤
      M.GeometricBandwidth * (τ.weightedRunCost M.weights S fuel + 1) :=
  hσ.2.1 τ hτ S fuel

theorem unbeatable_solutionEnvelope {M : InformationTheoreticCDCLModel} {σ τ : SearchStrategy}
    (hσ : M.UnbeatableEnvelope σ) (hτ : M.Admissible τ)
    {n : Nat} (S : SolverState n) (fuel : Nat) :
    τ.runSolutionUncertaintyDrop S fuel ≤
      M.SolutionBandwidth * (τ.weightedRunCost M.weights S fuel + 1) :=
  hσ.2.2 τ hτ S fuel

end InformationTheoreticCDCLModel

/-- A strategy is locally best among all free-coordinate branches when it optimizes the
cost-normalized geometric information score over the free-branch menu. -/
def LocallyBestFreeBranchingByGeometry (weights : SearchCostVector)
    (σ : SearchStrategy) : Prop :=
  LocallyGeometricInfoCostOptimal weights freeBranchMoveMenu σ

/-- A strategy is locally best among all free-coordinate branches when it optimizes the
cost-normalized candidate-solution information score over the free-branch menu. -/
def LocallyBestFreeBranchingBySolutions (weights : SearchCostVector)
    (σ : SearchStrategy) : Prop :=
  LocallySolutionInfoCostOptimal weights freeBranchMoveMenu σ

/-- A SAT certificate is an uncovered vertex. -/
structure SATCertificate (I : CoverInput) where
  v : Vertex I.n
  sound : IsUncovered I.blockers v

/-- An UNSAT certificate in the bad-region calculus: the whole cube is bad. -/
structure UNSATCertificate (I : CoverInput) where
  emptyBad : BadRegionFor I.blockers (Pattern.empty I.n)

theorem SATCertificate.coverSAT {I : CoverInput} (cert : SATCertificate I) :
    CoverSAT I :=
  ⟨cert.v, cert.sound⟩

theorem UNSATCertificate.coverUNSAT {I : CoverInput} (cert : UNSATCertificate I) :
    CoverUNSAT I := by
  unfold CoverUNSAT CoverSAT
  intro hsat
  obtain ⟨v, hv⟩ := hsat
  exact cert.emptyBad v (Pattern.empty_covers v) hv

/-- Certified solver answers. `unknown` is allowed for bounded runs; `sat` and `unsat`
carry checkable certificates. -/
inductive SolverAnswer (I : CoverInput) where
  | unknown : SolverAnswer I
  | sat : SATCertificate I → SolverAnswer I
  | unsat : UNSATCertificate I → SolverAnswer I

namespace SolverAnswer

/-- Semantic meaning of a certified answer. Unknown answers make no claim. -/
def Sound {I : CoverInput} : SolverAnswer I → Prop
  | unknown => True
  | sat _ => CoverSAT I
  | unsat _ => CoverUNSAT I

theorem sat_sound {I : CoverInput} (cert : SATCertificate I) :
    (SolverAnswer.sat cert).Sound :=
  cert.coverSAT

theorem unsat_sound {I : CoverInput} (cert : UNSATCertificate I) :
    (SolverAnswer.unsat cert).Sound :=
  cert.coverUNSAT

end SolverAnswer

/-- A certified bounded solver interface: every non-unknown result must carry its proof. -/
structure CertifiedSolver where
  run : (I : CoverInput) → Nat → SolverAnswer I

/-- A tiny syntax tree for strategy families. This does not execute by itself; it is the
enumerable grammar used to classify and search over solver policies. -/
inductive StrategyExpr where
  | bruteForce
  | dpll
  | cdcl
  | localSearch
  | ringDeficit
  | handoff
  | restartEvery : Nat → StrategyExpr
  | sequence : StrategyExpr → StrategyExpr → StrategyExpr
  | portfolio : List StrategyExpr → StrategyExpr
  | boundedSearch : Nat → StrategyExpr → StrategyExpr
  deriving Repr

/-- Syntactic size of a strategy expression. -/
def StrategyExpr.size : StrategyExpr → Nat
  | .bruteForce => 1
  | .dpll => 1
  | .cdcl => 1
  | .localSearch => 1
  | .ringDeficit => 1
  | .handoff => 1
  | .restartEvery _ => 1
  | .sequence a b => a.size + b.size + 1
  | .portfolio ss => ss.foldl (fun acc s => acc + s.size) 1
  | .boundedSearch _ s => s.size + 1

/-- A strategy expression belongs to a bounded grammar slice when its syntax tree has
size at most `s`. -/
def StrategyExpr.BoundedBy (S : StrategyExpr) (s : Nat) : Prop :=
  S.size ≤ s

/-- Every strategy expression has positive syntax size. -/
theorem StrategyExpr.size_pos (S : StrategyExpr) : 0 < S.size := by
  cases S with
  | portfolio ss =>
      unfold StrategyExpr.size
      have hfold : ∀ acc : Nat, 0 < acc →
          0 < ss.foldl (fun acc s => acc + s.size) acc := by
        induction ss with
        | nil =>
            intro acc hacc
            simpa using hacc
        | cons s ss ih =>
            intro acc hacc
            rw [List.foldl_cons]
            apply ih
            omega
      exact hfold 1 (by omega)
  | _ =>
      simp [StrategyExpr.size]

/-- Abstract performance profile for a strategy or architecture. `stepsOn` is concrete
per-input cost; `worstCaseSteps` is the size-indexed envelope used for asymptotics. -/
structure StrategyPerformance where
  stepsOn : CoverInput → Nat
  worstCaseSteps : Nat → Nat
  worst_bound : ∀ I : CoverInput, stepsOn I ≤ worstCaseSteps I.size

/-- A strategy/performance profile solves an input within a given step budget. -/
def SolvesWithin (P : StrategyPerformance) (I : CoverInput) (t : Nat) : Prop :=
  P.stepsOn I ≤ t

/-- Worst-case big-O comparison of two strategy profiles. -/
def StrategyAsymptoticallyDominates (A B : StrategyPerformance) : Prop :=
  BigO A.worstCaseSteps B.worstCaseSteps

/-- Pointwise dominance: `A` never spends more concrete steps than `B`. -/
def StrategyDominates (A B : StrategyPerformance) : Prop :=
  ∀ I : CoverInput, A.stepsOn I ≤ B.stepsOn I

/-- Pointwise strategy dominance is reflexive. -/
theorem StrategyDominates.refl (A : StrategyPerformance) :
    StrategyDominates A A := by
  intro I
  exact le_rfl

/-- Pointwise strategy dominance is transitive. -/
theorem StrategyDominates.trans {A B C : StrategyPerformance}
    (hAB : StrategyDominates A B) (hBC : StrategyDominates B C) :
    StrategyDominates A C := by
  intro I
  exact (hAB I).trans (hBC I)

/-- If `A` pointwise dominates `B`, then any budget that suffices for `B` also suffices
for `A`. -/
theorem StrategyDominates.solvesWithin {A B : StrategyPerformance}
    (hAB : StrategyDominates A B) {I : CoverInput} {t : Nat}
    (hB : SolvesWithin B I t) :
    SolvesWithin A I t :=
  (hAB I).trans hB

/-- Asymptotic dominance is reflexive. -/
theorem StrategyAsymptoticallyDominates.refl (A : StrategyPerformance) :
    StrategyAsymptoticallyDominates A A :=
  BigO.refl A.worstCaseSteps

/-- Asymptotic dominance is transitive. -/
theorem StrategyAsymptoticallyDominates.trans {A B C : StrategyPerformance}
    (hAB : StrategyAsymptoticallyDominates A B)
    (hBC : StrategyAsymptoticallyDominates B C) :
    StrategyAsymptoticallyDominates A C :=
  BigO.trans hAB hBC

/-- A solver class is any predicate on strategy expressions. -/
abbrev SolverClass := StrategyExpr → Prop

/-- A strategy expression is pointwise best in a class when it belongs to the class and its
performance profile pointwise dominates every other member's profile. This is the formal version
of "provably best" once a strategy language and performance interpretation have been fixed. -/
def StrategyBestInClass (Class : SolverClass) (Perf : StrategyExpr → StrategyPerformance)
    (A : StrategyExpr) : Prop :=
  Class A ∧ ∀ B : StrategyExpr, Class B → StrategyDominates (Perf A) (Perf B)

/-- Asymptotic version of best-in-class: `A` is in the class and its worst-case envelope is
big-O dominated by every other member's envelope. -/
def StrategyAsymptoticallyBestInClass
    (Class : SolverClass) (Perf : StrategyExpr → StrategyPerformance)
    (A : StrategyExpr) : Prop :=
  Class A ∧ ∀ B : StrategyExpr, Class B →
    StrategyAsymptoticallyDominates (Perf A) (Perf B)

theorem StrategyBestInClass.dominates {Class : SolverClass}
    {Perf : StrategyExpr → StrategyPerformance} {A B : StrategyExpr}
    (hA : StrategyBestInClass Class Perf A) (hB : Class B) :
    StrategyDominates (Perf A) (Perf B) :=
  hA.2 B hB

theorem StrategyAsymptoticallyBestInClass.dominates {Class : SolverClass}
    {Perf : StrategyExpr → StrategyPerformance} {A B : StrategyExpr}
    (hA : StrategyAsymptoticallyBestInClass Class Perf A) (hB : Class B) :
    StrategyAsymptoticallyDominates (Perf A) (Perf B) :=
  hA.2 B hB

/-- A strategy class has a polynomial member if some member has a polynomial worst-case envelope
under the chosen performance interpretation. -/
def StrategyClassHasPolynomialMember
    (Class : SolverClass) (Perf : StrategyExpr → StrategyPerformance) : Prop :=
  ∃ A : StrategyExpr, Class A ∧ PolynomialO (Perf A).worstCaseSteps

/-- A strategy class has a pointwise lower bound on a family if every member costs at least
`lower n` on the `n`th family member. -/
def StrategyClassFamilyLowerBound
    (Class : SolverClass) (Perf : StrategyExpr → StrategyPerformance)
    (Family : Nat → CoverInput) (lower : Nat → Nat) : Prop :=
  ∀ A : StrategyExpr, Class A → ∀ n : Nat, lower n ≤ (Perf A).stepsOn (Family n)

/-- A family-level lower bound for a class of strategy expressions, phrased through an
abstract performance interpretation for each strategy. This is the slot where proof-system
lower bounds can be transferred into solver lower bounds. -/
def SolverClassLowerBound (Class : SolverClass) (Perf : StrategyExpr → StrategyPerformance)
    (Family : Nat → CoverInput) (lower : Nat → Nat) : Prop :=
  ∀ S : StrategyExpr, Class S → ∀ n : Nat, lower n ≤ (Perf S).stepsOn (Family n)

theorem StrategyClassFamilyLowerBound.to_solverClassLowerBound {Class : SolverClass}
    {Perf : StrategyExpr → StrategyPerformance} {Family : Nat → CoverInput}
    {lower : Nat → Nat}
    (h : StrategyClassFamilyLowerBound Class Perf Family lower) :
    SolverClassLowerBound Class Perf Family lower :=
  h

/-- Bridge from proof-system lower bounds to solver-class lower bounds. The field
`proofSizeOf S n` abstracts "the proof/certificate implicitly produced by strategy `S` on
family member `n`"; `steps_ge_proof` says producing it costs at least its size, while
`proof_lower` is the mathematical lower bound for that proof system. -/
structure ProofLowerBoundBridge (Class : SolverClass)
    (Perf : StrategyExpr → StrategyPerformance) (Family : Nat → CoverInput)
    (lower : Nat → Nat) where
  proofSizeOf : StrategyExpr → Nat → Nat
  steps_ge_proof :
    ∀ S : StrategyExpr, Class S → ∀ n : Nat,
      proofSizeOf S n ≤ (Perf S).stepsOn (Family n)
  proof_lower :
    ∀ S : StrategyExpr, Class S → ∀ n : Nat, lower n ≤ proofSizeOf S n

/-- Any proof lower-bound bridge immediately yields a solver-class lower bound. -/
theorem SolverClassLowerBound.of_proof_bridge {Class : SolverClass}
    {Perf : StrategyExpr → StrategyPerformance} {Family : Nat → CoverInput}
    {lower : Nat → Nat}
    (B : ProofLowerBoundBridge Class Perf Family lower) :
    SolverClassLowerBound Class Perf Family lower := by
  intro S hS n
  exact (B.proof_lower S hS n).trans (B.steps_ge_proof S hS n)

/-- Strategies with syntax size at most `s`. This is the finite-grammar slice used by
bounded meta-search and strategy enumeration. -/
def BoundedStrategyClass (s : Nat) : SolverClass :=
  fun S => S.BoundedBy s

theorem BoundedStrategyClass.mono {a b : Nat} (hab : a ≤ b) :
    ∀ S : StrategyExpr, BoundedStrategyClass a S → BoundedStrategyClass b S := by
  intro S hS
  exact hS.trans hab

/-- If the empty pattern is semantically covered by a family of patterns, then every
hypercube vertex is covered by one of that family's patterns. -/
theorem full_cover_of_empty_pattern_covered {n : Nat} (Ps : List (Pattern n))
    (h : ∀ v, (Pattern.empty n).Covers v → ∃ P ∈ Ps, P.Covers v) :
    ∀ v : Vertex n, ∃ P ∈ Ps, P.Covers v := by
  intro v
  exact h v (Pattern.empty_covers v)

/-! ### §E1 Full covers and minimal UNSAT cores -/

/-- A finite family of patterns fully covers the hypercube. -/
def FullCover {n : Nat} (Ps : Finset (Pattern n)) : Prop :=
  ∀ v : Vertex n, ∃ P ∈ Ps, P.Covers v

/-- A full cover is minimal if erasing any pattern destroys full coverage. -/
def MinimalFullCover {n : Nat} (Ps : Finset (Pattern n)) : Prop :=
  FullCover Ps ∧ ∀ P ∈ Ps, ¬ FullCover (Ps.erase P)

/-- A private vertex for pattern `P` is covered by `P` and by no other pattern in the family. -/
def PrivateVertex {n : Nat} (Ps : Finset (Pattern n)) (P : Pattern n) (v : Vertex n) : Prop :=
  P ∈ Ps ∧ P.Covers v ∧ ∀ Q ∈ Ps, Q.Covers v → Q = P

/-- Full coverage is monotone under adding patterns. -/
theorem FullCover.mono {n : Nat} {Ps Qs : Finset (Pattern n)}
    (hsub : Ps ⊆ Qs) (hcover : FullCover Ps) :
    FullCover Qs := by
  intro v
  obtain ⟨P, hP, hPv⟩ := hcover v
  exact ⟨P, hsub hP, hPv⟩

/-- Minimal full covers have private vertices. -/
theorem minimal_full_cover_private_vertex {n : Nat} (Ps : Finset (Pattern n))
    (hmin : MinimalFullCover Ps) {P : Pattern n} (hP : P ∈ Ps) :
    ∃ v : Vertex n, PrivateVertex Ps P v := by
  rcases hmin with ⟨hcover, hminimal⟩
  have hnot : ¬ FullCover (Ps.erase P) := hminimal P hP
  unfold FullCover at hnot
  push Not at hnot
  rcases hnot with ⟨v, hvnot⟩
  obtain ⟨Q, hQ, hQv⟩ := hcover v
  have hQP : Q = P := by
    by_contra hneq
    have hQerase : Q ∈ Ps.erase P := by
      simp [hQ, hneq]
    exact hvnot Q hQerase hQv
  refine ⟨v, hP, ?_, ?_⟩
  · simpa [hQP] using hQv
  · intro R hR hRv
    by_contra hneq
    have hRerase : R ∈ Ps.erase P := by
      simp [hR, hneq]
    exact hvnot R hRerase hRv

/-! ### §E2 Local satisfiability radius and supports -/

/-- A pattern family is locally satisfiable up to radius `r` when no subfamily of size at most
`r` is already a full cover. -/
def LocallySatisfiableUpTo {n : Nat} (Ps : Finset (Pattern n)) (r : Nat) : Prop :=
  ∀ S : Finset (Pattern n), S ⊆ Ps → S.card ≤ r → ¬ FullCover S

/-- Local satisfiability is monotone downward in the radius. -/
theorem LocallySatisfiableUpTo.mono {n : Nat} {Ps : Finset (Pattern n)} {r s : Nat}
    (h : LocallySatisfiableUpTo Ps r) (hsr : s ≤ r) :
    LocallySatisfiableUpTo Ps s := by
  intro S hS hcard
  exact h S hS (hcard.trans hsr)

/-- If a family is locally satisfiable up to its own size, then it is not a full cover. -/
theorem not_fullCover_of_locallySatisfiable_card {n : Nat} {Ps : Finset (Pattern n)}
    (h : LocallySatisfiableUpTo Ps Ps.card) :
    ¬ FullCover Ps := by
  exact h Ps (by intro P hP; exact hP) le_rfl

/-- The coordinate support of a finite pattern family. -/
def PatternFamily.support {n : Nat} (Ps : Finset (Pattern n)) : Finset (Fin n) :=
  Ps.biUnion Pattern.support

theorem PatternFamily.mem_support_iff {n : Nat} (Ps : Finset (Pattern n)) (i : Fin n) :
    i ∈ PatternFamily.support Ps ↔ ∃ P ∈ Ps, i ∈ P.support := by
  unfold PatternFamily.support
  simp

/-- A pattern family is supported inside a coordinate set. -/
def PatternFamily.SupportedIn {n : Nat} (Ps : Finset (Pattern n)) (U : Finset (Fin n)) : Prop :=
  PatternFamily.support Ps ⊆ U

theorem PatternFamily.supportedIn_univ {n : Nat} (Ps : Finset (Pattern n)) :
    PatternFamily.SupportedIn Ps Finset.univ := by
  intro i hi
  exact Finset.mem_univ i

theorem PatternFamily.support_mono {n : Nat} {Ps Qs : Finset (Pattern n)}
    (hsub : Ps ⊆ Qs) :
    PatternFamily.support Ps ⊆ PatternFamily.support Qs := by
  intro i hi
  rw [PatternFamily.mem_support_iff] at hi ⊢
  obtain ⟨P, hP, hiP⟩ := hi
  exact ⟨P, hsub hP, hiP⟩

/-- A pattern crosses a proposed variable side `U` when its support touches both `U`
and the complement of `U`. This is the first hook for component/separator rules. -/
def PatternFamily.Crosses {n : Nat} (P : Pattern n) (U : Finset (Fin n)) : Prop :=
  ∃ i ∈ P.support, ∃ j ∈ P.support, i ∈ U ∧ j ∉ U

/-- A family is separated by `U` if every pattern is wholly inside `U` or wholly outside `U`.
This is a lightweight component-decomposition predicate. -/
def PatternFamily.SeparatedBy {n : Nat} (Ps : Finset (Pattern n)) (U : Finset (Fin n)) :
    Prop :=
  ∀ P ∈ Ps, P.support ⊆ U ∨ Disjoint P.support U

/-- A separated family has no pattern crossing the separator side. -/
theorem PatternFamily.not_crosses_of_separatedBy {n : Nat}
    {Ps : Finset (Pattern n)} {U : Finset (Fin n)}
    (hsep : PatternFamily.SeparatedBy Ps U) :
    ∀ P ∈ Ps, ¬ PatternFamily.Crosses P U := by
  intro P hP hcross
  rcases hsep P hP with hsub | hdisj
  · rcases hcross with ⟨_, _, j, hjP, _, hjnotU⟩
    exact hjnotU (hsub hjP)
  · rcases hcross with ⟨i, hiP, _, _, hiU, _⟩
    exact (Finset.disjoint_left.mp hdisj) hiP hiU

/-- The part of a family whose patterns are supported inside `U`. -/
def PatternFamily.inside {n : Nat} (Ps : Finset (Pattern n)) (U : Finset (Fin n)) :
    Finset (Pattern n) :=
  Ps.filter fun P => P.support ⊆ U

@[simp] theorem PatternFamily.mem_inside {n : Nat} {Ps : Finset (Pattern n)}
    {U : Finset (Fin n)} {P : Pattern n} :
    P ∈ PatternFamily.inside Ps U ↔ P ∈ Ps ∧ P.support ⊆ U := by
  simp [PatternFamily.inside]

/-- The inside subfamily is supported inside its side. -/
theorem PatternFamily.inside_supportedIn {n : Nat}
    (Ps : Finset (Pattern n)) (U : Finset (Fin n)) :
    PatternFamily.SupportedIn (PatternFamily.inside Ps U) U := by
  intro i hi
  rw [PatternFamily.mem_support_iff] at hi
  obtain ⟨P, hP, hiP⟩ := hi
  exact (PatternFamily.mem_inside.mp hP).2 hiP

/-! ### §E3 Pattern operations and derivation chains -/

/-- Semantically, `P` is covered by a family `Ps` when every vertex in `P` lies in some
pattern from `Ps`. This is the invariant preserved by proof/merge operations. -/
def Pattern.SemanticallyCoveredBy {n : Nat} (Ps : List (Pattern n)) (P : Pattern n) : Prop :=
  ∀ v : Vertex n, P.Covers v → ∃ Q ∈ Ps, Q.Covers v

/-- Any pattern listed in the base family is semantically covered by that family. -/
theorem Pattern.semCovered_of_mem {n : Nat} {Ps : List (Pattern n)} {P : Pattern n}
    (hP : P ∈ Ps) : P.SemanticallyCoveredBy Ps := by
  intro v hv
  exact ⟨P, hP, hv⟩

/-- Semantic coverage is monotone under adding known patterns. -/
theorem Pattern.SemanticallyCoveredBy.mono {n : Nat} {Ps Qs : List (Pattern n)}
    {P : Pattern n} (hsub : ∀ Q, Q ∈ Ps → Q ∈ Qs)
    (hcov : P.SemanticallyCoveredBy Ps) :
    P.SemanticallyCoveredBy Qs := by
  intro v hv
  obtain ⟨Q, hQ, hQv⟩ := hcov v hv
  exact ⟨Q, hsub Q hQ, hQv⟩

/-- If a larger face `P` is covered, every smaller face `Q ⊆ P` is covered too. -/
theorem Pattern.SemanticallyCoveredBy.of_le {n : Nat} {Ps : List (Pattern n)}
    {P Q : Pattern n} (hcov : P.SemanticallyCoveredBy Ps) (hPQ : P.Le Q) :
    Q.SemanticallyCoveredBy Ps := by
  intro v hQv
  exact hcov v (P.covers_of_le hPQ v hQv)

/-- A face is semantically covered by its two Boolean children along any free coordinate. -/
theorem Pattern.semCoveredBy_assign_split {n : Nat} (P : Pattern n) (i : Fin n)
    (hfree : P.Free i) :
    P.SemanticallyCoveredBy [P.assign i false hfree, P.assign i true hfree] := by
  intro v hPv
  rcases P.split_by_free i hfree v hPv with hfalse | htrue
  · exact ⟨P.assign i false hfree, by simp, hfalse⟩
  · exact ⟨P.assign i true hfree, by simp, htrue⟩

/-- If a target face is covered by a frontier whose head is `P`, then replacing that head
by the two assigned children of `P` preserves semantic coverage. -/
theorem Pattern.SemanticallyCoveredBy.branch_head {n : Nat}
    {Target P : Pattern n} {Rest : List (Pattern n)} {i : Fin n}
    (hfree : P.Free i) (hcov : Target.SemanticallyCoveredBy (P :: Rest)) :
    Target.SemanticallyCoveredBy
      (P.assign i false hfree :: P.assign i true hfree :: Rest) := by
  intro v hTv
  obtain ⟨Q, hQ, hQv⟩ := hcov v hTv
  rw [List.mem_cons] at hQ
  rcases hQ with hQP | hQrest
  · subst Q
    rcases P.split_by_free i hfree v hQv with hfalse | htrue
    · exact ⟨P.assign i false hfree, by simp, hfalse⟩
    · exact ⟨P.assign i true hfree, by simp, htrue⟩
  · exact ⟨Q, by simp [hQrest], hQv⟩

/-- Semantic merge rule: if the two Boolean halves are covered, the parent face is covered. -/
theorem Pattern.SemanticallyCoveredBy.merge {n : Nat} {Ps : List (Pattern n)}
    {A P0 P1 : Pattern n} (i : Fin n)
    (h0cov : P0.SemanticallyCoveredBy Ps) (h1cov : P1.SemanticallyCoveredBy Ps)
    (h0 : ∀ v, A.Covers v ∧ v i = false → P0.Covers v)
    (h1 : ∀ v, A.Covers v ∧ v i = true → P1.Covers v) :
    A.SemanticallyCoveredBy Ps := by
  intro v hA
  rcases pattern_merge_sound_semantic A P0 P1 i h0 h1 v hA with hP0 | hP1
  · exact h0cov v hP0
  · exact h1cov v hP1

/-- A syntactic derivability relation generated by hypotheses, refinement/subsumption,
and semantic merge. This is the first proof-chain API for subcube-cover reasoning. -/
inductive PatternDerivable {n : Nat} (base : List (Pattern n)) : Pattern n → Prop where
  | hyp {P : Pattern n} : P ∈ base → PatternDerivable base P
  | refine {P Q : Pattern n} :
      PatternDerivable base P → P.Le Q → PatternDerivable base Q
  | merge {A P0 P1 : Pattern n} (i : Fin n) :
      PatternDerivable base P0 →
      PatternDerivable base P1 →
      (∀ v, A.Covers v ∧ v i = false → P0.Covers v) →
      (∀ v, A.Covers v ∧ v i = true → P1.Covers v) →
      PatternDerivable base A

/-- Every derivation chain is semantically sound. -/
theorem PatternDerivable.sound {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (hder : PatternDerivable base P) :
    P.SemanticallyCoveredBy base := by
  induction hder with
  | hyp hP =>
      exact Pattern.semCovered_of_mem hP
  | refine hder hPQ ih =>
      exact ih.of_le hPQ
  | merge i h0der h1der h0 h1 ih0 ih1 =>
      exact Pattern.SemanticallyCoveredBy.merge i ih0 ih1 h0 h1

/-- Deriving the empty pattern is a sound certificate of full coverage by the base patterns. -/
theorem full_cover_of_derives_empty {n : Nat} {base : List (Pattern n)}
    (hder : PatternDerivable base (Pattern.empty n)) :
    ∀ v : Vertex n, ∃ P ∈ base, P.Covers v := by
  exact full_cover_of_empty_pattern_covered base hder.sound

/-- A proof object version of `PatternDerivable`, living in `Type`, so we can measure
steps and maximum width along a derivation chain. -/
inductive PatternProof {n : Nat} (base : List (Pattern n)) : Pattern n → Type where
  | hyp {P : Pattern n} : P ∈ base → PatternProof base P
  | refine {P Q : Pattern n} :
      PatternProof base P → P.Le Q → PatternProof base Q
  | merge {A P0 P1 : Pattern n} (i : Fin n) :
      PatternProof base P0 →
      PatternProof base P1 →
      (∀ v, A.Covers v ∧ v i = false → P0.Covers v) →
      (∀ v, A.Covers v ∧ v i = true → P1.Covers v) →
      PatternProof base A

namespace PatternProof

/-- The core operation kinds in the current geometric proof system. -/
inductive RuleKind where
  | hyp
  | refine
  | merge
  deriving DecidableEq, Repr

/-- A cost model assigns an operation cost to each core rule. Different models let us compare
"one merge = one step" against finer operational accounting later. -/
structure CostModel where
  hyp : Nat
  refine : Nat
  merge : Nat

/-- A rule system records which core operations are allowed. This is a lightweight way to compare
restricted proof worlds without changing the semantic proof objects. -/
structure RuleSystem where
  allowHyp : Bool
  allowRefine : Bool
  allowMerge : Bool

/-- The full core rule system. -/
def fullRuleSystem : RuleSystem where
  allowHyp := true
  allowRefine := true
  allowMerge := true

/-- A hypothesis-only rule system. Useful as a degenerate lower-bound baseline. -/
def hypOnlyRuleSystem : RuleSystem where
  allowHyp := true
  allowRefine := false
  allowMerge := false

/-- Check whether a rule kind is allowed by a rule system. -/
def RuleSystem.Allows (R : RuleSystem) : RuleKind → Prop
  | RuleKind.hyp => R.allowHyp = true
  | RuleKind.refine => R.allowRefine = true
  | RuleKind.merge => R.allowMerge = true

/-- The default structural cost model: hypotheses are free, refinement and merge cost one. -/
def defaultCost : CostModel where
  hyp := 0
  refine := 1
  merge := 1

/-- Look up the cost of one rule kind in a cost model. -/
def RuleKind.cost (C : CostModel) : RuleKind → Nat
  | RuleKind.hyp => C.hyp
  | RuleKind.refine => C.refine
  | RuleKind.merge => C.merge

/-- The expected number of derived premises consumed by each operation kind. Hypotheses are
imported from the base, refinement uses one previous pattern, merge uses two. -/
def RuleKind.arity : RuleKind → Nat
  | RuleKind.hyp => 0
  | RuleKind.refine => 1
  | RuleKind.merge => 2

/-- Maximum width in a list of patterns. -/
def listMaxWidth {n : Nat} : List (Pattern n) → Nat
  | [] => 0
  | P :: Ps => max P.width (listMaxWidth Ps)

/-- A multi-column operation accountant. Scalar costs are projections of this vector under
chosen weights; the vector itself records which kind of work happened. -/
structure CostVector where
  hyp : Nat
  refine : Nat
  merge : Nat
  blockerChecks : Nat
  vertexVisits : Nat
  memoryCells : Nat
  deriving DecidableEq, Repr

namespace CostVector

/-- The zero cost vector. -/
def zero : CostVector where
  hyp := 0
  refine := 0
  merge := 0
  blockerChecks := 0
  vertexVisits := 0
  memoryCells := 0

/-- Componentwise addition of accounting columns. -/
def add (a b : CostVector) : CostVector where
  hyp := a.hyp + b.hyp
  refine := a.refine + b.refine
  merge := a.merge + b.merge
  blockerChecks := a.blockerChecks + b.blockerChecks
  vertexVisits := a.vertexVisits + b.vertexVisits
  memoryCells := a.memoryCells + b.memoryCells

/-- Componentwise scalar multiplication. -/
def smul (k : Nat) (a : CostVector) : CostVector where
  hyp := k * a.hyp
  refine := k * a.refine
  merge := k * a.merge
  blockerChecks := k * a.blockerChecks
  vertexVisits := k * a.vertexVisits
  memoryCells := k * a.memoryCells

/-- Componentwise domination of accounting vectors. -/
def Le (a b : CostVector) : Prop :=
  a.hyp ≤ b.hyp ∧
  a.refine ≤ b.refine ∧
  a.merge ≤ b.merge ∧
  a.blockerChecks ≤ b.blockerChecks ∧
  a.vertexVisits ≤ b.vertexVisits ∧
  a.memoryCells ≤ b.memoryCells

/-- Collapse a vector account to a scalar by choosing nonnegative weights for every column. -/
def weighted (weights a : CostVector) : Nat :=
  weights.hyp * a.hyp +
  weights.refine * a.refine +
  weights.merge * a.merge +
  weights.blockerChecks * a.blockerChecks +
  weights.vertexVisits * a.vertexVisits +
  weights.memoryCells * a.memoryCells

/-- A unit vector for hypothesis imports. -/
def hypUnit : CostVector := { zero with hyp := 1 }

/-- A unit vector for refinement operations. -/
def refineUnit : CostVector := { zero with refine := 1 }

/-- A unit vector for merge operations. -/
def mergeUnit : CostVector := { zero with merge := 1 }

/-- A unit vector for blocker checks. -/
def blockerCheckUnit : CostVector := { zero with blockerChecks := 1 }

/-- A unit vector for vertex visits. -/
def vertexVisitUnit : CostVector := { zero with vertexVisits := 1 }

/-- A unit vector for memory cells. -/
def memoryCellUnit : CostVector := { zero with memoryCells := 1 }

/-- All columns have unit weight. -/
def allOnes : CostVector where
  hyp := 1
  refine := 1
  merge := 1
  blockerChecks := 1
  vertexVisits := 1
  memoryCells := 1

@[simp] theorem zero_add (a : CostVector) : add zero a = a := by
  cases a
  simp [add, zero]

@[simp] theorem add_zero (a : CostVector) : add a zero = a := by
  cases a
  simp [add, zero]

theorem add_comm (a b : CostVector) : add a b = add b a := by
  cases a
  cases b
  simp [add, Nat.add_comm]

theorem add_assoc (a b c : CostVector) :
    add (add a b) c = add a (add b c) := by
  cases a
  cases b
  cases c
  simp [add, Nat.add_assoc]

theorem Le.refl (a : CostVector) : Le a a := by
  simp [Le]

theorem Le.trans {a b c : CostVector} (hab : Le a b) (hbc : Le b c) : Le a c := by
  rcases hab with ⟨h1, h2, h3, h4, h5, h6⟩
  rcases hbc with ⟨k1, k2, k3, k4, k5, k6⟩
  exact ⟨h1.trans k1, h2.trans k2, h3.trans k3, h4.trans k4, h5.trans k5, h6.trans k6⟩

theorem weighted_add (weights a b : CostVector) :
    weighted weights (add a b) = weighted weights a + weighted weights b := by
  cases weights
  cases a
  cases b
  simp [weighted, add]
  ring

/-- Componentwise big-O for vector-valued cost families. -/
def BigO (f g : Nat → CostVector) : Prop :=
  _root_.HypercubeSAT.BigO (fun N => (f N).hyp) (fun N => (g N).hyp) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).refine) (fun N => (g N).refine) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).merge) (fun N => (g N).merge) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).blockerChecks) (fun N => (g N).blockerChecks) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).vertexVisits) (fun N => (g N).vertexVisits) ∧
  _root_.HypercubeSAT.BigO (fun N => (f N).memoryCells) (fun N => (g N).memoryCells)

/-- A vector family is polynomially bounded when every accounting column is polynomially bounded. -/
def PolynomialO (f : Nat → CostVector) : Prop :=
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).hyp) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).refine) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).merge) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).blockerChecks) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).vertexVisits) ∧
  _root_.HypercubeSAT.PolynomialO (fun N => (f N).memoryCells)

/-- Weighted scalar projection of a vector-valued cost family. -/
def weightedFamily (weights : CostVector) (f : Nat → CostVector) : Nat → Nat :=
  fun N => weighted weights (f N)

end CostVector

/-- Vector-accountant profile for the verifier: one blocker-check column per blocker,
and linear memory as a conservative input scan envelope. -/
def verifierCostVector (N : Nat) : CostVector where
  hyp := 0
  refine := 0
  merge := 0
  blockerChecks := N
  vertexVisits := 0
  memoryCells := N

/-- Vector-accountant profile for brute force over a size-`N` instance: visit `2^N` vertices
and perform `N * 2^N` blocker checks as a coarse envelope. -/
def bruteForceCostVector (N : Nat) : CostVector where
  hyp := 0
  refine := 0
  merge := 0
  blockerChecks := N * 2 ^ N
  vertexVisits := 2 ^ N
  memoryCells := N

theorem verifierCostVector_polynomialO :
    CostVector.PolynomialO verifierCostVector := by
  unfold CostVector.PolynomialO verifierCostVector
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
  · exact ⟨0, BigO.of_forall_le_mul (C := 0) (by simp)⟩
  · exact ⟨0, BigO.of_forall_le_mul (C := 0) (by simp)⟩
  · exact ⟨0, BigO.of_forall_le_mul (C := 0) (by simp)⟩
  · exact PolynomialO_of_PolyBound verify_steps_poly
  · exact ⟨0, BigO.of_forall_le_mul (C := 0) (by simp)⟩
  · exact PolynomialO_of_PolyBound verify_steps_poly

theorem bruteForceCostVector_columns_exp :
    ExponentialO (fun N => (bruteForceCostVector N).blockerChecks) ∧
      ExponentialO (fun N => (bruteForceCostVector N).vertexVisits) := by
  constructor
  · exact brute_force_steps_exponentialO
  · refine ⟨0, 1, 0, fun N _ => ?_⟩
    simp [bruteForceCostVector]

/-- A vector-valued cost model for the proof rules. -/
structure VectorCostModel where
  hyp : CostVector
  refine : CostVector
  merge : CostVector

/-- The default vector model counts each core proof operation in its own column. -/
def defaultVectorCost : VectorCostModel where
  hyp := CostVector.hypUnit
  refine := CostVector.refineUnit
  merge := CostVector.mergeUnit

/-- A first-class accounted operation: it records what an operation consumes, produces,
costs, and why it is semantically sound. This is the local ledger card for proof systems. -/
structure AccountedOperation (n : Nat) where
  kind : RuleKind
  premises : List (Pattern n)
  output : Pattern n
  cost : Nat
  sound :
    ∀ base : List (Pattern n),
      (∀ P ∈ premises, P.SemanticallyCoveredBy base) →
      output.SemanticallyCoveredBy base

namespace AccountedOperation

/-- Number of premise patterns consumed by the operation. -/
def arity {n : Nat} (op : AccountedOperation n) : Nat :=
  op.premises.length

/-- Maximum width among the operation's premises. -/
def premiseMaxWidth {n : Nat} (op : AccountedOperation n) : Nat :=
  listMaxWidth op.premises

/-- Maximum width locally touched by this operation, including its output. -/
def localMaxWidth {n : Nat} (op : AccountedOperation n) : Nat :=
  max op.output.width op.premiseMaxWidth

/-- An operation stays within a local envelope for cost, width, and number of premises. -/
def WithinEnvelope {n : Nat} (op : AccountedOperation n)
    (costLimit widthLimit arityLimit : Nat) : Prop :=
  op.cost ≤ costLimit ∧ op.localMaxWidth ≤ widthLimit ∧ op.arity ≤ arityLimit

/-- The standard accounted refinement operation. -/
def refinement {n : Nat} (P Q : Pattern n) (hPQ : P.Le Q) : AccountedOperation n where
  kind := RuleKind.refine
  premises := [P]
  output := Q
  cost := RuleKind.refine.cost defaultCost
  sound := by
    intro base hpre
    have hP : P.SemanticallyCoveredBy base := hpre P (by simp)
    exact hP.of_le hPQ

/-- The standard accounted merge operation. -/
def merge {n : Nat} (A P0 P1 : Pattern n) (i : Fin n)
    (h0 : ∀ v, A.Covers v ∧ v i = false → P0.Covers v)
    (h1 : ∀ v, A.Covers v ∧ v i = true → P1.Covers v) :
    AccountedOperation n where
  kind := RuleKind.merge
  premises := [P0, P1]
  output := A
  cost := RuleKind.merge.cost defaultCost
  sound := by
    intro base hpre
    have hP0 : P0.SemanticallyCoveredBy base := hpre P0 (by simp)
    have hP1 : P1.SemanticallyCoveredBy base := hpre P1 (by simp)
    exact Pattern.SemanticallyCoveredBy.merge i hP0 hP1 h0 h1

@[simp] theorem refinement_arity {n : Nat} (P Q : Pattern n) (hPQ : P.Le Q) :
    (refinement P Q hPQ).arity = RuleKind.refine.arity := by
  simp [arity, refinement, RuleKind.arity]

@[simp] theorem merge_arity {n : Nat} (A P0 P1 : Pattern n) (i : Fin n)
    (h0 : ∀ v, A.Covers v ∧ v i = false → P0.Covers v)
    (h1 : ∀ v, A.Covers v ∧ v i = true → P1.Covers v) :
    (merge A P0 P1 i h0 h1).arity = RuleKind.merge.arity := by
  simp [arity, merge, RuleKind.arity]

end AccountedOperation

/-- A raw operation trace is a ledger of accounted operations. It may or may not be a valid proof;
validity is a separate semantic/connectivity condition. -/
abbrev OperationTrace (n : Nat) := List (AccountedOperation n)

namespace OperationTrace

/-- Total operation cost in a raw trace. -/
def totalCost {n : Nat} : OperationTrace n → Nat
  | [] => 0
  | op :: ops => op.cost + totalCost ops

/-- Largest local width touched by any operation in a raw trace. -/
def maxLocalWidth {n : Nat} : OperationTrace n → Nat
  | [] => 0
  | op :: ops => max op.localMaxWidth (maxLocalWidth ops)

/-- Total number of premise-pattern touches across the trace. -/
def premiseTouches {n : Nat} : OperationTrace n → Nat
  | [] => 0
  | op :: ops => op.arity + premiseTouches ops

/-- A trace stays inside a cost/width/arity envelope. -/
def WithinEnvelope {n : Nat} (ops : OperationTrace n)
    (costLimit widthLimit premiseTouchLimit : Nat) : Prop :=
  totalCost ops ≤ costLimit ∧
  maxLocalWidth ops ≤ widthLimit ∧
  premiseTouches ops ≤ premiseTouchLimit

@[simp] theorem totalCost_append {n : Nat} (xs ys : OperationTrace n) :
    totalCost (xs ++ ys) = totalCost xs + totalCost ys := by
  induction xs with
  | nil => simp [totalCost]
  | cons op xs ih =>
      simp [totalCost, ih]
      omega

@[simp] theorem premiseTouches_append {n : Nat} (xs ys : OperationTrace n) :
    premiseTouches (xs ++ ys) = premiseTouches xs + premiseTouches ys := by
  induction xs with
  | nil => simp [premiseTouches]
  | cons op xs ih =>
      simp [premiseTouches, ih]
      omega

@[simp] theorem maxLocalWidth_append {n : Nat} (xs ys : OperationTrace n) :
    maxLocalWidth (xs ++ ys) = max (maxLocalWidth xs) (maxLocalWidth ys) := by
  induction xs with
  | nil => simp [maxLocalWidth]
  | cons op xs ih =>
      simp [maxLocalWidth, ih, max_assoc]

end OperationTrace

/-- The number of non-hypothesis operations in a pattern proof. -/
def steps {n : Nat} {base : List (Pattern n)} : {P : Pattern n} → PatternProof base P → Nat
  | _, hyp _ => 0
  | _, refine hp _ => hp.steps + 1
  | _, merge _ hp0 hp1 _ _ => hp0.steps + hp1.steps + 1

/-- Number of hypothesis leaves used in a proof tree. -/
def hypCount {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → PatternProof base P → Nat
  | _, hyp _ => 1
  | _, refine hp _ => hp.hypCount
  | _, merge _ hp0 hp1 _ _ => hp0.hypCount + hp1.hypCount

/-- Number of refinement/subsumption operations used in a proof tree. -/
def refineCount {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → PatternProof base P → Nat
  | _, hyp _ => 0
  | _, refine hp _ => hp.refineCount + 1
  | _, merge _ hp0 hp1 _ _ => hp0.refineCount + hp1.refineCount

/-- Number of merge/resolution operations used in a proof tree. -/
def mergeCount {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → PatternProof base P → Nat
  | _, hyp _ => 0
  | _, refine hp _ => hp.mergeCount
  | _, merge _ hp0 hp1 _ _ => hp0.mergeCount + hp1.mergeCount + 1

/-- Total number of nodes in the proof tree, counting hypotheses and operations. -/
def nodeCount {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → PatternProof base P → Nat
  | _, hyp _ => 1
  | _, refine hp _ => hp.nodeCount + 1
  | _, merge _ hp0 hp1 _ _ => hp0.nodeCount + hp1.nodeCount + 1

/-- Total cost of a proof tree under an arbitrary cost model. -/
def totalCost {n : Nat} {base : List (Pattern n)} (C : CostModel) :
    {P : Pattern n} → PatternProof base P → Nat
  | _, hyp _ => RuleKind.hyp.cost C
  | _, refine hp _ => hp.totalCost C + RuleKind.refine.cost C
  | _, merge _ hp0 hp1 _ _ => hp0.totalCost C + hp1.totalCost C + RuleKind.merge.cost C

/-- Vector-valued proof accounting under a vector cost model. -/
def totalVectorCost {n : Nat} {base : List (Pattern n)} (C : VectorCostModel) :
    {P : Pattern n} → PatternProof base P → CostVector
  | _, hyp _ => C.hyp
  | _, refine hp _ => CostVector.add (hp.totalVectorCost C) C.refine
  | _, merge _ hp0 hp1 _ _ =>
      CostVector.add (CostVector.add (hp0.totalVectorCost C) (hp1.totalVectorCost C)) C.merge

/-- The default vector account records the exact hypothesis/refinement/merge counts. -/
theorem totalVectorCost_default_eq_counts {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → (hp : PatternProof base P) →
      hp.totalVectorCost defaultVectorCost =
        { hyp := hp.hypCount
          refine := hp.refineCount
          merge := hp.mergeCount
          blockerChecks := 0
          vertexVisits := 0
          memoryCells := 0 }
  | _, hyp _ => by simp [totalVectorCost, defaultVectorCost, hypCount, refineCount, mergeCount,
      CostVector.hypUnit, CostVector.zero]
  | _, refine hp _ => by
      rw [totalVectorCost, totalVectorCost_default_eq_counts hp]
      simp [defaultVectorCost, hypCount, refineCount, mergeCount,
        CostVector.refineUnit, CostVector.add, CostVector.zero]
  | _, merge _ hp0 hp1 _ _ => by
      rw [totalVectorCost, totalVectorCost_default_eq_counts hp0,
        totalVectorCost_default_eq_counts hp1]
      simp [defaultVectorCost, hypCount, refineCount, mergeCount,
        CostVector.mergeUnit, CostVector.add, CostVector.zero]

/-- A proof uses only the operations allowed by a rule system. -/
def UsesOnly {n : Nat} {base : List (Pattern n)} (R : RuleSystem) :
    {P : Pattern n} → PatternProof base P → Prop
  | _, hyp _ => R.Allows RuleKind.hyp
  | _, refine hp _ => hp.UsesOnly R ∧ R.Allows RuleKind.refine
  | _, merge _ hp0 hp1 _ _ =>
      hp0.UsesOnly R ∧ hp1.UsesOnly R ∧ R.Allows RuleKind.merge

/-- Every proof uses only the full core rule system. -/
theorem usesOnly_fullRuleSystem {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → (hp : PatternProof base P) → hp.UsesOnly fullRuleSystem
  | _, hyp _ => by simp [UsesOnly, RuleSystem.Allows, fullRuleSystem]
  | _, refine hp _ => by
      exact ⟨usesOnly_fullRuleSystem hp, rfl⟩
  | _, merge _ hp0 hp1 _ _ => by
      exact ⟨usesOnly_fullRuleSystem hp0, usesOnly_fullRuleSystem hp1, rfl⟩

/-- The maximum pattern width appearing anywhere in a proof tree. -/
def maxWidth {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → PatternProof base P → Nat
  | P, hyp _ => P.width
  | Q, refine hp _ => max Q.width hp.maxWidth
  | A, merge _ hp0 hp1 _ _ => max A.width (max hp0.maxWidth hp1.maxWidth)

/-- Every proof tree stays within the absolute ambient width ceiling `2n`, because every
intermediate pattern is consistent. -/
theorem maxWidth_le_two_mul_n {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → (hp : PatternProof base P) → hp.maxWidth ≤ 2 * n
  | _, hyp _ => by
      simpa [maxWidth] using Pattern.width_le_two_mul_n _
  | _, refine hp _ => by
      rw [maxWidth]
      exact max_le (Pattern.width_le_two_mul_n _) (maxWidth_le_two_mul_n hp)
  | _, merge _ hp0 hp1 _ _ => by
      rw [maxWidth]
      exact max_le (Pattern.width_le_two_mul_n _)
        (max_le (maxWidth_le_two_mul_n hp0) (maxWidth_le_two_mul_n hp1))

/-- Forgetting costs turns a proof object into propositional derivability. -/
theorem toDerivable {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (hp : PatternProof base P) : PatternDerivable base P := by
  induction hp with
  | hyp hP =>
      exact PatternDerivable.hyp hP
  | refine hp hPQ ih =>
      exact PatternDerivable.refine ih hPQ
  | merge i hp0 hp1 h0 h1 ih0 ih1 =>
      exact PatternDerivable.merge i ih0 ih1 h0 h1

/-- Every measured proof object is semantically sound. -/
theorem sound {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (hp : PatternProof base P) :
    P.SemanticallyCoveredBy base :=
  hp.toDerivable.sound

/-- The result pattern's width is always within the proof's max-width envelope. -/
theorem width_le_maxWidth {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → (hp : PatternProof base P) → P.width ≤ hp.maxWidth
  | _, hyp _ => by simp [maxWidth]
  | _, refine hp _ => by simp [maxWidth]
  | _, merge _ hp0 hp1 _ _ => by simp [maxWidth]

/-- The default structural step count is exactly refinement count plus merge count. -/
theorem steps_eq_refineCount_add_mergeCount {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → (hp : PatternProof base P) →
      hp.steps = hp.refineCount + hp.mergeCount
  | _, hyp _ => by simp [steps, refineCount, mergeCount]
  | _, refine hp _ => by
      rw [steps, refineCount, mergeCount, steps_eq_refineCount_add_mergeCount hp]
      omega
  | _, merge _ hp0 hp1 _ _ => by
      rw [steps, refineCount, mergeCount,
        steps_eq_refineCount_add_mergeCount hp0,
        steps_eq_refineCount_add_mergeCount hp1]
      omega

/-- Under the default cost model, total cost agrees with the structural step count. -/
theorem totalCost_default_eq_steps {n : Nat} {base : List (Pattern n)} :
    {P : Pattern n} → (hp : PatternProof base P) →
      hp.totalCost defaultCost = hp.steps
  | _, hyp _ => by simp [totalCost, defaultCost, steps, RuleKind.cost]
  | _, refine hp _ => by
      rw [totalCost, steps, totalCost_default_eq_steps hp]
      simp [defaultCost, RuleKind.cost]
  | _, merge _ hp0 hp1 _ _ => by
      rw [totalCost, steps, totalCost_default_eq_steps hp0, totalCost_default_eq_steps hp1]
      simp [defaultCost, RuleKind.cost]

/-- A proof's operation ledger is the triple of hypothesis, refinement, and merge counts. -/
def ledger {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (hp : PatternProof base P) : Nat × Nat × Nat :=
  (hp.hypCount, hp.refineCount, hp.mergeCount)

/-- Proof `hp` is no more expensive than proof `hq` under cost model `C`. -/
def CostLE {n : Nat} {base : List (Pattern n)} {P Q : Pattern n}
    (C : CostModel) (hp : PatternProof base P) (hq : PatternProof base Q) : Prop :=
  hp.totalCost C ≤ hq.totalCost C

/-- Proof `hp` is strictly cheaper than proof `hq` under cost model `C`. -/
def CostLT {n : Nat} {base : List (Pattern n)} {P Q : Pattern n}
    (C : CostModel) (hp : PatternProof base P) (hq : PatternProof base Q) : Prop :=
  hp.totalCost C < hq.totalCost C

/-- A proof is cost-bounded by `c` under cost model `C`. -/
def CostBounded {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (C : CostModel) (hp : PatternProof base P) (c : Nat) : Prop :=
  hp.totalCost C ≤ c

/-- `hp` is a minimum-cost proof of its conclusion under model `C`. -/
def IsMinCostProof {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (C : CostModel) (hp : PatternProof base P) : Prop :=
  ∀ hq : PatternProof base P, hp.totalCost C ≤ hq.totalCost C

/-- `hp` is a minimum-width proof of its conclusion. -/
def IsMinWidthProof {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (hp : PatternProof base P) : Prop :=
  ∀ hq : PatternProof base P, hp.maxWidth ≤ hq.maxWidth

/-- `hp` lies within both a cost and width envelope. -/
def WithinEnvelope {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (C : CostModel) (hp : PatternProof base P) (cost width : Nat) : Prop :=
  hp.totalCost C ≤ cost ∧ hp.maxWidth ≤ width

/-- A proof is width-bounded by `w` when every intermediate pattern has width at most `w`. -/
def WidthBounded {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (hp : PatternProof base P) (w : Nat) : Prop :=
  hp.maxWidth ≤ w

/-- A proof is step-bounded by `s` when it uses at most `s` non-hypothesis operations. -/
def StepBounded {n : Nat} {base : List (Pattern n)} {P : Pattern n}
    (hp : PatternProof base P) (s : Nat) : Prop :=
  hp.steps ≤ s

end PatternProof

/-- There exists a derivation of `P` from `base` whose width never exceeds `w`. -/
def ExistsWidthBoundedProof {n : Nat} (base : List (Pattern n)) (P : Pattern n) (w : Nat) :
    Prop :=
  ∃ hp : PatternProof base P, hp.WidthBounded w

/-- There exists a derivation of `P` from `base` using at most `s` non-hypothesis operations. -/
def ExistsStepBoundedProof {n : Nat} (base : List (Pattern n)) (P : Pattern n) (s : Nat) :
    Prop :=
  ∃ hp : PatternProof base P, hp.StepBounded s

/-- Any width-bounded proof is still semantically sound; width is extra accounting. -/
theorem sound_of_existsWidthBoundedProof {n : Nat} {base : List (Pattern n)}
    {P : Pattern n} {w : Nat} (h : ExistsWidthBoundedProof base P w) :
    P.SemanticallyCoveredBy base := by
  obtain ⟨hp, -⟩ := h
  exact hp.sound

/-- The base patterns have a derivational full-cover certificate when they derive the empty
pattern. This is the geometric version of an UNSAT proof. -/
def HasFullCoverCertificate {n : Nat} (base : List (Pattern n)) : Prop :=
  Nonempty (PatternProof base (Pattern.empty n))

/-- Full-cover certificates can be restricted by a maximum width envelope. -/
def HasWidthBoundedFullCoverCertificate {n : Nat} (base : List (Pattern n)) (w : Nat) :
    Prop :=
  ExistsWidthBoundedProof base (Pattern.empty n) w

/-- Full-cover certificates can be restricted by an operation-count envelope. -/
def HasStepBoundedFullCoverCertificate {n : Nat} (base : List (Pattern n)) (s : Nat) :
    Prop :=
  ExistsStepBoundedProof base (Pattern.empty n) s

/-- Every full-cover certificate in this proof system has width at least `w`. -/
def FullCoverCertificateWidthAtLeast {n : Nat} (base : List (Pattern n)) (w : Nat) :
    Prop :=
  ∀ hp : PatternProof base (Pattern.empty n), w ≤ hp.maxWidth

/-- Every full-cover certificate in this proof system has at least `s` non-hypothesis steps. -/
def FullCoverCertificateStepsAtLeast {n : Nat} (base : List (Pattern n)) (s : Nat) :
    Prop :=
  ∀ hp : PatternProof base (Pattern.empty n), s ≤ hp.steps

/-- Width lower bounds rule out smaller width-bounded certificates. -/
theorem not_widthBounded_certificate_of_widthAtLeast_succ {n : Nat}
    {base : List (Pattern n)} {w : Nat}
    (hwidth : FullCoverCertificateWidthAtLeast base (w + 1)) :
    ¬ HasWidthBoundedFullCoverCertificate base w := by
  rintro ⟨hp, hbound⟩
  have hmin := hwidth hp
  unfold PatternProof.WidthBounded at hbound
  omega

/-- Step lower bounds rule out smaller step-bounded certificates. -/
theorem not_stepBounded_certificate_of_stepsAtLeast_succ {n : Nat}
    {base : List (Pattern n)} {s : Nat}
    (hsteps : FullCoverCertificateStepsAtLeast base (s + 1)) :
    ¬ HasStepBoundedFullCoverCertificate base s := by
  rintro ⟨hp, hbound⟩
  have hmin := hsteps hp
  unfold PatternProof.StepBounded at hbound
  omega

/-- A proof-complexity envelope for UNSAT-only study: the family has a full-cover
certificate, but every certificate must cross the given width and step thresholds. -/
def HardUNSATProofEnvelope {n : Nat} (base : List (Pattern n)) (width steps : Nat) : Prop :=
  HasFullCoverCertificate base ∧
  FullCoverCertificateWidthAtLeast base width ∧
  FullCoverCertificateStepsAtLeast base steps

/-- Proof-complexity hardness phrased as absence of both low-width and low-step full-cover
certificates. -/
def ProofHardAt {n : Nat} (base : List (Pattern n)) (width steps : Nat) : Prop :=
  HardUNSATProofEnvelope base width steps

/-- A proof-system view chooses a rule system and a cost model for geometric collapse
certificates. Different choices model resolution-like, bounded-width, or custom blocker
calculus studies. -/
structure GeometricProofSystem where
  rules : PatternProof.RuleSystem
  cost : PatternProof.CostModel

/-- A proof belongs to a geometric proof system when it uses only the allowed rules. -/
def GeometricProofSystem.Accepts {n : Nat} {base : List (Pattern n)}
    (S : GeometricProofSystem) {P : Pattern n} (hp : PatternProof base P) : Prop :=
  hp.UsesOnly S.rules

/-- A proof-system-specific full-cover certificate. -/
def HasSystemFullCoverCertificate {n : Nat} (S : GeometricProofSystem)
    (base : List (Pattern n)) : Prop :=
  ∃ hp : PatternProof base (Pattern.empty n), S.Accepts hp

/-- Width lower bound inside a chosen proof system. -/
def SystemFullCoverWidthAtLeast {n : Nat} (S : GeometricProofSystem)
    (base : List (Pattern n)) (w : Nat) : Prop :=
  ∀ hp : PatternProof base (Pattern.empty n), S.Accepts hp → w ≤ hp.maxWidth

/-- Step lower bound inside a chosen proof system. -/
def SystemFullCoverStepsAtLeast {n : Nat} (S : GeometricProofSystem)
    (base : List (Pattern n)) (s : Nat) : Prop :=
  ∀ hp : PatternProof base (Pattern.empty n), S.Accepts hp → s ≤ hp.steps

/-- System-relative proof hardness: there is a certificate, but none below the requested
width/step thresholds in the chosen rule system. -/
def SystemProofHardAt {n : Nat} (S : GeometricProofSystem)
    (base : List (Pattern n)) (width steps : Nat) : Prop :=
  HasSystemFullCoverCertificate S base ∧
  SystemFullCoverWidthAtLeast S base width ∧
  SystemFullCoverStepsAtLeast S base steps

/-- The full rule system accepts every proof. -/
def fullGeometricProofSystem : GeometricProofSystem where
  rules := PatternProof.fullRuleSystem
  cost := PatternProof.defaultCost

/-- System hardness for the full rule system implies the older system-free hardness
envelope. -/
theorem hardUNSATProofEnvelope_of_fullSystemHard {n : Nat}
    {base : List (Pattern n)} {width steps : Nat}
    (hhard : SystemProofHardAt fullGeometricProofSystem base width steps) :
    HardUNSATProofEnvelope base width steps := by
  rcases hhard with ⟨hcert, hwidth, hsteps⟩
  constructor
  · obtain ⟨hp, -⟩ := hcert
    exact ⟨hp⟩
  · constructor
    · intro hp
      exact hwidth hp (PatternProof.usesOnly_fullRuleSystem hp)
    · intro hp
      exact hsteps hp (PatternProof.usesOnly_fullRuleSystem hp)

/-- No certified UNSAT family can require proof width above the ambient `2n` ceiling in this
proof language. This rules out impossible "worst-case" width thresholds. -/
theorem not_HardUNSATProofEnvelope_width_gt_ambient {n : Nat}
    {base : List (Pattern n)} {width steps : Nat} (hwidth : 2 * n < width) :
    ¬ HardUNSATProofEnvelope base width steps := by
  rintro ⟨hcert, hwidthLower, -⟩
  rcases hcert with ⟨hp⟩
  have hlo : width ≤ hp.maxWidth := hwidthLower hp
  have hhi : hp.maxWidth ≤ 2 * n := hp.maxWidth_le_two_mul_n
  omega

/-- Any full-cover certificate semantically covers the whole hypercube. -/
theorem full_cover_of_certificate {n : Nat} {base : List (Pattern n)}
    (hcert : HasFullCoverCertificate base) :
    ∀ v : Vertex n, ∃ P ∈ base, P.Covers v := by
  rcases hcert with ⟨hp⟩
  exact full_cover_of_empty_pattern_covered base hp.sound

/-- Width-bounded full-cover certificates are sound full-cover certificates. -/
theorem full_cover_of_widthBounded_certificate {n : Nat} {base : List (Pattern n)} {w : Nat}
    (hcert : HasWidthBoundedFullCoverCertificate base w) :
    ∀ v : Vertex n, ∃ P ∈ base, P.Covers v := by
  exact full_cover_of_empty_pattern_covered base (sound_of_existsWidthBoundedProof hcert)

/-- Step-bounded full-cover certificates are sound full-cover certificates. -/
theorem full_cover_of_stepBounded_certificate {n : Nat} {base : List (Pattern n)} {s : Nat}
    (hcert : HasStepBoundedFullCoverCertificate base s) :
    ∀ v : Vertex n, ∃ P ∈ base, P.Covers v := by
  obtain ⟨hp, -⟩ := hcert
  exact full_cover_of_empty_pattern_covered base hp.sound

/-- Width never exceeds the ambient dimension plus a bit-choice per fixed coordinate. This
coarse bound is often enough for operation-envelope arguments. -/
theorem Pattern.width_le_card_fixed_univ {n : Nat} (P : Pattern n) :
    P.width ≤ Fintype.card (Fin n × Bool) := by
  unfold Pattern.width
  exact Finset.card_le_univ P.fixed

/-- Upper bound on the number of patterns of width at most `w`:
choose the fixed coordinates, then fix each to one of two bits. -/
def patternCountUpper (n w : Nat) : Nat :=
  ∑ r ∈ Finset.range (w + 1), Nat.choose n r * 2 ^ r

/-- Each variable is free / fixed-0 / fixed-1, so there are `3ⁿ` patterns in all. -/
def unrestrictedPatternCount (n : Nat) : Nat := 3 ^ n

/-- For fixed width `w`, the bounded-width pattern count is polynomial in `n` (polynomial side).
Proof sketch: `Nat.choose n r ≤ (n+1)^w` and `2^r ≤ 2^w` for `r ≤ w`, and there are `w+1` terms,
so `patternCountUpper n w ≤ (w+1)·2^w·(n+1)^w`. -/
theorem bounded_width_patterns_poly (w : Nat) :
    PolyBound (fun n => patternCountUpper n w) := by
  refine ⟨(w + 1) * 2 ^ w, w, fun n => ?_⟩
  unfold patternCountUpper
  have hterm :
      ∀ r ∈ Finset.range (w + 1),
        Nat.choose n r * 2 ^ r ≤ (n + 1) ^ w * 2 ^ w := by
    intro r hr
    have hrw : r ≤ w := by
      exact Nat.lt_succ_iff.mp (Finset.mem_range.mp hr)
    have hchoose : Nat.choose n r ≤ (n + 1) ^ w := by
      calc
        Nat.choose n r ≤ n ^ r := Nat.choose_le_pow n r
        _ ≤ (n + 1) ^ r := Nat.pow_le_pow_left (Nat.le_succ n) r
        _ ≤ (n + 1) ^ w := Nat.pow_le_pow_right (by omega) hrw
    have hbits : 2 ^ r ≤ 2 ^ w :=
      Nat.pow_le_pow_right (by omega) hrw
    exact Nat.mul_le_mul hchoose hbits
  have hsum :=
    Finset.sum_le_card_nsmul (Finset.range (w + 1))
      (fun r => Nat.choose n r * 2 ^ r) ((n + 1) ^ w * 2 ^ w) hterm
  calc
    (∑ r ∈ Finset.range (w + 1), Nat.choose n r * 2 ^ r)
        ≤ (Finset.range (w + 1)).card * ((n + 1) ^ w * 2 ^ w) := by
          simpa [nsmul_eq_mul] using hsum
    _ = (w + 1) * 2 ^ w * (n + 1) ^ w := by
          rw [Finset.card_range]
          ring

/-- The unrestricted pattern space has exactly `3ⁿ` elements (exponential side). -/
theorem unrestricted_patterns_eq_three_pow (n : Nat) :
    unrestrictedPatternCount n = 3 ^ n := rfl

/-! ## §F Normalized: unique cover normalized to `0ⁿ` -/

/-- Normalized blocker for a unique cover with survivor `0ⁿ`.
`D` = coordinates forced to `1`, `A` = coordinates forced to `0`; `D` is nonempty
so the blocker does not block `0ⁿ`. -/
structure NormBlocker (n : Nat) where
  D : Finset (Fin n)
  A : Finset (Fin n)
  hdisj : Disjoint D A
  hwidth : D.card + A.card = 3
  hD_nonempty : D.Nonempty
  deriving DecidableEq, Fintype

/-- A normalized blocker blocks the flipped-coordinate set `T`. -/
def NormBlocker.Blocks {n : Nat} (B : NormBlocker n) (T : Finset (Fin n)) : Prop :=
  B.D ⊆ T ∧ Disjoint B.A T

/-- The flipped-coordinate set of a vertex, relative to the normalized survivor `0ⁿ`. -/
def flippedSet {n : Nat} (v : Vertex n) : Finset (Fin n) :=
  Finset.univ.filter fun i => v i = true

@[simp] theorem mem_flippedSet {n : Nat} (v : Vertex n) (i : Fin n) :
    i ∈ flippedSet v ↔ v i = true := by
  simp [flippedSet]

/-- Turn a flipped-coordinate set back into its Boolean vertex. -/
def vertexOfFlippedSet {n : Nat} (T : Finset (Fin n)) : Vertex n :=
  fun i => decide (i ∈ T)

@[simp] theorem vertexOfFlippedSet_true {n : Nat} (T : Finset (Fin n)) {i : Fin n}
    (hi : i ∈ T) :
    vertexOfFlippedSet T i = true := by
  simp [vertexOfFlippedSet, hi]

@[simp] theorem vertexOfFlippedSet_false {n : Nat} (T : Finset (Fin n)) {i : Fin n}
    (hi : i ∉ T) :
    vertexOfFlippedSet T i = false := by
  simp [vertexOfFlippedSet, hi]

/-- Converting a flipped set to a vertex and back recovers the same set. -/
@[simp] theorem flippedSet_vertexOfFlippedSet {n : Nat} (T : Finset (Fin n)) :
    flippedSet (vertexOfFlippedSet T) = T := by
  ext i
  by_cases hi : i ∈ T <;> simp [hi]

@[simp] theorem flippedSet_zeroVertex (n : Nat) :
    flippedSet (zeroVertex n) = ∅ := by
  ext i
  simp [flippedSet, zeroVertex]

/-- A vertex has empty flipped set exactly when it is the normalized survivor `0ⁿ`. -/
theorem flippedSet_eq_empty_iff {n : Nat} (v : Vertex n) :
    flippedSet v = ∅ ↔ v = zeroVertex n := by
  constructor
  · intro h
    funext i
    by_cases hv : v i = true
    · have hi : i ∈ flippedSet v := (mem_flippedSet v i).mpr hv
      rw [h] at hi
      simp at hi
    · have hvfalse : v i = false := Bool.eq_false_of_not_eq_true hv
      simp [zeroVertex, hvfalse]
  · intro h
    rw [h, flippedSet_zeroVertex]

/-- A normalized blocker blocks a vertex when it blocks the vertex's flipped set. -/
def NormBlocker.BlocksVertex {n : Nat} (B : NormBlocker n) (v : Vertex n) : Prop :=
  B.Blocks (flippedSet v)

/-- No normalized blocker blocks the survivor `0ⁿ`, represented by the empty flipped set. -/
theorem NormBlocker.not_blocks_empty {n : Nat} (B : NormBlocker n) :
    ¬ B.Blocks ∅ := by
  intro hblocks
  have hDempty : B.D = ∅ := by
    ext i
    constructor
    · intro hi
      exact hblocks.1 hi
    · intro hi
      simp at hi
  exact B.hD_nonempty.ne_empty hDempty

/-- The blocker's type is the size of `D`. -/
def NormBlocker.type {n : Nat} (B : NormBlocker n) : Nat := B.D.card

def NormBlocker.Type1 {n : Nat} (B : NormBlocker n) : Prop := B.D.card = 1
def NormBlocker.Type2 {n : Nat} (B : NormBlocker n) : Prop := B.D.card = 2
def NormBlocker.Type3 {n : Nat} (B : NormBlocker n) : Prop := B.D.card = 3

/-! ### §F1 Ring rules -/

/-- Ring membership for flipped-coordinate sets: ring `r` is Hamming weight `r`. -/
def InRing {n : Nat} (T : Finset (Fin n)) (r : Nat) : Prop :=
  T.card = r

/-- The whole Hamming ring `r`, represented as flipped-coordinate sets. -/
def ringSets (n r : Nat) : Finset (Finset (Fin n)) :=
  Finset.powersetCard r (Finset.univ : Finset (Fin n))

@[simp] theorem mem_ringSets {n r : Nat} {T : Finset (Fin n)} :
    T ∈ ringSets n r ↔ InRing T r := by
  simp [ringSets, InRing]

/-- Ring `r` has `choose n r` vertices. -/
theorem ringSets_card (n r : Nat) :
    (ringSets n r).card = Nat.choose n r := by
  rw [ringSets, Finset.card_powersetCard, Finset.card_univ, Fintype.card_fin]

/-- A normalized blocker hits ring `r` when it blocks at least one set of size `r`. -/
def NormBlocker.HitsRing {n : Nat} (B : NormBlocker n) (r : Nat) : Prop :=
  ∃ T : Finset (Fin n), InRing T r ∧ B.Blocks T

/-- The footprint of one normalized blocker on ring `r`: all ring-`r` vertices it blocks,
represented by their flipped-coordinate sets. -/
noncomputable def NormBlocker.ringFootprint {n : Nat} (B : NormBlocker n) (r : Nat) :
    Finset (Finset (Fin n)) :=
  by
    classical
    exact Finset.univ.filter fun T => InRing T r ∧ B.Blocks T

@[simp] theorem NormBlocker.mem_ringFootprint {n r : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} :
    T ∈ B.ringFootprint r ↔ InRing T r ∧ B.Blocks T := by
  classical
  simp [NormBlocker.ringFootprint]

/-- A footprint is nonempty exactly when the blocker hits that ring. -/
theorem NormBlocker.ringFootprint_nonempty_iff_hitsRing {n r : Nat}
    (B : NormBlocker n) :
    (B.ringFootprint r).Nonempty ↔ B.HitsRing r := by
  constructor
  · rintro ⟨T, hT⟩
    exact ⟨T, (B.mem_ringFootprint.mp hT).1, (B.mem_ringFootprint.mp hT).2⟩
  · rintro ⟨T, hring, hblocks⟩
    exact ⟨T, B.mem_ringFootprint.mpr ⟨hring, hblocks⟩⟩

/-- A blocker's ring footprint is always a subset of the whole ring. -/
theorem NormBlocker.ringFootprint_subset_ringSets {n r : Nat}
    (B : NormBlocker n) :
    B.ringFootprint r ⊆ ringSets n r := by
  intro T hT
  exact mem_ringSets.mpr (B.mem_ringFootprint.mp hT).1

/-- A single blocker's ring footprint cannot be larger than the whole ring. -/
theorem NormBlocker.ringFootprint_card_le_ring_card {n r : Nat}
    (B : NormBlocker n) :
    (B.ringFootprint r).card ≤ (ringSets n r).card :=
  Finset.card_le_card (B.ringFootprint_subset_ringSets)

/-- A single blocker's ring footprint is bounded by `choose n r`. -/
theorem NormBlocker.ringFootprint_card_le_choose {n r : Nat}
    (B : NormBlocker n) :
    (B.ringFootprint r).card ≤ Nat.choose n r := by
  rw [← ringSets_card n r]
  exact B.ringFootprint_card_le_ring_card

/-- Type `t` is feasible on ring `r` in dimension `n` exactly when the positive side
fits inside the ring and the zero-side guards fit outside it. -/
def RingTypeFeasible (n r t : Nat) : Prop :=
  1 ≤ t ∧ t ≤ 3 ∧ t ≤ r ∧ r + (3 - t) ≤ n

/-- The generic ring rule: if a blocker of type `t` blocks a ring-`r` set, then `t`
is feasible for that ring. Most low/high-ring rules are just this statement specialized. -/
theorem NormBlocker.ringTypeFeasible_of_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} {r : Nat} (hcard : InRing T r) (hblocks : B.Blocks T) :
    RingTypeFeasible n r B.type := by
  unfold RingTypeFeasible NormBlocker.type InRing at *
  have hDpos : 1 ≤ B.D.card := Finset.card_pos.mpr B.hD_nonempty
  have hDle3 : B.D.card ≤ 3 := by
    have hw := B.hwidth
    omega
  have hDleT : B.D.card ≤ T.card := Finset.card_le_card hblocks.1
  have hAfit : B.A.card + T.card ≤ n := by
    have hsub : B.A ∪ T ⊆ (Finset.univ : Finset (Fin n)) := by
      intro x hx
      exact Finset.mem_univ x
    have hle := Finset.card_le_card hsub
    rw [Finset.card_union_of_disjoint hblocks.2, Finset.card_univ, Fintype.card_fin] at hle
    exact hle
  have hw := B.hwidth
  rw [← hcard]
  refine ⟨hDpos, hDle3, hDleT, ?_⟩
  omega

/-- A blocker hits ring `r` only if its type is feasible there. -/
theorem NormBlocker.ringTypeFeasible_of_hitsRing {n : Nat} (B : NormBlocker n)
    {r : Nat} (hhit : B.HitsRing r) :
    RingTypeFeasible n r B.type := by
  obtain ⟨T, hcard, hblocks⟩ := hhit
  exact B.ringTypeFeasible_of_blocks hcard hblocks

/-- If the type/ring combination is infeasible, the footprint is empty. -/
theorem NormBlocker.ringFootprint_eq_empty_of_not_feasible {n r : Nat}
    (B : NormBlocker n) (hbad : ¬ RingTypeFeasible n r B.type) :
    B.ringFootprint r = ∅ := by
  ext T
  simp only [NormBlocker.mem_ringFootprint, Finset.notMem_empty, iff_false, not_and]
  intro hring hblocks
  exact hbad (B.ringTypeFeasible_of_blocks hring hblocks)

/-- The survivor ring has empty footprint for every normalized blocker. -/
theorem NormBlocker.ringFootprint_zero_eq_empty {n : Nat} (B : NormBlocker n) :
    B.ringFootprint 0 = ∅ := by
  ext T
  simp only [NormBlocker.mem_ringFootprint, Finset.notMem_empty, iff_false, not_and]
  intro hring hblocks
  unfold InRing at hring
  have hTempty : T = ∅ := Finset.card_eq_zero.mp hring
  subst T
  exact B.not_blocks_empty hblocks

/-- Ring `0` is the survivor ring: no normalized blocker can hit it. -/
theorem NormBlocker.not_hitsRing_zero {n : Nat} (B : NormBlocker n) :
    ¬ B.HitsRing 0 := by
  rintro ⟨T, hcard, hblocks⟩
  have hTempty : T = ∅ := Finset.card_eq_zero.mp hcard
  exact B.not_blocks_empty (by simpa [hTempty] using hblocks)

/-- Ring membership for vertices by Hamming weight from `0ⁿ`. -/
def VertexInRing {n : Nat} (v : Vertex n) (r : Nat) : Prop :=
  InRing (flippedSet v) r

/-- A blocker hits the ring of any vertex it blocks. -/
theorem NormBlocker.ringTypeFeasible_of_blocksVertex {n r : Nat} (B : NormBlocker n)
    {v : Vertex n} (hring : VertexInRing v r) (hblocks : B.BlocksVertex v) :
    RingTypeFeasible n r B.type :=
  B.ringTypeFeasible_of_blocks hring hblocks

/-- The first ring can only be hit by type-1 blockers. -/
theorem NormBlocker.type1_of_blocks_ring_one {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (hcard : InRing T 1) (hblocks : B.Blocks T) :
    B.Type1 := by
  have hfeas := B.ringTypeFeasible_of_blocks hcard hblocks
  unfold RingTypeFeasible NormBlocker.type at hfeas
  unfold NormBlocker.Type1
  omega

/-- A blocker that hits the first ring is type 1. -/
theorem NormBlocker.type1_of_hitsRing_one {n : Nat} (B : NormBlocker n)
    (hhit : B.HitsRing 1) :
    B.Type1 := by
  obtain ⟨T, hcard, hblocks⟩ := hhit
  exact B.type1_of_blocks_ring_one hcard hblocks

/-- The top ring can only be hit by type-3 blockers. -/
theorem NormBlocker.type3_of_blocks_top_ring {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (hcard : InRing T n) (hblocks : B.Blocks T) :
    B.Type3 := by
  unfold NormBlocker.Type3
  have hfeas := B.ringTypeFeasible_of_blocks hcard hblocks
  unfold RingTypeFeasible NormBlocker.type at hfeas
  omega

/-- The next-to-top ring can only be hit by type-2 or type-3 blockers. -/
theorem NormBlocker.type2_or_type3_of_blocks_next_to_top_ring {n : Nat}
    (B : NormBlocker n) {T : Finset (Fin n)}
    (hcard : InRing T (n - 1)) (hnT : T.card + 1 = n) (hblocks : B.Blocks T) :
    B.Type2 ∨ B.Type3 := by
  have hfeas := B.ringTypeFeasible_of_blocks hcard hblocks
  unfold RingTypeFeasible NormBlocker.type at hfeas
  unfold NormBlocker.Type2 NormBlocker.Type3
  omega

/-- If a blocker blocks singleton `{i}`, then its positive side is exactly `{i}`. -/
theorem NormBlocker.D_eq_singleton_of_blocks_singleton {n : Nat} (B : NormBlocker n)
    (i : Fin n) (hblocks : B.Blocks {i}) :
    B.D = {i} := by
  rcases Finset.subset_singleton_iff.mp hblocks.1 with hDempty | hD
  · exact absurd hDempty B.hD_nonempty.ne_empty
  · exact hD

/-- If a blocker blocks a singleton, it is type 1. -/
theorem NormBlocker.type1_of_blocks_singleton {n : Nat} (B : NormBlocker n)
    (i : Fin n) (hblocks : B.Blocks {i}) :
    B.Type1 := by
  unfold NormBlocker.Type1
  rw [B.D_eq_singleton_of_blocks_singleton i hblocks, Finset.card_singleton]

/-- A blocker can only block sets at least as large as its positive side. -/
theorem NormBlocker.D_card_le_card_of_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (hblocks : B.Blocks T) :
    B.D.card ≤ T.card :=
  Finset.card_le_card hblocks.1

/-- A blocker's type is bounded by the Hamming weight of any flipped set it blocks. -/
theorem NormBlocker.type_le_card_of_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (hblocks : B.Blocks T) :
    B.type ≤ T.card :=
  B.D_card_le_card_of_blocks hblocks

/-- Type-2 blockers only hit flipped sets of size at least `2`. -/
theorem NormBlocker.card_ge_two_of_type2_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (h2 : B.Type2) (hblocks : B.Blocks T) :
    2 ≤ T.card := by
  unfold NormBlocker.Type2 at h2
  have hle := B.D_card_le_card_of_blocks hblocks
  omega

/-- Type-3 blockers only hit flipped sets of size at least `3`. -/
theorem NormBlocker.card_ge_three_of_type3_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (h3 : B.Type3) (hblocks : B.Blocks T) :
    3 ≤ T.card := by
  unfold NormBlocker.Type3 at h3
  have hle := B.D_card_le_card_of_blocks hblocks
  omega

/-- Type-1 blockers live on rings `1, ..., n-2`. -/
theorem NormBlocker.type1_ring_bounds_of_blocks {n r : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (h1 : B.Type1) (hring : InRing T r) (hblocks : B.Blocks T) :
    1 ≤ r ∧ r + 2 ≤ n := by
  unfold InRing at hring
  have hlow := B.D_card_le_card_of_blocks hblocks
  have hAfit : B.A.card + T.card ≤ n := by
    have hsub : B.A ∪ T ⊆ (Finset.univ : Finset (Fin n)) := by
      intro x hx
      exact Finset.mem_univ x
    have hle := Finset.card_le_card hsub
    rw [Finset.card_union_of_disjoint hblocks.2, Finset.card_univ, Fintype.card_fin] at hle
    exact hle
  have hw := B.hwidth
  unfold NormBlocker.Type1 at h1
  rw [← hring]
  omega

/-- Type-2 blockers live on rings `2, ..., n-1`. -/
theorem NormBlocker.type2_ring_bounds_of_blocks {n r : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (h2 : B.Type2) (hring : InRing T r) (hblocks : B.Blocks T) :
    2 ≤ r ∧ r + 1 ≤ n := by
  unfold InRing at hring
  have hlow := B.card_ge_two_of_type2_blocks h2 hblocks
  have hAfit : B.A.card + T.card ≤ n := by
    have hsub : B.A ∪ T ⊆ (Finset.univ : Finset (Fin n)) := by
      intro x hx
      exact Finset.mem_univ x
    have hle := Finset.card_le_card hsub
    rw [Finset.card_union_of_disjoint hblocks.2, Finset.card_univ, Fintype.card_fin] at hle
    exact hle
  have hw := B.hwidth
  unfold NormBlocker.Type2 at h2
  rw [← hring]
  omega

/-- Type-3 blockers live on rings `3, ..., n`. -/
theorem NormBlocker.type3_ring_bounds_of_blocks {n r : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (h3 : B.Type3) (hring : InRing T r) (hblocks : B.Blocks T) :
    3 ≤ r ∧ r ≤ n := by
  unfold InRing at hring
  have hlow := B.card_ge_three_of_type3_blocks h3 hblocks
  have hsub : T ⊆ (Finset.univ : Finset (Fin n)) := by
    intro i hi
    exact Finset.mem_univ i
  have hhigh := Finset.card_le_card hsub
  rw [Finset.card_univ, Fintype.card_fin] at hhigh
  rw [hring] at hlow hhigh
  exact ⟨hlow, hhigh⟩

/-- A blocker can only block `T` if its zero-side guards fit outside `T`. -/
theorem NormBlocker.A_card_add_card_le_of_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (hblocks : B.Blocks T) :
    B.A.card + T.card ≤ n := by
  have hsub : B.A ∪ T ⊆ (Finset.univ : Finset (Fin n)) := by
    intro x hx
    exact Finset.mem_univ x
  have hle := Finset.card_le_card hsub
  rw [Finset.card_union_of_disjoint hblocks.2, Finset.card_univ, Fintype.card_fin] at hle
  exact hle

/-- Type-1 blockers only hit flipped sets with at least two coordinates left unflipped. -/
theorem NormBlocker.type1_card_add_two_le_of_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (h1 : B.Type1) (hblocks : B.Blocks T) :
    T.card + 2 ≤ n := by
  have hAfit := B.A_card_add_card_le_of_blocks hblocks
  unfold NormBlocker.Type1 at h1
  have hw := B.hwidth
  omega

/-- Type-2 blockers only hit flipped sets with at least one coordinate left unflipped. -/
theorem NormBlocker.type2_card_add_one_le_of_blocks {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (h2 : B.Type2) (hblocks : B.Blocks T) :
    T.card + 1 ≤ n := by
  have hAfit := B.A_card_add_card_le_of_blocks hblocks
  unfold NormBlocker.Type2 at h2
  have hw := B.hwidth
  omega

/-- A blocked set with all `n` coordinates flipped can only be hit by a type-3 blocker. -/
theorem NormBlocker.type3_of_blocks_card_eq_n {n : Nat} (B : NormBlocker n)
    {T : Finset (Fin n)} (hcard : T.card = n) (hblocks : B.Blocks T) :
    B.Type3 := by
  unfold NormBlocker.Type3
  have hAfit := B.A_card_add_card_le_of_blocks hblocks
  have hw := B.hwidth
  omega

/-- A blocked set with `n-1` coordinates flipped can only be hit by type 2 or type 3. -/
theorem NormBlocker.type2_or_type3_of_blocks_card_add_one_eq_n {n : Nat}
    (B : NormBlocker n) {T : Finset (Fin n)}
    (hcard : T.card + 1 = n) (hblocks : B.Blocks T) :
    B.Type2 ∨ B.Type3 := by
  unfold NormBlocker.Type2 NormBlocker.Type3
  have hAfit := B.A_card_add_card_le_of_blocks hblocks
  have hpos : 0 < B.D.card := Finset.card_pos.mpr B.hD_nonempty
  have hw := B.hwidth
  omega

/-- The blockers cover every nonempty flipped-coordinate set. -/
def CoversAllNonempty {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  ∀ T : Finset (Fin n), T.Nonempty → ∃ B ∈ Bs, B.Blocks T

/-- Vertex-first version: the blockers cover every vertex except the normalized survivor. -/
def CoversAllNonzeroVertices {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  ∀ v : Vertex n, v ≠ zeroVertex n → ∃ B ∈ Bs, B.BlocksVertex v

/-- Covering all nonempty flipped sets is the same as covering all nonzero vertices. -/
theorem coversAllNonempty_iff_coversAllNonzeroVertices {n : Nat}
    (Bs : Finset (NormBlocker n)) :
    CoversAllNonempty Bs ↔ CoversAllNonzeroVertices Bs := by
  constructor
  · intro hcover v hv
    have hflips : (flippedSet v).Nonempty := by
      apply Finset.nonempty_iff_ne_empty.mpr
      intro hempty
      exact hv ((flippedSet_eq_empty_iff v).mp hempty)
    exact hcover (flippedSet v) hflips
  · intro hcover T hT
    let v := vertexOfFlippedSet T
    have hv : v ≠ zeroVertex n := by
      intro hz
      have hEmpty : T = ∅ := by
        rw [← flippedSet_vertexOfFlippedSet T]
        change flippedSet v = ∅
        rw [hz, flippedSet_zeroVertex]
      exact hT.ne_empty hEmpty
    obtain ⟨B, hB, hblocks⟩ := hcover v hv
    exact ⟨B, hB, by simpa [NormBlocker.BlocksVertex, v] using hblocks⟩

/-- Any cover of all nonempty sets covers each nonzero ring. -/
theorem CoversAllNonempty.exists_blocker_on_ring {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs)
    {T : Finset (Fin n)} (hcard : InRing T r) (hr : 0 < r) :
    ∃ B ∈ Bs, B.Blocks T := by
  have hTpos : 0 < T.card := by
    unfold InRing at hcard
    rw [hcard]
    exact hr
  exact hcover T (Finset.card_pos.mp hTpos)

/-- Every blocker chosen for a nonzero ring in a cover has a feasible type for that ring. -/
theorem CoversAllNonempty.exists_feasible_type_on_ring {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs)
    {T : Finset (Fin n)} (hcard : InRing T r) (hr : 0 < r) :
    ∃ B ∈ Bs, B.Blocks T ∧ RingTypeFeasible n r B.type := by
  obtain ⟨B, hB, hblocks⟩ := hcover.exists_blocker_on_ring hcard hr
  exact ⟨B, hB, hblocks, B.ringTypeFeasible_of_blocks hcard hblocks⟩

/-- Footprint version: every nonzero ring vertex lies in some blocker's footprint. -/
theorem CoversAllNonempty.exists_ringFootprint_member {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs)
    {T : Finset (Fin n)} (hcard : InRing T r) (hr : 0 < r) :
    ∃ B ∈ Bs, T ∈ B.ringFootprint r := by
  obtain ⟨B, hB, hblocks⟩ := hcover.exists_blocker_on_ring hcard hr
  exact ⟨B, hB, B.mem_ringFootprint.mpr ⟨hcard, hblocks⟩⟩

/-- Footprint version with feasibility: the footprint covering a nonzero ring vertex has
a feasible type for that ring. -/
theorem CoversAllNonempty.exists_feasible_ringFootprint_member {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs)
    {T : Finset (Fin n)} (hcard : InRing T r) (hr : 0 < r) :
    ∃ B ∈ Bs, T ∈ B.ringFootprint r ∧ RingTypeFeasible n r B.type := by
  obtain ⟨B, hB, hblocks, hfeasible⟩ := hcover.exists_feasible_type_on_ring hcard hr
  exact ⟨B, hB, B.mem_ringFootprint.mpr ⟨hcard, hblocks⟩, hfeasible⟩

/-- Ring-cover rule: on every nonzero ring, the union of blocker footprints covers the
whole ring. This is the finite combinatorial object that search scripts can target. -/
theorem CoversAllNonempty.ringSets_subset_biUnion_ringFootprints {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs) (hr : 0 < r) :
    ringSets n r ⊆ Bs.biUnion (fun B => B.ringFootprint r) := by
  intro T hT
  have hring : InRing T r := mem_ringSets.mp hT
  obtain ⟨B, hB, hFoot⟩ := hcover.exists_ringFootprint_member hring hr
  exact Finset.mem_biUnion.mpr ⟨B, hB, hFoot⟩

/-- Counting shadow of the ring-cover rule: each nonzero ring has at most the total
footprint capacity of the blockers on that ring. Overlaps make this only an upper
capacity bound, which is exactly why worst cases are delicate. -/
theorem CoversAllNonempty.ring_card_le_sum_ringFootprints {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs) (hr : 0 < r) :
    (ringSets n r).card ≤ ∑ B ∈ Bs, (B.ringFootprint r).card := by
  have hsub := hcover.ringSets_subset_biUnion_ringFootprints hr
  have hcard : (ringSets n r).card ≤ (Bs.biUnion (fun B => B.ringFootprint r)).card :=
    Finset.card_le_card hsub
  have hunion : (Bs.biUnion (fun B => B.ringFootprint r)).card ≤
      ∑ B ∈ Bs, (B.ringFootprint r).card := by
    exact Finset.card_biUnion_le
  exact hcard.trans hunion

/-- Binomial form of the ring capacity lower bound. -/
theorem CoversAllNonempty.choose_le_sum_ringFootprints {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs) (hr : 0 < r) :
    Nat.choose n r ≤ ∑ B ∈ Bs, (B.ringFootprint r).card := by
  rw [← ringSets_card n r]
  exact hcover.ring_card_le_sum_ringFootprints hr

/-- A vertex-level cover provides a feasible blocker for every nonzero vertex ring. -/
theorem CoversAllNonzeroVertices.exists_feasible_type_on_vertex_ring {n r : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonzeroVertices Bs)
    {v : Vertex n} (hvring : VertexInRing v r) (hv : v ≠ zeroVertex n) :
    ∃ B ∈ Bs, B.BlocksVertex v ∧ RingTypeFeasible n r B.type := by
  obtain ⟨B, hB, hblocks⟩ := hcover v hv
  exact ⟨B, hB, hblocks, B.ringTypeFeasible_of_blocksVertex hvring hblocks⟩

/-- Every singleton `{i}` is blocked by a type-1 blocker with `D = {i}`. -/
theorem singleton_requires_type1 {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) (i : Fin n) :
    ∃ B ∈ Bs, B.D = {i} := by
  obtain ⟨B, hB, hblocks⟩ := hcover {i} (Finset.singleton_nonempty i)
  obtain ⟨hsub, -⟩ := hblocks
  refine ⟨B, hB, ?_⟩
  rcases Finset.subset_singleton_iff.mp hsub with h | h
  · exact absurd h B.hD_nonempty.ne_empty
  · exact h

/-! ### §F2 Type-1 guard graph rules -/

/-- A type-1 blocker with `D = {i}` draws guard edges from `i` to every member of `A`. -/
def GuardEdge {n : Nat} (Bs : Finset (NormBlocker n)) (i j : Fin n) : Prop :=
  ∃ B ∈ Bs, B.D = {i} ∧ j ∈ B.A

/-- A flipped set survives one blocker when that blocker does not block it. -/
def SurvivesBlocker {n : Nat} (T : Finset (Fin n)) (B : NormBlocker n) : Prop :=
  ¬ B.Blocks T

/-- A flipped set survives a blocker family when no blocker in the family blocks it. -/
def SurvivesFamily {n : Nat} (T : Finset (Fin n)) (Bs : Finset (NormBlocker n)) : Prop :=
  ∀ B ∈ Bs, SurvivesBlocker T B

/-- The subfamily of type-1 blockers. -/
def Type1Set {n : Nat} (Bs : Finset (NormBlocker n)) : Finset (NormBlocker n) :=
  Bs.filter fun B => B.D.card = 1

/-- The subfamily of repair blockers, i.e. type-2 and type-3 blockers. -/
def RepairSet {n : Nat} (Bs : Finset (NormBlocker n)) : Finset (NormBlocker n) :=
  Bs.filter fun B => B.D.card = 2 ∨ B.D.card = 3

/-- A type-1-closed flipped set is nonempty and survives the type-1 skeleton. These are the
cycle-like survivors that repair blockers must hit. -/
def Type1Closed {n : Nat} (Bs : Finset (NormBlocker n)) (T : Finset (Fin n)) : Prop :=
  T.Nonempty ∧ SurvivesFamily T (Type1Set Bs)

@[simp] theorem mem_Type1Set {n : Nat} {Bs : Finset (NormBlocker n)} {B : NormBlocker n} :
    B ∈ Type1Set Bs ↔ B ∈ Bs ∧ B.Type1 := by
  simp [Type1Set, NormBlocker.Type1]

@[simp] theorem mem_RepairSet {n : Nat} {Bs : Finset (NormBlocker n)} {B : NormBlocker n} :
    B ∈ RepairSet Bs ↔ B ∈ Bs ∧ (B.Type2 ∨ B.Type3) := by
  simp [RepairSet, NormBlocker.Type2, NormBlocker.Type3]

/-- If a nonempty flipped set survives the type-1 skeleton of a full normalized cover,
then some repair blocker must block it. -/
theorem exists_repair_blocks_of_survives_type1 {n : Nat}
    {Bs : Finset (NormBlocker n)} {T : Finset (Fin n)}
    (hcover : CoversAllNonempty Bs) (hT : T.Nonempty)
    (hsurv : SurvivesFamily T (Type1Set Bs)) :
    ∃ B ∈ RepairSet Bs, B.Blocks T := by
  obtain ⟨B, hB, hblocks⟩ := hcover T hT
  have hnot1 : ¬ B.Type1 := by
    intro h1
    exact hsurv B (by rw [mem_Type1Set]; exact ⟨hB, h1⟩) hblocks
  have hpos : 0 < B.D.card := Finset.card_pos.mpr B.hD_nonempty
  have hle : B.D.card ≤ 3 := by
    have hw := B.hwidth
    omega
  have hcases : B.D.card = 1 ∨ B.D.card = 2 ∨ B.D.card = 3 := by omega
  rcases hcases with h1 | h2 | h3
  · exact False.elim (hnot1 h1)
  · exact ⟨B, by rw [mem_RepairSet]; exact ⟨hB, Or.inl h2⟩, hblocks⟩
  · exact ⟨B, by rw [mem_RepairSet]; exact ⟨hB, Or.inr h3⟩, hblocks⟩

/-- Type-1-closed survivor form of the repair rule. -/
theorem exists_repair_blocks_of_type1Closed {n : Nat}
    {Bs : Finset (NormBlocker n)} {T : Finset (Fin n)}
    (hcover : CoversAllNonempty Bs) (hclosed : Type1Closed Bs T) :
    ∃ B ∈ RepairSet Bs, B.Blocks T :=
  exists_repair_blocks_of_survives_type1 hcover hclosed.1 hclosed.2

/-- If a normalized blocker has singleton positive side, it has at least one guard. -/
theorem NormBlocker.A_nonempty_of_D_singleton {n : Nat} (B : NormBlocker n) {i : Fin n}
    (hD : B.D = {i}) : B.A.Nonempty := by
  apply Finset.card_pos.mp
  have hDcard : B.D.card = 1 := by
    rw [hD, Finset.card_singleton]
  have hw := B.hwidth
  omega

/-- In any normalized cover, every coordinate has at least one outgoing type-1 guard edge. -/
theorem exists_guardEdge_of_cover {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) (i : Fin n) :
    ∃ j : Fin n, GuardEdge Bs i j := by
  obtain ⟨B, hB, hD⟩ := singleton_requires_type1 Bs hcover i
  obtain ⟨j, hj⟩ := B.A_nonempty_of_D_singleton hD
  exact ⟨j, B, hB, hD, hj⟩

/-- If a set survives the type-1 skeleton, then every coordinate it contains can point to
a guard that also lies in the set. This is the local, graph-shaped form of type-1 survival. -/
theorem exists_guardEdge_inside_of_survives_type1 {n : Nat}
    {Bs : Finset (NormBlocker n)} {T : Finset (Fin n)}
    (hcover : CoversAllNonempty Bs)
    (hsurv : SurvivesFamily T (Type1Set Bs))
    {i : Fin n} (hi : i ∈ T) :
    ∃ j ∈ T, GuardEdge Bs i j := by
  obtain ⟨B, hB, hD⟩ := singleton_requires_type1 Bs hcover i
  have htype : B.Type1 := by
    unfold NormBlocker.Type1
    rw [hD, Finset.card_singleton]
  have hBtype : B ∈ Type1Set Bs := by
    rw [mem_Type1Set]
    exact ⟨hB, htype⟩
  have hnotblocks : ¬ B.Blocks T := hsurv B hBtype
  have hsub : B.D ⊆ T := by
    intro j hj
    rw [hD] at hj
    have hji : j = i := Finset.mem_singleton.mp hj
    simpa [hji] using hi
  have hnotdisj : ¬ Disjoint B.A T := by
    intro hdisj
    exact hnotblocks ⟨hsub, hdisj⟩
  have hex : ∃ j, j ∈ B.A ∧ j ∈ T := by
    by_contra hnone
    have hdisj : Disjoint B.A T := by
      rw [Finset.disjoint_left]
      intro j hjA hjT
      exact hnone ⟨j, hjA, hjT⟩
    exact hnotdisj hdisj
  obtain ⟨j, hjA, hjT⟩ := hex
  exact ⟨j, hjT, B, hB, hD, hjA⟩

/-- A type-1-closed set carries an internal outgoing guard edge from every coordinate
it contains. -/
theorem Type1Closed.exists_internal_guard {n : Nat}
    {Bs : Finset (NormBlocker n)} {T : Finset (Fin n)}
    (hcover : CoversAllNonempty Bs) (hclosed : Type1Closed Bs T) :
    ∀ i ∈ T, ∃ j ∈ T, GuardEdge Bs i j := by
  intro i hi
  exact exists_guardEdge_inside_of_survives_type1 hcover hclosed.2 hi

/-- A type-1 blocker cannot block the full flipped-coordinate set. -/
theorem NormBlocker.not_blocks_univ_of_type1 {n : Nat} (B : NormBlocker n)
    (h1 : B.Type1) : ¬ B.Blocks Finset.univ := by
  intro hblocks
  have hAempty : B.A = ∅ := by
    ext j
    constructor
    · intro hj
      exact False.elim ((Finset.disjoint_left.mp hblocks.2) hj (Finset.mem_univ j))
    · intro hj
      simp at hj
  have hAcard : B.A.card = 0 := by
    rw [hAempty, Finset.card_empty]
  unfold NormBlocker.Type1 at h1
  have hw := B.hwidth
  omega

/-- A blocker that blocks the full flipped-coordinate set must have no zero-side guards. -/
theorem NormBlocker.A_empty_of_blocks_univ {n : Nat} (B : NormBlocker n)
    (hblocks : B.Blocks Finset.univ) : B.A = ∅ := by
  ext j
  constructor
  · intro hj
    exact False.elim ((Finset.disjoint_left.mp hblocks.2) hj (Finset.mem_univ j))
  · intro hj
    simp at hj

/-- A blocker that blocks the full flipped-coordinate set is necessarily type 3. -/
theorem NormBlocker.type3_of_blocks_univ {n : Nat} (B : NormBlocker n)
    (hblocks : B.Blocks Finset.univ) : B.Type3 := by
  unfold NormBlocker.Type3
  have hAempty : B.A = ∅ := B.A_empty_of_blocks_univ hblocks
  have hAcard : B.A.card = 0 := by
    rw [hAempty, Finset.card_empty]
  have hw := B.hwidth
  omega

/-- Type-1 blockers alone cannot cover all nonempty flipped-coordinate sets. -/
theorem not_coversAllNonempty_of_all_type1 {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (h1 : ∀ B ∈ Bs, B.Type1) :
    ¬ CoversAllNonempty Bs := by
  intro hcover
  have huniv : (Finset.univ : Finset (Fin n)).Nonempty := by
    exact ⟨⟨0, hn⟩, Finset.mem_univ _⟩
  obtain ⟨B, hB, hblocks⟩ := hcover Finset.univ huniv
  exact B.not_blocks_univ_of_type1 (h1 B hB) hblocks

/-! ## §G Invariants: type counts and balance -/

/-- Count blockers of a given type. -/
def countType {n : Nat} (Bs : Finset (NormBlocker n)) (t : Nat) : Nat :=
  (Bs.filter fun B => B.D.card = t).card

def M1 {n : Nat} (Bs : Finset (NormBlocker n)) : Nat := countType Bs 1
def M2 {n : Nat} (Bs : Finset (NormBlocker n)) : Nat := countType Bs 2
def M3 {n : Nat} (Bs : Finset (NormBlocker n)) : Nat := countType Bs 3

/-- Total positive (`D`) slots. -/
def positiveSlots {n : Nat} (Bs : Finset (NormBlocker n)) : Nat :=
  ∑ B ∈ Bs, B.D.card

/-- Total zero (`A`) slots. -/
def zeroSlots {n : Nat} (Bs : Finset (NormBlocker n)) : Nat :=
  ∑ B ∈ Bs, B.A.card

/-- A cover is globally balanced when positive and zero slots agree. -/
def GloballyBalanced {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  positiveSlots Bs = zeroSlots Bs

/-- Positive slots incident to coordinate `i`. -/
def positiveCoordSlots {n : Nat} (Bs : Finset (NormBlocker n)) (i : Fin n) : Nat :=
  (Bs.filter fun B => i ∈ B.D).card

/-- Zero/guard slots incident to coordinate `i`. -/
def zeroCoordSlots {n : Nat} (Bs : Finset (NormBlocker n)) (i : Fin n) : Nat :=
  (Bs.filter fun B => i ∈ B.A).card

/-- Coordinate balance: each coordinate appears equally often on both sides. -/
def CoordBalanced {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  ∀ i : Fin n, positiveCoordSlots Bs i = zeroCoordSlots Bs i

theorem positiveSlots_eq_sum_positiveCoordSlots {n : Nat} (Bs : Finset (NormBlocker n)) :
    positiveSlots Bs = ∑ i : Fin n, positiveCoordSlots Bs i := by
  unfold positiveSlots positiveCoordSlots
  rw [show (∑ B ∈ Bs, B.D.card) = ∑ B ∈ Bs, ∑ i : Fin n, if i ∈ B.D then 1 else 0 by
    apply Finset.sum_congr rfl
    intro B hB
    rw [Finset.card_eq_sum_ones]
    simp]
  rw [Finset.sum_comm]
  apply Finset.sum_congr rfl
  intro i hi
  rw [Finset.card_eq_sum_ones]
  simp

theorem zeroSlots_eq_sum_zeroCoordSlots {n : Nat} (Bs : Finset (NormBlocker n)) :
    zeroSlots Bs = ∑ i : Fin n, zeroCoordSlots Bs i := by
  unfold zeroSlots zeroCoordSlots
  rw [show (∑ B ∈ Bs, B.A.card) = ∑ B ∈ Bs, ∑ i : Fin n, if i ∈ B.A then 1 else 0 by
    apply Finset.sum_congr rfl
    intro B hB
    rw [Finset.card_eq_sum_ones]
    simp]
  rw [Finset.sum_comm]
  apply Finset.sum_congr rfl
  intro i hi
  rw [Finset.card_eq_sum_ones]
  simp

/-- Per-coordinate balance implies global balance by double-counting slots. -/
theorem coordBalanced_implies_globalBalanced {n : Nat} (Bs : Finset (NormBlocker n))
    (h : CoordBalanced Bs) :
    GloballyBalanced Bs := by
  unfold GloballyBalanced
  rw [positiveSlots_eq_sum_positiveCoordSlots, zeroSlots_eq_sum_zeroCoordSlots]
  apply Finset.sum_congr rfl
  intro i hi
  exact h i

/-! ### §G2 Type partition, slot equations, and `M1 ≥ n` -/

theorem NormBlocker.D_card_pos {n : Nat} (B : NormBlocker n) : 0 < B.D.card :=
  Finset.card_pos.mpr B.hD_nonempty

theorem NormBlocker.D_card_le_three {n : Nat} (B : NormBlocker n) : B.D.card ≤ 3 := by
  have h := B.hwidth; omega

/-- Every normalized blocker is type 1, 2, or 3 (since `1 ≤ |D| ≤ 3`). -/
theorem NormBlocker.type_cases {n : Nat} (B : NormBlocker n) :
    B.D.card = 1 ∨ B.D.card = 2 ∨ B.D.card = 3 := by
  have hpos := B.D_card_pos
  have hle := B.D_card_le_three
  omega

/-- The blockers of a fixed type, as a finset. -/
def TypeSet {n : Nat} (Bs : Finset (NormBlocker n)) (t : Nat) : Finset (NormBlocker n) :=
  Bs.filter fun B => B.D.card = t

@[simp] theorem mem_TypeSet {n : Nat} {Bs : Finset (NormBlocker n)}
    {B : NormBlocker n} {t : Nat} :
    B ∈ TypeSet Bs t ↔ B ∈ Bs ∧ B.D.card = t := by
  simp [TypeSet]

theorem countType_eq_card_TypeSet {n : Nat} (Bs : Finset (NormBlocker n)) (t : Nat) :
    countType Bs t = (TypeSet Bs t).card := rfl

theorem TypeSet_disjoint {n : Nat} (Bs : Finset (NormBlocker n)) {a b : Nat} (hab : a ≠ b) :
    Disjoint (TypeSet Bs a) (TypeSet Bs b) := by
  rw [Finset.disjoint_left]
  intro B hBa hBb
  exact hab ((mem_TypeSet.mp hBa).2.symm.trans (mem_TypeSet.mp hBb).2)

theorem type_partition_eq {n : Nat} (Bs : Finset (NormBlocker n)) :
    TypeSet Bs 1 ∪ TypeSet Bs 2 ∪ TypeSet Bs 3 = Bs := by
  apply Finset.ext
  intro B
  constructor
  · intro h
    rcases Finset.mem_union.mp h with h12 | h3
    · rcases Finset.mem_union.mp h12 with h1 | h2
      · exact (mem_TypeSet.mp h1).1
      · exact (mem_TypeSet.mp h2).1
    · exact (mem_TypeSet.mp h3).1
  · intro hB
    rcases B.type_cases with h1 | h2 | h3
    · simp [Finset.mem_union, mem_TypeSet, hB, h1]
    · simp [Finset.mem_union, mem_TypeSet, hB, h2]
    · simp [Finset.mem_union, mem_TypeSet, hB, h3]

/-- Split a sum over `Bs` into its three type fibers. -/
theorem sum_split_by_type {n : Nat} (Bs : Finset (NormBlocker n)) (g : NormBlocker n → Nat) :
    ∑ B ∈ Bs, g B =
      (∑ B ∈ TypeSet Bs 1, g B) + (∑ B ∈ TypeSet Bs 2, g B) + (∑ B ∈ TypeSet Bs 3, g B) := by
  have h12 : Disjoint (TypeSet Bs 1) (TypeSet Bs 2) := TypeSet_disjoint Bs (by decide)
  have h123 : Disjoint (TypeSet Bs 1 ∪ TypeSet Bs 2) (TypeSet Bs 3) :=
    Finset.disjoint_union_left.mpr
      ⟨TypeSet_disjoint Bs (by decide), TypeSet_disjoint Bs (by decide)⟩
  conv_lhs => rw [← type_partition_eq Bs]
  rw [Finset.sum_union h123, Finset.sum_union h12]

theorem sum_D_card_on_TypeSet {n : Nat} (Bs : Finset (NormBlocker n)) (t : Nat) :
    (∑ B ∈ TypeSet Bs t, B.D.card) = t * (TypeSet Bs t).card := by
  have h : ∀ B ∈ TypeSet Bs t, B.D.card = t := fun B hB => (mem_TypeSet.mp hB).2
  rw [Finset.sum_congr rfl h, Finset.sum_const, smul_eq_mul, Nat.mul_comm]

theorem sum_A_card_on_TypeSet {n : Nat} (Bs : Finset (NormBlocker n)) (t : Nat) :
    (∑ B ∈ TypeSet Bs t, B.A.card) = (3 - t) * (TypeSet Bs t).card := by
  have h : ∀ B ∈ TypeSet Bs t, B.A.card = 3 - t := by
    intro B hB
    have ht := (mem_TypeSet.mp hB).2
    have hw := B.hwidth
    omega
  rw [Finset.sum_congr rfl h, Finset.sum_const, smul_eq_mul, Nat.mul_comm]

/-- `|Bs| = M1 + M2 + M3`. -/
theorem card_eq_type_counts {n : Nat} (Bs : Finset (NormBlocker n)) :
    Bs.card = M1 Bs + M2 Bs + M3 Bs := by
  rw [Finset.card_eq_sum_ones, sum_split_by_type Bs (fun _ => 1)]
  simp only [Finset.sum_const, smul_eq_mul, mul_one]
  unfold M1 M2 M3
  rw [countType_eq_card_TypeSet Bs 1, countType_eq_card_TypeSet Bs 2,
      countType_eq_card_TypeSet Bs 3]

/-- `positiveSlots = M1 + 2·M2 + 3·M3` (type-`t` blocker has `|D| = t`). -/
theorem positiveSlots_eq_type_counts {n : Nat} (Bs : Finset (NormBlocker n)) :
    positiveSlots Bs = M1 Bs + 2 * M2 Bs + 3 * M3 Bs := by
  unfold positiveSlots
  rw [sum_split_by_type Bs (fun B => B.D.card),
      sum_D_card_on_TypeSet Bs 1, sum_D_card_on_TypeSet Bs 2, sum_D_card_on_TypeSet Bs 3]
  unfold M1 M2 M3
  rw [countType_eq_card_TypeSet Bs 1, countType_eq_card_TypeSet Bs 2,
      countType_eq_card_TypeSet Bs 3]
  ring

/-- `zeroSlots = 2·M1 + M2` (type-`t` blocker has `|A| = 3 - t`). -/
theorem zeroSlots_eq_type_counts {n : Nat} (Bs : Finset (NormBlocker n)) :
    zeroSlots Bs = 2 * M1 Bs + M2 Bs := by
  unfold zeroSlots
  rw [sum_split_by_type Bs (fun B => B.A.card),
      sum_A_card_on_TypeSet Bs 1, sum_A_card_on_TypeSet Bs 2, sum_A_card_on_TypeSet Bs 3]
  unfold M1 M2
  rw [countType_eq_card_TypeSet Bs 1, countType_eq_card_TypeSet Bs 2]
  omega

/-- A chosen type-1 blocker witnessing that singleton `{i}` is forced. -/
noncomputable def singletonBlocker {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) (i : Fin n) : NormBlocker n :=
  (singleton_requires_type1 Bs hcover i).choose

theorem singletonBlocker_spec {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) (i : Fin n) :
    singletonBlocker Bs hcover i ∈ Bs ∧ (singletonBlocker Bs hcover i).D = {i} :=
  (singleton_requires_type1 Bs hcover i).choose_spec

theorem singletonBlocker_mem {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) (i : Fin n) :
    singletonBlocker Bs hcover i ∈ TypeSet Bs 1 := by
  rw [mem_TypeSet]
  refine ⟨(singletonBlocker_spec Bs hcover i).1, ?_⟩
  rw [(singletonBlocker_spec Bs hcover i).2, Finset.card_singleton]

theorem singletonBlocker_injective {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) :
    Function.Injective (singletonBlocker Bs hcover) := by
  intro i j hij
  have hi := (singletonBlocker_spec Bs hcover i).2
  have hj := (singletonBlocker_spec Bs hcover j).2
  have hset : ({i} : Finset (Fin n)) = {j} := by rw [← hi, hij, hj]
  exact Finset.singleton_inj.mp hset

/-- Every coordinate forces a distinct type-1 blocker, so `n ≤ M1`. -/
theorem M1_ge_n {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) :
    n ≤ M1 Bs := by
  have hinj : Function.Injective (singletonBlocker Bs hcover) :=
    singletonBlocker_injective Bs hcover
  have himg : Finset.univ.image (singletonBlocker Bs hcover) ⊆ TypeSet Bs 1 := by
    intro B hB
    rw [Finset.mem_image] at hB
    obtain ⟨i, _, rfl⟩ := hB
    exact singletonBlocker_mem Bs hcover i
  have hle := Finset.card_le_card himg
  rw [Finset.card_image_of_injective _ hinj, Finset.card_univ, Fintype.card_fin] at hle
  exact hle

/-! ### §G3 Universal size and repair lower bounds -/

/-- Any normalized cover has at least `n` blockers, because each coordinate forces
a distinct type-1 blocker. -/
theorem cover_card_ge_n {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) :
    n ≤ Bs.card := by
  have hM1 : n ≤ M1 Bs := M1_ge_n Bs hcover
  have hCard : Bs.card = M1 Bs + M2 Bs + M3 Bs := card_eq_type_counts Bs
  omega

/-- The full flipped-coordinate set forces a type-3 blocker in every positive-dimensional
normalized cover. -/
theorem exists_type3_of_cover {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (hcover : CoversAllNonempty Bs) :
    ∃ B ∈ Bs, B.Type3 := by
  have huniv : (Finset.univ : Finset (Fin n)).Nonempty :=
    ⟨⟨0, hn⟩, Finset.mem_univ _⟩
  obtain ⟨B, hB, hblocks⟩ := hcover Finset.univ huniv
  exact ⟨B, hB, B.type3_of_blocks_univ hblocks⟩

/-- A repair blocker is a type-2 or type-3 blocker: it repairs what type-1 guard rules
alone cannot cover. -/
def RepairBlocker {n : Nat} (B : NormBlocker n) : Prop :=
  B.Type2 ∨ B.Type3

/-- In a nonzero dimension, any normalized cover contains at least one repair blocker. -/
theorem exists_repairBlocker_of_cover {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (hcover : CoversAllNonempty Bs) :
    ∃ B ∈ Bs, RepairBlocker B := by
  by_contra hnone
  have h1 : ∀ B ∈ Bs, B.Type1 := by
    intro B hB
    rcases B.type_cases with h1 | h2 | h3
    · exact h1
    · exact False.elim (hnone ⟨B, hB, Or.inl h2⟩)
    · exact False.elim (hnone ⟨B, hB, Or.inr h3⟩)
  exact (not_coversAllNonempty_of_all_type1 Bs hn h1) hcover

/-- The repair count `M2 + M3` is positive in every nonzero-dimensional normalized cover. -/
theorem repair_count_pos_of_cover {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (hcover : CoversAllNonempty Bs) :
    0 < M2 Bs + M3 Bs := by
  obtain ⟨B, hB, hrepair⟩ := exists_repairBlocker_of_cover Bs hn hcover
  rcases hrepair with h2 | h3
  · have hM2 : 0 < M2 Bs := by
      unfold M2
      rw [countType_eq_card_TypeSet Bs 2]
      exact Finset.card_pos.mpr ⟨B, by rw [mem_TypeSet]; exact ⟨hB, h2⟩⟩
    omega
  · have hM3 : 0 < M3 Bs := by
      unfold M3
      rw [countType_eq_card_TypeSet Bs 3]
      exact Finset.card_pos.mpr ⟨B, by rw [mem_TypeSet]; exact ⟨hB, h3⟩⟩
    omega

/-- Every positive-dimensional normalized cover has at least one type-3 blocker. -/
theorem M3_pos_of_cover {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (hcover : CoversAllNonempty Bs) :
    0 < M3 Bs := by
  obtain ⟨B, hB, h3⟩ := exists_type3_of_cover Bs hn hcover
  unfold M3
  rw [countType_eq_card_TypeSet Bs 3]
  exact Finset.card_pos.mpr ⟨B, by rw [mem_TypeSet]; exact ⟨hB, h3⟩⟩

/-- First universal nontrivial size bound: every normalized cover in positive dimension has
at least `n + 1` blockers. The future `n + 4` theorem is a sharpening of the repair part. -/
theorem cover_card_ge_n_add_one {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (hcover : CoversAllNonempty Bs) :
    n + 1 ≤ Bs.card := by
  have hM1 : n ≤ M1 Bs := M1_ge_n Bs hcover
  have hRepair : 0 < M3 Bs := M3_pos_of_cover Bs hn hcover
  have hCard : Bs.card = M1 Bs + M2 Bs + M3 Bs := card_eq_type_counts Bs
  omega

/-- Balance equation: with all blockers of width 3, balance gives `M2 + 3·M3 = M1`. -/
theorem global_balance_type_equation {n : Nat} (Bs : Finset (NormBlocker n))
    (hbal : GloballyBalanced Bs) :
    M2 Bs + 3 * M3 Bs = M1 Bs := by
  unfold GloballyBalanced at hbal
  rw [positiveSlots_eq_type_counts, zeroSlots_eq_type_counts] at hbal
  omega

/-- Balanced lower bound: a balanced cover of all nonempty sets needs `3·|Bs| ≥ 4·n`. -/
theorem global_balance_lower_bound {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) (hbal : GloballyBalanced Bs) :
    4 * n ≤ 3 * Bs.card := by
  have hM1 : n ≤ M1 Bs := M1_ge_n Bs hcover
  have hEq : M2 Bs + 3 * M3 Bs = M1 Bs := global_balance_type_equation Bs hbal
  have hCard : Bs.card = M1 Bs + M2 Bs + M3 Bs := card_eq_type_counts Bs
  omega

/-! ### §G3 Minimality and private witnesses -/

/-- A cover is minimal if erasing any blocker destroys coverage. -/
def MinimalCoversAllNonempty {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  CoversAllNonempty Bs ∧
  ∀ B ∈ Bs, ¬ CoversAllNonempty (Bs.erase B)

/-- A private witness for `B` is a nonempty set blocked uniquely by `B`. -/
def PrivateWitness {n : Nat} (Bs : Finset (NormBlocker n))
    (B : NormBlocker n) (T : Finset (Fin n)) : Prop :=
  T.Nonempty ∧ B ∈ Bs ∧ B.Blocks T ∧
  ∀ C ∈ Bs, C.Blocks T → C = B

/-- In a minimal cover, every blocker has a private witness. -/
theorem minimal_unique_private_witness {n : Nat} (Bs : Finset (NormBlocker n))
    (hmin : MinimalCoversAllNonempty Bs) {B : NormBlocker n} (hB : B ∈ Bs) :
    ∃ T : Finset (Fin n), PrivateWitness Bs B T := by
  rcases hmin with ⟨hcover, hminimal⟩
  have hnot : ¬ CoversAllNonempty (Bs.erase B) := hminimal B hB
  unfold CoversAllNonempty at hnot
  push Not at hnot
  rcases hnot with ⟨T, hTne, hTnot⟩
  rcases hcover T hTne with ⟨C, hC, hCblocks⟩
  have hCB : C = B := by
    by_contra hneq
    have hCerase : C ∈ Bs.erase B := by
      simp [hC, hneq]
    exact hTnot C hCerase hCblocks
  refine ⟨T, hTne, hB, ?_, ?_⟩
  · simpa [hCB] using hCblocks
  · intro C' hC' hC'blocks
    by_contra hneq
    have hCerase : C' ∈ Bs.erase B := by
      simp [hC', hneq]
    exact hTnot C' hCerase hC'blocks

/-- Conversely, private witnesses for every blocker certify minimality of a cover. -/
theorem minimal_of_private_witnesses {n : Nat} {Bs : Finset (NormBlocker n)}
    (hcover : CoversAllNonempty Bs)
    (hpriv : ∀ B ∈ Bs, ∃ T : Finset (Fin n), PrivateWitness Bs B T) :
    MinimalCoversAllNonempty Bs := by
  refine ⟨hcover, ?_⟩
  intro B hB heraseCover
  obtain ⟨T, hTne, hBmem, hBblocks, huniq⟩ := hpriv B hB
  obtain ⟨C, hCerase, hCblocks⟩ := heraseCover T hTne
  have hCmem : C ∈ Bs := by
    exact (Finset.mem_erase.mp hCerase).2
  have hCB : C = B := huniq C hCmem hCblocks
  have hCne : C ≠ B := (Finset.mem_erase.mp hCerase).1
  exact hCne hCB

/-! ## §H DNF: exact-width DNF equivalence -/

/-- A normalized blocker as a DNF term `(⋀ i∈D, xᵢ) ∧ (⋀ j∈A, ¬xⱼ)`. -/
def evalNormBlocker {n : Nat} (B : NormBlocker n) (x : Fin n → Bool) : Bool :=
  decide (∀ i ∈ B.D, x i = true) && decide (∀ j ∈ B.A, x j = false)

/-- The DNF computed by a set of normalized blockers. -/
def evalDNF {n : Nat} (Bs : Finset (NormBlocker n)) (x : Fin n → Bool) : Bool :=
  decide (∃ B ∈ Bs, evalNormBlocker B x = true)

/-- The `n`-ary OR. -/
def ORn {n : Nat} (x : Fin n → Bool) : Bool :=
  decide (∃ i : Fin n, x i = true)

/-- The set of coordinates where an assignment is true. -/
def trueSet {n : Nat} (x : Fin n → Bool) : Finset (Fin n) :=
  Finset.univ.filter fun i => x i = true

@[simp] theorem mem_trueSet {n : Nat} (x : Fin n → Bool) (i : Fin n) :
    i ∈ trueSet x ↔ x i = true := by
  simp [trueSet]

/-- Boolean evaluation of a normalized blocker matches blocking the true-coordinate set. -/
theorem evalNormBlocker_true_iff_blocks_trueSet {n : Nat}
    (B : NormBlocker n) (x : Fin n → Bool) :
    evalNormBlocker B x = true ↔ B.Blocks (trueSet x) := by
  unfold evalNormBlocker NormBlocker.Blocks
  constructor
  · intro h
    have hBoth :
        decide (∀ i ∈ B.D, x i = true) = true ∧
          decide (∀ j ∈ B.A, x j = false) = true := by
      simpa only [Bool.and_eq_true_eq_eq_true_and_eq_true] using h
    have hD : ∀ i ∈ B.D, x i = true :=
      of_decide_eq_true hBoth.1
    have hA : ∀ j ∈ B.A, x j = false :=
      of_decide_eq_true hBoth.2
    constructor
    · intro i hi
      exact (mem_trueSet x i).mpr (hD i hi)
    · rw [Finset.disjoint_left]
      intro j hjA hjT
      have hxtrue : x j = true := (mem_trueSet x j).mp hjT
      have hxfalse : x j = false := hA j hjA
      rw [hxtrue] at hxfalse
      contradiction
  · rintro ⟨hDsub, hdisj⟩
    have hDdec : decide (∀ i ∈ B.D, x i = true) = true := by
      apply decide_eq_true
      intro i hi
      exact (mem_trueSet x i).mp (hDsub hi)
    have hAdec : decide (∀ j ∈ B.A, x j = false) = true := by
      apply decide_eq_true
      intro j hj
      by_cases hx : x j = true
      · have hjT : j ∈ trueSet x := (mem_trueSet x j).mpr hx
        exact False.elim ((Finset.disjoint_left.mp hdisj) hj hjT)
      · exact Bool.eq_false_of_not_eq_true hx
    simpa only [Bool.and_eq_true_eq_eq_true_and_eq_true] using And.intro hDdec hAdec

/-- `ORₙ` is true exactly when the true-coordinate set is nonempty. -/
theorem ORn_true_iff_trueSet_nonempty {n : Nat} (x : Fin n → Bool) :
    ORn x = true ↔ (trueSet x).Nonempty := by
  unfold ORn
  constructor
  · intro h
    rcases of_decide_eq_true h with ⟨i, hi⟩
    exact ⟨i, by simp [trueSet, hi]⟩
  · intro h
    apply decide_eq_true
    rcases h with ⟨i, hi⟩
    exact ⟨i, by simpa [trueSet] using hi⟩

/-- Covering all nonempty sets is equivalent to the DNF computing `ORₙ`. -/
theorem coversAllNonempty_iff_evalDNF_eq_OR {n : Nat} (Bs : Finset (NormBlocker n)) :
    CoversAllNonempty Bs ↔ ∀ x : Fin n → Bool, evalDNF Bs x = ORn x := by
  constructor
  · intro hcover x
    by_cases hT : (trueSet x).Nonempty
    · have hOR : ORn x = true := (ORn_true_iff_trueSet_nonempty x).mpr hT
      rcases hcover (trueSet x) hT with ⟨B, hB, hBlocks⟩
      have hEvalB : evalNormBlocker B x = true :=
        (evalNormBlocker_true_iff_blocks_trueSet B x).mpr hBlocks
      have hDNF : evalDNF Bs x = true := by
        unfold evalDNF
        apply decide_eq_true
        exact ⟨B, hB, hEvalB⟩
      rw [hDNF, hOR]
    · have hOR : ORn x = false := by
        cases hOR : ORn x
        · rfl
        · exact False.elim (hT ((ORn_true_iff_trueSet_nonempty x).mp hOR))
      have hDNF : evalDNF Bs x = false := by
        unfold evalDNF
        apply decide_eq_false
        intro hExists
        rcases hExists with ⟨B, -, hEval⟩
        have hBlocks := (evalNormBlocker_true_iff_blocks_trueSet B x).mp hEval
        rcases B.hD_nonempty with ⟨i, hiD⟩
        exact hT ⟨i, hBlocks.1 hiD⟩
      rw [hDNF, hOR]
  · intro h T hT
    let f : Fin n → Bool := fun i => decide (i ∈ T)
    have htrueSet : trueSet f = T := by
      ext i
      by_cases hi : i ∈ T <;> simp [trueSet, f, hi]
    have hOR : ORn f = true :=
      (ORn_true_iff_trueSet_nonempty f).mpr (by simpa [htrueSet] using hT)
    have hDNF : evalDNF Bs f = true := by
      rw [h f, hOR]
    unfold evalDNF at hDNF
    rcases of_decide_eq_true hDNF with ⟨B, hB, hEval⟩
    refine ⟨B, hB, ?_⟩
    have hBlocks := (evalNormBlocker_true_iff_blocks_trueSet B f).mp hEval
    simpa [htrueSet] using hBlocks

/-! ## §I Exact-width-3 size targets -/

/-- There is a normalized exact-width-3 unique-cover system of size `m`. -/
def ExistsUniqueCoverSize (n m : Nat) : Prop :=
  ∃ Bs : Finset (NormBlocker n), CoversAllNonempty Bs ∧ Bs.card = m

/-- There is a normalized exact-width-3 unique-cover system using at most `m` blockers. -/
def ExistsUniqueCoverSizeAtMost (n m : Nat) : Prop :=
  ∃ Bs : Finset (NormBlocker n), CoversAllNonempty Bs ∧ Bs.card ≤ m

/-- `m` is a minimum normalized unique-cover size in dimension `n`. -/
def IsMinimumUniqueCoverSize (n m : Nat) : Prop :=
  ExistsUniqueCoverSize n m ∧
  ∀ k : Nat, ExistsUniqueCoverSize n k → m ≤ k

/-- `m` is a maximum normalized unique-cover size in dimension `n`. Since adding blockers
preserves coverage, the maximum is the full finite universe whenever any cover exists. -/
def IsMaximumUniqueCoverSize (n m : Nat) : Prop :=
  ExistsUniqueCoverSize n m ∧
  ∀ k : Nat, ExistsUniqueCoverSize n k → k ≤ m

/-- Coverage is monotone under adding blockers. -/
theorem CoversAllNonempty.mono {n : Nat} {Bs Cs : Finset (NormBlocker n)}
    (hsub : Bs ⊆ Cs) (hcover : CoversAllNonempty Bs) :
    CoversAllNonempty Cs := by
  intro T hT
  obtain ⟨B, hB, hblocks⟩ := hcover T hT
  exact ⟨B, hsub hB, hblocks⟩

/-- No cover size can exceed the number of all normalized blockers. -/
theorem uniqueCoverSize_le_total {n m : Nat} (h : ExistsUniqueCoverSize n m) :
    m ≤ Fintype.card (NormBlocker n) := by
  obtain ⟨Bs, -, hcard⟩ := h
  rw [← hcard]
  exact Finset.card_le_univ Bs

/-- If any normalized cover exists, then the full universe of normalized blockers is also a cover,
so the maximum cover size is the total number of possible normalized blockers. -/
theorem maximumUniqueCoverSize_total_of_exists {n m : Nat}
    (h : ExistsUniqueCoverSize n m) :
    IsMaximumUniqueCoverSize n (Fintype.card (NormBlocker n)) := by
  obtain ⟨Bs, hcover, -⟩ := h
  refine ⟨?_, ?_⟩
  · refine ⟨Finset.univ, ?_, by simp⟩
    exact CoversAllNonempty.mono (by intro B _; exact Finset.mem_univ B) hcover
  · intro k hk
    exact uniqueCoverSize_le_total hk

/-- Any minimum cover in positive dimension has at least `n + 1` blockers. -/
theorem minimumUniqueCoverSize_ge_n_add_one {n m : Nat}
    (hn : 0 < n) (hmin : IsMinimumUniqueCoverSize n m) :
    n + 1 ≤ m := by
  obtain ⟨Bs, hcover, hcard⟩ := hmin.1
  rw [← hcard]
  exact cover_card_ge_n_add_one Bs hn hcover

/-! ### §H1 The `n + 4` gadget upper envelope -/

namespace ExactWidth3Gadget

variable {n : Nat} (a b c : Fin n) (hab : a ≠ b) (hac : a ≠ c) (hbc : b ≠ c)

/-- The two guards used by the type-1 blocker for coordinate `i`.
For the three distinguished coordinates, the guards are the other two distinguished
coordinates; every other coordinate is guarded by `a,b`. -/
def guardPair (i : Fin n) : Finset (Fin n) :=
  if i = a then {b, c} else if i = b then {a, c} else {a, b}

theorem guardPair_card (hab : a ≠ b) (hac : a ≠ c) (hbc : b ≠ c) (i : Fin n) :
    (guardPair a b c i).card = 2 := by
  unfold guardPair
  by_cases hia : i = a
  · subst i
    simp [hbc]
  · by_cases hib : i = b
    · subst i
      simp [hia, hac]
    · simp [hia, hib, hab]

theorem self_not_mem_guardPair (hab : a ≠ b) (hac : a ≠ c) (hbc : b ≠ c)
    (i : Fin n) :
    i ∉ guardPair a b c i := by
  unfold guardPair
  by_cases hia : i = a
  · subst i
    simp [hab, hac]
  · by_cases hib : i = b
    · subst i
      simp [hab.symm, hbc]
    · simp [hia, hib]

private theorem mem_pair_iff {x p q : Fin n} :
    x ∈ ({p, q} : Finset (Fin n)) ↔ x = p ∨ x = q := by
  simp

private theorem mem_triple_iff {x p q r : Fin n} :
    x ∈ ({p, q, r} : Finset (Fin n)) ↔ x = p ∨ x = q ∨ x = r := by
  simp

/-- The type-1 blocker assigned to coordinate `i` in the `n + 4` construction. -/
def type1 (i : Fin n) : NormBlocker n where
  D := {i}
  A := guardPair a b c i
  hdisj := by
    rw [Finset.disjoint_left]
    intro x hxD hxA
    rw [Finset.mem_singleton] at hxD
    subst x
    exact self_not_mem_guardPair a b c hab hac hbc i hxA
  hwidth := by
    rw [Finset.card_singleton, guardPair_card a b c hab hac hbc i]
  hD_nonempty := Finset.singleton_nonempty i

/-- The positive side of a gadget type-1 blocker is exactly its coordinate. -/
@[simp] theorem type1_D (i : Fin n) :
    (type1 a b c hab hac hbc i).D = {i} := rfl

/-- The gadget has one distinct type-1 blocker for each coordinate. -/
theorem type1_injective :
    Function.Injective (type1 a b c hab hac hbc) := by
  intro i j hij
  have hD : ({i} : Finset (Fin n)) = {j} := by
    simpa [type1_D] using congrArg NormBlocker.D hij
  exact Finset.singleton_inj.mp hD

/-- There are exactly `n` type-1 blockers in the gadget image. -/
theorem type1_image_card :
    (Finset.univ.image (type1 a b c hab hac hbc)).card = n := by
  rw [Finset.card_image_of_injective _ (type1_injective a b c hab hac hbc)]
  simp

/-- Repair blocker for the pair `{a,b}` guarded by `c`. -/
def pairAB : NormBlocker n where
  D := {a, b}
  A := {c}
  hdisj := by
    rw [Finset.disjoint_left]
    intro x hxD hxA
    rw [Finset.mem_singleton] at hxA
    subst x
    simp [hac.symm, hbc.symm] at hxD
  hwidth := by
    simp [hab]
  hD_nonempty := by
    exact ⟨a, by simp⟩

/-- Repair blocker for the pair `{a,c}` guarded by `b`. -/
def pairAC : NormBlocker n where
  D := {a, c}
  A := {b}
  hdisj := by
    rw [Finset.disjoint_left]
    intro x hxD hxA
    rw [Finset.mem_singleton] at hxA
    subst x
    simp [hab.symm, hbc] at hxD
  hwidth := by
    simp [hac]
  hD_nonempty := by
    exact ⟨a, by simp⟩

/-- Repair blocker for the pair `{b,c}` guarded by `a`. -/
def pairBC : NormBlocker n where
  D := {b, c}
  A := {a}
  hdisj := by
    rw [Finset.disjoint_left]
    intro x hxD hxA
    rw [Finset.mem_singleton] at hxA
    subst x
    simp [hab, hac] at hxD
  hwidth := by
    simp [hbc]
  hD_nonempty := by
    exact ⟨b, by simp⟩

/-- The type-3 repair blocker killing sets containing all three distinguished coordinates. -/
def tripleABC : NormBlocker n where
  D := {a, b, c}
  A := ∅
  hdisj := by simp
  hwidth := by
    simp [hab, hac, hbc]
  hD_nonempty := by
    exact ⟨a, by simp⟩

/-- The four constant-size repair blockers of the construction. -/
def repairs : Finset (NormBlocker n) :=
  {pairAB a b c hab hac hbc, pairAC a b c hab hac hbc,
    pairBC a b c hab hac hbc, tripleABC a b c hab hac hbc}

/-- The four repair blockers are distinct. -/
theorem repairs_card :
    (repairs a b c hab hac hbc).card = 4 := by
  rw [Finset.card_eq_four]
  refine ⟨pairAB a b c hab hac hbc, pairAC a b c hab hac hbc,
    pairBC a b c hab hac hbc, tripleABC a b c hab hac hbc, ?_, ?_, ?_, ?_, ?_, ?_, rfl⟩
  · intro h
    have hA := congrArg NormBlocker.A h
    change ({c} : Finset (Fin n)) = {b} at hA
    exact hbc.symm (Finset.singleton_inj.mp hA)
  · intro h
    have hA := congrArg NormBlocker.A h
    change ({c} : Finset (Fin n)) = {a} at hA
    exact hac.symm (Finset.singleton_inj.mp hA)
  · intro h
    have hA := congrArg NormBlocker.A h
    change ({c} : Finset (Fin n)) = ∅ at hA
    exact Finset.singleton_ne_empty c hA
  · intro h
    have hA := congrArg NormBlocker.A h
    change ({b} : Finset (Fin n)) = {a} at hA
    exact hab.symm (Finset.singleton_inj.mp hA)
  · intro h
    have hA := congrArg NormBlocker.A h
    change ({b} : Finset (Fin n)) = ∅ at hA
    exact Finset.singleton_ne_empty b hA
  · intro h
    have hA := congrArg NormBlocker.A h
    change ({a} : Finset (Fin n)) = ∅ at hA
    exact Finset.singleton_ne_empty a hA

/-- Every repair blocker has type 2 or type 3, never type 1. -/
theorem repair_not_type1 {B : NormBlocker n}
    (hB : B ∈ repairs a b c hab hac hbc) :
    ¬ B.Type1 := by
  intro h1
  have hcases :
      B = pairAB a b c hab hac hbc ∨
      B = pairAC a b c hab hac hbc ∨
      B = pairBC a b c hab hac hbc ∨
      B = tripleABC a b c hab hac hbc := by
    simpa [repairs] using hB
  rcases hcases with rfl | rfl | rfl | rfl
  · unfold NormBlocker.Type1 at h1
    simp [pairAB, hab] at h1
  · unfold NormBlocker.Type1 at h1
    simp [pairAC, hac] at h1
  · unfold NormBlocker.Type1 at h1
    simp [pairBC, hbc] at h1
  · unfold NormBlocker.Type1 at h1
    simp [tripleABC, hab, hac, hbc] at h1

/-- The type-1 image and the repair set are disjoint. -/
theorem type1_image_disjoint_repairs :
    Disjoint (Finset.univ.image (type1 a b c hab hac hbc))
      (repairs a b c hab hac hbc) := by
  rw [Finset.disjoint_left]
  intro B hBimg hBrep
  obtain ⟨i, -, rfl⟩ := Finset.mem_image.mp hBimg
  exact repair_not_type1 a b c hab hac hbc hBrep (by simp [NormBlocker.Type1, type1])

/-- The full `n + 4`-shape construction: one type-1 blocker per coordinate plus four repairs. -/
def cover : Finset (NormBlocker n) :=
  Finset.univ.image (type1 a b c hab hac hbc) ∪ repairs a b c hab hac hbc

theorem type1_mem_cover (i : Fin n) :
    type1 a b c hab hac hbc i ∈ cover a b c hab hac hbc := by
  unfold cover
  rw [Finset.mem_union]
  exact Or.inl (Finset.mem_image.mpr ⟨i, Finset.mem_univ i, rfl⟩)

theorem pairAB_mem_cover :
    pairAB a b c hab hac hbc ∈ cover a b c hab hac hbc := by
  unfold cover repairs
  simp

theorem pairAC_mem_cover :
    pairAC a b c hab hac hbc ∈ cover a b c hab hac hbc := by
  unfold cover repairs
  simp

theorem pairBC_mem_cover :
    pairBC a b c hab hac hbc ∈ cover a b c hab hac hbc := by
  unfold cover repairs
  simp

theorem tripleABC_mem_cover :
    tripleABC a b c hab hac hbc ∈ cover a b c hab hac hbc := by
  unfold cover repairs
  simp

theorem type1_blocks_of_mem_and_guards_absent {T : Finset (Fin n)}
    {i : Fin n} (hi : i ∈ T)
    (hdisj : Disjoint (guardPair a b c i) T) :
    (type1 a b c hab hac hbc i).Blocks T := by
  constructor
  · intro x hx
    change x ∈ ({i} : Finset (Fin n)) at hx
    rw [Finset.mem_singleton] at hx
    simpa [hx] using hi
  · change Disjoint (guardPair a b c i) T
    exact hdisj

/-- In the gadget, the singleton `{i}` is private to the type-1 blocker for `i`. -/
theorem type1_private_singleton (i : Fin n) :
    PrivateWitness (cover a b c hab hac hbc) (type1 a b c hab hac hbc i) {i} := by
  refine ⟨Finset.singleton_nonempty i, type1_mem_cover a b c hab hac hbc i, ?_, ?_⟩
  · apply type1_blocks_of_mem_and_guards_absent a b c hab hac hbc
    · simp
    · rw [Finset.disjoint_left]
      intro x hxguard hxi
      rw [Finset.mem_singleton] at hxi
      subst x
      exact self_not_mem_guardPair a b c hab hac hbc i hxguard
  · intro C hC hCblocks
    unfold cover at hC
    rcases Finset.mem_union.mp hC with hCtype1 | hCrepair
    · obtain ⟨j, -, rfl⟩ := Finset.mem_image.mp hCtype1
      have hD := (type1 a b c hab hac hbc j).D_eq_singleton_of_blocks_singleton i hCblocks
      change ({j} : Finset (Fin n)) = {i} at hD
      have hji : j = i := Finset.singleton_inj.mp hD
      subst j
      rfl
    · exact False.elim
        (repair_not_type1 a b c hab hac hbc hCrepair
          ((C).type1_of_blocks_singleton i hCblocks))

/-- The `{a,b}` repair owns the pair vertex `{a,b}`. -/
theorem pairAB_private :
    PrivateWitness (cover a b c hab hac hbc) (pairAB a b c hab hac hbc) {a, b} := by
  refine ⟨by simp, pairAB_mem_cover a b c hab hac hbc, ?_, ?_⟩
  · constructor
    · intro x hx
      change x ∈ ({a, b} : Finset (Fin n)) at hx
      exact hx
    · rw [Finset.disjoint_left]
      intro x hxA hxT
      change x ∈ ({c} : Finset (Fin n)) at hxA
      have hxc : x = c := by
        simpa only [Finset.mem_singleton] using hxA
      subst x
      rcases (mem_pair_iff.mp hxT) with hca | hcb
      · exact False.elim (hac.symm hca)
      · exact False.elim (hbc.symm hcb)
  · intro C hC hCblocks
    unfold cover at hC
    rcases Finset.mem_union.mp hC with hCtype1 | hCrepair
    · obtain ⟨j, -, rfl⟩ := Finset.mem_image.mp hCtype1
      have hjmem : j ∈ ({a, b} : Finset (Fin n)) := by
        exact hCblocks.1 (by simp [type1])
      have hdisj : Disjoint (guardPair a b c j) ({a, b} : Finset (Fin n)) := by
        simpa [type1] using hCblocks.2
      rcases (mem_pair_iff.mp hjmem) with hja | hjb
      · subst j
        have hbguard : b ∈ guardPair a b c a := by simp [guardPair, hbc]
        have hbT : b ∈ ({a, b} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) hbguard hbT)
      · subst j
        have haguard : a ∈ guardPair a b c b := by simp [guardPair, hab.symm, hac]
        have haT : a ∈ ({a, b} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) haguard haT)
    · have hcases :
          C = pairAB a b c hab hac hbc ∨
          C = pairAC a b c hab hac hbc ∨
          C = pairBC a b c hab hac hbc ∨
          C = tripleABC a b c hab hac hbc := by
        simpa [repairs] using hCrepair
      rcases hcases with rfl | rfl | rfl | rfl
      · rfl
      · have hcmem : c ∈ ({a, b} : Finset (Fin n)) := hCblocks.1 (by simp [pairAC])
        rcases (mem_pair_iff.mp hcmem) with hca | hcb
        · exact False.elim (hac.symm hca)
        · exact False.elim (hbc.symm hcb)
      · have hcmem : c ∈ ({a, b} : Finset (Fin n)) := hCblocks.1 (by simp [pairBC])
        rcases (mem_pair_iff.mp hcmem) with hca | hcb
        · exact False.elim (hac.symm hca)
        · exact False.elim (hbc.symm hcb)
      · have hcmem : c ∈ ({a, b} : Finset (Fin n)) := hCblocks.1 (by simp [tripleABC])
        rcases (mem_pair_iff.mp hcmem) with hca | hcb
        · exact False.elim (hac.symm hca)
        · exact False.elim (hbc.symm hcb)

/-- The `{a,c}` repair owns the pair vertex `{a,c}`. -/
theorem pairAC_private :
    PrivateWitness (cover a b c hab hac hbc) (pairAC a b c hab hac hbc) {a, c} := by
  refine ⟨by simp, pairAC_mem_cover a b c hab hac hbc, ?_, ?_⟩
  · constructor
    · intro x hx
      change x ∈ ({a, c} : Finset (Fin n)) at hx
      exact hx
    · rw [Finset.disjoint_left]
      intro x hxA hxT
      change x ∈ ({b} : Finset (Fin n)) at hxA
      have hxb : x = b := by
        simpa only [Finset.mem_singleton] using hxA
      subst x
      rcases (mem_pair_iff.mp hxT) with hba | hbc'
      · exact False.elim (hab.symm hba)
      · exact False.elim (hbc hbc')
  · intro C hC hCblocks
    unfold cover at hC
    rcases Finset.mem_union.mp hC with hCtype1 | hCrepair
    · obtain ⟨j, -, rfl⟩ := Finset.mem_image.mp hCtype1
      have hjmem : j ∈ ({a, c} : Finset (Fin n)) := by
        exact hCblocks.1 (by simp [type1])
      have hdisj : Disjoint (guardPair a b c j) ({a, c} : Finset (Fin n)) := by
        simpa [type1] using hCblocks.2
      rcases (mem_pair_iff.mp hjmem) with hja | hjc
      · subst j
        have hcguard : c ∈ guardPair a b c a := by simp [guardPair, hbc]
        have hcT : c ∈ ({a, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) hcguard hcT)
      · subst j
        have haguard : a ∈ guardPair a b c c := by simp [guardPair, hac.symm, hbc.symm]
        have haT : a ∈ ({a, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) haguard haT)
    · have hcases :
          C = pairAB a b c hab hac hbc ∨
          C = pairAC a b c hab hac hbc ∨
          C = pairBC a b c hab hac hbc ∨
          C = tripleABC a b c hab hac hbc := by
        simpa [repairs] using hCrepair
      rcases hcases with rfl | rfl | rfl | rfl
      · have hbmem : b ∈ ({a, c} : Finset (Fin n)) := hCblocks.1 (by simp [pairAB])
        rcases (mem_pair_iff.mp hbmem) with hba | hbc'
        · exact False.elim (hab.symm hba)
        · exact False.elim (hbc hbc')
      · rfl
      · have hbmem : b ∈ ({a, c} : Finset (Fin n)) := hCblocks.1 (by simp [pairBC])
        rcases (mem_pair_iff.mp hbmem) with hba | hbc'
        · exact False.elim (hab.symm hba)
        · exact False.elim (hbc hbc')
      · have hbmem : b ∈ ({a, c} : Finset (Fin n)) := hCblocks.1 (by simp [tripleABC])
        rcases (mem_pair_iff.mp hbmem) with hba | hbc'
        · exact False.elim (hab.symm hba)
        · exact False.elim (hbc hbc')

/-- The `{b,c}` repair owns the pair vertex `{b,c}`. -/
theorem pairBC_private :
    PrivateWitness (cover a b c hab hac hbc) (pairBC a b c hab hac hbc) {b, c} := by
  refine ⟨by simp, pairBC_mem_cover a b c hab hac hbc, ?_, ?_⟩
  · constructor
    · intro x hx
      change x ∈ ({b, c} : Finset (Fin n)) at hx
      exact hx
    · rw [Finset.disjoint_left]
      intro x hxA hxT
      change x ∈ ({a} : Finset (Fin n)) at hxA
      have hxa : x = a := by
        simpa only [Finset.mem_singleton] using hxA
      subst x
      rcases (mem_pair_iff.mp hxT) with hab' | hac'
      · exact False.elim (hab hab')
      · exact False.elim (hac hac')
  · intro C hC hCblocks
    unfold cover at hC
    rcases Finset.mem_union.mp hC with hCtype1 | hCrepair
    · obtain ⟨j, -, rfl⟩ := Finset.mem_image.mp hCtype1
      have hjmem : j ∈ ({b, c} : Finset (Fin n)) := by
        exact hCblocks.1 (by simp [type1])
      have hdisj : Disjoint (guardPair a b c j) ({b, c} : Finset (Fin n)) := by
        simpa [type1] using hCblocks.2
      rcases (mem_pair_iff.mp hjmem) with hjb | hjc
      · subst j
        have hcguard : c ∈ guardPair a b c b := by simp [guardPair, hab.symm, hbc]
        have hcT : c ∈ ({b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) hcguard hcT)
      · subst j
        have hbguard : b ∈ guardPair a b c c := by simp [guardPair, hac.symm, hbc.symm]
        have hbT : b ∈ ({b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) hbguard hbT)
    · have hcases :
          C = pairAB a b c hab hac hbc ∨
          C = pairAC a b c hab hac hbc ∨
          C = pairBC a b c hab hac hbc ∨
          C = tripleABC a b c hab hac hbc := by
        simpa [repairs] using hCrepair
      rcases hcases with rfl | rfl | rfl | rfl
      · have hamem : a ∈ ({b, c} : Finset (Fin n)) := hCblocks.1 (by simp [pairAB])
        rcases (mem_pair_iff.mp hamem) with hab' | hac'
        · exact False.elim (hab hab')
        · exact False.elim (hac hac')
      · have hamem : a ∈ ({b, c} : Finset (Fin n)) := hCblocks.1 (by simp [pairAC])
        rcases (mem_pair_iff.mp hamem) with hab' | hac'
        · exact False.elim (hab hab')
        · exact False.elim (hac hac')
      · rfl
      · have hamem : a ∈ ({b, c} : Finset (Fin n)) := hCblocks.1 (by simp [tripleABC])
        rcases (mem_pair_iff.mp hamem) with hab' | hac'
        · exact False.elim (hab hab')
        · exact False.elim (hac hac')

/-- The top repair owns the core triple vertex `{a,b,c}`. -/
theorem tripleABC_private :
    PrivateWitness (cover a b c hab hac hbc) (tripleABC a b c hab hac hbc) {a, b, c} := by
  refine ⟨by simp, tripleABC_mem_cover a b c hab hac hbc, ?_, ?_⟩
  · constructor
    · intro x hx
      change x ∈ ({a, b, c} : Finset (Fin n)) at hx
      exact hx
    · simp [tripleABC]
  · intro C hC hCblocks
    unfold cover at hC
    rcases Finset.mem_union.mp hC with hCtype1 | hCrepair
    · obtain ⟨j, -, rfl⟩ := Finset.mem_image.mp hCtype1
      have hjmem : j ∈ ({a, b, c} : Finset (Fin n)) := by
        exact hCblocks.1 (by simp [type1])
      have hdisj : Disjoint (guardPair a b c j) ({a, b, c} : Finset (Fin n)) := by
        simpa [type1] using hCblocks.2
      rcases (mem_triple_iff.mp hjmem) with hja | hjb | hjc
      · subst j
        have hbguard : b ∈ guardPair a b c a := by simp [guardPair, hbc]
        have hbT : b ∈ ({a, b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) hbguard hbT)
      · subst j
        have haguard : a ∈ guardPair a b c b := by simp [guardPair, hab.symm, hac]
        have haT : a ∈ ({a, b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) haguard haT)
      · subst j
        have haguard : a ∈ guardPair a b c c := by simp [guardPair, hac.symm, hbc.symm]
        have haT : a ∈ ({a, b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hdisj) haguard haT)
    · have hcases :
          C = pairAB a b c hab hac hbc ∨
          C = pairAC a b c hab hac hbc ∨
          C = pairBC a b c hab hac hbc ∨
          C = tripleABC a b c hab hac hbc := by
        simpa [repairs] using hCrepair
      rcases hcases with rfl | rfl | rfl | rfl
      · have hcA : c ∈ (pairAB a b c hab hac hbc).A := by simp [pairAB]
        have hcT : c ∈ ({a, b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hCblocks.2) hcA hcT)
      · have hbA : b ∈ (pairAC a b c hab hac hbc).A := by simp [pairAC]
        have hbT : b ∈ ({a, b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hCblocks.2) hbA hbT)
      · have haA : a ∈ (pairBC a b c hab hac hbc).A := by simp [pairBC]
        have haT : a ∈ ({a, b, c} : Finset (Fin n)) := by simp
        exact False.elim ((Finset.disjoint_left.mp hCblocks.2) haA haT)
      · rfl

theorem cover_coversAllNonempty :
    CoversAllNonempty (cover a b c hab hac hbc) := by
  intro T hT
  by_cases ha : a ∈ T
  · by_cases hb : b ∈ T
    · by_cases hc : c ∈ T
      · refine ⟨tripleABC a b c hab hac hbc, tripleABC_mem_cover a b c hab hac hbc, ?_⟩
        constructor
        · intro x hx
          change x ∈ ({a, b, c} : Finset (Fin n)) at hx
          rcases (mem_triple_iff.mp hx) with rfl | rfl | rfl
          · exact ha
          · exact hb
          · exact hc
        · simp [tripleABC]
      · refine ⟨pairAB a b c hab hac hbc, pairAB_mem_cover a b c hab hac hbc, ?_⟩
        constructor
        · intro x hx
          change x ∈ ({a, b} : Finset (Fin n)) at hx
          rcases (mem_pair_iff.mp hx) with rfl | rfl
          · exact ha
          · exact hb
        · rw [Finset.disjoint_left]
          intro x hxA hxT
          change x ∈ ({c} : Finset (Fin n)) at hxA
          have hxc : x = c := by
            simpa only [Finset.mem_singleton] using hxA
          subst x
          exact hc hxT
    · by_cases hc : c ∈ T
      · refine ⟨pairAC a b c hab hac hbc, pairAC_mem_cover a b c hab hac hbc, ?_⟩
        constructor
        · intro x hx
          change x ∈ ({a, c} : Finset (Fin n)) at hx
          rcases (mem_pair_iff.mp hx) with rfl | rfl
          · exact ha
          · exact hc
        · rw [Finset.disjoint_left]
          intro x hxA hxT
          change x ∈ ({b} : Finset (Fin n)) at hxA
          have hxb : x = b := by
            simpa only [Finset.mem_singleton] using hxA
          subst x
          exact hb hxT
      · refine ⟨type1 a b c hab hac hbc a, type1_mem_cover a b c hab hac hbc a, ?_⟩
        apply type1_blocks_of_mem_and_guards_absent a b c hab hac hbc ha
        rw [Finset.disjoint_left]
        intro x hxA hxT
        have hxCases : x = b ∨ x = c := by
          have hxA' : x ∈ ({b, c} : Finset (Fin n)) := by
            simpa [guardPair] using hxA
          exact mem_pair_iff.mp hxA'
        rcases hxCases with rfl | rfl
        · exact hb hxT
        · exact hc hxT
  · by_cases hb : b ∈ T
    · by_cases hc : c ∈ T
      · refine ⟨pairBC a b c hab hac hbc, pairBC_mem_cover a b c hab hac hbc, ?_⟩
        constructor
        · intro x hx
          change x ∈ ({b, c} : Finset (Fin n)) at hx
          rcases (mem_pair_iff.mp hx) with rfl | rfl
          · exact hb
          · exact hc
        · rw [Finset.disjoint_left]
          intro x hxA hxT
          change x ∈ ({a} : Finset (Fin n)) at hxA
          have hxa : x = a := by
            simpa only [Finset.mem_singleton] using hxA
          subst x
          exact ha hxT
      · refine ⟨type1 a b c hab hac hbc b, type1_mem_cover a b c hab hac hbc b, ?_⟩
        apply type1_blocks_of_mem_and_guards_absent a b c hab hac hbc hb
        rw [Finset.disjoint_left]
        intro x hxA hxT
        have hxCases : x = a ∨ x = c := by
          have hxA' : x ∈ ({a, c} : Finset (Fin n)) := by
            simpa [guardPair, hab.symm] using hxA
          exact mem_pair_iff.mp hxA'
        rcases hxCases with rfl | rfl
        · exact ha hxT
        · exact hc hxT
    · by_cases hc : c ∈ T
      · refine ⟨type1 a b c hab hac hbc c, type1_mem_cover a b c hab hac hbc c, ?_⟩
        apply type1_blocks_of_mem_and_guards_absent a b c hab hac hbc hc
        rw [Finset.disjoint_left]
        intro x hxA hxT
        have hxCases : x = a ∨ x = b := by
          simpa only [guardPair, if_neg hac.symm, if_neg hbc.symm,
            Finset.mem_insert, Finset.mem_singleton] using hxA
        rcases hxCases with rfl | rfl
        · exact ha hxT
        · exact hb hxT
      · obtain ⟨i, hi⟩ := hT
        have hia : i ≠ a := by
          intro h
          subst i
          exact ha hi
        have hib : i ≠ b := by
          intro h
          subst i
          exact hb hi
        refine ⟨type1 a b c hab hac hbc i, type1_mem_cover a b c hab hac hbc i, ?_⟩
        apply type1_blocks_of_mem_and_guards_absent a b c hab hac hbc hi
        rw [Finset.disjoint_left]
        intro x hxA hxT
        have hxCases : x = a ∨ x = b := by
          simpa only [guardPair, if_neg hia, if_neg hib, Finset.mem_insert,
            Finset.mem_singleton] using hxA
        rcases hxCases with rfl | rfl
        · exact ha hxT
        · exact hb hxT

/-- Every blocker in the gadget has a private witness. -/
theorem cover_private_witnesses :
    ∀ B ∈ cover a b c hab hac hbc,
      ∃ T : Finset (Fin n), PrivateWitness (cover a b c hab hac hbc) B T := by
  intro B hB
  unfold cover at hB
  rcases Finset.mem_union.mp hB with hBtype1 | hBrepair
  · obtain ⟨i, -, rfl⟩ := Finset.mem_image.mp hBtype1
    exact ⟨{i}, type1_private_singleton a b c hab hac hbc i⟩
  · have hcases :
        B = pairAB a b c hab hac hbc ∨
        B = pairAC a b c hab hac hbc ∨
        B = pairBC a b c hab hac hbc ∨
        B = tripleABC a b c hab hac hbc := by
      simpa [repairs] using hBrepair
    rcases hcases with rfl | rfl | rfl | rfl
    · exact ⟨{a, b}, pairAB_private a b c hab hac hbc⟩
    · exact ⟨{a, c}, pairAC_private a b c hab hac hbc⟩
    · exact ⟨{b, c}, pairBC_private a b c hab hac hbc⟩
    · exact ⟨{a, b, c}, tripleABC_private a b c hab hac hbc⟩

/-- The `n + 4` gadget is minimal: deleting any blocker exposes one private vertex. -/
theorem cover_minimal :
    MinimalCoversAllNonempty (cover a b c hab hac hbc) :=
  minimal_of_private_witnesses
    (cover_coversAllNonempty a b c hab hac hbc)
    (cover_private_witnesses a b c hab hac hbc)

theorem repairs_card_le_four :
    (repairs a b c hab hac hbc).card ≤ 4 := by
  simpa [repairs] using
    (Finset.card_le_four
      (a := pairAB a b c hab hac hbc)
      (b := pairAC a b c hab hac hbc)
      (c := pairBC a b c hab hac hbc)
      (d := tripleABC a b c hab hac hbc))

theorem cover_card_le_n_add_four :
    (cover a b c hab hac hbc).card ≤ n + 4 := by
  unfold cover
  calc
    (Finset.univ.image (type1 a b c hab hac hbc) ∪ repairs a b c hab hac hbc).card
        ≤ (Finset.univ.image (type1 a b c hab hac hbc)).card
            + (repairs a b c hab hac hbc).card := Finset.card_union_le _ _
    _ ≤ n + 4 := by
          have himage :
              (Finset.univ.image (type1 a b c hab hac hbc)).card ≤ n := by
            simpa using
              (Finset.card_image_le :
                (Finset.univ.image (type1 a b c hab hac hbc)).card ≤ Finset.univ.card)
          exact Nat.add_le_add himage (repairs_card_le_four a b c hab hac hbc)

/-- The `n + 4` gadget has exactly `n + 4` blockers. -/
theorem cover_card_eq_n_add_four :
    (cover a b c hab hac hbc).card = n + 4 := by
  unfold cover
  rw [Finset.card_union_of_disjoint (type1_image_disjoint_repairs a b c hab hac hbc),
    type1_image_card, repairs_card]

theorem exists_cover_sizeAtMost_n_add_four
    (a b c : Fin n) (hab : a ≠ b) (hac : a ≠ c) (hbc : b ≠ c) :
    ExistsUniqueCoverSizeAtMost n (n + 4) :=
  ⟨cover a b c hab hac hbc, cover_coversAllNonempty a b c hab hac hbc,
    cover_card_le_n_add_four a b c hab hac hbc⟩

theorem exists_cover_size_n_add_four
    (a b c : Fin n) (hab : a ≠ b) (hac : a ≠ c) (hbc : b ≠ c) :
    ExistsUniqueCoverSize n (n + 4) :=
  ⟨cover a b c hab hac hbc, cover_coversAllNonempty a b c hab hac hbc,
    cover_card_eq_n_add_four a b c hab hac hbc⟩

end ExactWidth3Gadget

/-- Public upper-envelope theorem: every dimension with three coordinates has a normalized
exact-width-3 cover of all nonzero vertices using at most `n + 4` blockers. -/
theorem exactWidth3UpperAtMost_of_three_le {n : Nat} (hn : 3 ≤ n) :
    ExistsUniqueCoverSizeAtMost n (n + 4) := by
  let a : Fin n := ⟨0, by omega⟩
  let b : Fin n := ⟨1, by omega⟩
  let c : Fin n := ⟨2, by omega⟩
  have hab : a ≠ b := by
    intro h
    have hval : (a : Nat) = (b : Nat) := by
      exact congrArg Fin.val h
    norm_num [a, b] at hval
  have hac : a ≠ c := by
    intro h
    have hval : (a : Nat) = (c : Nat) := by
      exact congrArg Fin.val h
    norm_num [a, c] at hval
  have hbc : b ≠ c := by
    intro h
    have hval : (b : Nat) = (c : Nat) := by
      exact congrArg Fin.val h
    norm_num [b, c] at hval
  exact ExactWidth3Gadget.exists_cover_sizeAtMost_n_add_four a b c hab hac hbc

/-- Proven minimum-size envelope: once a minimum exists, dimensions `n ≥ 3` have minimum
normalized unique-cover size between `n + 1` and `n + 4`. The conjecture is that the upper
edge is sharp. -/
theorem minimumUniqueCoverSize_between_n_add_one_and_n_add_four {n m : Nat}
    (hn : 3 ≤ n) (hmin : IsMinimumUniqueCoverSize n m) :
    n + 1 ≤ m ∧ m ≤ n + 4 := by
  constructor
  · exact minimumUniqueCoverSize_ge_n_add_one (by omega) hmin
  · obtain ⟨Bs, hcover, hcardle⟩ := exactWidth3UpperAtMost_of_three_le hn
    have hsize : ExistsUniqueCoverSize n Bs.card := ⟨Bs, hcover, rfl⟩
    exact (hmin.2 Bs.card hsize).trans hcardle

/-! ### Normalized exact-width-3 edge classes -/

/-- Number of non-type-1 blockers. These are the repair blockers needed after the
ring-1 singleton obligations are met. -/
def repairCount {n : Nat} (Bs : Finset (NormBlocker n)) : Nat :=
  M2 Bs + M3 Bs

/-- Total size splits into the forced singleton/type-1 part plus the repair part. -/
theorem card_eq_M1_add_repairCount {n : Nat} (Bs : Finset (NormBlocker n)) :
    Bs.card = M1 Bs + repairCount Bs := by
  rw [card_eq_type_counts]
  unfold repairCount
  omega

/-- Every normalized cover has at least `n + repairCount` blockers: ring-1 already forces
`n` type-1 blockers, and the rest are repairs. -/
theorem cover_card_ge_n_add_repairCount {n : Nat} (Bs : Finset (NormBlocker n))
    (hcover : CoversAllNonempty Bs) :
    n + repairCount Bs ≤ Bs.card := by
  have hM1 : n ≤ M1 Bs := M1_ge_n Bs hcover
  rw [card_eq_M1_add_repairCount]
  omega

/-- In nonzero dimension, every normalized cover has at least one repair blocker. -/
theorem repairCount_pos_of_cover' {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (hcover : CoversAllNonempty Bs) :
    0 < repairCount Bs := by
  unfold repairCount
  exact repair_count_pos_of_cover Bs hn hcover

/-- Equivalent non-strict spelling: every nonzero-dimensional cover has repair count at
least one. -/
theorem repairCount_ge_one_of_cover {n : Nat} (Bs : Finset (NormBlocker n))
    (hn : 0 < n) (hcover : CoversAllNonempty Bs) :
    1 ≤ repairCount Bs := by
  exact repairCount_pos_of_cover' Bs hn hcover

/-- The singleton skeleton is tight when the cover uses exactly the forced `n` type-1
blockers and no surplus type-1 blockers. -/
def TightSingletonSkeleton {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  M1 Bs = n

/-- A repair skeleton is sharp at four repairs, the number achieved by the explicit
`n + 4` gadget. -/
def SharpFourRepairSkeleton {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  repairCount Bs = 4

/-- The apparent extremal normalized shape: cover all nonempty vertices with a tight
singleton skeleton and exactly four repairs. -/
def ExactWidth3ExtremalShape {n : Nat} (Bs : Finset (NormBlocker n)) : Prop :=
  CoversAllNonempty Bs ∧ TightSingletonSkeleton Bs ∧ SharpFourRepairSkeleton Bs

/-- Any extremal-shape family has exactly `n + 4` blockers. -/
theorem card_eq_n_add_four_of_extremalShape {n : Nat} {Bs : Finset (NormBlocker n)}
    (hshape : ExactWidth3ExtremalShape Bs) :
    Bs.card = n + 4 := by
  rcases hshape with ⟨-, hM1, hRepair⟩
  have hCard : Bs.card = M1 Bs + repairCount Bs := card_eq_M1_add_repairCount Bs
  unfold SharpFourRepairSkeleton at hRepair
  rw [hCard, hM1]
  omega

/-- In any normalized cover, a total-size bound `n + k` bounds the repair count by `k`. -/
theorem repairCount_le_of_cover_card_le_n_add {n k : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs)
    (hcard : Bs.card ≤ n + k) :
    repairCount Bs ≤ k := by
  have hge : n + repairCount Bs ≤ Bs.card := cover_card_ge_n_add_repairCount Bs hcover
  omega

/-- The `k = 4` spelling used by the exact-width-3 target. -/
theorem repairCount_le_four_of_cover_card_le_n_add_four {n : Nat}
    {Bs : Finset (NormBlocker n)} (hcover : CoversAllNonempty Bs)
    (hcard : Bs.card ≤ n + 4) :
    repairCount Bs ≤ 4 :=
  repairCount_le_of_cover_card_le_n_add hcover hcard

/-- Conversely, a tight singleton skeleton with at most `k` repairs has size at most
`n + k`. -/
theorem cover_card_le_n_add_of_tight_repairCount_le {n k : Nat}
    {Bs : Finset (NormBlocker n)} (hM1 : TightSingletonSkeleton Bs)
    (hrepair : repairCount Bs ≤ k) :
    Bs.card ≤ n + k := by
  have hCard : Bs.card = M1 Bs + repairCount Bs := card_eq_M1_add_repairCount Bs
  unfold TightSingletonSkeleton at hM1
  rw [hCard, hM1]
  omega

/-- The `k = 4` spelling used by the exact-width-3 target. -/
theorem cover_card_le_n_add_four_of_tight_repairCount_le_four {n : Nat}
    {Bs : Finset (NormBlocker n)} (hM1 : TightSingletonSkeleton Bs)
    (hrepair : repairCount Bs ≤ 4) :
    Bs.card ≤ n + 4 :=
  cover_card_le_n_add_of_tight_repairCount_le hM1 hrepair

/-- The conjectural sharp upper bound target: exact-width-3 normalized covers of size `n + 4`. -/
def ExactWidth3UpperTarget (n : Nat) : Prop :=
  ExistsUniqueCoverSize n (n + 4)

/-- The upper half of the `n + 4` target is proved by the exact gadget construction. -/
theorem exactWidth3UpperTarget_of_three_le {n : Nat} (hn : 3 ≤ n) :
    ExactWidth3UpperTarget n := by
  let a : Fin n := ⟨0, by omega⟩
  let b : Fin n := ⟨1, by omega⟩
  let c : Fin n := ⟨2, by omega⟩
  have hab : a ≠ b := by
    intro h
    have hval : (a : Nat) = (b : Nat) := by
      exact congrArg Fin.val h
    norm_num [a, b] at hval
  have hac : a ≠ c := by
    intro h
    have hval : (a : Nat) = (c : Nat) := by
      exact congrArg Fin.val h
    norm_num [a, c] at hval
  have hbc : b ≠ c := by
    intro h
    have hval : (b : Nat) = (c : Nat) := by
      exact congrArg Fin.val h
    norm_num [b, c] at hval
  exact ExactWidth3Gadget.exists_cover_size_n_add_four a b c hab hac hbc

/-- The conjectural sharp lower bound target: no exact-width-3 normalized cover is smaller
than `n + 4`. -/
def ExactWidth3LowerTarget (n : Nat) : Prop :=
  ∀ m : Nat, ExistsUniqueCoverSize n m → n + 4 ≤ m

/-- Repair-count route to the lower bound: every cover needs at least four type-2/type-3
repair blockers. Existing results prove only that at least one repair blocker is needed. -/
def RepairCountGeFourTarget (n : Nat) : Prop :=
  ∀ Bs : Finset (NormBlocker n), CoversAllNonempty Bs → 4 ≤ M2 Bs + M3 Bs

/-- Equivalent repair-count spelling of the same sharp lower-bound route. -/
theorem repairCountGeFourTarget_iff (n : Nat) :
    RepairCountGeFourTarget n ↔
      ∀ Bs : Finset (NormBlocker n), CoversAllNonempty Bs → 4 ≤ repairCount Bs := by
  rfl

/-- Repair-count lower bounds directly rule out smaller-than-`n+4` covers. -/
theorem not_cover_card_lt_n_add_four_of_repairCountGeFour {n : Nat}
    (hrepair : RepairCountGeFourTarget n) {Bs : Finset (NormBlocker n)}
    (hcover : CoversAllNonempty Bs) :
    ¬ Bs.card < n + 4 := by
  intro hlt
  have hM1 : n ≤ M1 Bs := M1_ge_n Bs hcover
  have hRep : 4 ≤ repairCount Bs := by
    unfold repairCount
    exact hrepair Bs hcover
  have hCard : Bs.card = M1 Bs + repairCount Bs := card_eq_M1_add_repairCount Bs
  omega

/-- Under the four-repair rule, every cover of exact size `n + 4` has the extremal shape:
no surplus type-1 blockers and exactly four repairs. -/
theorem extremalShape_of_repairCountGeFour_and_card_n_add_four {n : Nat}
    (hrepair : RepairCountGeFourTarget n) {Bs : Finset (NormBlocker n)}
    (hcover : CoversAllNonempty Bs) (hcard : Bs.card = n + 4) :
    ExactWidth3ExtremalShape Bs := by
  have hM1ge : n ≤ M1 Bs := M1_ge_n Bs hcover
  have hRepge : 4 ≤ repairCount Bs := by
    unfold repairCount
    exact hrepair Bs hcover
  have hCard : Bs.card = M1 Bs + repairCount Bs := card_eq_M1_add_repairCount Bs
  refine ⟨hcover, ?_, ?_⟩
  · unfold TightSingletonSkeleton
    omega
  · unfold SharpFourRepairSkeleton
    omega

/-- Four repair blockers are enough to imply the sharp `n + 4` lower bound, because
ring-1 singletons already force at least `n` type-1 blockers. -/
theorem exactWidth3LowerTarget_of_repairCountGeFour {n : Nat}
    (hrepair : RepairCountGeFourTarget n) :
    ExactWidth3LowerTarget n := by
  intro m hm
  obtain ⟨Bs, hcover, hcard⟩ := hm
  have hM1 : n ≤ M1 Bs := M1_ge_n Bs hcover
  have hRep : 4 ≤ M2 Bs + M3 Bs := hrepair Bs hcover
  have hCard : Bs.card = M1 Bs + M2 Bs + M3 Bs := card_eq_type_counts Bs
  rw [← hcard, hCard]
  omega

/-- The conjectural exact minimum: `n + 4` blockers suffice and no smaller cover exists. -/
def ExactWidth3MinimumTarget (n : Nat) : Prop :=
  ExactWidth3UpperTarget n ∧ ExactWidth3LowerTarget n

/-- Since the gadget proves the upper half, the exact-minimum target is equivalent to
just the lower-bound target in every dimension `n ≥ 3`. -/
theorem exactWidth3MinimumTarget_iff_lowerTarget {n : Nat} (hn : 3 ≤ n) :
    ExactWidth3MinimumTarget n ↔ ExactWidth3LowerTarget n := by
  constructor
  · intro h
    exact h.2
  · intro hlow
    exact ⟨exactWidth3UpperTarget_of_three_le hn, hlow⟩

/-- If the lower-bound target holds, then `n + 4` is a genuine minimum cover size. -/
theorem minimumUniqueCoverSize_n_add_four_of_lowerTarget {n : Nat}
    (hn : 3 ≤ n) (hlow : ExactWidth3LowerTarget n) :
    IsMinimumUniqueCoverSize n (n + 4) := by
  refine ⟨exactWidth3UpperTarget_of_three_le hn, ?_⟩
  intro k hk
  exact hlow k hk

/-- The global exact-width-3 minimum-size conjecture, restricted to dimensions where a
three-coordinate gadget is available. -/
def ExactWidth3MinimumConjecture : Prop :=
  ∀ n : Nat, 3 ≤ n → ExactWidth3MinimumTarget n

/-- Dimensionwise repair-count lower bounds imply the dimensionwise exact-minimum target. -/
theorem exactWidth3MinimumTarget_of_repairCountGeFour {n : Nat}
    (hn : 3 ≤ n) (hrepair : RepairCountGeFourTarget n) :
    ExactWidth3MinimumTarget n :=
  (exactWidth3MinimumTarget_iff_lowerTarget hn).mpr
    (exactWidth3LowerTarget_of_repairCountGeFour hrepair)

/-- The global repair-count program implies the global `n + 4` exact-width-3 conjecture. -/
theorem exactWidth3MinimumConjecture_of_repairCountGeFour
    (hrepair : ∀ n : Nat, 3 ≤ n → RepairCountGeFourTarget n) :
    ExactWidth3MinimumConjecture := by
  intro n hn
  exact exactWidth3MinimumTarget_of_repairCountGeFour hn (hrepair n hn)

/-! ## §I Concrete Boolean circuits and finite lower bounds -/

/-- Boolean functions on fixed-length bitstrings. Concrete circuits naturally compute
objects of this shape rather than dependent `CoverInput`s directly. -/
abbrev BoolFunc (N : Nat) := (Fin N → Bool) → Bool

/-- Tree-shaped Boolean circuits over `N` input bits. This concrete syntax is intentionally
small: it is enough for structural induction, size/depth accounting, and restricted lower
bound arguments. DAG circuits can be layered on later. -/
inductive BoolCircuit (N : Nat) where
  | input : Fin N → BoolCircuit N
  | const : Bool → BoolCircuit N
  | not : BoolCircuit N → BoolCircuit N
  | and : BoolCircuit N → BoolCircuit N → BoolCircuit N
  | or : BoolCircuit N → BoolCircuit N → BoolCircuit N
  deriving Repr

namespace BoolCircuit

/-- Evaluate a concrete Boolean circuit on an input bitstring. -/
def eval {N : Nat} : BoolCircuit N → (Fin N → Bool) → Bool
  | input i, x => x i
  | const b, _ => b
  | not C, x => !(eval C x)
  | and A B, x => eval A x && eval B x
  | or A B, x => eval A x || eval B x

/-- Gate count, counting only non-input/non-constant gates. -/
def size {N : Nat} : BoolCircuit N → Nat
  | input _ => 0
  | const _ => 0
  | not C => size C + 1
  | and A B => size A + size B + 1
  | or A B => size A + size B + 1

/-- Circuit depth, with inputs and constants at depth zero. -/
def depth {N : Nat} : BoolCircuit N → Nat
  | input _ => 0
  | const _ => 0
  | not C => depth C + 1
  | and A B => max (depth A) (depth B) + 1
  | or A B => max (depth A) (depth B) + 1

/-- Circuit `C` computes Boolean function `f`. -/
def Computes {N : Nat} (C : BoolCircuit N) (f : BoolFunc N) : Prop :=
  ∀ x, C.eval x = f x

/-- Two circuits are extensionally equivalent. -/
def Equivalent {N : Nat} (C D : BoolCircuit N) : Prop :=
  ∀ x, C.eval x = D.eval x

/-- Syntactic monotone circuits: no `not` gates. -/
def IsMonotone {N : Nat} : BoolCircuit N → Prop
  | input _ => True
  | const _ => True
  | not _ => False
  | and A B => IsMonotone A ∧ IsMonotone B
  | or A B => IsMonotone A ∧ IsMonotone B

/-- Depth-bounded circuit class. -/
def IsDepthAtMost {N : Nat} (C : BoolCircuit N) (d : Nat) : Prop :=
  C.depth ≤ d

@[simp] theorem eval_input {N : Nat} (i : Fin N) (x : Fin N → Bool) :
    (BoolCircuit.input i).eval x = x i := rfl

@[simp] theorem eval_const {N : Nat} (b : Bool) (x : Fin N → Bool) :
    (BoolCircuit.const b).eval x = b := rfl

end BoolCircuit

/-- Pointwise order on bitstrings. -/
def AssignmentLe {N : Nat} (x y : Fin N → Bool) : Prop :=
  ∀ i, x i = true → y i = true

namespace BoolFunc

/-- Monotone Boolean functions preserve the pointwise order on assignments. -/
def Monotone {N : Nat} (f : BoolFunc N) : Prop :=
  ∀ x y, AssignmentLe x y → f x = true → f y = true

end BoolFunc

/-- Every syntactically monotone circuit computes a semantically monotone function. -/
theorem BoolCircuit.eval_monotone_of_isMonotone {N : Nat} {C : BoolCircuit N}
    (hC : C.IsMonotone) :
    BoolFunc.Monotone C.eval := by
  induction C with
  | input i =>
      intro x y hxy hx
      exact hxy i hx
  | const b =>
      intro x y hxy hx
      exact hx
  | not C ih =>
      contradiction
  | and A B ihA ihB =>
      rcases hC with ⟨hA, hB⟩
      intro x y hxy htrue
      rw [BoolCircuit.eval] at htrue ⊢
      rw [Bool.and_eq_true] at htrue ⊢
      exact ⟨ihA hA x y hxy htrue.1, ihB hB x y hxy htrue.2⟩
  | or A B ihA ihB =>
      rcases hC with ⟨hA, hB⟩
      intro x y hxy htrue
      rw [BoolCircuit.eval] at htrue ⊢
      rw [Bool.or_eq_true] at htrue ⊢
      rcases htrue with htrue | htrue
      · exact Or.inl (ihA hA x y hxy htrue)
      · exact Or.inr (ihB hB x y hxy htrue)

/-- Semantic non-monotonicity rules out every syntactically monotone circuit. -/
theorem nonmonotone_not_computable_by_monotone {N : Nat} {f : BoolFunc N}
    (hf : ¬ BoolFunc.Monotone f) :
    ∀ C : BoolCircuit N, C.IsMonotone → ¬ C.Computes f := by
  intro C hC hcomp
  apply hf
  intro x y hxy hx
  have hxC : C.eval x = true := by
    rw [hcomp x]
    exact hx
  have hyC : C.eval y = true :=
    BoolCircuit.eval_monotone_of_isMonotone hC x y hxy hxC
  rw [← hcomp y]
  exact hyC

/-- Finite lower-bound statement: every circuit computing `f` has size greater than `s`. -/
def RequiresSizeMoreThan {N : Nat} (f : BoolFunc N) (s : Nat) : Prop :=
  ∀ C : BoolCircuit N, C.Computes f → C.size > s

/-- Finite lower-bound statement with a non-strict threshold. This form is convenient for
literal exponential statements such as `2^N ≤ C.size`. -/
def RequiresSizeAtLeast {N : Nat} (f : BoolFunc N) (s : Nat) : Prop :=
  ∀ C : BoolCircuit N, C.Computes f → s ≤ C.size

/-- Promise/restricted correctness: a circuit only has to agree with `f` on inputs satisfying
the promise `hard`. This is useful for hard-envelope or restricted-instance lower bounds. -/
def CircuitCorrectOn {N : Nat} (C : BoolCircuit N) (hard : (Fin N → Bool) → Prop)
    (f : BoolFunc N) : Prop :=
  ∀ x, hard x → C.eval x = f x

/-- A concrete nonuniform circuit family: one Boolean circuit at each input length. -/
structure ConcreteCircuitFamily where
  circuit : (N : Nat) → BoolCircuit N

namespace ConcreteCircuitFamily

/-- Gate-count column of a concrete circuit family. -/
def gateCount (F : ConcreteCircuitFamily) (N : Nat) : Nat :=
  (F.circuit N).size

/-- The family decides a length-indexed Boolean-function family. -/
def Decides (F : ConcreteCircuitFamily) (L : (N : Nat) → BoolFunc N) : Prop :=
  ∀ N x, (F.circuit N).eval x = L N x

end ConcreteCircuitFamily

/-- Concrete polynomial-size circuit families. -/
def ConcretePolynomialSizeCircuitFamily (F : ConcreteCircuitFamily) : Prop :=
  PolynomialO F.gateCount

/-- Nonuniform circuit-solving target for a Boolean-function family. -/
def InPpoly (L : (N : Nat) → BoolFunc N) : Prop :=
  ∃ F : ConcreteCircuitFamily,
    F.Decides L ∧ ConcretePolynomialSizeCircuitFamily F

/-- Super-polynomial concrete circuit lower bound for a Boolean-function family. -/
def SuperPolynomialCircuitLowerBound (L : (N : Nat) → BoolFunc N) : Prop :=
  ∀ k : Nat, ∃ N₀ : Nat, ∀ N, N₀ ≤ N →
    RequiresSizeMoreThan (L N) ((N + 1) ^ k)

/-- Literal exponential lower-bound target for unrestricted circuits deciding a Boolean
function family. This says that, past some input length, every concrete circuit computing
`L N` has at least `2^N` gates.

This is the precise shape of "general circuits for this SAT encoding go exponential." -/
def ExponentialCircuitLowerBound (L : (N : Nat) → BoolFunc N) : Prop :=
  ∃ N₀ : Nat, ∀ N, N₀ ≤ N →
    RequiresSizeAtLeast (L N) (2 ^ N)

/-- Super-polynomial lower bounds rule out polynomial-size circuit families. -/
theorem not_inPpoly_of_superPolynomialCircuitLowerBound {L : (N : Nat) → BoolFunc N}
    (hlower : SuperPolynomialCircuitLowerBound L) :
    ¬ InPpoly L := by
  rintro ⟨F, hdec, hpoly⟩
  obtain ⟨k, hO⟩ := hpoly
  obtain ⟨C, Nbig, hbound⟩ := hO
  let K := k + C + 1
  obtain ⟨N₀, hlb⟩ := hlower K
  let N := max (max N₀ Nbig) 1
  have hN₀ : N₀ ≤ N := le_trans (Nat.le_max_left _ _) (Nat.le_max_left _ _)
  have hNbig : Nbig ≤ N := le_trans (Nat.le_max_right _ _) (Nat.le_max_left _ _)
  have hNbase : 1 < N + 1 := by omega
  have hsize_gt : (F.circuit N).size > (N + 1) ^ K := by
    exact hlb N hN₀ (F.circuit N) (hdec N)
  have hsize_le_base : (F.circuit N).size ≤ C * (N + 1) ^ k := by
    exact hbound N hNbig
  have hbase_le : C * (N + 1) ^ k ≤ (N + 1) ^ K := by
    have hC : C ≤ (N + 1) ^ C := by
      exact le_of_lt (Nat.lt_pow_self hNbase)
    calc
      C * (N + 1) ^ k ≤ (N + 1) ^ C * (N + 1) ^ k :=
        Nat.mul_le_mul_right ((N + 1) ^ k) hC
      _ = (N + 1) ^ (C + k) := by rw [← Nat.pow_add]
      _ ≤ (N + 1) ^ K := by
        apply Nat.pow_le_pow_right
        · exact Nat.succ_pos N
        · unfold K
          omega
  omega

/-- Circuit version of the 3-SAT lower-bound target. This is a nonuniform strengthening of
`ThreeSATNotInP` once a standard polynomial-time-to-polynomial-size-circuit simulation is
formalized for this project. -/
def ThreeSATCircuitLowerBound (ThreeSATBitFunction : (N : Nat) → BoolFunc N) : Prop :=
  SuperPolynomialCircuitLowerBound ThreeSATBitFunction

/-- P/poly-style circuit statement for a chosen bit-level 3-SAT encoding. -/
def ThreeSATInPpoly (ThreeSATBitFunction : (N : Nat) → BoolFunc N) : Prop :=
  InPpoly ThreeSATBitFunction

/-- Nonuniform lower-bound target for a chosen bit-level 3-SAT encoding. This is stronger
than `ThreeSATNotInP` once the standard uniform-to-nonuniform simulation is formalized. -/
def ThreeSATNotInPpoly (ThreeSATBitFunction : (N : Nat) → BoolFunc N) : Prop :=
  ¬ ThreeSATInPpoly ThreeSATBitFunction

/-- A bit-level encoding of compact cover-SAT instances. The decoder is intentionally
abstract here: concrete encodings can instantiate it later without changing the circuit
route statements. -/
structure CoverSATBitEncoding where
  decode : (N : Nat) → (Fin N → Bool) → CoverInput

/-- A faithful bit encoding of cover-SAT instances. The raw decoder may map ill-formed
bitstrings to arbitrary default instances, but every actual `CoverInput` has a bitstring
representation that decodes back to it, with polynomially bounded length.

This is the missing semantic condition that turns an arbitrary bit function into a genuine
encoding of the project-level SAT problem. -/
structure FaithfulCoverSATBitEncoding where
  encoding : CoverSATBitEncoding
  length : CoverInput → Nat
  encode : (I : CoverInput) → Fin (length I) → Bool
  lengthBound : Nat → Nat
  length_le_bound : ∀ I : CoverInput, length I ≤ lengthBound I.size
  lengthBound_poly : PolynomialO lengthBound
  decode_encode : ∀ I : CoverInput, encoding.decode (length I) (encode I) = I

namespace CoverSATBitEncoding

/-- The Boolean-function family induced by a bit-level cover-SAT encoding. -/
def bitFunction (E : CoverSATBitEncoding) : (N : Nat) → BoolFunc N :=
  fun N bits => decide (CoverSAT (E.decode N bits))

/-- P/poly statement for the encoded cover-SAT language. -/
def InPpoly (E : CoverSATBitEncoding) : Prop :=
  ThreeSATInPpoly E.bitFunction

/-- Nonuniform lower-bound target for the encoded cover-SAT language. -/
def NotInPpoly (E : CoverSATBitEncoding) : Prop :=
  ThreeSATNotInPpoly E.bitFunction

/-- Super-polynomial circuit lower-bound statement for the encoded cover-SAT language. -/
def CircuitLowerBound (E : CoverSATBitEncoding) : Prop :=
  ThreeSATCircuitLowerBound E.bitFunction

/-- Stronger literal exponential statement for unrestricted circuits deciding this encoded
cover-SAT language.

This is intentionally a named proposition, not an axiom or `sorry` theorem: it records the
open target exactly without assuming it. -/
def GeneralCircuitsGoExponential (E : CoverSATBitEncoding) : Prop :=
  ExponentialCircuitLowerBound E.bitFunction

/-- The theorem-shaped open target for a fixed bit-level cover-SAT/3-SAT encoding.

Proving this, for a concrete faithful encoding, is exactly the unrestricted exponential
circuit lower-bound claim: every general Boolean circuit deciding encoded SAT eventually
has at least `2^N` gates. -/
def GeneralCircuitExponentialTheoremTarget (E : CoverSATBitEncoding) : Prop :=
  ∃ N₀ : Nat, ∀ N, N₀ ≤ N →
    ∀ C : BoolCircuit N, C.Computes (E.bitFunction N) → 2 ^ N ≤ C.size

/-- The target theorem is definitionally the same as `GeneralCircuitsGoExponential`; this
lemma lets us use whichever spelling is clearer in later developments. -/
theorem generalCircuitExponentialTheoremTarget_iff (E : CoverSATBitEncoding) :
    E.GeneralCircuitExponentialTheoremTarget ↔ E.GeneralCircuitsGoExponential := by
  rfl

end CoverSATBitEncoding

namespace FaithfulCoverSATBitEncoding

/-- The Boolean-function family induced by the underlying faithful encoding. -/
def bitFunction (E : FaithfulCoverSATBitEncoding) : (N : Nat) → BoolFunc N :=
  E.encoding.bitFunction

/-- The theorem-shaped unrestricted exponential lower-bound target, now with the
faithfulness assumptions packaged into the encoding object. -/
def GeneralCircuitExponentialTheoremTarget (E : FaithfulCoverSATBitEncoding) : Prop :=
  E.encoding.GeneralCircuitExponentialTheoremTarget

/-- Equivalent shorter spelling for the same faithful-encoding target. -/
def GeneralCircuitsGoExponential (E : FaithfulCoverSATBitEncoding) : Prop :=
  E.encoding.GeneralCircuitsGoExponential

/-- Faithfulness says encoded project instances are evaluated as their original
`CoverSAT` predicate. -/
theorem bitFunction_encode_eq (E : FaithfulCoverSATBitEncoding) (I : CoverInput) :
    E.bitFunction (E.length I) (E.encode I) = decide (CoverSAT I) := by
  rw [bitFunction, CoverSATBitEncoding.bitFunction, E.decode_encode I]

/-- The faithful theorem target is exactly the raw target for the underlying encoding. -/
theorem generalCircuitExponentialTheoremTarget_iff (E : FaithfulCoverSATBitEncoding) :
    E.GeneralCircuitExponentialTheoremTarget ↔ E.encoding.GeneralCircuitsGoExponential := by
  rfl

end FaithfulCoverSATBitEncoding

end HypercubeSAT

/-! ### Local FormalConjectures compatibility layer -/

/-
This project is intentionally a single Lean file.  The upstream FormalConjectures file imports
`FormalConjectures.Util.ProblemImports`, which is not a dependency of this Lake project, so we copy
the public complexity-theory interface here and leave the Turing-machine predicates abstract.

If the external package is added later, this section can be replaced by:

```
import FormalConjectures.Util.ProblemImports
open Computability Turing
```

and the definitions below have the same names and shapes needed by our bridge.
-/

open Computability Turing

namespace Computability

/-- Boolean lists are encoded over the Boolean alphabet by identity. This supplies the name used
by the FormalConjectures complexity statement. -/
def finEncodingListBool : FinEncoding (List Bool) where
  Γ := Bool
  encode := id
  decode := some
  decode_encode := by intro x; rfl
  ΓFin := Bool.fintype

/-- Encoding of pairs of Boolean lists, kept abstract here because only the complexity-class
interface needs the object. A concrete delimiter encoding can replace this later. -/
axiom finEncodingListBoolProdListBool : FinEncoding (List Bool × List Bool)

end Computability

namespace ComplexityTheory

/- FormalConjectures' polynomial-time TM object, locally wrapped so this single-file project can
use the same class definitions even though Mathlib's `Turing.TM2ComputableInPolyTime` has a
different low-level signature. -/
axiom TM2ComputableInPolyTimeFC {α β : Type} :
  FinEncoding α → FinEncoding β → (α → β) → Type

/--
The type of decision problems.

We define these as functions from lists of booleans to booleans,
implicitly assuming the usual encodings.
-/
abbrev DecisionProblem := List Bool → Bool

/--
The type of complexity classes. We define these as sets of decision problems.
-/
abbrev ComplexityClass := Set DecisionProblem

/--
A simple definition to abstract the notion of a poly-time Turing machine into a predicate.
-/
def IsComputableInPolyTime {α β : Type} (ea : FinEncoding α) (eb : FinEncoding β)
    (f : α → β) :=
  Nonempty (TM2ComputableInPolyTimeFC ea eb f)

/--
The class P is the set of decision problems decidable in polynomial time by a deterministic
Turing machine.
-/
def P : ComplexityClass :=
  { L | IsComputableInPolyTime finEncodingListBool finEncodingBoolBool L }

/--
The class NP is the set of decision problems such that there exists a polynomial `p` over `ℕ`
and a poly-time verifier accepting `(x,w)` exactly for polynomially bounded witnesses.
-/
def NP : ComplexityClass :=
  { L | ∃ (p : Polynomial ℕ), ∃ R : (List Bool × List Bool) → Bool,
      IsComputableInPolyTime finEncodingListBoolProdListBool finEncodingBoolBool R ∧
      ∀ x, L x ↔ ∃ w : List Bool, w.length ≤ p.eval x.length ∧ R (x, w) }

/--
The class coNP is the set of decision problems whose complements are in NP.
-/
def coNP : ComplexityClass :=
  { L | Lᶜ ∈ NP }

/--
**P ≠ NP**: the conjecture that the complexity classes P and NP are not equal.

This is the standard open problem. It is intentionally left as a theorem-shaped
placeholder rather than converted into a proposition or axiom.
-/
theorem P_ne_NP : P ≠ NP := by
  sorry

/--
**NP ≠ coNP**: the conjecture that the complexity classes NP and coNP are not equal.

This is also open in the local FormalConjectures-style interface.
-/
theorem NP_ne_coNP : NP ≠ coNP := by
  sorry

/--
The theorem that the set of complements of languages in P is itself P.

Blocked in this single-file compatibility layer: `TM2ComputableInPolyTimeFC` is
opaque, and the file currently provides no complement-closure constructor turning
a machine for `L` into one for `Lᶜ`.
-/
theorem coP_eq_P :
    { L | Lᶜ ∈ P } = P := by
  sorry

/--
The theorem that P is a subset of NP.

Blocked by the same abstraction boundary: the local interface has no closure rule
that lifts a poly-time decider for `L` to a poly-time verifier on input/witness
pairs.
-/
theorem P_subset_NP :
    P ⊆ NP := by
  sorry

/--
The theorem that P is a subset of coNP.
-/
theorem P_subset_coNP :
    P ⊆ coNP := by
  rw [coNP, ← coP_eq_P]
  simp only [Set.setOf_subset_setOf]
  intros L hL
  exact P_subset_NP hL

end ComplexityTheory

namespace HypercubeSAT

/-! ### FormalConjectures-style decision-problem bridge -/

/-- Local alias for the FormalConjectures decision-problem type. -/
abbrev FCDecisionProblem := ComplexityTheory.DecisionProblem

/-- Convert a fixed-length Boolean-function family into one list-level decision problem by
using the input list length as the arity and `List.get` as the bit vector. -/
def BoolFuncFamily.toDecisionProblem (L : (N : Nat) → BoolFunc N) : FCDecisionProblem :=
  fun bits => L bits.length bits.get

/-- The FormalConjectures-shaped language induced by a cover-SAT bit encoding. -/
def CoverSATBitEncoding.decisionProblem (E : CoverSATBitEncoding) : FCDecisionProblem :=
  BoolFuncFamily.toDecisionProblem E.bitFunction

/-- The FormalConjectures-shaped language induced by a faithful cover-SAT bit encoding. -/
def FaithfulCoverSATBitEncoding.decisionProblem (E : FaithfulCoverSATBitEncoding) :
    FCDecisionProblem :=
  E.encoding.decisionProblem

theorem CoverSATBitEncoding.decisionProblem_eq {E : CoverSATBitEncoding} (bits : List Bool) :
    E.decisionProblem bits =
      decide (CoverSAT (E.decode bits.length bits.get)) := by
  rfl

/-- Witness codec for the NP verifier view of encoded cover-SAT. The formula/input is decoded
first; the witness is then decoded relative to that decoded instance, because its vertex dimension
is `I.n`. -/
structure CoverSATWitnessCodec where
  decodeWitness : (I : CoverInput) → List Bool → Option (Vertex I.n)
  encodeWitness : (I : CoverInput) → Vertex I.n → List Bool
  lengthBound : CoverInput → Nat
  length_encode_le : ∀ (I : CoverInput) (v : Vertex I.n),
    (encodeWitness I v).length ≤ lengthBound I
  lengthBound_poly : ∃ p : Nat → Nat, PolynomialO p ∧ ∀ I : CoverInput,
    lengthBound I ≤ p I.size
  decode_encode : ∀ (I : CoverInput) (v : Vertex I.n),
    decodeWitness I (encodeWitness I v) = some v

namespace CoverSATWitnessCodec

/-- The verifier relation for encoded cover-SAT: decode the instance, decode a candidate
assignment witness for that instance, then check uncoveredness. -/
def verifier (E : CoverSATBitEncoding) (W : CoverSATWitnessCodec) :
    (List Bool × List Bool) → Bool :=
  fun inputWitness =>
    let I := E.decode inputWitness.1.length inputWitness.1.get
    match W.decodeWitness I inputWitness.2 with
    | none => false
    | some v => decide (IsUncovered I.blockers v)

theorem verifier_encodeWitness_eq_true {E : CoverSATBitEncoding} (W : CoverSATWitnessCodec)
    (I : CoverInput) (inputBits : List Bool) (hdecode : E.decode inputBits.length inputBits.get = I)
    (v : Vertex I.n) (hv : IsUncovered I.blockers v) :
    W.verifier E (inputBits, W.encodeWitness I v) = true := by
  subst I
  unfold verifier
  simp [W.decode_encode, hv]

end CoverSATWitnessCodec

/-- Semantic NP condition for the encoded cover-SAT language, excluding only the Turing-machine
computability obligation from FormalConjectures. With an actual `IsComputableInPolyTime` proof
for `W.verifier E`, this is exactly the payload needed for `ComplexityTheory.NP`. -/
def CoverSATBitEncoding.SemanticNPVerifier (E : CoverSATBitEncoding)
    (W : CoverSATWitnessCodec) : Prop :=
  ∃ p : Nat → Nat, PolynomialO p ∧ ∀ bits : List Bool,
    E.decisionProblem bits = true ↔
      ∃ witness : List Bool, witness.length ≤ p bits.length ∧
        W.verifier E (bits, witness) = true

/-- A faithful input encoding plus a faithful witness codec gives the existential verifier
direction for every encoded yes-instance. This is the SAT-witness half of the NP bridge. -/
theorem FaithfulCoverSATBitEncoding.exists_verifier_witness_of_coverSAT
    (E : FaithfulCoverSATBitEncoding) (W : CoverSATWitnessCodec)
    (inputBits : List Bool) (I : CoverInput)
    (hdecode : E.encoding.decode inputBits.length inputBits.get = I)
    (hI : CoverSAT I) :
    ∃ witness : List Bool,
      witness.length ≤ W.lengthBound I ∧
        W.verifier E.encoding
          (inputBits, witness) = true := by
  obtain ⟨v, hv⟩ := hI
  refine ⟨W.encodeWitness I v, W.length_encode_le I v, ?_⟩
  apply W.verifier_encodeWitness_eq_true
  · exact hdecode
  · exact hv

/-- Any accepting verifier witness is semantically sound: it decodes to an uncovered assignment
for the decoded cover-SAT instance. -/
theorem CoverSATWitnessCodec.coverSAT_of_verifier_eq_true
    (E : CoverSATBitEncoding) (W : CoverSATWitnessCodec)
    (bits witness : List Bool)
    (h : W.verifier E (bits, witness) = true) :
    CoverSAT (E.decode bits.length bits.get) := by
  unfold verifier at h
  cases hdec : W.decodeWitness (E.decode bits.length bits.get) witness with
  | none =>
      simp [hdec] at h
  | some v =>
      refine ⟨v, ?_⟩
      exact of_decide_eq_true (by simpa [hdec] using h)

/-- Thus the list-level encoded cover-SAT language fits the FormalConjectures decision-problem
shape: it is literally a `List Bool → Bool`, with a polynomially bounded witness/verifier target
separated out as `SemanticNPVerifier`. -/
theorem CoverSATBitEncoding.decisionProblem_true_iff_coverSAT
    (E : CoverSATBitEncoding) (bits : List Bool) :
    E.decisionProblem bits = true ↔ CoverSAT (E.decode bits.length bits.get) := by
  rw [CoverSATBitEncoding.decisionProblem_eq]
  exact decide_eq_true_iff

/-- FormalConjectures `P` target for this project: the encoded cover-SAT decision problem is
polynomial-time decidable as a list language. -/
def CoverSATBitEncoding.FormalConjecturesPTimeTarget (E : CoverSATBitEncoding) : Prop :=
  E.decisionProblem ∈ ComplexityTheory.P

/-- FormalConjectures `NP` target for this project. -/
def CoverSATBitEncoding.FormalConjecturesNPTarget (E : CoverSATBitEncoding) : Prop :=
  E.decisionProblem ∈ ComplexityTheory.NP

/-- Exact FormalConjectures verifier package for the encoded cover-SAT language. -/
structure CoverSATBitEncoding.FormalNPVerifier (E : CoverSATBitEncoding) where
  p : Polynomial ℕ
  R : (List Bool × List Bool) → Bool
  computable :
    ComplexityTheory.IsComputableInPolyTime
      finEncodingListBoolProdListBool finEncodingBoolBool R
  spec : ∀ x, E.decisionProblem x ↔
    ∃ w : List Bool, w.length ≤ p.eval x.length ∧ R (x, w)

/-- A packaged FormalConjectures verifier places the encoded cover-SAT language in `NP`. -/
theorem CoverSATBitEncoding.formalNPTarget_of_formalVerifier
    {E : CoverSATBitEncoding} (V : E.FormalNPVerifier) :
    E.FormalConjecturesNPTarget := by
  exact ⟨V.p, V.R, V.computable, V.spec⟩

/-- Exact FormalConjectures decider package for the encoded cover-SAT language. -/
structure CoverSATBitEncoding.FormalPDecider (E : CoverSATBitEncoding) where
  computable :
    ComplexityTheory.IsComputableInPolyTime
      finEncodingListBool finEncodingBoolBool E.decisionProblem

/-- A packaged FormalConjectures decider places the encoded cover-SAT language in `P`. -/
theorem CoverSATBitEncoding.formalPTarget_of_formalDecider
    {E : CoverSATBitEncoding} (D : E.FormalPDecider) :
    E.FormalConjecturesPTimeTarget :=
  D.computable

/-- Lower bounds against the chosen bit-level 3-SAT family rule out the corresponding
nonuniform polynomial-size circuit family. -/
theorem threeSAT_circuit_lower_bound_implies_not_in_Ppoly
    {ThreeSATBitFunction : (N : Nat) → BoolFunc N}
    (h : ThreeSATCircuitLowerBound ThreeSATBitFunction) :
    ThreeSATNotInPpoly ThreeSATBitFunction :=
  not_inPpoly_of_superPolynomialCircuitLowerBound h

/-- Alias with the textbook wording: super-polynomial circuit lower bounds rule out P/poly. -/
theorem superpoly_lower_bound_implies_not_Ppoly {L : (N : Nat) → BoolFunc N}
    (h : SuperPolynomialCircuitLowerBound L) :
    ThreeSATNotInPpoly L :=
  not_inPpoly_of_superPolynomialCircuitLowerBound h

/-- Standard simulation bridge, expressed as a hypothesis rather than an axiom: a uniform
polynomial-time decider for cover-SAT yields a nonuniform polynomial-size circuit family for
the chosen bit-level language. -/
def UniformToCircuitSimulation (L : (N : Nat) → BoolFunc N) : Prop :=
  ThreeSATInP → ThreeSATInPpoly L

/-- Under the standard simulation bridge, nonuniform lower bounds imply the uniform
project-level 3-SAT form of P ≠ NP. -/
theorem threeSATNotInP_of_threeSATNotInPpoly {L : (N : Nat) → BoolFunc N}
    (hsim : UniformToCircuitSimulation L) :
    ThreeSATNotInPpoly L → ThreeSATNotInP := by
  intro hnp hp
  exact hnp (hsim hp)

/-- Circuit route to the project-level P-vs-NP-shaped target. This is the theorem to put
on the wall: once the standard simulation bridge is available, a super-polynomial circuit
lower bound for the encoded 3-SAT language implies `ThreeSATNotInP`. -/
theorem p_vs_np_from_circuit_lower_bound {L : (N : Nat) → BoolFunc N}
    (hsim : UniformToCircuitSimulation L) :
    ThreeSATCircuitLowerBound L → PvsNP_3SAT_Form := by
  intro hlower
  exact threeSATNotInP_of_threeSATNotInPpoly hsim
    (threeSAT_circuit_lower_bound_implies_not_in_Ppoly hlower)

/-- Encoding-specialized version of the circuit route. -/
theorem p_vs_np_from_encoded_coverSAT_circuit_lower_bound
    (E : CoverSATBitEncoding)
    (hsim : UniformToCircuitSimulation E.bitFunction) :
    E.CircuitLowerBound → PvsNP_3SAT_Form := by
  intro hlower
  exact p_vs_np_from_circuit_lower_bound hsim hlower

/-! ### Formula-evaluation verifier circuits -/

/-- Circuit for one literal over an assignment input. -/
def litCircuit {n : Nat} (l : Lit n) : BoolCircuit n :=
  if l.pos then
    BoolCircuit.input l.var
  else
    BoolCircuit.not (BoolCircuit.input l.var)

/-- Circuit for one clean 3-clause over an assignment input. -/
def clauseCircuit {n : Nat} (C : Clause3 n) : BoolCircuit n :=
  BoolCircuit.or (BoolCircuit.or (litCircuit C.a) (litCircuit C.b)) (litCircuit C.c)

/-- Circuit evaluating a fixed 3-CNF formula on an assignment input. This is the circuit
analogue of the NP verifier, not a SAT solver. -/
def formulaEvalCircuit {n : Nat} : Formula3 n → BoolCircuit n
  | [] => BoolCircuit.const true
  | C :: Cs => BoolCircuit.and (clauseCircuit C) (formulaEvalCircuit Cs)

theorem litCircuit_correct {n : Nat} (l : Lit n) :
    (litCircuit l).Computes l.eval := by
  intro x
  unfold litCircuit Lit.eval
  cases l.pos <;> rfl

theorem clauseCircuit_correct {n : Nat} (C : Clause3 n) :
    (clauseCircuit C).Computes C.eval := by
  intro x
  change ((litCircuit C.a).eval x || (litCircuit C.b).eval x || (litCircuit C.c).eval x) =
    (C.a.eval x || C.b.eval x || C.c.eval x)
  rw [litCircuit_correct C.a x, litCircuit_correct C.b x, litCircuit_correct C.c x]

theorem formulaEvalCircuit_correct {n : Nat} (F : Formula3 n) :
    (formulaEvalCircuit F).Computes F.eval := by
  intro x
  induction F with
  | nil =>
      rfl
  | cons C Cs ih =>
      change ((clauseCircuit C).eval x && (formulaEvalCircuit Cs).eval x) =
        (C.eval x && Formula3.eval Cs x)
      rw [clauseCircuit_correct C x, ih]

theorem litCircuit_size_le_one {n : Nat} (l : Lit n) :
    (litCircuit l).size ≤ 1 := by
  unfold litCircuit
  by_cases hpos : l.pos
  · simp [hpos, BoolCircuit.size]
  · simp [hpos, BoolCircuit.size]

theorem clauseCircuit_size_le_five {n : Nat} (C : Clause3 n) :
    (clauseCircuit C).size ≤ 5 := by
  unfold clauseCircuit
  have ha := litCircuit_size_le_one C.a
  have hb := litCircuit_size_le_one C.b
  have hc := litCircuit_size_le_one C.c
  simp [BoolCircuit.size]
  omega

/-- Formula verifier circuit size is linear in the number of clauses. -/
theorem formulaEvalCircuit_size_le {n : Nat} (F : Formula3 n) :
    (formulaEvalCircuit F).size ≤ 6 * F.length := by
  induction F with
  | nil =>
      simp [formulaEvalCircuit, BoolCircuit.size]
  | cons C Cs ih =>
      unfold formulaEvalCircuit
      have hC := clauseCircuit_size_le_five C
      simp [BoolCircuit.size]
      omega

/-! ### DNF lower-bound skeletons over hypercube patterns -/

/-- A DNF over the hypercube is a finite OR of patterns/subcubes. -/
abbrev DNF (n : Nat) := Finset (Pattern n)

namespace DNF

/-- DNF evaluation: true iff some term covers the vertex. -/
noncomputable def eval {n : Nat} (D : DNF n) (v : Vertex n) : Bool := by
  classical
  exact decide (∃ P ∈ D, P.Covers v)

/-- A DNF computes a Boolean function on hypercube vertices. -/
def Computes {n : Nat} (D : DNF n) (f : Vertex n → Bool) : Prop :=
  ∀ v, eval D v = f v

/-- Number of DNF terms. -/
def termCount {n : Nat} (D : DNF n) : Nat := D.card

/-- Finite DNF lower-bound statement: every DNF computing `f` has more than `s` terms. -/
def RequiresTermCountMoreThan {n : Nat} (f : Vertex n → Bool) (s : Nat) : Prop :=
  ∀ D : DNF n, D.Computes f → D.termCount > s

theorem eval_true_iff {n : Nat} (D : DNF n) (v : Vertex n) :
    D.eval v = true ↔ ∃ P ∈ D, P.Covers v := by
  classical
  unfold eval
  exact decide_eq_true_iff

end DNF

/-! ### DNF circuits -/

/-- Circuit for a single fixed literal constraint `x_i = b`. -/
def fixedBitCircuit {n : Nat} (ib : Fin n × Bool) : BoolCircuit n :=
  if ib.2 then
    BoolCircuit.input ib.1
  else
    BoolCircuit.not (BoolCircuit.input ib.1)

@[simp] theorem fixedBitCircuit_eval_true_iff {n : Nat} (ib : Fin n × Bool)
    (v : Vertex n) :
    (fixedBitCircuit ib).eval v = true ↔ v ib.1 = ib.2 := by
  cases ib with
  | mk i b =>
      cases b <;> simp [fixedBitCircuit, BoolCircuit.eval]

/-- Right-associated conjunction of a list of circuits. Empty conjunction is `true`. -/
def andListCircuit {n : Nat} : List (BoolCircuit n) → BoolCircuit n
  | [] => BoolCircuit.const true
  | C :: Cs => BoolCircuit.and C (andListCircuit Cs)

/-- Right-associated disjunction of a list of circuits. Empty disjunction is `false`. -/
def orListCircuit {n : Nat} : List (BoolCircuit n) → BoolCircuit n
  | [] => BoolCircuit.const false
  | C :: Cs => BoolCircuit.or C (orListCircuit Cs)

theorem andListCircuit_eval_true_iff {n : Nat} (Cs : List (BoolCircuit n))
    (v : Vertex n) :
    (andListCircuit Cs).eval v = true ↔ ∀ C ∈ Cs, C.eval v = true := by
  induction Cs with
  | nil =>
      simp [andListCircuit, BoolCircuit.eval]
  | cons C Cs ih =>
      rw [andListCircuit, BoolCircuit.eval, Bool.and_eq_true]
      constructor
      · rintro ⟨hC, hCs⟩ D hD
        rcases List.mem_cons.mp hD with hEq | hMem
        · subst D
          exact hC
        · exact ih.mp hCs D hMem
      · intro hAll
        exact ⟨hAll C (by simp), ih.mpr fun D hD => hAll D (by simp [hD])⟩

theorem orListCircuit_eval_true_iff {n : Nat} (Cs : List (BoolCircuit n))
    (v : Vertex n) :
    (orListCircuit Cs).eval v = true ↔ ∃ C ∈ Cs, C.eval v = true := by
  induction Cs with
  | nil =>
      simp [orListCircuit, BoolCircuit.eval]
  | cons C Cs ih =>
      rw [orListCircuit, BoolCircuit.eval, Bool.or_eq_true]
      constructor
      · intro h
        rcases h with hC | hCs
        · exact ⟨C, by simp, hC⟩
        · obtain ⟨D, hD, hEval⟩ := ih.mp hCs
          exact ⟨D, by simp [hD], hEval⟩
      · rintro ⟨D, hD, hEval⟩
        rcases List.mem_cons.mp hD with hEq | hMem
        · subst D
          exact Or.inl hEval
        · exact Or.inr (ih.mpr ⟨D, hMem, hEval⟩)

theorem orListCircuit_size_ge_length {n : Nat} (Cs : List (BoolCircuit n)) :
    Cs.length ≤ (orListCircuit Cs).size := by
  induction Cs with
  | nil =>
      simp [orListCircuit, BoolCircuit.size]
  | cons C Cs ih =>
      rw [orListCircuit, BoolCircuit.size]
      rw [List.length_cons]
      exact Nat.succ_le_succ (Nat.le_trans ih (Nat.le_add_left _ _))

namespace Pattern

/-- Canonical AND-circuit for a pattern/subcube. -/
noncomputable def toCircuit {n : Nat} (P : Pattern n) : BoolCircuit n :=
  andListCircuit (P.fixed.toList.map fixedBitCircuit)

theorem toCircuit_eval_true_iff {n : Nat} (P : Pattern n) (v : Vertex n) :
    P.toCircuit.eval v = true ↔ P.Covers v := by
  constructor
  · intro h ib hib
    have hAll := (andListCircuit_eval_true_iff (P.fixed.toList.map fixedBitCircuit) v).mp h
    have hmem : fixedBitCircuit ib ∈ P.fixed.toList.map fixedBitCircuit :=
      List.mem_map.mpr ⟨ib, by simpa using hib, rfl⟩
    exact (fixedBitCircuit_eval_true_iff ib v).mp (hAll (fixedBitCircuit ib) hmem)
  · intro hcov
    apply (andListCircuit_eval_true_iff (P.fixed.toList.map fixedBitCircuit) v).mpr
    intro C hC
    obtain ⟨ib, hib, hEq⟩ := List.mem_map.mp hC
    subst C
    exact (fixedBitCircuit_eval_true_iff ib v).mpr (hcov ib (by simpa using hib))

end Pattern

namespace DNF

/-- Canonical OR-of-ANDs circuit for a DNF. -/
noncomputable def toCircuit {n : Nat} (D : DNF n) : BoolCircuit n :=
  orListCircuit (D.toList.map Pattern.toCircuit)

theorem toCircuit_eval_true_iff {n : Nat} (D : DNF n) (v : Vertex n) :
    D.toCircuit.eval v = true ↔ ∃ P ∈ D, P.Covers v := by
  constructor
  · intro h
    obtain ⟨C, hC, hEval⟩ := (orListCircuit_eval_true_iff (D.toList.map Pattern.toCircuit) v).mp h
    obtain ⟨P, hP, hEq⟩ := List.mem_map.mp hC
    subst C
    exact ⟨P, by simpa using hP, (Pattern.toCircuit_eval_true_iff P v).mp hEval⟩
  · rintro ⟨P, hP, hcov⟩
    apply (orListCircuit_eval_true_iff (D.toList.map Pattern.toCircuit) v).mpr
    refine ⟨P.toCircuit, ?_, ?_⟩
    · exact List.mem_map.mpr ⟨P, by simpa using hP, rfl⟩
    · exact (Pattern.toCircuit_eval_true_iff P v).mpr hcov

theorem toCircuit_computes {n : Nat} (D : DNF n) :
    D.toCircuit.Computes D.eval := by
  intro v
  cases hc : D.toCircuit.eval v <;> cases hd : D.eval v
  · rfl
  · have hExists : ∃ P ∈ D, P.Covers v := (eval_true_iff D v).mp hd
    have : D.toCircuit.eval v = true := (toCircuit_eval_true_iff D v).mpr hExists
    rw [hc] at this
    contradiction
  · have hExists : ∃ P ∈ D, P.Covers v := (toCircuit_eval_true_iff D v).mp hc
    have : D.eval v = true := (eval_true_iff D v).mpr hExists
    rw [hd] at this
    contradiction
  · rfl

theorem card_le_toCircuit_size {n : Nat} (D : DNF n) :
    D.card ≤ D.toCircuit.size := by
  unfold toCircuit
  have h := orListCircuit_size_ge_length (D.toList.map Pattern.toCircuit)
  rw [List.length_map, Finset.length_toList] at h
  exact h

end DNF

/-- Vertices where a Boolean function is true. -/
def trueVertices {n : Nat} (f : Vertex n → Bool) : Finset (Vertex n) :=
  Finset.univ.filter fun v => f v = true

@[simp] theorem mem_trueVertices {n : Nat} {f : Vertex n → Bool} {v : Vertex n} :
    v ∈ trueVertices f ↔ f v = true := by
  simp [trueVertices]

/-- Vertices where a Boolean function is false. -/
def falseVertices {n : Nat} (f : Vertex n → Bool) : Finset (Vertex n) :=
  Finset.univ.filter fun v => f v = false

@[simp] theorem mem_falseVertices {n : Nat} {f : Vertex n → Bool} {v : Vertex n} :
    v ∈ falseVertices f ↔ f v = false := by
  simp [falseVertices]

/-- True and false vertices partition the whole hypercube. -/
theorem trueVertices_card_add_falseVertices_card {n : Nat} (f : Vertex n → Bool) :
    (trueVertices f).card + (falseVertices f).card = 2 ^ n := by
  classical
  have h := Finset.card_filter_add_card_filter_not
    (s := (Finset.univ : Finset (Vertex n))) (p := fun v => f v = true)
  have hfalse :
      (Finset.filter (fun v : Vertex n => ¬ f v = true) Finset.univ) =
        falseVertices f := by
    ext v
    cases hv : f v <;> simp [falseVertices, hv]
  rw [hfalse] at h
  simpa [trueVertices, allVertices_card, Vertex] using h

/-- The true vertices of `f` covered by one pattern. -/
noncomputable def Pattern.trueFootprint {n : Nat} (P : Pattern n) (f : Vertex n → Bool) :
    Finset (Vertex n) := by
  classical
  exact trueVertices f |>.filter fun v => P.Covers v

@[simp] theorem Pattern.mem_trueFootprint {n : Nat} (P : Pattern n)
    (f : Vertex n → Bool) {v : Vertex n} :
    v ∈ P.trueFootprint f ↔ f v = true ∧ P.Covers v := by
  classical
  simp [Pattern.trueFootprint]

/-! ### Information covers

An information cover is a solver/proof/circuit-agnostic certificate that the true set of a
Boolean function is covered by finitely many hypercube regions.  The key accounting principle
is purely information-theoretic: if each region carries at most `K` true vertices, then `s`
regions carry at most `s * K` true vertices. -/

/-- A finite family of hypercube regions covering all true vertices of `f`. This is the common
semantic shape behind DNFs, decision-tree accepting leaves, solver frontiers, and learned-region
partitions. -/
structure InformationCover {n : Nat} (f : Vertex n → Bool) where
  regions : Finset (Pattern n)
  coversTrue : ∀ v : Vertex n, f v = true → ∃ P ∈ regions, P.Covers v

namespace InformationCover

/-- Number of regions in an information cover. -/
def regionCount {n : Nat} {f : Vertex n → Bool} (C : InformationCover f) : Nat :=
  C.regions.card

/-- The true vertices carried by all regions of an information cover. -/
noncomputable def trueMassUnion {n : Nat} {f : Vertex n → Bool}
    (C : InformationCover f) : Finset (Vertex n) := by
  classical
  exact C.regions.biUnion fun P => P.trueFootprint f

theorem trueVertices_subset_trueMassUnion {n : Nat} {f : Vertex n → Bool}
    (C : InformationCover f) :
    trueVertices f ⊆ C.trueMassUnion := by
  intro v hv
  have hfv : f v = true := mem_trueVertices.mp hv
  obtain ⟨P, hP, hcov⟩ := C.coversTrue v hfv
  exact Finset.mem_biUnion.mpr
    ⟨P, hP, (P.mem_trueFootprint f).mpr ⟨hfv, hcov⟩⟩

/-- Information-capacity bound for arbitrary region covers. -/
theorem trueVertices_card_le_regionCount_mul_of_region_card_le {n K : Nat}
    {f : Vertex n → Bool} (C : InformationCover f)
    (hregion : ∀ P ∈ C.regions, (P.trueFootprint f).card ≤ K) :
    (trueVertices f).card ≤ C.regionCount * K := by
  have hcard : (trueVertices f).card ≤ C.trueMassUnion.card :=
    Finset.card_le_card C.trueVertices_subset_trueMassUnion
  have hunion : C.trueMassUnion.card ≤ ∑ P ∈ C.regions, (P.trueFootprint f).card := by
    unfold trueMassUnion
    exact Finset.card_biUnion_le
  have hsum : ∑ P ∈ C.regions, (P.trueFootprint f).card ≤ ∑ P ∈ C.regions, K := by
    exact Finset.sum_le_sum hregion
  have hK : ∑ P ∈ C.regions, K = C.regionCount * K := by
    simp [regionCount, Finset.sum_const, nsmul_eq_mul]
  exact hcard.trans (hunion.trans (hsum.trans (by rw [hK])))

/-- Strict lower-bound form: if `s` regions of capacity `K` cannot carry the true set, then
any such information cover needs more than `s` regions. -/
theorem regionCount_gt_of_trueVertices_gt_capacity {n s K : Nat}
    {f : Vertex n → Bool} (C : InformationCover f)
    (hregion : ∀ P ∈ C.regions, (P.trueFootprint f).card ≤ K)
    (hgt : s * K < (trueVertices f).card) :
    s < C.regionCount := by
  have hcap := C.trueVertices_card_le_regionCount_mul_of_region_card_le hregion
  by_contra hnot
  have hcount : C.regionCount ≤ s := by omega
  have hmul : C.regionCount * K ≤ s * K := Nat.mul_le_mul_right K hcount
  have hle : (trueVertices f).card ≤ s * K := hcap.trans hmul
  omega

end InformationCover

/-- A model/proof/solver object has an information cover of `f` with region count bounded by
an abstract cost. DNF cost is term count, decision-tree cost can be accepting leaves, and
solver cost can be frontier size. -/
structure HasInformationCoverAtCost {n : Nat} (f : Vertex n → Bool) (cost : Nat) where
  cover : InformationCover f
  regionCount_le_cost : cover.regionCount ≤ cost

namespace HasInformationCoverAtCost

/-- If a model exposes at most `cost` information regions and each region carries at most `K`
true vertices, then the true set has at most `cost * K` vertices. -/
theorem trueVertices_card_le_cost_mul_of_region_card_le {n cost K : Nat}
    {f : Vertex n → Bool} (H : HasInformationCoverAtCost f cost)
    (hregion : ∀ P ∈ H.cover.regions, (P.trueFootprint f).card ≤ K) :
    (trueVertices f).card ≤ cost * K := by
  have hcap := H.cover.trueVertices_card_le_regionCount_mul_of_region_card_le hregion
  exact hcap.trans (Nat.mul_le_mul_right K H.regionCount_le_cost)

/-- Infeasibility form of the information-capacity bound. -/
theorem false_of_trueVertices_gt_capacity {n cost K : Nat}
    {f : Vertex n → Bool} (H : HasInformationCoverAtCost f cost)
    (hregion : ∀ P ∈ H.cover.regions, (P.trueFootprint f).card ≤ K)
    (hgt : cost * K < (trueVertices f).card) :
    False := by
  have hle := H.trueVertices_card_le_cost_mul_of_region_card_le hregion
  omega

end HasInformationCoverAtCost

namespace DNF

/-- Every correct DNF is an information cover whose regions are its terms. -/
def toInformationCover {n : Nat} {D : DNF n} {f : Vertex n → Bool}
    (hcomp : D.Computes f) : InformationCover f where
  regions := D
  coversTrue := by
    intro v hfv
    have hEval : D.eval v = true := by
      rw [hcomp v]
      exact hfv
    exact DNF.eval_true_iff D v |>.mp hEval

@[simp] theorem toInformationCover_regionCount {n : Nat} {D : DNF n}
    {f : Vertex n → Bool} (hcomp : D.Computes f) :
    (DNF.toInformationCover (D := D) (f := f) hcomp).regionCount = D.card := by
  rfl

/-- DNF term count is a concrete model cost for information-cover accounting. -/
def hasInformationCoverAtTermCount {n : Nat} {D : DNF n} {f : Vertex n → Bool}
    (hcomp : D.Computes f) : HasInformationCoverAtCost f D.termCount where
  cover := D.toInformationCover hcomp
  regionCount_le_cost := by
    rfl

end DNF

/-- If a coordinate is not fixed by a pattern, flipping that coordinate preserves coverage. -/
theorem Pattern.covers_flip_of_not_mem_support {n : Nat} (P : Pattern n)
    {v : Vertex n} {i : Fin n} (hcov : P.Covers v) (hi : i ∉ P.support) :
    P.Covers (flipVertex v i) := by
  intro ib hib
  rcases ib with ⟨j, b⟩
  have hji : j ≠ i := by
    intro h
    subst j
    have hisupp : i ∈ P.support := (P.mem_support_iff i).mpr ⟨b, hib⟩
    exact hi hisupp
  simpa [flipVertex_ne v hji] using hcov (j, b) hib

/-- Edge-flipping functions change value along every hypercube edge. Parity is the canonical
example; the DNF lower-bound machinery only needs this edge property. -/
def EdgeFlipping {n : Nat} (f : Vertex n → Bool) : Prop :=
  ∀ v : Vertex n, ∀ i : Fin n, f (flipVertex v i) = !(f v)

/-- A nonempty all-true pattern for an edge-flipping function must fix every coordinate. -/
theorem Pattern.support_eq_univ_of_edgeFlipping_true {n : Nat} {P : Pattern n}
    {f : Vertex n → Bool} (hflip : EdgeFlipping f)
    (hnonempty : ∃ v : Vertex n, P.Covers v)
    (htrue : ∀ v : Vertex n, P.Covers v → f v = true) :
    P.support = Finset.univ := by
  apply Finset.eq_univ_iff_forall.mpr
  intro i
  by_contra hi
  obtain ⟨v, hv⟩ := hnonempty
  have hvflip : P.Covers (flipVertex v i) := P.covers_flip_of_not_mem_support hv hi
  have hvtrue : f v = true := htrue v hv
  have hfliptrue : f (flipVertex v i) = true := htrue (flipVertex v i) hvflip
  rw [hflip v i, hvtrue] at hfliptrue
  simp at hfliptrue

/-- A full-support pattern covers at most one vertex. -/
theorem Pattern.eq_of_full_support_covers {n : Nat} {P : Pattern n}
    (hfull : P.support = Finset.univ) {v w : Vertex n}
    (hv : P.Covers v) (hw : P.Covers w) :
    v = w := by
  funext i
  have hi : i ∈ P.support := by simp [hfull]
  obtain ⟨b, hib⟩ := (P.mem_support_iff i).mp hi
  have hvi : v i = b := hv (i, b) hib
  have hwi : w i = b := hw (i, b) hib
  exact hvi.trans hwi.symm

/-- If one DNF term is sound for an edge-flipping function, it contributes at most one true
vertex. This is the local geometric reason parity-like functions force many DNF terms. -/
theorem Pattern.trueFootprint_card_le_one_of_edgeFlipping {n : Nat}
    {P : Pattern n} {f : Vertex n → Bool} (hflip : EdgeFlipping f)
    (htrue : ∀ v : Vertex n, P.Covers v → f v = true) :
    (P.trueFootprint f).card ≤ 1 := by
  rw [Finset.card_le_one]
  intro v hv w hw
  have hvcov : P.Covers v := (P.mem_trueFootprint f).mp hv |>.2
  have hwcov : P.Covers w := (P.mem_trueFootprint f).mp hw |>.2
  have hnonempty : ∃ u : Vertex n, P.Covers u := ⟨v, hvcov⟩
  have hfull := P.support_eq_univ_of_edgeFlipping_true hflip hnonempty htrue
  exact P.eq_of_full_support_covers hfull hvcov hwcov

/-- General DNF counting lower bound: if every term contributes at most one true vertex,
then the DNF needs at least as many terms as the function has true vertices. -/
theorem DNF.trueVertices_card_le_terms_of_term_card_le_one {n : Nat}
    {D : DNF n} {f : Vertex n → Bool}
    (hcomp : D.Computes f)
    (hterm : ∀ P ∈ D, (P.trueFootprint f).card ≤ 1) :
    (trueVertices f).card ≤ D.card := by
  have hsub : trueVertices f ⊆ D.biUnion (fun P => P.trueFootprint f) := by
    intro v hv
    have hfv : f v = true := mem_trueVertices.mp hv
    have hEval : D.eval v = true := by
      rw [hcomp v]
      exact hfv
    obtain ⟨P, hP, hcov⟩ := (DNF.eval_true_iff D v).mp hEval
    exact Finset.mem_biUnion.mpr
      ⟨P, hP, (P.mem_trueFootprint f).mpr ⟨hfv, hcov⟩⟩
  have hcard : (trueVertices f).card ≤ (D.biUnion (fun P => P.trueFootprint f)).card :=
    Finset.card_le_card hsub
  have hunion : (D.biUnion (fun P => P.trueFootprint f)).card ≤
      ∑ P ∈ D, (P.trueFootprint f).card :=
    Finset.card_biUnion_le
  have hsum : ∑ P ∈ D, (P.trueFootprint f).card ≤ ∑ P ∈ D, 1 := by
    exact Finset.sum_le_sum hterm
  have hone : ∑ P ∈ D, (1 : Nat) = D.card := by
    rw [Finset.sum_const, nsmul_eq_mul]
    simp
  exact hcard.trans (hunion.trans (hsum.trans (by rw [hone])))

/-- General DNF counting lower bound: if every term contributes at most `K` true vertices,
then the true set has size at most `K` times the number of terms. This is the finite
normal-form testing rule used by the Python search lab. -/
theorem DNF.trueVertices_card_le_terms_mul_of_term_card_le {n K : Nat}
    {D : DNF n} {f : Vertex n → Bool}
    (hcomp : D.Computes f)
    (hterm : ∀ P ∈ D, (P.trueFootprint f).card ≤ K) :
    (trueVertices f).card ≤ D.card * K := by
  have hsub : trueVertices f ⊆ D.biUnion (fun P => P.trueFootprint f) := by
    intro v hv
    have hfv : f v = true := mem_trueVertices.mp hv
    have hEval : D.eval v = true := by
      rw [hcomp v]
      exact hfv
    obtain ⟨P, hP, hcov⟩ := (DNF.eval_true_iff D v).mp hEval
    exact Finset.mem_biUnion.mpr
      ⟨P, hP, (P.mem_trueFootprint f).mpr ⟨hfv, hcov⟩⟩
  have hcard : (trueVertices f).card ≤ (D.biUnion (fun P => P.trueFootprint f)).card :=
    Finset.card_le_card hsub
  have hunion : (D.biUnion (fun P => P.trueFootprint f)).card ≤
      ∑ P ∈ D, (P.trueFootprint f).card :=
    Finset.card_biUnion_le
  have hsum : ∑ P ∈ D, (P.trueFootprint f).card ≤ ∑ P ∈ D, K := by
    exact Finset.sum_le_sum hterm
  have hK : ∑ P ∈ D, K = D.card * K := by
    simp [Finset.sum_const, nsmul_eq_mul]
  exact hcard.trans (hunion.trans (hsum.trans (by rw [hK])))

/-- Information-capacity contrapositive for DNFs: if `s` terms of capacity `K` cannot cover
the true set, then any correct DNF whose terms each have capacity `K` must use more than `s`
terms. This is the normal-form lower-bound rule used to turn finite tests into proofs. -/
theorem DNF.termCount_gt_of_trueVertices_gt_terms_mul_capacity {n s K : Nat}
    {D : DNF n} {f : Vertex n → Bool}
    (hcomp : D.Computes f)
    (hterm : ∀ P ∈ D, (P.trueFootprint f).card ≤ K)
    (hgt : s * K < (trueVertices f).card) :
    s < D.termCount := by
  have hcap := DNF.trueVertices_card_le_terms_mul_of_term_card_le hcomp hterm
  unfold DNF.termCount
  by_contra hnot
  have hcard : D.card ≤ s := by omega
  have hmul : D.card * K ≤ s * K := Nat.mul_le_mul_right K hcard
  have hle : (trueVertices f).card ≤ s * K := hcap.trans hmul
  omega

/-- Family-level DNF information-capacity lower bound. If every correct DNF has term capacity
at most `K`, and `s * K` is smaller than the true set, then no correct DNF has at most `s`
terms. -/
theorem DNF.requiresTermCountMoreThan_of_trueVertices_gt_capacity {n s K : Nat}
    {f : Vertex n → Bool}
    (hterm : ∀ D : DNF n, D.Computes f →
      ∀ P ∈ D, (P.trueFootprint f).card ≤ K)
    (hgt : s * K < (trueVertices f).card) :
    DNF.RequiresTermCountMoreThan f s := by
  intro D hcomp
  exact DNF.termCount_gt_of_trueVertices_gt_terms_mul_capacity hcomp (hterm D hcomp) hgt

/-- Edge-flipping functions have DNF lower bounds equal to the size of their true set. -/
theorem DNF.trueVertices_card_le_terms_of_edgeFlipping {n : Nat}
    {D : DNF n} {f : Vertex n → Bool}
    (hflip : EdgeFlipping f) (hcomp : D.Computes f) :
    (trueVertices f).card ≤ D.card := by
  apply DNF.trueVertices_card_le_terms_of_term_card_le_one hcomp
  intro P hP
  apply Pattern.trueFootprint_card_le_one_of_edgeFlipping hflip
  intro v hcov
  have hEval : D.eval v = true := (DNF.eval_true_iff D v).mpr ⟨P, hP, hcov⟩
  rw [hcomp v] at hEval
  exact hEval

/-- Flipping coordinate `i` toggles membership of `i` in the flipped-coordinate set. -/
theorem flippedSet_flipVertex {n : Nat} (v : Vertex n) (i : Fin n) :
    flippedSet (flipVertex v i) =
      if v i = true then (flippedSet v).erase i else insert i (flippedSet v) := by
  ext j
  by_cases hji : j = i
  · subst j
    cases hvi : v i <;> simp [flippedSet, flipVertex, hvi]
  · cases hvi : v i <;> simp [flippedSet, flipVertex_ne v hji, hji, hvi]

/-- Flipping the same coordinate twice returns to the original vertex. -/
theorem flipVertex_flipVertex {n : Nat} (v : Vertex n) (i : Fin n) :
    flipVertex (flipVertex v i) i = v := by
  funext j
  by_cases hji : j = i
  · subst j
    simp [flipVertex]
  · simp [flipVertex_ne _ hji, flipVertex_ne v hji]

/-- Parity as a hypercube function, expressed by odd Hamming weight. -/
def parity {n : Nat} (v : Vertex n) : Bool :=
  decide (Odd (flippedSet v).card)

/-- Oddness toggles when adding one. -/
private theorem decide_odd_succ_eq_not_decide_odd (k : Nat) :
    decide (Odd (k + 1)) = !(decide (Odd k)) := by
  by_cases hk : Odd k
  · have hnot : ¬ Odd (k + 1) := by
      intro hsucc
      exact (Nat.odd_add_one.mp hsucc) hk
    simp [hk, hnot]
  · have hsucc : Odd (k + 1) := Nat.odd_add_one.mpr hk
    simp [hk, hsucc]

/-- Parity changes across every hypercube edge. -/
theorem parity_edgeFlipping {n : Nat} :
    EdgeFlipping (parity : Vertex n → Bool) := by
  intro v i
  unfold parity
  rw [flippedSet_flipVertex v i]
  by_cases hi : v i = true
  · rw [if_pos hi]
    have himem : i ∈ flippedSet v := (mem_flippedSet v i).mpr hi
    have hcard := Finset.card_erase_add_one himem
    have htoggle := decide_odd_succ_eq_not_decide_odd ((flippedSet v).erase i).card
    rw [hcard] at htoggle
    rw [htoggle]
    cases decide (Odd ((flippedSet v).erase i).card) <;> rfl
  · rw [if_neg hi]
    have hinot : i ∉ flippedSet v := by
      intro himem
      exact hi ((mem_flippedSet v i).mp himem)
    rw [Finset.card_insert_of_notMem hinot]
    exact decide_odd_succ_eq_not_decide_odd (flippedSet v).card

/-- For an edge-flipping function, flipping any fixed coordinate bijects true vertices with
false vertices. -/
theorem trueVertices_card_eq_falseVertices_card_of_edgeFlipping {n : Nat}
    {f : Vertex n → Bool} (hflip : EdgeFlipping f) (i : Fin n) :
    (trueVertices f).card = (falseVertices f).card := by
  classical
  apply Finset.card_bijective (fun v => flipVertex v i)
  · constructor
    · intro v w h
      have hcongr := congrArg (fun u => flipVertex u i) h
      simpa [flipVertex_flipVertex] using hcongr
    · intro w
      exact ⟨flipVertex w i, by simp [flipVertex_flipVertex]⟩
  · intro v
    constructor
    · intro hv
      have htrue : f v = true := mem_trueVertices.mp hv
      have htoggle := hflip v i
      rw [htrue] at htoggle
      exact mem_falseVertices.mpr htoggle
    · intro hv
      have hfalse : f (flipVertex v i) = false := mem_falseVertices.mp hv
      have htoggle := hflip v i
      cases hfv : f v
      · rw [hfv] at htoggle
        rw [htoggle] at hfalse
        simp at hfalse
      · exact mem_trueVertices.mpr hfv

/-- The exact true-count statement for parity, isolated as the remaining counting target.
The DNF lower-bound theorem below shows how this count plugs into an exponential result. -/
def ParityTrueCount (n : Nat) : Prop :=
  (trueVertices (parity : Vertex n → Bool)).card = 2 ^ (n - 1)

/-- Successor-dimension spelling of the parity true-count fact, avoiding subtraction. -/
def ParityTrueCountSucc (n : Nat) : Prop :=
  (trueVertices (parity : Vertex (n + 1) → Bool)).card = 2 ^ n

/-- Parity is true on exactly half of the `(n+1)`-dimensional cube. -/
theorem parity_true_count_succ (n : Nat) :
    ParityTrueCountSucc n := by
  let i : Fin (n + 1) := ⟨0, Nat.succ_pos n⟩
  have heq := trueVertices_card_eq_falseVertices_card_of_edgeFlipping
    (f := (parity : Vertex (n + 1) → Bool)) parity_edgeFlipping i
  have hsum := trueVertices_card_add_falseVertices_card
    (parity : Vertex (n + 1) → Bool)
  unfold ParityTrueCountSucc
  rw [← heq] at hsum
  rw [Nat.pow_succ] at hsum
  omega

/-- Once parity is known to be edge-flipping and balanced, every DNF for parity has
exponentially many terms. The edge-flip and count lemmas are deliberately separated so each
can be attacked independently. -/
theorem parity_dnf_lower_bound_of_edgeFlip_and_count {n : Nat} (D : DNF n)
    (hflip : EdgeFlipping (parity : Vertex n → Bool))
    (hcount : ParityTrueCount n)
    (hD : D.Computes (parity : Vertex n → Bool)) :
    2 ^ (n - 1) ≤ D.card := by
  rw [← hcount]
  exact DNF.trueVertices_card_le_terms_of_edgeFlipping hflip hD

/-- Parity DNF lower bound, reduced only to the standard true-count fact. -/
theorem parity_dnf_lower_bound_of_true_count {n : Nat} (D : DNF n)
    (hcount : ParityTrueCount n)
    (hD : D.Computes (parity : Vertex n → Bool)) :
    2 ^ (n - 1) ≤ D.card :=
  parity_dnf_lower_bound_of_edgeFlip_and_count D parity_edgeFlipping hcount hD

/-- Clean exponential form: in dimension `n+1`, parity DNFs need at least `2^n` terms,
assuming the standard half-cube true-count fact. -/
theorem parity_dnf_lower_bound_succ_of_true_count {n : Nat} (D : DNF (n + 1))
    (hcount : ParityTrueCountSucc n)
    (hD : D.Computes (parity : Vertex (n + 1) → Bool)) :
    2 ^ n ≤ D.card := by
  rw [← hcount]
  exact DNF.trueVertices_card_le_terms_of_edgeFlipping parity_edgeFlipping hD

/-- Unconditional exponential lower bound: parity in dimension `n+1` needs at least `2^n`
DNF terms. -/
theorem parity_dnf_lower_bound_succ {n : Nat} (D : DNF (n + 1))
    (hD : D.Computes (parity : Vertex (n + 1) → Bool)) :
    2 ^ n ≤ D.card :=
  parity_dnf_lower_bound_succ_of_true_count D (parity_true_count_succ n) hD

/-- Lower-bound-predicate form: no DNF with fewer than `2^n` terms computes parity in
dimension `n+1`. -/
theorem parity_dnf_requires_more_than_pred {n : Nat} :
    DNF.RequiresTermCountMoreThan (parity : Vertex (n + 1) → Bool) (2 ^ n - 1) := by
  intro D hD
  have hlb := parity_dnf_lower_bound_succ D hD
  have hpos : 0 < 2 ^ n := Nat.two_pow_pos n
  unfold DNF.termCount
  omega

/-- The canonical DNF circuit for any DNF computing parity has exponential size. This is
the first concrete circuit lower bound in the file: it transfers the geometric DNF
term-count lower bound into a `BoolCircuit.size` lower bound. -/
theorem parity_DNF_toCircuit_size_ge {n : Nat} (D : DNF (n + 1))
    (hD : D.Computes (parity : Vertex (n + 1) → Bool)) :
    2 ^ n ≤ D.toCircuit.size := by
  exact (parity_dnf_lower_bound_succ D hD).trans (DNF.card_le_toCircuit_size D)

/-- Correctness bridge: if a DNF computes parity, its canonical circuit computes the same
Boolean function on the hypercube bits. -/
theorem parity_DNF_toCircuit_computes {n : Nat} (D : DNF (n + 1))
    (hD : D.Computes (parity : Vertex (n + 1) → Bool)) :
    D.toCircuit.Computes (parity : BoolFunc (n + 1)) := by
  intro v
  rw [DNF.toCircuit_computes D v, hD v]

namespace BoolCircuit

/-- Restricted circuit class: circuits obtained by the canonical DNF-to-circuit translation
from some hypercube-pattern DNF computing `f`. This is a depth-2 OR-of-ANDs model. -/
noncomputable def IsCanonicalDNFFor {n : Nat} (C : BoolCircuit n)
    (f : BoolFunc n) : Prop :=
  ∃ D : DNF n, D.Computes f ∧ C = D.toCircuit

end BoolCircuit

/-- General transfer principle: a DNF term-count lower bound gives a size lower bound for
the corresponding canonical DNF circuit class. -/
theorem canonicalDNFCircuit_size_ge_of_dnf_lower_bound {n L : Nat}
    {f : BoolFunc n} {C : BoolCircuit n}
    (hC : C.IsCanonicalDNFFor f)
    (hLower : ∀ D : DNF n, D.Computes f → L ≤ D.card) :
    L ≤ C.size := by
  rcases hC with ⟨D, hD, rfl⟩
  exact (hLower D hD).trans (DNF.card_le_toCircuit_size D)

/-- Circuit-class form of the parity lower bound: every canonical DNF circuit computing
parity in dimension `n+1` has size at least `2^n`. -/
theorem parity_canonicalDNFCircuit_size_ge {n : Nat} {C : BoolCircuit (n + 1)}
    (hC : C.IsCanonicalDNFFor (parity : BoolFunc (n + 1))) :
    2 ^ n ≤ C.size :=
  canonicalDNFCircuit_size_ge_of_dnf_lower_bound hC
    (fun D hD => parity_dnf_lower_bound_succ D hD)

/-! ### Normal-form circuit lower bounds -/

/-- The normal-form circuit model for this project: circuits arising from canonical
hypercube-pattern DNFs. We keep this as a named alias so lower-bound work can stay focused
on normal forms rather than unrestricted circuits. -/
noncomputable def NormalFormCircuitFor {n : Nat} (C : BoolCircuit n)
    (f : BoolFunc n) : Prop :=
  C.IsCanonicalDNFFor f

/-- Finite normal-form lower-bound statement: every canonical pattern-DNF circuit computing
`f` has size at least `s`. -/
noncomputable def NormalFormRequiresSizeAtLeast {n : Nat} (f : BoolFunc n)
    (s : Nat) : Prop :=
  ∀ C : BoolCircuit n, NormalFormCircuitFor C f → s ≤ C.size

/-- Family-level normal-form exponential lower-bound target. This is the restricted,
normal-form version of the circuit program, not an unrestricted circuit claim. -/
noncomputable def NormalFormExponentialCircuitLowerBound
    (L : (N : Nat) → BoolFunc N) : Prop :=
  ∃ N₀ : Nat, ∀ N, N₀ ≤ N →
    NormalFormRequiresSizeAtLeast (L N) (2 ^ N)

/-- Normal-form lower bounds follow from DNF term-count lower bounds. -/
theorem normalForm_size_ge_of_dnf_lower_bound {n L : Nat} {f : BoolFunc n}
    (hLower : ∀ D : DNF n, D.Computes f → L ≤ D.card) :
    NormalFormRequiresSizeAtLeast f L := by
  intro C hC
  exact canonicalDNFCircuit_size_ge_of_dnf_lower_bound hC hLower

/-- Normal-form circuit counting rule: if every term in every correct DNF contributes at
most `K` true vertices, then any corresponding normal-form circuit has enough size for
`trueVertices ≤ size * K`. -/
theorem normalForm_trueVertices_card_le_size_mul_of_term_card_le {n K : Nat}
    {f : BoolFunc n} {C : BoolCircuit n}
    (hC : NormalFormCircuitFor C f)
    (hterm : ∀ D : DNF n, D.Computes f →
      ∀ P ∈ D, (P.trueFootprint f).card ≤ K) :
    (trueVertices f).card ≤ C.size * K := by
  rcases hC with ⟨D, hD, rfl⟩
  have hdnf := DNF.trueVertices_card_le_terms_mul_of_term_card_le hD (hterm D hD)
  exact hdnf.trans (Nat.mul_le_mul_right K (DNF.card_le_toCircuit_size D))

/-- Normal-form information-capacity lower bound in strict form. If `s` circuit-size units,
each carrying at most `K` true vertices, cannot cover the true set, then every correct
normal-form circuit must have size greater than `s`. -/
theorem normalForm_size_gt_of_trueVertices_gt_size_mul_capacity {n s K : Nat}
    {f : BoolFunc n} {C : BoolCircuit n}
    (hC : NormalFormCircuitFor C f)
    (hterm : ∀ D : DNF n, D.Computes f →
      ∀ P ∈ D, (P.trueFootprint f).card ≤ K)
    (hgt : s * K < (trueVertices f).card) :
    s < C.size := by
  have hcap := normalForm_trueVertices_card_le_size_mul_of_term_card_le hC hterm
  by_contra hnot
  have hsize : C.size ≤ s := by omega
  have hmul : C.size * K ≤ s * K := Nat.mul_le_mul_right K hsize
  have hle : (trueVertices f).card ≤ s * K := hcap.trans hmul
  omega

/-- Lower-bound predicate form of the normal-form information-capacity rule. -/
theorem normalForm_requiresSizeMoreThan_of_trueVertices_gt_capacity {n s K : Nat}
    {f : BoolFunc n}
    (hterm : ∀ D : DNF n, D.Computes f →
      ∀ P ∈ D, (P.trueFootprint f).card ≤ K)
    (hgt : s * K < (trueVertices f).card) :
    ∀ C : BoolCircuit n, NormalFormCircuitFor C f → s < C.size := by
  intro C hC
  exact normalForm_size_gt_of_trueVertices_gt_size_mul_capacity hC hterm hgt

/-- Edge-flipping functions force each normal-form term to contribute at most one true
vertex, so every normal-form circuit has size at least the number of true vertices. -/
theorem normalForm_size_ge_trueVertices_card_of_edgeFlipping {n : Nat}
    {f : BoolFunc n} {C : BoolCircuit n}
    (hflip : EdgeFlipping f) (hC : NormalFormCircuitFor C f) :
    (trueVertices f).card ≤ C.size := by
  have h := normalForm_trueVertices_card_le_size_mul_of_term_card_le
    (K := 1) hC
    (fun D hD P hP =>
      Pattern.trueFootprint_card_le_one_of_edgeFlipping hflip
        (by
          intro v hcov
          have hEval : D.eval v = true := (DNF.eval_true_iff D v).mpr ⟨P, hP, hcov⟩
          rw [hD v] at hEval
          exact hEval))
  simpa using h

/-- Parity is exponentially hard for the project's normal-form circuit model. -/
theorem parity_normalFormCircuit_size_ge {n : Nat} :
    NormalFormRequiresSizeAtLeast (parity : BoolFunc (n + 1)) (2 ^ n) :=
  normalForm_size_ge_of_dnf_lower_bound
    (fun D hD => parity_dnf_lower_bound_succ D hD)

/-! ### Stress tests for unrestricted circuits and normal-form loss

The normal-form lower bounds above are honest restricted lower bounds: they apply to
canonical hypercube-pattern DNFs.  To push beyond that, we need an invariant that survives
general gates and circuit sharing.  The next definitions are the accounting contracts for that
question.  They do not assert such an invariant exists; they make the missing obligation
precise. -/

/-- A natural-valued circuit invariant whose value can be charged locally to general Boolean
gates.  If a lower-bound argument can produce such a measure and prove that every circuit
computing `f` has large measure, then the lower bound transfers to unrestricted circuits.

This is the "survives arbitrary gates" test: input/constant leaves cost at most one unit, and
each Boolean gate increases the invariant by at most one plus its children. -/
structure GateStableCircuitInvariant (N : Nat) where
  measure : BoolCircuit N → Nat
  input_le : ∀ i : Fin N, measure (BoolCircuit.input i) ≤ 1
  const_le : ∀ b : Bool, measure (BoolCircuit.const b) ≤ 1
  not_le : ∀ C : BoolCircuit N, measure (BoolCircuit.not C) ≤ measure C + 1
  and_le : ∀ A B : BoolCircuit N,
    measure (BoolCircuit.and A B) ≤ measure A + measure B + 1
  or_le : ∀ A B : BoolCircuit N,
    measure (BoolCircuit.or A B) ≤ measure A + measure B + 1

namespace GateStableCircuitInvariant

/-- Gate-stable invariants are upper-bounded by twice the circuit size plus one. This is the formal
reason such an invariant can support general circuit lower bounds. -/
theorem measure_le_two_mul_size_add_one {N : Nat} (I : GateStableCircuitInvariant N)
    (C : BoolCircuit N) :
    I.measure C ≤ 2 * C.size + 1 := by
  induction C with
  | input i =>
      simpa [BoolCircuit.size] using I.input_le i
  | const b =>
      simpa [BoolCircuit.size] using I.const_le b
  | not C ih =>
      have h := I.not_le C
      calc
        I.measure (BoolCircuit.not C) ≤ I.measure C + 1 := h
        _ ≤ (2 * C.size + 1) + 1 := Nat.add_le_add_right ih 1
        _ ≤ 2 * (BoolCircuit.not C).size + 1 := by
          simp [BoolCircuit.size]
  | and A B ihA ihB =>
      have h := I.and_le A B
      calc
        I.measure (BoolCircuit.and A B) ≤ I.measure A + I.measure B + 1 := h
        _ ≤ (2 * A.size + 1) + (2 * B.size + 1) + 1 := by
          exact Nat.add_le_add_right (Nat.add_le_add ihA ihB) 1
        _ ≤ 2 * (BoolCircuit.and A B).size + 1 := by
          simp [BoolCircuit.size]
          omega
  | or A B ihA ihB =>
      have h := I.or_le A B
      calc
        I.measure (BoolCircuit.or A B) ≤ I.measure A + I.measure B + 1 := h
        _ ≤ (2 * A.size + 1) + (2 * B.size + 1) + 1 := by
          exact Nat.add_le_add_right (Nat.add_le_add ihA ihB) 1
        _ ≤ 2 * (BoolCircuit.or A B).size + 1 := by
          simp [BoolCircuit.size]
          omega

/-- A function forces measure `L` for an invariant if every circuit computing the function has
invariant at least `L`. -/
def ForcesMeasureAtLeast {N : Nat} (I : GateStableCircuitInvariant N)
    (f : BoolFunc N) (L : Nat) : Prop :=
  ∀ C : BoolCircuit N, C.Computes f → L ≤ I.measure C

/-- Transfer rule from a gate-stable invariant lower bound to a concrete circuit-size lower
bound. The `2 * size + 1` slack accounts for leaves plus gates in our tree syntax. -/
theorem two_mul_size_add_one_ge_of_forcesMeasureAtLeast {N L : Nat}
    {I : GateStableCircuitInvariant N} {f : BoolFunc N}
    (hforce : I.ForcesMeasureAtLeast f L) :
    ∀ C : BoolCircuit N, C.Computes f → L ≤ 2 * C.size + 1 := by
  intro C hC
  exact (hforce C hC).trans (I.measure_le_two_mul_size_add_one C)

end GateStableCircuitInvariant

/-- A normal-form compiler packages a translation from unrestricted circuits to a restricted
normal form, together with its semantic correctness and size blowup.  This is the exact place
where a DNF-style lower bound can lose generality. -/
structure NormalFormCompiler where
  normal : (N : Nat) → Type
  normalSize : {N : Nat} → normal N → Nat
  normalEval : {N : Nat} → normal N → BoolFunc N
  compile : {N : Nat} → BoolCircuit N → normal N
  blowup : Nat → Nat
  sound : ∀ {N : Nat} (C : BoolCircuit N) (x : Fin N → Bool),
    normalEval (compile C) x = C.eval x
  size_le : ∀ {N : Nat} (C : BoolCircuit N),
    normalSize (compile C) ≤ blowup C.size

namespace NormalFormCompiler

/-- The compiler has polynomial blowup when its blowup function is polynomially bounded. -/
def PolynomialBlowup (T : NormalFormCompiler) : Prop :=
  _root_.HypercubeSAT.PolynomialO T.blowup

/-- A lower bound inside the compiler's target normal form. -/
def RequiresNormalSizeAtLeast (T : NormalFormCompiler) {N : Nat}
    (f : BoolFunc N) (L : Nat) : Prop :=
  ∀ A : T.normal N, T.normalEval A = f → L ≤ T.normalSize A

/-- The compiler is exponentially lossy at scale `loss` when some circuit of size `N` may
compile to a normal form of size at least `loss N`.  This is a diagnostic, not a theorem:
parity's DNF blowup is the motivating example. -/
def HasLossWitness (T : NormalFormCompiler) (loss : Nat → Nat) : Prop :=
  ∀ N : Nat, ∃ C : BoolCircuit N, loss C.size ≤ T.normalSize (T.compile C)

/-- Soundness restated extensionally: the compiled normal form computes the same function as
the source circuit. -/
theorem compile_computes_eval (T : NormalFormCompiler) {N : Nat} (C : BoolCircuit N) :
    T.normalEval (T.compile C) = C.eval := by
  funext x
  exact T.sound C x

/-- Exact transfer rule: a normal-form lower bound only becomes an unrestricted circuit
lower-bound statement after paying the compiler's blowup. This answers where normal-form
arguments can lose generality. -/
theorem lower_bound_transfers_through_blowup (T : NormalFormCompiler) {N L : Nat}
    {f : BoolFunc N} (hlower : T.RequiresNormalSizeAtLeast f L)
    {C : BoolCircuit N} (hC : C.Computes f) :
    L ≤ T.blowup C.size := by
  have hcompiled : T.normalEval (T.compile C) = f := by
    funext x
    rw [T.sound C x, hC x]
  exact (hlower (T.compile C) hcompiled).trans (T.size_le C)

/-- Contrapositive compiler test: if a circuit is so small that even its compiled normal form
would be below the normal-form lower bound, then it cannot compute the function. -/
theorem not_computes_of_blowup_lt_lower_bound (T : NormalFormCompiler) {N L : Nat}
    {f : BoolFunc N} (hlower : T.RequiresNormalSizeAtLeast f L)
    {C : BoolCircuit N} (hsmall : T.blowup C.size < L) :
    ¬ C.Computes f := by
  intro hC
  have hle := T.lower_bound_transfers_through_blowup hlower hC
  omega

end NormalFormCompiler

/-- Linear functions over `GF(2)`, represented by the set of variables XORed together.
This is the tiny algebraic model that explains why parity is easy once XOR-like reasoning is in
the language, even though it is hard for DNF/normal-form region covers. -/
structure XorLinearForm (N : Nat) where
  support : Finset (Fin N)

namespace XorLinearForm

/-- Evaluate a linear XOR form: true iff an odd number of supported variables are true. -/
def eval {N : Nat} (F : XorLinearForm N) (x : Fin N → Bool) : Bool :=
  decide (Odd (F.support.filter fun i => x i = true).card)

/-- Size of a linear XOR form, counted as the number of variables participating. -/
def size {N : Nat} (F : XorLinearForm N) : Nat :=
  F.support.card

/-- A linear XOR form computes a Boolean function. -/
def Computes {N : Nat} (F : XorLinearForm N) (f : BoolFunc N) : Prop :=
  ∀ x, F.eval x = f x

/-- The XOR form for parity: XOR all coordinates. -/
def parityForm (N : Nat) : XorLinearForm N where
  support := Finset.univ

@[simp] theorem parityForm_size (N : Nat) :
    (parityForm N).size = N := by
  simp [parityForm, size]

/-- The all-coordinate XOR form computes hypercube parity exactly. -/
theorem parityForm_computes (N : Nat) :
    (parityForm N).Computes (parity : BoolFunc N) := by
  intro x
  simp [Computes, eval, parityForm, parity, flippedSet]

/-- The parity XOR-form family has linear, hence polynomial, size. -/
theorem parityForm_size_polynomial :
    _root_.HypercubeSAT.PolynomialO (fun N => (parityForm N).size) := by
  refine ⟨1, 1, 0, ?_⟩
  intro N _
  simp [parityForm_size]

end XorLinearForm

/-- Compiler from XOR-linear forms into an arbitrary normal-form language.  This is the right
object for testing whether a blocker/normal-form representation can absorb algebraic sharing. -/
structure XorToNormalCompiler where
  normal : (N : Nat) → Type
  normalSize : {N : Nat} → normal N → Nat
  normalEval : {N : Nat} → normal N → BoolFunc N
  compile : {N : Nat} → XorLinearForm N → normal N
  blowup : Nat → Nat
  sound : ∀ {N : Nat} (F : XorLinearForm N) (x : Fin N → Bool),
    normalEval (compile F) x = F.eval x
  size_le : ∀ {N : Nat} (F : XorLinearForm N),
    normalSize (compile F) ≤ blowup F.size

namespace XorToNormalCompiler

/-- A lower bound in the compiler's target normal form. -/
def RequiresNormalSizeAtLeast (T : XorToNormalCompiler) {N : Nat}
    (f : BoolFunc N) (L : Nat) : Prop :=
  ∀ A : T.normal N, T.normalEval A = f → L ≤ T.normalSize A

/-- The compiled XOR-linear parity form computes parity in the normal target. -/
theorem compiled_parity_computes (T : XorToNormalCompiler) (N : Nat) :
    T.normalEval (T.compile (XorLinearForm.parityForm N)) = (parity : BoolFunc N) := by
  funext x
  rw [T.sound, XorLinearForm.parityForm_computes]

/-- Parity forces exponential normal-form size for any semantics-preserving compiler from
XOR-linear forms into a target where parity has the normal-form lower bound. -/
theorem parity_forces_compiled_normal_size_succ (T : XorToNormalCompiler) {n : Nat}
    (hlower : T.RequiresNormalSizeAtLeast (parity : BoolFunc (n + 1)) (2 ^ n)) :
    2 ^ n ≤ T.normalSize (T.compile (XorLinearForm.parityForm (n + 1))) :=
  hlower (T.compile (XorLinearForm.parityForm (n + 1)))
    (T.compiled_parity_computes (n + 1))

/-- The same fact stated as a blowup lower bound: the parity source has size `n+1`, but any
normal-form image must pay at least `2^n` in the target. -/
theorem parity_forces_blowup_succ (T : XorToNormalCompiler) {n : Nat}
    (hlower : T.RequiresNormalSizeAtLeast (parity : BoolFunc (n + 1)) (2 ^ n)) :
    2 ^ n ≤ T.blowup (n + 1) := by
  have hcompiled := T.parity_forces_compiled_normal_size_succ hlower
  have hsize := T.size_le (XorLinearForm.parityForm (n + 1))
  simpa [XorLinearForm.parityForm_size] using hcompiled.trans hsize

/-- No compiler with too-small parity blowup can be semantics preserving into such a normal
form. This is the constructive refutation of polynomial-looking normal-form compilation at a
fixed dimension. -/
theorem false_of_parity_blowup_lt (T : XorToNormalCompiler) {n : Nat}
    (hlower : T.RequiresNormalSizeAtLeast (parity : BoolFunc (n + 1)) (2 ^ n))
    (hsmall : T.blowup (n + 1) < 2 ^ n) :
    False := by
  have hge := T.parity_forces_blowup_succ hlower
  omega

end XorToNormalCompiler

/-- Compiler from XOR-linear forms into hypercube-pattern DNFs specifically. -/
structure XorToDNFCompiler where
  compile : {N : Nat} → XorLinearForm N → DNF N
  blowup : Nat → Nat
  sound : ∀ {N : Nat} (F : XorLinearForm N), (compile F).Computes F.eval
  size_le : ∀ {N : Nat} (F : XorLinearForm N), (compile F).card ≤ blowup F.size

namespace XorToDNFCompiler

/-- Any semantics-preserving XOR-to-DNF compiler sends parity to a DNF with exponentially many
terms. This is the concrete "parity kills blocker-normal compilation" theorem. -/
theorem parity_compiled_terms_ge_succ (T : XorToDNFCompiler) {n : Nat} :
    2 ^ n ≤ (T.compile (XorLinearForm.parityForm (n + 1))).card := by
  apply parity_dnf_lower_bound_succ
  intro x
  rw [T.sound (XorLinearForm.parityForm (n + 1)) x,
    XorLinearForm.parityForm_computes]

/-- Therefore the compiler's blowup function must be exponential on the polynomial-size parity
source family. -/
theorem parity_forces_blowup_succ (T : XorToDNFCompiler) {n : Nat} :
    2 ^ n ≤ T.blowup (n + 1) := by
  have hterms := T.parity_compiled_terms_ge_succ (n := n)
  have hsize := T.size_le (XorLinearForm.parityForm (n + 1))
  simpa [XorLinearForm.parityForm_size] using hterms.trans hsize

/-- Fixed-dimension impossibility form. -/
theorem false_of_parity_blowup_lt (T : XorToDNFCompiler) {n : Nat}
    (hsmall : T.blowup (n + 1) < 2 ^ n) :
    False := by
  have hge := T.parity_forces_blowup_succ (n := n)
  omega

end XorToDNFCompiler

/-! ### Cover, quotient, and projection compression

The parity separation above identifies the compression mechanism: local covers enumerate
positive subcubes, while algebraic/quotient representations preserve a global invariant.  The
next definitions make that distinction semantic, and add a projection layer for NP/UP-style
"small verifier upstairs, complicated language downstairs" questions. -/

/-- Semantic certificate-cover lower bound: every DNF/local-positive-certificate cover for `f`
needs at least `L` regions. This abstracts away from DNF syntax and names the measure we are
lower-bounding. -/
def CertificateCoverLowerBound {n : Nat} (f : BoolFunc n) (L : Nat) : Prop :=
  ∀ D : DNF n, D.Computes f → L ≤ D.card

/-- A function has a certificate cover of size at most `s`. -/
def HasCertificateCoverAtMost {n : Nat} (f : BoolFunc n) (s : Nat) : Prop :=
  ∃ D : DNF n, D.Computes f ∧ D.card ≤ s

/-- Lower bounds rule out smaller certificate covers. -/
theorem not_hasCertificateCoverAtMost_of_lowerBound {n L s : Nat} {f : BoolFunc n}
    (hlower : CertificateCoverLowerBound f L) (hs : s < L) :
    ¬ HasCertificateCoverAtMost f s := by
  rintro ⟨D, hD, hcard⟩
  have hL : L ≤ D.card := hlower D hD
  omega

/-- Parity has exponential certificate-cover complexity in dimension `n+1`. -/
theorem parity_certificateCoverLowerBound_succ {n : Nat} :
    CertificateCoverLowerBound (parity : BoolFunc (n + 1)) (2 ^ n) := by
  intro D hD
  exact parity_dnf_lower_bound_succ D hD

/-- A quotient representation computes by mapping the cube into a state space and then accepting
states. If `stateMap` is arbitrary this is too powerful; the useful instances are compositional
ones such as XOR-linear forms, finite automata, counters, or low-degree algebraic summaries. -/
structure QuotientRepresentation (n : Nat) where
  State : Type
  stateSize : Nat
  stateMap : Vertex n → State
  accept : State → Bool

namespace QuotientRepresentation

/-- Evaluation of a quotient representation. -/
def eval {n : Nat} (Q : QuotientRepresentation n) : BoolFunc n :=
  fun x => Q.accept (Q.stateMap x)

/-- A quotient representation computes a Boolean function. -/
def Computes {n : Nat} (Q : QuotientRepresentation n) (f : BoolFunc n) : Prop :=
  ∀ x, Q.eval x = f x

end QuotientRepresentation

/-- Parity as the smallest quotient story: the quotient state is one bit, the invariant is
Hamming-weight parity, and acceptance is the odd state. -/
def parityQuotient (n : Nat) : QuotientRepresentation n where
  State := Bool
  stateSize := 2
  stateMap := parity
  accept := id

theorem parityQuotient_computes (n : Nat) :
    (parityQuotient n).Computes (parity : BoolFunc n) := by
  intro x
  rfl

/-- A Boolean relation between base variables and witness/auxiliary variables. -/
abbrev BoolRelation (n w : Nat) := (Vertex n) → (Vertex w) → Bool

/-- Existential projection of a lifted relation onto its base variables. -/
noncomputable def ProjectRelation {n w : Nat} (R : BoolRelation n w) : BoolFunc n :=
  fun x => decide (∃ y : Vertex w, R x y = true)

/-- A projected representation packages a lifted verifier/relation and an abstract upstairs cost.
The field `localVerifier` is intentionally a proposition: later instances can say this relation
comes from small 3-CNF, bounded-width proof rules, affine constraints, etc. -/
structure ProjectionRepresentation (n w : Nat) where
  relation : BoolRelation n w
  cost : Nat
  localVerifier : Prop

namespace ProjectionRepresentation

/-- The base language obtained by projecting out witnesses. -/
noncomputable def language {n w : Nat} (P : ProjectionRepresentation n w) : BoolFunc n :=
  ProjectRelation P.relation

/-- `P` computes `f` after existentially projecting witnesses. -/
def Computes {n w : Nat} (P : ProjectionRepresentation n w) (f : BoolFunc n) : Prop :=
  ∀ x, P.language x = f x

/-- Projection blowup theorem: if the projected language has certificate-cover lower bound `L`,
then every base-space certificate cover for the projection still has size at least `L`, no matter
how small the lifted representation was. This isolates projection as a compression operation. -/
theorem base_cover_lower_bound {n w L : Nat} (P : ProjectionRepresentation n w)
    {f : BoolFunc n} (hP : P.Computes f) (hlower : CertificateCoverLowerBound f L) :
    CertificateCoverLowerBound P.language L := by
  intro D hD
  apply hlower D
  intro x
  rw [← hP x]
  exact hD x

/-- If the lifted representation has cost below the projected cover lower bound, then projection
has produced a genuine gap between upstairs description cost and downstairs local-cover cost. -/
def HasCoverCompressionGap {n w : Nat} (P : ProjectionRepresentation n w) (L : Nat) : Prop :=
  P.cost < L ∧ CertificateCoverLowerBound P.language L

end ProjectionRepresentation

/-- A toy projected parity representation with one dummy witness bit. It uses the quotient
parity invariant in the lifted relation; later, this slot can be replaced by a genuinely local
chain verifier with auxiliary prefix-parity states. -/
noncomputable def parityProjection (n : Nat) : ProjectionRepresentation n 1 where
  relation := fun x _ => parity x
  cost := n + 1
  localVerifier := True

theorem parityProjection_computes (n : Nat) :
    (parityProjection n).Computes (parity : BoolFunc n) := by
  intro x
  unfold ProjectionRepresentation.language ProjectRelation parityProjection
  by_cases hp : parity x = true
  · apply Eq.trans
    · apply decide_eq_true
      exact ⟨fun _ => false, hp⟩
    · exact hp.symm
  · have hfalse : parity x = false := Bool.eq_false_of_not_eq_true hp
    apply Eq.trans
    · apply decide_eq_false
      intro h
      rcases h with ⟨y, hy⟩
      exact hp hy
    · exact hfalse.symm

/-- Projected parity inherits the exponential base-space certificate-cover lower bound. -/
theorem parityProjection_certificateCoverLowerBound_succ {n : Nat} :
    CertificateCoverLowerBound (parityProjection (n + 1)).language (2 ^ n) :=
  (parityProjection (n + 1)).base_cover_lower_bound
    (parityProjection_computes (n + 1))
    parity_certificateCoverLowerBound_succ

/-- The upstairs cost of the projected parity representation is polynomial. -/
theorem parityProjection_cost_polynomial :
    _root_.HypercubeSAT.PolynomialO (fun n => (parityProjection n).cost) := by
  refine ⟨1, 1, 0, ?_⟩
  intro N _
  simp [parityProjection]

/-! ### Local prefix-parity projection target

The toy `parityProjection` above uses the parity quotient upstairs.  The stronger Cook-Levin
miniature is a local prefix-state verifier: witnesses carry states `s_0, ..., s_n`, constraints
enforce `s_0 = 0`, `s_{i+1} = s_i xor x_i`, and acceptance enforces `s_n = 1`.  The following
contract names exactly that object and proves the reusable lower-bound consequence for any
implementation satisfying the contract. -/

/-- Boolean XOR, as inequality of bits. -/
def boolXor (a b : Bool) : Bool :=
  decide (a ≠ b)

/-- A local prefix-parity projection is an upstairs relation with `n+1` witness bits intended
to represent prefix parity states.  The `localChainVerifier` field is a contract for the local
constraint shape; the computational field says its existential projection is parity. -/
structure LocalPrefixParityProjection (n : Nat) where
  relation : BoolRelation n (n + 1)
  cost : Nat
  localChainVerifier : Prop
  computesParity : ProjectRelation relation = (parity : BoolFunc n)

namespace LocalPrefixParityProjection

/-- View a local prefix-parity projection as an ordinary projection representation. -/
noncomputable def toProjectionRepresentation {n : Nat}
    (P : LocalPrefixParityProjection n) : ProjectionRepresentation n (n + 1) where
  relation := P.relation
  cost := P.cost
  localVerifier := P.localChainVerifier

/-- The projected language of a local prefix-parity verifier is parity. -/
theorem computes {n : Nat} (P : LocalPrefixParityProjection n) :
    P.toProjectionRepresentation.Computes (parity : BoolFunc n) := by
  intro x
  exact congrFun P.computesParity x

/-- Any local prefix-parity verifier inherits the exponential downstairs certificate-cover
lower bound after projection. -/
theorem certificateCoverLowerBound_succ {n : Nat}
    (P : LocalPrefixParityProjection (n + 1)) :
    CertificateCoverLowerBound P.toProjectionRepresentation.language (2 ^ n) :=
  P.toProjectionRepresentation.base_cover_lower_bound P.computes
    parity_certificateCoverLowerBound_succ

/-- If the local prefix verifier has cost below the parity cover lower bound, then it exhibits
a real projection compression gap. -/
theorem hasCoverCompressionGap_succ {n : Nat}
    (P : LocalPrefixParityProjection (n + 1)) (hcost : P.cost < 2 ^ n) :
    P.toProjectionRepresentation.HasCoverCompressionGap (2 ^ n) :=
  ⟨hcost, P.certificateCoverLowerBound_succ⟩

end LocalPrefixParityProjection

/-- The explicit construction target for the next proof step.  Proving this means building the
prefix-state relation and proving its projection is exactly parity. -/
def LocalPrefixParityProjectionExists (n : Nat) : Prop :=
  ∃ P : LocalPrefixParityProjection n, P.cost ≤ n + 1 ∧ P.localChainVerifier

/-- The first prefix state index `s_0`. -/
def prefixStartIndex (n : Nat) : Fin (n + 1) :=
  ⟨0, Nat.succ_pos n⟩

/-- The final prefix state index `s_n`. -/
def prefixFinalIndex (n : Nat) : Fin (n + 1) :=
  ⟨n, Nat.lt_succ_self n⟩

/-- The intended local prefix-parity relation:
`s_0 = false`, every transition satisfies `s_{i+1} = s_i xor x_i`, and `s_n = true`.
The existential projection of this relation should be parity. -/
noncomputable def prefixParityRelation (n : Nat) : BoolRelation n (n + 1) :=
  fun x s =>
    decide
      (s (prefixStartIndex n) = false ∧
        (∀ i : Fin n, s (Fin.succ i) = boolXor (s (Fin.castSucc i)) (x i)) ∧
        s (prefixFinalIndex n) = true)

/-- The explicit theorem target for the prefix-chain verifier. -/
def PrefixParityRelationComputesParity (n : Nat) : Prop :=
  ProjectRelation (prefixParityRelation n) = (parity : BoolFunc n)

/-- Once the finite prefix-state recurrence is proved, the explicit relation becomes a local
prefix-parity projection with linear upstairs cost. -/
noncomputable def localPrefixParityProjectionOfComputes {n : Nat}
    (h : PrefixParityRelationComputesParity n) : LocalPrefixParityProjection n where
  relation := prefixParityRelation n
  cost := n + 1
  localChainVerifier := True
  computesParity := h

theorem localPrefixParityProjectionExists_of_prefixRelationComputes {n : Nat}
    (h : PrefixParityRelationComputesParity n) :
    LocalPrefixParityProjectionExists n := by
  refine ⟨localPrefixParityProjectionOfComputes h, ?_, trivial⟩
  simp [localPrefixParityProjectionOfComputes]

/-- A benchmark explaining whether a lower-bound method understands both sides of a function:
hardness for a chosen normal form and easiness for unrestricted circuits. Parity should fill
this shape: DNF-hard, but circuit-easy when sharing/XOR-like computation is allowed. -/
structure SharingDiagnostic (L : (N : Nat) → BoolFunc N) where
  normalHard : Prop
  unrestrictedEasy : Prop
  sharingExplanation : Prop

namespace SharingDiagnostic

/-- A diagnostic passes the parity-style sanity test when it records both restricted hardness
and unrestricted easiness, plus an explanation of the gap. -/
def Passes (D : SharingDiagnostic L) : Prop :=
  D.normalHard ∧ D.unrestrictedEasy ∧ D.sharingExplanation

end SharingDiagnostic

/-- Concrete parity diagnostic for this file's current theorem frontier. We have already proved
normal-form hardness. The unrestricted-easy and sharing-explanation fields are explicit
obligations to fill with a circuit construction or a richer circuit model later. -/
def paritySharingDiagnostic : SharingDiagnostic
    (fun N => (parity : BoolFunc N)) where
  normalHard := ∀ n : Nat,
    NormalFormRequiresSizeAtLeast (parity : BoolFunc (n + 1)) (2 ^ n)
  unrestrictedEasy := _root_.HypercubeSAT.PolynomialO
    (fun N => (XorLinearForm.parityForm N).size)
  sharingExplanation := ∀ N : Nat,
    (XorLinearForm.parityForm N).Computes (parity : BoolFunc N)

theorem paritySharingDiagnostic_normalHard :
    paritySharingDiagnostic.normalHard := by
  intro n
  exact parity_normalFormCircuit_size_ge

theorem paritySharingDiagnostic_passes :
    paritySharingDiagnostic.Passes := by
  refine ⟨paritySharingDiagnostic_normalHard, ?_, ?_⟩
  · exact XorLinearForm.parityForm_size_polynomial
  · exact XorLinearForm.parityForm_computes

/-- Random restrictions are represented abstractly as a map from smaller inputs to larger
inputs. This captures partial assignments without committing to a probability distribution yet. -/
structure Restriction (N M : Nat) where
  extend : (Fin M → Bool) → Fin N → Bool

namespace Restriction

/-- Restrict a Boolean function along a partial assignment/projection. -/
def apply {N M : Nat} (R : Restriction N M) (f : BoolFunc N) : BoolFunc M :=
  fun x => f (R.extend x)

end Restriction

/-- A random-restriction bridge says that every small circuit from a class becomes expressible
in a blocker/normal-form model after applying a restriction from `restrictions`. This is the
formal version of the dream question: "do small circuits become blocker-normal under random
restriction?" -/
structure RestrictionNormalizesCircuits
    (T : NormalFormCompiler) (small : Nat → Nat) where
  restrictions : (N M : Nat) → Finset (Restriction N M)
  normalizes : ∀ {N M : Nat} (C : BoolCircuit N),
    C.size ≤ small N →
    ∃ R ∈ restrictions N M, ∃ A : T.normal M,
      T.normalEval A = R.apply C.eval

/-- The lower-bound milestones we should distinguish.  Each one is meaningful on its own and
is much more honest than jumping directly to unrestricted P-vs-NP claims. -/
inductive LowerBoundMilestone where
  | dnf
  | normalForm
  | decisionTree
  | readOnceBranchingProgram
  | monotoneCircuit
  | ac0
  | resolution
  | polynomialCalculus
  | uniformSAT
  | generalCircuit
  deriving DecidableEq, Repr

/-- A named lower-bound claim at a specific milestone.  `proved` is deliberately a field of
type `Prop`, so the file can track which envelopes are established, conjectural, or conditional. -/
structure MilestoneClaim where
  milestone : LowerBoundMilestone
  family : (N : Nat) → BoolFunc N
  lower : Nat → Nat
  proved : Prop

/-- Milestones at or below normal-form/circuit-cover reasoning. -/
def LowerBoundMilestone.isHypercubeNormalForm :
    LowerBoundMilestone → Bool
  | .dnf => true
  | .normalForm => true
  | .decisionTree => true
  | _ => false

/-- Milestones that require reasoning beyond canonical subcube covers. -/
def LowerBoundMilestone.requiresSharingInvariant :
    LowerBoundMilestone → Bool
  | .readOnceBranchingProgram => true
  | .monotoneCircuit => true
  | .ac0 => true
  | .uniformSAT => true
  | .generalCircuit => true
  | _ => false

/-- A milestone cannot simultaneously be tagged as a pure normal-form milestone and as needing
an unrestricted sharing invariant in this classification. -/
theorem LowerBoundMilestone.normalForm_not_requiresSharingInvariant
    (m : LowerBoundMilestone)
    (h : m.isHypercubeNormalForm = true) :
    m.requiresSharingInvariant = false := by
  cases m <;> simp [LowerBoundMilestone.isHypercubeNormalForm,
    LowerBoundMilestone.requiresSharingInvariant] at h ⊢

/-! ### Decision-tree invariants -/

/-- Deterministic decision trees over `N` input bits. -/
inductive DecisionTree (N : Nat) where
  | leaf : Bool → DecisionTree N
  | query : Fin N → DecisionTree N → DecisionTree N → DecisionTree N
  deriving Repr

namespace DecisionTree

/-- Evaluate a decision tree. The left branch is the `false` branch and the right branch is
the `true` branch. -/
def eval {N : Nat} : DecisionTree N → (Fin N → Bool) → Bool
  | leaf b, _ => b
  | query i lo hi, x => if x i then eval hi x else eval lo x

/-- Number of query nodes. -/
def size {N : Nat} : DecisionTree N → Nat
  | leaf _ => 0
  | query _ lo hi => lo.size + hi.size + 1

/-- Number of leaves in a decision tree. Leaves are the terminal information regions. -/
def leafCount {N : Nat} : DecisionTree N → Nat
  | leaf _ => 1
  | query _ lo hi => leafCount lo + leafCount hi

/-- Number of accepting leaves in a decision tree. -/
def acceptingLeafCount {N : Nat} : DecisionTree N → Nat
  | leaf b => if b then 1 else 0
  | query _ lo hi => acceptingLeafCount lo + acceptingLeafCount hi

/-- A finite binary decision tree with `s` query nodes has exactly `s + 1` leaves. This is the
basic information accounting identity for decision trees. -/
theorem leafCount_eq_size_add_one {N : Nat} (T : DecisionTree N) :
    T.leafCount = T.size + 1 := by
  induction T with
  | leaf b =>
      simp [leafCount, size]
  | query i lo hi ihlo ihhi =>
      simp [leafCount, size, ihlo, ihhi]
      omega

/-- Accepting leaves are bounded by all leaves. -/
theorem acceptingLeafCount_le_leafCount {N : Nat} (T : DecisionTree N) :
    T.acceptingLeafCount ≤ T.leafCount := by
  induction T with
  | leaf b =>
      cases b <;> simp [acceptingLeafCount, leafCount]
  | query i lo hi ihlo ihhi =>
      simp [acceptingLeafCount, leafCount]
      omega

/-- Accepting leaves are bounded by query nodes plus one. -/
theorem acceptingLeafCount_le_size_add_one {N : Nat} (T : DecisionTree N) :
    T.acceptingLeafCount ≤ T.size + 1 := by
  rw [← T.leafCount_eq_size_add_one]
  exact T.acceptingLeafCount_le_leafCount

/-- If a lower-bound argument forces more accepting regions than `s + 1`, then the tree must
use more than `s` query nodes. -/
theorem size_gt_of_acceptingLeafCount_gt_succ {N s : Nat} {T : DecisionTree N}
    (h : s + 1 < T.acceptingLeafCount) :
    s < T.size := by
  have hle := T.acceptingLeafCount_le_size_add_one
  omega

/-- A decision tree has an accepting-leaf information cover for `f` when its accepting leaves
can be represented as hypercube regions covering all true vertices of `f`, with no more regions
than accepting leaves. For arbitrary trees this is a contract; read-once/path-normalized trees
will later construct it directly. -/
abbrev HasAcceptingLeafInformationCover {N : Nat} (T : DecisionTree N) (f : BoolFunc N) :
    Type :=
  HasInformationCoverAtCost f T.acceptingLeafCount

/-- If a decision tree has an accepting-leaf information cover and each accepting region carries
at most `K` true vertices, then its true set has at most `acceptingLeafCount * K` vertices. -/
theorem trueVertices_card_le_acceptingLeafCount_mul_of_region_card_le {N K : Nat}
    {T : DecisionTree N} {f : BoolFunc N}
    (hcover : T.HasAcceptingLeafInformationCover f)
    (hregion : ∀ P : Pattern N, P ∈ hcover.cover.regions → (P.trueFootprint f).card ≤ K) :
    (trueVertices f).card ≤ T.acceptingLeafCount * K :=
  hcover.trueVertices_card_le_cost_mul_of_region_card_le hregion

/-- Decision-tree information-capacity lower bound. If `s + 1` accepting regions of capacity
`K` cannot carry the true set, then any tree with such an accepting-leaf cover needs more than
`s` query nodes. -/
theorem size_gt_of_trueVertices_gt_accepting_capacity {N s K : Nat}
    {T : DecisionTree N} {f : BoolFunc N}
    (hcover : T.HasAcceptingLeafInformationCover f)
    (hregion : ∀ P : Pattern N, P ∈ hcover.cover.regions → (P.trueFootprint f).card ≤ K)
    (hgt : (s + 1) * K < (trueVertices f).card) :
    s < T.size := by
  have hcount : s + 1 < T.acceptingLeafCount := by
    have hcap := hcover.trueVertices_card_le_cost_mul_of_region_card_le hregion
    by_contra hnot
    have hle : T.acceptingLeafCount ≤ s + 1 := by omega
    have hmul : T.acceptingLeafCount * K ≤ (s + 1) * K :=
      Nat.mul_le_mul_right K hle
    omega
  exact T.size_gt_of_acceptingLeafCount_gt_succ hcount

/-- Coordinates that appear in some query node. -/
def queries {N : Nat} : DecisionTree N → Finset (Fin N)
  | leaf _ => ∅
  | query i lo hi => insert i (lo.queries ∪ hi.queries)

/-- A decision tree computes a Boolean function. -/
def Computes {N : Nat} (T : DecisionTree N) (f : BoolFunc N) : Prop :=
  ∀ x, T.eval x = f x

/-- A decision tree can mention at most one new coordinate per query node. -/
theorem queries_card_le_size {N : Nat} (T : DecisionTree N) :
    T.queries.card ≤ T.size := by
  induction T with
  | leaf b =>
      simp [queries, size]
  | query i lo hiT ihlo ihhi =>
      have hinsert := Finset.card_insert_le i (lo.queries ∪ hiT.queries)
      have hunion := Finset.card_union_le lo.queries hiT.queries
      simp [queries, size]
      omega

/-- If a coordinate is never queried, flipping it cannot change the tree output. -/
theorem eval_flip_of_not_mem_queries {N : Nat} (T : DecisionTree N)
    {x : Fin N → Bool} {i : Fin N} (hi : i ∉ T.queries) :
    T.eval (flipVertex x i) = T.eval x := by
  induction T with
  | leaf b =>
      rfl
  | query j lo hiT ihlo ihhi =>
      have hij : i ≠ j := by
        intro h
        subst i
        exact hi (by simp [queries])
      have hino_lo : i ∉ lo.queries := by
        intro hmem
        exact hi (by simp [queries, hmem])
      have hino_hi : i ∉ hiT.queries := by
        intro hmem
        exact hi (by simp [queries, hmem])
      rw [eval, eval]
      rw [flipVertex_ne x hij.symm]
      by_cases hxj : x j
      · simp [hxj, ihhi hino_hi]
      · simp [hxj, ihlo hino_lo]

/-- Every coordinate of an edge-flipping function must be queried somewhere by any decision
tree computing it. -/
theorem queries_eq_univ_of_computes_edgeFlipping {N : Nat} {T : DecisionTree N}
    {f : BoolFunc N} (hflip : EdgeFlipping f) (hcomp : T.Computes f) :
    T.queries = Finset.univ := by
  apply Finset.eq_univ_iff_forall.mpr
  intro i
  by_contra hnot
  let x : Vertex N := zeroVertex N
  have hsame : T.eval (flipVertex x i) = T.eval x :=
    T.eval_flip_of_not_mem_queries hnot
  have hfx : T.eval x = f x := hcomp x
  have hfflip : T.eval (flipVertex x i) = f (flipVertex x i) := hcomp (flipVertex x i)
  rw [hfflip, hfx, hflip x i] at hsame
  cases f x <;> simp at hsame

/-- Size lower bound for decision trees computing an edge-flipping function. -/
theorem size_ge_dimension_of_computes_edgeFlipping {N : Nat} {T : DecisionTree N}
    {f : BoolFunc N} (hflip : EdgeFlipping f) (hcomp : T.Computes f) :
    N ≤ T.size := by
  have hqueries := T.queries_eq_univ_of_computes_edgeFlipping hflip hcomp
  have hcard : T.queries.card = N := by
    rw [hqueries, Finset.card_univ, Fintype.card_fin]
  have hle := T.queries_card_le_size
  omega

/-- Parity decision trees must have at least one query node per coordinate. -/
theorem parity_decisionTree_size_ge_dimension {N : Nat} {T : DecisionTree N}
    (hcomp : T.Computes (parity : BoolFunc N)) :
    N ≤ T.size :=
  T.size_ge_dimension_of_computes_edgeFlipping parity_edgeFlipping hcomp

end DecisionTree

end HypercubeSAT
