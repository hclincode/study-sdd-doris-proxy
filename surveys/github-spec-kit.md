# Survey: GitHub Spec Kit

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** agent (research subagent)
> **Scope:** The `github/spec-kit` toolkit and its SDD methodology as of v0.16.1, evaluated for adoption on a greenfield async-Rust MySQL-wire-protocol proxy. Excludes forks, third-party reimplementations (e.g. the `spec-kit-mcp` Rust crate), and general SDD theory not embodied in this tool.
>
> **Scope update 2026-08-08.** The project has since narrowed to an L7 proxy whose purpose is
> **SQL rewriting for tenant isolation and schema remapping**, with 1:1 backend connections and
> passthrough auth (`discussions/milestone-1/02-proxy-scope.md`). Consequences for this survey:
> **(a)** §8's concern about `data-model.md` and `contracts/` being poorly matched to wire-protocol
> work is now confirmed hands-on — see `archives/milestone-1/trial/speckit/specs/001-l7-mysql-proxy/data-model.md`;
> **(b)** the **constitution** looks better than ever, because tenant isolation adds exactly the kind
> of cross-cutting, non-negotiable invariant it exists to hold (fail closed on parse failure;
> rewrites must not change placeholder count or result shape);
> **(c)** §6's regenerate cascade (issue #1059) is a worse problem than first assessed — the rewrite
> rule catalogue is append-and-refine forever, and Spec Kit has no spec lifecycle at all.
> A hands-on trial has since been run against the Spec Kit format: see `archives/milestone-1/trial/`.

## 1. Snapshot

| Field | Value |
|---|---|
| Name | Spec Kit (CLI binary: `specify`, PyPI package `specify-cli`) |
| Vendor / owner | GitHub, Inc. (`github/spec-kit`) |
| License | MIT |
| First release | v0.0.1, September 2025 (repo created 2025-08-21) |
| Latest version (as of survey date) | **v0.16.1, published 2026-08-07** — one day before this survey |
| Popularity signal | **125,795 stars**, 11,230 forks, 649 watchers |
| Maintenance signal | Extremely active. 16 minor lines in ~11 months; ≥10 releases in the last ~3 weeks (v0.14.0 on 2026-07-23 → v0.16.1 on 2026-08-07). 319 open issues+PRs (~152 issues / ~167 PRs). Last push 2026-08-07. |
| Primary interface | Python CLI (`specify`) for scaffolding + agent-native slash commands/skills for the workflow |
| Agent compatibility | 30+ integrations (README claims "30+"; docs list ~29 named + Generic): Copilot, Claude Code, Codex, Gemini, Cursor, Kilo, Zed, Cline, Forge, Kiro, Windsurf, Factory Droid, Alquimia, … |
| Language/stack neutrality | Neutral by construction — the tool ships prompts + Markdown templates, never touches your build system. Template *vocabulary* leans web/service (see §8). |
| Greenfield vs brownfield bias | Strongly greenfield. `specify init` scaffolds a fresh tree; per-feature numbered spec dirs assume net-new features. `/speckit.converge` (added ~2026) is the retrofit escape hatch for existing code. |

**Correction to the briefing:** the "111k stars, v0.10.2 in June 2026" figure is stale. Verified today via the GitHub API: **125,795 stars** and **v0.16.1 (2026-08-07)**. The README itself contains a stale `v0.12.11` example string; do not use README version strings as the source of truth.

## 2. One-paragraph thesis

Spec Kit's bet is that the bottleneck in AI-assisted development has moved from *writing code* to *stating intent precisely enough that generated code is the right code*. Its answer is to invert the usual artifact hierarchy: the Markdown specification becomes the durable, version-controlled source of truth and the code becomes its generated expression — "when specifications and implementation plans generate code, there is no gap, only transformation." Mechanically it does this by installing a fixed sequence of prompt-heavy slash commands into your coding agent, each of which reads the previous phase's Markdown, fills a rigid template, and writes the next phase's Markdown, all under a project "constitution" of non-negotiable principles that every downstream phase is instructed to re-check itself against. The claim is not that the AI writes better code, but that a structured spec→plan→tasks funnel makes the AI's output *reviewable and re-derivable*. The most substantiated counter-claim from practitioners is that it mostly produces a large volume of plausible Markdown whose review cost exceeds the cost of just writing the code (§6).

## 3. Workflow / artifact model

The developer runs a linear chain of agent commands. Each command invokes a shell/PowerShell/Python helper script (which does deterministic path resolution and prerequisite checking) and then executes a long prompt file that fills a Markdown template.

```
my-project/
├── .specify/
│   ├── memory/
│   │   └── constitution.md          # project principles — read by every later phase
│   ├── scripts/
│   │   └── bash/                    # (or powershell/ or python/, chosen at init)
│   │       ├── common.sh
│   │       ├── create-new-feature.sh   # numbers + creates specs/NNN-slug/, copies spec template
│   │       ├── setup-plan.sh
│   │       ├── setup-tasks.sh
│   │       └── check-prerequisites.sh  # the only real gate (see §4)
│   └── templates/
│       ├── spec-template.md         #  4,556 B
│       ├── plan-template.md         #  3,770 B
│       ├── tasks-template.md        #  9,182 B
│       ├── constitution-template.md #  2,346 B
│       ├── checklist-template.md    #  1,329 B
│       └── overrides/               # project-local template overrides (highest precedence)
├── .claude/
│   ├── skills/
│   │   └── speckit-<name>/SKILL.md  # Claude Code installs SKILLS, not plain commands
│   └── settings.json                # optional SessionStart/PreToolUse/PostToolUse/… hooks
└── specs/
    └── 001-connection-pooling/
        ├── spec.md                  # WHAT/WHY: user stories P1–P3, FR-001…, SC-001…, edge cases
        ├── plan.md                  # HOW: technical context, Constitution Check, project structure
        ├── research.md              # library/technology comparisons, resolved unknowns
        ├── data-model.md            # entities, fields, relationships
        ├── contracts/               # API/interface specs (OpenAPI, WebSocket events, …)
        ├── quickstart.md            # key validation scenarios
        ├── checklists/              # generated by /speckit.checklist
        └── tasks.md                 # ordered, ID'd tasks with [P] parallelization markers
```

**Command surface** (canonical `/speckit.*` form; Claude Code installs these as skills, invoked as `/speckit-<name>` — the dot is not legal in a skill directory name):

| # | Command | Required? | Produces |
|---|---|---|---|
| 1 | `/speckit.constitution` | once per project | `.specify/memory/constitution.md` |
| 2 | `/speckit.specify` | yes | `specs/NNN-slug/spec.md` (+ the feature dir) |
| 3 | `/speckit.clarify` | optional gate | edits `spec.md`, resolving `[NEEDS CLARIFICATION]` markers via ≤5 questions |
| 4 | `/speckit.plan` | yes | `plan.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md` |
| 5 | `/speckit.checklist` | optional gate | `checklists/*.md` |
| 6 | `/speckit.tasks` | yes | `tasks.md` |
| 7 | `/speckit.analyze` | optional gate | cross-artifact consistency report (no file mandated) |
| 8 | `/speckit.implement` | yes | source code + tests, ticking off `tasks.md` |
| 9 | `/speckit.converge` | optional | assesses existing codebase vs specs, lists remaining work |
| — | `/speckit.taskstoissues` | optional | GitHub issues from `tasks.md` |

Extensions add more: `bug` (`assess`/`fix`/`validate`), `git` (`commit`/`feature`/`initialize`/`remote`/`validate`), `assess` (`intake`/`research`/`shape`/`define`/`decide`).

**Install path:** requires [uv](https://docs.astral.sh/uv/). `uv tool install specify-cli` (persistent) or `uvx --from git+https://github.com/github/spec-kit.git specify init …` (one-shot). Then `specify init <project> --integration claude`. `--script sh|ps|py` selects the helper-script flavour. `specify preset add lean`, `specify extension add bug`, `specify integration list` manage the customization layers.

**Artifact lifecycle:** Specs are plain Markdown committed to your repo under `specs/NNN-slug/`. There is **no delta model and no archive step** — a spec directory is created, filled, and then simply sits there forever. Feature numbering is sequential (`001`, `002`, …), scanned from existing directory names, with `--number` and `--timestamp` overrides. Notably, `create-new-feature.sh` in current `main` **does not create a git branch**; it computes a branch-safe name and emits it, leaving branch creation to the caller or the `git` extension. (Earlier versions did auto-branch; if you read older blog posts expecting `git checkout -b`, that behaviour has moved.) There is no built-in mechanism to update a plan after a spec changes — you regenerate, which is the subject of a long-standing open issue (#1059, 15 reactions).

## 4. What it enforces vs. what it suggests

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | **Partially.** `check-prerequisites.sh` exits non-zero with `ERROR: Feature directory not found`, `ERROR: plan.md not found in $FEATURE_DIR`, and (under `--require-tasks`) `ERROR: tasks.md not found`. So `/plan` genuinely refuses to run without a feature dir, and `/implement` without `tasks.md`. | The gate only guards the *Spec Kit commands*. Nothing stops you — or the agent — from just editing source files directly. It is a turnstile, not a wall. |
| Spec ↔ code traceability | Nothing. No index, no backlink, no check that FR-007 was implemented. | Templates ask for `FR-###`/`SC-###` IDs and `tasks.md` entries that reference them; `/speckit.analyze` asks the LLM to look for orphans. |
| Test/acceptance criteria | Nothing. No test runner is invoked, no coverage threshold, no "tests must fail first" check. | `spec-template.md` mandates Given/When/Then acceptance scenarios and measurable `SC-###` success criteria; a test-first principle is conventionally placed in the constitution. Compliance is entirely the model's goodwill. |
| Review gate | Nothing. `/clarify`, `/checklist`, `/analyze` are all documented as **optional**. | The `[NEEDS CLARIFICATION: …]` marker convention, the ≤5-question `/clarify` interview, and self-review checklists embedded in templates. |
| Drift detection | Nothing deterministic. | `/speckit.analyze` (cross-artifact consistency) and `/speckit.converge` (codebase vs specs) are LLM comparisons of Markdown — non-reproducible, no exit code, no CI story. |
| Constitution compliance | Nothing. | `plan-template.md` contains a "Constitution Check" section evaluated **before Phase 0 research and again after Phase 1 design**, plus a "Complexity Tracking" table to justify violations. The template's own wording ("Fill ONLY if Constitution Check has violations that must be justified") makes clear a violation is *documented*, not *blocked*. |

**Bottom line:** the only hard enforcement in Spec Kit is *file existence in a fixed order*. Everything the methodology actually cares about — quality, traceability, testing, constitutional compliance — is prompt text. This matters for the adoption decision: Spec Kit does not give you a CI-checkable invariant. It gives you a habit plus a directory convention.

## 5. Strengths

- **Genuinely agent-neutral and vendor-neutral.** 30+ integrations, MIT licence, and the entire persistent state is Markdown in your repo. Switching from Claude Code to Codex is `specify init --integration codex` in the same repo; the specs are untouched.
- **The constitution is the highest-leverage part.** A short, project-authored principles file that every planning phase is instructed to re-read is a cheap, durable way to stop an agent re-litigating architectural decisions each session. For a proxy with hard invariants (never buffer a whole result set, never block the runtime, wire-format fidelity) this is the piece most worth stealing even if you reject the rest.
- **Real ordering enforcement exists.** Unlike purely prompt-based SDD frameworks, `check-prerequisites.sh` does return non-zero. The agent cannot accidentally `/implement` a feature with no `tasks.md`.
- **`/speckit.analyze` catches real contradictions.** The Rust hands-on writeup reports `/analyze` surfacing an architectural contradiction (a stateless-design principle violated by an in-memory history feature) *before* implementation, forcing a constitution amendment. That is the intended mechanism working.
- **Customization is first-class and layered**, with a documented precedence order: project-local `templates/overrides/` > presets > extensions > core. The bundled **`lean` preset** is an official acknowledgement of the verbosity problem — it replaces the template-driven commands with self-contained prompts producing just `spec.md`/`plan.md`/`tasks.md`/`constitution.md`, "just the prompt, just the artifact."
- **Backed and maintained by GitHub**, with release cadence measured in days. Low bus-factor risk over the next 12 months.
- **Claude Code integration is idiomatic**, not bolted on: skills in `.claude/skills/` with `user-invocable` and `argument-hint` frontmatter, plus support for all five lifecycle hook events (`SessionStart`, `PreToolUse`, `PostToolUse`, `SessionEnd`, `UserPromptSubmit`) wired into `.claude/settings.json`.

## 6. Weaknesses / friction

- **The output-volume-to-value ratio is the central, repeatedly-documented complaint.** The most rigorous public comparison (Scott Logic, Nov 2025) built the same feature twice: Spec Kit produced **689 lines of code alongside 2,577 lines of Markdown**, taking 33.5 min of agent time and **3.5 hours of human review**; the author's normal iterative prompting produced ~1,000 lines of code with no Markdown in **24 minutes total**. Roughly a 10× slowdown — and the Spec Kit run still shipped a bug, undercutting the "specs prevent errors" premise. Verdict: *"Asking an agent to write 1000s of lines of markdown rather than just asking it to write the code is a misuse of this technology."*
- **Standing context tax on every session.** The nine command prompt files in `templates/commands/` total **127,979 bytes** (`checklist.md` alone 20,796 B; `clarify.md` 19,022 B; `specify.md` 18,083 B) — very roughly **~32k tokens** of prompt text if fully resident. Open issue #1401 (Dec 2025, 15 reactions, **still open, no maintainer response, no mitigation shipped**) measured 18.6k tokens consumed per session at the time, i.e. ~29% of a 64k Copilot context and ~93% of Cursor's default. *Mitigating factor specific to Claude Code:* because Spec Kit now installs as **skills** rather than raw commands, only skill frontmatter should be resident until invocation, which should substantially blunt this — but I could not verify Claude Code's actual resident footprint empirically (§11).
- **Rigid phase coupling with no incremental update path.** Change the spec and you must regenerate plan, then tasks — losing hand edits. Open issue #1059 asks for exactly this and remains open. On a project where the protocol understanding will evolve as you read Doris/MySQL wire behaviour, this is the friction that will bite hardest.
- **"Illusion of work."** Discussion #1784 (`SpecKit creates the illusion of work, generating a bunch of text`) argues polished Markdown with emoji headings manufactures false confidence while the implementation still ignores requirements: *"you need to spend hours to fix plan, tasks and then fix implementation anyway."* No maintainer reply in the thread.
- **Context loss during `/implement`.** Multiple practitioners report the agent losing the thread partway through a long `tasks.md` run. The community's own workaround — delegate phases to subagents — is open issue #752 (25 reactions, top-voted, unimplemented). Spec Kit's own Claude integration currently forks **no** commands to subagent contexts, though the infrastructure exists.
- **Cost.** One report of hitting Claude Pro's 5-hour usage limit after two tasks, having never hit it before adopting Spec Kit. Treat Spec Kit as a multiplier on token spend, not a rounding error.
- **Documentation and command naming are in flux.** The `speckit.` prefix, the skills-vs-commands switch (Copilot's default flipped only in v0.16.0, three days ago), `/converge`, presets and extensions are all recent. Most blog posts and Stack Overflow answers you will find describe an older command surface. The README ships a stale version string. Expect to re-learn the tool during a 6-month project.
- **Constitution/CLAUDE.md overlap is unresolved.** Issue #609 ("Claude.md vs constitution.md", 24 reactions) asks the obvious question and there is no authoritative answer. You will maintain two overlapping principle files.
- **No archive/retire lifecycle.** `specs/` grows monotonically. After 40 features you have 40 directories of stale plans with no signal about which describe current behaviour.

## 7. Fit signals

**Strong fit when:**

- The feature is well-understood up front and the risk is *building the wrong thing*, not *not knowing how to build it*.
- Multiple people (or multiple agent sessions over weeks) need a shared, durable statement of intent.
- The stack is one the model already knows well, so the plan phase is genuinely about *choices* rather than *discovery*.
- You want agent-portability and refuse to be locked to one vendor's proprietary planning format.
- You will actually adopt the `lean` preset and skip the optional gates on routine work.

**Poor fit when:**

- The work is exploratory or research-heavy — where you learn the requirements by writing code. Protocol reverse-engineering, performance tuning, and concurrency-bug hunting are all this.
- Changes are small. Multiple reviewers make the same point: for a five-line fix the ceremony is absurd.
- Specs will churn — the regenerate-everything coupling punishes iteration.
- The correctness properties that matter cannot be written as Given/When/Then user stories (see §8).
- Context budget is already tight, or token spend is a real constraint.

## 8. Rust / systems-software notes

**Does it work on Rust at all? Yes, with direct evidence.** A detailed public writeup (40tude, 2026) drove Spec Kit with Claude Code (Opus 4.6) to build an Axum/Tokio Rust service end to end. Reported findings: cargo build/test commands slotted into `tasks.md` naturally; unit tests for domain logic and integration tests via `reqwest` were generated; the plan phase surfaced Rust-specific design questions unprompted (choosing `Arc<Mutex<…>>` for shared state, `thiserror` for error modelling). The author's constitution encoded domain purity, TDD, layering, tracing-based observability, and YAGNI — and `/analyze` caught a violation of it. That is a real, positive Rust data point.

**But note what that project was:** a BMI calculator web app. It is a CRUD-shaped service, which is exactly where Spec Kit's template vocabulary lives.

**Where the templates fight a wire-protocol proxy:**

- `/plan` mandates `data-model.md` and `contracts/`. `contracts/` is documented in terms of *API specifications and WebSocket events*. For a MySQL wire-protocol proxy the "contract" is a byte-level packet format with sequence-ID state machines and capability-flag negotiation — expressible in Markdown, but you are fitting it into a slot designed for OpenAPI. Expect to override the plan template.
- `spec-template.md`'s mandatory unit is the prioritized **user story** with Given/When/Then acceptance scenarios. A proxy's hardest requirements are *invariants*, not scenarios: "COM_STMT_PREPARE state must survive backend failover", "no code path buffers an unbounded result set", "a slow client must not stall the connection's peer". These are properties over all executions. Nothing in Spec Kit has a place for them except as prose bullets that no tool checks.
- **Performance budgets have a home but no teeth.** `plan-template.md`'s Technical Context section does have explicit *Performance Goals*, *Constraints*, and *Scale/Scope* fields — better than nothing, and a good place to pin p99 latency and connections-per-instance targets. But like everything else in §4, nothing verifies them.
- **Property-based and mutation testing are absent from the vocabulary.** I found no reference to `proptest`, `quickcheck`, fuzzing, `cargo-mutants`, or loom in any Spec Kit template or command. Given this repo's `.gitignore` already anticipates `cargo-mutants`, that testing discipline would have to be encoded by hand into the constitution and a template override — Spec Kit will not suggest it.
- **Concurrency reasoning is not modelled.** There is no artifact for "the state machine of a connection", "which tasks share which locks", or "cancellation-safety of this `select!`". `data-model.md` is entity/field/relationship shaped. For a proxy, the interesting model is a state machine and a task graph.
- **Legacy constitution articles are a mismatch trap.** `spec-driven.md` describes nine constitutional articles including *library-first design* and a *CLI interface mandate* — sensible for a toolkit, actively wrong for a long-running network daemon. The **currently shipped** `constitution-template.md` is only 2,346 bytes and is project-defined placeholder-driven, so those nine articles appear to be illustrative/legacy rather than imposed (medium confidence — see §11). Do not copy them.

**Net:** nothing in Spec Kit blocks Rust systems work, and the constitution + performance-budget fields are usable as-is. But the two artifacts it insists on — `data-model.md` and `contracts/` — are the two least useful for a wire proxy, and the correctness properties that will actually determine whether this project succeeds have no first-class representation. Budget for template overrides from day one.

## 9. Cost & lock-in

- **Money:** Free. MIT licence, no paid tier, no hosted service, no account. Cost is entirely your agent's token bill.
- **Token cost:** Substantial and the main real cost. Standing overhead of up to ~32k tokens of command prompts (materially lower with Claude Code's skill-based progressive loading — unverified). Per feature, the measured data point is 2,577 lines of generated Markdown for 689 lines of code — call it low-thousands of output tokens per artifact across 7–9 artifacts, each re-reading its predecessors. A rough planning figure: **assume the full workflow costs several times the tokens of just asking the agent to implement the feature**, and assume the `lean` preset cuts that meaningfully.
- **Lock-in:** Very low. Everything durable is plain Markdown (`specs/`, `.specify/memory/constitution.md`) plus shell/Python scripts in your own repo. The `specify` CLI is only needed for scaffolding and upgrades; the workflow itself is agent-executed prompt files you own.
- **Exit path:** Delete `.specify/` and the agent's skill/command directory. `specs/` remains as ordinary documentation. There is no database, no proprietary format, no export step. This is the single best thing about the tool from a risk standpoint — a 3-month trial is cheap to abandon.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://github.com/github/spec-kit | repo/README | 2026-08-08 (accessed) | Command list, install path, 30+ agents, MIT, "experimental research" disclaimer |
| 2 | https://api.github.com/repos/github/spec-kit | repo API | 2026-08-08 | 125,795 stars, 11,230 forks, 319 open issues+PRs, created 2025-08-21, pushed 2026-08-07 |
| 3 | https://api.github.com/repos/github/spec-kit/releases | repo API | 2026-08-08 | v0.16.1 (2026-08-07) latest; 10 releases 2026-07-23 → 2026-08-07 |
| 4 | https://api.github.com/repos/github/spec-kit/tags?page=3 | repo API | 2026-08-08 | Earliest tag v0.0.1 |
| 5 | https://raw.githubusercontent.com/github/spec-kit/main/spec-driven.md | methodology doc | accessed 2026-08-08 | "Executable specifications", nine constitutional articles, phase gates, `specs/` tree |
| 6 | https://raw.githubusercontent.com/github/spec-kit/main/templates/spec-template.md | template | accessed 2026-08-08 | ~280 lines; mandatory User Scenarios / Requirements / Success Criteria; `[NEEDS CLARIFICATION]` |
| 7 | https://raw.githubusercontent.com/github/spec-kit/main/templates/plan-template.md | template | accessed 2026-08-08 | Constitution Check gate (pre-Phase-0 and post-Phase-1), Complexity Tracking, Technical Context perf fields |
| 8 | https://raw.githubusercontent.com/github/spec-kit/main/scripts/bash/check-prerequisites.sh | source | accessed 2026-08-08 | Hard non-zero exits: missing feature dir, missing `plan.md`, missing `tasks.md` under `--require-tasks` |
| 9 | https://raw.githubusercontent.com/github/spec-kit/main/scripts/bash/create-new-feature.sh | source | accessed 2026-08-08 | Sequential numbering, `--number`/`--timestamp`/`--short-name`/`--json`/`--dry-run`; **does not create a git branch** |
| 10 | https://raw.githubusercontent.com/github/spec-kit/main/src/specify_cli/integrations/claude/__init__.py | source | accessed 2026-08-08 | Claude Code → `.claude/skills/*/SKILL.md`, `user-invocable`, `argument-hint`, 5 hook events, no subagent forking |
| 11 | https://api.github.com/repos/github/spec-kit/contents/templates/commands | repo API | 2026-08-08 | Byte sizes of all 10 command prompts, total 127,979 B |
| 12 | https://api.github.com/repos/github/spec-kit/contents/templates | repo API | 2026-08-08 | Template byte sizes (spec 4,556 / plan 3,770 / tasks 9,182 / constitution 2,346 / checklist 1,329) |
| 13 | https://github.com/github/spec-kit/issues/1401 | issue | 2025-12-29, open | 18.6k tokens/session; per-command breakdown; no maintainer response, no mitigation |
| 14 | https://github.com/github/spec-kit/issues/752 | issue | 2025-10-06, open, 25 reactions | Request to use Claude Code subagents to avoid context saturation |
| 15 | https://github.com/github/spec-kit/issues/609 | issue | 2025-09-26, open, 24 reactions | Unresolved CLAUDE.md vs constitution.md overlap |
| 16 | https://github.com/github/spec-kit/issues/1059 | issue | 2025-10-27, open, 15 reactions | No way to update a plan after the spec changes |
| 17 | https://github.com/github/spec-kit/discussions/1784 | discussion | 2026 | "SpecKit creates the illusion of work"; counter-arguments from community; no maintainer reply |
| 18 | https://blog.scottlogic.com/2025/11/26/putting-spec-kit-through-its-paces-radical-idea-or-reinvented-waterfall.html | hands-on report | 2025-11-26 | 689 LOC vs 2,577 lines MD; 33.5 min agent + 3.5 h review vs 24 min baseline; ~10× slower; shipped a bug |
| 19 | https://www.40tude.fr/docs/06_programmation/rust/027_speckit/speckit.html | hands-on report (Rust) | 2026 | Axum/Tokio Rust service via Claude Opus 4.6; constitution contents; `/analyze` caught a real contradiction; friction points |
| 20 | https://github.github.com/spec-kit/reference/overview.html | official docs | accessed 2026-08-08 | Canonical 9-command order; clarify/checklist/analyze explicitly optional; bug extension |
| 21 | https://github.com/github/spec-kit/tree/main/presets/lean | repo | accessed 2026-08-08 | `lean` preset: "just the prompt, just the artifact"; overrides 5 commands; `specify preset add lean` |
| 22 | https://ms3byoussef.medium.com/spec-kit-how-to-use-it-with-claude-codex-and-gemini-without-turning-your-project-into-3d4cdec2d7e1 | blog | 2026 | Confirms `.claude/skills` install location; ~29 named integrations; Claude Code + Codex use skills mode |
| 23 | https://medium.com/@makkalot/giving-a-try-to-spec-kit-dad6931ba396 | hands-on report | 2025–26 | Hit Claude Pro 5-hour limit after two tasks; regenerate-cascade complaint; context loss during implement |
| 24 | https://developer.microsoft.com/blog/spec-driven-development-spec-kit/ | vendor blog | 2026 | General workflow framing (used for corroboration only) |

## 11. Confidence & gaps

- **Confidence: high** on version, popularity, maintenance cadence, command surface, file tree, licence, install path, enforcement mechanics, and the existence and shape of the verbosity criticism — all read from primary sources (GitHub API, raw source files, official docs) today.
- **Confidence: medium** on the *magnitude* of token cost for this specific setup, and on the current status of the nine constitutional articles.

**Unverified claims (explicitly flagged):**

1. **The ~32k-token figure for command prompts is my own derivation** from measured byte counts (127,979 B ÷ ~4 B/token). It is an upper bound on prompt text, not a measured resident context footprint.
2. **Claude Code's actual resident context cost is unmeasured.** Because Spec Kit installs as skills, progressive disclosure *should* mean only frontmatter loads until invocation — which would make issue #1401's 18.6k figure largely inapplicable to Claude Code today. I could not confirm this empirically and found no source that measured it post-skills-migration. **This is the single most decision-relevant unknown and is cheap to test.**
3. **Whether the nine constitutional articles in `spec-driven.md` are still imposed.** The shipped `constitution-template.md` is only 2,346 B and appears placeholder-driven, suggesting the articles are illustrative/legacy. I did not read the template body verbatim.
4. **Exact `/speckit.converge` semantics** — added recently; documented only as "assesses codebase against specs and identifies remaining work". Its brownfield usefulness is unproven.
5. **CHANGELOG archaeology was inconclusive.** I could not pin the dates when the `speckit.` prefix, `/converge`, or the removal of automatic git-branch creation landed; the fetched changelog only covered v0.12.5 onward.
6. **`create-new-feature.sh` not creating a branch** is read from current `main`. Behaviour may differ in the released v0.16.1 artifact bundle.
7. **The `lean` preset's actual output reduction is unquantified.** No source measures it; the claim is the preset's own README language.
8. **No evidence found of Spec Kit used on a network proxy, wire protocol, or performance-critical systems project.** The only Rust data point is a CRUD web service. Fit for this project is inferred, not observed.

**Open questions a decision-maker should test hands-on (≤1 day):**

- Run `specify init --integration claude` on a throwaway repo and measure Claude Code's *actual* resident context before and after. This resolves gap (2) and is the make-or-break number.
- Run one real Doris-proxy feature (e.g. "handle COM_QUIT and connection teardown") through the full default workflow and again through the `lean` preset. Compare Markdown-lines-to-code-lines and wall-clock review time against the Scott Logic 2,577:689 baseline.
- Write the proxy's constitution first and check whether the invariants that matter (cancellation safety, no unbounded buffering, wire-format fidelity) survive into `plan.md`'s Constitution Check in a form that changes generated code — or whether they are merely restated.
- Deliberately change a spec after `tasks.md` exists and measure the cost of the regenerate cascade (issue #1059). On a protocol project this will happen weekly.
- Test whether `contracts/` and `data-model.md` can be overridden away via `.specify/templates/overrides/` without breaking `/speckit.plan`.
