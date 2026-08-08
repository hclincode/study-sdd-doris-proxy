# Survey: IDE-Native / Vendor-Locked SDD Environments (Kiro, Antigravity, Cursor, Cosmos)

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** agent (research subagent)
> **Scope:** SDD workflows that ship *as part of a vendor's editor/platform* rather than as a repo-level convention. Primary subject AWS Kiro; secondary Google Antigravity, Cursor plan mode, Augment Cosmos. Excludes repo-native/CLI-agnostic frameworks (OpenSpec, Spec Kit) — covered in sibling surveys.
>
> **Scope update 2026-08-08.** Project narrowed to an L7 Doris proxy doing SQL rewriting for tenant
> isolation, 1:1 backend connections, passthrough auth (`discussions/milestone-1/02-proxy-scope.md`). Verdict
> unchanged: all subjects here were **eliminated** on portability and cost grounds
> (`discussions/milestone-1/01-sdd-tool-selection.md`, R3/R8), not on domain fit. One idea remains worth
> stealing regardless of tool — **Kiro's EARS notation** (`WHEN <trigger> THE SYSTEM SHALL
> <response>`) maps onto protocol state machines and rewrite rules better than the user-story form
> either surviving candidate ships.

## 1. Snapshot

| Field | **AWS Kiro** | **Google Antigravity** | **Cursor (Plan Mode)** | **Augment Cosmos** |
|---|---|---|---|---|
| Name | Kiro | Antigravity 2.0 | Cursor Plan Mode / Plans | Cosmos (Augment Code) |
| Vendor / owner | Amazon / AWS | Google (DeepMind) | Anysphere | Augment Code |
| License | Proprietary; IDE is a **Code OSS fork**; public repo is issue-tracker only, no source [1][20] | Proprietary; Code OSS fork [20] | Proprietary; Code OSS fork [20] | Proprietary SaaS |
| First release | Preview 2025-07-14; **GA 2025-11-17** [2][3][14] | Preview 2025-11; **2.0 at I/O 2026-05-19** [15][16] | Plan Mode shipped 2025; iterated through 2026 [11][12] | Augment Code 2023-; Cosmos platform 2026 |
| Latest version (2026-08-08) | IDE GA + **Kiro CLI 3.0 (early release)**; CLI 2.0 shipped 2026-04-13 [4][5][6] | Antigravity 2.0 (desktop + `agy` CLI + SDK) [15][16] | Cursor 2.x, continuous release | Cosmos platform; **VS Code extension sunset ~2026-07** [19] |
| Popularity signal | `kirodotdev/Kiro` ≈ **4.1k stars / ≈2.6k open issues** (issue tracker, not code) [1]; heavy AWS marketing push | Replaced Gemini CLI for consumers (shutoff 2026-06-18) [16] — large captive base | Largest AI-IDE install base of the four | Enterprise-only; no public adoption numbers |
| Maintenance signal | High cadence: GA→CLI 2.0→CLI 3.0 in 9 months; **open-issue ratio is poor** (~2.6k open) [1][5][6] | High cadence but **disruptive**: killed Gemini CLI, repeatedly re-cut quotas [15][16][17] | High, stable, incremental | High cadence but **pivoting away from individual devs** [19] |
| Primary interface | IDE **+ CLI + Web + Mobile + Crew**; `.kiro/` shared across all surfaces [7] | Desktop IDE + `agy` CLI + SDK + Managed Agents API [15] | IDE only (plan mode is a chat mode) [12] | Web/platform + GitHub/GitLab/Slack/Jira/Linear triggers; no IDE [18][19] |
| Agent compatibility | Kiro's own agent only. Consumes `AGENTS.md`; **Powers use the open `agent-plugins.org` format** (Amazon/Cursor/Microsoft/OpenAI/Vercel) [8][9] | Antigravity agents only; multi-model (Gemini 3.x, Claude, GPT-OSS) [17] | Cursor agent only; supports MCP/skills/hooks [13] | Cosmos agents only |
| Language/stack neutrality | Claims neutral; **documented language guides cover only TS/JS, Python, Java — Rust absent** [10] | Neutral, web/mobile-leaning (Firebase/Android/AI Studio integrations) [15] | Neutral | Neutral; optimized for large monorepos (indexes ~500k files) [18] |
| Greenfield vs brownfield bias | **Greenfield-biased** — spec-per-feature model assumes you're adding features, weak at reconciling drift [21] | Greenfield/prototype-biased | Neutral (plan mode is task-scoped) | **Brownfield/enterprise** — built for large existing codebases |

## 2. One-paragraph thesis

Kiro's bet is that the failure mode of AI coding is not model capability but *unstated intent*: agents produce plausible code because nobody wrote down what "correct" meant. So Kiro makes the specification a first-class, on-disk, version-controlled artifact and refuses to let the agent code until you've approved it. Concretely, every feature becomes a directory of three markdown files — `requirements.md` (user stories with EARS-notation acceptance criteria), `design.md` (architecture, data models, interfaces), `tasks.md` (an ordered, dependency-aware checklist) — plus persistent project context in *steering* files and event-driven automation in *hooks*. At GA Kiro added the most interesting piece: it extracts universal properties from your EARS requirements and generates **property-based tests** to measure whether the code actually matches the spec [22][23]. The others are variations on a weaker theme: Antigravity produces plan/walkthrough *artifacts* to make agent reasoning reviewable but stores them outside your repo; Cursor's Plan Mode is a single reviewable markdown plan with no requirements layer; Augment Cosmos abandons the editor entirely for org-scale agent orchestration around tickets and PRs. Only Kiro is genuinely a *methodology* with tooling; the rest are UI affordances around agent planning.

## 3. Workflow / artifact model

**Kiro.** You create a spec from a prompt, and the agent walks a three-phase gated workflow: Requirements → Design → Tasks, with an explicit human approval between each phase. (A "Quick Spec" variant skips the gates; a "bugfix" spec swaps `requirements.md` for `bugfix.md`.) [24][25] Once `tasks.md` exists you execute tasks individually or run the whole list — Kiro builds a dependency graph and executes independent tasks concurrently in "waves" [24].

```
<repo>/
├── .kiro/
│   ├── specs/
│   │   ├── mysql-handshake/
│   │   │   ├── requirements.md      # user stories + EARS acceptance criteria
│   │   │   ├── design.md            # components, data models, interfaces
│   │   │   └── tasks.md             # ordered checklist, dependency-annotated
│   │   ├── connection-pooling/
│   │   └── query-routing/
│   ├── steering/
│   │   ├── product.md               # why the product exists (inclusion: always)
│   │   ├── tech.md                  # stack, libraries, tooling
│   │   ├── structure.md             # file layout, naming conventions
│   │   └── rust-conventions.md      # optional; YAML front-matter controls inclusion
│   ├── hooks/
│   │   └── <id>.json                # v1 schema, event-driven automation
│   └── settings/                    # MCP servers, permissions.yaml (CLI 3.0)
├── AGENTS.md                        # auto-detected, no inclusion modes
└── ~/.kiro/steering/                # global (cross-workspace) steering
```

EARS acceptance criteria take the form `WHEN <condition/event> THE SYSTEM SHALL <expected behavior>` — the documented example being *"WHEN a user submits a form with invalid data THE SYSTEM SHALL display validation errors next to the relevant fields"* [25]. There are five EARS patterns in the wild (ubiquitous / event-driven / state-driven / unwanted-behavior / optional), but Kiro's docs foreground the `WHEN…SHALL` form.

Steering files use optional YAML front-matter with four inclusion modes: `always` (default), `fileMatch` + `fileMatchPattern` (glob-conditional), `manual` (referenced as `#filename` in chat), and `auto` (agent decides from a `name`/`description`) [8]. Hooks are standalone JSON at `.kiro/hooks/<id>.json` with triggers `PostFileSave`, `PostFileCreate`, `PostFileDelete`, `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`, `PreTaskExec`, `PostTaskExec`; actions are either shell commands (session context on STDIN) or agent prompt injections; `PreToolUse`, `Stop`, and the task hooks can block [9].

**Command surface:**
- IDE: spec creation/execution via the Kiro panel; manual steering files surface as `/` slash commands; "Analyze Requirements" deep-check; "Run all tasks."
- CLI: `curl -fsSL https://cli.kiro.dev/install | bash`; `kiro-cli chat`; **`/spec new <name>`** brings the spec agent to the terminal (CLI 3.0) [6][7]; plan mode; tangent (branch a side-conversation and return).
- Headless/CI: `kiro-cli chat --no-interactive --trust-tools=read,grep "<prompt>"` with `KIRO_API_KEY` (Pro tier and above only) [5].

**Artifact lifecycle:** Specs are created per feature, approved phase-by-phase, executed task-by-task with checkbox state written back into `tasks.md`, and then… mostly left alone. **There is no delta/change-proposal model and no archive step** — a spec is a snapshot of intent at authoring time. Multiple hands-on reports call out that requirements and design artifacts do **not** auto-update as implementation diverges, producing spec↔code drift [21]. Version control is whatever git gives you: the files are plain markdown in your repo, so they diff and review like code.

**How the others differ.**
- *Antigravity*: the loop is Plan → Execute → Verify, producing **Artifacts** — task lists, `implementation_plan.md`, code diffs, browser recordings, and post-hoc `walkthrough.md` [15]. Crucially these live in `~/.gemini/antigravity/brain/<GUID>/` with `.metadata.json` sidecars and versioned backups — **not in your repo** [26]. There is no requirements layer and no EARS-equivalent.
- *Cursor*: `Shift+Tab` → Plan Mode. The agent asks clarifying questions, researches the codebase, and emits a **single markdown plan** (`*.plan.md`, rendered in a dedicated plan editor). Plans save to your home directory by default; "Save to workspace" moves them to `.cursor/plans/` [11][12]. One document, no phases, no acceptance criteria, no gate you can't skip.
- *Cosmos*: no spec artifacts at all in the Kiro sense. The unit of work is a ticket or PR; the platform's primitives are "Experts" (captured judgment made runnable), "Checkpoints" (human-in-the-loop escalation), and a Context Engine over the repo [18].

## 4. What it enforces vs. what it suggests

Read as **Kiro** unless another tool is named.

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | **Kiro: yes, inside the spec workflow** — phase gates require approval before design/tasks generation [25]. But you can drop to plain chat ("vibe") at any time, or use Quick Spec to skip gates. Nothing blocks `git commit` of unspecced code. *Antigravity/Cursor: no.* *Cosmos: no.* | That specs stay right-sized (one per feature, not monolithic) [24] |
| Spec ↔ code traceability | Weak. `tasks.md` checkboxes are updated by the executor, and tasks reference requirement IDs by convention. **No enforced link from a source file back to a requirement.** | Referencing requirement numbers in commits/PRs |
| Test/acceptance criteria | **Strongest feature.** EARS criteria are structurally required in `requirements.md`, and GA's spec-correctness feature *generates property-based tests from those criteria* and runs thousands of cases with shrinking [22][23]. Bugfix specs generate PBTs for both the fix and the preserved behavior [24]. | Which tests are "enough" |
| Review gate | Phase approvals (requirements → design → tasks) are real modal gates in the IDE flow [25]. Hooks with `PreToolUse`/`Stop`/`PreTaskExec` can *block* agent actions programmatically [9]. *Cursor's plan is reviewable but bypassable.* | Human code review of generated diffs |
| Drift detection | **Nothing.** Specs do not update as code changes; no staleness check, no reconcile command. Multiple hands-on reports name this as the top structural weakness [21]. Property-based tests are the only indirect signal that code left the spec. | "Update the spec when you change the design" |

## 5. Strengths

- **The artifacts are plain markdown in your repo.** `.kiro/specs/`, `.kiro/steering/`, `.kiro/hooks/` are ordinary files under version control. You can read, grep, diff, PR-review, and hand them to any other agent. This is the single most important fact for the adoption decision, and it is true (see §9).
- **EARS → property-based tests is a genuinely novel link.** Deriving universal properties from `WHEN…SHALL` statements and fuzzing them with shrinking is the only mechanism among these four tools that produces *machine-checkable* evidence that implementation matches specification [22][23]. Conceptually this is a strong match for protocol/state-machine work.
- **Not IDE-locked in practice.** The `.kiro/` directory is explicitly shared across IDE, CLI, Web and Mobile surfaces [7], and CLI 3.0 brings the spec agent to the terminal with `/spec new` [6]. Headless mode (`--no-interactive` + `KIRO_API_KEY`) makes specs runnable from CI [5].
- **Powers use an open, vendor-neutral plugin format.** `agent-plugins.org`, co-signed by Amazon, Cursor, Microsoft, OpenAI and Vercel; a Power is `plugin.json` + `skills/` + `mcp.json` + optional `dev.kiro/` extensions [9]. Skills authored for Kiro are portable by construction.
- **Steering inclusion modes are more expressive than a flat CLAUDE.md** — `fileMatch` globs and `auto` description-matching let you keep protocol-specific conventions out of context until relevant [8].
- **Hooks are a real automation surface**, not prompt text: shell commands on file/tool/task events with blocking semantics, and they run in both IDE and CLI [9].
- *Antigravity's* verification artifacts (browser recordings, walkthroughs) are the best "prove it worked" story of the four — irrelevant to a headless network proxy, but real.
- *Cursor's* Plan Mode is the lowest-ceremony option that still gets you a reviewable plan, and `.cursor/plans/*.plan.md` is committable.

## 6. Weaknesses / friction

- **Over-engineering is the most consistently reported failure mode.** Independent reports describe ~20 files and 1,500+ lines of console.log-riddled code for a task a human would do in ~200 lines, with the tool defaulting to "industrial, military-grade" output [21][27]. For a latency-sensitive proxy this is an active hazard, not a cosmetic one.
- **Specs go stale silently.** No auto-update, no drift detection, no delta/change-proposal model. Hands-on reviewers report requirements and design drifting from implementation and "breaking iterative flow," with the process feeling like waterfall [21].
- **Pricing history is a genuine trust problem.** Preview pricing (Free 50 / Pro $19 / Pro+ $39, interaction-based) was replaced on 2025-08-18 by a model splitting **spec requests at $0.20 and vibe requests at $0.04**, where a single task consumed several requests for "coordination." Developers reported burning a month's allocation in 15 minutes; one open-source developer's widely-circulated verdict was **"a wallet-wrecking tragedy,"** projecting ~$550/month for light use [28][29]. On 2025-08-20 AWS conceded *"we introduced a bug… some tasks are inaccurately consuming multiple requests,"* suspended charges and refunded August [28]. Earlier, in July 2025, AWS had already imposed a **waitlist and daily caps** on preview users due to demand and slow performance [30][31]. The current credit model (§9) is the third pricing scheme in twelve months.
- **Open-issue ratio is bad.** ~2.6k open issues against ~4.1k stars on the public tracker [1], and that tracker holds no source — you cannot patch anything yourself.
- **Reliability complaints persist**: "model is experiencing a high volume of traffic" errors, tasks stuck/failed/retried, context loss after failures forcing restarts, and circular debugging loops applying the same wrong fix [21].
- **Ceremony cost is front-loaded and real.** One consulting write-up budgets **2–3 weeks** for steering-file authoring and team onboarding before the workflow pays off [32]. Spec generation itself is 30–45s per feature [32]; credits per full spec cycle are unpublished.
- **Extension ecosystem is Open VSX, not the Microsoft Marketplace** [20] — fine for `rust-analyzer`, but proprietary Microsoft extensions are unavailable, and Kiro's own LSP integration has Rust bugs (§8).
- *Antigravity:* artifacts live outside the repo in a GUID-keyed brain directory [26]; quotas have been cut repeatedly (one report cites free-tier drops from 250 to 20 requests/day) [17]; and Google **shut off consumer Gemini CLI on 2026-06-18** to funnel users into Antigravity [16] — a vendor with a demonstrated willingness to kill your tool.
- *Cursor:* plan mode has no requirements layer, no acceptance criteria, no gate, no traceability. It is a better-scoped agent prompt, not SDD.
- *Cosmos:* Augment **sunset its VS Code extension** in 2026, forcing new customers onto the platform; a quoted solo developer: *"Losing the standalone VS Code plugin is a huge letdown for solo devs like me… Had to switch to Cursor for my personal projects"* [19]. It is priced and shaped for orgs, not one person on a greenfield repo.

## 7. Fit signals

**Strong fit when:**
- A team (3+) needs shared, reviewable intent artifacts and has authority to standardize on one editor.
- The work is feature-additive with stable requirements — CRUD, enterprise apps, AWS-native services.
- Acceptance criteria are naturally expressible as `WHEN…SHALL` over discrete inputs (parsers, validators, state machines) so PBT generation has something to bite on.
- You already pay AWS and want SSO/SAML, org dashboards, and consolidated billing.
- You want event-driven repo automation (hooks) without writing your own harness.

**Poor fit when:**
- **Single developer whose primary tool is Claude Code CLI** — you would pay a second subscription and switch editors to obtain an artifact model you can reproduce in-repo for free (§9).
- The project is exploratory/greenfield where requirements will be rewritten weekly — the gated phases and stale-spec problem tax every iteration.
- Minimal, performance-critical code is the goal; the over-engineering bias runs the other way.
- You need reproducible, model-pinned behavior — Kiro's model lineup and credit weights change under you.
- You need to patch tool behavior; there is no source.

## 8. Rust / systems-software notes

Honest assessment: **weakly supported, and materially worse than the marketing implies.**

- **Rust is absent from Kiro's documented language guides.** The languages-and-frameworks section covers TypeScript/JavaScript, Python and Java only; Rust receives no dedicated guidance [10]. Kiro will of course *write* Rust — the model does the work — but nothing in the product is tuned for it.
- **A concrete, open Rust bug:** `rust-analyzer` fails to load workspaces when any dependency declares `edition = "2024"`, erroring `unknown variant '2024', expected one of '2015', '2018', '2021'` — while the same project loads fine in VS Code. Reported 2026-02-09, still open, no maintainer response visible. The reporter names common crates (`time`, `base64ct`) as triggers and reports falling back to VS Code for Rust work; `cargo build/check/test` are unaffected [33]. For a tokio-based proxy in 2026 this is likely to bite on day one.
- **PBT language coverage for Rust is unconfirmed.** The spec-correctness feature is the strongest reason to consider Kiro for protocol work — MySQL wire-protocol invariants (packet round-trip, sequence-ID monotonicity, length-prefix framing, capability-flag negotiation) are textbook property targets. But Kiro's correctness docs describe the mechanism without naming supported languages or frameworks, and do **not** mention `proptest` or `quickcheck` [22]. Treat "Kiro generates Rust property tests" as **unverified** — it is the single highest-value thing to test hands-on before adopting.
- **EARS has no vocabulary for performance budgets.** `WHEN…THE SYSTEM SHALL` expresses functional behavior. Latency percentiles, allocation budgets, connection-pool saturation behavior, and backpressure semantics have to be smuggled into prose. Nothing in the tool tracks them.
- **Concurrency invariants are unaddressed.** No documented support for reasoning about `Send`/`Sync`, cancellation-safety in `tokio::select!`, or loom-style interleaving testing. Design docs are free-form markdown; the tool does not know what an async runtime is.
- **No mutation-testing story.** `cargo-mutants` (already anticipated in this repo's `.gitignore`) has no integration; you'd wire it via a hook shell command, which is straightforward but entirely on you.
- **Antigravity's verification artifacts are web-shaped** — browser recordings and screenshots are its proof mechanism, worthless for a headless TCP proxy. **Unknown** how well its verification loop maps to `cargo test` output.
- *Cursor* is the most Rust-comfortable of the four in day-to-day editing (largest extension/LSP maturity), but offers the least SDD structure.

## 9. Cost & lock-in

- **Money (Kiro, current):** Free $0 / 50 credits per month (Claude Sonnet 4.5 + open-weight models); Pro **$20**/1,000 credits; Pro+ **$40**/2,000; Pro Max **$100**/5,000; Power **$200**/10,000. Overage add-on credits **$0.04 each**. Team plans are the same per-seat prices plus consolidated billing, usage analytics, SAML/SCIM SSO and org dashboards. A credit is "a unit of work in response to user prompts," billed to 0.01 granularity; complex spec execution consumes many credits per prompt, and model choice changes the rate (Sonnet 4.6 ≈ 1.3× Auto) [34]. Note this is the **third** pricing model since July 2025 [28][29].
  - *Antigravity:* free in public preview with rate limits; higher limits bundled into Google AI Pro/Ultra subscriptions; **no stable standalone paid tier published** — treat pricing as provisional [17].
  - *Cursor:* Hobby free; Pro $20/mo; Pro+ and Ultra as usage multiples; Teams $40/user/mo; Enterprise custom [13].
  - *Cosmos:* enterprise-shaped. Reported figures conflict — one source cites Standard ≈$60/dev/mo with 130k pooled credits, another a Business plan at $100/mo for up to 50 seats [18][19]. **Unverified**; assume you must talk to sales.
- **Token cost:** Not directly measurable — Kiro bills credits, not tokens, and does not publish credits-per-spec. Structurally, a full three-phase spec cycle costs *considerably* more than a single prompt: requirements draft + your revisions, design draft + revisions, task generation, then per-task execution with review. One blog estimate puts the 50-credit free tier at roughly 2–3 complete spec-driven features [35] — i.e. **~15–25 credits per feature**, or roughly $0.30–$0.50 of Pro-tier allowance per feature *before* implementation credits. Treat that number as indicative only. The historical spec-vs-vibe split ($0.20 vs $0.04) is direct evidence that AWS itself prices spec work at ~5× ordinary chat [28].
- **Lock-in — the decisive question, answered:**
  - **Kiro: LOW for the artifacts, MEDIUM for the workflow.** `.kiro/specs/**/*.md`, `.kiro/steering/*.md` and `AGENTS.md` are plain markdown committed to your repo. Stop paying AWS tomorrow and you keep every requirement, design and task list, fully readable by Claude Code with zero conversion. `.kiro/hooks/*.json` is a documented v1 schema you'd have to re-express as Claude Code hooks. Powers follow the open `agent-plugins.org` spec and are portable by design [9].
  - **What you actually lose on exit:** the phase-gate state machine, the task dependency-wave executor, the "Analyze Requirements" consistency checker, and — most significantly — **spec-correctness property-based test generation**, which has no open equivalent. Those are runtime features, not file formats.
  - **Yes, you can use Kiro specs from Claude Code.** This is empirically settled: `gotalab/cc-sdd` (≈3.6k stars) reimplements the requirements→design→tasks→implementation flow with approval gates as Agent Skills across 8 agents (Claude Code and Codex marked stable), **defaults to writing into `.kiro/`**, and states plainly that *"Existing Kiro specs remain compatible and portable"* [36]. Several smaller projects (`mimicode25/cc-sdd-spec`, `ShortArrow/ccsdd`, `angelsen/claude-kiro`) do the same [37]. The practical consequence for this decision: **you can adopt Kiro's methodology without adopting Kiro.**
  - **Antigravity: HIGH.** Implementation plans and walkthroughs live in `~/.gemini/antigravity/brain/<GUID>/` as markdown plus `.metadata.json`, keyed by opaque GUIDs and outside your repository — a third party had to build an unofficial CLI just to read and edit them without the IDE [26]. Nothing is in git unless you copy it out by hand. Combined with Google's demonstrated willingness to shut down Gemini CLI [16], this is the worst lock-in profile of the four.
  - **Cursor: LOW but trivial.** `.cursor/plans/*.plan.md` is committable markdown — but note plans default to your **home directory**, so the portable outcome requires an explicit "Save to workspace" click each time [11][12]. There's little to lock in because there's little there.
  - **Cosmos: HIGH.** The value is a hosted Context Engine, org-scale orchestration and integration wiring. None of it is a file in your repo. Augment has already deleted a delivery surface its users depended on [19].
- **Exit path (Kiro):** `git rm -r --cached .kiro/hooks` and keep the rest; point Claude Code at `.kiro/specs/` and `.kiro/steering/` directly, or install `cc-sdd` to regain the gated workflow as slash commands/skills. Estimated cost: **hours, not weeks.** Migrating *off Antigravity* means manually exporting artifacts from `~/.gemini/antigravity/brain/` — days, and lossy.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://github.com/kirodotdev/Kiro | repo | fetched 2026-08-08 | Issue tracker only, no source. ~4.1k stars, ~2.6k open issues. License not stated on repo page. |
| 2 | https://aws.amazon.com/blogs/aws/aws-weekly-roundup-how-to-join-aws-reinvent-2025-plus-kiro-ga-and-lots-of-launches-nov-24-2025 | vendor blog | 2025-11-24 | Confirms GA "last week"; GA shipped PBT spec-correctness, checkpointing, Kiro CLI, enterprise team plans. |
| 3 | https://www.ciol.com/generative-ai/kiro-reaches-ga-with-property-based-tests-cli-and-team-controls-10799180 | press | 2025-11 | GA feature summary. |
| 4 | https://kiro.dev/docs/cli/ | docs | fetched 2026-08-08 | CLI install `curl -fsSL https://cli.kiro.dev/install \| bash`; steering, hooks, custom agents, MCP, permissions. |
| 5 | https://kiro.dev/docs/cli/headless/ | docs | fetched 2026-08-08 | `KIRO_API_KEY`, `--no-interactive`, `--trust-tools`, Pro+ only, admin governance applies. |
| 6 | https://kiro.dev/docs/cli/v3/ | docs | page updated 2026-08-05 | CLI v3 early release: Spec agent in terminal, `permissions.yaml`, standalone JSON hooks, tangent, plan mode; `.kiro` portable across IDE/CLI/Web. |
| 7 | https://kiro.dev/docs/ | docs | fetched 2026-08-08 | Surfaces: IDE, CLI, Web, Mobile, Crew. "Your `.kiro/` configuration is shared across all of them." |
| 8 | https://kiro.dev/docs/steering/ | docs | fetched 2026-08-08 | `.kiro/steering/` + `~/.kiro/steering/`; product/tech/structure.md; inclusion always/fileMatch/manual/auto; AGENTS.md auto-detected. |
| 9 | https://kiro.dev/docs/hooks/ and https://kiro.dev/docs/powers/ | docs | fetched 2026-08-08 | `.kiro/hooks/<id>.json` v1 schema, 9 trigger types, blocking hooks. Powers = `agent-plugins.org` open format (Amazon/Cursor/Microsoft/OpenAI/Vercel). |
| 10 | https://kiro.dev/docs/guides/languages-and-frameworks/ | docs | fetched 2026-08-08 | Documented guides: TS/JS, Python, Java. **Rust absent.** |
| 11 | https://cursor.com/blog/plan-mode | vendor blog | 2025 | Plans are markdown; "Save to workspace" → `.cursor/plans/`; `*.plan.md` opens in plan editor. |
| 12 | https://cursor.com/docs/agent/plan-mode | docs | fetched 2026-08-08 | "Plans are saved by default in your home directory." Persist across sessions. No requirements/acceptance-criteria structure. |
| 13 | https://cursor.com/pricing | vendor | fetched 2026-08-08 | Hobby free; Pro $20; Pro+/Ultra multiples; Teams $40/user; Enterprise custom. |
| 14 | https://caylent.com/blog/kiro-first-impressions | hands-on | 2025-07-14 | Preview-era hands-on; 30–45s spec generation; 2–3 week steering/onboarding investment; rigidity for prototyping. |
| 15 | https://www.marktechpost.com/2026/05/19/google-launches-antigravity-2-0-at-i-o-2026-a-standalone-agent-first-platform-with-cli-sdk-managed-execution-and-enterprise-support/ | press | 2026-05-19 | Antigravity 2.0: desktop + CLI + SDK + Managed Agents API + Enterprise. |
| 16 | https://jangwook.net/en/blog/en/google-io-2026-antigravity-2-agent-platform-analysis/ | analysis | 2026-05 | `agy` CLI (Go) replaces Gemini CLI; consumer Gemini CLI access ends 2026-06-18. |
| 17 | https://antigravity.google/blog/changes-to-antigravity-plans + https://blog.google/feed/new-antigravity-rate-limits-pro-ultra-subsribers/ | vendor | 2026 | Free preview with rate limits; Pro/Ultra higher quotas; repeated quota changes; models incl. Gemini 3.x, Claude Sonnet/Opus 4.6, GPT-OSS 120B. |
| 18 | https://www.augmentcode.com/ | vendor | fetched 2026-08-08 | Cosmos = agent orchestration; Experts, Checkpoints, Context Engine; GitHub/GitLab/Slack/Linear/Jira triggers; no IDE extension mentioned. |
| 19 | http://www.logicgrad.com/articles/augment-code-review-2026-cosmos-platform-and-the-end-of-the-ide-extension | review | 2026 | VS Code extension sunset; new customers must adopt Cosmos; solo-dev complaint quote; Business $100/mo up to 50 seats. |
| 20 | https://visualstudiomagazine.com/articles/2025/07/21/forked-again-awss-kiro-latest-ai-assistant-based-on-vs-code.aspx + https://www.vgtc.io/insights/vs-code-forks-ide-landscape-2026-h1 | press | 2025-07 / 2026 | Kiro is a Code OSS fork; uses Open VSX not MS Marketplace; AWS privacy policy governs telemetry. |
| 21 | https://dev.to/aws-builders/brilliant-broken-and-frustrating-my-deep-dive-into-amazons-kiro-ai-ide-the-flawed-junior-gn5 | hands-on | 2025-10-24 | 20 files/1,500 lines for a ~200-line task; traffic errors 1–2×/week; stuck tasks; context loss; specs don't auto-update → drift; "waterfall" feel. |
| 22 | https://kiro.dev/docs/specs/correctness/ | docs | fetched 2026-08-08 | PBT extracted from EARS requirements; hundreds/thousands of random cases. **Does not name supported languages or frameworks.** |
| 23 | https://kiro.dev/blog/property-based-testing/ + https://kiro.dev/blog/property-based-testing-fixed-security-bug/ | vendor blog | 2025–2026 | Property extraction, shrinking to minimal counterexamples, "does your code match your spec." |
| 24 | https://kiro.dev/docs/specs/ + https://kiro.dev/docs/specs/best-practices/ | docs | fetched 2026-08-08 | Three files; `bugfix.md` variant; `.kiro/specs/<feature-name>/`; dependency graph → concurrent "waves"; PBT for bugfixes; "Analyze Requirements". |
| 25 | https://kiro.dev/docs/specs/feature-specs/ | docs | fetched 2026-08-08 | EARS `WHEN … THE SYSTEM SHALL …` with worked example; approval gates between phases; Quick Spec bypasses gates. |
| 26 | https://github.com/michaelw9999/antigravity-cli | third-party repo | 2026 | **Unofficial.** Artifacts at `~/.gemini/antigravity/brain/<GUID>/` as `task.md`, `implementation_plan.md`, `walkthrough.md` + `.metadata.json`, with versioned backups. |
| 27 | https://medium.com/code-art/kiro-cli-config-is-over-engineered-and-im-the-one-who-built-it-cae868f90ce7 | practitioner | 2026-06 | Author of a Kiro config system calls it "a monument to over-engineering." |
| 28 | https://www.theregister.com/2025/08/18/aws_updated_kiro_pricing/ | press | 2025-08-18/20 | Spec $0.20 vs vibe $0.04 requests; "wallet-wrecking tragedy" (Antonio Ribeiro), ~$550/mo estimate; AWS admits metering bug 2025-08-20, refunds August. |
| 29 | https://news.ycombinator.com/item?id=44942600 | community | 2025-08 | HN thread on the pricing change. |
| 30 | https://www.theregister.com/2025/07/21/aws_kiro_usage_cap/ | press | 2025-07-21 | Preview waitlist + daily usage caps; slow-performance complaints. |
| 31 | https://www.infoworld.com/article/4026642/aws-imposes-caps-on-kiro-usage-introduces-waitlist-for-new-users.html | press | 2025-07 | Same event, independent coverage. |
| 32 | https://caylent.com/blog/kiro-first-impressions | hands-on | 2025-07-14 | 2–3 week steering/training investment; hook latency 2s–2min. |
| 33 | https://github.com/kirodotdev/Kiro/issues/5508 | issue | opened 2026-02-09, still open | rust-analyzer fails on dependencies with `edition = "2024"`; works in VS Code; no maintainer response. |
| 34 | https://kiro.dev/pricing/ | vendor | fetched 2026-08-08 | Free/50, Pro $20/1,000, Pro+ $40/2,000, Pro Max $100/5,000, Power $200/10,000; $0.04 overage; credit definition; Sonnet 4.6 ≈1.3× Auto. |
| 35 | https://pingax.com/kiro-pricing-free-tier/ | third-party blog | 2026 | Estimate that 50 free credits ≈ 2–3 full spec-driven features. Low-authority. |
| 36 | https://github.com/gotalab/cc-sdd | repo | fetched 2026-08-08 | ≈3.6k stars, 429 commits, 17 skills, 8 agents (Claude Code + Codex stable), defaults to `.kiro/`, `npx cc-sdd@latest`. States "Existing Kiro specs remain compatible and portable." |
| 37 | https://github.com/mimicode25/cc-sdd-spec, https://github.com/ShortArrow/ccsdd, https://github.com/angelsen/claude-kiro | repos | 2025–2026 | Independent Kiro-compatible SDD ports to Claude Code. |

## 11. Confidence & gaps

- **Confidence: medium-high** on Kiro's artifact model, on-disk layout, surfaces, pricing, and lock-in profile — all drawn from vendor primary docs fetched on the survey date and cross-checked against three independent hands-on reports and contemporaneous press. **Medium** on Antigravity and Cosmos (fewer primary sources; both are moving fast). **Medium-low** on anything quantitative about credit consumption.

- **Unverified claims — treat as open:**
  - **Whether Kiro's property-based test generation supports Rust (`proptest`/`quickcheck`) at all.** The correctness docs name no languages [22]; Rust is absent from language guides [10]. This is the highest-stakes unknown for this decision, since PBT is Kiro's main differentiator and protocol invariants are its best use case.
  - **Credits consumed per spec cycle.** No vendor figure exists. The "50 credits ≈ 2–3 features" estimate is a third-party blog [35], not measured.
  - **Antigravity artifact storage path** (`~/.gemini/antigravity/brain/<GUID>/`) comes from an unofficial third-party CLI [26], not Google docs; Google's own artifacts page declines to state where artifacts live. The *direction* (outside the repo) is corroborated by the tool's existence; the exact path is not officially confirmed.
  - **Kiro's GA date.** 2025-11-17 is reported by multiple secondary sources and consistent with AWS's 2025-11-24 "last week" roundup [2], but I did not find an AWS page stating the date outright. Several search summaries asserted March or May 2026 GA dates; those appear to be confabulations and are rejected.
  - **Cosmos pricing.** Two sources give incompatible figures ($60/dev/mo vs $100/mo for 50 seats) [18][19]; no vendor pricing page was retrievable.
  - **`kirodotdev/Kiro` license.** No LICENSE visible on the repo page; the IDE is proprietary regardless.
  - **Whether Kiro CLI 3.0's spec agent has full parity with the IDE's phase gates and "Analyze Requirements."** The v3 page advertises the Spec agent; a parity table was not found.
  - Whether Kiro's model roster is still Bedrock/Claude-exclusive. Pricing lists Claude Sonnet 4.5/4.6 plus unnamed open-weight models [34]; older analyses assert Bedrock-only. Not re-confirmed.

- **Open questions a decision-maker must test hands-on:**
  1. Point Kiro at a small tokio + `bytes` crate implementing MySQL packet framing. Does spec-correctness emit runnable `proptest` code, or nothing? **This alone decides whether Kiro has any unique value here.**
  2. Does Kiro issue #5508 (rust-analyzer / edition 2024) reproduce on a current `tokio` + `time` dependency tree? If yes, the IDE is unusable as an editor for this project today.
  3. Measure credits for one real feature spec end-to-end (requirements → design → tasks → execute) and extrapolate monthly cost against the $20 tier.
  4. Take a Kiro-generated `.kiro/specs/<feature>/` directory, delete Kiro, and drive it from Claude Code with `cc-sdd` installed. Confirm the round-trip is genuinely lossless — this is the cheap experiment that captures most of Kiro's value at zero subscription cost.
  5. Test the drift case explicitly: implement a feature, then hand-change the design, and see whether any tool surface notices.
