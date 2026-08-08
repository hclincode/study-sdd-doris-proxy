# Survey: OpenSpec

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** research agent (Claude Opus 5)
> **Scope:** OpenSpec (github.com/Fission-AI/OpenSpec) evaluated as a spec-driven development workflow for a greenfield async-Rust Apache Doris proxy speaking the MySQL wire protocol. Covers the v1.8.0 `/opsx:*` command surface, enforcement semantics, Claude Code wiring, maintenance health, and token cost. Excludes the Stores beta beyond a description.
>
> **Scope update 2026-08-08.** The project has since narrowed to an L7 proxy whose purpose is
> **SQL rewriting for tenant isolation and schema remapping**, with 1:1 backend connections and
> passthrough auth (`discussions/milestone-1/02-proxy-scope.md`). Three consequences for this survey:
> **(a)** the delta model's case strengthens — the rewrite-rule catalogue and statement allowlist
> grow forever, which is precisely its home turf;
> **(b)** §6's silent-overwrite bug (#1246) matters less, since a solo developer on one spec domain
> at a time is the pattern that avoids it;
> **(c)** §4's finding that OpenSpec enforces **nothing** semantic is now more serious, because
> isolation rules are security-relevant — the tool cannot tell you a rule was dropped. That guarantee
> must come from `cargo-mutants` and negative tests, not from the spec layer.
> A hands-on trial has since been run against the OpenSpec format: see `archives/milestone-1/trial/`.

## 1. Snapshot

| Field | Value |
|---|---|
| Name | OpenSpec (`@fission-ai/openspec`) |
| Vendor / owner | Fission AI — GitHub org `Fission-AI`; dominant author `TabishB` (516 commits), day-to-day maintainer `clay-good` (82 commits) |
| License | MIT |
| First release | Repo created 2025-08-05; first npm publish 2025-09-06; v1.0.0 shipped the `/opsx:*` rewrite |
| Latest version (as of survey date) | **v1.8.0**, published 2026-08-05 (npm `latest` = 1.8.0) |
| Popularity signal | **64,224 GitHub stars**, 4,427 forks, 271 watchers (GitHub API, 2026-08-08). npm: **351,739 downloads/week**, 1,394,401/month |
| Maintenance signal | 128 commits in the last 30 days; last commit 2026-08-07. Releases roughly monthly (v1.2 Feb → v1.8 Aug 2026). 93 open issues / 474 closed; 89 open PRs / 536 merged. 108 new issues in the last 60 days — high inbound, keeping up |
| Primary interface | Node CLI (`openspec`) + agent-side **Skills / slash commands**. **No MCP server** |
| Agent compatibility | 45 tools. Claude Code is first-class (skills + commands). Also Cursor, Codex, Copilot, Amazon Q, Devin, Kiro, Rovo Dev, MiniMax, and a vendor-neutral `.agents/skills/` target |
| Language/stack neutrality | **Format is stack-neutral; documentation is not.** Every example in `writing-specs.md` and `multi-language.md` is web/CRUD (session timeout, login endpoint, file upload, theme toggle; "Tech stack: TypeScript, React, Node.js"). Zero mention of Rust, Go, systems, or protocol work in the docs |
| Greenfield vs brownfield bias | **Explicitly brownfield-first.** "You do not document your whole codebase to start. You write specs only for what you're about to change." Works greenfield, but its central innovation (delta specs) has less to bite on when there is no base spec |

## 2. One-paragraph thesis

OpenSpec's bet is that the bottleneck in AI-assisted coding is not generation speed but **intent transfer**, and that the cheapest fix is a short written agreement produced *before* code, kept in the repo as plain Markdown. Its differentiator against heavier frameworks (Spec Kit, BMAD) is the **delta spec**: a change does not restate a whole specification, it declares `## ADDED / MODIFIED / REMOVED Requirements` against an existing baseline, and those deltas are merged into the canonical `openspec/specs/` only when the change is archived. This gives it two claimed properties — it works on mature codebases without a documentation backfill, and it stays cheap in tokens because each change is a diff, not a document. The philosophy is deliberately anti-waterfall: "fluid not rigid," artifacts form a dependency graph rather than phase gates, so the tool tells you what *becomes possible* next, not what you are *forced* to do next. The workflow lives in your AI assistant as slash commands; the CLI underneath is a structural validator, merger, and JSON-emitting oracle for the agent. Crucially, the CLI checks the **shape** of your specs, never their **truth** or their relationship to your code.

## 3. Workflow / artifact model

The developer loop runs almost entirely inside the AI assistant's chat. Two terminal commands set it up; after that you type slash commands.

```sh
npm install -g @fission-ai/openspec@latest   # requires Node >= 20.19.0
cd your-project && openspec init             # interactive: pick your tools/profile
```

The default loop:

```
/opsx:explore  →  /opsx:propose  →  /opsx:apply  →  /opsx:sync  →  /opsx:archive
  (optional)       write plan        write code      reconcile      merge deltas
```

**Artifact tree** (as created by `openspec init` and populated per change; layout confirmed against the OpenSpec repo's own `openspec/` directory):

```
openspec/
├── config.yaml                        # project context + per-artifact rules, injected into every planning prompt
├── specs/                             # CANONICAL truth, organized by domain
│   └── <domain>/
│       └── spec.md                    # ### Requirement: … + #### Scenario: … (GIVEN/WHEN/THEN)
└── changes/
    ├── <change-name>/                 # one in-flight change
    │   ├── .openspec.yaml             # change metadata (schema, skip_specs marker)
    │   ├── proposal.md                # why + scope
    │   ├── design.md                  # technical approach (optional)
    │   ├── tasks.md                   # - [ ] implementation checklist
    │   └── specs/                     # DELTA specs
    │       └── <domain>/
    │           └── spec.md            # ## ADDED / MODIFIED / REMOVED / RENAMED Requirements
    └── archive/
        └── <YYYY-MM-DD>-<change-name>/  # full change folder, preserved verbatim

.claude/
├── skills/openspec-<verb>/SKILL.md    # 13 skills; the actual behavioral instructions
└── commands/opsx/<id>.md              # thin slash-command entry points
```

Requirement format (RFC 2119 keywords, Given/When/Then scenarios):

```markdown
### Requirement: Theme selection
The app SHALL let users switch between light and dark themes,
defaulting to system preference.

#### Scenario: User toggles dark mode
- **WHEN** the user clicks the theme toggle
- **THEN** the app switches to dark mode and persists the choice
```

**Command surface.**

*Agent-facing slash commands* (Claude Code syntax; `/opsx-…` in Cursor/Copilot, `$openspec-…` in Codex):
`/opsx:explore`, `/opsx:propose <idea>`, `/opsx:apply`, `/opsx:sync`, `/opsx:archive` — core profile.
Expanded profile adds `/opsx:new`, `/opsx:continue`, `/opsx:update`, `/opsx:ff` (fast-forward), `/opsx:verify`, `/opsx:bulk-archive`, `/opsx:onboard`.

*CLI, human-facing:* `openspec init [--tools --profile --force]`, `openspec update`, `openspec view` (TUI dashboard), `openspec config edit|profile`, `openspec feedback`, `openspec completion install`, `openspec workset open`.

*CLI, agent-facing (all support `--json`):* `openspec list`, `openspec show [--deltas-only --requirements --no-scenarios]`, `openspec validate [--all --strict --concurrency]`, `openspec new change <name>`, `openspec status`, `openspec instructions <artifact>`, `openspec templates`, `openspec schemas`, `openspec doctor`, `openspec context`, `openspec archive --yes`.

*Schema/customization:* `openspec schema init|fork|validate|which`.
*Stores beta (cross-repo):* `openspec store setup|register|unregister|remove|list|doctor`.

**Artifact lifecycle.** `/opsx:propose` scaffolds a change folder. Work proceeds against `tasks.md`. `openspec validate` checks structure. `openspec archive` then (1) merges each delta section into the canonical spec — ADDED appended, MODIFIED replacing the matching requirement block, REMOVED deleted — (2) moves the change folder to `changes/archive/<date>-<name>/` preserving all artifacts, (3) leaves `openspec/specs/` as the new source of truth. Archive validates first unless `--no-validate`, and **requires `--yes` when stdin is closed**: "an AI agent, a CI job, or any run with stdin closed cannot answer step 2, so archive stops before touching anything, exits 1." Version control story is simply *it's all files in your repo* — specs, in-flight changes, and archive are ordinary git-tracked Markdown.

## 4. What it enforces vs. what it suggests

Determined by reading `src/core/validation/validator.ts` directly, not from marketing copy.

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | **Partial.** `openspec validate` ERRORs if a change has delta sections but zero parsed requirements, or no delta sections at all; a change with zero spec deltas fails validation unless it declares `skip_specs: true`. `openspec archive` validates before merging and exits 1 on failure | Nothing stops the agent from writing code without ever running `/opsx:propose`. There is no hook, no pre-commit gate, no CI integration shipped. The FAQ never claims otherwise — specs are positioned as an *agreement* mechanism, not a constraint |
| Spec ↔ code traceability | **None.** The validator never reads your source tree. No requirement→file/test linkage exists in the data model | `/opsx:verify` asks the agent to compare shipped code against the delta spec and report mismatches. Purely an LLM judgment call |
| Test/acceptance criteria | **Structural only.** ERROR if an `ADDED` or `MODIFIED` requirement has zero `#### Scenario:` blocks. ERROR on duplicate requirement names within a section, and on a requirement appearing in two of ADDED/MODIFIED/REMOVED. WARNING (ERROR under `--strict`) if a requirement lacks a SHALL/MUST keyword. v1.8 added a warning for ambiguous `tasks.md` numbering (#1523) | Whether the scenarios are *correct*, *sufficient*, or *actually tested* is entirely the agent's and reviewer's problem. No test runner integration. `config.yaml` per-artifact `rules:` are — in the docs' own words — "injected into the AI prompt during generation, helping enforce your team's standards **without being enforceable checks**" |
| Review gate | **None.** No approval state, no sign-off field, no branch protection | The artifact dependency graph means `design` and `tasks` are *available* after `proposal`+`specs` exist, but nothing forbids creating them all at once — this is the explicit "fluid not rigid" design choice. Community schemas (e.g. `anvil`) add a `review` artifact whose `VERDICT:` line the agent is *told* to treat as a gate |
| Drift detection | **Weak and late.** `openspec validate` does **not** check that a `MODIFIED`/`REMOVED` header actually exists in the base spec — that mismatch surfaces only at `archive` time, often weeks later (open issue #1112). Cross-change conflicts are not detected at all (#1246) | `/opsx:sync` and `/opsx:verify` ask the agent to reconcile. FAQ guidance is manual: "Bring them back in sync before you archive" / "If the code is now correct, update the delta spec to match what you shipped" |

**The one-line summary:** the CLI is a *Markdown structure linter and a merge engine*. Everything about whether the spec is right, whether it matches the code, and whether the agent honored it is prompt text and human review.

## 5. Strengths

- **The delta model genuinely solves the brownfield onboarding problem.** No 40-page baseline document before your first change. Specs accrete around the work you actually do. For a greenfield project this matters less on day one but a lot by month six.
- **Artifacts are small.** Measured against OpenSpec's own repository (2026-08-08): a change folder ranges from 1.4 KB / 2 files (`schema-alias-support`) to 60 KB / 8 files (`simplify-skill-installation`), with a median around 14 KB. That is roughly 350–15,000 tokens of artifact per change, typically ~3,500–8,000. Compare against reported Spec Kit output of 7+ files per feature with plan documents running hundreds of lines.
- **Real CLI, not just prompt files.** The merge engine, structural validator, JSON contract (`--json` emits exactly one 2-space-pretty-printed document per invocation, documented exit codes, stable diagnostic envelope) means the agent gets deterministic answers to "what exists / is this well-formed" instead of re-deriving it from the filesystem every turn.
- **Excellent Claude Code integration.** `openspec init` writes `.claude/skills/openspec-*/SKILL.md` plus `.claude/commands/opsx/<id>.md`. Skills are the model-invoked path, commands the explicit path — you get both. The maintainers actively track Claude Code conventions (recent commits move Codex to the canonical agents directory, add `.agents/skills/` as vendor-neutral).
- **Customizable workflow schemas.** `openspec schema fork spec-driven my-workflow` lets you add artifacts and `requires:` dependencies. The community `anvil` schema demonstrates a TDD-gated flow: `proposal → specs → design → review → test-plan → tasks → apply → verify`. This is the hook for adding Rust-specific discipline.
- **Near-zero lock-in.** Everything is plain Markdown plus one `config.yaml`. Uninstall the CLI and the artifacts remain readable and useful.
- **Extremely active.** 128 commits in 30 days, monthly releases, external contributors landing fixes (the last 15 commits include 8 distinct non-maintainer authors).

## 6. Weaknesses / friction

- **Validation is syntactic, and the gap bites in production.** Open issue **#1112** (filed 2026-05-21, still open with zero maintainer comments as of survey date) documents the sharpest edge: a delta that `MODIFIED`s a requirement header not present in the base spec passes `openspec validate` — *even with `--strict`* — and fails only at `archive`. The reporter hit it twice in six days: "The mismatch sat undetected from 2026-05-15 to 2026-05-21… By then the implementing PR has typically shipped, days or weeks earlier. The session that authored the spec is gone. Recovering requires forensics."
- **Silent data loss on concurrent changes.** Issue **#1246**: two changes that `MODIFIED` the same requirement, archived sequentially, cause the second to **silently overwrite** the first's scenarios. Root cause named precisely by the reporter — `buildUpdatedSpec` in `src/core/specs-apply.ts` applies MODIFIED as a whole-block replace keyed by requirement name. "No warning, no diff, no conflict marker; archive reports success." This is exactly the failure mode a two-developer or agent-parallel workflow will hit.
- **`tasks.md` checkboxes are decorative.** Open issue **#805** (since 2026-03-05): agents complete work but never flip `- [ ]` to `- [x]`, so `openspec list --json` reports `"completedTasks": 0, "totalTasks": 35` after a full apply. The prompt says "mark task complete" but supplies no mechanism. Progress tracking is unreliable.
- **Spec corpus grows without bound.** OpenSpec's own repo, one year in, carries 36 canonical spec files totaling 240 KB — roughly **60,000 tokens** of accumulated specification. The agent does not load all of it per turn, but discovery, `MODIFIED` targeting, and human review all get harder as this grows, and nothing in the tool prunes or summarizes.
- **Significant churn in a young tool.** v1.0 renamed every slash command (`/openspec:proposal` → `/opsx:propose`), replaced `.claude/commands/openspec/` with `.claude/skills/openspec-*/`, and moved project context from freeform `openspec/project.md` to structured `openspec/config.yaml` (manual migration required). Since then: v1.3–1.4 introduced "workspaces and initiatives," which v1.5 then *replaced* with "stores." Eight minor releases in seven months, with a beta feature already deprecated once.
- **Brownfield hallucination risk.** The delta model assumes the agent understands the unchanged code around the delta. The most detailed independent hands-on comparison flags exactly this as OpenSpec's primary weakness: "On projects with poor inline documentation, the agent may hallucinate surrounding context." It trades artifact compactness for a dependency on code clarity.
- **Ceremony cost on small work, acknowledged by the docs.** "For a one-character typo fix, the ceremony probably isn't worth it, and that's fine." In practice this means a per-change judgment call about whether to use the workflow at all — which erodes the discipline the tool exists to provide.
- **Team workflow is thin.** The FAQ covers solo editing ("Just edit the file") but offers no guidance on multi-developer review, PR integration, or cross-branch spec conflict resolution — which, given #1246, is the case most likely to lose data.
- **Bus factor.** Single-vendor project. `TabishB` holds 516 of the commits but recent activity is infrastructure/chore (last commit 2026-07-27); `clay-good` (82 commits) is doing most current feature work. Community PRs land, but direction is controlled by a very small group.
- **Telemetry on by default.** `telemetry.enabled` defaults to on when unset (auto-disabled in CI); opt out via `openspec config set telemetry.enabled false`, `OPENSPEC_TELEMETRY=0`, or `DO_NOT_TRACK=1`. Issue #1512 (fixed 2026-08-05) reported the documented key was actually *rejected at runtime* — the opt-out was broken while documented as working.
- **Node.js dependency.** Requires Node ≥ 20.19.0 and a global npm install on a project that otherwise needs only a Rust toolchain. Adds a runtime, a lockfile-less global install, and a version-skew surface to CI.

## 7. Fit signals

**Strong fit when:**
- The work decomposes into named, externally-observable behaviors that can be enumerated — a wire protocol is unusually good at this (`COM_QUERY` handling, handshake/auth negotiation, capability flag advertisement, error-packet mapping, prepared-statement lifecycle).
- You are iterating on an evolving surface where "what changed" matters more than "what is" — the delta model's home turf.
- One developer (or one agent lane) at a time touches a given spec domain, avoiding the #1246 overwrite hazard.
- You want artifacts that survive the tool. Plain Markdown in git, reviewable in a normal PR.
- You are already on Claude Code and want a low-friction skills-based workflow with no MCP server to run.

**Poor fit when:**
- You need the tooling to *enforce* anything. If the value you want is "the agent cannot ship code that contradicts the spec," OpenSpec does not provide it and does not claim to.
- The requirements that matter are non-functional — latency budgets, memory ceilings, connection-pool behavior under saturation, backpressure semantics. The spec format has no vocabulary for these and `writing-specs.md` offers zero guidance.
- Multiple parallel changes routinely touch the same requirements (see #1246).
- The change is small. Below some threshold the proposal/spec/design/tasks/archive cycle costs more than it returns, and the docs agree.
- You need auditable spec↔code traceability for compliance. There is no linkage model.

## 8. Rust / systems-software notes

**It does not assume web/CRUD structurally, but its entire documented worldview is web/CRUD.** The artifact format is language-agnostic Markdown — nothing in the validator, the merge engine, or `config.yaml` knows or cares what language you write. But every worked example in `docs/writing-specs.md` is session timeout, login endpoint, file upload, theme toggle; `docs/multi-language.md` (which is about *human* languages, not programming languages) uses "TypeScript, React, Node.js" and "PostgreSQL with Prisma ORM" as its stack examples. There is **no mention of Rust, Go, systems programming, embedded, real-time constraints, or protocol work anywhere in the documentation.**

**Concurrency, performance budgets, invariants: unaddressed.** `docs/writing-specs.md` gives one rule — "Keep the *how* — the queue, the library, the table schema — in `design.md` or the code" and "No implementation details are baked into the requirements" — and offers no guidance on throughput targets, latency percentiles, resource ceilings, or system invariants. For a proxy where "handles 10k concurrent connections without head-of-line blocking" is a first-class requirement, you would be inventing the convention yourself. The Given/When/Then scenario form can express a *functional* protocol behavior cleanly; it expresses "no task holds the connection lock across an await point" badly or not at all.

**Property and mutation testing: no integration, but a viable extension point.** OpenSpec has no test-runner awareness of any kind — it will not run `cargo test`, will not know `cargo-mutants` exists, and cannot tell whether a scenario has a corresponding test. The realistic path is `openspec schema fork` to add a `test-plan` artifact with `requires: [specs]`, mirroring the community `anvil` schema, plus `config.yaml` rules such as "every requirement with more than one input class must have a `proptest`." Understand clearly that these are **prompt injections, not checks** — the docs say so explicitly. Enforcement would have to come from your own CI (e.g. a job that fails if `openspec validate --all --strict` errors, plus your normal `cargo clippy`/`cargo mutants` gates), wired by you.

**Rust adoption evidence: essentially none, and what exists is inverted.** A GitHub code search for OpenSpec's delta marker (`"## ADDED Requirements"`) across public repos returned no significant Rust codebase using OpenSpec for its own development. What Rust *does* show up is tooling *about* OpenSpec: `oonid/OpenSpec-rs` (a Rust reimplementation of the CLI, 2 stars), `uwzis/OpenSpec-Zed` (Zed extension, 15 stars), `neosam/openspec-tui` (3 stars), and Rust projects that *parse* OpenSpec artifacts (`tumf/conflux` has `src/spec_delta.rs`; `m0nkmaster/afk` has `src/sources/openspec.rs`). This tells us Rust developers are interested enough to build integrations, and tells us nothing about whether the methodology holds up on a systems codebase. **You would be an early adopter in this niche.**

## 9. Cost & lock-in

- **Money:** Free. MIT licensed, no paid tier, no account, no hosted service. The only cost is your existing agent subscription/API spend.
- **Token cost:** Measured from OpenSpec's own repo (bytes ÷ 4 as a rough token proxy) — **per change: ~350 tokens (2 files) to ~15,000 tokens (8 files), typically ~3,500–8,000**. Per invocation the agent loads one `SKILL.md`: 3.7 KB for `openspec-new-change` up to 17 KB for `openspec-bulk-archive-change`, i.e. **~1,000–4,200 tokens** (`openspec-propose` is 11 KB ≈ 2,800 tokens; `openspec-explore` 16 KB ≈ 4,000). On top of that, `config.yaml` project context is injected into *every* planning request. The compounding cost is the canonical spec corpus — 60,000 tokens after one year on their own repo — which the agent must search and selectively read. Budget roughly **5,000–15,000 extra tokens per non-trivial change** early on, rising as `openspec/specs/` grows.
- **Lock-in:** Very low. Artifacts are plain Markdown (`proposal.md`, `design.md`, `tasks.md`, `spec.md`) plus one YAML config. Nothing is in a database, no proprietary format, no hosted state. The `## ADDED Requirements` / `### Requirement:` / `#### Scenario:` conventions are readable and hand-editable without the tool.
- **Exit path:** `npm uninstall -g @fission-ai/openspec` and delete `.claude/skills/openspec-*` plus `.claude/commands/opsx/`. The `openspec/` directory stays as ordinary project documentation. Migrating to another SDD tool means reformatting Markdown, which is mechanical. The real cost of leaving is not technical — it is that the *delta* history in `changes/archive/` is only meaningful under OpenSpec's merge semantics.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://github.com/Fission-AI/OpenSpec | repo/README | 2026-08-08 | Philosophy, `/opsx:*` commands, tool support, Stores beta, telemetry, MIT license |
| 2 | https://api.github.com/repos/Fission-AI/OpenSpec | API | 2026-08-08 | **Stars = 64,224** (corrects the "52k in June 2026" claim — see §11), forks 4,427, watchers 271, created 2025-08-05, pushed 2026-08-07 |
| 3 | https://api.github.com/repos/Fission-AI/OpenSpec/releases | API | 2026-08-08 | v1.8.0 (2026-08-05), v1.7.0 (2026-07-29), v1.6.0 (2026-07-10), v1.5.0 (2026-06-28), v1.4.x (June), v1.3.x (April), v1.2.0 (2026-02-23), v1.1.x (2026-01-30) |
| 4 | https://api.github.com/repos/Fission-AI/OpenSpec/contributors | API | 2026-08-08 | Bus factor: TabishB 516, clay-good 82, then bots and long-tail (≤7 each) |
| 5 | https://registry.npmjs.org/@fission-ai/openspec + api.npmjs.org/downloads | API | 2026-08-08 | latest 1.8.0, 43 versions, Node ≥20.19.0, 351,739 downloads/week |
| 6 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/cli.md | docs | 2026-08-08 | Full command surface, flags, human-vs-agent split, exit codes, archive `--yes` requirement |
| 7 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/concepts.md | docs | 2026-08-08 | Delta model, ADDED/MODIFIED/REMOVED semantics, RFC 2119, archive merge steps |
| 8 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/getting-started.md | docs | 2026-08-08 | `openspec/` directory tree, default loop incl. `/opsx:sync` |
| 9 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/src/core/validation/validator.ts | source | 2026-08-08 | **Primary evidence for §4.** Enumerates every ERROR/WARNING the validator emits; confirms enforcement is purely structural |
| 10 | https://github.com/Fission-AI/OpenSpec/issues/1112 | issue (open) | filed 2026-05-21 | MODIFIED against non-existent base header passes `validate --strict`, fails at archive weeks later; 0 comments |
| 11 | https://github.com/Fission-AI/OpenSpec/issues/1246 | issue (closed) | filed 2026-06-24 | Sequential archive of two changes MODIFYing the same requirement silently drops scenarios; root cause in `src/core/specs-apply.ts` `buildUpdatedSpec` |
| 12 | https://github.com/Fission-AI/OpenSpec/issues/805 | issue (open) | filed 2026-03-05 | `tasks.md` checkboxes never updated; `completedTasks: 0` after full apply |
| 13 | https://github.com/Fission-AI/OpenSpec/issues/1512 | issue (closed) | 2026-08-05 | Documented `telemetry.enabled` opt-out key rejected at runtime |
| 14 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/customization.md | docs | 2026-08-08 | Schema forking, `anvil` TDD-gated schema, and the key admission that `config.yaml` rules work "without being enforceable checks" |
| 15 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/existing-projects.md | docs | 2026-08-08 | Brownfield stance: "Resist the urge to back-fill everything" |
| 16 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/writing-specs.md | docs | 2026-08-08 | Confirms zero coverage of NFRs, performance, concurrency, protocol; all examples web/CRUD |
| 17 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/faq.md | docs | 2026-08-08 | "For a one-character typo fix, the ceremony probably isn't worth it"; drift handled by human reconciliation |
| 18 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/migration-guide.md | docs | 2026-08-08 | Legacy → OPSX breaking changes: command rename, `project.md` → `config.yaml`, commands → skills |
| 19 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/supported-tools.md | docs | 2026-08-08 | Claude Code paths: `.claude/skills/openspec-*/SKILL.md`, `.claude/commands/opsx/<id>.md`; 45 tools; no MCP |
| 20 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/agent-contract.md | docs | 2026-08-08 | JSON output contract, exit codes (0/1/130), diagnostic envelope |
| 21 | https://ypyl.github.io/programming/2026/06/03/openspec-vs-spec-kit-sdd.html | hands-on report | 2026-06-03 | Independent comparison; brownfield hallucination as OpenSpec's main weakness |
| 22 | GitHub tree API + `gh search code` over `Fission-AI/OpenSpec` | measurement | 2026-08-08 | All §9 token figures: per-change 1.4–60 KB, 13 skills totaling 125 KB, 36 canonical specs totaling 240 KB |
| 23 | https://medium.com/@reenbit/bmad-vs-spec-kit-vs-openspec-choosing-your-spec-driven-ai-framework-in-2026-a6996b3ebb8d | blog | 2026 | Source of the 12min/90min/5.5hr benchmark — treat as anecdote (see §11) |

## 11. Confidence & gaps

- **Confidence: high** on the command surface, artifact model, enforcement semantics, maintenance health, and token cost — all read from primary sources (repo docs, validator source code, GitHub/npm APIs, filed issues), with the enforcement claims verified against `validator.ts` rather than taken from documentation. **Medium** on real-world friction, since it rests on filed issues and one independent hands-on report rather than our own trial. **Low** on Rust suitability — that is an evidence gap, not a negative finding.

- **Unverified claims:**
  - **Star count corrected, not confirmed retroactively.** The 52k-stars-in-June-2026 figure in the briefing could not be checked against a historical snapshot. The current API value is **64,224 (2026-08-08)**, which is consistent with continued growth from ~52k in June. Treat 52k as plausibly correct-as-of-then; 64,224 is the number to use.
  - **"50 KB context limit."** A third-party comparison blog attributes a 50 KB context cap to OpenSpec. This appears nowhere in the primary docs or source we read. **Do not rely on it.**
  - **The 12min / 90min / 5.5hr benchmark** (OpenSpec vs Spec Kit vs BMAD on a CRM dashboard) comes from a single vendor-adjacent blog post with no published methodology. It is a marketing-grade anecdote.
  - **Whether issue #1246's silent-overwrite bug is actually fixed.** The issue is closed, but we did not trace the closing commit or confirm a regression test exists. Given the severity (silent spec data loss), **verify this hands-on before relying on parallel changes.**
  - **Skill loading cost.** The ~1,000–4,200 tokens/invocation figure assumes Claude Code loads one `SKILL.md` at a time rather than all 13 up front. That follows from how Claude Code skills generally work, not from anything OpenSpec documents. Measure it.
  - **Token estimates use bytes ÷ 4.** Fine as an order-of-magnitude proxy for English Markdown; the real count will differ, likely somewhat higher for spec-dense text with heavy punctuation.
  - **No Reddit or HN discussion was located.** Searches surfaced only SEO-oriented comparison content. The absence of organic community discussion for a 64k-star project is itself mildly notable and worth a second look before treating star count as a proxy for real-world use.

- **Open questions a decision-maker would still need to test hands-on:**
  1. Can a MySQL wire-protocol behavior be expressed usefully in the Requirement/Given-When-Then form? Draft one real spec (e.g. capability-flag negotiation during handshake) before committing. This is the single highest-value experiment.
  2. What actually happens when two changes MODIFY the same requirement on v1.8.0? Reproduce #1246 directly.
  3. Does a forked schema with a `test-plan` artifact meaningfully change agent behavior on a Rust codebase, or does the agent ignore prompt-level rules under implementation pressure?
  4. How do you express a latency or concurrency budget? There is no documented answer — you will be inventing a house convention, and should decide whether that belongs in `spec.md`, `design.md`, or outside OpenSpec entirely.
  5. What is the real steady-state token overhead per change on *this* project after 20–30 changes have accumulated in `openspec/specs/`?
  6. Is the Node.js dependency acceptable in your CI, and will you pin the CLI version to avoid the churn documented in §6?
