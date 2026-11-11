# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **`preprocess`: formula preprocessing before search.** Level-0 units, equivalent-literal substitution over the
  binary-implication graph, subsumption with self-subsuming strengthening, and bounded variable elimination that
  resolves only through an AND or XOR gate definition when one defines the variable. Every created clause is logged as
  a RUP step before the clauses it replaces are logged as deletions, so the steps replay with their deletions honoured,
  as a DRAT checker reads them, and followed by a search's learned clauses refute the input;
  every removed clause the simplified formula no longer implies goes on a reconstruction stack that `extend` walks
  backward to turn a model of the simplified formula into one of the input. `solve_preprocessed` decides a formula
  that way and checks its model against the input.
- **Congruence closure in `preprocess`** (Biere, Fleury & Froleyks, SAT 2024). AND gates (a long clause beside its
  binaries), XOR gates (parity gadgets of three to five variables, each variable read as the output) and
  if-then-else gates (four ternary clauses) are hashed by their inputs' representatives, and two gates of one key
  merge their outputs, closing until no gate merges; a gate whose inputs collapse fixes its output or equates it with
  its one input. Every merge is RUP-proved before the next — an AND's outright, an XOR's or an if-then-else's through a
  tree of case splits on its inputs or condition, deleted once the equality stands — and the merged variables are
  substituted out. The two copies of a circuit in an equivalence-checking miter fold into one: three SAT Competition
  2024 miters that timed out at 60 s (0a02751…, 1545660…, 1a20d90…) are refuted by preprocessing alone in under 2 s,
  32,649 merges on the first where Kissat's congruence closure finds 32,727. `Options::congruence`,
  `Preprocessed::congruent`.
- **Bounded variable addition in `preprocess`** (Manthey, Heule & Biere, *Automated Reencoding of Boolean Formulas*,
  2012; Kissat's factoring). Literals `l₁ … l_k` occurring beside the same rests `R₁ … R_m` are factored through a
  fresh `x`: the `k·m` clauses `lᵢ ∨ Rⱼ` become `lᵢ ∨ x` and `¬x ∨ Rⱼ`, grown greedily from the most frequent
  literal while the saving grows, the matches found once per pivot over clauses indexed by length. The new clauses
  are RAT on `x`, logged as PR steps witnessed by it. `Options::factor` (off by default, so a default run keeps the
  input's variables), `Preprocessed::num_vars` and `Preprocessed::factored`; the CDCL route turns it on and solves
  over the added variables. A pairwise at-most-one over 400 variables, 79,800 binaries, factors into a few thousand
  clauses. On the SAT Competition 2024 sample, 0ef029… (421 factorings, as Kissat), 1f44da… and cb2e8b… are refuted
  in under 11 s where the search timed out at 60 s; over four shuffles of the seven instances where factoring changed
  a run, it solves 13 of 28 against 2 without.
- **A forward proof checker by watched literals** (`pr::Forward`): every clause a proof ever holds enters one
  propagator, switched on as the proof adds it and off as it deletes it; a RUP step costs its propagation and a PR
  step with an assignment witness visits only the clauses holding a negated witness literal.
  `pr::check_pr_refutation_fast` uses it for proofs with deletions, and the DRAT and DPR emitters for their own
  checks, where each re-checked every step against the whole database (a factored refutation's export took minutes).
- **`walk`: ProbSAT local search.** Break-count-weighted flips with Kissat's fitted base per clause width, the best
  assignment met kept by a trail of flips folded into a snapshot, level-0 facts held fixed.
- **Stable-mode phases in the CDCL core.** Target and best phases (the longest conflict-free trails since the last
  rephase) drive stable-mode decisions, and stable mode rephases on Kissat's schedule — best, walk, inverted, best,
  walk, original — the walking steps running `walk` from the decision phases and keeping its best. `set_target_phases`,
  `set_walk`, `rephases`, `walks`.
- **Lucky phases** (Kissat's `lucky.c`): before search, complete assignments by propagation — the decision phases,
  then all false and all true, forward and backward — a conflicting decision flipped, a failed first decision learned
  as a unit; a pass that assigns every variable is the model. The attempts run on a copy of the solver, so a fruitless
  pass leaves only the units it proved: the search after it meets the same watches and phases as one without it.
  Answers in milliseconds the SAT Competition instances Kissat calls lucky, where the search took minutes. On by
  default; off with a decision set or LRAT tracing.
- **Gauss–Jordan elimination inside the search** (`xor_engine::WatchedXor`; Han & Jiang, CAV 2012; Soos, Gocht &
  Meel, CAV 2020). Each component of a formula's XOR equations is a bit matrix in reduced row echelon form, every row
  watching one column neither basic nor assigned, so an assignment visits only the rows watching its column and the
  row it is basic in; an assigned basic column trades places with a free one by row additions that backtracking never
  undoes. Every assignment the equations force is propagated, and every one they refute is a conflict, at the
  fixpoint it arises in. What a row says is read word by word, and a literal it forces goes to the search with the
  row as its token, its clause built only when conflict analysis resolves on it: a row with no free column is never
  changed while that literal stays assigned. The hybrid-XOR route of `solve` searches with it, `cdcl_stats` with
  `CDCL_XOR=watched`. On SATLIB's par32 parity-learning family, ten instances run four times each (as given and under
  three seeded shuffles) at 120 s CPU, the route takes 119.4 s in all and at most 8.6 s on one run; CryptoMiniSat
  5.11.21, whose Gauss–Jordan engine this family is the benchmark of, takes 296.3 s over four random seeds and at
  most 18.0 s, and Kissat 4.0.4 and the route without the theory decide none. The route lets the theory go once fewer
  than one of its calls in a thousand has ended in a refutation, weighed every 4096 calls: the formula's clauses,
  which the route keeps, force what it forces a step at a time, and what it alone finds early is a contradiction among
  rows. On a preimage of SHA-1's compression cut to 18 steps, 128 block bits given, the matrix spans thousands of
  rows, took most of the search's time and refuted almost nothing: 4.65 s becomes 0.34 s, against 0.78 s for Kissat
  and 0.07 s for CryptoMiniSat. On parity and on Trivium, where it refutes in most calls, it stays; the SAT arena is
  unchanged (71 of its 90 formulas decided either way, PAR-2 2367.7 and 2364.5).
- **A theory's propagations stay out of the clause database.** The CDCL core explains a literal a theory forced on a
  stack kept in trail order and cut by the backtrack that unassigns the literal: no watch, no learned-clause log, no
  reduction. An explanation is the theory's clause or its token (`cdcl::Theory::propagate_lazily`), whose clause
  conflict analysis asks for (`Theory::explain`) only for a literal it resolves on. On par32 a search had added 35
  clauses per conflict to the database; it now uses a tenth of the memory. A proof-logging solve still carries the
  clauses into the database. `Theory::propagate_kept` tells each theory how much of the trail it was given last time
  is still in place, so an incremental theory undoes only what lies past it.
- **Reason-side bumping** (`Solver::set_bump_reasons`, off by default; Kissat's `bumpreasons`; Liang, Ganesh, Poupart &
  Czarnecki, SAT 2016): conflict analysis bumps the literals of the learned clause's literals' reasons too, unless
  they number more than ten a learned literal. In the raw core at 60 s it decides 41 of the SAT Competition 2024
  sample's 62 formulas against 34 (PAR-2 3124 against 3683), and the same SATLIB and structured arena as without it;
  `reason_side_bumps` counts the literals it bumped.
- **`portfolio`: configurations of the CDCL core raced on threads, deterministically.** Every round each search
  continues for a budget of conflicts that doubles from round to round, and the first round in which any decides ends
  the race, won by the lowest-numbered configuration that decided in it; a search that decides stops the
  higher-numbered ones of its round, which cannot win it. Between rounds every search takes the units and binaries the
  others learned (`Solver::import`), so the winner, its model or its refutation (every search's learned clauses,
  round by round) follow from the formula and the configurations alone, never from the threads' timing;
  `portfolio::race` can keep the searches apart. `portfolio::DEFAULT` races the core, reason-side bumping, a focused
  search (Glucose restarts deciding by the move-to-front queue and backtracking chronologically past jumps of more
  than 100 levels, `Config::chrono`), and Luby restarts with positive phases; `cdcl_stats` races them with
  `CDCL_PORTFOLIO=1`, apart with `CDCL_PORTFOLIO_SHARE=0`. On the SAT Competition 2024 sample at 60 s of wall time, the
  CDCL route's preprocessing first, the four decide 60 of 62 formulas sharing (PAR-2 769, and 770 in a second run)
  and 59 apart (PAR-2 835 to 860 over three runs; the core winning 38 of them, reason-side bumping 13 and the other
  two 4 each). The core alone decides 57 (PAR-2 1043), and the race apart with Glucose restarts and reason-side
  bumping in the focused search's place 58 (PAR-2 949): only the focused search decides `ddc0720f`.
- **Decisions by justification.** `Solver::set_gate` makes a variable the conjunction of two literals by clauses the
  solver holds, and `Solver::set_justifying` decides by justification: each decision sets false a fan-in of the first
  gate on the trail assigned false with both fan-ins open, the fan-in of higher activity, and a search left with no
  such gate is satisfiable, answered with what it has assigned. For a circuit, every clause true in every evaluation
  of its gates, that assignment is the evaluation of the inputs it sets, so a query's search and its model are the
  part of the circuit its assumptions need, whatever the solver holds. `Solver::value` reads what the last search left
  a variable at, `None` for one it left open. `Solver::add_side_clause` gives such a search clauses beside the gates
  that it makes true itself: once no gate is left to justify, it decides an open literal of the first side clause with
  no literal true, so a circuit query may carry clauses over its inputs and gates and still leave open what nothing
  needs.
- **`Solver::unsat_core`.** After a refutation under assumptions, the assumptions it rests on, in the order given: the
  decisions conflict analysis reaches back to from the assumption found false, every decision then an assumption.
- **`cube`: cube-and-conquer, deterministic and certified.** A lookahead splits the formula into cubes: at each node
  the variables of most weight in the clauses left open are probed both ways on levels of their own, a polarity that
  conflicts forcing the other on the path, both refuting the node, and the node otherwise branches on the variable
  whose polarities assign the most (the product of the two counts) until `Options::depth` branches lie on the path.
  Incremental solvers on threads take the cubes round-robin and solve each under assumptions; the formula is
  satisfiable when a cube is, answered by the lowest-numbered one, so the answer follows from the formula and the
  options alone. A refutation is RUP throughout: the lookahead's failed literals, each solver's learned clauses and
  cube negations in its order, then the split's clauses bottom-up to the empty clause. `cdcl_stats` runs it with
  `CDCL_CUBE=<depth>` (`CDCL_CUBE_THREADS`, `CDCL_CUBE_PRESELECT`). On twelve hard random 3-SAT formulas of the
  scale arena, 120 s of wall time each, depth ten on four threads decides none, where Kissat and the portfolio each
  decide one; no route runs it.
- **Cover gates in the CDCL core's justification.** `Solver::set_cover_gate` makes a variable a gate of several leaves:
  true exactly when one of its products holds and false exactly when one of its others does, by clauses the solver
  holds. A justifying search justifies it by a product of its value whose literals all hold, and otherwise decides the
  most active open literal of one with no literal false (any open leaf when every product has one); an And gate is the
  cover of one product and two. Random circuits of cover gates decide as plain CDCL, every model evaluating to its
  assumptions whatever its open inputs take and every refutation replaying.
- **`pdr`: property-directed reachability over AIGER circuits on this crate's own solver, certified.** Frames of clauses
  over the latches, one incremental solver each; a bad state of the last frame blocked by relative induction, its cube
  shrunk to the latches the refutation's unsat core names and then by dropping literals while it stays blocked, its
  clause added at the highest frame it stays inductive at; a predecessor lifted to the latches the step into its
  successor needs, its inputs fixed, so a run replays from its initial state; clauses pushed forward until a frame
  equals the next. The invariant and the run are held to `itp`'s checks (each obligation an LRAT refutation, the run a
  simulation); `mc_check` decides AIGER files from the command line, by it or by `itp`. Every frame's solver justifies
  (`Solver::add_side_clause`, the frame's clauses and the query's beside the step's gates), so a query's search and
  model are the part of the step it needs, and a predecessor's lifting starts from the latches its model set. The search
  runs over the circuit cut to its bad states' and constraints' cone of influence, and its certificate, taken back to
  the whole circuit, is held to the whole circuit's checks. On the HWMCC'24 bit-level track at 60 s of CPU, every
  verdict as the competition's solvers gave it: justifying solvers decide 91 of the 319 designs against 79 for solvers
  that do not, run beside them, in 35% of their CPU on the 67 both decide; and cut to the cone it decides 100 (87 safe,
  13 unsafe) against 89 for the whole circuit run beside it, the 19.8 million latches of the track 3.5 million in their
  cones. A literal is dropped from a blocked cube by Bradley's down with counterexamples to generalization (Hassan,
  Bradley & Somenzi, FMCAD 2013): a predecessor that keeps the smaller cube from being blocked is itself blocked a frame
  lower, generalized, when it can be, up to `Options::ctgs` in a row (`Stats::ctgs` counts them), and the cube is
  otherwise cut to the literals the predecessor shares with it; that decides 109 (95 safe, 14 unsafe) against 102 for
  dropping literals alone, run beside it. ABC's `pdr`, given each design as AIGER 1.9 describes it (every latch free to
  start anywhere taking its first value from an input of its own, the constraints folded into the property), decides
  114, each verdict the competition's. Ternary-simulation lifting, tried, decides fewer and stays out, and so does
  excluding a cube by a clause that subsumes it before the frame's solver is asked. A cube is lifted with the invariant
  constraints assumed rather than kept (`Options::assume_constraints`), so it keeps no latch only a constraint reads,
  and a run through such cubes that breaks a constraint where it leaves the states it was lifted from has its bad state
  searched again keeping them (`Stats::restarts`; `mc_check --strict-lifting` keeps them throughout): 100 decided
  against 94 keeping them, run beside it, in 18% less CPU on the 93 both decide. A predecessor is lifted under the
  inputs its model set, those it left open free, and a frame's solver holds a gate once one of its queries reads it, so
  its propagation never runs into logic nothing asked of the frame depends on: 114 decided against 110, run beside it,
  in 58% less CPU on the 108 both decide, the 189,026-latch cv32e40x properties of the track in seconds. The certificate
  is checked once, on the whole circuit. The step is encoded by the cover of its cones with cuts of up to four leaves,
  as `itp::bmc` encodes a clock with cuts of up to six, each cut a gate over its leaves that a justifying solver
  justifies by a product of its value (`Solver::set_cover_gate`), where every And node was a gate: on the 260 designs of
  the track the competition decided, 121 decided against 115, run beside it, in 39% less CPU on the 114 both decide, and
  cuts of up to six leaves 122 in 28% more CPU than four on the 119 both decide. As the search begins and at each frame
  it opens it asks whether the bad states are inductive one clock deeper than the frames already free of them
  (`Options::induction`, the deepest asked, `Options::induction_conflicts`, the conflicts a depth may take), in one
  solver unrolled forward over each clock's backward cone: a yes is `Verdict::Induction(k)`, held to
  `itp::check_induction` on the whole circuit. `mc_check` asks to depth 4 within 100,000 conflicts unless `--induction`
  says otherwise, the depth that decided more than 8 or 32: on the model-checking arena 130 decided against 121 without
  it, nine more safe and none fewer.
- **`itp::bmc`: bounded model checking.** The circuit unrolled a clock at a time into one incremental solver, the
  constraints asserted at every clock and each clock's bad states asked for under an assumption, so the first clock that
  answers ends a shortest run; a run is returned only once it replays. The unrolling makes a node at a clock only once a
  constraint or a bad state asked about reads it, through any number of clocks back: a latch at clock `t + 1` is its
  next-state signal at clock `t` and one with an initial value that constant at clock 0, so the constants fold through
  the logic; and the cones are covered by cuts of up to six leaves (Eén, Mishchenko & Sörensson, *Applying logic
  synthesis for speeding up SAT*, SAT 2007; priority cuts chosen by area flow, a cut's area the clauses its function
  takes), each cut's root one variable defined by the irredundant sums of products of its function and of its
  complement, its constant leaves cofactored away and its repeated or complementary ones merged, one function over the
  same variables one variable at every clock. `mc_check --engine bmc --shortest` runs it: on the HWMCC'24 bit-level
  track at 60 s of CPU it finds runs on 42 of the 319 designs, each as the competition's solvers did, against 41 for
  ABC's `bmc3` given each design as AIGER 1.9 describes it, every one of them among ours, in 399 s of CPU against ABC's
  500 on the 41 both find; run beside an unrolling of the cone of influence with a variable and three clauses for each
  And node, on those 73, it finds 43 against 27. `itp::bmc_windowed` asks a window of clocks a query (the window rIC3's
  `bmc --step` asks): one solve asks whether a run is bad at any clock of the window, the constraints holding up to its
  bad clock, so a deep run needs no refutation of each clock before it, though the run need not be the shortest.
  `itp::bmc_interleaved` takes the two in turns, each turn within a conflict budget that doubles every round: a window
  of 64 clocks for a cone whose window unrolls at most a million nodes of the cover, and a clock at a time alone for a
  larger one, whose window costs more to unroll than the clocks it saves. `mc_check --engine bmc` runs it unless
  `--shortest` or `--window <n>` says otherwise: 45 runs against 42 a clock at a time alone, among them the arbitrated
  data-integrity designs' runs of more than forty clocks, and 41 for ABC's `bmc3`.
- **`localize`: PDR on localization abstractions** (McMillan & Amla, *Automatic abstraction without counterexamples*,
  TACAS 2003; Eén, Mishchenko & Amla, FMCAD 2010). A large circuit is model-checked on the latches a proof needs, the
  rest made inputs free at every clock, so an invariant or a k-induction of the abstraction is one of the circuit, held
  to `itp`'s checks on the whole circuit. The latches are found by bounded model checking with each one tied to its
  initial value and its next state by a literal of its own (`itp`'s tied unrolling): each clock is asked under the ties
  of the latches kept, and a run of the abstraction there is asked again under every tie, where a model is the circuit's
  own run and a refutation names the latches it rests on, cut down by asking again under those and the latches kept
  while that names fewer, the abstraction keeping them. Once the latches kept stay the same over eight clocks, PDR
  decides the abstraction; a run of it the circuit cannot take has the tied unrolling settle as many clocks as the run
  took, growing the abstraction by what their refutations rest on, and a cone of at most a thousand latches is decided
  by PDR whole. `mc_check --engine loc` runs it, and the model-checking arena races it against ABC: on the 260 designs
  of HWMCC'24 the competition decided, 138 decided against 125 for PDR alone, run beside it, among them the
  189,026-latch cv32e40x properties p355 and p128 on 94 and 88 of their cones' 6,017 latches and thirteen ILA processor
  checks; with cones of a thousand latches decided whole, 147 of the track's 319 designs against 114 for ABC's `pdr`
  given each design as AIGER 1.9 describes it. `Options::correspond` decides a cone reduced by its signal correspondence
  (`scorr`) first.
- **`scorr`: signal correspondence** (van Eijk, *Sequential equivalence checking based on structural similarities*, TCAD
  2000; Mishchenko, Case, Brayton & Jang, *Scalable and scalably-verifiable sequential synthesis*, ICCAD 2008, the
  scheme of ABC's `scorr`). The latches and the And nodes no input reaches, each a function of the state alone, are put
  into classes of signals alike or complementary on random runs from the initial states, a run's clock counted while its
  constraints held at every clock before; the classes are then refined until two questions put to `fraig` find no
  difference: whether every initial state satisfies every equality, no constraint assumed, and whether every successor
  of a state under a step whose constraints hold does, the step's first clock the circuit with every signal its
  representative. The classes left hold at every clock of every run whose constraints held before it. `scorr::reduce`
  merges each signal into its representative, the inputs and the latches in place, and `scorr::invariant` takes an
  invariant of the reduced circuit, with the equalities, to one of the circuit, held to `itp::check_invariant`; a search
  that would sweep more than `Options::nodes` nodes gives up. `localize` decides a cone reduced by its correspondence
  when the reduction leaves at most nine tenths of its latches, a k-induction of the reduced cone that does not check on
  the cone itself decided again for an invariant, and `mc_check --engine loc` reduces every cone so unless
  `--no-correspond`: on HWMCC'24, 149 of the 319 designs against 147 without it on the same cores, in 40% more CPU on
  the 147 both decide, against 115 for ABC's `pdr` given each design as AIGER 1.9 describes it; the competition's best
  within 60 s of CPU over all its threads, rIC3, decides 122 (185 within 60 s of real time).
- **`mc`: model checking by a race of certified engines, deterministic.** Property-directed reachability and bounded
  model checking search one circuit, cut to its cone of influence, side by side on threads of their own, each in slices
  (`pdr`'s search resumed in slices is the search run whole, `itp::Bmc` asked a clock at a time) that a word from the
  other can end, the first to answer stopping the other. The answer never depends on which finished first: a safe
  circuit is answered by PDR's invariant, the one PDR alone finds, and an unsafe one by BMC's run, the shortest there
  is, BMC asking clocks on up to the length of PDR's run when PDR found one first. `mc_check` runs it by default. On the
  HWMCC'24 bit-level track at 60 s of wall time it decides 135 designs (103 safe, 32 unsafe), every verdict as the
  competition's solvers gave it, against 120 (103 safe, 17 unsafe) for PDR alone on one thread run beside it.
- **`atpg`: test generation for single stuck-at faults, certified.** A full-scan circuit's inputs and latches are what a
  test sets, its outputs and next states what it observes. Random patterns go first, 64 to a word, every fault left
  simulated from its node forward only where a value differs from the circuit's; each fault still left gets a miter of
  the circuit against a copy with the fault (`atpg::miter`), a model a test simulated before it counts and then against
  every fault left, a refutation an untestable fault with an LRAT proof checked against its miter, a solve past its
  budget an aborted fault, never an untestable one. Each miter is solved by justification, the circuit's gates and the
  faulty copy's as gates and the differences as side clauses, and the inputs a test's search left open are filled at
  random, so that the test detects more of the faults left (`max` in 3.4 s against 8.7 s). `Report::check` replays every
  verdict, and `atpg_check` runs it from the command line. On the EPFL combinational suite at 300 s of CPU it settles
  every fault of 14 of the 20 circuits, every verdict replayed (`mem_ctrl`: 96,080 faults, 106 of them proven
  untestable, in 133 s); the six others run out of time.
- **`rewrite`: cut rewriting by exact synthesis, aware of the logic the graph holds, proven.** The graph is edited in
  place a node at a time in topological order: the node's cuts of up to four leaves enumerated, each cut's function
  taken to its NPN class, and each of the class's circuits weighed in the node's place by the nodes the replacement
  frees (its fanout-free cone above the cut) less what it costs, the gates the graph does not hold, those it holds
  among the freed and the freed nodes they keep, so that a gate the graph holds elsewhere is free; the node is replaced
  at once by the circuit of most gain. The circuits of the 222 classes of four inputs, every least circuit found up to
  sixteen and up to eight of one gate more, 3,784 in all, were found by SAT-based exact synthesis ahead of time and
  ship as `src/rewrite_npn4.txt`; every class of up to seven gates is proven least (`Report::minimal` counts those a
  rewrite met), and a test regenerates the library and refutes each least size it claims one gate down. The result
  and the input side by side (`rewrite::side_by_side`), every output, bad state, constraint and next state paired, are
  decided by `fraig::decide`, and `Report::check` replays the derivation; a result that does not check is not
  returned. On the EPFL combinational suite it leaves 425,711 And nodes of 462,884, each result also equal by ABC's
  `cec`. Against ABC's `rewrite` run four times it leaves fewer on 17 circuits and as many on 3 (437,182 in all), and
  against the same without its level preservation fewer on 15, as many on 4 and more on `div` (435,952); against the
  ten-command `resyn2` script, fewer on 4 (`adder` 893 against 1,019, `voter` 9,035 against 9,756), as many on 3 and
  more on 13 (424,633).
- **A search that pauses and resumes.** `Solver::begin` starts a search and `Solver::resume` continues it for a budget
  of conflicts, or until a flag `Solver::set_stop` gave is set: a search resumed in slices is the search run whole, by
  the same conflicts, decisions and model. `solve_with` is the two in one call.
- **`count`: exact model counting.** DPLL with unit propagation, decomposition into connected components, a component
  cache and clause learning; counts of any size. The formula stays put while it counts: an assignment is a trail, each
  clause keeps how many of its literals are true and how many false, and propagation walks the occurrence lists of
  what it assigns, so a branch costs what it assigns and taking it back the same. A component is found by a search
  over its open clauses, which counts each variable's open literals for the branch on the way, and is known in the
  cache by its variables and the numbers of those clauses, which name its residual formula exactly. A component
  branches on its variable a min-degree elimination of the formula's primal graph eliminates last, the separators of
  the tree decomposition the elimination makes, so a branch splits the formula where it is thinnest. A branch that
  propagates to a falsified clause teaches its first unique implication point's clause (GRASP); a taught clause
  propagates, watched by two literals, but makes no component and enters no key, and the clauses the first branch of a
  component taught are asserted for the second. The formula implies every taught clause, but one crossing two
  components can prune the component counted beside a sibling with no model, so when a part of a split comes out 0
  after a taught clause acted, the cache withdraws every count it took in since the split began (Sang, Bacchus, Beame,
  Kautz & Pitassi's Cachet, SAT 2004). Before the search the formula is preprocessed by the simplifications that keep
  its count: units, equivalent literals, congruence closure, subsumption, and elimination of the variables the others
  define, a gate's output or a variable a local check shows decided, taken only where it makes no clause, so the
  primal graph only loses edges; the variables settled leave the search. A circuit with no constraint has its gates
  eliminated from its outputs in: 400 random AND gates over 30 inputs count 2³⁰ models without a decision, where
  branching took 1.1 million decisions and 28 s. `count_check` counts DIMACS files from the command line; on the first
  track of the 2024 model counting competition it counts 36 of the 200 formulas within 60 s of CPU each, every count
  agreeing, against 32 with clause learning alone and 30 with neither, run back to back on each formula; branching on
  the elimination order took an earlier run of the same search from 12 to 33.
- **`preprocess` keeps a count.** `Options::defined_only` eliminates a variable only when the others decide it, a gate
  defining it or no assignment of the others satisfying the rests of both its positive and its negative clauses on the
  part of the formula around it (Lagniez & Marquis, AAAI 2014), and with `factor` off the simplified formula then has
  as many models over the variables left unsettled (`Preprocessed::settled`) as the input over all of its own.
  `Options::renew_costs` keeps the elimination schedule current, so a circuit's outputs go first and each gate follows
  once its last reader is gone. The defaults are unchanged: on the SAT Competition 2024 sample a renewed schedule
  eliminated more on some formulas and less on others, at up to 1.7 times the cost.
- **`eda_bench`: the EDA arenas.** Model checking on HWMCC'24 (`mc_check --engine loc` against ABC's `pdr` on each
  design read as AIGER 1.9 describes it, `logic; undc; strash; zero; fold`, every latch free to start anywhere given its
  first value by an input of its own, one thread each, each verdict scored against the competition's consensus, and the
  competition's own solvers counted within the same limit from its results table, in real seconds and in CPU seconds
  over all their threads), bounded model checking on its designs the competition did not find safe
  (`mc_check --engine bmc` against ABC's `bmc3`, read alike), logic rewriting on the EPFL suite (`rewrite_check` against
  ABC's `rewrite` run four times and `resyn2`, each result of ours proven by `fraig` and checked by ABC's `cec`), and
  exact model counting on the 2024 model counting competition's first track (`count_check`). Each run is a subprocess
  limited and timed in CPU seconds, `--jobs` at a time or one on each CPU `--cores` lists and pinned there (`EDA_CORES`
  in `run-eda.sh`), so a hybrid machine's run takes the CPU seconds of the kind of core it was given, and each arena
  writes `benchmarks/results/eda/<arena>.json` with the date, the CPU, the cores, ABC's version and how ABC read each
  design; a verdict of ours that contradicts the consensus, or a rewrite ABC's `cec` finds unequal, fails the run.
  `benchmarks/eda/run-eda.sh` builds and runs all four.
- **`cipher`: reduced-round ciphers as CNF.** A bit-level `Circuit` builder that folds constants and shares equal
  gates, with XOR of any width as one gate, majority, choice, rotation and addition modulo 2³²; SHA-1's compression
  function cut to any number of its 80 steps (`sha1_compress`, `sha1_circuit`) and Trivium's setup and keystream
  (`trivium_setup`, `trivium_keystream`), each held to its published vectors; and planted instances, a preimage of the
  cut compression function with some block bits given (`Sha1Preimage`) and Trivium's state recovered from its
  keystream with some state bits given (`TriviumRecovery`), each model checked by recomputing the cipher.
  `cipher_instances` writes them as DIMACS and `benchmarks/arena/gen-crypto.sh` makes the cryptanalysis arena of them.
  On its 36 instances at 60 s of CPU each — SHA-1 cut to 17 to 20 steps, Trivium from 288 and 320 keystream bits,
  three seeds a rung — the route of `solve` decides 23, every model checked, Kissat 22, CaDiCaL 21 and CryptoMiniSat
  19 (PAR-2 1788.0, 1766.6, 1899.4 and 2134.1); the 20-step and 320-bit rungs stop every engine.
- **Search options for A/B**: a move-to-front decision queue in focused mode (`set_focused_queue`), chronological
  backtracking (`set_chrono`, Nadel & Ryvchin: an implied literal at the highest level of its reason, the second
  watch kept on that level, a conflict analyzed at its own level or assigned when it was a missed implication).
- **`ReducePolicy::Tiered`**: Kissat's learned-clause reduction (usage counters, glue promotion on use, tier limits
  read off the uses, a `√k` conflict schedule), beside the default `ReducePolicy::Halving`; `set_reduce_policy`,
  `reductions`.

- **Craig interpolation and interpolation model checking on the crate's own solver.** `interpolate`: McMillan
  interpolants read off an LRAT refutation of `A ∧ B` — each step's hints replayed as the resolution chain they stand
  for, partial interpolants joined by disjunction on `A`-local pivots and conjunction otherwise, the result a
  structurally hashed and-inverter graph over the shared variables; a proof that does not refute is refused.
  `itp`: unbounded safety of AIGER circuits by interpolation, bad-state properties (or outputs), invariant constraints
  and free initial values honoured; a counterexample replays by simulation and an invariant is accepted only once its
  three obligations are established by checked LRAT proofs, each encoded over the logic it reads — the constraints'
  cones, the next states of the latches the invariant reads, the bad states' — and the first two by deriving the
  invariant's root as a unit, asked whole within a conflict for every four of its conjuncts and past that conjunct by
  conjunct in one incremental solver: a PDR invariant of 1,086 clauses over 51 latches (HWMCC'24's sw_ball2004_2) checks
  in under a second where its step took 34 s, and one over a 189,026-latch circuit in about one where the whole
  circuit's encoding took 37. `check_induction` decides k-induction by two checked refutations over the clocks' backward
  cones. Locked against explicit-state reachability.
- **Kernel-checked SAT refutations.** `kernel_refutation` checks an LRAT refutation as a kernel development: the
  formula's atoms and clauses (as right-nested `Or`) are declared from the formula itself, each clause and step is a
  definition `¬l₁ → … → ¬l_k → False` the kernel checks and the independent kernel re-checks, and the last states
  `False`. Declarations rather than one closed term, so a refutation of any length fits under the kernel's term-depth
  bound. Intuitionistic. `refute` solves afresh; `certify` takes a refutation in hand, so the 2-SAT and Horn verdicts
  and the parity and GF(p) refutations compiled to resolution become kernel theorems too. A PR step is refused: it
  preserves satisfiability but is no consequence of the formula.
- **Partial symmetries.** `Witness::PartialSymmetry` certifies a clause by Buss & Thapen's substitution redundancy —
  every image of the database and of the clause follows by unit propagation from its negation — with no automorphism
  required. `groupoid::ModelGroupoid` is the groupoid of a formula's models under such maps, and
  `sym_certify::conditional_symmetry_breaks` breaks every transposition that breaks only a few clauses, guarded by
  their images and checked against every break before it, so the breaks compose and every component keeps a model.
  The almost-symmetry route uses it and returns its proof.
- **Nullstellensatz over every field and ring, symmetric degree at every scale, resolution width.** `polycalc_gfp`
  (prime fields and GF(4)), `polycalc_zm` (ℤ/m), `orbit_stability` and `res_width`.
- **`solve::certified_proof`**: a proof of an UNSAT verdict checked against the formula alone — the route's own
  stream when it checks, the parity and GF(p) refutations compiled to resolution, a fresh CDCL refutation where the
  search leaned on mined clauses — or `None` for a verdict that rests on its own certificate. 2-SAT refutations carry
  their two RUP units; the covering collapse carries its PR refutation.

### Changed

- `lrat::derivation` and `lrat::refutation` propagate what the formula forces once: the unit-propagation fixpoint of the
  clauses before each step is carried forward as the trace is read and cut back as the backward walk withdraws each
  clause, so a chain propagates only its clause's negation, a literal the fixpoint already holds taken as the negation's
  own. The clauses lie end to end in one arena under watches with a blocker literal, a binary clause decided by its
  watch alone. A random 3-CNF core guarded by a literal its units force, beside 150,000 literals 50,000 units force, has
  its 3,528 learned clauses trimmed in 73 ms where they took 49 s, and the 8,867 a model checker's 1,086-clause
  invariant step learns in 1.8 s where they took 34 to 44.
- `fraig` decides the EPFL combinational suite, each circuit against its `resyn2` optimization, in 6.32 s of CPU against
  210.02 s for ABC's `&cec`, every derivation checked: all 20 circuits, 18 of them faster and `mem_ctrl` and `bar`
  within 0.02 s (`hyp` 2.19 s against 176.15 s, `log2` 0.97 s against 17.21 s, `mem_ctrl` 0.62 s against 0.60 s), where
  it decided 19 in 322.79 s, `hyp` not within 300 s. The simulation holds its values word by word and its classes in
  groups, the nodes alike on every word but the pending one, each group a tree splitting on the pending word's bits: a
  counterexample simulates one word of each node and looks only at the classes of more than one node (`Stats::simulated`
  counts the words), where it simulated every word and rebuilt every class. The solver decides by justification, so a
  model is the part of the pair's cones its assumptions need, where it assigned every variable the solver held, and a
  counterexample draws the inputs its model leaves open, where they read false (`mem_ctrl` takes 1208 counterexamples
  where it took 3453); a model is checked on the pair's cones alone, where the whole miter was evaluated; and
  `Options::recycle` is a million variables, where a hundred thousand replaced the solver every few queries on a circuit
  whose cones are larger.
- The CDCL core stores every clause in one flat arena (its length, its index, then its literals) under eight-byte
  watchers, a long clause's watcher holding its arena offset and a binary one its index, and keeps a value per
  literal: on random 3-SAT at the threshold the same search runs in half the cycles.
- `Solver::original_clauses` returns the clauses copied out of the arena.
- The CDCL route of `solve` lets an elimination add up to four clauses beyond those it removes, where it added none.
  On an exact cover, every variable in four exactly-one constraints, no variable eliminated at bound 0. On the SAT
  Competition 2024 sample at 60 s the route decides 57 of 62 formulas against 47 (PAR-2 1049 against 2320), among
  them all ten of its tilings, nine of which it did not decide; the SATLIB and structured arena is unchanged.
  `cdcl_stats` takes the bound as `CDCL_BOUND` and reason-side bumping as `CDCL_BUMPREASONS=1`.
- The CDCL route of `solve` preprocesses the formula first and returns the preprocessing steps followed by the
  learned clauses as its proof; a model is extended back and checked against the formula. It no longer adds
  `mine_clauses`'s bundle: read off the input, its Gaussian clauses ran a dense elimination and spoke of variables
  preprocessing had eliminated, and it lost wherever it changed a run — on the SAT Competition 2024 sample at 60 s
  the route's PAR-2 is 2507 without it against 2592 with it (1009c7… 5.5 s against a timeout), on the SATLIB and
  structured arena 70 of 80 against 69.
- `preprocess` counts a unit of effort for every clause a subsumption pass examines and keeps each clause's
  signature, so a run stops when its effort is spent however wide the clauses are (af750c…: 72.7 s to 11.5 s), and
  finds repeated input clauses by sorting rather than hashing. Each pass is held to a multiple of the formula's
  literals besides, so one pass cannot spend the run's effort and starve the rest: subsumption had spent all of it on
  1f44da… and found nothing, before factoring or elimination ran.
- The PR checkers, `rup::check_refutation` and the DRAT, DPR and SR emitters accept proofs naming variables past the
  formula's — the variables a PR step introduces, as bounded variable addition's do — each step still checked against
  the clauses that name its variables. LRAT and kernel-checked refutations stay over the formula's variables.
- `symmetry_detect::find_generators` searches as nauty does: cell-indexed equitable refinement, the first path, then
  each level's alternatives deepest first with orbit pruning, a branch pruned when its partition's shape departs
  from the first path's — a complete generating set, every generator still re-verified as an automorphism.
- Adaptive restarts space the Luby restarts of their stable phases at 1024 conflicts a unit.
- `arena_bench` limits and times each run in CPU seconds by default (`ARENA_CLOCK=cpu`: `prlimit --cpu` and the
  child's rusage), so engines rank alike on a loaded machine, and races CryptoMiniSat beside Kissat and CaDiCaL;
  `cdcl_stats` reports the raw core's search counters and throughput with A/B switches for each heuristic, and with
  `CDCL_PREFIX=1` the seconds and route of the dispatcher's specialist chain.
- **The dispatcher's specialist chain costs what the formula's structure allows.** On the SAT Competition 2024
  main-track sample it took from a second to over four minutes before the CDCL route began, and past four minutes on
  four instances Kissat decides within one: quadratic or worse in every place below.
  - `xorsat::eliminate` (behind `solve` and the new `rank`) eliminates sparsely, Markowitz-fashion — the column in
    fewest rows pivoted on its shortest row — and finishes a filled-in core on dense bit rows; a refutation is read
    by replaying the additions backwards from the `0 = 1` row, the assignment by back-substitution. Dense
    Gauss–Jordan over every variable was the cost on formulas with thousands of equivalence and XOR-gate gadgets
    (351 s of 1009c7's chain). `lyapunov::gaussian_lyapunov` reads its potential off the same elimination.
  - `xorsat::xor_gadgets` finds the XOR gadgets by sorting clauses on their variable sets; `lyapunov::extract_xor`
    and the parity recognizer (`xorsat::refute_via_parity_clauses`, which `refute_via_parity` now wraps) share it,
    and the equations come out in one canonical order instead of a hash map's.
  - `lll::lll_certifies_sat` counts dependency degrees only up to the largest the local lemma admits, declining from
    the occurrence counts alone when a variable is in too many clauses — one pass on any formula with a short clause —
    where it formed every clause's neighbour union through hash sets (269 s of 170b13af's chain).
    `lll::moser_tardos_witness` re-examines only the clauses sharing a resampled variable and resamples the same
    clauses in the same order.
  - The matching and cutting-plane recognizers run only on a covering, every clause all-positive or a binary
    exclusion — the only formulas they read — so the formula's `ProofExpr` view is no longer built for every input.
  - `lyapunov::covering_collapse` stands only when its breaks refute the formula by unit propagation, the descent's
    level 0; a residual that needs search declines onward, where it was searched to its end inside the chain
    (0ef029…, symmetric under an item swap but no covering: over 100 s before the CDCL route began). Measure
    synthesis tries an item swap only when it keeps every variable's signed occurrence counts.

### Fixed

- A solve with restricted decisions (`Solver::set_decision_vars`) answered SAT once every decision variable was
  assigned, a variable left to propagation that propagation never reached taking false: a model that could violate a
  clause. Such a variable is now decided.
- `xor_engine::IncXor` synced its matrix to the trail by variable, not by literal, so a variable assigned the other way
  at the same trail position after a backtrack kept its old value. `XorEngine` and `IncXor` read a variable named
  twice in an equation as named once; it cancels, as `xorsat` reads it.
- Compacting the clause database read the stale reason of an unassigned variable, past the end of a database an
  earlier compaction had shrunk (an out-of-bounds panic on `hanoi5`); an unassigned variable now holds no reason, and
  a clause that is the reason of a trail literal is never deleted.
- The 2-SAT, Horn, parity, GF(p)/ℤ/m, SoS and Nullstellensatz routes re-check their certificates in release builds
  and decline onward when one does not check; before, the 2-SAT and Horn refutations, the parity combination, the SoS
  multipliers and the Nullstellensatz verdict were not re-checked, and the GF(p)/ℤ/m combinations only in debug builds.
- `logos-sat` and `largo sat` write only proofs checked against the formula: a CDCL refutation that leaned on a
  Gaussian-derived clause is re-derived rather than left unexportable, and a verdict with no clausal proof gets a note
  saying it rests on the route's own certificate, not a claim of internal verification.

## [0.11.0] - 2026-11-02

### Added

- **Combinational and sequential equivalence, certified.** `aig`: and-inverter graphs with structural hashing,
  AIGER ASCII and binary read and write, 64-way simulation, cones, `compact`/`compact_keeping` (only the nodes
  an output, a property, a latch or a kept literal reads), and `sequential_cone` (the outputs asked for with only the
  latches that reach them through any number of clocks). `sweep`: two circuits joined by `miter` (inputs shared and
  outputs paired by name, kept to the outputs' cones), swept by simulation classes into lemmas each a refuted
  difference whose LRAT proof `check` replays, a lemma first put on a depth-6 cut, a depth-16 cut and the cut both
  cones meet at before the whole cone; `equivalent` decides every output pair with its certificate, `separating_input`
  finds one input that tells two circuits apart. `distinguish` tells a family of circuits apart: the members that
  compute one function, each equality certified, a greedy cover of input assignments on which every two classes
  differ, and each class's outputs there (`Distinction::check` replays all of it). A decision's `Stats` count every
  query by the cut it was put at (`CutCost`: queries, verdicts, variables, clauses, conflicts) and how much of the cone
  the sweep has covered; `Options::progress` hands them to a report every so many candidates. `seq_equiv`: `unroll`,
  `at_latency` (a pipeline's outputs at clock L over the clock-0 inputs, every initial state free), `aligned` and
  `latency_equivalent`, and `always_equal` by register correspondence with an `InductionCertificate` and bounded
  model checking for a difference from reset. `lrat`: watched-literal LRAT hints, backward-trimmed, and a structured
  checker; `derivation` trims a log back from any target clauses, and `check_derivation` replays one and finds every
  claimed clause derived. `aig_equivalence_lock`, `rtl_equivalence_lock`.
- **`fraig`: equivalence by incremental SAT sweeping, the scheme of ABC's `&cec`, certified.** One solver answers every
  query under assumptions within a conflict budget, holding only the cones asked about, defined over their fan-ins'
  representatives, and numbered as it meets them; only the nodes the pairs reach are swept and simulated, so a miter
  decided a few pairs at a time costs their cones, not the whole; nodes merge by structure with no solver call, and a fresh solver
  takes over once one holds `Options::recycle` variables. Every clause made carries its LRAT hints as it is made: a
  learned clause its conflict analysis's chain, a solved merge its refutation's, a structural merge or definition the
  chain propagation finds. What nothing a later clause can name rests on is let go as the log grows (`Options::collect`)
  and with each replaced solver, and the derivation is what the claims reach back to (`Report::check` replays it
  against the miter's own formula). `seq_equiv::{aligned_by_fraig, latency_equivalent_by_fraig}` decide a pipeline by
  it. On the exp_p re-cut (141,443-node miter) it proves 32 of 32 outputs in 38 s, its derivation checked in 0.3 s.
- **`units`: equivalence over units proven apart.** Two circuits with their units cut out (results free inputs,
  operands outputs), each unrolled as `Unrolled` says (every named input bound per clock: shared with the other side
  by a name, a constant, or fresh; output bits claimed at the clocks each side makes them), `decide` the case of a
  pipeline read at its latency. They are decided in rounds: a unit's function is its name on its side and the
  function `functions` pairs it with on the other, and the applications of one function whose operands fraig proves
  equal are identified, on one side or across (functional consistency, a pipelined unit's latency matched), a pair
  refuted leaving its group's other applications to be tried; the outputs are then decided over the units as
  functions nothing is known of — a difference there is unknown, never a verdict. An input with no name (an undefined
  bit) is a fresh one of its own side at every clock, and a latch with no name starts from a fresh value of its own.
  Every round's miter, derivation and identification is re-checked (`Report::check`, `check_unrolled`); an
  application made one twice, or one with itself through others, does not check.
- **`streams`: stream equivalence over units.** Two valid-ready circuits at their own widths and paces
  (`StreamSide`, `Ports`), each under an arrival pattern (`Environment`: the clocks a token is offered and taken,
  every other input per clock). Each side's handshakes are proven constants with the data and the units' results
  free (`schedule`: a `Schedule` whose derivation `check` replays, its clocks in token order), so a side whose ready
  or valid moves with the data is refused by name (`StreamError::DataDependent`); then each side's j-th input token
  is read where that side takes it (`Ports::token` names the symbols a token's bits are, so a side twice as wide reads
  words 2j and 2j+1), each k-th output token is claimed where it is given, and `decide` proves the tokens equal over
  the units (`Report::check` replays both schedules and every round). A schedule unrolls only the handshakes'
  sequential cone, so a datapath they never read costs it nothing.
- **`cdcl::Solver`, incremental and proof-producing**: `solve_under_assumptions_budgeted` (a conflict budget, Luby
  restarts, learned clauses reduced without touching the originals), `add_vars`, `add_decision_vars`, clauses added
  between solves placed against the level-0 facts, and online LRAT (`set_lrat`, `add_clause_with_id`,
  `set_next_clause_id`): every learned clause's hints recorded as it is learned, a level-0 fact named by its unit
  clause (derived once), `unsat_hints` for the clause of an unsatisfiable solve's negated assumptions, and
  `take_learned` / `live_ids` for a caller that keeps its own log.
- **`cycle_ratio`**: the maximum cycle ratio of a graph whose edges carry a delay and a register count, by Howard's
  policy iteration per strongly connected component, exact as a ratio in lowest terms, with the potential that
  certifies it (`check`); a loop of no registers is returned as the loop.
- **`min_cost_flow`**: network simplex (block-search pricing, a strongly feasible tree), the optimum certified by
  complementary slackness, infeasibility by a cut and unboundedness by a negative cycle (`check`).
- `tactic_script::canonical` is public: it maps a tactic word, synonyms included (`Suppose`/`assume`/`let` → `intro`), to the engine's own name, so an editor highlights exactly the words the engine accepts.

- **A Lean-shaped tactic vocabulary, in the API and in `Proof:` scripts.** Every step still ends in one kernel-certified derivation.
  - **`ProofState`:** `exact_term`, `refine_term` (one subgoal per `?_` hole; a bare hole is refused), `have_term`, `apply_named`, `rfl`, `by_contra` (certified through `ClassicalReductio`, so the axiom footprint records `dne`), `ring`, `linarith`, `calc` (named steps; the relation may change along the chain), `by_rule`, `cases_with` (component names chosen by the proof; a name already in scope is refused), `induction_on` (an inner variable is generalized first), `rewrite_at`, `rewrite_hyp`/`rewrite_hyp_at`, `named_simp_set`, `value_term`, and `intro` on a `¬P` goal. `tactic::PTerm` is the script term.
  - **Combinators:** `tactic::combinators` gains a value for each, plus `any_goals`, `focus` (an error when the block leaves its goal open), `simp_only`, `simp_with`, `exists_term`, `refine` and `have`.
  - **Script grammar:** `tactic_script::parse_script`, which a `Proof:` line runs, takes a term (names, application by juxtaposition, `+`/`-`/`*`, `(a, b)` tuples, `?_` holes) as the argument of `exact`/`exists`/`have`/`refine`, and reads `apply h`, `rfl`, `ring`, `by_contra h`, `intro a b c`, `cases h with a b`, `rewrite h at <k>` and `rewrite h at <hyp> [<k>]`, `simp only [h₁, h₂]` and `simp [h]` (names checked), `calc h₁ h₂ …`, `by_rule <Rule>`, `have h from <term>`, `induction n`, and `all_goals`/`any_goals`/`focus`. A connective filler word is skipped only when another word follows it, so `exists A` and `cases it` parse.
  - **Nothing unchecked reports success:** `by_rule` certifies its node through `verify::ScopedChecker` before it succeeds, a Leibniz step abstracts the motive position the kernel will check, and the eigenconstant-renaming walkers are exhaustive over rule payloads (`InferenceRule::map_payload`).
  - Tests: `crates/logicaffeine_tests/tests/T_tactics_extra.rs`, `crates/logicaffeine_tests/tests/tactic_surface_extra.rs`, `crates/logicaffeine_tests/tests/tactic_script.rs`.
- **Inference rules for positional rewriting, order totality, term witnesses and ground refutation.** `InferenceRule::RewriteAt { from, to, occurrence }` abstracts one occurrence in the `Eq_rec` motive, so `P(a, a)` can become `P(b, a)`. `LeTotal` certifies to `le_total`. `ExistentialIntroTerm` carries a `ProofTerm` witness (`use (n + 1)`). `GroundRefutation` proves `¬P` for a closed `P` that computes false through the `Bool` discriminator, with no new axiom. `LeSuccLt` is declared and certifiable against `le_succ_lt`, which the prelude does not register, so no producer builds it.
- **Rule introspection for proof producers.** `InferenceRule::{uncertifiable_reason, is_certifiable, NAMEABLE, from_name, canonical_name, map_payload}` and `DerivationTree::uncertifiable_rules`. The seven modal and temporal rules with no certifier arm report the live-rule translation, and `certifier::certify` refuses them with that reason instead of a generic "not implemented". `crates/logicaffeine_tests/tests/automation_extra.rs`.
- **`#print axioms` for a certified proof.** `VerifiedProof::axiom_footprint()` returns `verify::AxiomFootprint { trusted, introduced }`, read off the kernel term transitively through definitions and checked against a freshly registered `StandardLibrary`: prelude declarations the proof uses are `trusted`, and a premise, a symbol or a smuggled `sorryAx` lands in `introduced`. `crates/logicaffeine_tests/tests/axiom_footprint_complete.rs`; `crates/logicaffeine_tests/tests/axiom_footprint_lean_parity.rs` holds it to Lean 4.14's rule.
- **Scoped checking and premise-variable modes.** `verify::ScopedChecker::{new, certifies}` prepares a scope once and checks many candidate derivations against it; `verify::close_over_context_locals` closes a proof term over the context locals it mentions. `engine::PremiseVariables` (re-exported at the crate root) with `BackwardChainer::set_premise_variables`: a standalone chainer reads a knowledge-base equation as a rewrite schema (`Schematic`, the default), while the trust door and the tactic layer run `Local`, where a theorem's premise is a hypothesis about fixed values, so the search never builds an instance the certificate cannot cite.
- **`@[simp]` lemmas reach the proof-layer simplifier.** `simp::SimpSet::register_context_lemmas(&Context)` compiles every rule registered in a kernel context, and `ProofState::context_simp_set` combines them with the scope; a rewrite enters a certificate only where its lemma is in scope.
- **Gate-level Tseitin and a CNF-in certified refutation.** `cnf::Cnf::{add_clause, atom_lit, encode_and, encode_or, encode_implies, encode_iff}` and `sat::prove_unsat_certified_cnf(num_vars, clauses)` let a producer holding a shared gate DAG clausify it without building the exponentially larger `ProofExpr` tree and still get the same certified refutation; the SVA bit-blaster in `crates/logicaffeine_compile/src/codegen_sva/bitblast.rs` is the first consumer.
- **The algebraic refutation layer is reusable.** `polycalc::{poly_mul_mono, close_perm_group, gf2_solve, apply_perm_to_mono}` are public, and `polycalc::NsCertificate::coeffs()` exposes the Nullstellensatz witness polynomials.
- **`satcli::main()`: the whole `logos-sat` process, which the binary now is.** `crates/logicaffeine_proof/src/bin/logos-sat.rs` is that one call, and so is the test crate's `logos-sat-under-test`.
- **Exact linear algebra over ℚ, and kernel-certified solves.** `linalg::{solve, det, lu, inverse, rank, certify_solve, residual_is_zero, …}` decides `A·x = b` as an exact identity with an independently re-multiplied residual. `linalg_certify::{certify_system, prove_solve, prove_solution, prove_lu, prove_inverse, prove_nonzero}` emits a kernel-checked `Eq Int` term per row after clearing denominators, within the i64 cleared-integer width. `lapack_provable_lock` proves the stdlib `gesv` solve exactly; `crates/logicaffeine_tests/tests/linalg_induction_kernel.rs` proves trace additivity for every `n` by kernel induction.
- **XL linearization and a Rainbow reference.** `macaulay` enumerates and multiplies monomials. `xl::xl_solve` over `xl::{Gf2, Gf256}` returns a solution only after checking it against the original system. `rainbow::{keygen, sign, verify, forge_system}` is a two-layer Oil-and-Vinegar signature over GF(2⁸). `crates/logicaffeine_tests/tests/rainbow_logos.rs` verifies its signatures with the LOGOS `rainbowVerify` on the tree-walker, the VM and native code.
- **Independent replay checkers for physical claims.** `interval_replay::{check_range_claim, check_reduction_claim, check_integrator_claim}` replays claimed analog node bounds with outward-rounded `f64` intervals (no clipping, no noise-floor starvation; any NaN rejects). `relax_replay::{check_relax_claim, check_minimize_claim, check_ising_claim}` judges a settled readout by its exact rational residual. `thermal_replay::check_thermal_claim` reassembles the Laplacian from primitive edges and demands `G·ΔT = P` exactly. All three are dependency-free and fail closed. `analog_range_certificate_lock`, `analog_relax_solve_lock`, `thermal_map_lock`.
- **`revgate`: reversible-gate certificates.** `certify_gate` accepts an `n`-bit unit iff its truth table is a permutation, and `certify_linear` accepts a CNOT/XOR block iff its GF(2) matrix is invertible; `cnot`, `toffoli` and `fredkin` are the reference tables. `crates/logicaffeine_tests/unit/logicaffeine_proof/revgate/tests.rs`.
- **`tactic_cert` re-exports the kernel's tactic certificates.** `logicaffeine_kernel::tactic_cert` holds the LIA, ring and congruence certificate emitters and the checkers the second kernel replays. `crates/logicaffeine_tests/tests/tactic_cert_integration.rs`.

### Breaking changes
- **Seven public modules are gone.** They are listed under Removed. Migration: pin `logicaffeine-proof = "=0.10.1"` for them; code that rebuilt on their GF(2) core can use `polycalc`'s now-public `poly_mul_mono`, `close_perm_group`, `gf2_solve` and `apply_perm_to_mono`.
- **`InferenceRule` gained five variants.** `RewriteAt`, `LeTotal`, `ExistentialIntroTerm`, `GroundRefutation` and `LeSuccLt`; the enum is not `#[non_exhaustive]`, so an exhaustive `match` stops compiling. Migration: add the arms, or route through `InferenceRule::map_payload` and `uncertifiable_reason`, which already cover every variant.
- **`linarith` no longer means `omega`.** In 0.10.1 `linarith` was an alias of `omega`, which tightens `a < b` to `a + 1 ≤ b` — sound over ℤ, unsound over an ordered field. `linarith` (alias `rationals`) now runs rational Fourier–Motzkin with a Farkas certificate and declines a goal that needs integrality, so an integer goal such as `x < y, y < x + 1 ⊢ ⊥` fails under it. Migration: write `omega` (or `arithmetic`/`integers`). `crates/logicaffeine_tests/tests/omega_tactic.rs`.
- **`arith::prove_int_eq` returns `None` in a context whose `add`/`mul`/`sub` are not the checked integer operations.** Migration: build the context with `StandardLibrary::register`, as `linalg_certify::ring_context` does.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.
- **The RUP checker reads a clause without allocating.** One clause rule, `rup::clause_state` (a conflict, the unit a
  clause forces, or quiet), serves `rup::propagate` and the LRAT emitter's hint search and step check, where each
  collected a clause's distinct unset literals into a fresh vector on every pass; a clause is read only to its second
  distinct unset literal. The verdicts are the collected rule's, and wide bit-vector equivalence certifies 15x faster:
  `phase_hw_native_vs_z3`'s sixteen- and thirty-two-bit identities in 32 s, from 500 s.

- **Certification asks the standard library's one registration.** Every certifier entry (`prepare_ctx_with_defs`, behind `prove_development`, `prove_library_with_axioms` and the bounded certifiers), `VerifiedProof::axiom_footprint`, the tactic and search engines' prelude questions and `linalg_certify::ring_context` borrow or clone `StandardLibrary::context()` instead of registering the prelude; the opening of Tarski's geometry, eight theorems, registered it eight times, and its debug run now takes 23.1 G instructions instead of 133.8 G.
- **`omega` proves goals, not only contradictions.** It decides a ground `≤`/`<` by computation, a `≤` by chaining in-scope facts, and an `Int` equality through the ring oracle (`⊢ n + 0 = n` with no hypotheses). Any other `≤` goal is split on `le_total a b` with the right branch refuted, so no new axiom is needed. A strict symbolic conclusion is declined with a message naming the missing `le_succ_lt`. `crates/logicaffeine_tests/tests/proof_order_axioms.rs`.
- **The ring oracle computes with exact integers.** `arith::prove_int_eq` keeps coefficients as `BigInt`, decides closed goals by checked kernel normalization at any literal width, sorts monomial products constructively at any degree and reaches subtraction through `sub_elim`. The ring laws it cites have proof bodies in the prelude, so its proofs have empty transitive axiom dependencies. `crates/logicaffeine_tests/tests/symbolic_bigint_closed_proofs.rs`, `crates/logicaffeine_tests/tests/arith_identity_certified.rs`.
- **Order chains mix `=` and `≤`.** An equality is a two-way edge in the `≤` graph through positional Leibniz, so `a = b, b ≤ c, c ≤ d ⊢ a ≤ d` composes, and `calc` E-matches its named `∀`-equality steps at the terms the chain mentions.
- **`formula` reads nested function terms.** `Eq(Mul(a, b), Mul(b, a))` parses its arguments, and an equality with an arithmetic side is a ring identity rather than an uninterpreted `Eq`.
- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG, so neither `tests/` nor `examples/` is in the tarball (`cargo run --example solver_bench` needs a checkout); the unit-test bodies live in `crates/logicaffeine_tests/unit/logicaffeine_proof/` behind one-line `#[cfg(test)] #[path]` stubs and the integration suites in `crates/logicaffeine_tests/tests/`. Without its tests, examples and research modules, the tarball is 0.79 MB (1.99 MB in 0.10.1). The declared `rust-version` is 1.88 (0.10.1 declared 1.75, below what its own dependency graph needed).

### Removed
- **Seven research modules, reachable from no product code, left the crate.** `category_collapse`, `census`, `cofactor`, `res_width`, `orbit_stability`, `polycalc_gfp` and `polycalc_zm` were public in 0.10.1.

### Fixed

- `unify::instantiate_expr`: a binder instantiated once and capture-avoiding; every quantifier instantiation in
  `tactic`, `engine`, `crush`, `simp` and `certifier` goes through it. `apply_subst_to_term` refuses a cyclic
  substitution by name instead of recursing until the stack is gone.
- A goal may be any conjunct of a rule's conclusion (`unify_with_conclusion`, `project_goal`); the ⊥ search remembers the fact sets it failed on (`FalsumMemo`) and skips case splits over premises the all-true interpretation satisfies or a Horn set propagation left open, so a refusal that took 194 s is immediate; an existential goal may be witnessed from its context (`collect_witnesses`).
- The engine no longer drops self-named guards (`dog(Dog)`) from rule antecedents and goals: the certifier kept them, so the engine proved rules stronger than the premises and the certifier rejected the proofs. Engine and certifier read every premise alike.
- A definite description presupposes its noun. Every lowercase constant that is also a noun (`butler`, "the butler"), meaning a predicate the problem never applies to more than one argument, yields `butler(butler)`. The engine takes it as a premise (`BackwardChainer::prove`/`prove_with_goal`, from the premises, the goal and its context, for that proof only), and the certifier registers it as a hypothesis (`verify`). Both read it from one symbol walk, `symbols::SymbolCollector`, moved out of `verify`. Proper names and nouns also used as relations presuppose nothing.
- The kernel files a predicate that shares its name with an individual under a name of its own (`symbols::KernelNames`, `is_<name>`). This includes a definiendum, whether or not the problem applies it. The renaming is applied to definitions, hypotheses, the goal type and the derivation at the verify door, so `butler(butler)` no longer applies the predicate to itself, and "a butler is one who serves" certifies beside "the butler". The engine and the returned derivation keep their names.
- `exprs_structurally_equal` matches every `ProofExpr` variant exhaustively, and each is equal to itself. A tensed proposition (`Past(see(butler))`) had been unequal to itself, so the ⊥ search never met it against its negation, and a rule concluding a tensed fact could not be refuted through a conjunct (`phase60_proof_engine::test_butler_did_it_theorem`). `Unsupported` stays equal to nothing.

- The certified solver validates a complete root-propagation model against every original clause before symmetry discovery. Duplicate literals retain unit-clause behavior, implication chains propagate to a fixed point, and inconclusive or conflicting inputs keep the SDCL/CDCL refutation path.
- **The certifier gives a term one sort across a statement.** `SymbolCollector` typed each occurrence on its own, so `f(3) = 3` declared `f : Entity → Entity` and then applied it to and equated it with an `Int`, a hypothesis whose type is not a type, which the kernel's assumption door now refuses. Sorts settle to a fixpoint (`settle_sorts`): the two sides of an equation share a sort, an argument position that takes an `Int` takes it at every application, and `register_function` declares each position's domain.
- **`decide` divides the way the kernels do.** `div`/`mod` are Euclidean, total and arbitrary-precision: `div(-7, 2)` is `-4`, and a zero divisor gives quotient `0` with the dividend as remainder. 0.10.1 divided with truncating `i64` arithmetic, answered `-3`, and got stuck at a zero divisor, which called true goals false and passed false ones on to the trust door. A goal that evaluates false is reported false, not undecided, and its negation is proved by `GroundRefutation`. `T_tactics_extra::decide_folds_division_the_way_all_four_kernel_engines_do` in `crates/logicaffeine_tests/tests/T_tactics_extra.rs`.
- **Integer literals past `i64` are numbers in every proof layer.** `decide`, the certifier, the domain classifier and the symbol collector share one numeral reading, so `9223372036854775808 + 1 = 9223372036854775809` is provable; the literal used to become an undeclared `Entity` global. `crates/logicaffeine_tests/unit/logicaffeine_proof/numeral/tests.rs`.
- **A theorem may mention a free variable.** A statement's free variables are registered as kernel locals typed by use: `Int` in arithmetic, the inductive a premise gives its unary predicate, and `Entity` otherwise. An `∃` binder takes the domain its body uses, so `∃m. m = n + 1` states and certifies. The independent re-checker is handed a closed term, so `double_check` no longer reports a disagreement over a valid proof by induction on a free local.
- **A development's definitions certify whatever order their atoms are collected in.** Definitions are registered after every atom their bodies mention, and a definiendum is never also declared opaque; the order of a hash map used to decide whether δ could unfold a definition. `definition_registration_lock`.

## [0.10.0] - 2026-07-08

### Added
- `satcli` module: the SAT command-line driver (DIMACS in, `s SATISFIABLE`/`s UNSATISFIABLE` + model out, DRAT/DPR/SR certificate export, competition exit codes) extracted from the `logos-sat` binary with injected output streams — shared verbatim by `logos-sat` and `largo sat`, and directly testable.
- Modal translation and independent verification.
- CDCL core with an incremental grid solver, grounding, and trust-tiers; label/PP convergence.

### Changed
- **`prove_unsat` — the expensive certified cuts are escalation-only.** The Lyapunov collapse, Nullstellensatz, Polynomial Calculus, and recursive certified-symmetry rungs ran on every call, taxing microsecond equivalence/BMC obligations ~50 ms each. The cheap structural recognizers stay upfront; the complete CDCL search then runs under a conflict budget (`EASY_SEARCH_CONFLICTS`), and only an instance that exhausts it escalates to the certified cuts, finishing on a fresh solver so the refutation certificate stays self-contained. Verdicts unchanged; the native-vs-Z3 speed locks hold again and the hard symmetric families still refute through the escalation path.

### Fixed
- **`cert_farkas` kernel reconstruction on doubled constants** — the proof-producing normalizer had no proof path for like monomials recombining to coefficient 1, and a merge that cancelled an entire prefix concluded with an unproven `add 0 x` residue the kernel rejected. Both close (locked by `arith::tests`), and `probe_double_constant_le_via_auto` runs un-ignored.

See the root CHANGELOG for the cross-crate history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Backward-chaining proof engine implementing Curry-Howard correspondence
- Robinson's unification algorithm with occurs check
- Higher-order pattern unification (Miller patterns)
- Alpha-equivalence for quantifiers and lambda expressions
- Beta-reduction to weak head normal form
- Comprehensive inference rules (modus ponens, induction, etc.)
- Structural induction over inductive types
- Modal and temporal logic support
- Socratic hint generation for pedagogical guidance
- DerivationTree to Kernel Term certification
- Optional `verification` feature for Z3 oracle fallback
- Liskov Invariant: no dependency on language crate
