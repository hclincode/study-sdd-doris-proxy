---

description: "Task list template for feature implementation"
---

# Tasks: Row-Cap Rewriting Proxy for Doris

**Input**: Design documents from `/specs/001-l7-mysql-proxy/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: Test tasks are included and are NOT optional for this feature. Constitution Principle IV
(Rewriter Is Test-First) requires each rewrite rule to have a paired positive and negative case written
before the rule itself. The negative cases are the ones that catch Principle I violations.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Single project**: `src/`, `tests/` at repository root, per plan.md's Structure Decision

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and basic structure

- [ ] T001 Create the crate and directory skeleton from plan.md: `src/{proxy,rewrite,observe,config}/`, `tests/{contract,integration,unit}/`
- [ ] T002 Initialize `Cargo.toml` with `sqlparser`, `tokio`, `tracing`, `tracing-subscriber`; dev-dependencies `proptest`
- [ ] T003 [P] Configure `rustfmt.toml` and `clippy.toml`; add a CI job running `cargo fmt --check` and `cargo clippy -- -D warnings`
- [ ] T004 [P] Add `cargo-mutants` config scoped to `src/rewrite/classify.rs` (per quickstart.md Scenario 3)
- [ ] T005 Assemble the reference corpus of at least 200 real statements from recorded traffic into `tests/fixtures/corpus.sql`, required by SC-002 and by the research.md R3 parse-rate measurement
- [ ] T006 Measure the `MySqlDialect` parse-failure rate over the corpus and record it; this is the SC-003 baseline and gates whether the design assumption in research.md R3 holds

**Checkpoint**: Toolchain ready and the corpus exists. T005/T006 gate nothing in code but gate the SC-002 and SC-003 claims.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T007 Define `CapDecision`, its variants, and the closed reason set in `src/rewrite/reason.rs` per data-model.md
- [ ] T008 [P] Define `CapPosition` with its `kind` and `safety` fields in `src/rewrite/classify.rs`, with the default arm returning `Other`/`Unsafe` (classifier invariant C2)
- [ ] T009 [P] Implement statement fingerprinting with literal normalization in `src/observe/fingerprint.rs`
- [ ] T010 Implement the `RewriteRecord` struct and its `tracing` emission in `src/observe/record.rs` per contracts/rewrite-record.md, including the privacy rule that original SQL is never recorded
- [ ] T011 [P] Implement config loading (listen address, upstream address, cap value) in `src/config/mod.rs`
- [ ] T012 Implement the `decide(sql, cap) -> CapDecision` entry point signature in `src/rewrite/mod.rs` as a total function that returns `ForwardedUnchanged { NoSafePosition }` for everything; every later task narrows this default
- [ ] T013 [P] Write the totality property test (D1) in `tests/unit/roundtrip_props.rs`: no input, including arbitrary bytes, causes a panic or an error return

**Checkpoint**: `decide` exists, is total, and caps nothing. Every user story now adds behavior to a safe baseline rather than removing behavior from an unsafe one.

---

## Phase 3: User Story 1 - Unbounded exploratory query is capped (Priority: P1) 🎯 MVP

**Goal**: A bare `SELECT` with no row count returns at most 200 rows.

**Independent Test**: `SELECT * FROM events` through the proxy returns exactly 200 rows; `SELECT * FROM small_lookup` returns all 12.

### Tests for User Story 1 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T014 [P] [US1] Positive case for classifier position P1 in `tests/contract/classifier_positions.rs`: `SELECT a FROM t` yields exactly one `Safe` position
- [ ] T015 [P] [US1] Negative case for P1 in `tests/contract/classifier_positions.rs`: `SELECT 1` yields no `Safe` position
- [ ] T016 [P] [US1] Invariant C1 test in `tests/contract/classifier_positions.rs`: no statement in the corpus yields two `Safe` positions
- [ ] T017 [P] [US1] Reason reachability for `NoRowSet` in `tests/contract/decision_reasons.rs`: `SET x = 1` and `USE db` forward unchanged
- [ ] T018 [P] [US1] Integration test in `tests/integration/passthrough.rs`: a statement returning no row set is relayed unchanged in both directions

### Implementation for User Story 1

- [ ] T019 [US1] Classify `OutermostQuery` as `Safe` in `src/rewrite/classify.rs` (position P1); T014 and T015 must both pass
- [ ] T020 [US1] Implement cap application at a safe position in `src/rewrite/apply.rs`: set the limit when absent
- [ ] T021 [US1] Implement AST serialization of the rewritten statement in `src/rewrite/apply.rs` (FR-009, serialization only, no string splicing)
- [ ] T022 [US1] Detect statements that return no row set and return `ForwardedUnchanged { NoRowSet }` in `src/rewrite/mod.rs` step 3
- [ ] T023 [US1] Implement the listener and per-client accept loop in `src/proxy/listener.rs`
- [ ] T024 [US1] Implement the one-client-to-one-backend session pairing in `src/proxy/session.rs` (FR-012), with passthrough authentication relayed untouched (FR-013)
- [ ] T025 [US1] Implement the statement relay path in `src/proxy/relay.rs`: call `decide` once per statement, forward the decision's text, relay the response
- [ ] T026 [US1] Emit a `RewriteRecord` for every statement on this path, including pass-throughs

**Checkpoint**: The MVP. A plain unbounded `SELECT` is capped; everything else passes through untouched. This alone delivers the load reduction for the majority of ad-hoc traffic.

---

## Phase 4: User Story 2 - Aggregates and joins keep returning true values (Priority: P2)

**Goal**: No cap is ever placed in a position whose rows feed another operator. This is the story that makes the proxy safe to put in front of real traffic.

**Independent Test**: the corpus differential run reports zero value discrepancies outside truncation.

### Tests for User Story 2 ⚠️

> **NOTE: Each pair below is one row of the required-test-pairs table in contracts/cap-position-classifier.md. Write both halves before the matching rule.**

- [ ] T027 [P] [US2] P4 pair in `tests/contract/classifier_positions.rs`: `SELECT * FROM (SELECT a FROM big) t` caps the outer query only; `SELECT COUNT(*) FROM (SELECT a FROM big) t` places no cap on the derived table
- [ ] T028 [P] [US2] P2/P3 pair in `tests/contract/classifier_positions.rs`: a `UNION ALL` yields one `Safe` position at the set-operation root and zero among its branches
- [ ] T029 [P] [US2] P5 pair in `tests/contract/classifier_positions.rs`: a `WITH` clause body is never capped; the outer query is
- [ ] T030 [P] [US2] P6/P7 pair in `tests/contract/classifier_positions.rs`: `IN (SELECT …)` and `EXISTS (SELECT …)` sub-queries are never capped
- [ ] T031 [P] [US2] P8 pair in `tests/contract/classifier_positions.rs`: a scalar sub-query is never capped
- [ ] T032 [P] [US2] P9 negative case in `tests/contract/decision_reasons.rs`: `INSERT INTO s SELECT a FROM big` yields no cap and the reason `WriteCarryingQuery` (FR-007)
- [ ] T033 [P] [US2] Differential test harness in `tests/integration/differential.rs`: run each corpus statement direct and through the proxy, compare cell by cell, ignoring rows absent solely due to truncation (SC-002)
- [ ] T034 [P] [US2] Verification-failure test in `tests/contract/decision_reasons.rs`: a statement whose serialized form does not re-parse to the intended AST yields `VerificationFailed` and is forwarded unchanged

### Implementation for User Story 2

- [ ] T035 [US2] Classify `SetOperationRoot` as `Safe` and `SetOperationBranch` as `Unsafe` in `src/rewrite/classify.rs` (P2, P3)
- [ ] T036 [US2] Classify `DerivedTable`, `CteBody`, `InPredicate`, `ExistsPredicate`, `ScalarSubquery` as `Unsafe` with their causes in `src/rewrite/classify.rs` (P4–P8)
- [ ] T037 [US2] Detect write-carrying queries and return `ForwardedUnchanged { WriteCarryingQuery }` in `src/rewrite/mod.rs` step 4, ordered before classification so writes report their own reason (P9)
- [ ] T038 [US2] Implement the post-serialize verification pass in `src/rewrite/verify.rs`: re-parse and compare structurally, downgrading to `ForwardedUnchanged { VerificationFailed }` on mismatch
- [ ] T039 [US2] Enforce classifier invariant C1 in `src/rewrite/mod.rs`: more than one `Safe` position is treated as unclassifiable, never resolved by picking one
- [ ] T040 [US2] Run `cargo mutants --file src/rewrite/classify.rs` and close any missed mutant by adding the test that would have caught it (Principle IV)

**Checkpoint**: the proxy is deployable to real traffic. US1 still works; no aggregate, join, union, CTE or predicate sub-query can be corrupted by a cap.

---

## Phase 5: User Story 3 - A limit the user wrote is honored (Priority: P3)

**Goal**: an existing row count is reduced to the cap when larger, left alone when smaller, and the offset is never touched.

**Independent Test**: `LIMIT 50` returns 50; `LIMIT 500` returns 200; `LIMIT 50 OFFSET 1000` returns rows 1001–1050.

### Tests for User Story 3 ⚠️

- [ ] T041 [P] [US3] Unit tests in `tests/unit/apply_limit.rs`: existing limit below the cap yields `AlreadyBounded` with the statement forwarded byte for byte unchanged (D3)
- [ ] T042 [P] [US3] Unit tests in `tests/unit/apply_limit.rs`: existing limit above the cap is reduced to 200
- [ ] T043 [P] [US3] Offset-preservation tests in `tests/unit/apply_limit.rs` across present/absent offset with limits above and below the cap (D5)
- [ ] T044 [P] [US3] Idempotence property in `tests/unit/roundtrip_props.rs`: deciding on a previously capped statement yields `AlreadyBounded`, never a doubled limit (D2)

### Implementation for User Story 3

- [ ] T045 [US3] Read `existing_limit` and `existing_offset` at the safe position in `src/rewrite/classify.rs`
- [ ] T046 [US3] Implement the minimum rule in `src/rewrite/apply.rs`: `min(existing, cap)`, returning `AlreadyBounded` when the existing count already satisfies the cap (FR-006)
- [ ] T047 [US3] Ensure the offset is read-only on every path in `src/rewrite/apply.rs`

**Checkpoint**: pagination and deliberate limits behave predictably. US1 and US2 unaffected.

---

## Phase 6: User Story 4 - Operators can see what the proxy did (Priority: P4)

**Goal**: every statement's handling is answerable from records alone, and a truncated client learns it was truncated.

**Independent Test**: submit one statement of each decision kind, then classify all four correctly from the log output alone.

### Tests for User Story 4 ⚠️

- [ ] T048 [P] [US4] Record-completeness test in `tests/contract/decision_reasons.rs`: exactly one record per statement, including pass-throughs, with no gaps across a multi-statement submission
- [ ] T049 [P] [US4] Field-presence test: `reason` is present exactly when the decision is `forwarded_unchanged`; `forwarded_sql` and `applied_limit` exactly when it is `capped`
- [ ] T050 [P] [US4] Privacy test: no record for a pass-through or parse failure contains the original statement text
- [ ] T051 [P] [US4] Advisory test in `tests/integration/passthrough.rs`: an advisory is attached for `Capped` and for nothing else, and does not change the rows or columns returned (FR-008, SC-008)

### Implementation for User Story 4

- [ ] T052 [US4] Populate every `RewriteRecord` field per contracts/rewrite-record.md in `src/observe/record.rs`, including `elapsed_micros`
- [ ] T053 [US4] Emit the FR-008 truncation advisory on the response path in `src/proxy/relay.rs` for `Capped` decisions only
- [ ] T054 [US4] Assign `connection_id` and per-submission `sequence` in `src/proxy/session.rs` so FR-011 independence is visible in records
- [ ] T055 [US4] Document the three operator queries from contracts/rewrite-record.md in `docs/runbook.md`

**Checkpoint**: all user stories independently functional.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Improvements that affect multiple user stories

- [ ] T056 [P] Write `docs/deployment.md` covering the cap value, listen and upstream configuration, and the restart-drops-connections behavior
- [ ] T057 Measure p99 `elapsed_micros` and end-to-end added latency against SC-004; optimize only if the budget is exceeded
- [ ] T058 Measure total rows returned across a representative day with and without the proxy against SC-005, and record the honest capped-statement share — expected to be well below what the original request implied, per contracts/rewrite-record.md
- [ ] T059 [P] Re-run `cargo mutants` across `src/rewrite/` and close remaining gaps
- [ ] T060 Run every scenario in quickstart.md end to end and record the results as the release gate
- [ ] T061 Review the two Complexity Tracking deviations in plan.md against the finished code; remove them or promote them to a constitution amendment, per the Governance section

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - US1 must land before US2 in practice, because US2's differential test needs a running proxy
  - US3 and US4 can proceed in parallel with each other once US1 is done
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P2)**: Can start after Foundational (Phase 2); its unit-level classifier tests need nothing from US1, but its differential test (T033) needs the relay path from T023–T025
- **User Story 3 (P3)**: Can start after Foundational (Phase 2); touches only `apply.rs` and is independently testable without the network
- **User Story 4 (P4)**: Can start after Foundational (Phase 2); T053 needs the relay path from T025

### Within Each User Story

- Tests MUST be written and FAIL before implementation (Constitution Principle IV, non-negotiable here)
- Classification before application: a position must be labelled before anything is placed at it
- Application before serialization; serialization before verification
- Story complete before moving to next priority

### Parallel Opportunities

- T003, T004 in Setup
- T008, T009, T011, T013 in Foundational
- All of T014–T018 (US1 tests) together
- All of T027–T034 (US2 tests) together — eight independent test files' worth of cases, and the largest parallel block in the plan
- All of T041–T044 (US3 tests) together
- All of T048–T051 (US4 tests) together
- US3 and US4 implementation blocks by different developers once US1 has landed

---

## Parallel Example: User Story 2

```bash
# Launch all classifier position pairs for User Story 2 together:
Task: "P4 pair: derived table not capped, COUNT(*) stays correct, in tests/contract/classifier_positions.rs"
Task: "P2/P3 pair: union root capped, branches not, in tests/contract/classifier_positions.rs"
Task: "P5 pair: CTE body never capped, in tests/contract/classifier_positions.rs"
Task: "P6/P7 pair: IN and EXISTS sub-queries never capped, in tests/contract/classifier_positions.rs"
Task: "P8 pair: scalar sub-query never capped, in tests/contract/classifier_positions.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - `decide` total and capping nothing)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: quickstart.md Scenarios 1 and 4
5. Do NOT deploy to shared traffic yet — US1 alone caps only the outermost query, which is safe, but the pass-through paths of US2 are not yet proven by the differential run

### Incremental Delivery

1. Setup + Foundational → a rewriter that decides "no" to everything
2. US1 → the cap works on plain selects → validate → demo
3. US2 → the cap is proven never to corrupt a result → validate → **this is the deploy gate**
4. US3 → explicit limits and pagination behave → validate → deploy
5. US4 → the proxy becomes debuggable → validate → deploy

### Parallel Team Strategy

1. Team completes Setup + Foundational together
2. One developer takes US1 through the relay path, since it is the only story touching `src/proxy/`
3. A second developer takes US2's classifier work in `src/rewrite/classify.rs` in parallel — it needs no network
4. US3 and US4 pick up once US1's relay path exists

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing — for this feature the negative cases are the point, and a negative test that passes before its rule exists is proving nothing
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
