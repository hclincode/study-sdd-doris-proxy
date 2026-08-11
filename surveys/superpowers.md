# Survey: Superpowers (obra) — and how it compares to OpenSpec

> **Template version:** 1.0 · **Surveyed:** 2026-08-09 · **Author:** research team (Claude Opus 5, 3 agents)
> **Scope:** `obra/superpowers` v6.2.0 evaluated as a candidate process framework for this project, and compared head-to-head with OpenSpec v1.8.0 (`surveys/openspec.md`). Covers the plugin's skill inventory, workflow, artifacts, enforcement mechanics, token cost, independent reception and maintenance health. Adds two sections beyond the template — **§12 head-to-head** and **§13 when to use which** — because the request was a comparison and a recommendation, not a survey alone. Excludes the sibling plugins (`superpowers-chrome`, `superpowers-lab`, `episodic-memory`) beyond naming them.
>
> **The finding that governs everything below:** Superpowers and OpenSpec are **not competitors**. OpenSpec governs the *artifacts that describe what the system does*; Superpowers governs *how the agent behaves while producing them*. They occupy different layers, they overlap in exactly two steps, and each is silent where the other is strongest. Reading this as "which one wins" produces the wrong answer. A serious independent SDD comparison (`cameronsjo/spec-compare`, six tools) **omits Superpowers entirely** — people building those comparisons do not necessarily classify it as an SDD tool at all.

## 1. Snapshot

| Field | Value |
|---|---|
| Name | Superpowers (`superpowers`) — a plugin, not a package |
| Vendor / owner | Jesse Vincent (`obra`), a personal account; commercially backed by **Prime Radiant** (`sales@primeradiant.com` for enterprise support) |
| License | MIT |
| First release | Repo created 2025-10-09; `RELEASE-NOTES.md` documents ~45 versions back to v3.0.1 (2025-10-16) |
| Latest version (as of survey date) | **v6.2.0**, published 2026-07-24 |
| Popularity signal | **269,185 GitHub stars**, 24,038 forks, 1,017 subscribers (API, read independently twice on 2026-08-08/09). Accepted into Anthropic's **official plugin marketplace 2026-01-15**; that listing reports **1,009,371 installs** — vendor-reported, no methodology. Growth curve corroborated across independent secondary sources (~150k → 177k in May → ~268k in August) |
| Maintenance signal | 52 commits on `main` in 30 days, but **160 on `dev`** — `CLAUDE.md` requires PRs to target `dev`, so `main` understates activity. `dev` is 119 commits ahead. 149 open / 681 closed issues; 174 open / 923 closed PRs. **No CI** — `.github/` has no `workflows/` directory |
| Primary interface | **Claude Code plugin** (also 10 other harnesses). No CLI, no MCP server, no npm package for consumption |
| Agent compatibility | 11 harnesses with per-ecosystem manifests: Claude Code, Antigravity, Codex App/CLI, Cursor, Factory Droid, Gemini CLI, Copilot CLI, Kimi Code, OpenCode, Pi |
| Language/stack neutrality | Structurally neutral — the skills describe process, not stacks. But the TDD rhythm and plan granularity carry a hidden fast-test-loop premise, and the evidence of use is overwhelmingly web/infra (§8) |
| Greenfield vs brownfield bias | **Neither, and that is the point.** It is task-shaped, not codebase-shaped: it triggers per unit of work regardless of what exists around it |

**Bus factor: 1.** Consolidating multiple email identities across all 680 `main` commits: Jesse Vincent **535 (78.7%)**, Drew Ritter **86 (12.6%)** — both `@primeradiant.com` — CL Kao 8, and ~40 other identities sharing ~51. In the last 90 days: Vincent 168, Ritter 55, everyone else 1–2 each. `CLAUDE.md` states, twice, **"This repo has a 94% PR rejection rate,"** and the README says "we don't generally accept contributions of new skills." This is a deliberately closed core with a very large audience — and a **~9% fork-to-star ratio** (24,038 / 269,185), which is high, and which the community evidence in §6 explains: the dominant adoption pattern among experienced users is *fork and rewrite*, not install.

## 2. One-paragraph thesis

Superpowers' bet is that the bottleneck in AI-assisted coding is not intent transfer but **agent discipline under pressure** — that a capable model knows how to do TDD, trace a root cause, and verify before claiming success, and will nonetheless skip all three the moment a task feels urgent or simple. Its answer is a library of 14 behavioural skills written not as documentation but as *counter-rationalization pressure*: an Iron Law (`NO PRODUCTION CODE WITHOUT A FAILING TEST FIRST` — "Write code before the test? Delete it. Start over."), a `<HARD-GATE>` forbidding any implementation before a design is approved, a twelve-row "Common Rationalizations" table pre-refuting the excuses a model reaches for, and a bootstrap injected at every session start wrapped in `<EXTREMELY_IMPORTANT>` tags reading "IF A SKILL APPLIES TO YOUR TASK, YOU DO NOT HAVE A CHOICE." The distinguishing methodological claim is not the content — TDD and root-cause analysis are decades old — but that **the skill text is treated as tuned code and validated empirically**: a public eval harness drives real tmux sessions of Claude Code, Codex and Gemini CLI and scores compliance, and at least one compression change was reverted because it measurably degraded test-first behaviour (control 8/10 → treatment 5/10 under "just write it, tests after" pressure). `CLAUDE.md` says it directly: *"Skills are not prose — they are code that shapes agent behavior."* Nothing in the plugin can block a tool call; the whole framework is prompt text, and the author's compensating mechanism is measurement rather than mechanism. Whether that measurement supports the *product* claim rather than the version-over-version one is the central open dispute (§6).

## 3. Workflow / artifact model

**What the developer types: nothing.** README: *"because the skills trigger automatically, you don't need to do anything special."* The project's own acceptance test for a new harness integration is the single user message `Let's make a react todo list` — a working install must auto-trigger `brainstorming` before any code is written. `CLAUDE.md` is explicit that this is what integration *means*: *"A real integration loads the `using-superpowers` bootstrap at session start… Without it, the skills are dead weight — present on disk but never invoked."*

The documented chain:

```
brainstorming  →  using-git-worktrees  →  writing-plans  →  subagent-driven-development
                                                             (or executing-plans)
                                                                     ↓
                            test-driven-development  →  requesting-code-review
                                                                     ↓
                                                    finishing-a-development-branch
```

**The 14 skills**, each a directory under `skills/` containing a `SKILL.md`:

| Skill | Role |
|---|---|
| `using-superpowers` | The bootstrap. Injected at every session start; teaches skill discovery and mandates invocation |
| `brainstorming` | Elicitation before any creative work; ends in a committed design document |
| `writing-plans` | Turns a design into a task-by-task implementation plan |
| `executing-plans` | Runs a plan in a separate session with review checkpoints |
| `subagent-driven-development` | Runs a plan in *this* session via implementer/reviewer subagents |
| `test-driven-development` | The Iron Law; red-green-refactor rhythm |
| `systematic-debugging` | Four mandatory phases; "NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST" |
| `verification-before-completion` | "NO COMPLETION CLAIMS WITHOUT FRESH VERIFICATION EVIDENCE" |
| `requesting-code-review` | Dispatches a reviewer subagent from a template |
| `receiving-code-review` | Forbids performative agreement; requires verifying feedback before acting on it |
| `using-git-worktrees` | Isolated workspace before feature work |
| `dispatching-parallel-agents` | For 2+ independent, non-shared-state tasks |
| `finishing-a-development-branch` | Integration decision; typed-confirmation ritual before destructive actions |
| `writing-skills` | Meta-skill for authoring and pressure-testing new skills |

Three corrections worth recording, because the names circulate wrongly: there is **no YAGNI skill** (YAGNI is prose inside five other skills), and **`root-cause-tracing` and `defense-in-depth` are not skills** — they are reference documents inside `skills/systematic-debugging/`. The count is 14 skills plus 39 supporting files, **343 KB** under `skills/` in total.

**Artifact tree.** Two kinds of artifact with different lifecycles, both written into *your* project (the plugin writes nothing at install time):

```
your-project/
├── docs/superpowers/
│   ├── specs/YYYY-MM-DD-<topic>-design.md    # durable, git-committed, from brainstorming
│   └── plans/YYYY-MM-DD-<feature>.md         # durable, git-committed, from writing-plans
└── .superpowers/sdd/<plan-basename>/         # EPHEMERAL scratch: task briefs, implementer
    └── .gitignore  → "*"                     # reports, review packages, progress ledger.
                                              # Self-ignoring. Deleted when final review is clean.
```

Both durable paths are overridable ("User preferences for spec/plan location override this default"). The repo dogfoods the model: **17 committed specs and 11 committed plans**, dated 2026-01-22 through 2026-07-15.

**Command surface.** There is none in the usual sense. Installation is one line per harness:

```sh
/plugin install superpowers@claude-plugins-official        # Anthropic's official marketplace
/plugin marketplace add obra/superpowers-marketplace       # or the author's own
/plugin install superpowers@superpowers-marketplace
```

After that the surface is the harness's own `Skill` tool plus 13 helper scripts the agent is *told* to run (`sdd-workspace`, `task-brief`, `review-package`, `find-polluter.sh`, the brainstorming visual-companion server, `render-graphs.js`).

**The plan format is highly prescriptive.** `writing-plans` mandates a header block (Goal / Architecture / Tech Stack / Global Constraints), tasks naming exact `Files: Create/Modify/Test` paths, an `Interfaces: Consumes/Produces` block, and steps as checkboxes sized at **2–5 minutes each** following the five-step TDD rhythm. Placeholders are banned by name — "TBD", "TODO", "implement later", "Add appropriate error handling", "Similar to Task N" — all labelled "plan failures". Users report this tips into over-specification: issue **#895**, *"the generated plan contained complete implementation code — full function bodies, 70-line test files, exact shell commands."*

**Artifact lifecycle — and the gap.** Designs and plans are created, committed, and then **accumulate forever**. There is **no archive model, no superseding, no status field, no merge into a current-truth document** — grep for archive/lifecycle across `skills/` returns nothing relevant, and the repo's own 28 accumulated dated files confirm the pattern. What Superpowers produces is a **journal of decisions**, not a **statement of current behaviour**. That is precisely the axis OpenSpec is built on, and §12 turns on it.

The ephemeral half has one documented hazard, stated in the v6.0.3 notes: because `.superpowers/sdd/` is git-ignored working-tree scratch, **`git clean -fdx` deletes the in-flight progress ledger**; recovery is from `git log`.

## 4. What it enforces vs. what it suggests

Determined by reading `hooks/hooks.json`, `hooks/session-start`, `.github/`, and the `SKILL.md` corpus directly.

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | **None.** No hook can block a tool call | `brainstorming` opens with a `<HARD-GATE>`: *"Do NOT invoke any implementation skill, write any code, scaffold any project, or take any implementation action until you have presented a design and the user has approved it. This applies to EVERY project regardless of perceived simplicity."* |
| Spec ↔ code traceability | **None.** Nothing reads your source tree | `writing-plans` requires every task to name exact `Files: Create/Modify/Test` paths and an `Interfaces: Consumes/Produces` block. A traceability *convention*, unchecked by anything |
| Test/acceptance criteria | **None.** No test-runner integration, no CI, no exit codes | The strongest prompt-level pressure in either framework: the Iron Law, "Verify RED — Watch It Fail: **MANDATORY. Never skip.**", a 12-row rationalization table, a 13-item "Red Flags — STOP and Start Over" list, and *"Violating the letter of the rules is violating the spirit of the rules."* |
| Review gate | **None** | `requesting-code-review` dispatches a `general-purpose` reviewer subagent from `code-reviewer.md`. `subagent-driven-development` runs implementer → task-reviewer → scoped re-review with a **five-round circuit breaker**: rounds ≤3 resume the implementer, ≥4 use a fresh one on a more capable model, at 5 the breaker trips and *"Any load-bearing finding? → STOP: report BLOCKED to human partner."* All of it graphviz and prose |
| Drift detection | **None.** No validator, no structural check, no notion of a canonical document to drift *from* | `verification-before-completion` demands fresh evidence before any success claim — *"Skip any step = lying, not verifying"* |

**The one mechanical component in the entire plugin** is a single `SessionStart` hook (matcher `startup|clear|compact`) running a 49-line bash script that reads `using-superpowers/SKILL.md`, JSON-escapes it, wraps it in `<EXTREMELY_IMPORTANT>` tags and emits it as `additionalContext`. It **always exits 0 — it cannot block anything**. There is no `PreToolUse`, no `PostToolUse`, no `Stop`, no `UserPromptSubmit`, no `PreCompact` anywhere in the repo, across any of the six harness manifest directories.

**One-line summary:** Superpowers is a *context injector wrapped around a prompt library*. It enforces less, mechanically, than OpenSpec does — OpenSpec at least ships a validator with exit codes — and it *pushes* far harder. The difference between the two is not tooling strictness; it is that one wrote a linter and the other wrote better prompts and then measured them.

## 5. Strengths

- **It fills the exact hole OpenSpec leaves open.** OpenSpec's `apply` skill — the one that writes code — was read in full for this survey: its implementation loop is *"Make the code changes required / Keep changes minimal and focused / Mark task complete / Continue to next task."* **It never mentions tests.** The word "test" appears 5 times across all 12 shipped OpenSpec skills, never as guidance to write one. Superpowers' entire centre of gravity is that step.
- **The elicitation phase is the part users independently praise.** This is the clearest signal in the reception evidence: four separate HN commenters converge on it unprompted. jghn: *"Using its brainstorm mode helps to keep it from shifting to just writing code… when I don't trigger brainstorm mode, even using the built in plan mode, it's just never as in depth of a brainstorm partner for me."* devnonymous, to the critics: *"Did the people who found it underwhelming not try starting with the brainstorming skill first?"* Notably, the praise attaches to `brainstorming` specifically — **not** to the execution machinery.
- **The skill text is empirically tuned, and this is rare.** A public eval harness (`prime-radiant-inc/superpowers-evals`, the "Quorum"/"drill" lab) drives real tmux sessions of Claude Code, Codex, Gemini and Kimi and grades workflow compliance with an LLM verifier plus deterministic post-checks. During the v6.2.0 compression campaign a deletion was **reverted on measurement**: removing one section degraded test-first behaviour from 8/10 to 5/10 under adversarial "just write it, tests after" pressure, corroborated on two harnesses. No other tool in this survey series shows any evidence of A/B-testing its own prompts. (The limits of that harness are in §6 — it grades compliance, not outcomes.)
- **Skills are written against model failure modes, not for human readers.** The rationalization tables and *"Thinking 'skip TDD just this once'? Stop. That's rationalization."* target the specific way a model talks itself out of process. `CLAUDE.md` defends the posture against Anthropic's own published guidance: *"PRs that restructure, reword, or reformat skills to 'comply' with Anthropic's skills documentation will not be accepted without extensive eval evidence showing the change improves outcomes."*
- **Zero project footprint at install, zero runtime dependencies.** Nothing is written into your repo until the agent produces a design document; *"Superpowers is a zero-dependency plugin by design."* Contrast OpenSpec, which needs Node ≥ 20.19 and copies 12 files (~116 KB) into every project, regenerated per project by `openspec update`. For a Rust-only toolchain this is a real advantage.
- **Genuinely multi-harness.** Eleven ecosystems with real per-harness manifests and reference files, plus a bootstrap that branches on `CURSOR_PLUGIN_ROOT` / `CLAUDE_PLUGIN_ROOT` / `COPILOT_CLI`. Only the Claude Code path was verified here.
- **Subagent orchestration is a first-class, specified loop** — implementer/reviewer separation, scoped re-reviews, a bounded retry policy, an explicit BLOCKED terminal state. The one HN commenter who used it most carefully rates it the best part despite the cost: *"Yes, it's SUPER slow… But that's sort of the point. It is very deliberate… it leads to code which complies with the spec far better."* OpenSpec has nothing comparable; its only delegation language is a **prohibition** (`archive-change/SKILL.md:120`: *"Do not delegate it to a background task"*).
- **Low lock-in.** Designs and plans are ordinary dated Markdown with no tool-specific semantics. Nothing merges, so nothing depends on a merge engine to stay meaningful — arguably *lower* lock-in than OpenSpec, whose delta history is only interpretable under its own merge rules.

## 6. Weaknesses / friction

**Token cost is the single dominant complaint, and it is unmeasured by anyone independent.**

- The only hard user number is issue **#1194**: a C++/Qt6 desktop app, *"68M tokens, ~3300 lines of code… also 22M tokens just for planning."* Roughly 20k tokens per shipped line, with a third of the budget spent before implementation began. Caveats: n=1, OpenCode + gpt-5.4 rather than Claude Code, Superpowers v5.x, no control run.
- Issue **#1152**: *"Codex: Implementation consumes full 5h token budget in single run"* — a regression after upgrading 4.x → 5.0.7 on a "not-so-complex repo." Issue **#743** (24 comments): *"since installation my claude code has started to slow down quite a bit. Maybe the context of the superpower skill is eating up too much space."* Issue **#2017**: *"Superpowers is fantastic but it uses a lot of tokens and takes a long time… A Slim version would be great."*
- On HN: *"it burnt through all my max plan (before it could start making any changes)"* (yard2010); *"I only found it burning too many tokens to do too little"* (smusamashah — the same commenter reports two large projects at their workplace succeeding with it, so this is a genuinely split observation, not a hostile one); *"tokenmaxxing for marginal improvements"* (newswasboring).
- **The author's own headline figures do not answer the question being asked.** "Up to 50% faster and up to 60% cheaper" is **v6 versus v5**, measured on his own eval suite — not Superpowers versus no Superpowers. The most-upvoted critique is precisely this: *"To be blunt I can't take this product seriously when they don't even run benchmarks. Your prompts make Claude better? Cool: prove it."* (johnfn). And the evals repo grades **workflow compliance against scenario criteria** — i.e. whether the agent followed the process — not output quality against a control.
- **Do not cite the circulating "9% cheaper / 14% fewer tokens" figure.** It traces to mcp.directory, which contains no experiment, self-describes as an aggregation of HN and Reddit quotes, and has a commercial interest in the unbundled skills it recommends.

**The TDD skill has a documented integrity defect, and it is the most important weakness for this project.**

- Issue **#2046** (2026-07-27): *"writing-plans can mistake structural RED for behavior-test evidence"* — an import error is treated as proof the test genuinely fails for the right reason.
- Its human-visible counterpart, from HN (steve-atx-7600): *"I ripped it out after I kept seeing Claude/codex TDD things like when I asked them to make a pydantic model immutable. I'd end up with unit tests testing that my immutably configured pydantic model was immutable, or tests that setting foo=bar was actually foo=bar in app config."*
- Both are the same failure: **the ceremony is satisfied while the test proves nothing.** A green suite of tautologies is worse than no suite, because it is evidence-shaped. On a protocol decoder — where the tests that matter are adversarial, byte-level, and negative — this is exactly the failure mode that would be most expensive here.
- The author names an adjacent one himself: *"reviewers given only the diff package produce confident spec verdicts that silently redefine 'spec' as the global constraints — 0/5 flagged the missing brief."* The review subagent can confidently approve against the wrong specification.

**Selective adoption is impossible, and this is the most-repeated structural complaint.**

- tmach32, the best-articulated critique on HN: *"I wish I could turn it on selectively. Many of my requests do not require the 'verification before completion' and TDD ceremony. For example, agents using stock Superpowers will go so far as to grep a file every time you ask to add something to them to verify that the edit really landed."*
- The bootstrap mandates invoking any applicable skill, skills auto-trigger, and there is no documented per-skill disable. On a repo like this one — mostly surveys, discussions and milestones, with a nested workspace — every conversation inherits the full gate stack. Its own Red Flags table closes the escape hatch: *"This is just a simple question" → "Questions are tasks. Check for skills."*
- mattm: *"I tried something similar to Superpowers and it went completely overboard for a small bug fix - writing a TDD, generating artifacts, etc."*

**Skills do not reliably trigger, and they contradict each other.**

- **#2051**: *"using-superpowers' 'check any skill before every action' doesn't hold once a workflow step is underway"* — re-triggering fails mid-conversation after another skill's workflow completes, despite the stated policy.
- **#1961**: the SDD task-reviewer's *"do not re-run the suite"* versus `verification-before-completion` — *"Two skills give what reads as conflicting guidance to an agent following both."*
- **#642**: `dispatching-parallel-agents` and `verification-before-completion` appear **orphaned** from the workflow graph.
- **#955**: an unquoted YAML scalar containing `: ` in a skill description causes **silent skill-discovery failure**. **#1479**: `plugin.json` was missing its `skills` field, and Claude Code could not discover any of the 14.
- Corroborating the general phenomenon from outside the project: JetBrains' controlled study of a *different* skill found **"zero times"** self-activation when the skill was merely optional rather than hook-injected. The bootstrap is not a stylistic choice; it is the only reason any of this fires.

**Planning over-specification defeats the purpose for iterative work.** tmach32 again: *"If one LLM session is responsible for writing out the entire plan, we aren't solving context rot… Implementation is an iterative process. You find things out as you go… This is why writing out a precise plan ahead of time is an issue."* Issue **#1803**, from a user reporting 90+ sessions: *"The built-in writing-plans self-review is confirmatory — it checks coverage, types, and placeholders. It does not attempt to break the plan."*

**It overrides the operator's own conventions.** Issue **#1856**: *"Your skills should not specify git workflows"* — the skills override the user's source-control instructions. **#1246**: brainstorming commits `plan.md` straight to master. **#429** (21 comments): skills lack awareness of Claude Code Agent Teams. For a repo with a carefully written `CLAUDE.md`, this is a live collision risk, mitigated but not eliminated by the stated precedence rule.

**The adoption pattern among experienced users is "fork and rewrite," not "install."** marcus_holmes: *"I found the original superpowers was a great start, but rewrote all those skills to fit my workflow… Writing skills is the new writing code."* alex_c gives the structural argument: *"Taking someone else's skills and blindly applying them to my situation feels odd. I don't know what rough edges those skills were made to address."* The 9% fork ratio and 292 derivative repos (including `superpowers-zh` at 7.5k stars and the competing `tw93/Waza` at 6.8k) are consistent with this.

**A real dissent worth weighing: capable models may not need it.** arcticfox: *"It's just not necessary with the modern models and fills up the context windows with garbage and adds an insane number of turns for no benefit. I had similar prompts back when the models were terrible at instruction-following."* losvedir compares it to elaborate `.vimrc` culture. jannyfer notes the v6 announcement's own story is *"I prompted Fable with a goal and went to sleep"* — i.e. no Superpowers loop. Against this, Syntaf reports a reversal in both directions and is the most credible pro voice: dropped it at v1 for *"higher spend, slower throughput and mediocre results,"* came back because *"6.x does feel different."*

**Project health caveats.** Bus factor 1, no CI, 94% PR rejection, 33 git tags but only 11 published Release objects, `RELEASE-NOTES.md` and the Releases API disagreeing on v6.2.0's date, and development on a `dev` branch 119 commits ahead of `main`. Opt-out telemetry (the brainstorming visual companion loads a Prime Radiant logo carrying the version; disabled via `SUPERPOWERS_DISABLE_TELEMETRY`, honours `DISABLE_TELEMETRY`) — OpenSpec has the same posture, so that one is a wash.

**The rhetorical register will collide with house instructions.** `<EXTREMELY_IMPORTANT>`, "YOU DO NOT HAVE A CHOICE", "This is not negotiable", and persuasion principles deliberately imported from Cialdini (`writing-skills/persuasion-principles.md`). It works — that is what the evals measure — but this project's `CLAUDE.md` already carries a set of invariants and a "learning is the deliverable" framing, and two loud instruction sets would now compete for attention every session.

## 7. Fit signals

**Strong fit when:**
- Code is being written in volume, and the failure you are preventing is the agent skipping tests, patching symptoms, or claiming done without checking.
- The test loop is fast enough that red-green-per-task is not painful.
- Work decomposes into independent tasks suitable for implementer/reviewer subagents, and you value spec-compliance over wall-clock.
- You want process discipline without a runtime dependency or a per-project install.
- You are willing to fork and tune the skills — the pattern its own experienced users report.

**Poor fit when:**
- The deliverable is a *document corpus* rather than code. Superpowers has no opinion about keeping a specification current, because that is not what it is for.
- You need a queryable current-state answer ("is `INSERT … SELECT` capped today?") — journal-shaped artifacts answer this badly.
- You need selective adoption. There is no supported way to take four skills out of fourteen.
- The session is mostly research, writing or decision-recording, where an implementation gate stack is pure overhead.
- Token budget is a live constraint. The evidence is contested but it runs one way: nobody reports it being cheap.
- You need the tooling to enforce. It enforces less, mechanically, than OpenSpec does.

## 8. Rust / systems-software notes

**Structurally neutral; empirically near-unevidenced.** Nothing in the skills is stack-specific — `test-driven-development` ships `writing-good-tests.md`, and the plan format names files and interfaces without caring about language. Unlike OpenSpec, whose documentation is uniformly web/CRUD in its examples, Superpowers' skills are about *process*, which travels better across domains. But travelling well in principle is not evidence.

**The evidence search returned a clear negative.** Across 102 HN comments read in full, five GitHub issue searches and every surfaced blog post: **one** systems-adjacent usage report (#1194, the C++/Qt6 app), **three** incidental Rust mentions, and **zero** mentions of C, embedded, kernel, driver, wire-protocol, codec or parser work. Every named real-world use skews web/scripting/infra: library deduplication, shell→Ansible migration, Terraform/JIRA/Snowflake, a geospatial platform, UI work, crypto fintech. Claims that Superpowers "works with Python, Go, Rust, Java, C#, PHP" come exclusively from SEO content and merely restate that Claude Code supports those languages.

**The one direct Rust datapoint is a bug.** Issue **#1577** (closed, 2026-05-18): the review loop's `cargo fmt` auto-recovery probe defaulted to **rustfmt edition 2015** and failed on `async fn`. That is thin evidence — but it proves someone ran it on async Rust, and that the tooling assumptions had not met Rust before.

Three further frictions specific to this project, flagged as reasoning rather than measurement:

1. **The red-green rhythm assumes a fast test loop.** `writing-plans` sizes steps at **2–5 minutes each**, each containing write-failing-test → run → minimal-impl → run → commit. A crate with `sqlparser` and a tokio dependency tree does not compile in that budget cold. Warm incremental `cargo test` is fine; the first run of each session and any run after a dependency change breaks the cadence. Nothing in the skills anticipates compile-time cost.
2. **`using-git-worktrees` interacts badly with Cargo by default.** Each worktree gets its own `target/`, so an isolated workspace means a full cold rebuild unless `CARGO_TARGET_DIR` is shared — and a shared target directory across concurrent worktrees serialises on Cargo's lock. Solvable by configuration, unaddressed by the skill, and discovered the expensive way.
3. **Path collision with the milestone-2 layout.** Superpowers writes to `docs/superpowers/{specs,plans}/` relative to the project root, while the crate and OpenSpec workspace live under `milestones/mysql-proxy-project/openspec-workspace/`. The paths are overridable by user preference; the default scatters artifacts across two roots. Decide before installing.

**Where it genuinely helps this project.** Milestone 1's durable conclusion was *"the tests keep the code honest; the spec keeps the tests honest,"* and *"the only two lines in either artifact with real teeth were the `proptest` and `cargo-mutants` annotations — and both point out of the spec system into Rust's toolchain."* Superpowers is the only tool surveyed so far that operates **on that side of the boundary**. The `CLAUDE.md` invariants — no panic on client input, no unbounded buffering, no change to placeholder count or result shape — all live or die on whether tests get written before the code meant to satisfy them.

**Against that, weigh #2046 honestly.** The invariants above are exactly the kind that a tautological test satisfies on paper. "No panic on client input" is a property for `proptest` and a fuzzer, not for a hand-written red-green step; a test asserting that a specific well-formed packet does not panic is structurally green and epistemically worthless. Superpowers will make tests *exist*; `cargo-mutants` is what will tell you whether they *bite*. The two are complementary, and neither substitutes for the other.

**Two hard prerequisites, currently unmet.** `cargo` and `rustc` are **not installed on this machine** (`CLAUDE.md`, Toolchain). Until `rustup` lands, the Iron Law has nothing to run and `verification-before-completion` has no command to verify with — the framework's core value is inert. Second, as with OpenSpec, there is no evidence of use on a proxy, wire protocol or performance-critical systems codebase: **you would be an early adopter either way.**

## 9. Cost & lock-in

- **Money:** Free, MIT. Enterprise support is sold separately by Prime Radiant; nothing is gated.
- **Resident token cost:** **~3,063 bytes (~800 tokens)** — `using-superpowers/SKILL.md`, injected on `startup|clear|compact`, so it is **re-paid after every compaction** — plus ~1.6 KB of skill `description:` strings as the trigger index. The author treats this as a first-class cost and has actively trimmed it (v6.1.0, "Lower Per-Session Token Cost").
- **On-demand skill cost:** 2.3 KB (`executing-plans`) to **28 KB (`subagent-driven-development`)** and 26 KB (`writing-skills`). The ones actually hit: `brainstorming` 10 KB, `systematic-debugging` 9.5 KB, `test-driven-development` 9 KB, `writing-plans` 6.9 KB. **The 14 `SKILL.md` files total 128 KB** — within 2% of OpenSpec's measured 126 KB of standing skills-and-commands scaffolding. Useful calibration: *both frameworks are prompt libraries of essentially the same weight.*
- **The real cost is the loop, not the corpus.** Implementer + reviewer + up to five re-review rounds, per task. #1194's 68M tokens for 3,300 LoC (22M of it before any code) is the only user-side measurement in existence; treat it as an upper-bound anecdote from a different harness and an older version, not a forecast. One usage-rate datapoint from HN suggests light real-world triggering in practice: *"superpowers:test-driven-development 5%, and superpowers:subagent-driven… 1%."*
- **Calibration from a rigorous adjacent study.** JetBrains ran a paired A/B of a *different* Claude skill over 80 SkillsBench tasks with Wilcoxon and sign tests: advertised −54% code reduction measured at **−15.4%**; cost −10.3% (p=0.004, significant); **no statistically significant quality difference** across 80 tasks. It does not test Superpowers, but it is the methodological bar, and it suggests skill marketing generally does not survive measurement.
- **Lock-in:** Very low. Dated Markdown with no tool-specific semantics.
- **Exit path:** `/plugin uninstall`. Nothing was written into your repo except the documents, which stand alone. What you lose is behaviour, not data.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://github.com/obra/superpowers | repo/README | 2026-08-08 | 11 harnesses + install commands, workflow chain, philosophy, telemetry disclosure, "Mandatory workflows, not suggestions" |
| 2 | https://api.github.com/repos/obra/superpowers | API | 2026-08-08 & 08-09 | Stars 269,183 → **269,185**, forks 24,038, subscribers 1,017, MIT, created 2025-10-09. Read twice, independently |
| 3 | git clone @ `44c9b2d` (main) | clone | 2026-08-08 | All byte counts, commit distribution (680 main / 798 dev / 119 ahead), 44 author identities, 33 tags |
| 4 | `skills/*/SKILL.md` (14 files) | source | 2026-08-08 | **Primary evidence for §3–§5.** Skill names, triggers, Iron Law, `<HARD-GATE>`, rationalization tables, artifact paths |
| 5 | `hooks/hooks.json`, `hooks/session-start`, `hooks/hooks-cursor.json` | source | 2026-08-08 | **Primary evidence for §4.** Sole hook is SessionStart; injects bootstrap as `additionalContext`; always exits 0 |
| 6 | `.github/` listing | source | 2026-08-08 | **No `workflows/` — no CI** |
| 7 | `CLAUDE.md` (contributor guide) | source | 2026-08-08 | 94% PR rejection; "skills are code, not prose"; divergence from Anthropic's skill guidance; zero-dependency rule; bootstrap-is-the-integration test |
| 8 | `RELEASE-NOTES.md` (91.5 KB, ~45 versions) | source | 2026-08-08 | v6.2.0 workspace + 5-round breaker; v6.1.0 token trim; v6.0.3 `git clean -fdx` hazard; the 8/10→5/10 eval revert |
| 9 | `docs/superpowers/{specs,plans}/` listing | source | 2026-08-08 | 17 specs + 11 plans committed; **no archive directory** |
| 10 | https://news.ycombinator.com/item?id=48739459 | HN | 2026-07-02 | **196 pts / 77 comments.** Primary source for most of §6. Roughly 60/40 negative |
| 11 | https://news.ycombinator.com/item?id=47623101 | HN | 2026-04-03 | 50 pts / 25 comments; "makes more mistakes when using superpowers" |
| 12 | HN Algolia search | API | 2026-08-09 | Only two Superpowers threads drew discussion; all other submissions 1–4 pts, 0 comments |
| 13 | Issues #1194, #1152, #743, #2017, #809 | GitHub | 2026-03→07 | Token burn: 68M/3,300 LoC; 5h budget in one run; context bloat; "a Slim version would be great" |
| 14 | Issues **#2046**, #1803, #895, #512 | GitHub | 2026-02→07 | **Structural-vs-behavioral RED confusion**; confirmatory (non-adversarial) plan review; plans containing full function bodies |
| 15 | Issues #2051, #1961, #642, #955, #1479 | GitHub | 2026-02→07 | Skills not re-triggering mid-workflow; skill-vs-skill conflict; orphaned skills; silent YAML discovery failure; missing `skills` field |
| 16 | Issues #1856, #1246, #429, #1799, #2040 | GitHub | 2026-02→07 | Skills override user git workflow; commits to master; no Agent Teams awareness; `$PWD` assumptions; lost executable bits |
| 17 | **Issue #1577** | GitHub | 2026-05-18 | **The only direct Rust datapoint** — review-loop `cargo fmt` probe defaulted to rustfmt edition 2015, failed on `async fn` |
| 18 | https://blog.fsck.com/2026/06/15/Superpowers-6/ | blog (author) | 2026-06-15 | "50% faster / 60% cheaper" — **v6 vs v5**, own eval suite; reviewer-with-only-diff failure 0/5; "N=5 gate battery is still owed" |
| 19 | https://blog.fsck.com/2025/10/09/superpowers/ | blog (author) | 2025-10-09 | Launch philosophy; deliberate Cialdini persuasion; two admitted incomplete parts |
| 20 | https://api.github.com/repos/prime-radiant-inc/superpowers-evals | API | 2026-08-08 | First-party eval lab; 95 stars; grades **workflow compliance**, not output quality vs control |
| 21 | https://blog.jetbrains.com/ai/2026/07/ponytail-skill-claude-tested/ | vendor research | 2026-07 | **Methodological benchmark.** 80-task paired A/B; advertised −54% measured −15.4%; no significant quality delta; "zero times" self-activation without hook injection. Tests a *different* skill |
| 22 | https://claude.com/plugins/superpowers | vendor page | 2026-08-08 | Official marketplace listing (accepted 2026-01-15); **1,009,371 installs**, vendor-reported |
| 23 | https://api.github.com/repos/obra/superpowers-skills | API | 2026-08-08 | **Archived** 2025-10-14, 737 stars — the community-editable skills experiment ended |
| 24 | `jnMetaCode/superpowers-zh` (7,553★), `tw93/Waza` (6,787★), `rihebty/flow-kit`, 292 search hits | API | 2026-08-09 | Fork-and-rewrite ecosystem; 9% fork-to-star ratio |
| 25 | https://github.com/cameronsjo/spec-compare | repo | 2026-08-09 | Serious 6-tool SDD comparison that **excludes Superpowers** — the omission is the finding |
| 26 | `Fission-AI/OpenSpec` `skills/openspec-*/SKILL.md` — all 12, grepped | source | 2026-08-09 | **Primary evidence for §12.** Zero hits for TDD / test-first / red-green / hook / worktree / subagent. Apply loop quoted in full |
| 27 | https://raw.githubusercontent.com/Fission-AI/OpenSpec/main/docs/customization.md | docs | 2026-08-09 | The `superpowers-bridge` community schema; the `anvil` "requires external CI or hooks" caveat |
| 28 | https://github.com/JiangWay/openspec-schemas | repo | 2026-08-09 | `superpowers-bridge`, 209 stars, created 2026-05-02, **last push 2026-06-10** (pre-dates OpenSpec v1.7/v1.8) |
| 29 | `.claude/skills/openspec-explore/SKILL.md` (local, generated) | source | 2026-08-09 | *"This is a stance, not a workflow"*; guardrails; the artifact-capture procedure |
| 30 | `surveys/openspec.md`, `milestones/milestone-1.md`, `discussions/milestone-1/01-sdd-tool-selection.md`, `studies/2026-08-08_openspec-vs-speckit.md` | local | 2026-08-09 | R1–R8, D1, the enforcement findings, and the measured cost figures reused in §9 and §13 |

**Sources deliberately discounted:** mcp.directory (origin of the circulating "9% cheaper / 14% fewer tokens" claim — no experiment, commercial interest in the skills it recommends), joanmedia.dev (honest synthesis but launders that figure), felipefontoura.com (author promotes his own competing `pi-sdd-kit`; concedes no hands-on Superpowers trial), docs.bswen.com (documentation synthesis, admits no field testing), awesomeclaudeskills.com (states a creation date contradicted by the API), and the general run of SEO/AI-generated listicles. A widely repeated "47 skills tested, 40 made output worse" claim **has no locatable primary source** and is not cited.

## 11. Confidence & gaps

- **Confidence: high** on the skill inventory, artifact model, enforcement mechanics, token weights and maintenance health — every claim in §3–§5 and §9's static figures comes from the cloned repo, its manifests, its hook scripts and its release notes rather than documentation *about* it. **High** on the OpenSpec side of §12, re-verified today against all 12 shipped OpenSpec skills rather than taken from the existing survey. **Medium** on §6, which rests on filed issues and one large HN thread — real evidence, but selection-biased toward problems. **Low** on Rust suitability and on runtime cost: no trial was run, and §8's frictions are reasoned from artifacts, not observed.

- **Unverified claims:**
  - **Every performance and cost figure is author-reported or n=1.** "50% faster / 60% cheaper" is v6-vs-v5 on a first-party suite; #1194's 68M tokens is one project on a different harness and an older version. **No independent controlled Superpowers-vs-baseline measurement appears to exist publicly** as of 2026-08-09.
  - **1,009,371 installs** is a vendor-page number with no disclosed methodology — the most quotable figure about Superpowers and the least verifiable.
  - **269,185 stars** was read from the API twice and corroborated by an independent growth curve, but not cross-checked against a star-history service.
  - **Reddit, Lobsters and X were not reachable** by the research tooling. The only Reddit content encountered was quoted second-hand and is not cited. A reader who wants that channel must fetch it by hand.
  - **Non-Claude harness behaviour was not tested.** Manifests exist and the bootstrap branches per harness; no install into Cursor, Codex or Copilot was performed.
  - **`superpowers-bridge` was not run.** 209 stars and real, but its last push (2026-06-10) pre-dates OpenSpec v1.7 and v1.8. Assume drift.
  - **The §8 Cargo/worktree friction is inference,** not measurement.

- **Open questions a decision-maker would still need to test hands-on:**
  1. Does the `brainstorming` gate catch a *semantically wrong requirement* the way Spec Kit's Constitution Check caught the `LIMIT 200` hazard? The gate forces a design conversation but carries no project-specific principles to check against. This is the highest-value experiment available, and it is directly comparable to a result this project already owns.
  2. Where do project invariants live once Superpowers is installed? Its own precedence rule puts `CLAUDE.md` above skills — which implies `CLAUDE.md` is the constitution, not any Superpowers artifact. Confirm the model honours that ordering under implementation pressure, given #1856.
  3. What does the red-green rhythm cost on a real Rust crate with a cold `target/`?
  4. Can four skills be adopted without the other ten? If not, is the full gate stack tolerable on a repo that is 90% documents?
  5. Does the bootstrap's instruction density degrade adherence to this project's existing `CLAUDE.md` invariants?
  6. On a decoder, does the Iron Law produce tests that *bite* — measured by `cargo-mutants` — or tests that merely exist (#2046)?

---

## 12. Head-to-head with OpenSpec

Both sides were checked against primary sources on the same day; the OpenSpec column is a fresh read of all 12 shipped skills, not a citation of the earlier survey.

| Axis | OpenSpec v1.8.0 | Superpowers v6.2.0 |
|---|---|---|
| **What it governs** | The artifacts describing *what the system does* | The agent's behaviour while *building* it |
| **Unit of work** | A *change* (proposal → deltas → tasks → archive) | A *task* (brainstorm → plan → red/green → review) |
| **Durable output** | A canonical, merged, current-truth spec corpus | A dated journal of designs and plans |
| **Lifecycle** | Delta → validate → archive → merge into canonical | Create → commit → accumulate. **No archive, no supersede** |
| **Mechanical enforcement** | A real CLI validator: structural ERRORs, exit codes, a merge engine | **One SessionStart context injector that always exits 0** |
| **Prompt-level pressure** | Low and procedural. `explore` is explicitly *"a stance, not a workflow"* | Very high and adversarial. Iron Laws, HARD-GATEs, rationalization tables |
| **Prompt text validated?** | No evidence of testing | **Yes** — public eval harness; one change reverted on a measured regression. But it grades *compliance*, not outcomes |
| **Tests** | Absent from the `apply` loop. 5 uses of "test" across 12 skills, none prescriptive | The centre of the framework — with a documented integrity defect (#2046) |
| **Subagents** | None; one explicit **prohibition** on delegating sync | A specified implementer/reviewer loop with a 5-round breaker |
| **Hooks / worktrees** | None of either | SessionStart hook; a dedicated worktree skill |
| **Elicitation before building** | `propose`: one sentence — *"Do NOT proceed without understanding what the user wants to build"* | A whole skill with a `<HARD-GATE>` — and the part users praise most |
| **Install footprint** | Node ≥ 20.19 + 12 files (~116 KB) copied into the repo, git-tracked | Plugin cache outside the repo; **zero project footprint, zero dependencies** |
| **Can ship hooks/agents/MCP** | **No** — its distribution channel is skills and commands only | Yes, and it ships one hook |
| **Standing prompt weight** | ~126 KB of skills + commands | ~128 KB of `SKILL.md` — *within 2% of each other* |
| **Resident per-session cost** | ~0 (loaded on demand) + `config.yaml` injected per artifact | ~800 tokens, re-paid after every compaction |
| **Reported runtime cost** | ~3,500–8,000 tokens of artifact per change | Contested and unmeasured independently; every report says "a lot" |
| **Portability** | 45 tools | 11 harnesses |
| **Bus factor** | 1 (TabishB 516 commits, clay-good 82) | 1 (Vincent 78.7%, Ritter 12.6%, same company) |
| **Popularity** | 64,274 stars, 352k npm downloads/week | 269,185 stars, ~1.0M reported installs, 9% fork rate |

**The overlap is two steps wide, not the whole workflow.** OpenSpec's `explore` overlaps Superpowers' `brainstorming`; OpenSpec's `propose` (design + `tasks.md`) overlaps `writing-plans`. Everything else is disjoint. And the evidence points in opposite directions on those two cells:

- On **elicitation**, Superpowers is stronger. OpenSpec's `explore` is self-described as *"a stance, not a workflow"* with an explicit list of things it does not require ("Reach a conclusion", "Produce a specific artifact"). Superpowers' `brainstorming` is a hard gate that terminates in a committed design document, and it is the single component its users praise unprompted.
- On **planning**, OpenSpec is stronger, or at least cheaper. `writing-plans` is the most-criticised skill in the corpus — plans containing full function bodies (#895), confirmatory rather than adversarial self-review (#1803), and a context-rot argument that plans written up front are wrong by the time they are executed. OpenSpec's `tasks.md` is a lighter checklist derived from deltas that nobody accuses of over-specifying.

**Everything outside those two cells is complementary, and the ecosystem noticed first.** OpenSpec's own `docs/customization.md` lists a community schema, `superpowers-bridge` (JiangWay, 209 stars), whose stated purpose is: *"Merges OpenSpec's artifact governance with obra/superpowers execution capabilities. Introduces evidence-first `retrospective` artifact addressing a gap Superpowers doesn't cover natively."* Its alternate phrasing enumerates the split precisely — Superpowers supplies *"brainstorming, writing-plans, TDD via subagents, code review, finishing"*; OpenSpec supplies *artifact governance*. An independent third party, with no stake in this survey, decomposed the two tools along exactly the line above. That is the strongest single piece of evidence here — tempered by the bridge being two OpenSpec minor versions stale.

**Three findings worth stating plainly:**

1. **Neither enforces semantics, and Superpowers enforces even less than OpenSpec.** Milestone 1 concluded this about OpenSpec and Spec Kit; it holds here with a twist. OpenSpec has a validator with exit codes; Superpowers has a hook that cannot fail. What Superpowers has instead is *evidence its prompts change behaviour* — which is, on reflection, a more honest response to the shared limitation than adding a linter that checks Markdown headings. It is also, per its critics, unproven where it matters: compliance is not quality.

2. **Superpowers restores the phase gates OpenSpec deliberately removed — but not the constitution.** Milestone 1 recorded the tension: *"'Fluid not rigid' deliberately removes the phase gates, which for R1 is precisely the anatomy you were trying to study."* Superpowers is emphatically gated: design before code, plan before implementation, failing test before production code, verification before any completion claim. But its gates are **process gates, not content gates**. Spec Kit's Constitution Check failed on *Semantic Preservation* — a project-specific principle about this project's SQL. Superpowers has no equivalent slot; its principles are universal (TDD, YAGNI, root cause, evidence). Under its own precedence rule, the constitution for a Superpowers project is `CLAUDE.md`. **The constitution gap milestone 1 identified is not closed by Superpowers.** It is closed by writing invariants where the model reads them — which this project has already done.

3. **The framework that talks about tests is the one this project's own conclusion pointed at.** Milestone 1: *"The tests keep the code honest; the spec keeps the tests honest"* and *"the only two lines with real teeth were the `proptest` and `cargo-mutants` annotations — and both point out of the spec system."* OpenSpec's apply loop is silent on tests. That silence is the hole; Superpowers is a framework built entirely inside it. The caveat from §6 travels with that: it makes tests *exist*, and only `cargo-mutants` will tell you whether they bite.

## 13. When to use which

**The decision is not either/or. It is which layer you are working in.**

| Use **OpenSpec** when | Use **Superpowers** when |
|---|---|
| The output is a durable statement of externally-observable behaviour that must still be true at change 15 | The output is code, and the risk is the agent skipping tests or patching symptoms |
| You need one current answer to "what does it do today" — the rewrite-rule catalogue, the statement allowlist, the compatibility matrix | You are implementing a task from a plan already agreed |
| The work is a *behaviour change* worth proposing, reviewing and archiving | You are debugging, and the temptation is to fix the symptom |
| You want to see the anatomy of SDD — requirement, scenario, delta, merge, archive (R1) | You are about to claim something works |
| You want artifacts a reviewer can diff in a PR | You have 2+ independent tasks and want them farmed out with review |
| The request is under-specified and you want the ambiguity captured *as an artifact* | The request is under-specified and you want the ambiguity **resolved in conversation before anything is written** |

**Use neither** when the task is a typo, a rename, or a research read. Both frameworks say so about themselves — except Superpowers, whose Red Flags table says the opposite (*"Questions are tasks"*). On that specific point its own advice should be discounted; it is the most-reported source of wasted tokens in §6.

**Use both when** you have crossed from writing specs to writing code — which is exactly where this project stands. The division the evidence supports:

```
Superpowers   brainstorming                                  ← elicitation, the praised part
     ↓
OpenSpec      proposal → delta specs → design → tasks.md     ← what and why, durable
     ↓                        (skip writing-plans)
Superpowers   worktree → red/green per task → review         ← how, disciplined
     ↓
OpenSpec      sync → archive → canonical spec                ← current truth restored
     ↓
cargo-mutants                                                ← whether any of it bit
```

Skip `writing-plans` — it duplicates `tasks.md` and it is the most-criticised skill in the corpus. Keep `brainstorming`, `test-driven-development`, `systematic-debugging`, `verification-before-completion` and `requesting-code-review`, which OpenSpec leaves entirely unaddressed. Capture brainstorming's output into the OpenSpec proposal rather than into `docs/superpowers/specs/`, so there remains exactly one place where "what the system does" is recorded. Note honestly that **this selective use is a convention you maintain by hand** — the plugin has no supported way to disable a skill, and its bootstrap tells the model to invoke any skill that might apply (#2051 shows even the tool's own triggering rule is unreliable).

### Recommendation for this project

**Keep OpenSpec as the spec layer. Do not adopt Superpowers yet. Adopt it at the moment the first Rust code is written, and not before.** Against the criteria this project has already committed to:

- **It is not tool-hopping, so R5 is intact.** R5 said "one tool, deeply, end-to-end" about *SDD tools*. Superpowers is not one — the most serious independent SDD comparison in the field omits it as out of category. Milestone 2 can run OpenSpec end-to-end and still add it later.
- **R1 favours waiting.** The deliverable is learning SDD. Fourteen auto-triggering skills would blur the attribution of every observation in milestone 2 — when the loop feels good or bad, you would not know which framework caused it. Milestone 1 already paid for the lesson that mixed premises wreck this kind of study.
- **R4 is the standing risk, and a second framework multiplies it.** Process adopted without a felt pain gets abandoned around week three. The pain that would justify Superpowers — the agent shipping untested Rust — has not happened, because there is no Rust.
- **R6 (moderate ceremony) is where it is most vulnerable.** The ceremony budget that ruled out BMAD's personas is the same budget that #6's critics say Superpowers exceeds. It is per-task, unavoidable, and cannot be partially disabled.
- **It is currently inert.** `cargo` is not installed. The Iron Law has nothing to run and `verification-before-completion` has no command to verify with. **Installing `rustup` is the real gate**, and it blocks the useful half of this decision entirely.
- **The trigger to revisit is specific:** the first time an OpenSpec `apply` produces Rust that lands without a test written first. That is not hypothetical — it is what the apply skill's own text instructs, and it is exactly the hole Superpowers fills.

Three things worth doing now, all cheap and independent of the decision:

1. **Treat `CLAUDE.md` as the constitution.** True under Superpowers' precedence rule and under OpenSpec's `config.yaml` injection, and it is the artifact that closes the constitution gap milestone 1 identified. The invariants are already written; what remains is a house rule that every proposal re-checks them, which costs nothing and requires adopting nothing.
2. **Decide artifact paths before installing** — `docs/superpowers/` at the repo root collides with the milestone-2 workspace layout (§8.3).
3. **When you do trial it, trial `brainstorming` alone first**, against the one experiment that has a known answer: does its gate catch the `LIMIT 200` semantic hazard the way Spec Kit's Constitution Check did? That is a one-session experiment, it is directly comparable to evidence this project already holds, and it tests the component with the strongest independent support rather than the machinery with the loudest complaints.
