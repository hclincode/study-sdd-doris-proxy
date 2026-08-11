# Survey: Editing OpenSpec Artifacts After `/opsx:propose` — By Hand or Through the Tool?

> **Template version:** 1.0 · **Surveyed:** 2026-08-09 · **Author:** agent (Claude Opus 5, hands-on experiments against openspec v1.8.0 on this machine)
> **Scope:** Answers one process question — once `/opsx:propose` has written `proposal.md`, `design.md`, `tasks.md` and the delta specs, what is the sanctioned way to change them. Covers the CLI's actual write surface, what the markdown parser tolerates and rejects, where errors surface (validate vs. archive), what is silently discarded, and the token cost of each route. Excludes the content question (whether the artifacts say the right thing) and excludes `/opsx:apply` code-editing behaviour.
>
> *(Adapted: this characterises a **practice**, not a product. Template section names retained; §1 values reinterpreted; §8 repurposed from "Rust/systems notes" to "notes for this project"; §12 added because the request was for a recommendation, not a survey alone.)*

---

## ⚠️ Lead finding — the question contains a false premise

**There is no CLI command that edits artifact content.** Not for `proposal.md`, not for `design.md`, not for `tasks.md`, not for `spec.md`. The choice "hand-edit vs. use the CLI" does not exist, because the second option was never on the menu.

Every `openspec` subcommand was enumerated at v1.8.0 [E0]. The complete set of commands that write anything at all:

| Command | What it writes |
|---|---|
| `init` / `update` | agent instruction files and `openspec/config.yaml` |
| `new change <name>` | a directory containing **only** `.openspec.yaml` (two lines: `schema`, `created`) — no artifact stubs [E14] |
| `archive <name>` | merges delta specs into `openspec/specs/`, moves the change to `changes/archive/<date>-<name>/` |
| `config set` / `config edit` / `schema fork` / `schema init` / `store *` | machine configuration, never artifacts |

`list`, `show`, `view`, `status`, `validate`, `instructions`, `templates`, `doctor`, `context` are all **read-only**. The artifacts are written by whoever holds a text editor — a human, or an agent running `/opsx:propose`, `/opsx:update`, `/opsx:sync` or `/opsx:apply`. Those slash commands are not a CLI path; they are the same hand-editing, performed by a model that has read the rules first.

Upstream says so directly. `docs/editing-changes.md` [S2] offers exactly two routes and calls direct editing sufficient — *"nothing else is required"* — with the guidance *"Small wording tweak? Edit the file. Substantive rethink? Let the AI revise with full context."* The `/opsx:propose` skill is built for it: it instructs the agent to re-read dependency artifacts from disk *"even if you saw them earlier in the conversation (the user may have edited them)"* [S3].

**So the real question is: your editor, or `/opsx:update`?** That is a question about *coherence across artifacts*, not about tool correctness. §12 answers it.

**Second finding, and the one with teeth:** `openspec validate --all --strict` is **not** a proxy for archivability. Three separate hand-edit mistakes pass strict validation and are only caught when `archive` runs — where they abort atomically [E10, E11, E12]. And exactly one class of hand edit is **lost in silence**: prose written in a delta spec outside a `### Requirement:` block [E13].

---

## 1. Snapshot

| Field | Value |
|---|---|
| Name | Post-`/opsx:propose` artifact revision |
| Vendor / owner | OpenSpec (Fission-AI), v1.8.0, global npm install on this machine |
| License | N/A (practice). Tool: MIT |
| First release | N/A |
| Latest version (as of survey date) | Behaviour verified against openspec 1.8.0, workflow profile `core` |
| Popularity signal | N/A |
| Maintenance signal | Upstream ships a dedicated doc page for it (`docs/editing-changes.md`), so the practice is supported, not tolerated |
| Primary interface | **A text editor.** Optionally mediated by `/opsx:update`, `/opsx:sync` |
| Agent compatibility | Both routes leave identical plain markdown; no agent lock-in |
| Language/stack neutrality | Total — the artifacts are markdown |
| Greenfield vs brownfield bias | None |

**Installed profile matters.** `openspec config list` reports `profile: core`, workflows `propose, explore, apply, update, sync, archive` [E15]. The expanded-profile commands the upstream docs reach for — `/opsx:continue`, `/opsx:new`, `/opsx:verify` — **are not installed here**. That is load-bearing: `docs/editing-changes.md` recommends `/opsx:verify` as the drift detector after manual edits [S2], and this workspace does not have it. `/opsx:update`'s coherence pass is the only reconciliation mechanism available.

## 2. One-paragraph thesis

OpenSpec keeps **no database and no derived state**. `openspec status` is file-existence only — the propose skill states this outright ("`status` is file-existence only") [S3] and it holds under test: deleting a change's `.openspec.yaml` leaves `validate` passing and `status` reporting 4/4 artifacts complete [E8], and a capability directory created by hand with `mkdir` and a text editor appears in `artifactPaths.specs.existingOutputPaths` on the very next `status` call [E9]. Nothing can desynchronise, because there is nothing to synchronise with: the markdown *is* the state. That makes hand-editing structurally safe in a way it would not be under a tool that maintained an index. What hand-editing risks instead is two much narrower things — **breaking the one machine-read grammar** (delta spec files, parsed by `validate` and `archive`), and **breaking coherence between artifacts** that no tool checks at all. The first is mechanically caught, with one exception. The second is caught by nobody, and is the entire argument for routing substantive edits through `/opsx:update`.

## 3. Workflow / artifact model

**Who writes what.** After `/opsx:propose` completes, the write surface looks like this:

```
openspec/
├── config.yaml                      ← human. Never touched by the CLI after init.
├── specs/<capability>/spec.md       ← WRITTEN BY `archive` (programmatic merge)
│                                      or by `/opsx:sync` (agent-driven merge).
│                                      Hand-edited directly for exactly one
│                                      documented case: the Purpose of an
│                                      existing capability [S1].
└── changes/<name>/
    ├── .openspec.yaml               ← `new change`. Holds schema + skip_specs +
    │                                  retire_capabilities markers. Leave it alone.
    ├── proposal.md                  ← free-form markdown. No machine grammar.
    ├── design.md                    ← free-form markdown. No machine grammar.
    ├── tasks.md                     ← checkbox state IS machine-read (`- [x]`)
    └── specs/<capability>/spec.md   ← THE ONLY STRICT GRAMMAR IN THE SYSTEM
```

**Command surface for revision:** none from the CLI. The available routes are your editor, `/opsx:update` (revise + reconcile, per-artifact confirmation, planning artifacts only [S4]), and `/opsx:sync` (push deltas into main specs mid-flight without archiving [S5]).

**Artifact lifecycle:** delta specs accumulate under a change; `archive` merges them into `openspec/specs/` and files the change under `changes/archive/<YYYY-MM-DD>-<name>/`. Verified end to end: a clean archive reported `+ 15, ~ 0, - 0` and produced canonical main specs in `# <cap> Specification` / `## Purpose` / `## Requirements` shape [E-arch1].

**An asymmetry worth noting.** `archive` merges **programmatically** — it parsed and applied 15 requirements with no model involved [E-arch1]. `/opsx:sync` merges **by agent**: its own skill says *"This is an agent-driven operation — you will read delta specs and directly edit main specs"* [S5]. So even the officially blessed mid-flight sync path is an LLM hand-editing markdown. There is no privileged machine channel to be loyal to.

## 4. What it enforces vs. what it suggests

Thirteen hand edits were applied to a real copy of `add-row-filter-proxy-mvp` in a throwaway OpenSpec root and run through `validate --strict`, `show --deltas-only` and `archive`.

| # | Hand edit | `validate --strict` | Verdict |
|---|---|---|---|
| E1 | `## ADDED Requirements` → `## Added Requirements`, and → `## ADDED requirements` | **valid**; 15 deltas still parsed; operation still recorded as `ADDED` | tolerated — header matching is case-insensitive |
| E2 | Delete every `#### Scenario:` under one requirement | **ERROR** — *"must include at least one scenario"* | blocked |
| E3 | `SHALL` → `must` throughout a file | WARNING ×5 — *"should contain SHALL or MUST (RFC 2119)"* | **passes**; archives fine |
| E4 | Append a hand-written requirement with no scenario | **ERROR** — named the requirement | blocked |
| E5 | `### Requirement:` → `### Req:` | INFO per header + **ERROR** *"Delta sections ## ADDED Requirements were found, but no requirement entries parsed"*; 5 deltas vanished from `show` | blocked, and the INFO names every dropped header |
| E6 | `#### Scenario:` → `#### scenario:` | **valid**; scenarios still parsed | tolerated — case-insensitive |
| E7 | Scenario written at `###` instead of `####` | INFO + **ERROR** *"must include at least one scenario"* | blocked — **contradicts the tool's own instruction text**, which warns "Using 3 hashtags or bullets will **fail silently**" [S1]. In 1.8.0 it does not; it fails loudly |
| E8 | Delete a change's `.openspec.yaml` | **valid**; `status` still reports 4/4 complete | tolerated *on the default schema only* — the file carries `schema`, `skip_specs`, `retire_capabilities`; deleting it loses those |
| E9 | `mkdir specs/hand-made/` + hand-written `spec.md` | **valid**; appears in `existingOutputPaths` immediately | works — the specs artifact is a glob |
| E10 | `## ADDED` → `## MODIFIED` for a capability with no main spec | **valid** | **archive aborts**: *"target spec does not exist; only ADDED requirements are allowed for new specs."* `Aborted. No files were changed.` |
| E11 | MODIFIED requirement title with a one-letter typo (`unambigous`) | **valid** | **archive aborts**: *"MODIFIED failed for header … — not found"* |
| E12 | MODIFIED block rewritten from memory, omitting 3 scenarios the main spec has | **valid** | **archive aborts**: *"current spec contains scenario(s) not present in the modified block: … Refresh the change spec before archiving to avoid dropping scenarios."* |
| E13 | Hand-written `## Purpose` rewrite **and** a `## Notes for reviewers` section in a delta for an **existing** capability | **valid** | **archive succeeds** — and both sections are **silently discarded**. A blockquote written *inside* the requirement body survived |

Mapped onto the template's concerns:

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Delta spec grammar | Yes — requirement/scenario structure, at `validate` [E2, E4, E5, E7] | — |
| RFC 2119 wording | No — warning only [E3] | Yes |
| MODIFIED matches an existing requirement | Yes, but **only at `archive`** [E10, E11] | — |
| MODIFIED preserves existing scenarios | Yes, but **only at `archive`** [E12] | — |
| Non-requirement prose in a delta | **No — dropped without a word** [E13] | — |
| `proposal.md` / `design.md` content | No grammar at all; two soft warnings seen (Why >1000 chars, >10 deltas) | Yes, via `instructions … rules` |
| Coherence between artifacts | **Nothing checks it** | `/opsx:update` step 4 [S4] |
| `tasks.md` checkbox honesty | Read, not verified — archive reported `0/63` and warned, then continued under `--yes` | Yes |

## 5. Strengths (of editing by hand)

- **Zero derived state to corrupt.** Verified, not assumed: `status` is file-existence only [S3, E8], and new capability directories are discovered by glob on the next call [E9]. There is no index, no lockfile, no cache to invalidate.
- **Upstream-sanctioned, with a doc page of its own.** *"Open the Markdown artifacts in your editor and change them directly — nothing else is required"* [S2].
- **The propose workflow is built to expect it.** It re-reads dependencies from disk specifically because *"the user may have edited them"* [S3].
- **Free.** No tokens, no round trip, no confirmation loop. Against the measured cost in §9 this is the whole argument for small edits.
- **The mechanical failure modes are loud.** Of 13 mutations, 5 were blocked at validate, 3 blocked at archive with the change left untouched (`Aborted. No files were changed.`), 4 were harmless, 1 was silent. The one silent case is a single, learnable rule (§6).
- **Git handles the rest.** Plain markdown in a git repo; every hand edit is diffable and revertible, which is more review surface than an agent edit that arrives mid-transcript.

## 6. Weaknesses / friction

- **The one silent loss: non-requirement prose in a delta spec [E13].** A hand-written `## Purpose` on a delta for an *existing* capability is ignored (both `archive` and `/opsx:sync` say the main spec's Purpose is authoritative [S1, S5]); a `## Notes for reviewers` section simply never reaches the main spec. Only content inside a `### Requirement:` block survives the merge. Nothing warns. **If a note matters, it belongs in `design.md` or `proposal.md`, which are never merged and never parsed.**
- **`validate --strict` is not the gate you think it is.** E10–E12 all pass strict validation and die at archive. A change can look green for weeks and be unarchivable. Mitigation: `archive` performs its checks *before* writing and aborts atomically, so the failure is safe — but it is late.
- **Hand-assembled `MODIFIED` blocks are the single most dangerous edit.** The rule is copy the *entire* requirement block from `openspec/specs/<cap>/spec.md`, then edit it — the tool's own instruction says so, and calls the alternative out: *"Common pitfall: Using MODIFIED with partial content loses detail at archive time"* [S1]. In 1.8.0 archive does catch dropped scenarios [E12], so the pitfall is now a hard stop rather than data loss — but a *reworded* requirement body still silently replaces the old one.
- **No coherence check exists anywhere.** Edit a requirement in `specs/` and nothing notices that `tasks.md` still implements the old behaviour or that `design.md` still justifies it. This is where hand-editing genuinely loses to `/opsx:update`, whose step 4 reconciles in **both** directions [S4].
- **The drift detector the docs recommend is not installed.** `/opsx:verify` is expanded-profile; this workspace is `core` [E15, S2].
- **Documentation is wrong in at least one place.** The 3-hashtag scenario pitfall does not fail silently in 1.8.0 [E7 vs S1] — a reminder that the artifact instructions are guidance written for a model, not a specification of the parser.
- **`/opsx:update` has its own friction**, in the opposite direction: it confirms every artifact one at a time, refuses to create files that do not yet exist, and refuses to touch code [S4]. For a typo that is a lot of ceremony for a character.

## 7. Fit signals

**Hand-edit when:**
- Wording, typos, formatting, tightening a sentence.
- Editing one requirement's body or adding a scenario, where nothing else in the change depends on it.
- Checking off a task box, reordering or splitting tasks.
- Deleting something that is plainly wrong.
- You want the change to appear in `git diff` as *your* edit, attributable and reviewable.
- **Always followed by:** `openspec validate --all --strict`.

**Route through `/opsx:update` when:**
- A requirement is added or removed — the task list and design almost certainly move with it.
- A design decision is reversed, or scope changes.
- You are assembling a `## MODIFIED Requirements` block (it will fetch the current main spec and copy the whole block).
- You have already hand-edited several files and want the coherence sweep — `/opsx:update` with no specific instruction is exactly that [S4].
- You are unsure which other artifacts a change touches. That uncertainty *is* the signal.

**Never hand-edit:**
- `.openspec.yaml` markers you did not put there (`skip_specs`, `retire_capabilities`, a non-default `schema`).
- Main specs under `openspec/specs/` while a change targeting that capability is in flight — you will be editing the merge target of a pending delta. **One documented exception:** changing an existing capability's `## Purpose`, including a leftover `TBD` placeholder, *must* be done by editing `openspec/specs/<cap>/spec.md` directly; the tool instructs it explicitly [S1].

## 8. Notes for this project

- **The scope change makes the silent-loss case worse.** The proxy is now a row-level security control with a fail-closed posture (`CLAUDE.md`), where the specs *are* the security argument. E13 means a hand-written caveat in a delta spec — "this requirement does not cover views", say — evaporates at archive unless it lives inside a requirement body or in `design.md`. In a resource-protection project that costs a footnote; here it can cost the record of a known bypass vector.
- **The five bypass vectors in `CLAUDE.md` are prose, not requirements.** They belong in `design.md` (never merged, never parsed, free-form) or in an ADR. Do not put them in a delta spec expecting them to reach the canonical spec.
- **`openspec/config.yaml` is a third editing surface with different rules.** It is hand-edited only, and it fails *silently* on YAML errors and unknown `rules` keys — `validate --strict` will not catch it (see `openspec-config-fails-silently` in the project memory). That is a genuinely riskier file to hand-edit than any artifact.
- **For the SDD learning goal specifically:** hand-editing is not cheating and does not skip ceremony. The ceremony is the artifact being the source of truth; both routes produce identical markdown. What *would* skip the ceremony is editing code to match a decision and never folding it back — which is why `docs/editing-changes.md` insists *"before you archive, make the specs honest about what the code does"* [S2].

## 9. Cost & lock-in

- **Money:** none either way.
- **Token cost — measured on this change.** The four artifacts total **47,087 bytes** (`proposal` 6.6 KB, `design` 15.1 KB, `tasks` 8.1 KB, three delta specs 17.3 KB) ≈ 12k tokens. Each `openspec instructions <artifact> --json` payload is **10.5–14.2 KB** (~3k tokens) [E16]. A `/opsx:update` invocation reads `status --json`, the artifacts it touches plus every artifact it checks for coherence, and `instructions` for anything it substantially rewrites — **realistically 15k+ tokens per revision**, before the confirmation round trips. A hand edit costs zero. For a one-word fix that ratio is the whole answer.
- **Lock-in:** none. Plain markdown in git. Both routes leave byte-identical output.
- **Exit path:** delete `openspec/` and keep the markdown. Nothing else references it.

## 10. Evidence & sources

**Experiments** — all run on 2026-08-09 against openspec v1.8.0 in a throwaway root at `<scratchpad>/edittest`, seeded with a verbatim copy of `add-row-filter-proxy-mvp`'s artifacts. Each mutation was applied to a restored-from-backup copy, so results are independent. Baseline: `Change 'test-edit' is valid`, `deltaCount = 15` (`connection-routing` 4, `policy-config` 5, `row-filter-rewrite` 6).

| # | Experiment | Result |
|---|---|---|
| E0 | Enumerate every subcommand's `--help` | No content-editing command exists |
| E1–E13 | The mutation table in §4 | See §4 |
| E14 | `openspec new change scaffoldtest`; `find` the directory | Only `.openspec.yaml` created |
| E15 | `openspec config list` | `profile: core`; workflows `propose, explore, apply, update, sync, archive` |
| E16 | `wc -c` on artifacts; byte size of each `instructions --json` | 47,087 B artifacts; 10,458–14,195 B per instruction payload |
| E-arch1 | Clean `archive test-edit --yes` | `+ 15, ~ 0, - 0`; canonical main specs created; change moved to `changes/archive/2026-08-09-test-edit/` |

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| S1 | `openspec instructions specs --change add-row-filter-proxy-mvp --json` (`.instruction`) | tool output, this machine | 2026-08-09 | The MODIFIED workflow, the Purpose rule, the "edit `openspec/specs/…` directly" instruction, and the (inaccurate) 3-hashtag warning |
| S2 | https://github.com/Fission-AI/OpenSpec/blob/main/docs/editing-changes.md | upstream docs | fetched 2026-08-09 | *"nothing else is required"*; the tweak-vs-rethink heuristic; `/opsx:verify` for drift; archive-time honesty |
| S3 | `.claude/skills/openspec-propose/SKILL.md` (lines 99, 108, 146) | installed skill, v1.8.0 | 2026-08-09 | Re-read from disk "the user may have edited them"; "`status` is file-existence only" |
| S4 | `.claude/skills/openspec-update-change/SKILL.md` | installed skill, v1.8.0 | 2026-08-09 | Coherence reconciliation in both directions; per-artifact confirmation; never creates artifacts; never edits code |
| S5 | `.claude/skills/openspec-sync-specs/SKILL.md` (lines 15, 141–143, 229) | installed skill, v1.8.0 | 2026-08-09 | Sync is agent-driven; delta Purpose ignored for existing capabilities; MODIFIED must carry every surviving scenario |
| S6 | https://github.com/Fission-AI/OpenSpec/blob/main/README.md | upstream docs | fetched 2026-08-09 | *"Work fluidly — update any artifact anytime, no rigid phase gates"*; documented command surface |

## 11. Confidence & gaps

- **Confidence: high** for everything in §4 and §5 — the CLI was run, the mutations applied, the outputs read. Nothing in this survey is inferred from documentation alone, and where documentation and behaviour disagree (E7) the behaviour is reported and the discrepancy flagged.
- **Unverified claims:**
  - `/opsx:update`'s token cost is estimated from measured artifact and payload sizes, not from an instrumented run. The 15k figure is an order of magnitude, not a measurement.
  - Behaviour of `REMOVED` and `RENAMED` delta operations under hand-editing was not tested; only `ADDED` and `MODIFIED` were.
  - Expanded-profile commands (`/opsx:verify`, `/opsx:continue`) were not exercised — they are not installed.
  - Whether `openspec update` (the CLI command) ever rewrites artifacts was not tested; its documented job is refreshing agent instruction files.
- **Open questions:**
  - Does a hand-edited `design.md` ever get read back? `archive` never parses it; only `/opsx:apply` and `/opsx:update` do, and both by model. So a design edit is invisible to tooling by construction — probably fine, worth confirming before relying on it.
  - Is there a case where `/opsx:sync` (agent merge) and `archive` (programmatic merge) disagree on the same delta? Both were run, never against the same input.

## 12. Recommendation — the house rule

**Edit by hand. Validate after. Escalate to `/opsx:update` when the edit changes meaning rather than wording.**

Concretely, for this project:

1. **Default to the editor.** Typos, phrasing, one requirement's body, task checkboxes, reordering. The tool is designed for this and the token cost of the alternative is real.
2. **After any hand edit, run** `openspec validate --all --strict` from the workspace. It catches the structural half.
3. **Before archiving, treat archive itself as the gate.** It checks three things validate does not, and aborts without writing [E10–E12]. Do not read a green `validate` as "ready to archive."
4. **Escalate to `/opsx:update` when an edit adds or removes a requirement, reverses a design decision, or changes scope** — because those propagate into `tasks.md` and `design.md`, and nothing else in the system will notice that they have not.
5. **Assemble `MODIFIED` blocks by copying, never by writing.** Copy the whole requirement block out of `openspec/specs/<cap>/spec.md`, paste, then edit. This is the single edit most likely to go wrong.
6. **Keep non-requirement prose out of delta specs.** It is silently discarded at merge [E13]. Caveats, hazards and rationale go in `design.md`, `proposal.md`, or an ADR.
7. **Leave `.openspec.yaml` and in-flight main specs alone** — with the one documented exception of an existing capability's `## Purpose`, which must be hand-edited in the main spec [S1].
