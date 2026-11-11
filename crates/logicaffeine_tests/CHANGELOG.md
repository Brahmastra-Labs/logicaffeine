# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `dag_sem_spines`: the value engine's neutrals and environments against a model — an application built one argument at a time, all at once, or a prefix and then the rest is one value; its arguments read back in order; it is open exactly when its head or an argument is; finding one stores nothing and a new one stores its arguments once; an environment extended by a value, by another, and by the first again is the first environment; a session reset remembers no extension; a pruning keeps exactly the entries its mask marks, is one frame per content (finding it stores nothing, a new one stores its live entries once), and an environment remembers its two latest prunings.
- `dag_sem_bounds` also holds the caches to the dag: a term holding a subterm twice at every level, 18 deep, is evaluated and checked with the caches no larger than for one 2 deep — a shared subterm's value and type are found the second time (`evaluating_a_shared_subterm_costs_its_dag_not_its_tree`, `checking_a_shared_subterm_costs_its_dag_not_its_tree`).
- `dag_relevance::every_absent_mask_is_the_one_a_walk_of_the_definition_finds`: every definition of `init-prelude` has the absent-parameter mask a walk of its value and type finds, a reference that reads none of the loose-variable masks the store keeps.
- `dag::expr` unit `reserving_room_keeps_every_node_and_then_grows_no_more`: reserving room in the persistent table goes straight to the capacity that holds the reserved nodes, finds every node interned before, and does not grow while they arrive.
- `release_assets_lock` runs `release.yml`'s release-notes step as the runner runs it, against every released section of the changelog and a synthetic one: each is a body GitHub accepts (125,000 characters), a section that fits ships whole, and a longer one keeps every heading, each heading's first entries verbatim and a pointer to the full notes for the rest (0.11.0 keeps Highlights, Breaking changes, Removed and Security whole). `a_dispatch_releases_an_existing_tag_with_the_dispatched_workflow`: a dispatch names the tag, every checkout checks it out, nothing reads the ref's own name, and the release names the tag and publishes it.
- `release_assets_lock::a_release_holds_the_tags_binaries_to_the_dispatching_workflows_size_budgets`: each size-budget step runs `./release-policy/scripts/check-binary-sizes.sh` after a checkout of `${{ github.sha }}` into `release-policy` carrying `.config/size-budgets.toml` and the script, the LSP and largo checks once each; `the_size_check_reads_its_own_trees_budgets_and_the_callers_binaries`: the script, copied into a scratch tree, passes a binary at its budget and refuses one a byte over, its relative `--bin-dir` read from the caller's directory. The dispatch test exempts the release-policy checkout from checking out the tag.
- `release_assets_lock::every_script_a_workflow_runs_by_its_path_is_executable`: every `./path` a workflow's `run:` names is a file of the tree with its exec bit (a script run from the release-policy checkout read at the tree's root), so a script committed without it fails here, not at its step on a runner.
- `compiler_stack_safety_lock`: `the_optimizer_lattice_compiles_loop_interleaved_recursion_in_bounded_time` and `the_rust_backend_compiles_loop_interleaved_recursion_in_bounded_time` (a self-call inside a constant loop, the inliner's n-queens shape, compiles within 300 s on an ordinary thread); the kernel's `lia` units `a_budget_no_step_reaches_gives_the_exact_verdict`, `the_budget_bounds_the_constraints_an_elimination_step_leaves`, `a_system_that_outgrows_its_budget_is_given_up_on_promptly`.
- `ondemand_exclusion_lock::every_card_cosimulation_runs_on_demand_and_no_fast_card_test_does`: the silicon selector names exactly the `d5005_engine_card_lock` tests that start Verilator, derived from the file.
- `docs_site_lock::a_link_written_as_code_in_the_docs_is_not_a_link_the_site_must_ship`: an item summary that shows a link as inline code (`` `[Alias](URI)` ``) builds; `site.py`'s map-link check reads no link inside a code span.
- `size_budget_lock::the_web_bundle_builds_for_size_and_its_budget_fits_the_host`: `[profile.wasm-release]` builds at opt-level `s`, and the `[web]` wasm budget stays under Cloudflare Pages' 25 MiB per-file limit.
- `ci_local_runner_lock`: `scripts/run-ci-locally.py plan` accepts the shipped `test.yml` on every trigger and never runs the job that publishes; it refuses by name an unmapped action, an unknown runner label, a key, trigger, context or expression function it cannot reproduce, and a matrix given by an expression; static `if:` conditions follow GitHub's expression rules; `include` and `exclude` expand as GitHub documents; a dispatch reads its declared inputs and refuses an undeclared one, and a tag ref is a tag; `hashFiles` digests the matched workspace files in path order; a cache step is a cold start; a shell the image lacks is refused only where a step would run it; a macOS, Windows or arm64 leg, and what needs only it, is named as not run; the shipped `release.yml` plans as the dispatch of a tag (the two linux-x64 `largo` legs run here, the publishing job and the installs of the published release never); each runner image installs nothing its workflow installs for itself and no tool the hosted image lacks.
- `workflow_linker_lock`: every job that installs Rust on a Linux runner installs the packages its workflow's link
  line names before its first build, read from the workflow's own `env:`. The workflow readers live in
  `common/workflows.rs`, and `release_assets_lock` reads them from there.
- `release_assets_lock::every_release_job_on_windows_checks_out_the_trees_own_line_endings`: a release job that can run
  on Windows turns off the checkout's CRLF conversion before it checks out. The workflow readers' `script` (a step's
  shell, comments left out) lives in `common/workflows.rs` beside the others.
- `release_assets_lock::the_vsix_gate_launches_vs_code_by_the_name_its_app_gives`: the VSIX gate installs a
  `@vscode/test-electron` of at least 3.1.0 between its dependencies and itself.
- `real_constants_every_tier_lock`: a bare `pi`, `π` and `e` are the nearest doubles outside an exact context, in
  arithmetic, a function, a closure, a condition, a list and a text, beside the exact reading, and a `Let`, parameter,
  closure parameter, loop variable or arm binding so named keeps its value, on every backend that runs a program.
- `real_text_every_tier_lock`: six `Real`s, written as literals and computed at run time, each shown alone, in a bare
  hole, padded right, left, centred and by a bare width, as a record's field, a list's items and the value of a map
  keyed by text and by an integer: the tree-walker's shortest decimal on every backend.
- `installer_docs_lock`'s `each_installer_ends_on_the_first_run_the_quick_start_proves`: the `get started:` line
  `install.sh` and `install.ps1` print is the README quick start's commands, the first run `getting_started_lock`
  replays with nothing but `largo` on the PATH.
- `dag::sem::value`'s `content_controls` (a slot is half a hash and an id; every entry is found after every growth;
  entries whose hashes share their top half are told apart by content; content is compared only where the kept half
  agrees; an emptied table) and `memory_controls` (the arena's memory report counts every store, its fields
  destructured whole, and names each part once); `dag::sem`'s `memory_controls` (a session's report counts every
  store); `dag::export`'s `size_controls` (each of the Arena's large exports reserves the persistent table its
  expressions need, and the estimate covers it within a tenth).
- `dag_levels::leq_holds_exactly_when_it_holds_under_every_assignment`: the `≤` decision procedure against every
  assignment over {0, …, 4}, on every pair of depth-one levels over two parameters and 30,000 pairs of depth-two ones.
- `dag_level_allocation`: a counting global allocator, by thread: deciding 4,000 orders of levels, finding a known
  substitution and substituting into 3,000 lists allocate only as a memo grows.
- `interface::lean::native_decide`'s `export_controls` (an export without a native decision is read as exported,
  uncopied; only each leaf's line is replaced, by its re-derivation; the kernel core accepts the re-derived export and
  refuses the export as exported) and `pipeline::native_controls::the_kernel_core_rechecks_the_native_result_as_the_logos_kernel_read_it`.
- `dag_quotient_integrity`: a quotient's constants are its own, in every engine of the `dag` core. The two exports of
  Lean Kernel Arena issue #39 (`fixtures/quotient_forgery/`, byte for byte as reported) are rejected by their rules, and
  by the `Term` kernel's importer. Over the Arena's own `130_quotLiftReduction`, each member restated as an ordinary
  axiom in its place, declared before the quotient, or declared again after it is rejected by its rule, as are records
  in an order their canonical types do not allow; the lift before the constructor is admitted. The official package is
  admitted and computes. With one admitted quotient constant restated as an axiom behind the kernel's back, the
  quotient left installed, the lift no longer computes. Under `nbe` the closure-conversion configurations run too
  (`run-opt-in-feature-tests.sh`).
- `dag_crafted` decides every crafted case in the substitution engine, the value engine and both at once, holds
  `fixtures/crafted/` to exactly what `craft-cases.py --out DIR` writes, and `craft-cases.py` builds Lean's `Eq` and
  quotient: a control package whose theorem checks only where the lift computes, an export using `Quot.mk` with no
  quotient declared (admitted, as Lean admits it), and ten forgeries — each member as an axiom, `Quot` as a
  definition, `Quot.mk` declared first, `Quot.ind` before `Quot.mk`, a constructor, an inductive type and a mutual
  block's later member taking a quotient's name — each rejected by its rule.
- `dag_trusted_prefix::a_quotients_names_are_checked_even_inside_the_trusted_prefix`: an axiom, an inductive's
  constructor and a mutual block's later member taking a quotient's name are checked inside a trusted prefix.
- `samba_exam_numerics_lock` (E2.8, E2.11), `spatial_scatter_lock`, `spatial_written_map_lock`,
  `samba_exam_agility_lock`, `samba_exam_fusion_report_lock`, `samba_exam_unit_use_lock`, `samba_exam_recompile_lock`
  and `samba_exam_certificate_lock`; the intake's reader, twin and conformance tests for every new family in
  `model_hf_intake_lock`, `model_tiny_checkpoint_lock` and `model_conformance_lock`;
  `spatial_mixed_precision_lock`'s held cache and island-reads-an-accumulator tests;
  `spatial_one_ir_lock::every_op_family_lowers_from_its_own_file`.
- `timing_calibration_lock`'s `no_build_whose_memory_quartus_built_in_its_fabric_is_calibrated`, `timing_graph_lock`'s
  `a_box_given_its_area_by_its_caller_adds_it_for_every_instance_as_a_declared_one_does`, `d5005_rtl_lock`'s
  `the_timing_model_boxes_every_memory_the_generator_writes_its_parts_included` and the timing graph's unit
  `a_box_that_takes_an_area_says_so_and_one_that_takes_none_writes_nothing_more`: a memory Quartus built in its fabric
  is no calibrated build, a box takes the area its caller gives it, and every memory the RAM generator writes is boxed.


### Changed
- The Arena corpus at lean-kernel-arena `8139f284`: 247 cases, the 11 new ones (six `bugs/` rejects, `other/inductive-undeclared-universe` and four `corner-cases/`) vendored by `scripts/lean/arena-crown/vendor-corpus.py` from the official archive or from the corner cases rebuilt byte-identical at `d7f74a9e`; a regenerated export is now looked for in every round held, newest first, and `cedar`, re-exported at every round since, keeps its regenerated export with the new digests in its evidence. `arena_corpus_lock`, `arena_entry_lock`, the Arena README, the rehearsal and the scoreboard pin the round.
- `arena_entry_lock` holds the Lean Kernel Arena entries to the kernel's `unchecked-access` feature: each entry builds its checker with it, the kernel declares it, and `the_measured_snapshot_is_built_as_the_entry_builds` compares `snapshot.sh`'s `cargo build` arguments with the entry's as well as their `RUSTFLAGS`, so a measurement cannot run a build the Arena would not. An entry trains its profile on accept tests it exports itself with the Arena's `lka.py build-test`, each through its own run line, either into the profile directory or into a profile written apart and merged with an explicit weight (`every_build_trains_its_profile_on_the_run_line_over_accept_cases`, which requires each export before its training and the merge to weigh exactly the profiles written apart); `the_entries_train_on_the_same_tests_with_the_same_weights`; and `the_measured_snapshot_trains_as_the_entry_trains` holds `snapshot.sh`'s training inputs and weights to the entry's.
- The logic suites read a quantified plural as its singular's predicate (`logic_determiner_readings`,
  `logic_infinitary_quantifiers`, `language_L_language_extra`), `spatial_program_path_ledger_lock` holds the probe
  table's 36 paths, the editor's diagnostic rows leave the editor surface to `lower_view_functions`,
  `propositional_logic` reads the Unicode connectives, and `benchmarks/results/speed_floors.json` is re-recorded by
  `speed_floor_lock`'s recorder on main's ledgers: the 44 achievements without a floor, the six vanished
  `clique_coloring/24` floors dropped, and the Lean Kernel Arena pin's counts.
- `collected_heap_lock`'s rows collect at least twice on every collected tier: `record_with_a_list_field` and `text_by_appending` run 300,000 iterations, and `list_grown_by_pushes` hands its list to a variable that outlives the iteration, since a list of twenty pushes built and read inside one iteration never reaches the heap on the C tiers.
- `wasm_host_abi_lock` freezes `read_reals` and checks it on every host (`compile_common/wasm_host.rs`);
  `parity_opt_lattice_lock` scans whole identifiers; `eda_tools_seam_lock` admits `benchmark_remote_run_lock`'s
  tools under a temporary root; the Studio locks read the debounced preview where `studio_workbench.rs` keeps it;
  `H_honesty_extra` keeps only its scratch probes and leaves `per_process_dir_lock`'s hand-made list; `INDEX.md`
  gains its `one_*` and `studio_*` rows.
- The `cuda-gpu` test group, which gives a test the card alone, takes `model_golden_lock` and `numerics_contract_lock`,
  which probe the GPU (`nextest_schedule_lock`); `spatial_int64_lock`'s KERNEL-6 override sits above the generated
  widths block (`test_ledger_lock`).
- The Studio and Learn end-to-end scripts name no file of the private workspace. Locked by `workspace_reads_lock`.
- `benchmarks/results/speed_floors.json` drops the 28 Lean Kernel Arena floors named for a test's former best rival
  (Tristen, 11-07): the Arena pin of 9447fcc83 changed the field's best checker on those tests, and each margin is
  floored under the new rival's name (`arena/bugs/large-elim-param/over-kefir`, `arena/mathlib/over-nano-lean`, …).
  `arena/navier-stokes-euler/over-sokonanoda` stays: on that test the pin's field has a faster checker than ours.
- `benchmarks/results/speed_floors.json` drops `arena/navier-stokes-euler/over-sokonanoda` (Tristen, 11-07): at the
  Arena pin of 9447fcc83 nano-lean checks that test in fewer instructions than ours, so there is no lead to hold;
  `arena/navier-stokes-euler/instructions` still floors our own count.
- `page_host::lowered`'s unit tests: a failure no handler absorbs, and one the page absorbs, are driven through a
  replayed turn instead of a mount (a mount is no longer a guarded entry, Tristen 11-07), and
  `a_mount_that_fails_is_the_statements_failure` pins the mount's own failure.
- `arena_entry_lock`: each entry's description and the kernel README credit every checker the core builds on, with its
  link, and the entry's run line, `largo lean check` and the `logos-kernel` binary check through `dag::arena::run`.
- `arena_scoreboard_lock` holds every measured test to its peak-memory ceiling as well as its instruction ceiling
  (`logos-arena-ceilings/3`).

### Fixed
- `bringup_lock` follows the chips it tests: the SN50 brings up an expert mixture (it deploys SambaNova's published families), the D5005 refuses it naming "experts"; the undescribed checkpoint is written with no metadata (`tiny_models::write_bare_checkpoint`), since every tiny checkpoint now carries its keys.
- `compile_determinism_lock`'s children write each program's renderings to their own file as they make them, and the parent compares the two processes a program at a time: the whole-corpus row held every rendering of the corpus in each child and both children's in the parent (about 45 GB a process, OOM-killing the release gate); a process-dependent rendering is still caught (275 of 303 in the sample row).

## [0.11.0] - 2026-11-02

### Added

- `license_lock`: LOGOS is MIT everywhere the tree authors. `LICENSE.md` is the MIT License word for word under
  Datacenter.Dev's copyright line; every published crate and the VS Code extension carries a byte-identical copy and
  nothing else copies it; every `Cargo.toml`, `package.json` and `package-lock.json` root declares MIT; every SPDX
  identifier we author is MIT; each third-party tree (SoftFloat, the RISC-V ISA tests, the Lean arena cases, the
  OpenLane overlay, two fonts) states its own license and the README's license line names it; and no authored file
  outside the CHANGELOGs speaks of the Business Source License, the verification license key or reserved rights.
  `license_lock::the_scan_sees_each_banned_phrase_and_only_where_it_stands` holds the scanner to what it claims.
  `legal_pages_lock::the_terms_leave_the_software_to_the_mit_license` and
  `ssr_render::every_footer_releases_the_site_under_the_mit_license` hold the terms and the footer to MIT.
- `benchmark_harness_fixture_lock::a_machine_that_goes_quiet_after_any_number_of_busy_checks_is_quiet`: `quiet_box_wait` (`benchmarks/lib/suite.sh`) answers what the gate answers, after any number of busy checks and past its 120 s retry cap, and a machine busy past `BENCH_QUIET_WAIT` is not quiet. Past the cap, the wait reported a gate that had passed as "never went quiet".
- `ssr_render::the_benchmarks_page_renders_no_under_the_hood_panel`: `/benchmarks` renders its page with the Flamegraph PDF download and no Under the hood panel; the benchmarks page's `the_page_offers_exactly_the_language_solver_and_codec_tabs` (unit): its tabs are the programming language, the SAT solver and the transport codec, none of them Tenstorrent's.
- `qwen4_exp_intake_lock` (QWEN4-1): Qwen3.8-Flash-Next's twin (`hf-tiny-twin.py qwen4exp`: four lanes, QSA over blocks
  of four with a budget of eight, the n-gram embedding on layer 2, the sigmoid gate, Qwen's mixture, the tower and the
  draft block, its layer types as Qwen publishes them) — `…::the_spec_carries_every_configuration_key` (and as
  transformers saves the configuration), `…::every_tensor_of_the_table_is_bound` (every key and tensor as the converter
  writes it, a computed exponential within one f32 step, nothing unread, and the reference's step reading every tensor
  but the draft block's), `…::the_reference_equals_transformers_on_the_tiny_checkpoint` (positions across an end of
  document and past the budget, the prompt whole and a token at a time) and
  `…::every_part_the_author_does_not_write_is_refused_by_name` (and by `largo emit rdu`).
  `model_hf_intake_lock::every_published_configuration_reads` reads Qwen3.8-Flash-Next's. `hf-oracle.py logits-at` and
  `common::hf_twins::transformers_logits_at`: transformers' logits at chosen positions of one pass.
- `qwen38_real_dims_lock` deploys Qwen 3.5 and Qwen 3.8 through the generic path:
  `…::the_tiny_bundle_equals_the_vm_and_the_references` (the `qwen35` twin), and
  `…::every_real_width_layer_kind_lowers_and_certifies` and
  `…::every_real_width_mixture_layer_kind_lowers_and_certifies` for every release (Qwen3.5-0.8B to 397B-A17B,
  Qwen3.8-27B and 2.4T-A95B), each held to its own `config.json` by `…::every_size_is_its_published_configuration`.
  `deploy_reds_lock::qwen_3_5s_mixtures_tiny_bundle_equals_the_vm_and_the_references` (handed, NUM-1): the mixture's
  twin is the reference's at fp32 and routes as it does, and lands 2.54% of the largest logit off on the bf16
  datapath, 1.34% with the SIMD path's fp32 witness. `model_hf_intake_lock` reads every Qwen 3.5 and Qwen 3.8
  configuration (`…::every_published_configuration_reads`) and Gated DeltaNet's gate by its activation
  (`…::a_gated_deltanets_output_gate_is_read_by_its_activation`).
- The `qwen35moe-causal` twin, Qwen3.8-2.4T's layout (the text model alone under `model.`, untied, its experts stored
  as one gate-and-up and one down tensor a block: `hf-tiny-twin.py`'s `fuse_experts`):
  `model_hf_intake_lock::a_qwen35moe_text_model_reads_as_its_converted_gguf` and
  `qwen38_real_dims_lock::the_text_model_layouts_tiny_bundle_equals_the_vm_and_the_references`.
- `spatial_moe_real_width_lock::{a_router_row_past_one_accumulator_computes_the_block,
  at_kimi_k2_geometry_the_choice_lowers_and_only_the_chosen_experts_move,
  at_qwen4_exp_geometry_the_top_ten_of_512_lowers_and_only_the_chosen_experts_move}` and `spatial_moe_options_lock`'s
  wide-row option, routing and choice cases (`wide_data`); `deploy_kimi_glm_lock::every_kimi_k2_layer_kind_lowers_and_certifies`,
  landed from `deploy_reds_lock`.
- `model_hf_intake_lock::every_published_configuration_reads` (`scripts/accel/fetch-hf-configs.sh`, tool `hf-configs`)
  and its handed REDs (`model_hf_config_reds_lock`: MXFP4, int4 compressed tensors, the multimodal checkpoints);
  `…::a_tiktoken_vocabulary_encodes_as_tiktoken_encodes`, `…::every_class_of_kimi_k2s_pattern_matches_what_tiktoken_matches`,
  `…::kimi_k2s_probe_names_its_pre_tokenizer` and `…::kimi_k2s_vocabulary_is_its_published_ggufs`
  (`scripts/accel/fetch-gguf-heads.sh`, tool `gguf-heads`; `hf-oracle.py tiktoken`, `tiktoken-classes`);
  `deploy_kimi_glm_lock::kimi_k2s_tiny_bundle_equals_the_vm_and_the_references` (the `kimi_k2` twin) and
  Kimi K2's real width (`deploy_kimi_glm_lock::every_kimi_k2_layer_kind_lowers_and_certifies`).
- `model_quant_lock::fp8_block_scaled_dequantizes_as_the_reference` (the oracle: `hf-oracle.py scaled`, the converter's
  own `dequant_simple`) and `…::a_scale_the_converter_would_not_broadcast_is_refused`;
  `model_hf_intake_lock::an_fp8_block_scaled_directory_reads_as_its_converted_gguf` and
  `…::a_quantization_the_reader_does_not_read_is_refused_by_name`; the `deepseek2-fp8` twin.
- `deploy_kimi_glm_lock` (GLM-4.5's half); `model_hf_intake_lock::a_glm4moe_directory_reads_as_its_converted_gguf`,
  `…::a_rotation_share_and_original_context_are_read_from_the_rope_parameters_first` and tokenizer parity for Qwen 2.5,
  Mixtral, MiniMax-M2, Phi-4, GLM-4 and GLM-4.5; `model_tiny_checkpoint_lock::a_glm4moe_checkpoint_reads_back_as_its_spec`;
  the `glm4moe` twin with its draft block and GLM-4.5's tokenizer; `common/samba_deploy.rs::Size` takes a size's own
  QK norm.
- `deploy_glm_lock`; `model_hf_intake_lock::a_glm4_directory_reads_as_its_converted_gguf` and
  `model_tiny_checkpoint_lock::a_glm4_checkpoint_reads_back_as_its_spec`; the `glm4` twin and GLM-4's tokenizer in
  `scripts/accel/fetch-hf-tokenizers.sh`; `common/samba_deploy.rs::Size` takes a size's own attention bias.
- `deploy_phi_lock`; `model_hf_intake_lock::a_phi3_directory_reads_as_its_converted_gguf` and
  `model_tiny_checkpoint_lock::a_phi3_checkpoint_reads_back_as_its_spec`; the `phi3` twin (Phi-4's shape) and Phi-4's
  tokenizer in `scripts/accel/fetch-hf-tokenizers.sh`.
- `deploy_gemma_lock` (every Gemma 3 size's layer kinds lowered and certified) and `deploy_reds_lock` (the DEPLOY REDs a
  ticket owes, frozen: Gemma 3's twin on the bf16 datapath, NUM-1); `common/samba_deploy.rs::Size` takes a size's own
  score scale and window; the `deploy_*` family's row in `tests/INDEX.md`.
- `deploy_minimax_lock`; `model_hf_intake_lock::a_minimax_m2_directory_reads_as_its_converted_gguf`,
  `model_tiny_checkpoint_lock::a_minimax_m2_checkpoint_reads_back_as_its_spec` and
  `model_conformance_lock::a_qk_norm_across_every_head_equals_the_reference`; the `minimax_m2` twin
  (`scripts/accel/hf-tiny-twin.py`, its routing bias drawn as the parameters are) and MiniMax-M2's tokenizer in
  `scripts/accel/fetch-hf-tokenizers.sh`.
- The Samba book's DEPLOY locks, `deploy_qwen25_lock`, `deploy_llama3_lock`, `deploy_mixtral_lock`, `deploy_oss_lock`,
  `deploy_deepseek3_lock` and `deploy_llama4_lock` (`common/samba_deploy.rs`): a family's twin from its directory held
  to transformers, the reference, the VM and the SN50 bundle, and each published size's layer kinds lowered at real
  width and certified. `common/model_harness.rs` writes latent attention, attention sinks, expert biases, sandwich
  norms and Llama 3's frequency factors, and fills a program from any checkpoint (`filled_from`);
  `common/hf_twins.rs::transformers_logits` runs transformers' forward of a directory; `common/tiny_models.rs` writes a
  fused projection and values unlike the keys in width;
  `samba_exam_bringup_lock::a_fused_projection_and_values_unlike_the_keys_bring_up` (E1.15).
- `tensor_softcap_lock`: `capped at C` and `X capped at C` against the f64 formula on the VM, the tree-walker, C and
  AOT-Rust, alone and with a scale, a window, a causal block and sinks; `±C` in both tails; refusals by name.
  `model_conformance_lock` gains values narrower and wider than the keys, a fused projection (with and without its
  bias, under either rotation), capped scores and logits, and each split over 2 and 3 chips
  (`common/model_harness.rs::filled_split`).
- `tensor_graph_vjp_lock` gains a scaled attention and causal block, a sliding window, a window over a causal block and
  chunks, sinks with their gradient, values wider than the keys, a capped attention and a capped tensor, and every
  clause at once, each against central differences.
- `tensor_gated_delta_gate_lock` (QWEN4-8): the sigmoid-gated step against the f64 formula on the VM (its output and
  memory, five shapes, a block of rows), the tree-walker, C and AOT-Rust; the front end's lift; the graph evaluator
  value for value; the SN50 against the VM; the CUDA lane, the forward, the D5005 engine and Qwen3.8's CUDA
  configuration refusing it by name; a census of every reader of the op.
  `model_conformance_lock::a_sigmoid_gated_deltanet_prefills_and_decodes_to_the_reference`.
- `model_hf_intake_lock::a_multimodal_checkpoints_language_model_reads_as_its_converted_gguf` (`hf-tiny-twin.py
  gemma3-mm` and `llama4-mm`: the text model beside a tiny vision tower and projector) and
  `…::llama4_experts_stored_one_linear_each_are_refused_by_name`; the handed RED for the multimodal checkpoints'
  published configurations lands in `every_published_configuration_reads`, Gemma 3 4B and 12B beside it, and
  `deploy_gemma_lock::every_size_is_its_published_configuration` holds every Gemma 3 size.
- `model_hf_intake_lock` reads Gemma and Gemma 2 (`hf-tiny-twin.py gemma` and `gemma2`: heads wider than the width
  over the heads; a 4-token window on every other block and caps that bind): each directory as its converted GGUF, the
  f64 reference against transformers (a capped model judged at its raw logits' scale), every setting the GGUF cannot
  carry refused by name, and their releases' published configurations. `deploy_gemma_lock` brings both twins up on the
  SN50 and lowers every Gemma and Gemma 2 size, each held to its published configuration. `hf-oracle.py`'s model runs
  at float64 throughout: eager attention, every softmax kept at f64. `common/samba_bringup.rs` judges a bring-up in one
  place (`checkpoint_brings_up_close`), a capped head against the bound of its raw logits; `common/samba_deploy.rs`
  sizes carry their caps (`Size::softcaps`) and scale the embedding by their own `√d`.
- `spatial_softcap_lock` (ARCH-SOFTCAP): a capped attention and a capped tensor lowered whole for the SN50 equal the VM
  within the bf16 datapath's error: a decode row over its whole cache and a causal block, grouped heads, heads 2 wide
  (the sections form) and 32 wide (the flash form, three blocks), with a scale, a window and sinks, and scores past
  ±100 against a cap of 0.5; each cap moves the result. `tensor_softcap_lock` gains the front end lifting each cap into
  its op, the SN50's Gemma 2 shaped decode step (an fp32 twin of the chip within a relative 2⁻¹⁶), every shim without
  a cap refusing it by name while every spatial shim resolves it, the backward pass through both caps, the graph
  evaluator against the VM value for value, and a census of every file that reads an attention's spec.
  `tensor_recognize::qwen`'s unit tests read the cap from its loop written out and no near miss of its form. The
  `softcap` probe joins the backend matrix, its identity pins and the CUDA stage golden.
- `weights_at_run_time_lock`: `readReals` reads the same bits on every tier — the tree-walker, the VM and its native
  tier, C, Rust, RISC-V under qemu and WebAssembly under the node shim; a tensor filled from a file computes what the
  tensor filled item by item computes; a missing file and a file that is not whole values stop the program by name.
  `tier_parity_lock` probes `readReals`, `forge_builtin_compliance_ratchet` freezes 270 builtins, `guide_coverage`
  gives it its home, and `goldens/builtin_wire_ids.txt` appends it.
- `rust_observed_loan_lock` and `rust_nullable_lock` (every `c_nullable_*` program on Rust), byte-exact on
  the tree-walker, the VM and the generated Rust; `logicaffeine_data::opt` unit tests. From the Codex lanes of
  10-01/02: `rust_global_loan_{,copy_,depth_,edges_,readonly_}lock`, `rust_in_out_{collection,projection}_lock`,
  `rust_map_loan_audit_lock`, `c_in_out_field_{collection,direct_write,ownership,values}
  _lock` and `c_in_out_projection_observation_lock`; the `rust_*` family's row in `tests/INDEX.md`.
- `lane_compile_lock::the_rotation_kernels_keep_no_stack_frame`: every rotation kernel any tiny architecture's lane
  instantiates compiles with no stack frame (`ptxas -v`).
- `proof_receipts_lock` and `demo_quality_sides_lock` join the baseline (PROOF-RECEIPTS-FRESH's solver half, DEMO-QUALITY-SIDES);
  both leave `scripts/run-handed-reds-tests.sh`.
- `model_golden_lock::the_gpu_golden_runs_with_llama_benchs_batches`: llama.cpp's GPU golden records llama-bench's
  batch sizes and runs with them.
- `model_golden_lock::a_golden_writes_valid_utf8_where_a_continuation_stops_inside_a_character`: the golden tool's own
  escape, compiled and held to `String::from_utf8_lossy` on truncated, overlong, surrogate and out-of-range input.
- `riscv_aot_lock::every_jump_to_a_stop_stub_is_a_far_jump` and
  `riscv_aot_lock::a_check_stops_a_program_larger_than_a_jump_reaches`: a program whose code runs past 1 MiB links and
  stops on its first statement's zero divisor with the VM's words.
- `xc7_image_flow_lock::the_router_binds_a_clock_pads_path_to_its_global_buffers_before_routing`: the patched
  nextpnr-xilinx binds a clock pad's net to the global buffers it feeds before routing, either bank's input buffer.
- `demo_bringup_run_lock` joins the baseline: `largo bringup --run` is built (DEMO-BRINGUP-RUN); the suite leaves
  `scripts/run-handed-reds-tests.sh`.
- `bringup_lock`: a run through the one program's cuts. The VM generates the f64 reference's three tokens on each of
  the nine tiny architectures, one test each (`bringup_lock::the_vm_runs_the_qwen35_hybrid_to_the_references_tokens`
  among them), held to `tiny_models::ALL` by `bringup_lock::every_tiny_architecture_has_a_vm_run`;
  each CPU tier generating the reference's tokens on the hybrid, every kind of carried state crossing its cuts, one
  test each (`bringup_lock::the_c_tier_generates_the_references_tokens` among them); `bringup_lock::a_chip_that_cannot_run_here_says_why_by_name`;
  `bringup_lock::the_d5005_runs_its_engines_machine_model_when_no_card_is_here`;
  `bringup_lock::a_run_past_its_context_is_refused_by_name`;
  `bringup_lock::a_golden_gives_the_prompt_and_the_tokens_a_run_is_judged_against`; unit `bringup` tests for the
  values file's round trip, the declaration's size, the leading-run agreement, and the Rust cut's manifest cut to the
  runtime it reads with (held to the manifest `compile_to_dir` stages).
- `bringup_lock`: runs of several sequences, one generation program a chip. Each CPU tier generates the f64
  reference's tokens for three prompts of unequal lengths on the hybrid, from one program it wrote once: every
  statement of the decode step that computes, in order, inside its loops, every declaration once, every weight read
  before the first loop, one test each (`bringup_lock::the_c_tier_generates_every_sequence_from_one_program` among
  them); `bringup_lock::the_d5005_machine_model_runs_every_sequence` (a sequence generates what it generates alone);
  `bringup_lock::a_sequence_past_its_context_is_refused_by_name`;
  `bringup_lock::a_golden_gives_every_prompt_with_its_continuation`; the heavy
  `bringup_lock::a_hugging_face_directory_brings_up_as_the_program_its_gguf_is` (the Llama and Qwen3.5 twins: one
  identity, one text and the same tokens on the VM and C from the directory as from its GGUF);
  `demo_bringup_run_lock::a_run_over_every_prompt_of_a_golden_reports_each_sequence`; unit `bringup` tests for the
  step's token and position names and the shown tokens read back a list a sequence.
- `model_scoreboard_lock`: the scoreboard times only a card it has to itself —
  `model_scoreboard_lock::a_side_starts_only_once_no_other_process_holds_the_card`,
  `model_scoreboard_lock::a_side_another_process_joined_runs_again_once_the_card_is_free`,
  `model_scoreboard_lock::a_sides_own_processes_on_the_card_are_not_a_peer`,
  `model_scoreboard_lock::a_card_held_past_the_wait_refuses_and_a_side_that_fails_alone_still_refuses`, against a
  stand-in `nvidia-smi` that refuses any other query as the real one does.
  It times the lane its build reported writing —
  `model_scoreboard_lock::the_lane_timed_is_the_executable_its_build_reports`,
  `model_scoreboard_lock::a_build_that_reports_no_lane_is_refused`, against a stand-in `cargo`.
- `c_observed_loan_lock`, `c_number_compare_lock` and `c_program_binding_scope_lock`, byte-exact on the tree-walker,
  the VM and a release C build; from the Codex lanes of 10-01/02, `c_nullable_collection_flow_lock`,
  `c_nullable_collection_ops_lock`, `c_nullable_map_set_equality_lock` and `c_nullable_structural_equality_lock`;
  `optimize::capture` unit tests for block-scoped free names; `analysis::absence` unit tests for the depths a stored
  `nothing` reaches.
- `c_table_equality_lock`: maps and sets in a list, Int-, Text- and List-valued, byte-exact on the tree-walker, the
  VM and a release C build: insertion order, a removed key, a grown-and-shrunk capacity, a subset, and two maps or
  two sets compared outside a list.
- `hart_rvc_lock`, `hart_isa_lock` (+ the vendored riscv-tests and the hart contract programs), `hart_fuzz_lock`,
  `hart_rtl_lock`, `hart_soc_lock`, `hart_ddr2_lock`, `hart_firmware_lock`, `onboard_compiler_lock` and
  `onboard_boot_lock` (+ `support/{rv_fuzz,hart_soc_sim,ddr2_device,onboard_board,fabric_program_gen}.rs`);
  `tool_preflight_lock::a_rust_target_is_installed_only_when_its_std_is` (`common::rust_target_installed`).
- `c_in_out_argument_lock`, `c_global_write_capture_control`, `rust_callee_iteration_snapshot_lock`,
  `statement_bounds_effect_lock` and `wasm_integer_growth_lock`: mutable arguments returned to the place that lent
  them on C, collection loans keeping absence, a callee mutating an iterated source, bounds checks surviving a call
  that resizes the destination, and unbounded Int arithmetic staying exact in standalone WebAssembly.
- `accel_region_lock`, `accel_c_model_lock` (+ `accel_common`), `accum_bound_interval_lock`,
  `compile_determinism_lock` (+ `support/program_corpus.rs`), `nan_wire_canonical_lock`, and the `abstract_interp`
  hoist-order unit tests; `marshal::tests::wire_floats_xor_fuzz_bit_identical` holds the canonical NaN.
- `tactic_induction`: induction over Nat and over a list whose step binds the goal's own variable; the `unify`
  unit tests for `instantiate_expr` (substitute once, identity, capture avoided, shadowing, atoms untouched) and for
  refusing one- and two-variable cycles.
- `timing_graph_lock::the_stratix10_delays_are_fit27s_measured_paths_in_levels`.
- `fabric_terminal_lock` (+ `tests/assets/terminal/`: the host end-to-end model, the PS/2, top and activity
  benches; `tests/common/gate_level.rs`).
- `lane_compile_lock::{the_embedding_gather_norms_its_row_in_the_same_launch,
  a_decode_layers_query_key_and_value_are_one_launch_with_their_head_epilogue,
  a_projection_feeding_an_add_and_norm_finishes_them_in_its_last_block, every_decode_layer_launches_what_its_census_says}`
  (each real model's decode census, `tests/fixtures/lane_census/`) and
  `cuda_lane_lock::{the_head_epilogue_computes_what_the_rope_launches_compute,
  the_last_block_norm_computes_what_the_norm_launch_computes}`; the lane tests of older rules hold the folds that take
  their launches in. `fusion_class_lock::{a_pointwise_dag_fuses_into_one_region,
  a_row_that_feeds_no_output_and_no_state_is_removed, two_independent_row_ops_over_one_source_fuse_horizontally}`,
  the forward's diamond tests in `cuda_elementwise_fusion_lock`, and two region programs in
  `cuda_fusion_census_lock::every_forward_fusion_computes_what_its_kernels_compute_apart`.
- `lane_compile_lock::{a_decode_mixture_adds_and_norms_in_its_last_down_block,
  a_wide_chunks_expert_combine_folds_into_the_next_add_and_norm, a_decode_attention_quantizes_its_output_in_its_fold,
  a_stepped_delta_rule_norms_each_head_in_its_last_block}` and
  `cuda_lane_lock::{the_mixtures_last_step_norms_as_the_norm_launch_does,
  the_stepped_delta_rules_last_block_norms_as_the_norm_kernel_does}`; the lane tests that read a mixture find it
  inside the fold that finishes it (`mixture_launches`, `Launch::parts`), and `one_ir_lock`'s link-against-plan
  differential sets a folded mixture's schedule aside as it sets a mixture's.
  `spatial_forward_lock::a_pointwise_region_is_one_section_and_one_body`.
- `lane_compile_lock::a_block_format_projection_finishes_its_add_and_norm_in_its_last_block` and
  `cuda_lane_lock::the_block_projections_last_block_norms_as_the_norm_launch_does`.
- `fusion_class_lock::a_gated_attentions_output_gate_folds_into_its_attention`,
  `lane_compile_lock::{a_gated_attention_gates_its_output_in_its_store,
  a_batchs_gated_attentions_gate_their_rows_in_their_store}` and
  `cuda_lane_lock::the_gated_attention_store_computes_what_the_gated_product_computes`; Qwen3.5's attention is held to
  every attention's quantize-in-store checks (`lane_compile_lock::a_decode_attention_quantizes_its_output_in_its_fold`).
  `attn_output_gate`'s hold-backs join `common/fusion_corpus.rs` (`HOLD_BACKS`): a second reader of the attention's
  result or the batch's table, the host shown either, a row set from no attention, a row no set writes, a row set
  twice. `one_ir_lock::the_shared_table_fires_what_the_lane_fires` compares a batch's gated attentions, which the
  lane plans as each sequence's attention and its row's set, as the one op the shared table takes.
- `lane_compile_lock::a_batchs_attentions_and_delta_rules_quantize_their_rows_in_their_store`; the attention-quantize
  test holds a batch's attention to quantizing in its store, and `one_ir_lock`'s batch join takes the gated
  attentions with their quantize fold (`plan::gates_batch_rows`).
- `lane_compile_lock::no_lane_kernel_keeps_an_array_in_local_memory`: every kernel of every tiny architecture's lane,
  under every numerics, its chunks and its batch of 64, compiles to PTX with no local depot — the array a kernel
  indexes by a value known only at run time, which ptxas's own spills never show.
- `lane_compile_lock::an_attention_launch_compiles_only_the_path_it_takes`: every attention launch of every tiny
  architecture's chunks and batch of 64 over a cache of 64 positions names the path its grid and arguments take and
  its run width; `an_attention_in_waves_holds_three_blocks_a_multiprocessor` reads a launch's blocks a multiprocessor
  as its fourth template argument.
- `lane_compile_lock::every_folded_norm_runs_on_a_block_of_its_width`: every `lk_gemv_blk_norm` and `lk_moe_down_norm`
  launch of every tiny architecture's program, the checkpoints 2048 wide in Q4_K and Llama's wide ones in every block
  format plays one of its norm's warps a warp. `a_block_format_projection_finishes_its_add_and_norm_in_its_last_block`
  and `a_decode_mixture_adds_and_norms_in_its_last_down_block` hold the 2048-wide Q4_K checkpoints, whose blocks are
  narrower than their norm, to running each projection or mixture and its norm apart (approved by Tristen, 11-02).
- `aead_official_vectors` (ChaCha20-Poly1305, XChaCha20-Poly1305, HMAC-SHA256 and PBKDF2-HMAC-SHA256 against every case
  of Wycheproof's files and draft-irtf-cfrg-xchacha-03, natively and through `crypto.lg`), `sealed_box_builtins`,
  `aead_builtin_contracts`, `c_random_unpredictable_lock`, `sha2_stdlib_contracts` and `session_constant_time_lock`;
  the vendored `tests/data/wycheproof/{chacha20_poly1305,xchacha20_poly1305,hmac_sha256,pbkdf2_hmac_sha256}.json`.
  The PNP unit tests read the two-region layout, and the PNP, channel and wired suites gain the forgery, ledger,
  restart and replay cases; `qa_secure_frames` and `qa_codec_streaming` pin a sealed frame to its plaintext and tag.
- `pq_official_vectors` (the native ML-DSA-65 and ML-KEM-768 kernels against every case of NIST ACVP's and Project
  Wycheproof's files), `pq_kernel_contracts` (every native kernel's argument contract on every tier),
  `pq_stdlib_contracts` (the same contract in `crypto.lg`, and the handshake's context) and `pq_constant_time_lock`;
  the vendored `tests/data/acvp/{mldsa65_siggen_context,mldsa65_sigver,mlkem768_keycheck}.json` and
  `tests/data/wycheproof/`.
- `d5005_card_runs_lock`, `fusion_map_lock`, `layer_tour_lock`, `sota_contract_lock` (the book-internal slice of
  LEDGER-3), `frozen_reds_lock` and `measured_fields_lock`; the `handed-reds` feature, behind which a ticket's RED tests
  are staged (`fabric_settle_probe_lock`, `fabric_build_recipes_lock`, `fabric_author_model_lock`,
  `demo_quality_sides_lock`, `demo_bringup_run_lock`, `proof_receipts_lock`, `one_ir_lane_identity_lock`) and
  `scripts/run-handed-reds-tests.sh`, which runs them on purpose, green while each suite is red and red the day one
  passes behind its gate; shared helpers `common/fusion_map.rs`,
  `common/layer_tour.rs`, `common/board_settle.rs` and `common/board_census.rs`.
- `model_hf_intake_lock`: a Hugging Face directory reads as the GGUF the pinned converter writes from it — keys, spec,
  tensor directory, dims, values and tokenizer keys — for tiny random twins of seven families (`common::hf_twins`,
  built by `scripts/accel/hf-tiny-twin.py` once per script digest under a file lock) in bf16, f16 and mixed dtypes, and
  for the golden models' published checkpoints (`models/<model>/hf`); the reference's logits are the same from either
  source; the tokenizer port encodes, splits and normalizes as the `tokenizers` library does
  (`scripts/accel/hf-oracle.py`); the Unicode tables are their generator's; an 812 GB checkpoint is never read whole.
  New tool probes `hf-twins`, `hf-published` and `unicode-data`.
- `samba_papers_lock`: every bibliography entry of the Samba book is pinned (venue, year, locator, capture, digest),
  every capture matches its pin, and every number a sentence or table row attributes to a source is in the source's
  pinned text or derived by arithmetic the book writes out; tool probe `samba-papers`.
- `qwen38_flash_next_facts_lock`: section 7 of the Samba book against Qwen3.8-Flash-Next's own tensor table — every budget,
  every tensor owned by a component the book names — and `model::ngram_hash` against transformers' module
  (`scripts/accel/qwen4exp-oracle.py`) and the checkpoint's stored hash constants; tool probe `qwen4exp-headers`.
- `spatial_int64_lock`: every 64-bit operation `spatial::wide_int` lowers equals `u64` arithmetic on every input
  class and seeded random inputs, every lowering's proof checks and a tampered one does not, the edge cases (products
  near 2^63, a modulus by 1, moduli near 2^31) hold, and the n-gram hash runs exact on 32-bit lanes.
- `samba_cells_lock`: the Samba book's cell ledger (`benchmarks/silicon/samba_cells.json`, `common/samba_cells.rs`)
  keeps section 2.4: a modeled row never wins, a won row carries its quality gates and is ahead of every rival on its
  slowest run, a Plasticine cell is won only by a simulated row and an RDU cell only by a measured one, our rows carry
  their program identity, a cell's Copy bar is its latest published row; every section 2.1 measure is a column with its
  definition and source; every Copy bar's cell id reads under section 2.3.
- `samba_exam_bringup_lock` and `samba_exam_bringup_reds_lock`: the Samba book's exam E1, cold bring-up of unseen
  configurations. Each row's checkpoint is written to disk, handed to `largo emit rdu --model <checkpoint>`, the bundle
  read back off disk, re-certified and run for one decode step with every tensor filled from the checkpoint as a host
  fills it, its token held to the f64 reference's (`common/samba_bringup.rs`; a row over several shapes names every
  shape that fails). The lock holds the rows that pass: every published vocabulary (and GPT-2's unpadded 50,257), one
  and all KV heads, a Llama from its Hugging Face directory alone, hidden widths of 2,880 and 7,168. The reds suite
  (behind `handed-reds`, listed in `scripts/run-handed-reds-tests.sh`, frozen in `fixtures/frozen_reds.tsv` with the
  harness functions they judge through) holds the rest: every published head dimension, head counts no 16-socket width
  divides, 61 and 1 layers, tied, untied and biased heads, an unknown architecture refused naming what is missing, the
  bring-up clock of every Tier A model, and an exported PyTorch graph.
- `rdu_bringup_lock`: every tiny architecture brings up on the SN50 at its own width
  (`every_tiny_architecture_brings_up_on_the_sn50`, moved green from `rdu_bringup_reds_lock`, DeepSeek-V2 included), and
  every architecture the spatial lowering takes (Qwen3, Llama, a tied Llama, Qwen3-MoE, Qwen3.5, Gemma 3, gpt-oss) brings
  up on the SN50 from its GGUF alone, at its width and four times as wide (the dense ones), its logits within
  the bf16 datapath's bound of the f64 reference and its token the reference's wherever the reference's lead exceeds
  that bound (`common/samba_bringup.rs::brings_up_close`); layer kinds found by program digest, a one-op difference two
  kinds, every layer its own kind, interleaved dense and expert layers, storage that differs by layer, rotary tables
  every layer's. `rdu_bringup_reds_lock` (handed REDs, frozen): every tiny architecture at both widths.
- `spatial_attention_window_lock`: a windowed attention keeps the last `W` positions of each row and an attention with
  sinks puts each head's sink in its softmax's denominator, on the SN50 as on the VM, in the sections and flash forms.
- `spatial_moe_options_lock`: gpt-oss's and DeepSeek's expert mixtures (each bias alone, the clamped SwiGLU, the
  weighting without renormalizing, all together) compute the VM's mixture in the flat and the blocked form, held to
  the program-path ledger's wiring and chip-precision rule, on inputs where each option moves the answer.
- `spatial_latent_tiles_lock`: the latent-attention family at whole-tile widths — tile head slices and joins, the
  per-head GEMV, the absorbed step with keys wider than values past an accumulator page — computes the VM's values,
  and DeepSeek-V2-Lite's step lowers whole and certifies. `tensor_moe_options_lock` gains
  `each_expert_bias_alone_is_lifted_into_its_own_slot`; its and `tensor_moe_weighting_lock`'s accelerator tests now hold
  the spatial IR to lowering the options whole on the SN50 (the CUDA forward still refuses them by name).
- `spatial_column_blocks_lock`: a flat row write (float and integer, a literal and a named row), a join and a head slice
  whose widths are not whole tiles and whose results are wider than an accumulator page run an item per block of the
  result and compute the VM's values; the tiny int8 transformer's decode step lowers whole on the SN50 and shows exactly
  what the VM shows. `one_ir_identity`'s spatial pins re-recorded for the 19 artifacts these lowerings changed.
- `spatial_disaggregation_lock` (`common/samba_handoff.rs`): a model's work shared between chips — the decode bundle
  states the state it carries; a prompt prefilled on the GPU target decodes on the SN50 bundle and the reverse; a model
  split over two chips hands its hidden state on; a hand-off that breaks the contract is refused by name. The SN50's
  own bf16 prefill and decode hold handed RED rows of NUM-1.
- `spatial_rope_window_lock`: a partial rotation from a named or a literal position reads its table row through a
  window when the row is not whole tiles, and heads that are not whole tiles turn as one flat stage blocked by columns,
  each the VM's on the SN50.
- `qwen38_tensor_parallel_lock`: `twenty_four_heads_shard_over_sixteen_chips` and `…_in_a_batch` — Qwen3.8-27B's heads,
  one KV head on every chip and fewer Gated-DeltaNet heads than chips over 16 chips: every head on exactly one chip,
  balanced, with the heads it reads; the step and the batched step the VM's unsharded ones; one certified program over
  the 16 chips emitting their tokens. `a_fusion_never_spans_two_chips_nor_holds_a_fold` holds `TensorParallelShim` to
  its contract.
- `model_triage_lock`: every tiny family under another architecture name is named by its tensors (Llama's and Qwen3's
  alike), a checkpoint no family reads names its nearest reading with what it lacks and leaves unread, a reading the
  checkpoint cannot satisfy names what it needs, every architecture an arm reads is triaged, and a triage takes well
  under a second. `samba_exam_bringup_lock` gains the exam's rows E1.3 (every published head dimension) and E1.9 (an
  unknown architecture refused naming what it carries).
- `samba_exam_correctness_lock` and `samba_exam_correctness_reds_lock` (`common/samba_exam.rs`): the exam's section
  10.2 — two runs of one bundle bit-identical and an outlier channel through bf16 sums hold; a sequence alone and in a
  batch of 64, the cache's last position, NaN and infinity propagation and the SN50's drift beside the Wormhole's are
  handed REDs. `spatial_column_blocks_lock` gains a row combined with every row of a block.
- `samba_techniques_lock`: every technique the Samba book's pinned sources describe is a row of
  `benchmarks/samba_techniques.tsv` naming a ticket on the book's board, quoted from its pinned text; one technique has
  one answer; a hardware feature names a chip-facts ticket; going past a source is shown by a lock; every pinned source
  has rows or is named as describing none; the book's summary tables draw on the ledger.
- `tensor_moe_routing_lock`: `moe … by sigmoid` and `weighting the inputs` on every tier, every combination with the
  renormalizing clause and with biases, against the formula; the tensor graph's evaluator; malformed clauses; the CUDA
  forward and lane refusing them. `spatial_moe_options_lock` gains the routings on the SN50 in both forms, a NaN score
  first, and the refused renormalized input weighting. `model_conformance_lock` gains both sigmoid routings and Llama 4
  (chunks, the weightless norm, the temperature, the input-weighted mixture beside a shared expert), prefill and decode
  to the reference, its harness moved to `common/model_harness.rs`. `spatial_nope_attention_lock`: Llama 4's decode step
  lowered whole is the VM's on the SN50 and its fp32 twin.
- `spatial_attention_window_lock` gains chunked-local attention in both forms, from a position and over a literal
  bound; `tensor_attention_window_lock` gains the lane's refusal of a chunk. `model_hf_intake_lock` gains Qwen 2.5,
  Mixtral (read as experts) and the refused Mistral window. `rdu_bringup_lock` gains two bound names never merged into
  one; `spatial_column_blocks_lock` gains a row over every row of a computed block.
- `tensor_moe_grouped_routing_lock`: DeepSeek-V3's routing — the selection bias, the group limit, the clamped
  renormalization, the routed scale — on the VM, the tree-walker, C and Rust against the formula, every combination;
  an even selection bias changing nothing; the tensor graph's evaluator; malformed clauses; the CUDA forward and lane
  refusing it. `spatial_moe_routing_lock`: a DeepSeek-V3-routed model's decode step lowered whole is the VM's on the
  SN50 and its fp32 twin, and an even selection bias leaves the chip's logits bit for bit. `spatial_moe_options_lock`
  gains the flat form's gathers (the tiles moved per table counted, every expert read from its own run, 64 experts and
  Llama 4's small mixture on the twin) and DeepSeek-V3's choice in both forms on inputs where the group limit binds,
  with the clamp on the chip. `spatial_moe_real_width_lock` gains DeepSeek-V3's geometry with its whole choice.
  `model_conformance_lock` gains DeepSeek-V3's routings, alone and together, a DeepSeek-V3-routed model, and the group
  limits the author refuses; `common/tiny_models.rs` writes a selection bias where the gating chooses by one.
- Qwen's gated shared experts and the qwen2moe intake: `model_hf_intake_lock` gains Qwen2-MoE (its directory, its
  tokenizer and the reference over it), the renormalization a GGUF cannot say, and the converter's vocabulary rule, from
  the twins `qwen2moe`, `qwen2-spm` and `qwen3moe-spm` (`scripts/accel/hf-tiny-twin.py`). `model_tiny_checkpoint_lock`
  gains the qwen2moe round trip, llama.cpp's width defaults and the routed scale four arms never read.
  `model_conformance_lock` gains a gated shared expert and a Qwen2-MoE-shaped model against the reference, the gate
  moving the logits, and every host table a step declares being one the host computes. `spatial_moe_routing_lock` gains a
  Qwen2-MoE-shaped decode step on the SN50 and its fp32 twin. `model_triage_lock` reads the arms from `from_source`'s match
  and triages under every one. `common/model_harness.rs` gains `qwen2_moe_spec` and checkpoints with attention biases and
  shared gates; `common/tiny_models.rs` writes the gate and `expert_shared_feed_forward_length`.
  `samba_exam_moe_lock`: the exam's E6.5 (a group limit leaving the best expert out) on the `largo emit rdu` bundle of
  the tiny DeepSeek-V2 routed as DeepSeek-V3; `samba_exam_moe_reds_lock`: E6.1 (a batch of 64 on one expert), E6.4 (a
  tie at the k-th place) and E6.6 (a NaN router row) as frozen handed REDs on NUM-1, each routing as the reference does.
- Qwen3.5-MoE: `model_hf_intake_lock` gains its directory, tokenizer and reference (twin `qwen35moe`, whose draft block
  stores its experts as one gate-and-up tensor), `model_tiny_checkpoint_lock` its round trip, `model_conformance_lock` and
  `spatial_moe_routing_lock` its prefill, decode and SN50 step; `common/model_harness.rs` gains `qwen35_spec` (the small
  Qwen3.5 hybrid, moved from the conformance lock) and `qwen35_moe_spec`.
- Qwen3-Next: `model_hf_intake_lock` gains its directory (twin `qwen3next`, with a draft block), its refusal when read
  raw, and the f64 reference over Qwen3-Next, Qwen3.5, Qwen3.5-MoE, Qwen3-MoE and Qwen2-MoE directories held to
  transformers' own forward at f64 (`scripts/accel/hf-oracle.py logits`); the directory-against-GGUF reference check
  opens both sides as `largo` opens them and covers Qwen3-Next. `samba_exam_bringup_reds_lock` (handed-reds, frozen on
  INTAKE-2): its bring-up on the SN50 from its directory and its GGUF within the bf16 bound (`common/samba_bringup.rs`
  gains `checkpoint_brings_up_close`). `scripts/accel/hf-tiny-twin.py`: the Qwen3-Next twin holds six value heads over two
  key heads, a shape at which the grouped and tiled orders are not each other's inverse.
- `spatial_mixed_precision_lock`: Qwen3-Next's own checkpoint decodes on the SN50's fp32 twin (fp32 in place of bf16
  in every unit) as the f64 reference computes, to fp32's rounding; `common/samba_handoff.rs` gains `Model::open`, a
  checkpoint opened as `largo` opens it.
- `tensor_gdn_row_blocks_lock`: a real-width step with fp32 beside its bf16 products lowers whole, certifies and
  moves its memory once. `spatial_grouped_decode_attention_lock`: with fp32 beside its bf16 products, Qwen3.8's
  decode attention still folds one item per K/V head. `spatial_latent_tiles_lock`: DeepSeek-V2-Lite's latent step
  lowers whole at fp32.
- `tensor_gdn_row_blocks_lock`: a gated delta whose heads are not whole tiles (16, 8, 48 and 80, every memory past one
  tile, a decode step and a block) computes the VM's values on the SN50, and so does one whose memory is within a tile
  but past one accumulator page of its row (Qwen 3.5's four heads of 16; two of 16, three of 12, one of 24).
- `spatial_mixed_precision_lock`: an input a product and a sum read at two widths composes on Wormhole and computes the
  VM's values. `spatial_mixed_precision_reds_lock` (handed-reds, frozen on NUM-1): every island fp32 on the SN50, and
  its decode-step drift within an eighth of Wormhole's datapath on the SN50's geometry.
- `rdu_chip_facts_lock`: every RDU chip fact is its pinned source's figure in the source's unit — capacities read in
  powers of two as the vendor counts, the SN40L's HBM 1.6 TB/s as published, DDR and P2P rates, every disagreeing
  figure kept with its source, every published citation's figure in its pinned text.
- `model_tiny_checkpoint_lock`: a tiny model `common/tiny_models.rs` writes is a whole checkpoint, its GGUF carrying the
  metadata a converter writes (`tiny_models::checkpoint_meta`), so `ModelSpec::from_gguf` reads every tiny architecture
  at both widths, and every shape the bring-up exam varies, back as the spec it was written from.
- `model_host_fill_lock`: what a host fills a program's declared tensors with from the checkpoint alone. `View::declared`
  is the layout `values` reads and the author declares, for every weight of every tiny model; `q8_planes` is the
  checkpoint's bytes and multiplies back to the weight element for element, and refuses a tensor stored in another
  format, blocks that run along the declared columns and rows that are not whole blocks; every rotary table
  `rotary_values` fills turns each position by its frequency, over the checkpoint's factors when the rotation reads them.
- `samba_climb_lock`: the Samba book's experiment ledger (`benchmarks/silicon/samba_experiments.json`,
  `common/samba_climb.rs`) holds a row for every `EXPERIMENT SAMBA` line (`SAMBA_LANES`, tool probe `samba-lanes`;
  `LOGOS_UPDATE_SAMBA_EXPERIMENTS=1` appends the missing ones) and the dead ends migrated from earlier campaigns, each
  pointing at its record.
- `samba_book_lock`: `benchmarks/PREPARE_FOR_SAMBA.md` holds together. Every ticket body has one index line that
  agrees with it and the whole template; every ticket is in one stream and no file has two owners; every dependency
  resolves on this board or the fusion book's and none closes a cycle; every exam test names its lock and ticket, a
  written one sits where its row says (a ticked row's in a suite the baseline runs, an open row's behind the handed-reds
  gate) and every exam suite's test is some row's; every Copy bar cites the bibliography; every cited path exists; the
  preface's counts are the book's own. `SAMBA_BOOK` points a mutation run at a mutated copy.
- `frozen_reds_lock` reads the board of every work-order book (`FUSIONS_WE_NEED.md`, `PREPARE_FOR_SAMBA.md`), and
  refuses a ticket id two books use, so a handed RED may name a ticket of either.
- `fabric_perf_lock::the_fabric_cycle_count_is_read_from_a_counter_the_rtl_drives`.
- `speed_floor_lock`: every speed on record has a floor five per cent short of its best, in
  `benchmarks/results/speed_floors.json` — the compiled lane's decode and prefill rates on the RTX 3060 and its rates
  over llama.cpp, each serving engine and vLLM's batches, the D5005 card's and the Nexys decode boards' tokens a second,
  the Lean kernel's instructions on every Arena case and its margin over the best other checker, and the certified SAT
  prover's times and margins. A re-measured ledger under a floor fails, an unfloored measurement fails, a gain past the
  margin fails until its floor is raised, and recording only raises; `LOGOS_SPEED_LEDGERS` judges a fresh measurement
  in a ledger's place (`scripts/accel/check-speed-floors.sh`).
- `one_ir_identity_lock` (every engine, spatial and forward artifact the one-IR migration could move, pinned by digest
  one family to a fixture under `tests/fixtures/one_ir_identity/`), `one_ir_lane_identity_lock` (the lane's, recorded
  at the start of the lane's freeze window), and `one_ir_lock` (no fusion added outside
  `tensor_graph`: each representation's rewrites held to an allowlist that only shrinks; the shared table under the
  lane's shim fires what `plan.rs` fires and leaves the same launches, rule by rule and launch by launch, on every tiny,
  wide and medium architecture under every numerics, with the epilogue quantize on and off, in the decode step, the
  prefill chunks and a batch; the lane's link builds `plan.rs`'s launches from the optimized graph).
- `tensor_graph_eval_lock`: the one IR's evaluator computes what the VM computes, bit for bit — every tiny cut, every
  op kind, every access and destination, state carried across steps, windows, signed zeros, NaNs and integer overflow.
- `fusion_class_lock`: every class the ledger and the book name is registered with its kind and an exemplar or the
  reason it has none; every fused class with an exemplar, and every whole-block class on the tiny cut that holds it, is
  built by the shared table and means what its parts mean under the evaluator; every rule fires somewhere, can be held,
  and holds back where its dataflow forbids; every numerics fold means what its parts mean.
- `shim_lock`: every chip's shim answers every class by name, with no catch-all arm; every refusal says why; a class run
  as its parts with a refused part is refused whole, naming the part; every optimized graph a chip resolves lifted still
  resolves; the older forward's shim refuses exactly what `compile_to_cuda_forward` refuses, in its words; a chip's own
  rules run where they are linked and can be held; the classes the shared table builds that no chip links yet are listed
  with the ticket that links each, and the list only shrinks.
- `fusion_report_lock`: every compile reports each place a rule could fire and what became of it — fired, refused by the
  chip with its reason, held — in the table's order; holding a rule reports it held where it would have fired and leaves
  the decisions before it alone; the report and its digest are a function of the graph, the table and the chip.
- `common/lane_cuts.rs`: a compiled lane form's cut lifted into the one IR.
- `tensor_rule_proof_lock` (OPT-5): every step of the shared table and of the lane is proven with Z3 on every witness
  `fusion_class_lock` judges, every rule's witness that its dataflow forbids it and its twin, and every lane cut — the
  graph the steps before it make against the graph it makes of that, for every input and every meaning of the ops the
  IR leaves open. `fixtures/tensor_rule_proofs.tsv` pins each obligation by its digest, re-checked without a solver in
  the default suite; the identities the links compute fused classes by (projections by weights side by side, a
  reassociated product, a scale through a product) are proven at every shape up to 3; a range fact is assumed and
  cited; a slice moved past a write to its source and a false identity are refuted. `common/fusion_corpus.rs` and
  `common/lane_corpus.rs` hold the witnesses `fusion_class_lock` and `shim_lock` built alone.
- `shim_lock::the_books_fusion_map_is_every_shims_answers` and `every_class_in_the_ledger_has_a_row_in_the_map`
  (ONE-PROGRAM-FUSION-MAP-LOCK): section 3.4 of `benchmarks/FUSIONS_WE_NEED.md` says what the shims do — a chip family's
  box is ticked exactly when every shim answering for it links the class whole (the CUDA lane, the four spatial
  targets, the D5005 engine; a family with no shim stays open), its line names each answer where they differ and the
  lane's with `fuse_quant` off where that differs, and a row ticks when every family does. The map's 26 boxes that said
  otherwise now say what the shims answer. `tensor_rule_proof_lock::a_term_two_proven_pairs_share_is_one_class_with_both`
  and `a_false_pair_beside_a_true_one_is_refuted`: an obligation is made in finite steps when one term is in two pairs.
- `numerics_contract_lock` (OPT-6): every shim declares its knob point's numerics contract, the D5005 engine at each of
  its real knob points; every scheme a chip can quantize in — the lane's one and two int8 words, the engine's power-of-two codes
  per 32 values and per weight block, one word or two — and every storage format keeps its declared bound on every value
  the reference's own quantizer and the engine's keep (its int8 cache rows, rotated or not, through its own `cache_row`),
  and reaches two fifths of it; the lane's contract follows its
  knobs; on every lane cut a product reads an activation quantized only in the scheme its chip declares, from a quantize
  before it.
  `numerics_contract_lock::every_integer_accumulator_is_proven_wide_enough`: on the integer probes, the int8 decode
  model's step, a product past 32 bits and a product of an operand no range bounds, every chip refuses an integer sum by
  name or the program's ranges keep it within the width it declares.
  `numerics_contract_lock::every_chip_fused_equals_apart_within_its_declared_bound`: under every chip's shim every
  fusion means what its parts mean; the D5005 machine with every vector job taken apart into its parts' jobs computes
  the machine's logits and memory bit for bit at every real point, in Q8_0 and f32, and each projection of a merged
  matrix is the projection alone (every other row scrambled); on every spatial chip a joined program computes what it
  computes stored, exactly where it claims exactness; and each GPU chip names the device test that holds it:
  `cuda_lane_lock`'s two apart tests and `cuda_fusion_census_lock::every_forward_fusion_computes_what_its_kernels_compute_apart`
  (the forward's census, quantized at every numerics and in float, fused and with every step held, bit for bit on the
  device). `cuda_fusion_census_lock::an_add_whose_norm_is_not_the_next_stage_launches_apart`,
  `fusion_class_lock::the_numerics_fold_can_be_held`, and
  `spatial_forward_lock::a_joined_norm_and_rotation_reads_every_element_the_norm_wrote`. `common/fusion_corpus.rs` gives
  every comparison of two graphs its inputs (`value`, `inputs`).
  `numerics_contract_lock` holds every attention scheme a chip declares to its own quantizer — the lane's two- and
  three-word rows by quotients and by products, the engine's unsigned weights and whole-head queries — and
  `the_lanes_narrowed_attentions_are_held_on_the_device` names the lane's device tests of its f16 and two-word attentions.
- `fusion_class_lock::the_rules_keep_what_the_host_reads_across_steps` (a run never carries a step's logits or next
  token in its registers; its twin through another name does) and `a_view_is_quantized_in_place_only_at_whole_blocks`
  (a row read in place at a whole block is quantized in its table, one read inside a block as its own tensor).
- The shared side of five tier-1 parity fusions, each with the ticket's edge cases (`fusion_class_lock`):
  `a_projection_feeding_a_residual_and_a_norm_folds_them`, `a_norm_feeding_a_scale_fuses`,
  `a_gemm_feeding_a_scale_fuses`, `a_gather_feeding_a_norm_fuses_into_one_op`,
  `a_norm_feeding_an_add_fuses_into_one_row_op`. `shim_lock::the_lane_shim_is_its_link` judges every fused op the lane
  could be handed at every depth, a fold's members as much as the fold, and requires a class judged only inside a fold.
- `fusion_class_lock::a_projection_feeding_per_head_norms_and_rotations_folds_them_into_its_epilogue`: on every tiny
  architecture whose attention rotates (and reshaped to 4:1, 8:1, 3:1 heads 256, 128 and 64 wide) every rotation is
  computed by the op that computes its projection, which holds the layer's value projection too;
  `a_merged_projection_reads_one_activation`; and two hold-back witnesses of `merged_projections` (a product's cache row
  read, and its rotation table written, between the products), proven with the others.
- `fusion_class_lock::a_delta_step_with_its_conv_gates_norms_and_output_fuses_into_one_op`: a one-row gated delta
  layer's step is one op with as many value heads as key heads and more, at state widths 32 to 256, from a zero state
  and over 64 steps carrying its memory and rings, with its output's quantize in its epilogue in one word and in two; on
  Qwen3.5's decode with the input epilogue held, and on its batch (each sequence's step one row), every layer's step is
  that op, and a prefill chunk's is not.
  `a_projection_feeding_a_causal_conv_and_gates_folds_them_per_row`: every input projection of one activation with the
  convolutions and gates as one op, at three value heads a key head, with f32 gate projections beside int8 ones, from a
  zero history, on Qwen3.5's decode as it is and at Qwen3.8's heads and gates; a prefill chunk's convolutions and gates
  as one op, carried chunk to chunk. `gdn_decode_whole` has an exemplar; hold-back witnesses of `gdn_decode_whole`,
  `gdn_post_conv_prep` and `gdn_input_epilogue`, proven with the others; `every_numerics_fold_means_what_its_parts_mean`
  no longer counts the whole step a numerics fold, and `shim_lock`'s `AWAITING_A_LINK` names it with its ticket (a
  class the shared table newly builds joins the list; Tristen's go).
- `fusion_class_lock::an_expert_combine_feeding_an_add_and_norm_fuses_into_it`: every mixture of Qwen3-MoE (its
  weights renormalized and not), gpt-oss and DeepSeek-V2, on a decode step, a prefill chunk and a batch, is combined in
  the next add and norm, a step's last layer's in the final norm; five hold-back witnesses of `moe_deferred_finalize`
  (a second reader of the mixture or of its sum with the shared expert, its activation written before the add, a
  mixture multiplied rather than summed), proven with the others.
- `one_ir_lane_identity_lock` joins the baseline: `tests/fixtures/one_ir_identity/lane.txt` pins the lane's 2700
  programs on 51ceaa25e (generated twice, alike), recorded in the lane's freeze window before MIGRATE-1; the suite
  leaves `scripts/run-handed-reds-tests.sh`.
- `one_ir_lock::the_lane_derives_no_fusion`: the lane's launch rewriters are its schedule's and the only firings it
  records are the schedule's; the allowlist sheds the lane's rule functions as MIGRATE-1 deletes them.
  `lane_compile_lock::a_quantized_projection_of_an_activation_short_of_whole_blocks_is_refused` (the activation, its
  length and the next whole blocks named under each quantized numerics) and
  `the_wide_shapes_keep_one_row_a_warp_with_rows_a_warp_held`.
- `common/scratch.rs` and `per_process_dir_lock`: a suite's per-process scratch directory, `<temp>/<prefix>_<pid>`, made
  empty at its first use and removed at the process's exit, passed or failed; the first use of a prefix also removes
  the prefix's directories whose process is gone (one killed by a signal or a timeout runs no exit handler), never a
  running process's, a look-alike name's or a file. `cuda_lane_lock`, `lane_compile_lock` and `model_gguf_lock` take
  theirs from it; they had left 393 directories, 13.7 GB, under `/tmp`. A census holds every other suite that makes a
  per-process directory by hand and never removes one to a list that may only shrink.
- The lane's batch: `lane_compile_lock::a_batch_lane_decodes_its_sequences_in_one_step` (every tiny architecture under
  every numerics: the batch cut beside the decode step and chunks, each sequence's own advance with `sequences` held,
  one launch over the sequences together, the same census at four sequences as at two, every op lowered once, the
  program building), `cuda_lane_lock::a_batch_decodes_each_sequence_as_its_own_step_does` (four prompts of four
  lengths through one program: each sequence's tokens and every logit in the batch, merged and apart, bit for bit as
  it decodes alone) and `a_batch_of_long_spans_folds_each_sequence_as_it_folds_alone` (four spans each cut into a
  different number of attention chunks, folded, for grouped heads, sinks beside windows and latent attention). Twelve
  kernel mutants of a sequence's offsets (its run, cache row, position, index, state, ring, span, fold and caches),
  each killed by one of them. Past the GEMV's rows: `lane_compile_lock::a_batchs_projections_sum_each_sequence_as_the_decode_step_does`
  (at 4, 24 and 64 sequences no batch projection on the tensor cores' GEMM, every one a batch's, the Q8_0 ones on
  `lk_gemv_mma` under the quantized numerics) and `cuda_lane_lock::a_batch_past_the_gemvs_rows_decodes_each_sequence_as_alone`
  (twenty sequences, bit for bit as alone). `cuda_lane_lock::the_batch_gemv_on_the_tensor_cores_sums_as_the_decode_steps_gemv_does`:
  `lk_gemv_mma` at every shape — 1 to 64 rows, `k` of no whole round, whole rounds and tails, one to three parts, a
  bias, a cache row, and the tile the lane launches — writes what the decode step's `lk_gemv_q8<Q, 1>` writes a row
  alone, bit for bit, under both quantized numerics. `lane_compile_lock::a_batchs_projections_take_the_widest_tile_their_rows_fill`:
  4, 8 and 16 n-tiles at up to 16, 32 and 64 rows (at 64, a warp two n-tiles over two m-tiles), a launch of 64 and
  one of the rest past 64, the shared memory set where the tile needs more than the default.
- The `recurrent_state` knob: `lane_compile_lock::a_bf16_recurrent_state_is_carried_in_bf16` (every gated-delta memory
  of every form a `DType::Bf16` buffer declared `__nv_bfloat16`, every delta-rule launch reading it so, and the pool
  the plan sizes shrinking by exactly what the program's bf16 allocations save; the default all `f32`) and `cuda_lane_lock::a_bf16_recurrent_state_decodes_alike_in_every_form_and_near_its_reference` (chunks
  continue as a token at a time and a batch decodes as alone, bit for bit, both numerics; under the precise numerics
  within 1e-2 of the bf16-state reference's logits); `cuda_knobs_lock` and `cuda_knob_consumers_lock` list the knob.
- `lane_compile_lock::a_row_taken_whole_weighs_its_chunks_side_by_side` (a batch's held attention names
  `attn_side_chunks` side by side and takes their shared memory) and
  `cuda_lane_lock::a_rows_chunks_weighed_side_by_side_compute_what_they_compute_one_after_another` (every width, one
  at a time and the split blocks, bit for bit, over grouped, wide and latent-width heads, windows with sinks, `f64`
  sums, chunks past a tile, a chunk's rows and a batch's sequences).
- `cuda_lane_lock::a_tiles_scores_sum_each_dot_as_one_warp_sum_sums_it`: `lk_attn_scores` against the dot taken a
  key and a head at a time with one warp sum, bit for bit — heads 4 to 576 wide, one to sixteen heads a block, `f32`
  and `f64` sums, `f32` and binary16 keys, tiles of one key to a whole tile, zeros of both signs, products cancelling
  in pairs and underflowing to zeros of either sign — every score written once and nothing past the tile.
- The `cache_precision` knob: `lane_compile_lock::an_f16_cache_is_carried_in_f16` (every architecture, numerics and
  form: every array an attention reads its keys or values from, and every view into it, a `DType::F16` buffer declared
  `__half`, nothing else, and the pool shrinking by exactly what the program's binary16 allocations save);
  `cuda_lane_lock::an_f16_cache_decodes_alike_in_every_form_and_near_its_reference` (every architecture under every
  numerics: chunks continue as a token at a time and a batch decodes as alone, bit for bit; each run held to the
  binary16-cache reference within its numerics' bound through the near calls it decided the other way, a cached value
  rounded to the other binary16 neighbour among them — latent attention excepted, whose cached straddles cascade through
  every later layer — and under the precise numerics nearer it than the exact one; the
  search holding a run to the reference keeps a pair of calls where no one call carries a parting — under one-word
  activations a cached value's rounding moves an activation's rounding near its half-way point);
  `model_reference_lock::an_f16_caches_near_roundings_are_calls_a_run_may_take_the_other_way` (a cached value rounded
  within `NEAR_ROUNDING` of a binary16 half-way point is a near call; flipped, that one element takes its other
  neighbour);
  `cuda_lane_lock::every_attention_reads_a_binary16_cache_as_the_f32_cache_of_its_values` (every attention kernel at
  every path, over binary16 keys, values and a split key's rest, writes bit for bit what it writes over the `f32` arrays
  of the same values) and `cuda_lane_lock::every_cache_writer_stores_its_f32_result_rounded_into_a_binary16_cache` (every
  producer that writes a table's rows stores its `f32` result rounded, bit for bit, and the slice and the tiled per-head
  matmul read a binary16 cache as its `f32` values); `cuda_knobs_lock` and `cuda_knob_consumers_lock` list the knob.
- `lane_compile_lock::a_recurrent_layers_gate_projections_are_read_in_the_checkpoints_format` (the author declares
  them quantized; a Q8_0 checkpoint's are quantized projections in the decode step and the batch, an `f32` one's take
  the `f32` GEMV and the fused gates); `spatial_q8_weights_lock`'s third pin is now
  `a_width_of_part_tiles_folds_over_its_padded_tiles_and_computes` (40 and 33 columns fold over padded tiles, every
  padded int8 and scale read once, the VM's product), where it pinned such widths dense; the Qwen3.8 program digests
  (`tests/fixtures/qwen38_authored_digests.txt`, `qwen38_tp_authored_digests.txt`) re-recorded for the moved weight
  manifests; `qwen38_real_dims_lock` holds every stored int8 and scale of a weight's column tiles read once (the last
  tile padded with zero columns, as a gate's 48 heads and a chip's 6 of them are), and `rdu_fit_report_lock` derives
  the Q8_0 bytes with the gates' projections among them.
- `lane_compile_lock::a_narrow_projection_of_long_rows_takes_a_block_of_64_outputs` (a 1024-wide Llama with a down
  projection of 4096-value rows: at 64 sequences each layer's down projection launches blocks of 64 outputs, the wider
  projections blocks of 128, none in slabs); the tensor-core GEMV harness runs every warp tiling to sixteen warps — a
  warp one, two or four m-tiles by one, two or four n-tiles — and the lane's own launch of each case.
- `cuda_lane_lock::a_convs_ring_keeps_each_position_in_its_own_slot`: the rotating ring against the shifting ring
  written out as the conv was, bit for bit, over 2 and 4 taps, plain, through silu and normed, in `f32` and `f64` sums,
  through single steps, runs of rows, chunks longer and shorter than the window and a batch's three sequences at
  positions of their own — after every launch the last `taps` positions each in their own slot (mutants 5/5 killed);
  `a_chunks_conv_and_f32_gemv_over_its_rows_at_once_compute_what_its_rows_one_after_another_compute` gives each chunk's
  launches its first row's position.
- `lane_compile_lock::a_recurrent_layers_six_projections_run_as_one_launch` (a Qwen3.5 with its gates in Q8_0, as the
  real checkpoints keep them: each recurrent layer's six projections one launch at the decode step and in a batch of
  64 under every numerics, none alone, and the kernels' `LK_PARTS` the planner's `GEMM_PARTS`); the tensor-core GEMV
  harness adds a batch of 64 by six matrices, one biased and one into a table's rows.
- `spatial_one_ir_lock` freezes the generation lane's mbarriers (`lk_blk_streamed`, the streamed Q8_0 GEMM's stages) at
  their four: the CUDA chip's own kernel library, as the knob lattice's `__pipeline_*` rungs are.
- `lane_compile_lock::an_attention_in_waves_holds_three_blocks_a_multiprocessor`: a batch of 64's attentions under
  the `f32` sums launch at three blocks a multiprocessor, the decode step's and every `f64` one at the default.
- `lane_compile_lock::a_fused_gate_projection_cuts_its_dots_as_its_projections_apart_do` (Qwen3.5 at the medium width,
  its matrices of whole blocks in Q8_0 as the real checkpoint's: the fused gates' cut of each head's dots is the cut its
  projections take apart, more than one warp) and `cuda_lane_lock::every_fusion_of_the_hybrid_step_computes_what_its_launches_compute_apart_at_width`
  (that model's fused program and its program with every rule held back, tokens and logits bit for bit, both
  numerics).
- `cuda_lane_lock::the_q8_0_gemv_sums_each_output_in_four_interleaved_partials`: the Q8_0 GEMV's quantized order
  computed on the host — block `b`'s step into partial `b mod 4`, a partial's blocks in order, `(p0 + p2) + (p1 +
  p3)` — and `lk_gemv_q8` of one to sixteen activation rows and `lk_gemv_parts` of one and two output rows a warp, over
  `k` of a ragged tail, whole rounds and less than a round, held to it bit for bit under both quantized numerics.
- `model_scoreboard_lock`: the serving engines beside llama.cpp (vLLM, ExLlamaV3). Every engine side keeps every run on
  the wall's clock and the engine's own, names the model's checkpoint it ran (its weight files' bytes recomputed where
  it is on the machine, their digest) and the one version the ledger names; ours over an engine is derived, the
  engine's rate the faster of its two clocks.
- `model_scoreboard_lock::the_decode_rates_stand_beside_the_cards_measured_read_ceiling`: the scoreboard records the
  card's streaming read ceiling in its own boot (`scripts/accel/read_ceiling.cu`, every run) and each row's decode
  bytes a token (recomputed from the checkpoint where it is on the machine); no dense model's rate streams its bytes
  past the ceiling.
- `cuda_lane_lock::the_streamed_q8_0_path_computes_what_the_block_path_computes`: `lk_gemm_blk<8>`'s streamed path
  against its block path over the same matrix and activation with a zero block appended to every row, bit for bit in
  every variant the lane launches (dense, into a table's rows, in parts, paired through the SwiGLU and GELU into `f32`
  and quantized outputs, grouped by expert in tiles of 64 and 128 rows), under both quantized numerics; its cut last
  wave against the sums' replica, and a cut paired launch against its whole tiles.
- `cuda_lane_lock::the_program_profiles_each_prefill_chunk_launch_by_launch`: `--profile` prints a line for each launch of
  each prefill chunk the prompt runs, in the form's order, beside the decode step's unchanged lines.
- `arena_entry_lock`: every entry's checker links position-dependent through the kernel's build script (for the examples
  alone, on Linux), with no link argument, relocation model or `--target` in its recipe; the snapshot every measurement
  runs is built with the logos-kernel entry's own `RUSTFLAGS`, build for build.
- `dag_struct_eta_major`: which structures a recursor may take apart by their projections before reducing the major
  (`Env::eliminates_into_every_sort`, read off each inductive's own recursor): `Prod`, `PProd`, `And`, `True` and `Eq`
  eliminate into every universe, `Nonempty` (a proposition holding data) does not, and a nested auxiliary recursor
  asks about its major's own inductive.
- The Arena corpus at lean-kernel-arena `87512b60`: 236 cases, the 15 new ones vendored by
  `scripts/lean/arena-crown/vendor-corpus.py` from the official archive or from exports rebuilt byte-identical
  (`reachability` from its pinned Mathlib release); an export an earlier round already rebuilt to the published digest
  is taken from that round's directory. `arena_corpus_lock` pins the round.
- `one_program_lock`: every CUDA lane form names its cut of the one program; a cut is the author's text with every
  input at its placeholder; the D5005 engine's program is read off the decode step and its directory carries that
  text; two models of one architecture lower to their own programs; the SN50's embedding, layer and head programs are
  the decode step cut apart, op for op, and its bundle names each; censuses of the program writers, the hand-written
  lanes and the hand lowering's callers.
- `bringup_lock`: one checkpoint on three chips (the CUDA lane, the D5005, the SN50) is one program; a chip that does
  not run a model refuses it by name, each gap once with its layers, and the others bring it up; only the targets named
  run; every chip the command line names is a backend, the ten DEMO-BRINGUP-RUN names; an undescribed checkpoint is
  refused in the spec reader's words.
- `model_scoreboard_lock`: the RTX 3060 scoreboard's rows have both sides and the same runs; every statistic is
  recomputed, and llama-bench's reported ones are its runs'; each llama.cpp run is its tokens over its own
  nanoseconds; our side is the compiled lane running the model's one program; llama.cpp is the pinned build.
- `d5005_card_port_lock::every_write_run_a_hybrid_channel_takes_is_reported_done_at_every_user_clock_the_card_runs`
  (fixture `d5005_card_port/tb_channel_done.v`); `d5005_engine_card_lock`: the two-context decode on hybrid9e's build at
  the card's clock (`hybrid9e_build_at`, `decodes_over_two_contexts_on_hybrid9es_build_at`);
  `d5005_flow_lock::{a_fifo_stops_a_simulation_whose_writer_lets_a_refused_word_fall,
  a_token_fifo_takes_every_token_its_writer_pulses_whatever_its_room,
  every_fifo_the_shell_builds_says_what_its_writer_is}` (fixtures `d5005_flow/tb_dcfifo_refused.v`,
  `d5005_flow/tb_dcfifo_tokens.v`).
- `d5005_flow_lock`: no register of an engine clears asynchronously; the column array's activation copies addressed by a
  token-row's place and a block's alone; every landing's valid starts from the reset or from its clock; every rows-table
  read of an engine of contexts at its job's context; a build holding one to its most passes of its deepest vector job
  on its vector side. `d5005_engine_rtl_lock`: a conv path side's vector unit of eight registers refusing a job that
  names a ninth; every convolution job behind the pipelined store; two contexts' steps with every write landing late; a
  side five vectors deep running its deepest jobs' passes in groups. `d5005_machine_lock`: every vector job but a
  convolution names fewer registers than a conv path side's unit holds. `d5005_rtl_lock`: the conv path behind stores of
  two to four clocks. `d5005_quartus_queue_lock`: a flat fit stopped after its retiming resumes at its finalize stage,
  and runs whole when the fitter refuses it, the refusal leaving the job's log no failure line.
- `d5005_machine_lock`: a fold is the add or the gated product it replaces, and folding leaves every value as it was; a
  gated norm folds only into the recurrence whose read-outs it norms; the 27B folds every residual add, gated product and
  gated norm; every store laid at its step's widest point; the machine is the model at every knob point flushed every
  window, a window behind and rewritten every pass; a trailing flush's log is a ring of two windows.
- `d5005_numerics_lock`: a multiply-add is the multiply then the add, bit for bit; a trailing walk of a log gives the
  fold of its newer tokens and of the whole log. `d5005_roofline_lock`: a trailing flush writes what a flush every window
  writes and reads a window more log. `d5005_capability_lock`: a step flushed a window behind runs on no build that
  replays a whole log.
- `d5005_engine_lock`: Qwen3.5-0.8B's and Qwen3.8-27B's fastest points flushed a window behind meet their bars, short
  and long prompts; every knob point's bar on the card's multiplier for Qwen3.5-0.8B, Llama-3.2-1B and Qwen3.8-27B; a
  trailing flush leaves each row the token-by-token memory a window later.
- `d5005_rtl_lock`: the fold lane, the gate, the conv path, the pipelined store (against the store of one clock, and
  every layer read every clock as its block RAM reads), the user-clock vector unit's multiply-add and groups past the
  add's latency, the token-pair array (≡ `dot`, a pair a clock), the one-word lane at the widest sums its units make,
  every memory `rtl::ram` builds asking for the RAM its depth fills; head lanes reading far ahead, flushing over several
  flush units, replaying each row's share a window behind (at the card's width, wrapping the ring, a row short of a
  window), norming every head's read-outs, running a job's rows one after another and reusing a log copy or a flush bank
  only once it is free, each ≡ the numerics.
- `d5005_engine_rtl_lock`: every matvec job of a folded step on one, four and sixteen fold lanes and on token pairs,
  every recurrence job on head lanes flushed a window behind, over split flushes, reading far ahead and making the gated
  norm, every convolution on the conv path, and the whole step with every unit on the fast side for each, ≡ the machine;
  a sequence's rows after each row's writes land late; a run busy until its writes are on the card; a convolution
  waiting only for its own sequence's ring.
- `d5005_engine_card_lock`: six sequences decoded on a folding engine, with the gated norms on the head lanes, and on
  two head lanes reading far ahead flushed a window behind, each the machine's; two contexts on the 55 MHz image's
  build, at the tiny and the small model's widths and from power-up noise; the refusals of a folded or trailing program
  on the other build; every program's job listing at its job table's words.
- `d5005_flow_lock`: the builds of every new unit against the D5005 PIM; their 200 MHz stage checks with their wide-net
  ratchets; the compute side's multicycle on exactly the compute clock's paths; the compute clock's late marks; the
  pipelined bank chosen for a build whose core reads the store for the host alone; a memory of 32 words or fewer asking
  for MLABs or registers and the RAM gate refusing one that does not; a name with a value declared apart; every build
  flag the emitter takes offered by `scripts/d5005/build-afu.sh`; a given-up command's last progress; the cosimulation's
  log. `d5005_card_port_lock`: the card's port mix through the multiplexer never stops. `d5005_quartus_queue_lock`: the
  IP cache, the stall watch, a stopped step tried again, the timing analyses and the failing-paths report.
  `codegen_sva::timing_graph`'s unit tests: a net's fan-out with each memory's blocks, a register's copies, the wide nets named
  widest first.
- `d5005_rtl_lock`: the block quantizer over blocks of every length it serves back to back and at gaps, the ring at its
  fullest, every block's scale, codes and tag `quantize_blocks`' at the clock `act_blocks_out` gives; the streaming
  matvec over blocks in slots of 16 and 64 bytes ≡ `numerics::dot` (`striped_matrix_in`, `streaming_matvec_runs_in`).
- `d5005_machine_lock`: a load kept exactly when it is a tensor at stride 0 into a register nothing else writes, its
  word's flag, a lying flag refused; every tensor a job of the tiny model or the 27B reads at stride 0 kept.
- `d5005_engine_lock`: the knob points a family is held to follow whether its attention norms its keys (the Qwens the
  standard points, a Llama or any un-normed layer the rotated int8 points); the real-checkpoint bars iterate each
  checkpoint's family points, `llama3_2_1b_meets_every_knob_points_bar` (+ long prompts) on Llama-3.2-1B's goldens.
- `model_reference_lock`: an f16 cache keeps each key and value as llama.cpp's default cache does (the first layer's
  rows the exact run's rounded, every element cached a binary16, the logits moved and near, the exact cache the
  default); the front door's binary16 rounding is the engine's at every boundary. `timing_graph_lock`: a box declared with its area adds it for every
  instance, an area not five counts refused. `d5005_preflight_lock`: a design on the card's floating-point blocks
  counts a DSP block each, the block given by the device library as a box; Quartus packs a register-bound design by
  its registers and every synthesis by the larger term; the fitter's time held over persona compiles alone.
  `d5005_quartus_builds_lock`: a persona's region read from persona compiles alone, a flat image's fit recording its
  floorplan's Logic Lock regions. The preflight unit tests: a flat fit and its floorplan's regions are no persona, nor
  is a flat fit's region named for the PR slot; a register-bound design held to its registers' packing; a synthesis
  judged by the term its packings set, not by which it holds more of; a design on the floating-point blocks given the
  block as a box; a flat image's needs every partition's estimates and the M20K blocks the fitter gives its memories,
  each resource past the device's refused by name.
- `d5005_machine_lock`: a rotation without frequency factors is its unscaled angle bit for bit (a Llama and the 27B's
  multi-section rotation); a factored Llama's table divides each pair's angle by its factor, the lanes' `rope_tables`
  bit for bit; the machine computes what the model computes for a Llama with and without factors at every knob point;
  a factor table missing or not one factor a pair refused by the machine and the model. `d5005_engine_lock`: a factored
  Llama turned by its factors 380 times nearer the reference than turned without them. `d5005_capability_lock`: Llama
  3.x admitted; the capability unit tests admit a factored rotation and refuse one without its table. `tiny_models`
  writes `rope_freqs.weight` for a factored spec (`with_rope_factors`).
- `d5005_engine_rtl_lock`: every kept load read once a job (the recurrent channels' `SERVED` lines), RED on the vector
  side without its skip; every matvec job and the whole fast-side step with blocks in slots (`Sides::slot`).
- `d5005_weight_image_lock`: a striped image lays each block in its slot, its pad zeros, its tiles and tile runs
  counting slots, a slot of one byte the image without one; the 27B in slots of 16 pads Q6_K alone, under 2%.
- `d5005_flow_lock`: a slotted build elaborates against the PIM, names its slot in `LOGOS_D5005_STREAMING` and gathers
  a beat a slot at a time (`fast_side_08b_packed`); the hard-float wrappers' corrections read from their
  documentation, their code checked beside it (`module_doc`).
- `d5005_rtl_lock`: the column array of a set taking Q8_0 in octets, alone and beside Q4_K, ≡ `numerics::dot` over one
  to eight token-rows and both word counts (`set_array_runs`, the array harness for any set and format).
- `d5005_engine_rtl_lock`: every matvec job and the whole fast-side step with Q8_0 in octets; every float matrix's job
  reads its weight rows once a group of rows, counted by the recurrent channels (`SERVED`), its rows two groups; a
  hybrid world's sides one value (`Sides`: granule, vectors, width, attention, octets, float rows).
- `aig_equivalence_lock`, `rtl_equivalence_lock` (fp32's pipelined units proven their combinational units 3 and 4
  clocks late by both engines, the lane's pipelined rounder its rounder 5 clocks late and the activation scale its
  scale 3 clocks late, the half codec 2 clocks late and the integer-to-f32 conversion 2 clocks late, each with a
  mutant told apart; over units each proven apart: the sixteen-lane tree 16 clocks late over its adders, the
  exponential, reciprocal, reciprocal square root and logarithm over their multiplies and adds (and the conversion),
  the user clock's lane function unit 102 clocks late over its cores, the staged lane function unit the unit at once
  four steps later, each with an operand held a clock short or long told apart; a product one circuit however its
  source spells it; a unit's function its module and the values its instance gives, one module at -1 and at
  32'hffffffff never one function; the GDN read-out at two sixteens a clock giving the one-sixteen read-out's rows
  token for token, offered every clock and with one side offered three clocks of four, and a carried value added a
  clock early not the stream), `d5005_stream_rate_lock` (one channel of the streaming matvec, tile run to image
  stream to feed, every handshake proven constant whatever the lines hold: Q4_K and Q8_0 in octets take a beat every
  clock and lose one clock between tiles at 64- and 72-byte lines, each tile's lines within its beats' clocks; plain
  Q8_0 is dealt a 34-byte block a clock, about half the channel; an image stream holding one tile at a time breaks the
  law; the Q4_K and Q8_0 streaming columns, their datapath cut, take exactly `column_clocks` over 72 shapes each
  whatever their data), `d5005_floorplan_lock` (hybrid3's reports: each EMIF user clock from its bank's tile control,
  channel k on bank 3 − k, each channel's need, the persona's density; each channel's square inside the persona or the
  device, nearest its bank, none overlapping, the same every time, settings naming only the channels; a memory's fewest
  M20K blocks, its stack a power of two deep; a unit's needs its synthesis, hybrid7b's store and attention side the
  1,024 and 45 M20Ks its fit gave them; a module's graph through its own logic and the modules it dissolves, every
  instance named, no clock pin a wire, no net two instances only read a wire between them (a broadcast a star from
  its driver) and no shared net counted twice; each channel's anchor where its bank's clock
  leaves the IO tile control, and the anchors ledger what hybrid3's reports place; the geometry probe's 332 tiles
  hybrid3's device but the ten its first fit found no flip-flop in, together the device's ALMs, M20K and DSP blocks,
  four registers an ALM), the floorplan unit tests (tiles, the probe, the tiles a probe's fit found no flip-flop in,
  the geometry, a region's fill of logic and of hard blocks, candidates, rectangles placed and regions grown round what
  no rectangle clears, region names, a unit floorplan's wires and refusals, a group's wires to each member's anchor,
  its refusal by any member the module lacks and its needs packed together, the loose instances by the bits they
  carry to the units, a memory's M20K blocks and where each MLAB
  shape ends, the RAM summary read with each memory's parameters, a unit's memories under its own path alone, either
  port's byte enables),
  `timing_graph_lock`, `autopipe_lock` (the
  multiply cut anywhere to a third of its depth proven the multiply that many clocks late; an uneven pipeline cut
  again at its own latency), `fp_hypotheses_lock` (the ENGINE member is the engine's multiply and add, the IEEE member
  this machine's, and the discriminator ledger what every member computes), `d5005_fp_oracle_lock` (the oracle beside
  a stand-in block under iverilog counts no difference for the block's class and some for every other, only at the
  tap that is the block's latency; a recording run's records are its vectors with the block's three results, its
  counts its records'; past the store it keeps the first records and compares every step; the streams are their
  seed's, the near one landing where flushing tells classes apart and its records a fifth or more at the least normal
  with sums that cancel deep; blocks of different depths each met at their own and aligned together; the
  least-registered blocks recorded their latency ahead; the whole persona through the shell, its measurement run
  by the simulation's host, read back into the stand-in's blocks; and a multiply no class of the oracle names still
  measured to its depth, the add named and read alone; a record holding 128 product classes and 96 sum classes, one
  more of either refused), `fp_hypotheses_lock` (the family's 98 members, the card's multiply among them; an
  XOR-signed NaN's name and result; every member's software multiply and add its unit on the corpus, every
  discriminator vector and adds cancelling a power of two into the binade below), the `fp_hypotheses` unit tests (the ENGINE member the engine's numerics, the IEEE member this
  machine's `f32`, the card's multiply the engine's but for its NaN sign and the least normal), the `fp_hard` unit tests
  (the plan for the engine the plan, every member planned for itself needing nothing, another target's corrections and
  named refusals; the wrappers for the engine the wrappers, byte for byte), `rtl_equivalence_lock`'s wrapper planned
  for the card's multiply proven the card's member (its own block padded or least-registered, a block keeping
  denormal operands decided beside it with the card's NaN) and one for an ARM core's flushing multiply proven that
  member (a NaN operand's payload decided beside the block), `d5005_fp_receipt_lock` (every receipt in
  `fp_hard_receipts.json` read again from the inputs kept beside it, from a build that passed and the engine it
  compiled; fp-oracle5's add the engine's own three registers deep, wrapped at four clocks with one register after it,
  its multiply no member of the family, its multiply-add rounding twice; one step against the agreeing class and no
  class is the add; refused for another image, a reconfiguration error, no user clock, times out of shape or order, an
  unfinished measurement, a missing, failed or engineless build or one naming two engines, discriminators that make no
  oracle, an engine the build did not compile or one of whose own modules the discriminators do not regenerate (a
  library module moving on held by the build's SHA alone), and a measurement recorded already; appended after what the
  ledger held,
  written one space a level), `fp_hypotheses_lock`'s
  `no_member_leaves_a_bit_undefined`, `rtl_equivalence_lock`'s hard wrappers (the multiply and the add on the
  engine's, an x86 core's and NaN-variant blocks, padded and least-registered, and the wrapper stepping at an enable,
  each ≡ the engine's unit at the pipelined latency; a wrapper short of a correction or a register told apart),
  `d5005_lower_graph_lock`, `d5005_quartus_builds_lock` (every fit
  records its hold delay and its clocks off the globals), `d5005_quartus_queue_lock` (every attempt waits for the
  DDM start-up check; a job whose ledger row could not be recorded refused before it queues; a resumed job's row
  names the compile it went on from, and a job whose own earlier attempt ran the fitter names itself and is judged
  without the fitter), `timing_calibration_lock` (`benchmarks/silicon/d5005/timing_calibration.json`: every synthesis
  the ledger records calibrated or refused by its Verilog, every top within a factor of two of Quartus's Max Depth,
  the fit centred and its spread within a quarter; every instance's area as Quartus counted it, the ALUT fit
  recomputed from the rows, every instance of 10,000+ ALUTs predicted within a factor of two), and `d5005_capability_lock`'s builds (a build covers a step
  exactly when their union is the build, over every pair of admitted checkpoints; every gap named field by field; a
  hybrid build's streamed formats; a checkpoint's header judged against a build; a stale matrix named by checkpoint), `d5005_preflight_lock` (the PR
  region every persona was placed in, Quartus's ALMs an ALUT, the fitter's time within a factor of two of every
  persona's, over refused, past the fullest fit tight, well inside a go), and `d5005_quartus_builds_lock`'s regions
  (every persona fit read since records the one region, the same six rectangles; a job judged without a fitter its
  flow ran names the compile it resumed, recorded earlier from the same tree, which ran it).
- `d5005_flow_lock`: a sixteen-row hybrid build with every unit on the fast side and the recurrence unit at a line a
  clock elaborates against the PIM with no warning; a hard-float build carries the block's wrappers and the stand-in for
  simulation alone, elaborates, and a block no wrapper can serve is refused.
- `d5005_engine_rtl_lock`: the recurrent channels give lines back in bursts on `+burst=1` (`HybridWorld::bursting`);
  the twenty-head float matrices and the whole fast-side step in bursts; the whole fast-side step on hard f32 units.
- `d5005_engine_rtl_lock`: every vector job on the vector unit's side over a model whose float matrices have twenty
  heads (three groups of the float-matrix path), each job's store and flat space the machine's.
- The recurrence unit at a line a clock and the attention side. `d5005_rtl_lock`: `logos_gdn_read_w` and
  `logos_gdn_flush_w` ≡ `gdn_read`/`gdn_flush` at 32 and 64 elements a clock (heads of 128, 64 and 32, logs of 16, 7,
  3, 1 and no tokens, f32 and f16, a full log within a quarter of its passes' clocks), the side at W 2 over the eight
  cases the side at W 1 runs, `logos_attend_int8_p` ≡ `attend_int8` (heads of 64 and 256 over 1–70 positions, zero rows,
  every kind of f16 scale), all under Verilator. `d5005_engine_rtl_lock`: every recurrence job and the whole step at a
  line a clock; every attention job and the whole step on the attention side. `d5005_flow_lock`: the attended build's
  hand-off, census and PIM elaboration. `d5005_engine_card_lock`: six sequences greedy with the recurrence at a line a
  clock, and with every unit on the fast side (in the silicon on-demand class).
- The greedy head. `d5005_rtl_lock`: `logos_greedy` keeps each token-row's `numerics::argmax` of its rows and that
  row's value over 600 cases of four token-rows, beats as the streaming matvec's writer gives them (stripes, a beat met
  twice, lanes not present holding other numbers, idle clocks), the values ties at the largest, NaNs of either sign,
  both zeros, both infinities, subnormals and rows of no number. `d5005_engine_rtl_lock`: a greedy head's job on the
  streaming matvec leaves each row's arg-max of the machine's logits and that logit at one, three and four pairs a
  channel and both word counts, every other matvec job as on the machine. `d5005_engine_card_lock`: the greedy program
  of six sequences decodes on a vectored build of four pairs a channel whose store is laid greedy, each sequence's
  tokens the machine's (in the silicon on-demand class); a program for more rows than its build, or a larger store, is
  refused before the card is touched. `d5005_flow_lock`: a vectored lazy build's core keeps no vector unit or row
  engine, a recurrent build's keeps its vector unit.
- The one-clock accelerator. `d5005_flow_lock`: no clock made inside the accelerator (every build's PIM top handing the
  shell the platform's clocks; the shell and all it instantiates, flattened by yosys at divides 1 to 64, clocking every
  register and memory port from a clock port; the compute clock at divides 2 to 128 and the card port from their
  clocks, through `codegen_sva::netlist::clock_sources`); the compute side stepping only at the compute clock's step
  (with a compute clock that never steps, no register bit the user clock clocks in the shell can change but the
  engine's and the fast ports', and with the real one over a thousand can, on the divided, engine, hybrid and recurrent
  builds); every FIFO side stepping only at its enable, under enables of one cycle in 3 to 64 on both FIFOs (the fixture
  `tb_dcfifo.v`'s `WCE`/`RCE`), its outputs never moving at an edge it does not step at; a divided build's shell, its
  step, its FIFO sides and constraints; a divided shell moving every line as the undivided one does (`tb_shell.v`'s
  `DIVIDE`); `run-step.sh` on a host of stand-ins, a fixed-clock persona loaded as root and run at the clock asked.
- `d5005_rtl_lock`: `logos_round88_p`, the exact sum's rounding a clock at a time, against `logos_round88` over every
  power of two and its neighbours, every halfway point at every shift, random sums and the exponent's bounds; and
  `logos_wide_scale` against `wide_scale` at every rounded significand its ceilings over 127 see, and
  `logos_wide_scale_p` against it a clock at a time over those significands at four exponents, zero and seeded
  magnitudes of every exponent, in both word counts.
- `d5005_engine_rtl_lock`: an engine with the recurrence unit prints for 32-, 64- and 128-wide heads and refuses,
  naming the width, 16-, 48- and 80-wide ones.
- `d5005_rtl_lock`: the recurrence side over a memory that lands each write 64 to 127 clocks after taking it and
  reports itself written only once none waits (the log holding only the row's entry), and through a slow write port.
- `d5005_engine_card_lock`: the plain, recurrent and vectored hybrid engines decode as the machine does from the
  state a partial reconfiguration leaves them in, every register and memory no reset reaches starting as noise of two
  seeds (`run-cosim.sh`'s `LOGOS_SIM_SEED`).
- `d5005_engine_card_lock`: the vectored hybrid engine built for three token-rows decodes three prompts of different
  lengths at once, each sequence's tokens the machine's for its prompt alone (in the silicon on-demand class).
- The user clock's vector unit and the vector unit's side. `d5005_rtl_lock`: the pipelined reciprocal, reciprocal
  square root, logarithm and integer conversion, each its function a clock at a time; the pipelined function unit; the
  user clock's vector unit on the micro-program, shared-function-unit, rotary/cache/conv/greedy and power-of-two case
  lists the engine's unit runs (the lists now shared by both units' tests), and beside the engine's unit on random
  programs over every instruction at 16, 4 and 1 function units, every bank, length and scalar register the engine
  unit's after every instruction. `d5005_engine_rtl_lock`: every vector job of a traced step on the side, the whole step
  and its twin. `d5005_flow_lock`: the vectored build's crossings, hand-off and PIM elaboration.
  `d5005_engine_card_lock`: the vectored AFU decoding eighteen tokens as the machine does.
- `d5005_engine_card_lock::the_cosimulation_hands_its_host_program_every_argument_in_order`: eight hundred `+arg=`
  plusargs among others, some ten kilobytes of command line, reach a host program whole through `sim_start` and
  `sim_cmdline`, and its exit status comes back.
- The hybrid engine's recurrence unit. `d5005_card_port_lock`: the fast port beside the card port on the hybrid shell's
  channels at eight, sixty-four and the engine plan's 512 lines a granule, its runs across granules and channels longer
  than a descriptor and than a channel's write FIFO, its lines below the space faulting with zeros, every read in order
  and every channel line where the map puts it (`common::card_memory`'s `Run::{script_where, fast_script, check_beside}`
  and `FastOps`, the fixture `tb_fast_port.v`). `d5005_engine_rtl_lock`: every recurrence job of a traced step and the
  whole step with the recurrence unit beside the streaming unit, one sequence's rows logging, flushing and logging on
  the memory the flush left, the flat space dealt over the four channels, and its simulation twin the engine's.
  `d5005_flow_lock`: the recurrent build's shell through the crossing gate, its fast writes, map, recurrence unit and
  timing marks, and its elaboration against the PIM. `d5005_rtl_lock`'s Gated-DeltaNet side reads the log's count,
  appends the row's entry from the store, its fields' spare lanes holding anything, and writes the count back: the log's
  lines and its count line after are the machine's byte for byte, over a memory reporting its writes written a while
  after it takes them, logs of one token to sixteen. `d5005_engine_card_lock`: the recurrent hybrid AFU decoding
  eighteen tokens through a flush as the machine does.
- `d5005_rtl_lock`: the function unit in its stages, an operand a cycle, each result `Func::apply`'s `LANE_FUNC_STAGES`
  cycles after its operand; the vector unit's harness reads back only the rows within each register's length.
- `d5005_rtl_lock`: the pipelined tree of sixteen lanes the numerics' fold a clock at a time, over every vector the f32
  units are held over and over cancelling lanes; the streaming read-out reading every row of heads streamed back to back
  as `gdn_read` does, at 128, 32 and 16 elements a row, the beats held back at random and held off by the unit when its
  buffers fill; the streaming flush replaying every row as `gdn_flush` does at logs of sixteen to no tokens, f32 and
  f16, the rows held back on their way out; the streaming fold folding every head as `gdn_fold` does; the pipelined
  exponential `exp` a clock at a time over every operand the exponential is held over.
  `common::require_tool("hidden-frames")`, a named probe: the page map hides this process's physical frames, where
  `d5005_flow_lock`'s physical-backend refusal runs; the Gated-DeltaNet side running a token row's block laid out as the
  machine lays it, every read-out `gdn_read`'s and every flushed row `gdn_flush`'s, over a memory answering reads at
  random later and taking writes at random, f16 and f32, one and several heads a group.
- The engine in the user clock and the hybrid engine's locks. `d5005_engine_rtl_lock`: every job and the whole step on
  the engine stepping in the user clock under enables (its testbench drives the user clock through the card's divider);
  every matvec job of a traced step and the whole step on the hybrid engine, its engine clock six, eight and ten user
  cycles, the store and the flat space the machine's; a one-word engine faulting a two-word job; the crossing meeting
  the shell as an engine in the shell's clock does at six, eight and sixty-four; the simulation twin the engine with
  only its core's clock gated, refusing an engine whose core could change at an edge the gate drops, and running the
  whole step of the engine and of the hybrid engine to the engine's store, memory and cycle count. `d5005_flow_lock`:
  the divider's edge marks, an engine build's clock map, shell, SDC and header, its shell through the crossing gate with
  the core (and a hybrid's user-clock side) boxed and the bridge declared, the bridge gate refusing each defect, the
  single-clock box proof taking an asynchronous reset, the engine's timing marks where the SDC's multicycles assume
  them, the recorder requiring the bridge's gate, a hybrid build elaborating against the PIM, an engine build carrying
  its engine's simulation twin, which `run-cosim.sh` simulates. `d5005_rtl_lock`: the enabled memory under the divider's
  marks, the word read at the read mark the word before the engine cycle's write and held to the step.
  `d5005_engine_card_lock`: the engine builds at a compute clock an eighth of the user clock; the step program decoding
  the tiny model on the hybrid AFU as the machine does, and refusing a program written for another streaming unit.
- `d5005_flow_lock` also holds the crossing gate's single-clock boxes: a value through a FIFO and on through a box of
  two levels passing, a box with a register on a second clock, a child on a clock it derives, a box fed from the other
  clock and a box instantiated at other parameters refused; and Qwen3.5-0.8B's engine build's shell crossing only
  through its FIFOs with the engine boxed. Its fixtures are in `crates/logicaffeine_tests/tests/fixtures/d5005_flow/cdc/`.
- `d5005_rtl_lock` also holds the pipelined f32 multiply and add a pair a clock over the combinational units'
  vectors, the pipelined lane's every block value the lane model's from −0, the registered-read block RAM, every
  decoder's multipliers as signed bytes and its offset a power of two, the pipelined code unit a value a clock and the
  pipelined quantizer block for block, the streaming column and the column array (after a run cut short by a reset, at
  random gaps and holds) multiplying matrices block-major as `dot` does and taking exactly their cycle models' clocks
  at one to four passes an issue, each channel's feed dealing its tiles' blocks to its cores in pairs, and the
  streaming matvec from a striped image's channel beats, one built for every format the 27B holds. `d5005_weight_image_lock`
  holds the stripes (streamed tiles block-major, a stripe's four tiles the four channels', the refusals) and a channel's
  tiles of a tensor as one tile run; `d5005_engine_rtl_lock` every matvec job of a traced step on the streaming matvec
  from the four channels' memories as on the machine, and its Verilog under a strict front end at the card's size.
- `d5005_physical_map_lock`: the card's memory past the weight image as `physical::PhysicalMap` deals it — every byte
  one place, granules taking the channels in turn, a run one stretch on each channel it touches, the channels holding
  the machine's memory byte for byte, and a machine past the channels refused.
- `d5005_card_port_lock`: `logos_card_port` under iverilog reading and writing the flat memory where the map puts it at
  eight and sixty-four lines a granule and at the engine plan's, a line below its space faulting with the weight image
  left as it was, and on SECDED lines reading corrected and faulting on a word the code cannot correct.
- `d5005_engine_card_lock`: the engine build's engine running a step with its memory on the card's channels through the
  card port, `sw/logos_step.c` decoding the tiny model's tokens on the engine build as the machine does, a faulting
  step reported by the engine's run, and the step program compiling against OPAE 2.12 and linking against the physical
  backend.
- `d5005_flow_lock` also holds: SECDED lines — each line its data and every word's codeword checking, one flip a word
  corrected and two found, every codeword on 72 pins under either burst order, the emitted coder and decoder the code,
  a channel reading back what its flips make, the C header naming the check lanes, the receipt proving the code on
  every channel and failing when the RTL is wrong, and its recorder refusing a code that did not hold; the receipt under
  Verilator through the DPI-C backend; a divided build running its shell, and an engine build its core, on the user
  clock divided, the divider releasing its reset in its own clock, and the divided build elaborating against the FIM's
  PIM; the PIM boundary setting the shell's line and the PIM check refusing a width cut inside an instance; the recorder
  recording a flat image and refusing its failures; and the physical backend (`sw/logos_hal_pci.c`) over a device tree
  laid out in files — the accelerator opened by its AFU ID on the one driverless VF of the FIM's PF, memory decoding and
  bus mastering only while it is open with every function it looked at given its command word back, a pinned function
  opened or none, the ID on the PF, on another vendor's function or on a VF a driver holds never opened, a VF behind a
  translating IOMMU refused for VFIO, a buffer placed only on frames its page map gives present, nonzero and in order
  — and the receipt and loader linking against it; the core's progress records (every command of the shell run with
  them among its own, a long fill's growing record to record under its tag with the count of completed commands left
  alone, a fill whose channels take nothing reporting none); and a host's wait bounded by progress, not length
  (`sw/logos_core.h` against a scripted device). Its probes are in `crates/logicaffeine_tests/tests/fixtures/d5005_flow/`.
- `d5005_rtl_lock` also holds `logos_act_stream` quantizing each block as the numerics do; a lane's function unit
  computing every function of the instruction set with one core of each function in it; each vector lane's own
  arithmetic four multiplies and two adds every instruction shares; and the vector unit's row of function units shared
  across its lanes at one, two, four and eight units a row, with one row in it.
- `d5005_engine_rtl_lock` also holds the engine's Verilog elaborating under a strict front end (pyslang, refusing
  implicit nets, unconnected ports, cut widths, sign compares and out-of-range selects); one engine running the 27B and
  the 0.8B; a step on an engine larger than it needs; the 27B's engine quantizing every block with one streaming
  quantizer and sharing its vector unit's function units; and the card's memory taking the engine's writes under random
  backpressure.
- `d5005_weight_image_lock` also holds the image holding the machine's memory (`image::image_of`).
- `fanout_within_limit_lock::every_legalization_after_an_edit_holds_each_diode_and_its_nearest_pin_where_they_are`: in
  the post-diode upsize, the pairs are taken after the grid-building legalization and after every legalization since
  (each diode and the instance of its net's nearest pin, with its place and cell), each legalization after an edit is
  bracketed by `logos_diodes_hold` and `logos_diodes_release`, and the hold makes FIRM only a diode unmoved since the
  pairs were taken and a partner unmoved and unresized.
- `c_bytecode_lock::a_constant_sum_or_difference_is_checked_by_the_overflow_flag`: on machine words, in Main and in a
  word entry (Takeuchi's function among them), every sum or difference with a constant is checked by the overflow flag
  and no guard compares against a bound past 32 bits, while a product by a constant keeps its range test. The
  speculated-integer corpus gains seven rows that cross the word by a constant: an increment, `3 + k`, `k + (0 - 5)`, a
  decrement, `k - (0 - 7)`, `1 - x` in a function and Takeuchi's decrements past the bottom, each printing what the VM
  prints.
- `dist_profile_lock::the_gate_profile_optimizes_every_dependency_and_keeps_the_tests_checks`: the gate's
  `[profile.test-opt]` inherits `test`, builds every dependency at opt-level 2, and turns off neither overflow checks
  nor debug assertions, for the profile or any package in it.
- `logicaffeine_proof::rup` units: `clause_state` on conflicts, units, repeated literals, tautologies, a true literal
  anywhere and two distinct unset literals; exhaustively against the collected rule it replaces on every clause of up to
  four literals over three variables under all 27 partial assignments; and `propagate` against fixpoint propagation over
  that rule on 4,000 random databases, verdict and final assignment.
- `phase_exodia_oracle::{a_list_a_loop_pushes_onto_keeps_every_length_it_reaches,
  a_loop_condition_holds_every_value_its_operands_take}` and
  `mod_wrap_lock::{a_remainder_in_a_loop_condition_keeps_its_division_when_the_body_grows_its_dividend,
  a_loop_whose_condition_reads_a_grown_remainder_ends_on_every_tier}`: the range oracle's length after 200 and 500
  pushes, the padding loop's condition reading 201 and 272, and the padding loop ending at 272 on the tree-walker, the
  VM (under a step budget) and C (under a wall-clock limit).
- `sky130_ledger_registry_lock::a_staging_copy_outside_the_tree_records_as_the_ledger_it_is_named_for`: a registered
  path records as itself, a copy outside the repository named as one registered ledger as that ledger, and an
  unregistered name, the registry's own name, a name two registered ledgers share, or a registered name at another
  path in the tree as none.
- `caravel_project_lock::each_project_carries_its_own_setup_margin` and
  `flow_config_lock::the_setup_repair_margin_is_a_lever_emitted_only_when_chosen`: the command player keeps OpenLane's
  setup repair and the engine closes 1.25 ns past its estimates, a chosen margin reaches the scripts in ns as
  `CP_SETUP_MARGIN` and the macro harden passes it; both margins are absent by default and emitted once each when
  chosen, for a block and a vehicle macro alike, and the emitter refuses a margin that is not a non-negative time;
  `flow_script_guard_lock`'s sandboxed runner forwards `SETUP_SLACK_MARGIN`.
- `caravel_project_lock::each_project_carries_its_own_legalization_padding`: the command player keeps the placement it
  was signed off with, a chosen padding reaches the scripts as `CP_DPL_PADDING`, and the macro harden passes it to the
  emitter.
- `flow_config_lock::{the_legalization_padding_is_a_lever_emitted_only_when_chosen,
  a_diode_threshold_that_is_no_positive_length_is_refused}`: `DPL_CELL_PADDING` is absent by default and twice the
  sites a side when chosen, and the emitter refuses a padding or a diode threshold that is not positive rather than
  building at its default; `flow_script_guard_lock`'s sandboxed runner forwards `DPL_PADDING` as
  `--dpl-padding-per-side` and nothing when it is unset.
- `caravel_project_lock::each_project_carries_its_own_diode_threshold`: the command player keeps the default pass, the
  engine diodes nets past 700 um, a chosen threshold reaches the scripts as `CP_ANTENNA_THRESHOLD`, and the macro
  harden passes it to the emitter.
- `caravel_project_lock::each_project_carries_its_own_antenna_repair`: the command player keeps the default repair,
  the engine repairs at 20%, a chosen margin and pass count reach the scripts as `CP_GRT_ANT_MARGIN` and
  `CP_GRT_ANT_ITERS`, and the macro harden passes both to the emitter.
- `test_ledger_lock`: the full profile wraps every test in `scripts/test/test-ledger.sh` and no other profile does; the
  wrapper passes a test's arguments, output and exit status through unchanged, ends by the signal that ended the test
  (an abort, and nextest's TERM to the process group, the test still recorded), counts the CPU of the processes the
  test waited for, hides the ledger from the test, and keeps 48 lines ending together whole; the report reads the run
  with the most lines and names the heaviest, the widest, the most crowded stretch and the signalled; the fast script
  opens the ledger for pass 1 alone and names it in the summary; `--widths` writes the widest tests as exact
  `threads-required` overrides capped at the run's threads, and every width the full profile declares names a test
  that exists, after every hand-written override.
- `phase_fast_div`: a dividend `>= 0` with no upper bound (`i - 1` over a counter a runtime `n` bounds) lowering to the
  precomputed magic multiply and summing to 36; a dividend that can be negative (`i - 7`) and a counter from -3 keeping
  the hardware `%`, summing to -18 and 59.

- `wpe_offchip_memory_lock::a_cache_stalls_from_registered_hits_alone` (yosys: `busy`'s combinational inputs are the
  request strobes and the registered hits) and `every_address_a_cache_serves_is_set_beside_its_hit` (every assignment
  of a served address sits in the task that registers its hit, beside a refresh for a held one); the cache bench, the
  arbiter bench and the off-chip engine runs fail on any registered hit that disagrees with its cache's lines, and the
  cache bench ends with a one-cycle reset over valid lines.
- `postgres_protocol::a_cancel_waits_for_the_postmaster_so_it_never_cancels_the_next_query`: a scripted postmaster holds the cancel connection open 300 ms and answers a statement that arrives before its close with ErrorResponse 57014, as the signalled backend would; the query after the cancel still answers 42 on both tiers.
- `emit_wasm::a_linked_module_carries_none_of_its_runtimes_debug_information`: a `--emit wasm-linked` module holds no `.debug_*` section and computes what the VM computes.
- `fabric_capacity_lock::the_lut_ram_bound_counts_a_lut_copy_per_read_port`: Board A's 8-lane decode step reads a pool through a port a lane, and the LUT-RAM bound is the per-memory ⌈bits/64⌉ × read ports summed, more than one copy, and within the part.
- `d5005_roofline_lock`: the D5005 serving roofline recomputed from the header ledgers — its five rates and their cited facts, the card's stream shares the DDR4 ledger's row for row and each byte priced at the share of the way it moves, the trick ladder, the lanes the plan sizes for its target checkpoint, the full 72-bit beat.
- `d5005_numerics_lock`: the decode engine's arithmetic against independent oracles — the f16 codec, exact sums, both activation scalings, the coded rows against ggml's dequantizer, the block-scale lane against the per-block dot, the transcendentals, the Hadamard rotation, the vector unit's micro-programs, the rotary turn, the int8 cache row and its codes, the largest magnitude, the convolution, and the arg-max folded in any lane order.
- `d5005_engine_lock`: every knob point of the engine within its bar of the reference on the tiny models, batch and prompt-chunk invariance, the lazy flush bit for bit, the IQ4 formats, and the real-checkpoint bars against llama.cpp's runs of a corpus, the corpus one batch: Qwen3.5-0.8B's, and Qwen3.8-27B's a knob point a test against the goldens in `benchmarks/silicon/d5005/goldens/qwen3.8-27b/`, the checkpoint their provenance names held to its recorded SHA-256 and the batched `f32` run to the batch-equivalence record.
- `d5005_rtl_lock`: every unit of the engine in Verilog under iverilog against its numerics function, bit for bit, the vector unit's int8 code unit over every class of operand and inverse scale, the matrix-vector step and the array's column against `numerics::dot` for every format and both word counts, the attention unit's vector and int8 paths against `numerics::attend_vector` and `attend_int8`, and the lazily flushed row engine against `gdn_read` and `gdn_flush`.
- `d5005_program_lock`: the step program on the tiny model, the 27B's header ledgers and the downloaded checkpoints — every tensor of the model's blocks read once a step in layer order with the multi-token-prediction blocks left to the draft, every matrix streamed once, every slot written before it is read, each block's state carried by its own commands, the Walsh–Hadamard transforms exactly where the cache is rotated, and the model running its own knob point's program.
- `d5005_machine_lock`: the machine's jobs over the card's memory computing what the engine model computes bit for bit — every knob point across a lazy flush, every block format the array reads at the real points, a batch of sequences over grouped attention — its layout apart and on lines with every row whole lines and every span on a beat, the 27B's region within 2% of the pricer's held bytes, a step writing only its sequences' regions, a reused region starting empty, each command the jobs of its unit, the sequencer's words decoding to the jobs at every field's extremes and refusing what no job encodes, and the vector unit's instruction words decoding to their instructions.
- `d5005_engine_rtl_lock`: the engine in Verilog under Verilator (iverilog without it) leaving the store's vectors and the card's memory exactly as the machine does — a whole step after the host's embedding at the fastest point, over an eager f32 state and an f16 cache in two words, and in Q4_K; every vector job over f32 and f16 states and f16, int8 and rotated int8 caches; every matrix in every block format the array reads, one activation word and two; every recurrence, eager and lazily flushed through a replay; every attention on the vector units and the array. In `d5005_numerics_lock` an exact power-of-two scale parting from a product only at the least normal, and `vector::Op::Scale` its `ldexp`; in `d5005_rtl_lock` the scale on `logos_simd`. `common::verilator` and the `verilator` probe, a narrow seam of `eda_tools_seam_lock`.
- `d5005_twin_lock`: the cycle twin's weights, rewritten and appended traffic, array multiply-adds and lane cycles equal to the pricer's on the 27B's ledger at every knob point it describes (a lazily flushed one over every phase of its window), a state rewritten only in the step that writes it, the schedule between its serial and unbounded-window bounds and never past the roofline, `benchmarks/silicon/d5005/predicted.json` recomputed, and every header ledger reading as its downloaded checkpoint's architecture.
- `model_quant_lock` holds IQ4_NL and IQ4_XS to ggml's dequantizers.
- `ondemand_exclusion_lock` holds the decode engine's real-checkpoint bars to their own class: `scripts/run-model-bars-tests.sh` runs exactly what `MODEL_BARS_EXCLUDE` drops from the baseline.
- `d5005_flow_lock`: the D5005 accelerator's shell end to end under iverilog (four memory channels on four unrelated clocks, the host channel and the core on two more: the feature header and ID, fills and checks of every channel, a flipped bit found, host-to-memory and memory-to-host copies, an unknown op refused, doorbells past a full command FIFO dropped and counted), each dual-clock FIFO in order under six clock ratios, the stream engine reserving room for every line it reads, every crossing a FIFO and the crossing gate refusing six defects, the accelerator elaborating against the FIM's PIM and refusing a wrong port and a wrong width, the AFU JSON in OPAE's schema with the feature header's UUID, sources.txt as OPAE's rtl_src_config reads it, the C header the register map, the constraints holding every FIFO crossing, and the image recorder refusing a failed fit, timing, user clock or gate and a second record of one image; the bring-up receipt passing unchanged against the RTL through the VPI backend, the same to the cycle on a second run, failing when the RTL fills wrong, changes a host line, carries another AFU ID or checks blind, compiling against OPAE 2.12, and its recorder refusing a receipt that did not pass or ran on an unrecorded image; the receipt passing on DDR4-timed channels, every channel's log replayed through the controller model; the image loader against the RTL at both line widths, its image read back equal to its files with every check word holding, a damaged line found in its tile and the tile loaded again, and an image of another line width refused. Its testbenches and host-memory model are `crates/logicaffeine_tests/tests/fixtures/d5005_flow/`; the memory channels' models are the emitted ones.
- `d5005_ddr_model_lock`: the D5005's DDR4 grade against the shell's own EMIF configuration, its clocking against JESD79-4's arithmetic and Ramulator 2's DDR4-2400U row, every rule of the checker at its boundary (one clock inside passes, one past is refused by that rule at that command), the controller model's traces legal on every stream shape with its refresh stalls and bus turnarounds exactly what the rules charge, the efficiency ledger recomputed, and the channel model in Verilog under a scripted Avalon-MM master: every line, every command and every read's return clock the controller model's, data read back as its byte enables merged it, refreshes issued while the channel waits, a grade whose tRC outlasts tRAS + tRP, and bursts kept apart at a column spacing shorter than a burst.
- `d5005_weight_image_lock`: the weight image laid in the order the engine's step reads the checkpoint, every tensor byte in exactly one place that `locate` finds, the channels within one tile of each other after every tile, each channel's tiles end to end in the read order, check words that find every flipped bit, CRC-32C's published vectors, the layouts it refuses, the image ledger recomputed, the streaming writer equal to `fill` and refusing a tile out of its channel's order, the tile table, and, where Qwen3.8-27B's Q4_K_M checkpoint is on the machine, its layout the header ledger's.
- `d5005_image_stream_lock`: the image stream engine under iverilog on the shell's own channel path, every beat of every tile equal to its software twin's under random waits, gaps and stalls at both line widths, the last beat keeping only rows, a flipped bit in a tile's rows, padding or check word caught in that tile and no other, the channel's line rate held across 511-line tiles, the twin's beats the rows `Image::locate` finds, and the engine's CRC equations CRC-32C's.
- `hyperreal_scales`: several ordered infinitesimals — declarations, order through every declaration, arithmetic and printing, the real closure, germs, flows across infinitesimals, supertasks nested to three levels and in sequence, standpoints, residues of several generators under every limit rule, infinite integers of several and counts infinite in several, with every refusal by its words. `supertask_strategies`: nested supertasks that read the outer clock or change what the passes carry, a pass value its supertask reads, and exact sums of polynomial maps. Units for `multi`, `closure`, `expansion` and `ordered_field`, `tower` (several residues, integer parts of several, their encoding), `supertask::stage` (a binding left in the pass); tier-parity probes for germs, residues and integers of several and for nested and several-count supertasks; the value-model pool and the wire's representatives gain germs, dependences and integers of several.
- `fabric_capacity_lock`: the fabric refuses a program past its part's block RAM before a line of RTL (a 768×1024 weight on the Artix-7 100T, naming the part and the resource), reads the budget from the part (the 200T holds the same weight), never refuses a program that fits or a target with no part, counts exactly the memories the printer declares (every tensor memory and pool at the depth and bits the RTL declares, on a weight whose rows take a row pitch and on both decode images; its RAMB18 is the block memories' bits over 18,432, its DSP a MAC an array cell), and — as the routed Board A image used zero block RAM — needs none for the decode step on the 16-bank part.
- `zcu104_sizing_lock`: the ZCU104 sized from its own datasheets — the board holds the XCZU7EV's block RAMs and UltraRAMs and its 2 GB DDR4, the PL reads that DDR at its ports times its clock up to the DDR's own rate, every checkpoint we have is priced from its own GGUF header, a tied head reads the embedding table from the tier that holds it, a rotation table is held as a weight and read by the first attention block, each floor is its bytes over its bandwidth, a checkpoint past 2 GB is refused with its size, and the committed `zcu104_sizing.json` is what the planner computes (biggest-that-fits Llama-3.2-1B, fastest Qwen3.5-0.8B). `part_resources_lock`: every part's figures name their source, the two Artix parts are DS180's and the device the routed decode images were placed on, the fabric presets hold their part's block RAM, and the ZU7EV is DS891's part down to its UltraRAM having no bitstream init.
- `algebraic::tests`: sums and products in six quadratic fields against an oracle built from `√d` and rational arithmetic alone, collapses to rationals (`(1 + √2)(1 − √2) = −1`, `φ² = φ + 1`), two fields still meeting through their resultant, and a thousand sums in the field of `√2` within a deadline.
- `disk_headroom_lock`: `scripts/test/disk-headroom.sh` refuses a floor no disk meets and names the directory, passes a floor every disk meets, and measures a directory not made yet on the disk it will be made on; both full runners call it at 150 GB on their `TMPDIR` and target directory before their first cargo command.
- `heavy_jobs_lock`: `scripts/test/heavy-jobs.sh` names and refuses every heavy tool by the pid of `sleep` run through a symlink under the tool's executable name — yosys, every `nextpnr-<arch>`, every `quartus_<stage>`, Vivado, the OpenROAD flow's tools, qemu; a shell whose command line only mentions the tools and an ordinary process are not named; both full runners ask it before their first cargo command.
- `supertask_strategies`: a state whose steps grow quadratically, over rational functions of `dt` and of `√dt`, is refused on every tier within 60 seconds. `hyperreal_closure`: a power past the degree bound is refused in the root it counts. Unit tests: `puiseux`'s field arithmetic on one grid and the power refusal's wording.
- `hyperreal_functions`: the exponential, the logarithm and roots of hyperreal numbers on every tier — `exp(dt)`, `exp(2 + dt)` and `exp(√dt)` by their series, `ln(1 + dt)` and `ln(exp(dt))`, comparisons read at the first order two series differ, a germ's rational powers and roots, `√dt` beside `π`, a flow's end state taking a root and a logarithm, flows from a root of `dt` and with a root of `dt` in their rate, and the refusals of the exponential of an infinite value, the logarithm of an infinitesimal, an infinite or a negative value, an even root of a negative germ and a step with a term below `dt`. Unit tests: `germ` (grids, refinement, `exp`, `ln`, roots, flows on a root of `dt`, the bytes of every new node), `ExactReal::nth_root`, `Puiseux`'s grid accessors and the shared term printing, and `tower`'s functions of hyperreals and powers of germs.
- `hyperreal_closure`: the real closure of the hyperreals on every tier — `√dt` and its square, order against `dt` and against standard numbers, polynomials in a root of `dt`, `√(1 + dt)` and its square, standard and integer parts, a radical's constant taken out, an algebraic number beside a root of `dt`, a value printed by its leading orders, roots told apart at a high order, one value reached two ways, rational powers of hyperreals and algebraic numbers, a float base keeping the float power, the refusals of an even root of a negative value and of an infinite root's integer part, and a supertask's end state taking a root. Three semantic probes run the closure on every backend. Unit tests: `puiseux` (Newton's polygon, ramification, separation, every operation, display, equality and hashing, the bytes), `elimination`, `Algebraic::real_roots`, the `Hyperreal` helpers, `tower`'s roots, exact power and transport, and `resolve_division`'s exact power. The builtin catalog gains `exactPower` (wire golden, forge ratchet, guide coverage).
- `pipelined_systolic_lock` P6: every pipelined tile `benchmarks/silicon` ships, array and self-test wrapper, is what the library prints for it; the output-stationary wrapper, shipped and emitted at shapes the shipped tile does not have, reads no more than one row's accumulators in any register and every row word once in its half-folds; and under iverilog its MISR absorbs the XOR of every accumulator exactly two clocks late, with `sig` its parity on every clock.
- `cts_kmeans_lock`: the overlay's clock-tree k-means exponent is the largest up to 4 whose min-cost flow stays within a signed int, re-derived from LEMON's artificial cost; `cts.tcl` passes it on every flow, before the tree is built; and the overlay's own tclsh tests (`test_cts_kmeans.tcl`) pass, its odb reads among them.
- `rational_number_lock`: a rational is the VM's exact number on the ten backends that compile a program. It covers 31 programs and their folded twins: lists of rationals; counters that gain a fraction; whole items under `/ % // ** ^`, indexes, slices and stores; fractions refused in the VM's words; a record's rational field; pops in a function and from an empty list. A record built without its rational field must match the VM or be refused by name.
- `popped_value_lock`: a value taken from a list, read or popped, computes as the VM's on eleven backends; `nothing` refuses in the VM's words.
- The exact extension's corpus (`tier_parity/exact.rs`) slices a list of its values, indexes and bounds with one, and refuses a fraction at each place.
- Unit tests: `vm::variables`, `vm::numbers` and `codegen::rational`, plus `LogosRational`'s VM rules and the value ABI's slice.
- `defined_reals`: a real a function defines by its digits or its approximations, on every tier — printed, ordered and floored; carried through functions, lists and records; a digit outside its base, approximations that disagree and an answer that is no number stopping the program; every refusal before the program runs, a call of `definedReal` in another form included. `supertask_imamura_paper` gains Proposition 3 and Corollary 4. The extension's semantic probes gain six defined-real programs on every backend, a parallel block's read among them, and two `concurrency/` probes on the engines and the Rust tier: a defined real handed to a task, alone and in a germ, stays with its program; `exact_on_c_lock`, `exact_on_riscv_lock` and `exact_on_wasm_lock` hold the program's function reached under a collection at every allocation and the reader emitted only for a program that defines a real; `exact_on_riscv_lock` a task handed one stopping in the VM's words. Unit tests: `semantics::defined`, `ExactReal::defined` and its enclosure cache, the ABI's defined real and its crossing check, `vm::tower`'s rule for it, the relocatable object's exported names, the C tree-shaking, `marshal`'s refusal of a defined real.
- `supertask_strategies` gains a supertask in each pass of a supertask (`1/dt^2`), a lamp switched in each pass (by residue, declared even, declared odd), a lamp worth `dt²` summed into the program's own `dt`, a pass that reads a name before binding it, a name the pass binds afresh leaving the program's own, and an inner supertask over the outer state still refused; two semantic probes run the nested programs on every backend. A pass that changes a name and then binds it keeps the two apart; one that binds its clock again, or a name a loop's pattern binds, is refused with the reason. Unit tests: `supertask::stage`'s split, `supertask::scope`'s renaming, and `affine_power` on each class, an answer holding the infinitesimal in the program's own scale.
- Locks for the comparative-synthesis bench: `qor_harness_lock` (every LOGOS and third-party candidate folds its kernel's golden; the pipelined K1, K2, K4, K5 and K6 candidates at their latencies and clocks per operation), `qor_dram_lock` (each K7 controller under JEDEC on its PHY model; `ddr2_dfi_ctrl` under random traffic — bank conflicts, byte masks, held-back write data, refresh repaid through saturated stretches — at DDR2-400 and DDR2-533), `comparative_synthesis_lock` (every joined number re-derived from its runs; the Artix router ladder and nextpnr binary record), `qor_recorders_lock`, `qor_sky130_runner_lock`, `qor_power_lock`, `qor_manifest_lock`, `qor_kernel_source_lock`, `sky130_ledger_registry_lock`, `systolic_os_streaming_lock`, and the Plasticine `plasticine_twin_lock` and `plasticine_table4_lock`; `residual_pipeline_lock` gains decision fabrics (every compare's mask exact at every narrow width), pipelines reading their ports and the decision for standard cells (hoisting, folds and native adds exact at every narrow width, the whole chain certified by our SAT at 5 bits from the gate DAG), `gpu_silicon_lock` the streamed SIMT engine; `qor_harness_lock` the K1 standard-cell candidates and the SIMT step on ports, `systolic_os_streaming_lock` the de-skew count, `select_hoist`'s and `simt_ports`' unit tests. `comparative_synthesis_lock` binds every run to the RTL the bench builds now (`every_run_measured_the_rtl_the_bench_builds`), and `qor_recorders_lock` commits a row and the excerpts it names in one hold of the ledger's lock, refusing a row that names other excerpts than its run gives; a K7 traffic run records its CAS latency and the probe's read spacing, and its joined summary carries them.
- `openroad_build_lock`: the bench's OpenROAD is the pinned build with one patch (every source pinned by SHA-256 and copied in, the commit OpenLane at the overlay's base builds, the patch touching only the buffered-net walk and applying to the pinned source), its image named by everything it is built from, and only a bench run or a control flows in it; `qor_sky130_runner_lock` and `qor_recorders_lock` hold the image in a run's flow, so an attempt in another image is run again.
- `qor_sky130_runner_lock` holds the bench's flows side by side: a run's record is its own run's extract whatever another flow wrote, its own flows run up to `BENCH_FLOWS`, a start holds the next only until its container is up, and a kernel runs its searches, iso period and controls `--jobs` at a time; `sky130_ledger_registry_lock` E holds the recorder to a whole, locked ledger write; `openroad_build_lock` the image's architecture tag.
- `openroad_build_lock` D holds a run to name an image beside itself only when its launcher ran one of its own, E a run's IR replay to its own tree and the image it names (OpenLane as released for an unnamed run in `tools/OpenLane`), refusing any other run, and F every ledger entry that names an image to name the bench's; `flow_script_guard_lock` holds a bench run or a control to name its image before the replay and the recorder read the run, and to record nothing when its IR is not anchored; `qor_recorders_lock` and `qor_sky130_runner_lock` hold the IR analysis in a run's flow, so an attempt under another analysis is run again; `qor_sky130_runner_lock` holds a flow waiting for room on the disk and an unrecorded run kept to its logs and reports.
- `qor_sky130_runner_lock` holds several kernels run whole at once with their own controls, a kernel's power blocks for its candidates or only those named, an empty stand-in for each init file a template defaults to and no tool emitted, and a block with a stand-in staged as its own candidate.
- `qor_sky130_runner_lock` holds a search starting at the period named for its candidate, GRT-0232 as congestion and a lint refusal named; `qor_power_lock` holds OpenSTA seeing a checkout's linked trees, a routed netlist's tie cells read with no net, and `all` passing over a missed block; `comparative_synthesis_lock` checks a control by its whole flow identity and holds a stand-in empty.
- The Sky130 ledger sweeps read `benchmarks/silicon/sky130_ledgers.json` (each ledger's class: signoff or comparative) instead of file-name globs. A ledger is registered before its first run and written then with no runs, so the recorder and every sweep find it from its registration; `sky130_ledger_registry_lock` holds every registered ledger on disk and public, and `comparative_synthesis_lock` every ledger a bench script names (`every_ledger_the_bench_names_is_public`).
- `eda_tools_seam_lock` holds a read under a path a narrower probe answers for to that probe (`litex`: `tools/qor/litex-venv`), and such a read makes its binary no reader of the EDA tree.
- Locks for register-stage pipelining: `residual_pipeline_lock` (the carry-save lowering exact on every input tried at 64 bits and on every input at every narrow width, certified by our SAT at 5 bits; generated residuals stream exactly through their pipelines; the schedule within its budget; the placement minimal), `loop_pipeline_lock` (the pipelined loop CGRA returns the original's result under iverilog, exactly its term stages later), `csa_multiplier_lock` (every product at every register placement; `fpu.v` carries the generator's multiplier byte for byte; the Caravel engine elaborates no variable multiply, a board engine only its DSP ones) and `synth_strategy_registry_lock` (every registered block's evidence run was synthesised under its registered strategy, and the emitter refuses another).
- `supertask_imamura_paper`: Lemma 1, the floor function on the hyperreals counted to on standard, limited and infinite values and under a declared residue; Proposition 6, the limit lemma, with its evidence. `supertask_strategies`: the evidence of answers joined. `tier_parity`: the floor of Lemma 1, counts toward an infinite bound and the limit lemma on every backend. Unit tests: `supertask::first_exit`, the engine's guard formulas, evidence joining and the wire's refusal of a nested answer. `phase_partial_eval`: a constant argument read inside a resolved supertask is folded into it, and one a closure reads is never dropped from the signature.
- Locks for the int8 decode on the fabric: `int_tensor_sugar_lock`, `int8_decode_lock`, `value_range_lock`,
  `fabric_integer_lock`, `fabric_banks_lock`, `fabric_lanes_lock`, `fabric_pipeline_lock`, `fabric_fold_lock`,
  `fabric_pool_lock`, `fabric_width_lock`, `fabric_cycle_breakdown_lock`, `fabric_postsynth_lock` (a silicon sim),
  `decode_board_lock`, `decode_images_lock`, `decode_tensor_parallel_lock`, `decode_split_images_lock`,
  `cuda_spatial_general_lock`, `decode_gpu_lock` and `spatial_forward_lock`; and the fabric's unit tests of its row
  pitch (`spatial::lower::fabric::tests`: the word a row moves in, an offset at a pitch, which walks keep within a row).
  `fabric_banks_lock` holds every tensor's image to its memory's rows at its pitch, a zero tensor's included, and
  `fabric_lanes_lock` every member of a pool to reading through every one of its ports.
- `common::require_tool("decode-images")`: whether a checkout holds any bitstream the decode ledgers name, which
  `.gitignore` keeps out of a clone (`common::decode_images_at`).
- `tensor_graph_vjp_lock` holds the integer forms' refusals; `spatial_decode_lock` the decode model's residency order
  and its fewest chips.
- `silicon_antenna_safety_lock::the_rewritten_selects_compute_what_they_replaced`: every incrementer of the
  embedded FPU is `v + cin`, and the engine's overflow test is the sign-extension compare, by yosys's SAT over every
  input, each with a wrong answer refused. `teach_parity` decides the quick guide's integer tensor row.
- `xc7_image_flow_lock`: the netlist and FASM gates refuse the defects they exist for on designs built to have them,
  the image recorder refuses a clock outside the contract or an image its gates fail, the recorder's clock allowance
  is the compiler's, the flow synthesizes as `fabric_postsynth_lock` simulates and reaches yosys with every source of
  each board's image in order (a Nexys A7's, a Nexys Video's with its FT245 bridge, a split chip's), and every openXC7
  patch applies to the pinned nextpnr-xilinx (`common::require_tool("openxc7-source")`). `decode_gpu_lock` holds the
  GPU ledger's recorder to a measured run and its refusals; `decode_images_lock` holds every script an image record
  names to the tree.
- `flow_config_lock::the_global_route_effort_is_a_lever_emitted_only_when_chosen`,
  `caravel_project_lock::each_project_carries_its_own_global_route_effort` (a project's own effort reaches the scripts
  and the macro harden passes it), and the runner's sandbox forwards `GRT_OVERFLOW_ITERS` and `GRT_ADJUSTMENT` to the
  emitter (`flow_script_guard_lock`).
- `mod_wrap_lock`: the VM compiler marks the carried residue `wrap` and the VM trace reports `modwrap`, and `## No modwrap` keeps the magic remainder; a value below its divisor emits no remainder; a sum of two residues answers exactly across `m - 1`, `m` and `2m - 2` on every tier and through the C walker; the C walker spells the conditional subtraction, traces it, and keeps `%` under `## No modwrap`, and its build of the residue loop answers exactly.
- `lean_audit_axioms_lock::an_audited_constant_answers_for_everything_below_it`: a walk given the axioms of an audited constant takes them there and walks nothing below it, and finds what the full walk finds.
- `postgres_protocol`'s two-connection server serves the cancel connection beside the session and closes it after reading the CancelRequest, as a postmaster does, where it served the cancel connection only after the session ended; every frame it asserts is unchanged.
- `germ::tests` matches `GermError::NotComputed` and `StandardOnly` by their order as a `Rational`, and `tower::tests::a_root_of_a_hyperreal_beside_a_computable_real_is_a_germ` asserts the germ a root of `dt` beside `π` is, where it asserted the refusal Phase 7a gave it.
- `fabric_postsynth_lock` fails, naming the places it looked, when yosys is on the `PATH` but its 7-series cell models are not, instead of passing as a skip.
- `tower::tests::a_function_of_the_reals_refuses_what_has_no_exact_answer` takes the natural logarithm of `dt` for a function with no exact answer: the square root of `dt` has one now, `√dt`, asserted on its own.
- `wpe_caravel_lock` pins the 47.62 MHz project: the period the engine's slow corner meets (`the_engine_runs_at_the_period_its_slow_corner_meets`), divisor 413, two engine cycles per SCLK half and the 42 ns read window, with a one-cycle-per-half edge at 21 ns refused for a window shorter than the round trip.
- The WPE on `user_clock2`: `caravel_project_lock` holds every project to a clock the chip is signed for (the 21 ns engine on the core clock refused by name, the core period Caravel's own signoff SDC's), the wrapper timed at every clock the chip runs and its ledger recording them; `flow_config_lock` holds the core-clock ports to the scaffold SDC's and a user-clock macro's bus to `wb_clk_i`, asynchronous and moved off the macro's clock; `wpe_caravel_lock` simulates the macro top around a stub engine (the engine clocked by `user_clock2` alone, its reset asserted at once and released on the second edge from `wb_rst_i` or a driven probe 0, the acknowledge on `wb_clk_i`), the bench's two clocks and the read window's clock.
- `infinichip_silicon_lock`: the SECDED codec equals its oracle on every fault class and the vectors reach every verdict; yosys's longest path holds the encoder to its widest parity's tree and the decoder to two trees and a gate; the infinichip top registers both codecs; the shipped infinichip and SECDED benches pass.
- `sky130_provenance_lock::a_run_records_the_image_its_launcher_names_and_only_then`: on a real signoff run and a linked mirror of it carrying an image marker, the recorder records the marker's image and changes nothing else, records none for the unmarked run, and its verifier flags an entry naming an image its run no longer carries but holds no unnamed entry to a marker.
- The `cuda-gpu` group serializes `decode_gpu_lock`, `spatial_forward_lock` and `spatial_section_accumulator_lock`.
- `benchmark_logos_tiers_lock::every_benchmark_built_through_rust_prints_its_expected_bytes` gives each of its four workers a target directory of its own: cargo holds a target directory's build lock for a whole build, so the 35 release builds that shared one ran one at a time (1,382–1,497 s in three full runs, past the 1,800 s backstop in the fourth; 674 s now under load). The builds and their checks are unchanged.
- The Lean imports of T00-017's export (`original_dependent_terms`' two replay tests, `vernacular_roundtrip::fixture_t00_017_round_trips`) start with the wall-clock heavies under a 60-minute backstop (`.config/nextest.toml`): one `omega` proof in the export costs most of a 254–620 s import, and the full run's 30-minute backstop terminated one of them in three of five runs.
- `public_manifest::git_ignored` answered ignored for an untracked file a negated `.gitignore` rule keeps public; it reads the deciding rule now (`scripts_root_lock::a_path_a_negated_rule_keeps_is_not_ignored`).
- **`tensor_graph_vjp_lock`.** Every reverse-mode rule of `tensor_graph::vjp` is checked against central differences on the VM in f64: each op alone (head slices, joins whole and across heads, and per-head matmuls among them), a tensor read twice, a two-layer MLP, scaled attention scores, an attention core, a pre-norm residual block, an RMSNorm over a fused residual, weightless and per-head norms, a per-head norm between two projections, every norm at an epsilon its program gives, full, grouped-query, up-to-position and causal-block attention, attention at scores far outside `exp`'s range, rope over full and partial widths, embedding rows that repeat an index, a row write, and an input the loss does not reach; and every refusal is held by name (argmax, rope tables, integer indices, causal convolution, decay, gated delta, mixture of experts).
- **`c_temp_names_lock`.** Every prefix the C emitter numbers a temporary with, read from `codegen_c`'s source, is checked against the macros `cc`, `gcc` and `clang` predefine for a debug build's default target, a release build's `-march=native` and 32-bit x86: none is a prefix followed by a counter (GCC and clang define `__k8` and `__i386`).
- **`compiled_main_stack_lock`, `oracle_seeded_build_length`, `repeat_range_lowering`.** A compiled program's `main` switches stacks and starts no thread, a running one has one thread, recursion 47 MiB deep runs, and a run past the stack's end shows its output, the call-depth sentence and status 1; a seeded DP table and the coins benchmark index unchecked while a read past the table keeps its check and fails alike on every tier; `Repeat` ranges compile half-open exactly where `end + 1` exists and answer alike on the tree-walker, the VM, AOT-Rust and C at the largest Int, for an empty range and for a body that sets its variable, every run under a deadline. Unit tests for `logicaffeine_system::stack`.
- **`affine_equality_bounds_lock`.** A merge sort's merge store and the mergesort benchmark's compile unchecked, and the sort answers alike on every tier; a result too short for the merge, an index that does not move in step, a recursion that grows its argument, and a counter that may start past its bound each keep their checks and fail alike on every tier; a push through another name and a map store answer alike. Unit tests for `optimize::karr` (hulls, increments, the merge loop's `k = li + ri - 1`, a counted build's `length = i - 1`, exit meets, column order). A loop carrying forty names in step compiles within 30 seconds and prints alike; the tiny Qwen forward lowers to CUDA within 6 GiB and two minutes, watched as it runs; unit tests for `AffineSpace::solved` (each substitution exact, the top solves to nothing).
- **`cold_start_lock`.** The calling thread's allocations against the library's size: splicing `Show 0.` and establishing its library provenance allocate under 1/64 of it, splicing a program that calls `trim` under three times what it splices plus 1/64, finding that program's library functions likewise, and interpreting `Show 0.` under the library's size. The embedded stdlib index equals `stdlib_text::index` of the files `PRELUDE_FILES` names, each module's pulls rechecked name by name and every library section found by lookup; the whole-word prefilter hides no use in any program under `examples/` or `benchmarks/programs/` or any module; a program that declares one of a module's natives keeps the module out even while calling its other functions, so no native is renamed; and a program calling any function of a module that calls `text_from_bytes` gets `strings.lg`'s `validUtf8`. `stdlib_text` unit tests: section ranges against the line rule on every spliced module, line runs, source ranges, whole words, a three-module index, and a library section the splice cuts stops the build.
- **`bit_pack_lock`.** A sieve's flags compile to one bit each and count alike on every tier, pushes and stores across word boundaries read back on every tier, an index from the back and the length agree, a read past the end fails naming its index, and flags handed to a function, shown, iterated, returned, copied, rebound or popped keep their `Vec<bool>`; `Opt::BitPack` switched off keeps the bytes. Unit tests for `LogosBits`. `phase_opt_trace` sees it fire.
- **`fold_fusion_lock`.** binary_trees checks every tree through `check_of_bottomUpTree` and `Main` builds none; it prints alike at depths 0 to 10 on every tier and against C, fused and unfused; a tree of payloads sums without being built; an arm that drops a subtree, a field that can fail, a tree used twice and a consumer that calls out keep their trees and answer alike; `## No foldfuse` on the consumer or the builder keeps that pair apart; a builder that divides by zero fails alike. `phase_opt_trace` sees it fire. `c_collector_lock` runs its benchmark corpus and its enum churn with fold fusion off, so they still build the trees they hold the collector to.
- **`sort_nest_lock`.** The bubble_sort benchmark's nest sorts by a heap sort and prints alike at sizes 1 to 200, failing alike with no items; nine spellings of a sorting nest (an exchange through one local, descending, the comparison reversed, equal items exchanged, every pass over the whole window, `Repeat` loops, the pair below the counter, a window inside the sequence, more passes than needed) sort by heap and print alike at nine sizes on every tier and against C, sorted and unsorted; a `While` counter ends where the loop leaves it; a function sorts its argument and `## No sortnest` opts it out; passes counted from the length take the branch their guard allows; ten nests outside the shape keep their loops; Reals keep their nest; a window past the end fails alike. `phase_opt_trace` sees it fire, and `benchmark_collapse_lock` holds bubble_sort's collapse note to "heap sort" and binary_trees' to "closed form".
- **`divisor_sieve_lock`.** The primes benchmark reads a sieve and counts alike at thirteen sizes from 0 to 10007 on every tier and against C, sieved and unsieved; nine spellings of the search (`d <= i / d`, zero on the left, divisors from 3 and from 7, a range from below zero, a `Repeat` loop, an exclusive end with two Sets, inside a function) read the sieve and print alike; nine searches outside the shape (the divisor read afterwards or kept, a step of two, a bound short of the root, a count of divisions, an otherwise, no break, divisors from 1, a statement between) keep their loops; a range past the cap runs as written. `benchmark_collapse_lock` holds primes' collapse note to "sieve".
- **`trace_memo_lock`.** The collatz benchmark keeps a table of finished starts and counts alike at nine sizes from 0 to 5000 on every tier and against C, memoized and walked; collatz from 5, two per step, a greater-than test, digit sums down to one digit, an accumulator only some steps add to, a `Repeat` loop and sums past what an entry holds keep a table and print alike; a step that reads the start, a test on the accumulator, two accumulators, the walk's end read afterwards, a walk that never shrinks, a negative step and a show inside the walk keep their walks; the guard bounds the table at 2^24 starts. `benchmark_collapse_lock` holds collatz's collapse note to "finished start".
- **`probe_fuse_lock`.** The collect benchmark looks each key up once (both tests of the map are `if let Some(…) = m.get(…)`, no `logos_contains`), its hash arithmetic stays on machine words, and it hashes alike at eight sizes from 0 to 4096 on every tier and against C, fused and unfused; an inline read, the read written as the branch's first `Let`, two reads with an otherwise, a read in a loop of the branch, a read under a nested test, a map of `Bool`, a literal key, a `Text` key, a read of another key beside it, a read of the key after the branch and a map parameter inside a function are one lookup and print alike; a branch that writes the map, one that moves the key, a key that is an expression, a read of another key, a branch that reads no value and a map of `Text` keep both lookups.
- **`bits_stride_fill_lock`.** The sieve benchmark crosses off every prime's multiples through `fill_stride_through_unchecked` and stores no bit one pass at a time, and counts the known number of primes at twenty-one sizes from 0 to 65537 on every tier and against C; five strided loops (strides 3, 7, 64 and 65 and a stride the Oracle knows from a guard, `at most` and `less than`, stores of `true` and `false`, offset indices) fill by stride, and with a sixth that never runs print alike, with each counter's exit value, at nineteen sizes and five strides from 0 to 2^62; a stride the Oracle knows only to be at least 0 (its stores proven, but a stride of 0 never ends), a store it cannot prove in range (failing alike at item 13) and a loop with more to do keep their stores.
- **`LogosBits` strided-fill unit tests** (`logicaffeine_data::types::logos_bits_tests`). Every step from 1 to 140, ten starting offsets, six counts and both values over eight lengths store exactly what one store at a time stores and nothing else; a fill to the last item leaves no bit past it; a fill through an offset counts its steps.
- **`emitter_statelessness_lock`, extended.** The probe binding counter (`PROBE_ID`) is reset for every program and its stack (`PROBES`) is statement-local; a program of two fused map tests is numbered alike however often it is compiled and in any order among the other programs.
- **`e2e_codegen_optimization`, ported.** A loop counter kept after its loop is bound `let mut i: i64 = 5;`, typed as every emitted binder is, and the bubble pass's swap, whose indices the range analysis now proves (`j` is at least `i + 1` inside `While j is greater than i`), reads through `get_unchecked` into `__cs_e1` where it read through the checked `__swap_tmp`.
- **`push_i64` unit tests, extended** (`logicaffeine_data::ops::itoa_tests`). Twenty thousand values of every digit count and both signs append exactly what `Display` writes after text already in a string, with no spare capacity, one byte and 64.
- **`loop_counter_exit_lock`, extended.** A counter whose `Let` a fill lowering consumed squares past 2^63 exactly in a function under an `If`, and the sieve's store loop steps on checked arithmetic outside the version proven to fit and counts alike at four limits on every tier and against C; both failed before the counter kept its type.
- **Range narrowing unit tests** (`abstract_interp::domain_tests`). Random intervals and values for every comparison, holding and failing, with a name and with a literal: no pair that satisfies the comparison leaves the narrowed intervals; and a name compared with a name is bounded by its interval.
- **`bitwise_dual_lock`.** nqueens isolates its lowest set bit with one checked negation and an `&` on machine words, its exact form only out of line, as shipped and at sizes 1 to 9 on every tier and against C; twenty-four bit operations over unbounded chains (both sides, negative literals, a `1 << k` leaf, a division in the chain) answer alike; past the word a big operand beside a low mask keeps its low bits and every other refuses in the operator's words; a call in an operand runs once and in order; and a call inside an exact chain takes an `Int` argument narrowed, the chain around it exact.
- **`bounds_hint_version_lock`.** counting_sort's checksum loop runs a copy that assumes its hint and one that assumes nothing, and prints alike at eight sizes on every tier and against C; a loop reading past its list's end fails where every tier fails; only the innermost hinted loop is versioned.
- **`branchless_select_lock`, extended.** Two arms over the same leaves test their guard once.
- **`benchmark_fairness_lock`, extended.** No benchmark script runs a compiler of its own (a detector, checked against the shapes it must see and the lines it must pass, scans every script for a compiler run as a command); `suite_build`, run on a fixture with logging stub compilers, builds the registry's exact command line, stamps the artifact with it and the toolchain, reuses it only while both hold and it is newer than its source (a LOGOS artifact also newer than largo), rebuilds under `FORCE_BUILD=1`, stamps a directory artifact beside it, and leaves nothing behind when a build fails or produces nothing; collect and two_sum hash by the golden-ratio multiplier and use no standard map or set in any of nine languages; the README's build table and every compile-time label state the flags `languages.tsv` passes. `benchmark_registry_lock` holds every timing in every benchmark script, not only `run.sh`'s, to the performance cores, and `benchmark_collapse_lock` holds what each collapse note says about each LOGOS row to that row's own optimization trace.
- **`loop_counter_exit_lock`.** A reused counter set inside an enclosing loop and shown after it prints alike on every tier and against C for a range loop (inclusive and exclusive), a whole copy, a slice, two runs of copies, a run continued from a counter set before its sequence, a sequence copied into itself, the tiled matrix nest (which runs tiled) and a substring search.
- **`wide_accumulator_lock`.** A counter read only through an item's index stays narrow, builds and prints what the VM prints.
- **`variant_box_lock`.** A recursive enum declares one box per self-holding variant and none per field, a fieldless variant is a plain value, and `Opt::VariantBox` switched off boxes each field; a tree, a generic tree, a cons list built in a loop, an expression tree compared and copied, and trees inside a list and a record count, show and compare alike on every tier; the binary_trees benchmark boxes its nodes whole. `phase_opt_trace` sees it fire.
- **`branchless_select_lock`.** A two-armed assignment whose odd arm keeps `3 * k + 1` exact behind a guard selects between both arms, every tier takes the same step on each side of the guard's bound, an arm that divides by a variable or reads an item is never computed ahead, a branch that assigns two names keeps its branch, and the collatz benchmark selects its step; `Opt::BranchlessSelect` switched off keeps the branch. `phase_opt_trace` sees it fire.
- **`mod_wrap_lock`.** A carried residue and a value below its divisor compile without a division on AOT-Rust and C, a value that may reach twice the divisor or fall below zero keeps its remainder, every case answers alike on every tier, and the fib_iterative and coins benchmarks carry their residues without dividing; `Opt::ModWrap` switched off keeps the remainder. Unit tests for `optimize::modwrap`, and every Int table stays at most half full presized or grown.

- **`riscv_heap_lock`.** A program that allocates more than its address space while holding little runs to the VM's output with its heap report bounded; trees, records, maps, text, nested lists and tasks holding trees across yields print the VM's output with a collection before every allocation (`LOGOS_GC_STRESS=1`).
- **`item_growth_lock`, and parity sweeps that cannot pass vacuously.** The item-growth guard and its raw additions on AOT-Rust and C, `## No accumbound`, and four programs on both sides of a guard against the VM on every backend; `optimize::item_growth`'s unit tests pin the growth and the bound for a count and a rolled row and refuse a doubling item, a sequence a call receives, an increment a nested loop repeats and a negative one. `tier_parity` refuses a suite without `tier_parity_worker` (`missing_worker`); `c_word_lock` and `c_set_iteration_lock` declare theirs and now compare their isolated rows.
- **`mask_narrow_lock`, `exact_bitwise_lock`, `optimization_guide_lock`.** A masked chain's generated Rust and C form, its gate and its refusal of an unbounded chain, and every width, operator and unbounded chain against the VM on the tree-walker, the generated Rust and both C backends; the runtime's exact bit operations against `semantics::arith` over words, masks and big integers of both signs; the guide's keyword table against the registry, group by group. `strength_reduce_pow2_lock` gains a remainder of a negative dividend; the oracle's interval domain tests hold the new `&`, `|`, `^` and `<<` transfers against the reference on random intervals; `codegen::lowbits` checks every narrowed chain against the reference arithmetic whenever its proofs hold.
- WASM collection and global-loan locks check exact values, independent copies, complete reclamation, and absence during mutable loans; the feature gate includes every new suite.

- `c_word_lock`: thirty-two programs, each first checked to reach the backends (a program the front end refuses would fail alike everywhere and pin nothing), hold every machine-word rule (display, ring wrap, unsigned divide and order, shifts, conversions, word parameters and results, a word handed to a task, sent through a pipe, keying a map and filling one, words held in a list, a set and a map, read back, shown and iterated, and the three refusals) on both C backends against the VM.

- `c_set_iteration_lock`: a loop over a `Set of Int` or `Set of Text` binds each element once in insertion order, skips a removed one and runs no pass over an empty set, on both C backends against the VM.

- `wasm_batch_reclamation_lock` covers root elision: a copy-on-write collection operation re-syncs its register, a callee that only reaches a safepoint through another function still syncs, and a function that cannot reach one skips the sync. `device_lift` and `cshake` units cover macro-only allocator references and multibyte allocation routing.

- `failure_boundary_lock`: the runtime catches a panic only through `failure::caught`; a generated program ends on a failure no boundary catches; a served program's release project unwinds; both project writers take the panic strategy from the compiler; and a release server answers a failed render with `500` and keeps serving.
- `stepped_tier_lock` gains three programs whose integers outgrow a word — a waiting function returning one, a waiting function taking one, a task launched with one — and runs them on both C tiers collecting at every allocation. `native_pe_wire` checks the cdylib entry's parameter against its shim on the generated source.
- pow_numeric_type_lock checks exact integer returns, real operands, rejected real-to-integer returns, and program-defined power signatures.
- Semantic probes retain optimized-VM parity for dynamic returns and separately require typed C to reject contradictory return annotations, with mutation controls for both boundaries.
- Optimization trace checks cover fixed buffers and retained ownership in every emitted word and exact region, checked text indexing, and the independent bounds proof when branchless accumulation is disabled.
- C collector and scheduler tests bound retained memory, check task retirement under AddressSanitizer, and reject retained-reference and early-free mutants. Exact record regressions require byte-identical VM and C output for small and unbounded integers.
- JIT call-exit regressions cover exact overflow, bounds errors, callers inside functions, and recursive native frames, with allocation-scope checks and a frame-discard mutation control.
- WASM collection-copy regressions compare integer and text maps against exact VM output, with assignment and mutation-isolation controls.
- Batch ownership checks cover text, list children and record fields returned implicitly through mutable parameters. Exact-modulo allocator checks isolate owned remainder storage across tenfold workloads.
- Mutable-call regressions cover integer, float and text restoration; CFG liveness controls cover branches, backedges, implicit results and calls. Exact C floor-division heap checks release input/result wrappers and compare retained bytes after tenfold workloads.
- C map regressions cover Fibonacci beyond the signed-word boundary, overwrite/copy/iteration behavior, signed word edges and typed calls. Bounded-loop execution traps exact-integer helpers, with an injected-helper negative control; overflow workloads retain exact-output parity. Count-proof and WASM Nothing controls cover nested work, effects, inclusive limits and large exact operands.
- **What the optimizer knows after a loop is what holds after it.** `loop_exit_facts_lock` holds nine programs whose answer turns on it (a counter to a run-time, literal or parameter bound, one that never runs or starts past its bound, an exit by `Break` from a `While`, a `Repeat` and a nested loop, a value written just before the `Break`) to their literal output on the tree-walker, the VM over the raw compile and over the run path's optimized program, `largo run --interpret`'s entry on the tree-walker, VM and JIT engines, and both C tiers. `tier_parity::semantic` family `loop_exit/` puts five of them on every backend, the generated Rust under both shipped profiles included. `rich_walk_tests` holds the fact walk's exit after a `Break` (the value written before it survives, the condition promises nothing) and its precision after an exit through the condition; `loop_shape`'s units hold that a loop a `Break` of its own can end is not counted, and that a `Break` of a nested loop leaves the outer one counted.
- `c_bytecode_word_entry_lock`: fifteen programs cross a bytecode-C word entry every way a value can — a word in and out, an escape inside the callee, deep in a recursion, at the outermost call and before a word, inside a loop over a set and while the function owns a list, an exact argument and a parsed integer taking the tagged entry, odd and negative words, Text results, mutual recursion, an escape returned through a word caller — each run as on the VM; and Takeuchi's calls on machine words tag, test and strip nothing, its word entry carries no part of the region held exact, and its tagged entry only enters one or the other.
- `nextest_schedule_lock`: in every profile, each `cpu-timing` lock requires every test thread and outranks every other override that sets a priority.
- **Every net within the fanout limit, every runner setting live.** `fanout_within_limit_lock` holds the OpenLane overlay to the fanout limit with antenna diodes counted as loads: the clock tree's clamp and clone, the diode-aware split (whole gates, guarded centroid buffers, the critical gates carved onto the driver when no whole bin fits), the ECO's limit, and the post-diode upsize on the flow's own route that never repairs a held net. `flow_script_guard_lock` runs the real `openlane-run.sh` in a sandbox and holds each setting it accepts to a forward or a refusal; `flow_config_lock` pins the strap-pitch and hold-margin levers; `jog_router_geometry_lock` pins the neck refusal and `antenna_diode_credit_lock` the stacked jog. `soft_core_op_support_lock`, `universal_completeness_lock` and `soft_core_fuzz_differential` hold `text_bytes` (chip opcode 237) to the VM, and `soft_core_width_parity` runs every benchmark to the VM at width 64. `fanout_within_limit_lock::a_held_wire_at_the_strongest_drive_is_rebuffered_or_its_longest_arm_halved` and `fanout_within_limit_lock::the_diode_rules_have_one_home` pin the upsize's last remedies and the one file both diode-aware steps take their rules from. `fanout_within_limit_lock::the_split_has_one_home_and_both_steps_call_it`, `fanout_within_limit_lock::a_diode_belongs_to_the_pin_it_was_placed_beside`, `fanout_within_limit_lock::no_edit_leaves_a_diode_behind` and `fanout_within_limit_lock::the_route_detailed_routing_follows_is_judged_for_antennas` pin the shared split, diode ownership, the left-behind check and the antenna judgment of the final route.
- **A literal reads only as the constructors it names, in both kernels and the core.** `type_checker`'s and `recheck`'s units hold the bridge, the index unifier and the match on a literal to `Succ` assumed, rebound into `Bool` or taking two fields, `0 ≡ Zero` holding while `Zero` is `Nat`'s and `1 ≡ Succ 0` while `Succ` is, beside a third constructor; `dag::tc` holds the core's match; `context::arithmetic_attestation_cache_controls` holds `Context::canonical_numerals` to every registration and to a clone's registries; `kernel_observational` and `dag_observational` hold `Obs` on a literal in all three kernels, a literal no reading holds for clashing with no constructor; `theory_logos_parity` adds `Succ` rebound, assumed, or universe-polymorphic. Seventeen mutants of the gates: fourteen killed by the lock named for each, three equivalent (the core's recursor ι, unreachable in LOGOS's theory, and the two observational gates, which the rules' own family check subsumes).
- **A bare reference carries its level, in both kernels and the core.** `kernel_paradox_corpus::a_bare_reference_carries_its_level_into_the_declaration_around_it` refuses the proof of `False` through `P2 : Sort u := B2` and `B2_rec.{1}` over an implicitly polymorphic `B2`, with the two halves admitted at levels of their own and a reference in an argument's place unfolding at the instance. `U_universes_extra` §19 holds both kernels to a bare non-head reference at its declaration's own levels and no other, a bare head to the instance its arguments solve, and a definition's telescope to its type's levels then its body's, a stated one refused when it binds fewer. `theory_logos_parity` adds the paradox's two steps and their controls, a body-only level and an implicitly polymorphic family put to the core as its door puts it; the harness builds every candidate through `dag::logos_env::{candidate_definition, candidate_block}`. Thirteen mutants of the rule are each killed by the lock named for it.
- **`theory_logos_parity`: the quotient a kernel installs computes in both kernels.** `kernel.quot.lift … (kernel.quot.mk … (Succ Zero))` is `Zero` by computation, the quotient installed through the vernacular's `Quotient` commands over a canonical equality of its own; four mutants of the rule are killed by it.
- **Observational seams that can fail.** `kernel_observational`, `dag_observational` and the unit seam tables gain an indexed family with independent fields (`Tagged`), an index read as a parameter (`flag false : Flag true` never `flag true`), a field typed by an earlier field (`Dep`), an `Obs_cast` look-alike, the exact wrong reduct of a proposition compared by its former, a squash matched without naming `SAnd`, and a certification after a cached verdict; every seam fixture asserts the names it declares, and the harnesses count a pending declaration as refused. `dag_mirror` syncs from a 256 KiB thread; `theory_logos_parity` holds structure η in both kernels, and the shadow controls cover a downstream refusal, an unchecked root and a context with local bindings.
- Roadmap generation takes a release's curated Highlights lead even when a later fix is written before it, while releases without Highlights retain their first-entry headline.
- **The board hosts run from a kit, on Linux and macOS.** `logos_chip_lock::the_port_resolver_names_each_board_channel_on_linux_and_macos` resolves both boards' channels from a Linux listing and from both macOS drivers' listings, preferring the callout node; `the_loader_runs_under_sudo_only_where_the_os_needs_it` pins the loader argv per OS; `a_claim_refused_on_macos_is_retried_under_sudo_and_nothing_else_is` drives a stand-in loader and sudo to hold the one retry macOS needs when its FTDI driver holds the JTAG channel; `persisting_an_image_names_the_boards_part_and_reports_a_write_that_did_not_happen` holds the configuration-flash write to each board's FPGA part and to failing when the loader exits 0 without writing; `a_board_kit_runs_with_no_compiler` holds an exported kit's protocol table, streams and goldens to the compiler's, and runs its host with no cargo on the PATH; `a_board_kit_drives_both_boards_with_no_compiler` runs a kit on attached boards. `board_host_portability_lock` holds every script under `scripts/` to finding and flashing its board through `scripts/lib/ftdi_ports.py`: no Linux device node, no udev lookup, no loader invocation of its own, and every host that opens a serial port imports the resolver.
- Native helper controls preserve frame inputs, aliased results, caller-saved integer and float registers, and exact state at a rejected operation (regalloc::tests::host_calls).
- **The reflection algebra stays defined.** `tcb_ledger_lock::the_reflection_algebra_is_defined_never_assumed` holds the seven operations to checked definitions, and the ledger no longer lists them; `reflection/tests.rs` locks their attestation (a changed definition or `Syntax` is not accelerated), the native accelerator against the definition on a generated corpus, and the independent checker computing them by the definition.
- **`SProp` storage.** `sprop` admits an `SProp` family storing a type and a squash at any universe, beside a predicative family refused for the same field; `kernel_paradox_corpus` refuses reading the stored type back out (Coquand's impredicative `Set`); `theory_logos_parity` holds the core to all three; the gate's two doors are controlled directly (`type_checker/constructor_universe_controls`).
- **Observational equality, in both kernels and the core.** `kernel_observational` locks each computation rule by a declaration that needs it, in both kernels (function and propositional extensionality, constructor clash and fields, heterogeneous fields, type formers, canonicity through `Obs_cast`, quotient classes and `Quot_sound` as a theorem, `Eq` and `Obs` each implying the other) and each seam by a declaration both refuse (two proofs, a proposition seen as a type, Hur's family, a cast along `SFalse`, between propositions, along a stuck type, of a variable, of an indexed or dependent value). `dag_observational` puts the same declarations to the `dag` core beside the `Term` kernel. The unit controls hold each guard stuck beside a control that steps, in each kernel, and the certificate's revocation, impostors and accounting; the admission-log lock covers the observational certificate's doors.
- WebAssembly exact record controls cover cross-frame copies, repeated safepoints, nested records, mixed fields, whole-value formatting, exact host ownership and bounded allocation. The allocation census includes integer cells and their buffers (wasm_exact_record_storage_lock, wasm_alloc_site_lock).
- C text-byte view regressions compare output with both reference interpreters and the disabled optimization across repeated function-local names, mutated sequences, and nested shadowing (c_text_bytes_scope_lock).
- WebAssembly ownership controls require exact holder census, reclaimed storage and bounded heap growth across repeated page turns and batch loops, including nested callers, returned children, host callbacks, maps, records and word lists. The million-turn soak preserves its exact census and growth bound (wasm_batch_reclamation_lock, wasm_heap_leak_lock, wasm_closure_reclamation_lock, wasm_heap_soak_lock).
- **The core admits a Lean import's context whole.** `theory_logos_parity` replays each `lean_scale` original's imported context into the core declaration by declaration within a deadline (Omega, `Nat` comparison, string literals, division and its impostors, `Nat.Linear`, `Nat.modCore`, projections), and `a_declaration_that_builds_a_structure_value_waits_for_the_projections` a hand-written structure recursor that names no projection. `dag::tc::tests` holds the core's `match` to its motive when only inferring (`inferring_only_reads_a_match_type_off_its_motive`); every validated `Nat` operation of Lean's import, a validated `Nat.beq`, and Lean's division programs to native computation with no δ-unfolding, and division at 2^128 and by zero to conversion (`a_validated_lean_primitive_computes_natively_in_logos_theory`, `a_certified_division_program_computes_natively_in_logos_theory`); programs shaped like the modulus to their own values (`a_program_shaped_like_the_modulus_is_never_computed_as_one_in_logos_theory`); and a followed context's simp rule to withdrawing the certificates (`a_kernel_that_follows_a_context_reads_its_certificates_again`). The tests over an import fail past a deadline rather than hang.
- **`theory_logos_parity`: the `dag` core in `Theory::Logos` decides what the `Term` kernel decides.** Every declaration of the standard library is replayed into the core and admitted; each paradox of `kernel_paradox_corpus` (Coq 8.4's `propext` × guard, Girard through a stored universe, Russell through positivity, large elimination of a proposition with data or two proofs, `SProp` elimination, a non-decreasing fixpoint, a family that may be a proposition eliminated into data, a constructor at the wrong family or parameter) is put to both kernels after the same setup and refused by each by its rule, beside controls both admit (among them a universe-polymorphic family's auxiliary at a use site). Each core rule switched off is killed in a private copy (15 of 15, logs/arena-crown/stage-d/mutants/). `dag_levels`' evaluator gains the `LevelNode::SProp` arm its generator never reaches.
- Root-propagation controls count symmetry-discovery entries, validate SAT models and UNSAT certificates against original inputs, and check every two-variable CNF formula. The existing Marlin fragment-map certificate exercises the resulting hardware proof path.
- **`kernel_tactics`: tactics over the kernel's own goals.** Dependent goals the first-order tactic language cannot state, each proof checked by the kernel: `intro`/`exact` on `Π(A)(x : A). Eq A x x`; `apply` leaving premises in order; `rfl` by conversion; induction, `change` and rewriting proving `plus n Zero = n`; induction over the indexed `Vect` with its index generalized; rewriting under a binder; `cases` closing `Eq Nat (Succ a) Zero` and splitting with dependents carried; refusals (a non-function `intro`, `exact` at another type naming both types, `qed` naming open goals, a non-hypothesis index) that leave the state unchanged; scripts, their failing step named; the vernacular `Theorem … := by …`, published, cited, refused when unfinished, and printed back to itself.
- **`kernel_core_extra::both_kernels_read_a_constructors_indices_at_the_scrutinees_parameters`**: a `match` with no case on `h : Eq Nat (Succ a) Zero` types in both kernels and `double_check` agrees, while at `Eq Nat a Zero`, with `a` open, both owe `refl`'s case.
- **`dag_mirror`: the core's mirror ends where a replay ends.** Every route (current, advanced, rebased on the standard library, replayed) and after each a comparison with a fresh `logos_env::replay` of the same context, constant by constant and verdict by verdict; a certifier's one-theorem-per-context pattern never replays after the first; a REPL session advances command by command; a redeclaration, a held name registered again as a constructor of a new family, a constructor added to an admitted family or to a held indexed family, a declaration waiting for a name and a context that comes to read `Int` literals as data each restart.
- **`context_admission_log_lock`**: every write to the registries a declaration lands in goes through a logging door (`Context::record`, the removals of `displace` and `invalidate_certificates`, the certificates' installation), with controls that the scan refuses each unlogged write and admits the doors. White-box: `context::admission_log_controls` (every registered name of the standard library logged, one removal between a redeclaration's registrations, certification and revocation logged, a clone sharing the log, a long log dropped in constant stack).
- **`derived_auxiliaries_extra::a_polymorphic_familys_auxiliaries_are_polymorphic_in_its_levels`**: every auxiliary registered for `Big.{u}` is polymorphic in `u` and names the family with its levels written out, `Big_below` types at two instances in both kernels, and a monomorphic family's pair stays monomorphic.
- **`prelude_registered_once_lock`: no production source registers the standard library again.** Every package's `src/` but the test crate's is scanned for `StandardLibrary::register(` in code; production asks `StandardLibrary::context()`, borrowed or cloned. A control holds the scan to finding a registration and only a registration.
- **`lean_conformance_defects`: the declared type itself is checked.** A definition at an ill-typed declared type and an axiom at a term that is not a type are refused, a declared type naming a later constant waits for it as a pending obligation, and every declaration of the standard library has a type the kernel types (the bit-vector equations through the convoy reading), a census that first holds the prelude whole (nothing refused or owed, its subject `bv_and_eq_def` present), so a declaration refused at registration cannot pass it by being absent.
- **`lean_conformance_t04_recursion`: recursion through a convoy and fixpoints checked against their position.** An accumulator threaded through `(match l … ) n` is structural on `l` in both kernels, computes, and stays stuck on an open list; recursing on the threaded argument is refused; a fixpoint whose type no motive states (a `let` around its `match`) checks as an argument and as a definition's value in both kernels.
- **`kernel_paradox_corpus`: the known paradoxes of a kernel of this shape, written out in the kernel's vernacular, each refused by the rule that must refuse it.** Coq 8.4's `propext` × guard inconsistency (a value cast along `(False → Tw) = Tw` taken for a subterm), Girard's paradox through a constructor storing its own universe, a universe member of itself (directly and through a universe-polymorphic instance), Coquand's paradox and LOGOS's proof-irrelevant conversion through large elimination of a proposition with data or two proofs, `SProp` elimination into `Prop` or data, Russell's paradox through strict positivity (directly and through `TList`), a non-decreasing fixpoint, the refutation refinement's closedness, a universe-polymorphic family that may be a proposition eliminated into data (plain and indexed `match`, refused by both kernels, with the elimination at the family's own level, a subsingleton and a never-`Prop` family as controls), and `SProp` as a universe instance (explicit, or solved from an argument in `SProp`, which now solves at `Prop`). Each test accepts the well-formed part of its document, pins the refusing rule's own message, and leaves nothing inhabiting `False`; each rule's mutant (Coq 8.4's guard, the constructor universe gate, the large-elimination restriction, the `SProp` arm, strict positivity) is killed in a private copy.
- **`kernel_oracle_differential`: LOGOS's two kernels on every Arena case both read.** The `Term` kernel, reached through the Lean importer, and the `dag` core never accept an invalid proof and never disagree on a verdict the Arena decides; the six cases the importer cannot yet decide (no string-literal reading for Lean 4.34.1, the shared-term materialization budget, two deadlines) are pinned with their reasons in a list that only shrinks, and a case pinned for a reason other than its deadline runs to the slow deadline, so a loaded suite cannot stop it short of the reason it asserts. An importer that accepts everything is caught on every invalid case.
- **`dag_sem_probe::a_probe_spends_its_budget_on_unfolding_too`**: an 8-step probe cannot unfold 64 definitions, so it is abandoned undecided rather than refuting `id⁶⁴ x ≢ y` after uncharged unfolding.
- `show_buffering_lock`: 100,000 shown lines take at most 12 `write(2)` calls in a debug and a release build and under `largo run --interpret`; the output before a failure is complete and precedes its reason, a prompt precedes the read of its answer, an `Escape to Rust:` line keeps its place, and a terminal is still written a line at a time.
- Add scalar-swap execution and adapter locks for native execution, mixed scalar kinds, word widths, self-swaps, overflow recovery, collection copy-on-write, liveness and refusal of pinned values.
- `wasm_float_display_lock`: both WebAssembly hosts carry `web_host::FMT_F64_JS`, format 120,024 doubles (named edges and exact ties, seeded bit patterns, seeded everyday values) as `fmt_f64` does under node, and show a compiled module's Reals as the VM does.
- `e2e_concurrency` adds eight probes for `Simultaneously:` on the Rust tiers: four and five bound tasks, a binding beside a call, literal bindings and literal bounds keeping the machine word, bindings meeting a Real, a Text binding's value semantics, and a reassigned binding. `phase9_structured_concurrency` adds `test_parallel_literal_tasks_state_their_type`, and `test_parallel_three_tasks_uses_threads` reads the join tree (approved port).
- New `exact_int_every_tier_lock`: an `Int` that outgrows a machine word prints the tree-walker's bytes, or fails with its sentence, on the VM, AST C, bytecode C, the WebAssembly test host and `largo run --emit wasm` under Node — products, doubling sums, Fibonacci, a recursion's result, parameters and `mutable` parameters, comparisons, division and remainders, powers, interpolation, an exact index that fits and one that fails alike, program bindings, copies, closures, Reals, a list-returning function, swaps (an exact value and a word, around a loop, inside a function, and a word handed to a variable that outgrows it later) — and a long exact loop leaves the host holding none of its integers. Every tier is held to exact bytes: a process's standard output and standard error as written, an in-process engine's shown lines and sentence each ended by a newline (`WasmRun::lines` keeps the test host's lines apart), nothing trimmed. `wasm_host_abi_lock` declares the `exact_*` imports in `FROZEN` with the decision; `runtime_failure_location_lock` gains rows for an index past a word on a list and a Text and a store at one, and states each code that is not a row in `NOT_ROWS` with its reason (`IntTooWide` is the module's own refusal).
- `int_representation_lock` adds five probes for an `Int` whose width only integer literals decide: a MINSTD seed read as a Real, a wide literal on its own, a range counter and a counted-while counter squared into a Real, and a counted loop ending at the last `i32`. Approved ports: the generated-Rust assertions of `phase23_stmt`, `phase24_codegen`, `phase24_wired_types`, `phase25_type_expr`, `phase26_e2e`, `phase30_iteration`, `phase32_functions`, `phase85_zones`, `phase_escape_hatch`, `phase_mountain_climb`, `phase_optimize`, `phase_optimize_v2` and the `ffi_requires_basic` snapshot read the typed binding (`let x: i64 = 5`) and range (`(1..=10_i64)`), each checking what it checked.
- `statement_snapshot_lock` adds interpreter/VM/optimized-VM regressions for statement operand snapshots, nested key evaluation order, mutable calls in control flow, and direct VM statement inputs.
- `in_out_storage_lock` pins mutable-place results across the interpreter, VM and optimized VM for globals, nested fields, collection elements, saved aliases and exact integers. `vm_in_out_wire_lock` checks byte-identical ISA re-encoding and ownership behavior after decoding. `jit_in_out_storage_lock` adds 14 exact-output controls for hot native scalar/list outputs, forwarded and recursive calls, list replacement and aliases, and recovery from native integer overflow; every row requires completed native execution.
- Add interpreter/VM snapshot regressions for mutable calls inside arithmetic, indexing, membership, slicing, set operations, maps, ranges, and closure arguments, with a pure-expression control.
- `spatial_exact_in_place_lock`: exact GEMMs on Wormhole and Blackhole from 1536³ to 2048³ read each operand once, split K and never reload a partial; an in-place finish computes the exact product under 68 schedules, on a ragged grid too; the output is reserved before a partial lands in its memory, and taking the reservation page by page clobbers under some schedule; external data keeps its order; rounding numerics and the fabric never alias; the intermediate is an index of the output's circular buffer. `spatial_buffers_lock`, `spatial_tensix_lowering_lock`, `spatial_chip_facts_lock` and `spatial_tensix_ttsim_lock` follow aliased buffers and the in-place exact plan, and `spatial_cost_model_lock` checks every compiler row of the ledger under the `l1acc` knob it was recorded at.
- `spatial_work_model_lock`: on Wormhole, Blackhole, the RTX 3060, both RDUs and the fabric, under exact and rounding numerics, every count the planner's closed form gives a candidate equals the count of the choreography `build_candidate` emits for it, to the byte, over candidates of every arrangement (fused or not, streamed, ragged, engine packing, packer accumulate, sub-block); the Wormhole closed-form tolerance is zero.
- `spatial_fabric_plan_lock`: the fabric's matrix engine is the systolic array's sub-block API, and on the fabric, whose time is exact, the planner's GEMM takes exactly the cycles of the fastest candidate the cycle twin times, under exact and rounding numerics at seven shapes. `spatial_work_model_lock` pins each matrix engine's calls and operand loads at the numbers its source states (Tensix's unpacker, the fabric's array, the GPU's warp grid, a tile pair at a time), and its closed-form tests compare the loads too.
- `spatial_roofs_lock`: every preset's roofs cite one of its sources (a derived roof its inputs and arithmetic, a modeled one the gap it fills); the Tenstorrent DRAM roofs equal the SoC descriptors' channels × 32 bits × the cards' data rate, and one matrix pass per cycle reproduces the n150's 74 and the p150's 664 TFLOPS; the RTX 3060 tensor roofs come from the GA10x SM, checked against the whitepaper's RTX 3080 column; Blackhole never inherits Wormhole's roofs; the RDU presets' roofs are the RDU facts'; each Plasticine mesh's roofs are its own bandwidth, clock and fp32 peak; our fabric, whose array each program sizes, carries no matrix roof, and its address lane is counted in elements, one per cycle.
- `roofline_invariant_lock`: nothing predicted or measured beats a roof. Every metered `tenstorrent_gemm.json` row moves at least the `(MK + KN + MN)` elements a GEMM must read and write (the floor is reached: the 128³ blocked plan sits two semaphores above it), every GEMM the pipeline plans on a preset with a matrix engine moves at least as many DRAM bytes, and every `plasticine_report.json` row's DRAM term is its bytes over its target's DRAM roof, its cycles at least both bounds, its share of the fp32 peak at most 100%.
- `spatial_roofline_lock`: a program's roofline is its work over its target's roofs. A GEMM's roofline counts `2 · M · N · K` operations on every preset whatever the fidelity; the Tensix bounds are one MVMUL per cycle, 288 bytes per cycle and one 32-byte flit per cycle; the RTX 3060's chip-wide tensor rate bounds the whole program's arithmetic; a per-second rate goes through the clock; a resource with no roof has no bound; and the fabric's exact cycle twin never beats its roofline. The roofline objective leads with the binding bound and breaks ties as `Balanced` does, admits every plan, never chooses a slower bound or a plan worse on all four work counts, and on the fabric, whose time is exact, never a slower plan. An element lane is paced by the elements it carries, not their bytes.
- **`machine_sympathy_corpus`: the kernel shapes an optimizing compiler is judged on, run bit for bit on every CPU tier.** Fifteen programs in `tests/fixtures/machine_sympathy/`, one per runnable claim of the CPU performance-engineering microbenchmarks: a data-dependent branch over unsorted and sorted data, one dependency chain against eight lanes in Int and in Real, a Sattolo pointer chase, a result nobody observes, saxpy, a seven-point stencil, a Horner map and the triad (rising arithmetic intensity), one field of an array of records against its own sequence, a call whose two sequences are one sequence, independent parallel counters, a first-touch fill, a line per `Show`, and a dense Real product. Each runs at sizes 1, 3, 9, 17, 30 and 64 and agrees byte for byte with the VM on the tree-walker, the optimized VM, the JIT, `largo run --interpret`, both Rust profiles, both C lowerings, WebAssembly and RISC-V; the two written orders of the harmonic Real sum differ at 9, 30 and 64, so a tier that reorders either prints a different number. `tests/machine_sympathy_census.tsv` records the optimizations that fire on each kernel on the AOT, run and VM paths, and one that stops firing is RED until the change names what replaces it (`UPDATE_SNAPSHOTS=1` records the census). The kernels join the parity census in `tests/tier_parity/corpus.rs`, and the Rust batch build compiles each distinct source once, so the benchmarks' three sizes share one binary.
- **The kernel's conversion engines are held to linear work and to each other.** New suite `dag_conversion_scaling` counts conversion steps on a literal recursion stuck on differing variables (linear in the literal) and witnesses the refutation memo's mode and proof-generation keys; `dag_nbe_differential` and `dag_nbe_checker` gain the matching NbE locks.
- **Every test in one package.**
  - **Integration suites:** the crate-local `tests/` directories moved into `crates/logicaffeine_tests/tests/` under their own names, crate-prefixed where a name was taken; the package holds 2,424 suite files.
  - **Unit-test bodies:** each crate's inline `#[cfg(test)]` bodies live under `crates/logicaffeine_tests/unit/<crate dir>/<module path>.rs` (705 files), reached through a one-line `#[path]` stub in the crate, so they keep their names and private access and still run under `cargo nextest run -p <crate>`. `crates/logicaffeine_tests/examples/test_home.rs` performs and ledgers such a move.
  - **Enforcement:** `unit_home_lock` refuses a `#[test]` in any `src/`, a stub that does not resolve and an orphaned unit file; `test_home_lock` refuses a `tests/` directory in any other package.
  - **Features:** every suite keeps the build condition it had at home. The package forwards `codegen`, `serde`, `wasm-jit`, `http`, `persistence` and `concurrency` by default and `nbe`, `io-uring`, `capnproto`, `ffi-link-tests` and `verification` on request (each run by `scripts/run-opt-in-feature-tests.sh` or the baseline, which `feature_coverage_lock` holds), and `unexpected_cfgs = "deny"` makes an undeclared feature name a build error, so no suite can be compiled out by a feature the package does not know.
- **The CLI suites spawn the shipped binaries.** `largo-under-test` and `logos-sat-under-test` are each one call to the product crate's library entry (`logicaffeine_cli::largo_main`, `logicaffeine_proof::satcli::main`), the call the shipped `main` makes, and every suite finds them where the run put them: `tests/support/under_test.rs` reads the run-time `CARGO_BIN_EXE_<name>` (nextest sets it to the extracted binary in an archive run) before the build-time path, and no other file bakes a build path in. `test_bin_shim_lock`.
- **A spawned server is dialed where it says it listens.** `tests/support/served.rs` starts every server a suite spawns — `largo serve`, `largo run`, a compiled binary — on port 0 and dials the address its announcement names; `largo serve` moves to the next port when its own is taken, so a port reserved and released before a seconds-long build was a port another suite's server could take. A spawned server leads a process group of its own and is killed with it, so nothing it started outlives the test. `served_address_lock` holds every tier's announcement (the VM's through a re-executed test process, the compiled tier's on the engine it names), the group kill, and the census of the in-process suites that still reserve a port, each with its reason.
- **A missing tool is counted, never passed.** `common::require_tool` prints `LOGOS_TOOL_SKIP: <tool>` and fails outright when `LOGOS_REQUIRE_TOOLS` names the tool; capabilities that are not binaries use `LOGOS_CAP_SKIP` and `LOGOS_REQUIRE_CAPS`. The machine-local EDA tree under `tools/` (OpenLane with the hardened designs, the Caravel checkouts) is a named input, `eda-tools`: every test that reads it asks the seam first, so a clone records a counted skip where it used to fail or pass having checked nothing, and `eda_tools_seam_lock` holds the census of its readers. `cli_tool_preflight_lock`.
- **Mutation and coverage ratchets, run on demand.** `.config/mutants.toml` scopes cargo-mutants to the kernel, the SAT certifier, the wire codec and the crypto primitives, and `mutation_lock` requires the scope intact and a recorded verdict for every survivor in `.config/mutants/known-survivors.txt`. `.config/coverage/thresholds.toml` holds per-crate floors that only rise, checked by `scripts/test/check-coverage-floors.sh`.
- **The release surfaces are locked.**
  - **Packaging and release:** `packaging_lock`, `size_budget_lock`, `dist_profile_lock`, `release_assets_lock`, `installer_docs_lock`, `version_lock`, `bump_version_script_lock`, `repo_shape_lock`.
  - **Gate honesty:** `ondemand_exclusion_lock`, `feature_coverage_lock`, `wasm_gated_suite_lock`, `feature_forwarding_lock`.
  - **Benchmarks:** `benchmark_metadata_lock`, `benchmark_fairness_lock`, `solver_bench_provenance_lock`.
  - **Release notes:** `changelog_citation_lock` holds every `CHANGELOG.md` section from 0.11.0 on to what the public tree carries. A backticked `<name>_lock` (or `<name>_lock::<test>`) must be a public suite in `crates/logicaffeine_tests/tests/` holding that test; a backticked repo path must be public or untracked; no text may name a top-level directory the manifest withholds, except the run-output `logs/`. Its self-test feeds each rule the shapes it refuses and the ones it accepts. The shared manifest classifier gains `Classifier::from_rules` and `withheld_dirs`.
  - **Documentation:** `cli_docs_lock` (the documented `largo` surface is the one the binary parses), `docs_index_lock` (every guide, project document and example is reachable from the indexes), `tier_map_lock` (the crate tiers the docs state are the ones the manifests imply), `crates_page_lock` (the site's `/crates` page uses each manifest's own description), `security_scope_lock`, `readme_benchmark_numbers_lock` (every quoted benchmark number is one the charts were drawn with, from the results on disk) and `public_prose_lock` (public documentation cites only what the public tree holds). `changelog_release_gate_lock`: a published crate cannot ship changed code under an empty CHANGELOG.
  - **Repository shape:** `scripts_root_lock` resolves every path a script writes relative to its own location; `tests_index_lock` holds `crates/logicaffeine_tests/tests/INDEX.md`, a row per family of suites, to the suites; `repo_shape_lock` gains the scripts folders, the scripts index and unique script names.
  - **Private material:** `private_names_lock` scans every tracked path and file with the private-material guard installed in the checkout's git directory, whose lists no commit carries, so the lock's source names nothing it guards. A refusal names a file, a line and an entry's number, never the matched text, and a self-test plants every path-list entry and holds the guard to that. A checkout without the guard records a counted `LOGOS_TOOL_SKIP: private-guard`.
- `c_bytecode_lock::a_new_sequence_never_takes_the_buffer_of_another_kind`: one register holding a `Seq of Bool` (or a `Set of Int`) and later a `Seq of Int`, on the walker as on the VM.
- **`c_bytecode_lock` observes the bytecode→C walker against the VM on stdout, stderr and exit status** for value semantics through every alias, generic enums, text bytes and UTF-8 validation, empty sequences, tuple parameters, function values, the offline network and every relay address form, the router's functions, the sandboxed file system, the VM's failure sentences, linear text growth under a 256 MB address space, exact integers (and that a growing integer runs on machine words until it outgrows them, each word boundary crossed in a region of its own), natives, and all 1,000 fuzz seeds, where a refusal is now a failure.
  - `c_offload_seam_lock`: the offload seam.
  - `support/fixed_clock.rs`: the one instant every harness gives `today`/`now` (2024-10-04 12:34:56 UTC); `c_bytecode_lock` and `c_temporal_lock` read it. The execution ratchet runs every binary under it: the harvested rows `today_direct`, `today_let`, `now_direct` and `now_let` were WRONG under the wall clock (4 of 540) and are correct under the harness instant, and the floor rises to 540. Test-data correction: `now_direct`/`now_let` recorded "2024-10-04 12:04", which no clock produced; they now record "2024-10-04 12:34", what the VM prints at the harness instant (`gauntlet/corpus.jsonl`, `op_backend_matrix_lock.rs`).
  - `c_bytecode_perf_lock`: the call-convention probe is Takeuchi's `tak(28, 18, 8)` (bytecode 77 ms, AST 84 ms, 0.92x), since the memoizer collapses `fib(40)` (0.37 ms vs 102 ms); `every_perf_probe_keeps_the_work_it_measures` holds every probe to code the memoizer leaves alone.
  - `codegen_c::runtime::bignum_lib_tests` compile the runtime the walker ships (`c_runtime()`, the VM's sentences filled in) instead of its template.
- **The whole-program engine's UART protocol has one definition.** `chip_protocol_lock` holds `codegen_sva::chip_protocol` to one meaning per byte and to a lossless framing that reports a cut-off record as short; holds the engine's Verilog templates to naming every protocol byte by placeholder, so the emitted engine and its host testbench carry each byte where the table puts it; and holds `scripts/logos-chip.py` to reading the table from the `chip_host` example rather than restating it, decoding every value shape through `logos-chip.py decode` as the VM prints it. `logos_chip_lock::the_host_decoders_pass_their_selftest` also runs `scripts/nexys-host.py --selftest`, whose golden vectors are the soft core's reply bytes.
- **`crdt_core_differential_lock` runs `codegen_c::CRDT_CORE` and the data crate's CRDTs on the same operation scripts** (seven pinned scripts, one per bias's defining case, and 600 seeded scripts of 120 operations: adds, removes, writes, copies, merges between replicas in every direction) and requires every observation to agree. The core runs under ASan, UBSan and LeakSanitizer, so a leaked byte fails it.
- **`c_ast_vm_parity_lock` observes `compile_to_c` against the VM on stdout, stderr and exit status**, the VM's run pinned per row and the C built with the release C flags (`-O3 -march=native -flto -ffp-contract=off`, `common::build_c_release`): control flow out of loops, zones and tasks, value semantics through bindings, callees, containers, loops and closures, text characters and building, exact integers through tuples, records, closures and bounded folds, maps and sets through removal and exact keys, `Swap` of every kind, `Inspect` failures, presizing, and the VM's failure sentences. Scaling rows run under an address-space cap (text building, lists handed through calls, repeated reservations counted by an `LD_PRELOAD` allocation counter); `no_program_lowers_to_a_placeholder_or_a_refusal` compiles the whole gauntlet corpus and every benchmark and forbids `/* unsupported` and `#error` in the C.
  - `common::{C_RELEASE_FLAGS, c_compiler, build_c_release}`: the suites that build AST C (`assert_c_output`, `construct_tier_totality_lock`, the gauntlet's C leg) build it as a release C build does.
  - The gauntlet's C ledgers shrink: no compile gaps; one divergence left (`corpus_int_overflow_at_run_time`).
  - `stepped_tier_lock` holds a `Break` out of a loop whose condition waits, a `Return` in a zone of a function that waits, a Text parameter grown across a wait, and a Text condition that waits; the C refusal ledger of `optimizer_name_capture_lock` loses the closure row the C tier now runs as the interpreter does.
  - `c_promotion_parity_lock`: a masked hash behind a tuple return and a remainder LCG keyed into a map stay `int64_t` on C and `i64` on Rust, and print the same bytes on both.
- **Every program is held to one observable result on every backend.** `statement_tier_totality_lock` runs the semantic probes in `crates/logicaffeine_tests/tests/tier_parity/semantic.rs` (indexing, loops, narrowing, slices, specialization, maps, swaps, decimals, arithmetic failures, short-circuit `and`/`or`, tiled nests) on the tree-walker, the VM, `largo run --interpret` and generated Rust under both shipped profiles, comparing standard output, standard error and how the run ends byte for byte, and its census proves every statement and expression kind has a probe that parses. A test ends a child's process group only through `crates/logicaffeine_tests/tests/support/process_group.rs` (`libc::kill(-pgid)`, behind `pgid > 1`).
- **The RISC-V tier is held to the VM's output and to an instruction budget.** `riscv_aot_lock` runs a parity corpus against the VM on stdout, stderr and exit status: failures in the VM's own words, `nothing`, enums by constructor name, copies, merges, the offline inbox, files, empty literals and `rotl`. Its rows run on four threads and report in row order. It also runs an ownership corpus of lists and counters in records' fields against the tree-walker, and a long loop of each mutating op within a 512 MB address space. `riscv_instruction_budget_lock` (new) counts, under `qemu-riscv64`, the instructions the emitted code of 64 programs executes and holds each to a shrink-only budget; it also requires that the runtime carry no `.eh_frame`. `crates/logicaffeine_tests/tests/support/riscv_runner.rs` (new) links and runs every RISC-V suite's binary inside a 512 MB address space and a 60 s limit. The emitter's unit tests state the kind of every mutating op. `phase_vm_error_parity` runs the VM's file statements on the networking driver.
- **Every benchmark on every backend.** `benchmark_logos_tiers_lock` runs all 35 registry programs at their `small_size` through Rust, C from the program and from its bytecode, the JIT, the VM alone, the tree-walker, WebAssembly (`largo run --emit wasm`) and RISC-V (under `qemu-riscv64`), each held to `expected_<n>.txt` byte for byte; `benchmark_registry_lock` and `benchmark_harness_fixture_lock` hold the registry (`benchmarks/lib/*.tsv`) and `benchmarks/run.sh` on stub toolchains (every row built, verified at every size, a timed-out size keeping the smaller ones, an engine row that ran on another engine stopping the run); `benchmark_collapse_lock`, `benchmark_profile_assemble_lock`, `flame_tree_trim_lock`, `largo_build_emit_c`.
- **The shared optimizer's proofs and their consumers.** `iteration_lock` (an Int range is a counted loop, a sequence nothing changes is walked in place; narrowed, returned, aliased and constant-table sequences on every tier), `purity_lock`, `inline_tiny_lock`, `vm_listing_lock`, `seq_iteration_borrow_lock`, `with_capacity_transparency_lock`, `walker_soundness_lock`, `swap_every_tier_lock`.
- **The C tiers and WebAssembly.** `in_out_lock` (a `mutable` parameter of every kind, lent from a binding, an element or a field, handed on, outgrowing a word, on the VM, both C tiers and WebAssembly), `c_operand_order_lock`; `in_out_unrolling_lock` adds both C tiers; `c_bytecode_lock`'s fuzz generator adds a loop that writes the list it walks and a range whose end a call computes, so the iterator ops stay covered, and its op floor rises to 60.
- **Models brought up as data.** `model_tokenizer_lock` (every pre-tokenizer's split and every bundled vocabulary against llama.cpp's own code, compiled from the pinned source as an oracle; every local checkpoint's vocabulary; the libstdc++ sort, hash and map order the special tokens depend on), `model_golden_lock` (each local model's 32-prompt corpus against llama.cpp's CPU runs of its shipped and its `f32` weights, the runs recorded with no GPU visible and no operation offloaded), `model_gguf_lock`. `common/tiny_models.rs` writes small checkpoints of every architecture with weights exact in Q8_0 and `f32`.
- **The compiled generation lane.** `lane_compile_lock` (the step graph's parameters and carried state, every op lowered once, each projection read in place in its binding's row order, the fusion census of dense, expert and hybrid steps, each knob's effect, refusals by name, `nvcc` builds of every architecture under every numerics, and a program's refusal of a checkpoint or sidecar it was not compiled with) and `cuda_lane_lock` (each architecture's generation held to the `f64` reference at the lane's own activation precision, a prompt run in chunks continuing as one fed a token at a time, every knob point generating the same tokens bit for bit, Qwen3-8B continuing llama.cpp's golden prompt, and every point of the `activations` knob meeting its bar against llama.cpp's runs of the Qwen3.5 and Llama 3.2 corpora). `tensor_norm_epsilon_lock` holds the `with epsilon E` clause from the parser to every tier and accelerator emitter; `cuda_knob_consumers_lock` holds every knob the lane reads to a change in its program. `tensor_attention_window_lock` holds `within a window of W` to the `f64` formula on the VM, the tree-walker, C and AOT-Rust (a single query, every head of a group, a causal block's rows, a window as wide as the history), its lifting into `AttentionSpec::window`, its refusals, and the refusal by name of the paths with no windowed kernel; `tensor_scale_lock` holds `X scaled by C` the same way, and the probe corpus gains `scale` for the backend matrix and the spatial ledger. `tensor_attention_sinks_lock`, `tensor_row_broadcast_lock` and `tensor_moe_options_lock` hold sinks, a row over a block and the mixture's biases and clamped gate to the `f64` formula on the VM, the tree-walker, C and AOT-Rust, their lifting into the specs, their refusals, and the accelerators' refusals by name; `lane_compile_lock::the_biased_step_folds_its_biases_and_routes_by_an_f32_router` pins a tiny gpt-oss step. `lane_compile_lock::every_block_format_is_read_in_place_through_its_decoder` and `cuda_lane_lock::every_weight_format_generates_what_the_reference_computes` hold every GGUF quantized format to its block kernels and to the reference decoding the same blocks, under every numerics, in dense steps, expert mixtures and chunked prefills; `cuda_lane_lock::the_activation_quantizer_is_the_references_bit_for_bit` holds `lk_quant` to `reference::quantize_activations` bit for bit. `common/tiny_models.rs` writes a checkpoint in any format (`write_checkpoint_as`, seeded blocks for the K-quants, the legacy formats and MXFP4) at a wide size (`Tiny::wide`) whose rows are whole super-blocks, and adds a gpt-oss (alternating windows, sinks, biases, a YaRN rotation, an `f32` router, the clamped SwiGLU) and a Gemma 3 (windowed local layers with their own rotation, sandwich norms, a GeLU gate, the embedding scale, a tied head), and `lane_compile_lock::the_sandwiched_step_windows_its_local_layers_and_fuses_its_gates` pins its step's census, windows, rotary tables, scaled gather and GeLU gate.
- **The lane's streams, held to the memory its launches touch.** `lane_compile_lock::stream_corpus` (every tiny architecture at its own width and four times it in Q8_0, Qwen3 and DeepSeek-V2 at their medium widths in Q4_K, under every numerics) is walked by `every_branched_launch_follows_every_launch_it_depends_on` — every conflicting pair of launches ordered by stream order or events, every side stream joined, the closing arg-max writing the token and position, every launch staggerable — and `every_arena_array_a_launch_names_lies_in_its_regions`, which reads the emitted program (each pointer's first byte in the arena, through aliases, and each launch's block of statements) and holds every arena array a launch names to the regions the streams order it by. `cuda_lane_lock::a_branched_step_computes_what_its_launches_compute_in_order` runs every tiny architecture and the medium Qwen3 and DeepSeek-V2 in Q4_K, a 40-token prompt in chunks of 32 and 8 then decode steps, under every numerics, branched and under four seeds of `--stagger`, bit for bit against every launch in order.
- **The two-word attention, held to its operands.** `cuda_lane_lock::the_two_word_attention_computes_what_the_attention_computes` holds `lk_kv_two_words` + `lk_attention_i8` to `lk_attention` on thirteen shapes and the expanded latent split, and to the `f64` attention of its operands taken into two words by the lane's quantizer replicated bit for bit (`two_words_row`, `attention_f64`) within 1e-5; and runs the program under the pinned compute-sanitizer (`scripts/accel/fetch-cuda-sanitizer.sh`, `common::cuda_sanitizer`, `require_tool("cuda-sanitizer")`) — no shared-memory race, misplaced barrier or stray access, as a warning or an error. `a_chunks_conv_and_f32_gemv_…` and `an_f32_projection_in_tiles_…` hold the `f32` GEMV cut over warps to its rows' groups and to the `f64` sum rounded.
- **The fusion ledger credits the compiled generation lane.** `benchmarks/fusion_parity.json` marks 28 of its 36 classes `compiled`, 11 before: the QK-norm, rotation and cache-row writes, the gated products and their quantization, merged and paired projections, projection biases, the folded shared expert, experts grouped over a chunk's rows, split-K decode attention, tiled prefill attention, latent attention, the delta rule, the convolution with its SiLU and the arg-max on the device — each citing its census in `lane_compile_lock` and its values test in `cuda_lane_lock`; windows and sinks inside the attention kernel are `partial`. `fusion_parity_lock` accepts a census through the lane's compiler (`census_compiles`: a `lane_compile_lock` test, every one of which compiles through `codegen_cuda::lane`) beside the single-token forward, raises its floor to 28, and `a_census_must_compile_through_a_compiler` refuses a GPU values test or a hand-written lane's test as a census. `lane_compile_lock::the_decode_step_attends_a_long_cache_in_chunks_one_launch_a_layer` is the split-K decode census.
- **Where a lane's matrices live.** `lane_compile_lock` walks every architecture's plan under every numerics, and an expert model with its head tied, down a budget a byte smaller at a time: one more matrix moves each time, in the order a step misses them least (a tied table after the expert stacks, read whole by the head), the two sides summing to the whole, to the refusal a byte under the arrays that cannot move; it pins the program's placement constants, its order and each upload's rank to the plan, a matrix's bytes to its pooled arrays, and a tied head to its table's upload. `cuda_lane_lock` builds every tiny architecture and the wide models in every block format once under every numerics and runs each on the memory it finds free and on `--device-bytes` of its whole, half its matrices and none — the same tokens and logits bit for bit, each placement `residence::place`'s, and the refusal by name before it allocates — and runs Llama-3.2-1B while another process holds all of the device but half its matrices, which it moves to host memory on its own and generates the same. gpt-oss-20b joins the real checkpoints held to every activation precision's bar, on the RTX 3060.
- **Every format the reader dequantizes, the lane reads.** `lane_compile_lock::every_format_the_reader_dequantizes_the_lane_reads` holds every block format `model::quant` reads to a lane decoder (`kernels::decodes`) and to `tiny_models::BLOCK_FORMATS`, the one list every lane lock runs — the GPU locks' six lists of their own folded into it — so a format the reader gains turns the census red until the lane decodes it and every lock runs it; IQ4_NL and IQ4_XS join them.
- **The staged per-head matmul fits a block's default shared memory.** `lane_compile_lock::every_staged_per_head_launch_fits_the_default_shared_memory` holds every staged launch of DeepSeek-V2's wide program, in each format the staged decode reads, to its rows plus `kernels::AXPY_SUMS` within 48 KB, and `cuda_lane_lock::the_staged_per_head_matmul_takes_the_shared_memory_its_tile_leaves` holds `AXPY_SUMS` to the static shared memory the driver reports for the kernel in each of those formats. `lane_compile_lock::every_block_formats_staged_unit_is_its_checkpoint_block` holds the kernels' `lk_unit_bytes`, `lk_unit_subs` and `lk_axpy_pitch`, evaluated on the host from their source, to each format's checkpoint block and to `kernels::axpy_pitch`.
- **Every row-reading kernel decodes in registers.** `lane_compile_lock::every_row_reading_kernel_keeps_its_decode_in_registers` compiles the decode step of an expert model with every matrix in each format under both quantized numerics and holds every projection, gated-pair and expert kernel to no stack frame and no spill, as `ptxas` reports it.
- **A real IQ4_XS checkpoint.** Llama 3.2 1B as bartowski's IQ4_XS quantization (its projections IQ4_XS, its values Q5_K, its tied table Q6_K) joins the real checkpoints: llama.cpp's goldens of it and of its f32 copy (`examples/gguf_f32`) in `benchmarks/models/llama3.2-1b-iq4xs`, `model_golden_lock::llama3_2_1b_iq4xs_matches_its_llama_cpp_goldens` (the reference's top logits within 1e-5 of llama.cpp's f32 run), and `cuda_lane_lock::llama3_2_1b_iq4xs_meets_every_activation_precisions_bar`: the precise and two-word lanes follow llama.cpp's f32 run 499 of 512 steps, its shipped run 411.
- **DeepSeek's latent attention, held piece by piece and whole.** `tensor_latent_attention_lock` holds the head slices, joins and per-head matmul, an attention reading values narrower than its keys at a given scale, and the absorbed latent-attention step to their `f64` formulas (the absorbed step to the expanded attention it computes) on the VM, the tree-walker, C and AOT-Rust, their lifting into `HeadSliceSpec`, `JoinSpec`, `HeadMatmulSpec` and `AttentionSpec::{vd, scale}`, their refusals by name, and the spatial IR's lowering of the narrower values and the absorbed step (its `f32` twin within 2⁻¹⁶ of the VM); `tensor_moe_weighting_lock` holds `without renormalizing` the same way. The probe corpus gains `head_slice`, `join` and `head_matmul` for the backend matrix and both spatial ledgers (`PROGRAM_PATH_FLOOR` 31, the kernel registry's frozen total 887). `common/tiny_models.rs` adds two DeepSeek-V2s (the Lite layout with one `attn_kv_b`, and a low-rank query with `attn_k_b`/`attn_v_b`), each with a dense first layer, a shared expert and un-renormalized routing, so every lane lock over `ALL` covers latent attention; `lane_compile_lock::the_latent_step_absorbs_each_heads_query_and_reads_its_values_in_the_cache` pins their step's census, its per-head products' layouts and its strided values, and `cuda_lane_lock`'s precision bar adds DeepSeek-V2-Lite.
- **The lane's in-place folds and its split attention, held where their dataflow allows.** `lane_compile_lock` pins the latent step's cache row and queries landing in place (`slice_read`, `row_write`, `join_in_place`, the absorption reading its part of the query projection when a head is whole blocks) at both widths; `the_in_place_folds_hold_back_where_a_launch_between_would_see_the_difference` plans hand-written steps where a launch between a fold's producer and its consumer would see the difference; `a_row_of_a_table_written_before_its_last_reader_is_copied_where_it_is_taken` holds a literal row of a carried table to a copy when the table is written before the row's last reader. `cuda_lane_lock::a_long_prompt_attends_in_chunks_alike_in_every_form` gives grouped heads, sinks beside windowed layers and latent attention prompts long enough that every chunk of the split attention outgrows a tile, and holds the run token by token, in chunks of 16 and in chunks of 64 to one another bit for bit and to the reference. A cache of 30,000 positions now compiles, and the refusal moves to a head past one block's shared memory or a key head the vector loads cannot read. `cuda_lane_lock::every_tiny_architecture_generates_what_the_reference_computes` runs eight prompts an architecture under every numerics and holds each run to the reference with the near calls its `f32` arithmetic decided the other way taken its way (`Reference::near_calls`, `Reference::flip`), nearest the parting first; a parting no near call explains fails. `cuda_lane_lock::a_program_run_over_many_prompts_gives_each_the_run_it_gives_alone` and `the_program_profiles_its_decode_step_launch_by_launch` hold the program's `--prompts` and `--profile` runs; `every_block_decoder_agrees_with_the_sub_block_decoder` holds the per-element and word-wise block decoders to the sub-block decoder bit for bit in every format.
- `lane_compile_lock`'s census locks count every dense FFN's gated pair as one `gemv_glu` launch (`glu_pair`) in the dense, sandwiched, hybrid and latent steps and in prefill chunks of up to sixteen rows, and a wide chunk's 64- and 32-row forms keep every projection on the tensor cores.
- `lane_compile_lock::the_shared_expert_folds_into_its_mixture_where_nothing_else_reads_its_parts` plans a mixture with a shared expert by hand (`planned_rows`, its weights as Q8_0 checkpoint rows). The fold fires for the sum in either order. It is held back when the mixture, the down projection or the gated pair is read elsewhere, when the shared expert takes another activation, under the clamped SwiGLU, and when 32 experts are kept. The latent step's census counts each mixture's shared expert inside its launches. `every_fusion_rule_can_be_held_back` compiles every architecture with every rule held back and checks that no rule fires and every op is lowered once. `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart` runs each architecture with every rule held back and every activation quantized in its own launch, and holds it to the fused program bit for bit under the int8 and the precise numerics.
- `lane_compile_lock::the_wide_shapes_take_four_rows_a_warp` checks DeepSeek-V2 at its medium width (`Tiny::medium`) under every numerics. Each wide projection, gated pair and mixture takes four rows a warp, the latent query projection quantizes in its epilogue, and prefill chunks take four rows a warp only in their per-row mixtures. A Q8_0 checkpoint and the tiny widths keep one row a warp, and an `f32` router takes four warps an expert. `cuda_lane_lock::the_medium_widths_generate_what_the_reference_computes` holds Qwen3, Qwen3-MoE and DeepSeek-V2 at their medium width in Q4_K, with an `f32` router, to the reference under every numerics. `every_fusion_computes_what_its_launches_compute_apart` adds the medium Qwen3 and DeepSeek-V2 and a Qwen3 with 256-wide heads, and checks each bit for bit against its program with every rule (`rows_a_warp` included) held back.
- `lane_compile_lock::the_absorption_stages_its_rows_where_staging_wins` checks the latent absorption of every form under each numerics in Q4_K, Q5_0, Q6_K and Q8_0. It stages in every form alike where the policy says to, and keeps the per-lane kernel elsewhere.
- `lane_compile_lock::a_prefill_chunk_takes_its_rows_in_one_launch_per_op` counts each layer's mixture over the chunk's rows as one launch (`moe_rows`). `the_expert_step_routes_each_token_in_one_launch_set` holds the decode step's mixtures to one row. `every_fusion_computes_what_its_launches_compute_apart` and `a_prompt_run_in_chunks_continues_as_one_fed_a_token_at_a_time` hold the folded mixture to the per-row ones bit for bit.
- `cuda_lane_lock::the_precise_norms_sum_in_f64` runs `lk_rmsnorm` under the precise numerics on rows 64 to 8192 wide, whose values span six orders of magnitude, and holds it within half an `f32` step of the `f64` norm. The precise lane's bound against the reference is now 2e-5 of the logits' scale. Its `f32` activations reach 1.1e-5 at the most sensitive position of the tiny corpora; typically they sit at 1–4e-6.
- `lane_compile_lock::a_wide_chunk_in_a_block_format_runs_on_the_tensor_cores` counts every projection of a wide Qwen3's 64- and 32-row chunks on the block GEMM in Q4_K and Q6_K under both quantized numerics, its gated pairs held back to two GEMMs, the 16-row chunk on the block GEMV, and none under the precise numerics. `a_wide_chunks_experts_run_on_the_tensor_cores` counts each wide chunk's mixtures' grouped gate, up and down GEMMs and gated activations for Qwen3-MoE in Q4_K and Q8_0, DeepSeek-V2 in Q6_K and gpt-oss in MXFP4 with its clamped gate, the slabs of every chunk of at most sixteen rows and of every chunk under the precise numerics, and builds each program. `cuda_lane_lock::a_wide_chunks_experts_on_the_tensor_cores_generate_what_the_reference_computes` runs the wide expert models with their matrices in Q8_0, Q4_K, Q6_K and MXFP4 by turns over a 230-token prompt in chunks of 128, 64 and 32 — an expert's rows past one tile of 64 — under the two-word numerics, and holds each run to the reference; under one word a prompt that long makes more near roundings than the reference's near calls can account for, even fed a token at a time. `the_grouped_expert_gemms_compute_what_the_slabs_compute` holds the grouped gate, up and down GEMMs to the slab kernels on the same inputs (the down rows reading the same quantized hidden rows) in every weight format under both quantized numerics, with SwiGLU and, in MXFP4 and Q8_0, gpt-oss's clamped gate and biases, over 160 rows choosing two of four experts 150, 90, 50 and 30 times: each result within 1e-5 of its row's largest, every output written. `every_weight_format_generates_what_the_reference_computes` holds its 32-row chunks to the token-at-a-time run bit for bit under the precise numerics, and to the reference under the two-word numerics; under one word a 40-token prompt makes more near roundings than the reference's near calls account for, fed a token at a time or in chunks, so the one-word chunk's kernels are held by the kernel locks.
- `cuda_lane_lock`'s `held_to_reference` now keeps, each round, the near call that carries the parting furthest — to a later position, or nearer the reference at the same one — so the near calls one position needs together are found one after another (a gpt-oss prompt whose first generated position needed two roundings at margins of 0 and 7.6e-6 levels decided the other way).
- `lane_compile_lock::every_step_local_array_lives_in_one_arena_by_its_lifetime` checks every architecture's every form under every numerics, and DeepSeek-V2 at its medium width in a 64-row chunk. Each placed array is a computed tensor's own and is first touched by a write. It is all written before any launch reads it, whole or as parts covering every unit, never as rows or through a view, and lives to its last touch through its views. No two arrays live together share a byte. Every launch's scratch but its arrival counts is placed, sized as the launch describes it, clear of every array live at the launch and of its other scratch. The program allocates one poisoned arena of the largest form's size. The medium chunk places at least 95% of its step-local bytes, in under a third of their sum. `every_knob_changes_the_program` now finds the two-word numerics' low words as pointers into the arena, one for each placed quantized activation, and none under one word.
- `lane_compile_lock::a_wide_chunk_attends_in_tiles` counts the tiled attention in every attention of a wide Qwen3's, gpt-oss's and DeepSeek-V2's 64- and 32-row chunks under both quantized numerics, the per-row attention in every narrower form, and none under the precise numerics; `a_wide_chunks_per_head_products_run_in_tiles` does the same for DeepSeek-V2's per-head products, with one expansion and with its halves split. `cuda_lane_lock::the_tiled_attention_computes_what_the_attention_computes` holds `lk_attention_tiled` to `lk_attention` on the same queries, caches and positions — grouped heads, a window with sinks, the latent attention reading its values in place, a head a group; 40 rows after 70 cached positions — each output within 1e-5 of its head's largest. `the_tiled_per_head_products_compute_what_the_slabs_compute` holds `lk_head_tiled` to the slabs of `lk_head_gemv` and `lk_head_axpy` at the latent attention's shapes in six formats under both quantized numerics, within 1e-5.
- `lane_compile_lock::a_wide_chunks_experts_run_on_the_tensor_cores` also counts each wide chunk's mixtures picking from their logits, each pick right after the one GEMM of its logits (`lk_gemm_f32` for gpt-oss, the block GEMM otherwise), and the narrower forms routing a block a row. `cuda_lane_lock::the_router_as_a_gemm_routes_as_the_router_does` holds the logits GEMM and `lk_moe_pick` to `lk_moe_route` over 160 rows choosing 6 of 64 experts through an `f32` router with a bias and through Q4_K and Q8_0 routers, renormalized and not, under both quantized numerics: the logits within 1e-5 of each row's largest, the pick over the router's own logits its choices and weights bit for bit, and the same choices wherever no near tie separates a row's sixth and seventh. `the_grouped_expert_gemms_compute_what_the_slabs_compute` runs every case at 64 and at 128 rows a block, the expert grouping in parallel.
- `model_reference_lock::a_latent_attentions_expansion_reads_its_activations_unquantized`: a tiny DeepSeek-V2 in both conversions, every matrix `f32` but the latent's expansion, continues its prompt under the one- and two-word precisions with the exact reference's logits bit for bit; with the latent's projection quantized too, both part from it. `lane_compile_lock::a_wide_chunks_latent_attention_attends_expanded` holds every latent layer of DeepSeek-V2's wide 64- and 32-row chunks, in both conversions under both quantized numerics, to the expanded attention — the two expansion halves over the cache's rows up to the chunk's last position, the attention's split query and key, the rotated query part's buffer of each row's heads alone — and the 16-row chunk, the decode step and the precise numerics to the absorbed one. `cuda_lane_lock::the_expanded_latent_attention_computes_what_the_absorbed_one_computes` holds the two forms to each other at DeepSeek-V2-Lite's latent widths, 40 rows after 70 cached positions, the halves in Q8_0, Q4_K, Q6_K and Q5_0 (a split key half in the 32-value formats), within 1e-4 of each head's largest; `the_tiled_per_head_products_compute_what_the_slabs_compute` runs under `Q` 3 too. `the_latent_step_absorbs_each_heads_query_and_reads_its_values_in_the_cache` counts three quantizations (the halves read `f32`), and `the_wide_shapes_take_four_rows_a_warp` holds the epilogue quantization on a hand-written projection of a projection (`planned_rows_as`) and DeepSeek's query projection `f32` alone.
- `cuda_lane_lock::the_paired_gate_and_up_gemm_quantizes_the_hidden_rows_as_the_unpaired_launches_do` holds `lk_gemm_blk`'s paired mode to the gate GEMM, the up GEMM, `lk_moe_glu` and `lk_quant` byte for byte — words, low words and scales — in every block format under both quantized numerics, with SwiGLU, and MXFP4 and Q8_0 also with gpt-oss's clamped gate and the experts' biases, at 64 and 128 rows a block. `lane_compile_lock::a_wide_chunks_experts_run_on_the_tensor_cores` counts one paired GEMM for each wide chunk's mixture and no gated activation apart, and a mixture whose gate and up are in two formats keeping two GEMMs and `lk_moe_glu`.
- `cuda_lane_lock::a_dense_gemms_last_wave_split_computes_what_the_whole_tiles_compute` holds the dense block GEMM whose last wave cuts its tiles along `k` to the same GEMM a whole tile a block, in Q4_K, Q6_K, Q5_0 and MXFP4 under both quantized numerics, at DeepSeek-V2-Lite's dense shapes (tiles past a whole wave, in whole waves, fewer than a wave), with and without a bias and with splits forced to 2 and 3: within 1e-5 of each row's largest, and bit for bit where nothing splits. `the_router_as_a_gemm_routes_as_the_router_does` runs the `f32` router's GEMM cut as the lane cuts it on the chip.
- `cuda_lane_lock::the_shared_row_attention_computes_what_the_attention_computes` holds `lk_attention_shared` and `lk_attention_fold` to `lk_attention` at its planned heads a block: DeepSeek-V2-Lite's latent attention (16 heads over 576-wide rows, values the first 512), the tiny latent attention and 8 heads within a window and with sinks, one row and four, after 70 cached positions and after 1100, the span cut into at most 1, 7, 17 and 40 chunks — one chunk written directly, the fold's first sixteen chunks, more, and more than a warp's lanes; within 1e-5 of each head's largest, every output written, and the values read from rows sliced out of the key rows bit for bit those read from the key rows. `lane_compile_lock::every_attention_takes_the_same_kernel_with_the_rules_held_back` holds every attention's kernel — tiled, every head in one block with its chunks, or its heads a block with theirs — to the same choice with every rule held back, for every tiny architecture and Qwen3, gpt-oss and DeepSeek-V2 wide, under every numerics. `lane_compile_lock::a_wide_chunk_attends_in_tiles` counts DeepSeek-V2's narrow forms reading each cached row once (`lk_attention_shared` and its fold) under both quantized numerics, and Qwen3's and gpt-oss's keeping `lk_attention`.
- `cuda_lane_lock::the_expert_down_takes_several_outputs_a_block_as_a_block_of_one_does` holds `lk_moe_down` with two and four outputs a block, and the planner's choice, to a block of one output bit for bit — three kept experts' 768-wide rows in every weight format with their biases, and Q8_0 and Q5_0 beside a shared expert as wide as an expert's rows (Q4_K) and twice as wide (Q6_K), under the precise, one- and two-word numerics — and each output within 1e-5 of an `f64` sum over the dequantized weights and the activations as each numerics reads them, every block's scale its own so no power of two hides the order of a product.
- `cuda_lane_lock::a_chunks_conv_and_f32_gemv_over_its_rows_at_once_compute_what_its_rows_one_after_another_compute` holds `lk_conv_rows` and `lk_conv_carry` to `lk_conv` fed a row at a time over a run of chunks of 1, 2, 3, 16, 5 and 64 rows — shorter than the window too — at 2 and 4 taps, plain, through silu and with each head's L2 norm, in `f32` and `f64` sums, every output and the ring bit for bit, and `lk_gemv_f32`'s row groups to one block of every row. `a_dense_gemms_last_wave_split_computes_what_the_whole_tiles_compute` now covers Q8_0's split arrays, every block's scale its own. `lane_compile_lock::a_wide_chunk_runs_its_projections_on_the_tensor_cores` counts a Q8_0 chunk's projections on `lk_gemm_blk<8>` and no `lk_gemm_q8`.
- `cuda_lane_lock::the_flash_attention_computes_what_the_attention_computes` holds `lk_attention_flash` to `lk_attention` within 1e-5 of each head's largest — grouped heads, a window with sinks, a head a group, Qwen3's 32 heads over 8 groups, Gemma's 256-wide heads in a window, values narrower than the keys, a 72-wide latent read in place, spans crossing tiles — and its split query and key to the tiles' split; `the_tensor_core_attention_computes_what_the_attention_computes` holds `lk_attention_mma` to `lk_attention` within 1e-3, what rounding the operands to f16 moves, over the same kinds of shape and the split. `lane_compile_lock::a_wide_chunk_attends_in_tiles` counts the one-word numerics' wide attentions on `lk_attention_mma`, the two-word numerics' on `lk_attention_flash`, 288- and 320-wide heads on `lk_attention_tiled`, and checks that the flash attention's shared memory fits every chip's block at every width it takes.
- `cuda_lane_lock::the_chunked_delta_rule_computes_what_the_delta_rule_a_row_at_a_time_computes` holds `lk_delta_prep` and `lk_delta_state` (their scratch sized by `delta_chunk_scratch`) to `lk_delta_step` — four value heads over two key heads, heads 64 and 128 wide, unit keys and queries, write strengths in (0, 1), decays in (0.5, 1), a memory left by earlier rows, over 64, 100 and 512 rows — each output and each row of the memory it leaves within 1e-5 of its largest. `*_meets_every_activation_precisions_bar_on_long_prompts` hold Qwen3.5-0.8B, Llama-3.2-1B, Gemma-3-1B and DeepSeek-V2-Lite to the same bars over `benchmarks/models/prompts-long.txt` — eight prompts of 290 to 770 tokens, code, prose, mathematics, structured data, three languages and a conversation — prefilled 64 rows a chunk, so the wide chunks' paths meet llama.cpp's runs (`golden-long-f32.json`, `golden-long-cpu.json`, made by `scripts/accel/model-golden.sh` on the CPU alone).
- `cuda_lane_lock::an_f32_projection_in_tiles_computes_what_its_f64_product_computes` holds `lk_gemm_f32` to the `f64` product within `√k·ε` of each output's `Σ|x·w|` (and `lk_gemv_f32` within its sum's and bias's rounding), whole and cut into the parts the chip takes, over Qwen3.5's gate shapes, tiles' edges, a `k` ending inside a staged chunk, with and without a bias; the cut tiles sum their parts in one order, a second run the same bits, the counts left at zero. `lane_compile_lock::a_wide_chunk_multiplies_its_f32_projections_in_tiles` holds the lowering: the 64- and 32-row chunks' `f32` projections in tiles with the grid and parts the lane computes, the 16-row chunk and the precise numerics on the GEMV.
- `cuda_lane_lock::a_gemm_in_parts_computes_each_parts_gemm` holds `lk_gemm_blk`'s `G` 4 to each part's GEMM alone, bit for bit without a cut, within 1e-5 of each row's largest with the lane's cut — Q8_0, Q4_K and Q6_K, both quantized numerics, Qwen3.5's four projections, a Llama-like query, key and value over a partial row tile (the value into a cache's rows from a position, the rows beside them left as they were), the narrowest wide chunk, biases. `cuda_lane_lock::a_paired_gemm_computes_what_its_gate_and_up_compute_apart` holds `G` 5 to the gate's GEMM, the up's and the gated launch apart, bit for bit in its `f32` values, quantized words and scales — SwiGLU and a GELU gate, three formats, both numerics, whole waves, a cut last wave and every tile cut. `a_dense_gemms_last_wave_split_computes_what_the_whole_tiles_compute`, `a_gemm_in_parts_computes_each_parts_gemm` and `a_paired_gemm_computes_what_its_gate_and_up_compute_apart` launch the lane's tiles (`gemm_blk_rows`), and `lane_compile_lock::a_wide_chunk_runs_its_projections_on_the_tensor_cores` holds two words to `lk_gemm_blk<8, 2, 0, 64>`. `lane_compile_lock::a_wide_chunk_in_a_block_format_runs_on_the_tensor_cores` now counts each layer's query, key and value (into the cache's rows) in parts, its gated pair paired, its output and down projections a GEMM each. `cuda_lane_lock::the_tensor_core_attention_computes_what_the_attention_computes` adds Gemma-3's 256-wide heads within a window and Qwen3.5's, short and long; `lane_compile_lock::a_wide_chunk_attends_in_tiles` holds 256-wide heads to the tensor cores under one word and flash style under two.
- `cuda_lane_lock::the_q8_0_block_gemm_sums_each_output_as_its_replica_does` holds `lk_gemm_blk<8, Q>`, in the lane's tiles, to its replica on the CPU bit for bit — each block's int8 dot exact, `fma` of the row's and the column's scales block after block, under two words the high and low dots folded into one integer at a 128th, a cut tile's parts each from zero and summed in order, the bias last — under both numerics, whole and cut as the lane cuts on this chip, over `k` of whole stages (the streamed path) and one ending inside a stage (the register path), with and without biases.
- `cuda_attention_gqa_lock::an_attention_over_one_tensor_as_keys_and_values_lifts_it_as_both` checks `over K and K` on the VM against the reference with the keys as the values, and checks that the front end lifts `K` as both operands.
- **The native tier and the Rust tier's representations.** `forge_word_list_lane`, `forge_word_return_lane`, `jit_enum_lock`, `vm_enum_walk_lock`, `jit_perf_map_lock`, `enum_param_borrow_lock`, `construction_move_lock`, `mut_borrowed_seq_copy_lock`, `string_accum_char_append_lock`.
- **`nonstandard_surface`, `supertask_strategies`, `tier_deadline_lock`.** The English surface of infinitesimals (declarations with residue facts, the predicates, the standard part, limit rules, exact declared types); the supertask engine's strategies one program each — affine closed forms, cycles, the growth bound, linear and provisional searches, limit-rule scope — with the loops it leaves as written; and `run_all_tiers_within`/`assert_all_tiers_within`, under which a program that does not finish on every tier fails at a deadline instead of hanging the suite. Unit bodies for `poly`, `algebraic`, `exact_real`, `hyperreal`, `hypernatural`, `limit_value`, `ordinal`, `exact_codec`, `semantics::exact`, `supertask` and `supertask::runtime`.
- **`dt_flows`.** Exponential growth and decay, the harmonic and damped oscillators, a symplectic step whose second order is refused, a step count that depends on the residue of `⌈1/dt⌉`, and a flow's value compared with standard numbers, each under a deadline on both engines. Unit bodies for `germ`, `matrix_exp` and `complex_algebraic`.
- **`jit_region_in_loop_return_lock`.** Functions whose loop returns from inside, called until the loop compiles to a native region and then alternating calls that return from inside with calls that run the loop out: a Boolean flag, an Int flag, a counter read after the loop, a nested loop returning from the inner one, a `While` loop and a Real result, each against the VM and the tree-walker. Forge unit bodies for the constant hoist's liveness condition, and every `compile_region_regalloc` call passes its live-out slots.
- **Every run path for the exact extension's suites.** `common::run_every_run_path` (with `run_every_run_path_within` and `assert_every_run_path_within`) runs a program on the tree-walker and the VM, then on the path `largo run` takes, the optimizer followed by the VM alone and with the native tier, and requires the same output and the same stopping sentence from each. `nonstandard_surface`, `supertask_strategies`, `dt_flows` and `dt_nonlinear_flows` run on it; `nonstandard_surface::a_condition_on_the_residue_waits_for_a_fact` holds a residue condition with and without a fact that settles it. Unit bodies for `analysis::tower` and for the fold's divisibility test beside the extension.
- **The exact extension on AOT-Rust.** Eight more `tier_parity` semantic probes: a function of the infinitesimal (reached parameters and results), a list of hyperreals (index, length, `Push`, `contains`, `Set item`, `Pop`, `Repeat`), bit operations on an exact integer, the lifting builtins on exact values, conditions joined by `and`/`or` with text joined to a hyperreal, the exact types computing, a provisional search called again and again, and a clock inside a function; with the first 24, all 32 run bit for bit on the tree-walker, the VM, `largo run --interpret` and Rust debug and release. `exact_value_model_parity` adds the bit operations, `is approximately` and `followed by` to its operators, and compares the lifting builtins, every collection reading (length, items, membership, item, push) and every builtin of the extension through the shared dispatcher, over a pool with non-ASCII text, a list holding the infinitesimal and a tuple. `nonstandard_surface::text_joins_an_exact_value_as_it_prints`. The AOT wiring lock names `LogosValue` for the five exact types, and the emitter catalogue scopes `TOWER` and `SCOPE`. Unit bodies for `logicaffeine_base::extension` and the builtin-name lock.
- **Recursion and loops over exact values on every engine.** Two more `tier_parity` semantic probes: recursion carrying exact values (a tail call passing an `Int` into a `Hyperreal` parameter, a tail call specialized on a constant, factorial through the accumulator past `i64`, mutual even/odd, a doubly recursive `fib`) and a counter that becomes exact; the exact-types probe adds floor division of a negative and of an infinite integer, and the builtins probe a negative `pow` exponent. `exact_value_model_parity` compares `Pop` and `Set item`; data unit bodies hold a compiled condition on an undecided value and compiled indexing to the interpreter's sentences; `nonstandard_surface::text_meets_an_exact_value_only_by_joining`.
- **`resolution_every_seam_lock`.** A census over the compiler's source (`syn`): every function that calls `parse_program()` calls `resolve_program`, or is named as one that only analyzes or Core-encodes what it parsed, with a stale-entry check and a check that the census finds the seams known to resolve. A Rational division and a supertask through every entry point that runs or emits a program — the Studio, the asynchronous interpreter, the VM's IO runner, the v2 optimizer, the first projection, and, built with rustc and run, `largo build`, the checked compile, the Studio's Rust view, the verified compile and a VM function compiled to native code.
- **The exact core on a value model of its own.** `unit/logicaffeine_base/tower` and `unit/logicaffeine_base/supertask` run `tower` and `supertask` on a model with no interpreter in it (integers, rationals, booleans, text, lists, the exact types): the infinitesimal, canonical forms, residue-dependent integers and text lifted branch by branch, limit rules, provisional evidence, ordinals, transport, trip counts, the lamp's affine power, the closed form's certificate, cycles, searches, state sizes, and Euler steps of growth ending infinitely close to `e`.
- **`dt_nonlinear_flows`.** The logistic equation to its sigmoid's certified digits, a Riccati equation to exactly `1/2`, a nonlinear rotation to `cos(1)` and `sin(1)`, a flow whose rate reads its clock, a flow's printed value, and blow-up before and after the loop's time. Unit bodies for `ode`, the standard and unlimited germs, `ExactReal::flow` and `polynomialFlow`.
- **`logic_determiner_readings`, `logic_infinitary_quantifiers`.** Every quantifying determiner in every subject and object construction is held twice: on the parsed tree (the quantifier it binds and its restrictor's join) and against the reading under `some` or `all` with only the quantifier changed. The infinitary quantifiers are held as tokens, in every output format, in the arithmetic layer and in their lowering to the proof engine, which no finite set of witnesses satisfies.

- **`pop_empty_list_lock`.** `Pop` from an empty list, into a name or not, past the last item, from a field list, from an item of a list of lists and inside a function's loop stops with the sentence naming the list on the tree-walker, the VM, the optimized VM and the JIT, after what was printed; a pop from a list that has items is unchanged. The programs a proven pop could get wrong run there too: a list shadowed in a block, a list a call borrows and empties, a counted loop one pass too long, two pops a pass over `1 to length of xs`, a loop that breaks early. `resolve_pop::nonempty`'s unit tests pin which pops the proof leaves unchecked. `tier_parity`'s semantic probes hold the Rust backends to the same; `c_ast_vm_parity_lock`, `op_backend_matrix_lock` (and the gauntlet corpus harvested from it), `c_bytecode_lock` and `riscv_aot_lock` pin the stop where they pinned a popped `nothing`, with clean pop programs beside them; `nothing`'s arithmetic and truth stay pinned through a zone's name and a function that answers nothing. `wasm_aot_lock` classifies `Fail` as self-contained and runs a checked pop and a stopping one.
- **`exact_by_context`.** `sqrt`, `exp`, `ln`, `pi`/`π` and `e` are exact in a declared exact context and stay `Real` outside one; `omega`/`ω` is ω in an `Ordinal` context (`1 + ω` is `ω`); a bound name or a function of the program is never replaced; an argument at an exact parameter, a comparison with an exact operand and a spine with an exact operand are exact contexts; the exact constructors called by name; the refusals of `sqrt` of a negative and `ln` of zero — on the tree-walker, the VM, the optimized VM and the JIT. Two semantic probes hold the Rust backends to the same, base units pin `tower::real_fn` and the six constructors through `extension::call`, and `guide_coverage` gives them their planned home.
- **`exact_on_c_lock`**: every semantic probe of the extension and sixteen boundary programs (a binding, a parameter and a result, a field, a field declared exact holding a standard number, text joined to an exact value, a list walked, grown and copied, a condition decided or refused, a failure) on the tree-walker and both C backends against the VM; a program the extension never reaches links nothing more, and one it reaches links the archive then every system library rustc names; a loop that makes 200,000 exact values holds a fraction of them at once (`values_high`); a handle reached only through the heap survives a collection at every allocation; the CUDA tier refuses such a program by name; the bytecode carries each declared exact type. `tier_parity::semantic` gains `nonstandard/a_field_of_an_exact_type`, which every tier runs. `c_bytecode_temporal_lock`: the walker shows and orders a date, a span and a time of day as the VM does, and refuses arithmetic on one by name. Unit bodies for the C ABI (`unit/logicaffeine_data/abi`), the archive build (`unit/logicaffeine_compile/codegen_c/value_runtime`) and the ABI census that holds `tower.c`'s declarations to the Rust entries, name and arity (`unit/logicaffeine_compile/codegen_c/tower`). `largo_build_emit_c` builds an exact program on both C lowerings and links `largo emit c -o`'s output with the command it prints.
- **`c_mixed_list_lock`**: a list literal of items of different kinds (a real then integers, an integer then a real, a boolean then an integer, an integer and text either way) runs on both C backends as on the VM or is refused naming it, never printed as another value; lists of one kind keep running as the VM runs them.
- **`exact_on_wasm_lock`**: every semantic probe of the extension and the boundary corpus on the tree-walker and linked WebAssembly against the VM; a self-contained module refuses an exact program naming `wasm-linked`; a module whose program never reaches the extension imports none of the value runtime. The corpus is `tier_parity::exact`, shared with `exact_on_c_lock`, and gains eight programs both locks run: a field read out of a record and grown, a field grown in place while a copy of the record keeps the old list, a global holding its own copy, a loop variable over machine values that lands exact, an exact index into a machine list and range, a tuple among exact values, and a parameter declared `Int`, and one declared `Seq of Int`, joining text as its exact value. `wasm_host_abi_lock` declares the value runtime's imports and `logos_rt_rational_cmp` (all linked-only). `wasm_linked_rational_lock`: a linked module orders, equates and accumulates rationals as the VM does. Unit bodies for `vm::tower`'s rules and for the census that holds every value import of the linked emitter to a function its runtime defines, wasm signature and all (`unit/logicaffeine_compile/vm/wasm/module/tower/tests.rs`). A tier-parity refusal reports everything the backend said, its stage and its reason, not only the first line.
- **`lent_value_lock`**: a list lent to a `mutable` parameter and then stored into a global, a local, a record's field or another list, and changed after, leaves each of those its own copy on the tree-walker, both C backends and both WebAssembly modules as on the VM; a lent list changed in place, and one shared then grown, still reaches its caller.
- **`exact_on_riscv_lock`**: every semantic probe of the extension and the boundary corpus on the tree-walker and RISC-V against the VM; a program the extension never reaches links the shim alone; one it reaches links the value runtime's archive built for RISC-V (its objects RISC-V, no shared unwinder named) then its system libraries; the values a loop lets go of are freed, and a handle reached through the heap survives a collection at every allocation. `tier_parity::exact` gains a `mutable` parameter that hands its exact list back, a tuple holding an exact value, maps of exact values keyed by `Text` and by `Int`, an exact list stored into and read out of a map, each its own copy, a global a function makes exact after Main set it to an integer, and a deep copy of an exact list; `tier_parity`'s RISC-V backend and `support/riscv_runner` link what `codegen_riscv::runtime` names. `lent_value_lock` runs on RISC-V too, and `map_missing_key_traps_lock` holds a generic C map family to the trap.
- **`exact_on_devices_lock`**: the Tenstorrent, analog, Plasticine and RDU tiers, the Silicon ISA, the soft core and the fabric each refuse a program the exact extension reaches by name, and none refuses a program it never reaches for it. A `tier_parity` sweep builds every runtime its backends link once, untimed, before any program is timed — linked WebAssembly's runtime and the value runtime's archive for the host and for RISC-V: a first build of one, on a box whose temporary directory starts empty or after the value model changes, outlasts one program's limit.
- **`supertask_hybrid`, `supertask_imamura_paper`.** Hybrid supertasks on every interpreter tier: the paper's bouncing ball resting after its Zeno time with only its standard bounces recorded, every bounce recorded up to the loop's end, a rebound known positive, a ball dropped from a height and from a height whose fall is irrational (`3√5/5`), a loop ending mid-flight and at the Zeno time, the clock's exact end, a passenger crossing the Zeno time, an elastic ball counting half a million bounces, a thermostat switching for a million seconds, a switch thrown at a set time, a jump checked before the flow, a guard that holds on, a finite clock run as written; and the refusals (a bounce in the last step, counting infinitely many bounces, a condition on values infinitely close, an `Otherwise:` beside a flow, a jump that shows, a flow that is not a polynomial in time, a Zeno tail no scaling settles). `supertask_imamura_paper` runs Imamura's programs end to end. `tier_parity::semantic` gains six hybrid programs every backend runs (a Zeno ball with its germs shown, an algebraic Zeno time, an elastic count, a thermostat, a switch with a text phase, and the last-step refusal). `c_ast_vm_parity_lock` pins an exact quotient inside arithmetic and as a `Rational` argument, and a list declared `Seq of Rational`. `builtin_wire_ids.txt` records `HybridFlow`; the builtin ratchet freezes 261. Unit bodies for `hybrid` (roots by formula and free norm, Lie series, every kind of run, the scaling certificate and its tampering), for `supertask::hybrid_flow` on base's own value model, and for the engine's recognition of a hybrid loop.

### Changed

- `changelog_shape_lock` reads the CHANGELOGs the repository tracks (`git ls-files`), not every `CHANGELOG.md` under the root: the third-party checkouts under `external/` and `tools/` keep their own.
- `version_lock`'s three license tests move to `license_lock`, broadened to the exact MIT text, every shipped package's
  copy and every manifest in the checkout. `packaging_lock` requires `LICENSE.md` in every published crate's `include`.
  `ssr_render`'s two `/pricing` tests follow the page to `/contact`: `the_contact_page_states_the_license_and_the_company_address`
  wants the MIT line where its predecessor wanted the BSL's Logic Service, and the prerender marker is the contact
  heading. `caravel_project_lock` wants an MIT SPDX header on every overlay file and the scaffold's copyright line in
  `user_defines.v`. `landing_page`'s banned claims drop "fully open source", which is now true. The VS Code integration
  suite checks that the extension contributes and registers no license command and that Verify starts `largo verify`
  with no prompt and no extra environment.
- `demo_quality_sides_lock` re-pinned (Tristen's ruling, 2026-10-30): a prompt is the unit — a win at matched quality is
  ours' leading run of agreed steps less the rival's, over the golden's prompts paired, at least zero within two
  standard errors — and every engine the ledger measured is checked at its own key.
- `one_ir_lock` holds two more ratchets that only shrink: no enum outside the IR carries the front end's specs
  (`no_backend_keeps_a_private_op_enum`; the older forward's `Stage` awaits MIGRATE-2) and no backend walks the source
  itself (`every_backend_lowers_the_tensor_graph`; the lane, the engine and the SN50 path await their tickets).
  `spatial_forward_lock` reads the graph's facts and the spatial link in place of the walk's.
- MIGRATE-2's ports: `recognizer_migration_lock` renders its golden from the shared walk through the forward's lens
  (unchanged) and holds `the_forward_lifts_what_the_golden_shows` (every program the forward lowers lifts the golden's
  lines; of those the golden shows lines for, it refuses exactly the tiny Qwen3.8, the gelu probe and the
  partial-rotary probe, each by its kinds); `spatial_recognize_stages_lock` holds that the forward refuses exactly the
  walk's ops it has no kernel for and the tiny Qwen3.8's partial rotary, each in its words;
  `cuda_elementwise_fusion_lock`'s liveness guard is the shared chain rule's; `common/forward_corpus.rs` gives
  `one_ir_identity_lock` the forward's corpus, with a softmax, a layernorm and an attention of narrow heads feeding an
  int8 projection; `one_ir_lock`'s ratchets lose the forward's rows.
- `frozen_reds_lock` finds a suite behind `handed-reds` by its gate line wherever it stands in the file, as the runner
  and `ondemand_exclusion_lock` do, and `fixtures/frozen_reds.tsv` freezes `one_ir_lane_identity_lock`'s two tests under
  IR-0.
- `d5005_lower_graph_lock` reads the step the shared table fuses under the engine's shim, and holds MIGRATE-4:
  `the_engine_reads_fused_symbols` (on every model the engine runs, the fused symbols are exactly its merges, block by
  block, and the reader takes each by name) and `capability_is_the_engine_shim` (the capability module and the shim
  refuse the same architectures, the shim in the capability module's words); `shim_lock` judges the engine's shim beside
  the others.
- Two locks whose time is another toolchain's start with the wall-clock heavies under a 60-minute backstop
  (`.config/nextest.toml`): `qwen38_batched_decode_lock::the_batched_step_is_the_vms_in_spatials_simulator` (Spatial
  compiling and simulating a 316-section app, 995 s alone, 1,322–1,683 s in the full gate) and
  `qwen38_authored_lock::every_emitted_batched_lane_compiles_for_hopper` (four nvcc builds for sm_90a, 347 s alone, up
  to 1,493 s in the gate).
- The eleven co-simulations of `d5005_engine_card_lock` run in `scripts/run-silicon-tests.sh` instead of the fast gate
  (`HEAVY_ONDEMAND_EXCLUDE` and `SILICON_ONLY`); its four fast tests stay in the gate.
- `cuda_lane_lock::a_branched_step_computes_what_its_launches_compute_in_order` compiles its 33 lanes first and builds
  their programs on a pool as wide as the machine (`build_all`), then runs each on the GPU in order. Its time was 33
  nvcc builds one after another, 7.8 s each alone and up to 147 s in the full gate's heaviest stretch, where it reached
  the 30-minute backstop.

- **`run_all_tiers` runs every in-process tier.** Beside the tree-walker and the VM it runs the optimized VM (the optimizer at its full tier, the path `largo run --interpret` takes) and the optimized VM with the native tier compiling hot loops and functions, so each of the behavioral tests built on `run_all_tiers`/`assert_all_tiers` holds the optimizer and the JIT to the tree-walker's output and error text. `Tier` gains `OptimizedVm` and `OptimizedVmNative`; `run_every_run_path` and its deadline forms, which ran the same four for a handful of suites, are folded into it.
- The five FFI codegen snapshots show `main` calling `logicaffeine_system::stack::run_main(_logos_main)`.

- The bootstrap scope census follows the native PE cdylib compile helper and still requires every bootstrap run to enter reference semantics.

- `vdom_size_lock` measures the counter, display and reconciler modules at exact `Int` semantics and records what each costs without exactness. The `device_lift` and `cshake` units cover a documented macro's callee.

- External-tool gates in sixteen GPU, C-bytecode and checkpoint suites route through `common::require_tool`, and `goldens/skip_debt.txt` drops their counts.

- Locks that read generated code follow the exact `Int` rule and the C collector's allocator: `phase_codegen_c`, `runpath_bce` and `runpath_bce_vm` accept the `LInt` signature a function takes where nothing bounds its integers, and `recursive_enum_c_allocation_lock` and `c_bytecode_unique_drop_lock` read `lc_gc_alloc` and `lc_gc_free`. `c_header_names_lock` reads a header behind a platform guard only on that platform. `semantic_probes_lock` and the C gauntlet's divergence ledger drop the programs now fixed (`map_missing_key`, `seq_index_out_of_bounds`, `empty_seq_first_item`, `corpus_int_overflow_at_run_time`).

- `riscv_instruction_budget_lock`'s budgets are the counted-range loop's counts: a `Repeat` over a range iterates in emitted code where the range iterator ran in the uncounted runtime, and whole-program instructions fall (`int_accumulator` −31%, `text_append` −15%).
- The full profile starts its early heavies (the fuzzers, `phase_futamura`, `phase_pe_differential`, the ttsim and Plasticine chains) at priority 90, below the timing group's 100: a timing lock waiting for every test thread held back the whole run while a heavy that had started first ran for minutes.
- The forge unit test of the precise region gate states what the gate admits: a cross-function `Call` compiles through its callee's table slot, and without a status channel the region declines.
- Text-slice and radix-buffer locks check the emitted borrowed helper and owned buffers while preserving byte-identical VM/AOT and optimization-toggle controls.
- The slice and reversal code-shape locks recognize the checked helpers, retain exact-output controls, and reject reversal lowering for a non-converging loop.
- The byte-slice return lock recognizes the borrowed range helper and checks borrowing together with disabled-optimization, escaping-return, and VM/AOT output controls.
- **`lean_conformance_t04_recursion` states what its tests assert.** Every section is `(MUST, Native)` as `verdicts/t04_recursion.json` records, the module header and each formerly-`Gap` section name the mechanism that closes it (the equation compiler, index-allowed coverage, `WellFounded_fix`, `I_below`/`I_brecOn`, equation lemmas, the guard's nested and `let`-alias smallness, `decLtNat`), and T04-006's test is named for what it proves (`a_derived_nat_recursor_iota_computes_and_its_polymorphic_form_reaches_type_one`).
- **The Arena corpus is lean-kernel-arena `66c354d6`: 221 cases (132 accept, 71 reject, 18 either), exported with Lean v4.34.1**, adding `navier-stokes-euler` and `perf/proj-stuck-struct`. Every case is the Arena's exact bytes: `arena_corpus_lock::every_case_is_the_arenas_exact_bytes` holds each row to the SHA-256 `results.json` publishes, whether the bytes came from the official archive or from `scripts/lean/arena-crown/build-arena-inputs.py`, which rebuilds the unpublished inputs from source tarballs the way `lka.py build-test` does. `vendor-corpus.py` no longer reuses previously vendored bytes.
- **Arena scoreboard ceilings belong to the bytes they were measured on.** `ceilings.json` (`logos-arena-ceilings/2`) records each ceiling's input SHA-256 and the live revision: a re-exported test starts a fresh ceiling, and the first-place floor is re-based when the revision moves. `arena_scoreboard_lock` requires every measured row and ceiling to name its corpus case's digest.
- **`arena_corpus_lock`'s input census reads `arena_heavy_case!`'s own body** (`macro_body`, braces matched) instead of everything up to the next `macro_rules!`, so a suite whose heavy macro comes last is judged by that macro alone; `the_census_reads_exactly_a_macros_own_body` holds it. Cedar, whose Lean build two local runs reproduce differently and neither as the Arena's bytes, is the one corpus case recorded with its own digest beside the Arena's (`NONDETERMINISTIC_UPSTREAM`).
- **`dag_trusted_prefix` counts the declarations an export states** from the export's records instead of pinning init-prelude's count (1,777 under Lean 4.29.1, 1,824 under 4.34.1), so the trusted-prefix test that trusts the whole prelude keeps trusting all of it.
- `phase24_wired_types` and `phase26_e2e` read SEMANTICS-1's lowerings (approved ports): a slice is `logos_slice(2, 5)` under the one index/slice rule, and `Assert` is checked in every build.
- **Approved ports for the fair benchmark programs** (checksums on all 35; `collect`, `two_sum`, `binary_trees`, `knapsack` and `string_search` redesigned): `phase_benchmark_interp` reads each program's `expected_<n>.txt`; `optimizer_proof` and `phase_opt_trace` prove `densemap`, `affine`, `cascade`, `indexstring` and `simd` on fixture programs of their own; `benchmark_metadata_lock` reads the registry. `phase_optimize`'s four unchanged-sequence rows assert the shared borrow their names describe, and each runs its program. API ports: `phase102_bridge` (`RuntimeValue::Inductive` holds an `Rc`), `project` (`BuildConfig::backend`), `flamegraph_pdf_download_lock` (the page's module path). `optimizer_name_capture_lock`'s `C_REFUSES` is empty: every row builds as C.
- **The full-suite runner guards its own baseline.** `scripts/run-all-tests-fast.sh` refuses to start beside another test run or an FPGA synthesis, physical-design or emulation job, runs at reduced priority on three quarters of the cores, requires Lean, Chrome, the `reference-kernels/` inputs, the EDA tree and the private-material guard by default (`LOGOS_REQUIRE_TOOLS=lean,chrome,reference-kernels,eda-tools,private-guard`), and takes `--exclude <crate>` and an opt-in `--overlap-doctests`. Each of its six exclusion classes has a companion runner: `scripts/run-research-tests.sh`, `scripts/run-opt-audit-tests.sh`, `scripts/run-silicon-tests.sh`, `scripts/run-kernel-pending-tests.sh`, `scripts/run-qemu-corpus-tests.sh` and `scripts/run-arena-mathlib-tests.sh`. Timing-, GPU-, simulator- and database-bound binaries run in their own capped `.config/nextest.toml` test groups.
- The optimizer-graph, integer-strength and opcode-coverage locks carry their own witness programs (`tests/fixtures/opt_witness`, and a Float-bounded range in `op_coverage_lock`'s corpus), so a benchmark rewritten for its own reasons cannot take an edge's witness with it.

### Removed

- `phase_verification::test_license_plan_verification_access`, with the `LicensePlan` import that kept the whole
  `verification` build from compiling once the plan gate was gone.
- The benchmarks page's unit tests of the Under the hood panel, removed with it: `the_flamegraph_lays_frames_out_by_their_share`, `zooming_widens_a_frame_and_keeps_its_ancestors_as_context`, `frames_too_thin_to_see_are_not_drawn`, `profile_numbers_read_at_the_precision_they_deserve` and `the_counters_table_compares_every_tier_with_c`.

### Fixed

- `codegen_sva::autopipe`'s unit tests build their graphs with the timing graph's fan-out field. `d5005_flow_lock`'s 200
  MHz checks give a stage's parameter that is an expression of numbers its number (`param_number`), so the vector unit
  is timed at the rows its side gives it.
- `spatial_forward_lock::a_joined_section_rounds_what_its_store_rounded` holds a join to the program with its result
  stored: bit for bit across a bf16 page, within the chip's declared bound across an f32 one.
- The board census (`common/board_census.rs`) reads as board tops only the designs that stream bytes out of a
  `uart_tx` pin: the Fabric Terminal's SLIP framer, link and keyboard port name a `tx_byte` but are parts of a
  design, and the framer's END and ESC bytes are no measurement (`measured_fields_lock`, with a test of what a board
  top is). `accel_c_model_lock` and `hart_firmware_lock` take their per-process scratch from `common/scratch.rs`,
  which removes it at exit (`per_process_dir_lock`).

- `vm::wasm::link`'s unit harness defines the value runtime's `logos_rt_fail` sink, which every linked program holding a rational imports since linked WebAssembly refuses in the VM's words: the three `linked_rational_*` tests could not instantiate their modules under `wasm-jit`.
- `c_ast_vm_parity_lock`: a running real that reads an accumulator each loop binds afresh, and a name each loop binds to an integer read into a real.
- The tier-parity harness leaves no build behind: a worker keeps its scratch in its job's directory, which its parent removes with the job, and a sweep releases the Rust batch's project and its private target when it is done (`a_worker_keeps_its_scratch_in_its_job`, `a_sweep_leaves_no_rust_build_behind`). A box's temporary directory had gathered one directory per worker and a Rust target per sweeping process.
- `browser_tier_lock::every_task_due_by_the_end_of_an_advance_has_run_when_it_returns`: 2,000 advances each arm a timer due at their end whose follow-up is queued at that instant, every round's tasks named by the round so one held from a round cannot pass for the next; and a zero advance runs a timer due now. Without the end-of-advance release every one of the 2,000 is held. `a_page_timer_lands_when_its_time_has_passed`: a click arms a fuse for one second and the test advances exactly that, with the page's `performance.now()` held short and shorter at the wake than at the click, the coarsening that left `modal_toast` and `stopwatch` timers unlanded in the example corpus about one run in six.
- `native_seam_lock`: `get` reads a set and an unset variable on the tree-walker and the VM; `now()` is milliseconds; a native no engine implements is refused in one sentence on the tree-walker, the VM and in the Rust backends' compile error, and a Rust function the same way wherever Rust does not link it, a FIPS-202 hash a program declares native included, by the Rust function it names; a program's own `get` is called as written; every native the standard library declares compiles to an implementation on the VM (a census that fails for an unimplemented one); and a session cookie signed under `LOGOS_SESSION_SECRET=probe1` on `largo run --interpret` for the tree-walker, the VM and the JIT is the HMAC-SHA-256 Python's `hmac` computes. The parity corpus gains `env/*` and `time/*` probes on every backend (the harness runs each process with `NO_COLOR=1`; `now()` compared with an epoch-sized literal, and `sleep` given an `Int`, on AOT-Rust too), and builtin probes for `EnvGet` and `TimeNow`. `wasm_alloc_site_lock` counts the two allocation sites of the Text `get` builds on WebAssembly and reads the `TextHeader` stamp off it at run time.
- `nextest_schedule_lock::every_test_that_reaches_the_gpu_takes_its_turn_on_the_one_device`: the `cuda-gpu` group (one test on the GPU at a time) selects every test binary whose source probes the device — the `cuda_*` class by prefix and each GPU binary outside it by name, the `sass_*_gpu` timing locks among them — exactly, in every profile; a GPU binary outside the group, a name that no longer probes the device, or a filter term the census cannot read fails it. The full profile starts the group first.
- `benchmark_harness_fixture_lock::no_phase_waits_on_a_scripts_file_mode`: `run.sh` reaches every script of its own through `.`, `bash` or `python3` and tests none for its executable bit. `readme_benchmark_numbers_lock::the_charts_are_what_the_generator_draws_from_the_results` redraws the README charts from the committed results into a temporary directory, the generator's label checks included, and requires every file to equal the committed one. `benchmark_profile_assemble_lock::the_profile_script_calls_the_assembler_with_the_flags_it_declares` holds `profile-benchmarks.sh`'s call to the flags `profile_assemble.py` accepts and requires. `solver_bench_provenance_lock::a_raced_solver_that_cannot_answer_stops_the_run_by_name` stubs the raced solvers and refuses one that fails as a binary whose library is gone.
- `concurrency::marshal::tests::build_in_place_encode_is_at_capnp_parity` times each encode after one untimed round of both, in alternating rounds judged by their best, with the frame's checksum off on both sides. It timed a single cold round of each with the FNV pass on, so whichever ran first carried the process's warm-up (2 runs in 20 failed with the code at parity) and the shared checksum hid the encodes (an encoder doing three times the work still passed); it now passes its 1.5× bound at 1.5× to spare and fails that encoder every run.
- `session_block_lock::the_session_cookie_is_signed_rather_than_readable` reads the cookie as its format: 64 lowercase hex digits of HMAC-SHA-256 after the last `|`, every payload field lowercase hex, one of them the visitor's name. The name now has letters no hex digit spells; a name spelled in hex digits appeared by chance in the MAC under the per-process random key about one run in 67.
- Optimizer-fact controls copy every expression fact through an AST rebuild and exercise the real monomorphization map path, including integer-strength proofs that previously disappeared on generic-free backends.
- Fixed-list fabric controls cover interleaved buffers, source functions, zero general reserve and CFG capacity contracts with exact VM parity. The RDU ledger carries all 120 ISA operations and 118 hosted operations; explicit ordinary-list bytecode preserves its executable coverage witness (fabric_fixed_list_lock, rdu_op_coverage_lock).
- Runtime-link cache controls exercise cleanup, archive replacement, retry after build and link failures, and two native processes overlapping the build-to-runtime-compilation window (cache_snapshot_tests, cache_process_tests).
- Mixed native swap controls retain exact tree-walker and VM output while requiring native execution through roundtrips, loop exits, overflow deoptimization, and Boolean/Word64 exchanges (jit_scalar_swap_edges_lock).
- Generated release replay programs preserve the output and failure status across six seeds after a task fails. Deadlock diagnostics follow buffered output on a combined stream. Ordinary programs and user text containing replay code retain the ordinary panic profile. [Regression: replay_failure_profile_lock, e2e_deterministic_replay]
- WASM linker scratch controls force two linkers to overlap, compare each returned program exactly, preserve caller files, and check successful and failed invocation cleanup (scratch_isolation).
- Record-field ownership checks require no copies for growing unaliased local or global records, preserve genuine aliases, and retain earlier arguments across later argument calls. Scaling checks cover nine list, map and record mutation shapes (record_copy_work, vm_cow_scaling_timing_lock).
- Emulator wrapper controls verify inherited hard and soft caps, stricter requested caps, disabled caps, and exact guest stderr and exit status, including a soft cap that the hard limit would permit raising (qemu_wrapper_limits_lock, qemu_wrapper_lock).
- Linked-runtime artifact fixtures reject stale and host archives and require the successful build’s primary library output to remain in the linker dependency set.
- `lean_bridge_lock` reads the generated refusal tests in any layout: each `Refusal { … }` literal splits at the commas outside its strings and brackets, so rustfmt's multi-line taint paths read as the generator's single lines do, and a test the scanner cannot read fails the check by name with its reason instead of dropping out of it (the_refusal_scanner_reads_every_layout, the_refusal_scanner_names_every_test_it_cannot_read).
- The memory-capped `qemu-riscv64` wrapper (`scripts/test/toolwrap`) keeps an inherited address-space limit that is already at or under its cap, and writes nothing. Under `run-all-tests-fast.sh` the parity harness runs every RISC-V guest under its own 512 MiB cap; the wrapper could not raise it, and its warning landed in each guest's stderr, so every RISC-V row of `machine_sympathy_corpus` diverged from the VM. `qemu_wrapper_lock` reads the limit from a static RV64 guest's `/proc/self/limits`: an inherited cap kept in silence, an unlimited run capped at 8192 MB, the override and `0` honoured.
- `scripts/test/wasm-jit-filter.txt` lists every `wasm-jit`-gated suite again (`wasm_gated_suite_lock`): `in_out_lock`, `swap_every_tier_lock`, `exact_int_every_tier_lock` and `wasm_word_copy_lock` were dark to the gated runner. The `vm::machine::host_calls` fixtures walk a Text (a recorded port): a range of Ints is now a counted loop with no iterator, so `deep` failing inside its calls left no iterator open and the quiesce tests stopped exercising one. `wasm_aot_lock` classifies `NewEmptyListFixed`.
- `wasm_word_copy_lock` checks scalar copies, record fields, indexed writes, append, value parameters, and maximum word payloads against the VM. Linked-runtime units cover both word widths through the inline clone fallback.
- `optimizer_in_out_specialization_lock` checks caller writeback, remapped parameter positions, paired outputs, mutable temporaries, and continued specialization of ordinary static arguments.
- `optimizer_in_out_leaf_lock` checks caller updates for scalar, forwarded, paired, and text arguments against the tree walker and both VM modes, with an ordinary value-parameter control.
- Arena units cover exact destructor counts, reset epochs, growth, zero-sized values, iterator and destructor panics, nested allocation, and references between nodes. The same rows run under Miri to check pointer and ownership safety.
- The semantic tier corpus compares the first failure of tiled nests with differently sized rows on both generated Rust profiles. Optimizer facts units statically reject moving or sharing a scope across threads.
- `riscv_equality_lock` pins the tree-walker, VM and RISC-V to the same output for aliased NaN payloads, list identity and deep copies, nominal record types, exact Int/Float comparison at the precision boundary, Bool/Int separation, empty sequences, nested tuples, optional payloads, temporal sequence elements, and exact Rational/Float equality over signs, precision boundaries, subnormal values, infinities and NaN. Runtime rational cases hold whole-rational/Int equality and ordering whose cross-products exceed 64 bits.
- RISC-V equality contracts cover absent Option/Result payloads, deep copies, nested numeric layouts and declared constructor boundaries. Corpus differential checks preserve every stdout byte and require matching stderr and exit status. `riscv_equality_lock`, `riscv_qemu_corpus_differential`.
- **The post-synthesis gate-level locks simulate every block RAM the netlist uses.** `crates/logicaffeine_tests/tests/assets/ramb_behav.v` modelled TDP ×1/×9/×36 and SDP-72 only, while the depth-banked heap maps to ×2 tiles, so every `postsynth_*` sim that reads the heap lost its heap writes and read the output latch's power-up value. The model covers TDP ×1/2/4/9/18/36 and SDP-72/36 over the device's data and parity layout, powers up as the device does (INIT_xx/INITP_xx and INIT_A/INIT_B, an x bit as 0), and stops the sim naming any configuration it does not implement. Every gate-level sim builds it with a seeded random power-up, which the testbench prints and the harness requires, so a dropped write cannot match the VM by coincidence; the testbench stops on the cycle a held FSM register goes X and at a cycle budget derived from the program, printing where the FSM stood. `postsynth_reverting_the_heap_fix_diverges_from_the_vm` injects bug #1, heap bank 0 without its depth decode, into the synthesized netlist once it has checked that tiling is there (yosys 0.33 decodes a depth-tiled heap correctly at every depth), and requires the aliased 999. The `^postsynth_` backstop in `.config/nextest.toml` is derived from the slowest test's worst case, and `scripts/run-silicon-tests.sh` keeps the sims off the slowest core class. Locked by `wpe_postsynth_silicon_lock::the_ramb_model_round_trips_every_width`, `wpe_postsynth_silicon_lock::the_ramb_model_powers_up_as_the_silicon_does`, `wpe_postsynth_silicon_lock::the_ramb_model_refuses_what_it_does_not_implement`, `wpe_postsynth_silicon_lock::the_gatelevel_testbench_stops_a_stalled_netlist_and_names_it` and `wpe_postsynth_silicon_lock::the_heap_decode_drop_rewires_every_bank0_strobe_and_refuses_an_untiled_bank`.

## [0.10.0] - 2026-07-08

Synced to workspace version 0.10.0. See root CHANGELOG for full history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial release (internal test suite, not published to crates.io).

### Added

- Comprehensive test suite across all logicaffeine crates
- Phase-organized tests by linguistic complexity
- End-to-end tests: collections, functions, structs, enums
- Phase 41: Event adjectives
- Phase 42: DRS (Discourse Representation Structures)
- Phase 50: Security policy analysis
- Phase 85: Memory zones
- Grand challenge: Mergesort verification
