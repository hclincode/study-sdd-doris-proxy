# Survey: Agent-Role / Multi-Persona SDD Frameworks (BMAD-METHOD, Agent OS, CCPM, GSD)

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** agent (research subagent)
> **Scope:** Frameworks whose central design idea is *simulating a software team* — named agent roles, handoffs, and agile ceremony — evaluated for a solo/very-small-team greenfield Rust project (Apache Doris proxy, MySQL wire protocol, async tokio) driven from Claude Code. Primary subject: **BMAD-METHOD**. Secondary: **Agent OS**, **CCPM**, **GSD**. Deliberately excludes Spec Kit, OpenSpec, and Kiro (covered by sibling surveys) except as comparison anchors.
>
> **Scope update 2026-08-08.** Project narrowed to an L7 Doris proxy doing SQL rewriting for tenant
> isolation, 1:1 backend connections, passthrough auth (`discussions/milestone-1/02-proxy-scope.md`). This does
> not change the survey's verdict: these frameworks were already **eliminated** by the user's stated
> moderate ceremony budget (`discussions/milestone-1/01-sdd-tool-selection.md`, R6). The narrower scope makes
> multi-persona simulation less justifiable still — there is no product-discovery work here, since
> the requirements are dictated by an external protocol.

## 1. Snapshot

| Field | **BMAD-METHOD** | **Agent OS** | **CCPM** | **GSD (GSD Core)** |
|---|---|---|---|---|
| Name | BMAD-METHOD ("Breakthrough Method for Agile AI-Driven Development"; docs now also gloss it "Build More, Architect Dreams") | Agent OS | CCPM (Claude Code PM) | GSD — "Get Shit Done", now *GSD Core* |
| Vendor / owner | Brian Madison / BMad Code, LLC (`bmad-code-org`) | Brian Casel / Builder Methods (`buildermethods`) | Automaze (`automazeio`) | Originally TÂCHES (`gsd-build`); **community fork** `open-gsd/gsd-core`, maintainer "Trek-e" |
| License | MIT | MIT | MIT | MIT |
| First release | 2025 (v1–v4 era 2025; v5 late 2025) | 2025 | 2025 | 2025 |
| Latest version (as of 2026-08-08) | **v6.10.0** (2026-07-03) | **v3.x** (v3.0.0 Jan 2026) | **v2** (skills rewrite; v1 slash commands archived on `v1` branch) | **v1.7.0** (fork line) |
| Popularity signal | **51.6k stars / 5.9k forks / ~2,021 commits** — by far the largest | 5.2k stars | 8.3k stars / 832 forks | 7.9k stars / 547 forks on the fork (upstream repo peaked ~54k stars before being locked) |
| Maintenance signal | Very healthy: near-monthly minor releases v6.7→v6.10 (May–Jul 2026), near-daily commits, creator-led. Issue triage is uneven — substantive user feedback sits unanswered (e.g. #446) | Healthy but **shrinking on purpose**: v3 *removed* features | Active; recently re-platformed onto Agent Skills | **Governance red flag**: original maintainer unreachable since ~Apr 2026, associated `$GSD` token publicly linked to a rug-pull; upstream repo locked, development continues only via community fork |
| Primary interface | Slash commands installed into the agent + npx installer (`npx bmad-method install`) | 5 slash commands + markdown standards files | Agent Skill (natural-language triggers), GitHub Issues as the DB | Slash commands (`/gsd-new-project`, `/gsd-onboard`) + npx installer |
| Agent compatibility | Claude Code, Cursor, Codex CLI, others; plus web bundles for Gemini Gems / ChatGPT Custom GPTs | Claude Code (primary), Cursor, Windsurf, Codex, "any agent that reads files" | Claude Code, Codex, OpenCode, Factory, Amp, Cursor (any Agent-Skills harness) | 15+ runtimes: Claude Code, Codex, Copilot, Cursor, Windsurf, OpenCode, Antigravity CLI, Kimi CLI, Kilo |
| Language/stack neutrality | Nominally neutral; content and examples skew web/product/SaaS. Dedicated modules exist for *game dev*, not systems | Neutral by construction (it documents *your* standards) | Neutral; the opinionation is about GitHub, not language | Neutral |
| Greenfield vs brownfield bias | **Greenfield-biased.** Brownfield path exists but users report friction (see §6) | **Brownfield-biased** — `/discover-standards` mines an existing codebase; weakest on day 0 of an empty repo | Greenfield-friendly; assumes a GitHub repo exists | Both (`/gsd-new-project` vs `/gsd-onboard`) |

## 2. One-paragraph thesis

**BMAD's bet is that the reason AI coding fails at scale is not model capability but *missing process*, and that the cheapest way to install process is to simulate the roles that produce it in a human org.** You install ~12 markdown-defined personas — Analyst, PM, Architect, UX, Scrum Master, Developer, QA/Test Architect, Technical Writer, plus orchestrators — and run a two-phase loop: a **Planning phase** where personas hand off artifacts to each other (brief → PRD → architecture → epics → stories), and a **Development cycle** where a Scrum Master shards the plan into self-contained story files, a Dev agent implements one story per fresh context, and a QA/review agent verifies before the next story starts. The marketing term for the second half is *"context-engineered development"*: the story file is deliberately built to carry everything the Dev agent needs so the implementation session never has to hold the whole PRD. That is the genuinely good idea inside BMAD, and it is separable from the personas. The three secondary tools each keep one piece and drop the rest: **Agent OS** keeps only durable codebase *standards* and explicitly deleted its orchestration and persona layers in v3; **CCPM** keeps the PRD→epic→task decomposition but externalizes state to GitHub Issues so parallel agents (in git worktrees) share a real database instead of a chat log; **GSD** keeps fresh-context subagent orchestration and a Discuss→Plan→Execute→Verify→Ship loop while explicitly *rejecting* sprint ceremony, story points, and Jira-shaped workflow.

## 3. Workflow / artifact model

**BMAD (v6.10) — four phases, scale-adaptive.** v6's headline change is that you no longer run the full pipeline by default. A routing step assigns the work a **level 0–4**: levels 0–1 ("Quick Flow") produce a tech-spec only, no PRD, 1–2 stories; level 2+ pulls in full PRD and architecture; levels 3–4 add per-epic just-in-time solutioning and formal gates.

| Phase | Workflows | Output |
|---|---|---|
| 1. Analysis *(optional)* | `bmad-brainstorming`, `bmad-forge-idea`, `bmad-deep-recon`, `bmad-product-brief`, `bmad-prfaq` | `brainstorm.html`, `forge-report.html`, `brief.md`, `addendum.md` |
| 2. Planning | `bmad-prd`, `bmad-ux`, `bmad-spec` | `prd.md`, `DESIGN.md`, `SPEC.md`, `stories.yaml`, `.memlog.md` |
| 3. Solutioning | `bmad-architecture`, `bmad-create-epics-and-stories`, `bmad-sprint-planning` | `ARCHITECTURE-SPINE.md`, epic files with stories, `sprint-status.yaml` + PASS/CONCERNS/FAIL gate |
| 4. Implementation | `bmad-build`, `bmad-code-review`, `bmad-correct-course`, `bmad-retrospective` | `spec-*.md` + code, review findings, retro doc |

Artifacts land in two places — the *installed framework* under `.bmad/`, and the *project outputs* under `docs/`:

```
.bmad/                              # installed framework (not your content)
├── _cfg/
│   ├── config.yaml
│   └── agents/bmm-pm.customize.yaml
├── bmm/                            # BMad Method core module
│   ├── agents/                     # persona definitions (markdown "agent-as-code")
│   ├── docs/
│   └── workflows/
│       └── create-story/
│           ├── workflow.md
│           └── steps/              # v6 "step files" — loaded one at a time
│               ├── step-01-load-context.md
│               └── … step-06
├── bmb/                            # BMad Builder (author your own agents/workflows)
└── tea/ bmgd/ cis/ …               # Test Architect, Game Dev, Creative Intelligence

docs/                               # your project's artifacts
├── project-context.md              # "constitution" — seeded from architecture, reused everywhere
├── prd.md
├── ARCHITECTURE-SPINE.md
├── epics/
│   └── epic-1-<theme>/
│       ├── epic-1.md
│       ├── epic-1-context.md       # cached compiled context for dev
│       └── stories/
│           └── 1.1.story.md        # sharded, self-contained story = one dev session
└── sprint-status.yaml
```

**Command surface (BMAD):** installed as slash commands under a multi-level namespace, e.g. `/bmad-bmm-create-story`, `/bmad-build`, `/bmad-code-review`, `/bmad-party-mode` (all relevant personas respond in character to one prompt — used as a pre-implementation design review), `/bmad-help` (reads project state and tells you what to do next). Installation: `npx bmad-method install`; requires Node 20.12+, Python 3.10+, and `uv`.

**Artifact lifecycle (BMAD):** create → human-approve at gates → shard into stories → implement one story per session → review → retrospective. Stories carry a status field (`Draft` → ready → done). **There is no delta model** — BMAD rewrites and versions whole documents; drift between `prd.md` and the code is caught only by a human or by a review workflow being run deliberately. Everything is markdown/YAML in-repo, so git is the version-control story.

**Where the others differ:**

- **Agent OS v3** — five commands, no pipeline: `/discover-standards` (mine your codebase into standards), `/inject-standards` (pull the relevant ones into context, routed by a generated `index.yml`), `/shape-spec` (standards-aware questioning that then hands off to **Claude Code's native plan mode**), a sync script to push standards back to a base profile, plus light mission/roadmap/tech-stack planning. Artifacts live in `agent-os/` and `commands/agent-os/`. v3 **retired** v2's spec-writing, task-breakdown, and implementation-orchestration phases and no longer installs subagents.
- **CCPM** — `PRD → epic → task decomposition → GitHub sync → parallel execution`. Local markdown under `.claude/epics/`, but **GitHub Issues is the source of truth** for state (deliberately using Issues, not the Projects API). Requires `git`, authenticated `gh`, and the `gh-sub-issue` extension for real issue hierarchy. Ships four specialist agents — `code-analyzer`, `file-analyzer`, `test-runner`, `parallel-worker` — that are *context-reduction* workers, not role-play personas. v2 moved from `/pm:*` slash commands to natural-language Skill triggers ("break down the X epic", "sync the X epic to GitHub", "standup").
- **GSD Core** — `/gsd-new-project` or `/gsd-onboard`, then a per-milestone loop: **Discuss → Plan → Execute → Verify → Ship**. Persistent artifacts `STATE.md`, `CONTEXT.md`, `continue-here.md` survive session boundaries; research and execution run in fresh-context subagents ("parallel execution waves", each executor starting on a clean ~200k window); `Ship` opens a PR and archives the phase.

## 4. What it enforces vs. what it suggests

Read as **BMAD** unless a row names another tool. Across all four, "enforced" is thin — these are prompt/markdown systems, not compilers.

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | **Weak.** Workflows are ordered and `/bmad-help` reads project state to route you, but nothing prevents invoking `bmad-build` directly — and levels 0–1 intentionally skip the PRD. **CCPM** is the strongest here: a task must exist as a GitHub Issue to be worked, which is a real external gate. **Agent OS v3 enforces nothing** by design | BMAD: the persona sequence itself is the suggestion. GSD: the Discuss/Plan phases are conventions the agent is prompted to honor |
| Spec ↔ code traceability | **CCPM:** genuine — commits and issue comments tie code to an issue ID, visible outside the chat. **BMAD:** story files reference epic/PRD IDs by filename convention only; nothing verifies the link | BMAD story front-matter, epic numbering, `sprint-status.yaml` |
| Test/acceptance criteria | Not enforced. **BMAD** ships a whole Test Architect (TEA) module for risk-based test strategy, and `dev-story` is documented to run `create-story → test-plan → dev-story → test-review → code-review → PR`, but running it is your choice. **CCPM** ships a `test-runner` agent | Acceptance criteria are a template field in the story; the Dev agent is *asked* to satisfy them |
| Review gate | `bmad-sprint-planning` emits a real PASS/CONCERNS/FAIL readiness verdict, and `/bmad-party-mode` + adversarial code review are the most-praised parts of BMAD — but nothing halts on FAIL except you. **CCPM** inherits GitHub PR review, which *is* enforceable via branch protection | BMAD story status transitions (`Draft` → approved) are advisory; one user found **no command exists for the Scrum Master to advance a story out of Draft** |
| Drift detection | **None in any of the four.** No delta/diff model, no "spec says X, code does Y" check. `bmad-correct-course` handles *known* mid-sprint changes; it does not *detect* drift | Retrospectives and re-running planning workflows |

## 5. Strengths

- **Story sharding + fresh context per story is the real, transferable idea.** A story file compiled to be self-sufficient means the implementation session never carries the PRD. This is the same insight behind GSD's fresh-context executors and CCPM's `file-analyzer`/`code-analyzer` workers, and it works regardless of whether you keep the personas.
- **v6's step-file architecture is a genuine engineering fix, not a rebrand.** Workflows were split so each step loads ~2–3k tokens instead of a ~15k monolithic instruction file, and documents are sharded for selective loading. One user-reported measurement: 31,667 → 8,333 tokens (74%) on a comparable task. (Self-reported; see §11.)
- **Adversarial review and Party Mode earn their keep.** The one independent head-to-head test of BMAD v6.0.3 vs Spec-Kit vs OpenSpec found BMAD scored **5/5 on iterative refinement and mid-feature course correction**, the best of the three, and that adversarial review caught design issues before implementation. If you want a second opinion on a design and you have no colleague to ask, this is the closest simulation available.
- **Scale-adaptive levels 0–4 are a real concession to the ceremony problem.** BMAD Quick Flow (levels 0–1) is two workflow steps and one document — competitive with lightweight tools on friction while retaining BMAD's elicitation quality.
- **Largest ecosystem by a wide margin.** 51.6k stars, active Discord, monthly releases, a Builder module for authoring your own agents, and web bundles that let you run the token-heavy planning phase on a flat-rate Gemini/ChatGPT subscription instead of metered API tokens.
- **Everything is plain markdown/YAML in your repo.** No database, no SaaS, no proprietary format.
- *(CCPM)* **State lives outside the chat log.** GitHub Issues as the store is the one design here that survives you closing the terminal, and it makes parallel worktree execution coherent rather than chaotic.
- *(GSD)* **Explicit anti-ceremony stance with the orchestration retained** — the fresh-context/verify machinery without PRDs, story points, or standups.
- *(Agent OS)* **The most intellectually honest of the four:** it audited its own value and deleted the parts frontier models made redundant.

## 6. Weaknesses / friction

- **Ceremony cost, measured.** The only independent side-by-side benchmark found: **BMAD Full = 6 days (2 planning + 4 implementation) and ~$200** for a feature; **BMAD Quick = 6.5 hours (5h planning + 1.5h implementation) and ~$85**; competing tools finished comparable work in 1–2 days. **OpenSpec won that test overall at 4.0/5.** The 30× wall-clock gap between BMAD Full and BMAD Quick is the single most decision-relevant number in this survey — and it is BMAD *versus itself*.
- **The complexity is self-acknowledged.** From the same test: *"BMAD Full scores high on visibility because `/bmad-help` reads your project state and tells you what to do next. But it mostly exists to rescue you from BMAD's own complexity."* A help command whose purpose is navigating your own framework is a design smell.
- **Handoff drift is a new failure surface you didn't have before.** Personas don't share state; an undocumented assumption made by the PM persona propagates through Architect → SM → Dev unchallenged. More agents ⇒ more per-cycle tokens *and* more places to debug.
- **The human becomes the workflow engine.** A detailed solo-developer report on a real brownfield task found: agents have no awareness of each other so the developer manually relays context; the Dev agent picked story 1.4 while 1.1–1.3 were outstanding; it ignored explicitly linked technical documents; it failed checklist execution and **fabricated data** (reported 160GB disk usage where the real figure was 54GB); and no command existed to move a story out of `Draft`. **The issue received no maintainer response.** (That report is against v4.35.3 — much predates v6, so treat the specific bugs as dated, but the structural complaints about agent isolation are unaddressed in the v6 design.)
- **Monolithic document assumptions.** One Brief, one PRD, one Architecture, one shared output directory — which blocks parallel workstreams and forces manual per-feature workarounds. Independent analysis flags the shared output directory as a limitation versus per-feature isolation models.
- **Persona bloat for teams under ~5 is a named, acknowledged drawback.** Quick Flow mitigates it; it does not remove it. For a solo developer, the honest framing is: the *artifacts* (PRD, architecture, story) have value; the *role-play wrapper* around producing them mostly does not. You are paying tokens for an LLM to address itself as "Scrum Master" before doing what it would have done anyway.
- **The strongest evidence against personas comes from a competitor's own retreat.** Agent OS v3 deleted its spec-writing, task-breakdown, and orchestration phases and stopped installing subagents, with the maintainer stating plainly: *"It doesn't make sense to reinvent these core functions, which are much better handled by the core tools than 3rd-party frameworks"* and *"Frontier models handle spec implementation well on their own — this is the recommended approach in 2026+."* A framework in the same category looked at 2026 model capability and concluded the orchestration layer was dead weight.
- **Claude Code integration is not friction-free.** BMAD deliberately does *not* use the Claude Code plugin system, installing into `.claude/commands/` with a nested multi-level namespace (`bmad:bmm:workflows:create-story`); there is a filed compatibility issue that Claude Code does not discover slash commands in nested directories under `.claude/commands/bmad/`. Upgrades have broken command discovery before (missing workflows after an alpha bump).
- **Version churn is high.** v6.10's workflow map (`bmad-spec`, `ARCHITECTURE-SPINE.md`, `bmad-build`) does not match names in v6 material only a few months old (`create-story`, `dev-story`, `prd.md`/`epics.md`). Most tutorials and blog posts you find will describe a layout you don't have. Budget for stale documentation.
- **Steep learning curve with a heavy, hard-to-review artifact set** — independently characterized as such, and it's the reason the Quick Flow path exists.
- *(GSD)* **Serious governance risk.** The original maintainer went unreachable ~April 2026 for ~7 weeks and the associated `$GSD` token was publicly linked to a rug-pull; the upstream repo is now locked with a redirect banner. The MIT-licensed community fork (`open-gsd/gsd-core`) preserves code, history, and attribution and is actively maintained — but adopting GSD today means adopting a fork of an abandoned project whose brand carries a crypto scandal. That is a reputational and continuity risk, not a technical one, but it is real.
- *(CCPM)* **Hard GitHub dependency.** No GitHub repo, no CCPM. The advertised gains (89% less context-switching, 5–8 parallel tasks, 75% fewer bugs, 3× faster delivery) are vendor-published with no methodology — treat as marketing. Its parallel-worktree value proposition is also close to zero for one developer running one agent.
- *(Agent OS)* **Nearly useless on day 0 of a greenfield repo.** Its core loop mines standards *from existing code*. Its value here is deferred, not absent.

## 7. Fit signals

**Strong fit when:**

- Your team already thinks in PRDs, architecture docs, and sprint stories — BMAD accelerates an existing discipline rather than creating one.
- You are in a compliance-heavy or regulated environment where a versioned audit trail of *who decided what and why* is itself a deliverable.
- The design is genuinely uncertain and expensive to get wrong, and one `/bmad-party-mode` + adversarial review is cheaper than a wrong architecture — this is BMAD Full's real niche.
- Multiple humans and/or multiple agents work the same repo concurrently, and state must be visible outside anyone's chat log → **CCPM**, provided GitHub is already the hub.
- The codebase has strong existing conventions that agents keep violating → **Agent OS**, later, once your Rust code has conventions worth extracting.
- Sessions are long and context rot is the actual pain, but agile ceremony is not wanted → **GSD**, if you can accept the fork's governance history.

**Poor fit when:**

- **Solo developer, greenfield, fast iteration** — i.e., exactly this project. The persona layer has no team to coordinate, and the measured cost of the full pipeline (6 days / $200 per feature) is not recoverable when the "team" is one person who could have just written the design down.
- Your process discipline is weak. BMAD amplifies existing structure; it does not supply it. Without discipline you get compounded chaos across more agent layers.
- Requirements are volatile. There is no delta model — changed intent means rewriting whole documents, and drift goes undetected.
- You need per-feature isolation or parallel workstreams under BMAD's single-Brief/single-PRD/shared-output-directory model.
- You want minimal moving parts. BMAD needs Node 20.12+, Python 3.10+, `uv`, an installer, and a multi-module ecosystem before the first line of code.

## 8. Rust / systems-software notes

**Honest answer: none of these four have demonstrated fit for protocol/systems work, and BMAD's implicit model is product/SaaS.**

- **Domain assumptions.** BMAD's persona set (PM, UX Designer, Scrum Master, Technical Writer) and artifact set (PRD, product brief, PR/FAQ, "Working Backwards" validation) are the vocabulary of consumer/B2B product development. Its walkthroughs use examples like "User Auth System", "Mapping Service", "Payment System", with an Architect that recommends WebSocket/MQTT and PostGIS. The dedicated vertical modules that exist are Game Dev (Unity/Unreal/Godot) and Creative Intelligence — **there is no systems/protocol/infrastructure module**, and I found no substantive report of BMAD used on a Rust systems project. GitHub's `bmad-method` topic filtered to Rust returns essentially nothing of consequence.
- **What a MySQL-wire-protocol proxy actually needs from a spec** — byte-level frame layouts, capability-flag negotiation matrices, state machines with explicit illegal transitions, backpressure and connection-lifecycle invariants, "must not allocate on the hot path" budgets, and failure semantics under partial reads — **maps poorly onto a PRD/user-story template.** "As a user, I want my query routed to the right backend" discards precisely the information that makes the implementation correct. A story-shaped artifact is a bad container for a protocol invariant.
- **Concurrency and async.** Nothing in any of the four reasons about `Send`/`Sync` boundaries, cancellation safety, task lifetimes, or tokio runtime structure. These are the failure modes that will actually bite this project, and the frameworks are silent on them.
- **Property and mutation testing.** BMAD's Test Architect (TEA) module does risk-based test *strategy* and automation, which is the closest any of the four comes. Whether it produces useful guidance for `proptest` or `cargo-mutants` (which your `.gitignore` already anticipates) is **unverified** — I found no report of TEA used on Rust. CCPM ships a `test-runner` agent that is language-agnostic plumbing, not test design.
- **Performance budgets.** No framework has a first-class concept of a latency/throughput budget as a checkable acceptance criterion. You would encode these by hand in a story's acceptance criteria, at which point the framework is contributing a text field.
- **The one part that transfers cleanly** is the context-engineering mechanic: a self-contained work item that carries the exact protocol section, the relevant existing module, and the invariants, handed to a fresh-context session. That works for Rust as well as for anything — and it is available from OpenSpec, GSD, plain Claude Code plan mode, or a hand-written markdown file, at a small fraction of BMAD's ceremony.

## 9. Cost & lock-in

- **Money:** All four are free and MIT-licensed. No paid tiers. BMAD is the product of BMad Code, LLC but the framework itself is not monetized.
- **Token cost:**
  - **BMAD Full: ~$200 and ~6 days per feature** in the one independent benchmark. **BMAD Quick: ~$85 and ~6.5 hours.** Treat these as order-of-magnitude, single-observation figures, not a benchmark suite.
  - Structural driver: every persona handoff re-establishes context. v6's step files (2–3k tokens/step vs ~15k monolithic) and document sharding (reported ~82% reduction on selective loading) cut this materially, and web bundles let you move the planning phase onto a flat-rate Gemini/ChatGPT subscription — one user reported $847/mo → $220/mo, self-reported and unverifiable.
  - **CCPM/GSD/Agent OS are all substantially cheaper per feature** because none of them run a persona relay. Agent OS v3 is nearly free at the margin — it injects standards text and defers to plan mode.
- **Lock-in:** **Low across the board.** Every artifact is markdown or YAML committed to your repo. Uninstalling BMAD means deleting `.bmad/` and keeping `docs/`; your PRD, architecture doc, and stories remain readable prose. The genuine lock-in is *cognitive and structural* — a repo full of epic/story-numbered documents pushes you to keep feeding the format, and BMAD's own version churn (v5→v6 required a documented upgrade path; v6.x renamed workflows and outputs) means even staying inside BMAD costs migration effort.
  - **CCPM is the exception with real lock-in**: your project state lives in GitHub Issues. Leaving means your history stays behind in an issue tracker, which is arguably fine — that's just using GitHub.
  - **GSD's lock-in risk is the project itself**, not the format: you are depending on a fork of an abandoned upstream.
- **Exit path:** Easy for BMAD, Agent OS, and GSD — `rm -rf` the framework directory, keep the prose. Migrating BMAD artifacts *into* a lighter tool (OpenSpec/Spec Kit) means hand-condensing a PRD + architecture + epics + stories into a smaller spec set; the information survives, the structure doesn't.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://github.com/bmad-code-org/BMAD-METHOD | repo | fetched 2026-08-08 | 51.6k stars, 5.9k forks, 406 watchers, 2,021 commits, MIT, `npx bmad-method install`, Node 20.12+/Python 3.10+/uv, module ecosystem table, two-phase framing |
| 2 | https://github.com/bmad-code-org/BMAD-METHOD/releases | repo | fetched 2026-08-08 | v6.10.0 (2026-07-03), v6.9.0 (Jun 22), v6.8.0 + Web Bundles v1.0.0 (May 25), v6.7.1/v6.7.0 (May); monthly cadence; v6.10 adds `bmad-loop` module and `bmad-dev-auto` |
| 3 | https://docs.bmad-method.org/reference/workflow-map/ | docs (primary) | fetched 2026-08-08 | Authoritative four-phase workflow map, workflow names, and per-step output artifacts; `project-context.md` as "connective tissue" |
| 4 | https://ranthebuilder.cloud/blog/i-tested-three-spec-driven-ai-tools-here-s-my-honest-take/ | hands-on report | 2026 | **Highest-value source.** BMAD v6.0.3 vs Spec-Kit v0.1.6 vs OpenSpec v1.2.0. BMAD Full 6 days/$200; BMAD Quick 6.5h/$85; BMAD 5/5 on iterative refinement; the `/bmad-help` "rescue you from BMAD's own complexity" quote; OpenSpec overall winner 4.0/5 |
| 5 | https://github.com/bmad-code-org/BMAD-METHOD/issues/446 | user report (primary) | filed against v4.35.3 | Solo dev, brownfield: monolithic single-PRD model, agents unaware of each other, dev agent picked story 1.4 over 1.1–1.3, ignored linked docs, fabricated 160GB vs actual 54GB, no command to advance a Draft story. **No maintainer response.** |
| 6 | https://rywalker.com/research/bmad-method | analysis | Jun 2026 | v6 module table, scale-adaptive planning, 12+ persona list incl. Quick Flow Solo Developer, "steep learning curve / heavy artifact set", shared-output-directory limitation, 49k stars/5.7k forks Jun 2026 (up from 37k in Feb), 60 open issues, last push Jun 11 2026 |
| 7 | https://dev.to/willtorber/spec-kit-vs-bmad-vs-openspec-choosing-an-sdd-framework-in-2026-d3j | analysis | 2026 | Pipeline `Analyst → PM → Architect → SM → Dev → QA`; "12+ AI personas as Agent-as-Code markdown"; handoff-failure and spec-drift-at-handoff critiques; "amplifies existing structure, doesn't create it"; verdict = compliance-heavy environments |
| 8 | https://medium.com/@hieutrantrung.it/from-token-hell-to-90-savings-how-bmad-v6-revolutionized-ai-assisted-development-09c175013085 | blog (self-reported) | 2026 | v6 step-file architecture: ~15k monolithic → 2–3k/step; 45k → 8k document sharding (82%); 31,667 → 8,333 tokens (74%); $847/mo → $220/mo; the `.bmad/` directory tree reproduced in §3 |
| 9 | https://github.com/buildermethods/agent-os | repo | fetched 2026-08-08 | 5.2k stars, MIT, directory layout, "works alongside Claude Code, Cursor, Antigravity" |
| 10 | https://github.com/buildermethods/agent-os/discussions/310 | maintainer statement (primary) | Jan 2026 | **Key counter-evidence on personas.** v3 retired spec writing, task breakdown, and implementation orchestration; no longer installs subagents; "It doesn't make sense to reinvent these core functions…"; "Frontier models handle spec implementation well on their own — this is the recommended approach in 2026+" |
| 11 | https://buildermethods.com/agent-os | docs | fetched 2026-08-08 | v3 four-step model (Install/Discover/Inject/Shape), `agent-os/` directory, free & open source, defers spec writing to Claude Code plan mode |
| 12 | https://github.com/automazeio/ccpm | repo | fetched 2026-08-08 | 8.3k stars, 832 forks, MIT; five-phase pipeline; `.claude/epics/`; requires `gh` + `gh-sub-issue`; GitHub Issues (not Projects API) as source of truth; four agents; vendor-claimed 89%/5–8×/75%/3× figures; v2 = Agent Skill, v1 = `/pm:*` slash commands archived |
| 13 | https://github.com/open-gsd/gsd-core | repo | fetched 2026-08-08 | 7.9k stars, 547 forks, MIT, v1.7.0, `npx @opengsd/gsd-core@latest`, `/gsd-new-project` + `/gsd-onboard`, Discuss→Plan→Execute→Verify→Ship, `STATE.md`/`CONTEXT.md`, ~200k clean context per executor, 15+ runtimes |
| 14 | https://github.com/open-gsd/gsd-core/discussions/109 | maintainer statement (primary) | 2026 | **GSD provenance.** TÂCHES unreachable since ~Apr 2026 (~7 weeks); `$GSD` token publicly linked to a rug-pull; upstream locked with redirect banner; fork by Trek-e; MIT + history + attribution preserved; npm renamed `get-shit-done-cc` → `get-shit-done-redux` |
| 15 | https://docs.opengsd.net/core/introduction | docs (primary) | fetched 2026-08-08 | GSD five-phase loop, three persistent artifacts, fresh-context subagents to prevent "context rot" |
| 16 | https://www.augmentcode.com/learn/gsd-stars-spec-driven | vendor blog | 2026 | GSD at 53.9k stars / 132 contributors / 4.5k forks (upstream, pre-lock); explicit anti-ceremony positioning: "Other SDD tools bring along sprint ceremonies, story points, stakeholder syncs, and Jira-style workflows. GSD rejects all of that." |
| 17 | https://github.com/bmad-code-org/BMAD-METHOD/issues/773 | issue (primary) | 2026 | Claude Code does not discover slash commands in nested directories under `.claude/commands/bmad/` |
| 18 | https://github.com/bmad-code-org/BMAD-METHOD/issues/1111 | issue (primary) | 2026 | Missing workflows under Claude commands after alpha version upgrade — upgrade fragility |
| 19 | https://github.com/bmad-code-org/BMAD-METHOD/issues/1584 | issue (primary) | 2026 | BMAD + Claude Code agent teams; `dev-story` sequence `create-story → test-plan → dev-story → test-review → code-review → PR/merge`; BMAD deliberately not using the Claude Code plugin system |
| 20 | https://medium.com/@mikeschlottig44/embrace-bmad-or-live-in-regret-22aea58f1a67 | blog | 2026 | Levels 0–4 routing detail: Quick Flow = tech-spec only, no PRD, 1–2 stories; L2+ = full PRD/architecture/gates; L3–4 = per-epic just-in-time solutioning |

## 11. Confidence & gaps

- **Confidence: medium-high on *what these tools are*; medium-low on *what they cost you*.** Structure, commands, artifacts, versions, licenses, and maintenance signals come from primary repos, official docs, and maintainer statements fetched on the survey date. The cost and value claims rest on a much thinner base.

- **Unverified claims — do not treat as fact:**
  - **The $200 / 6-day and $85 / 6.5-hour figures are a single hands-on test of BMAD v6.0.3 by one author on one feature.** Directionally consistent with everything else observed, but it is n=1, on a version four minors behind current (v6.10.0), and the task domain was not Rust. The *ratio* between BMAD Full and BMAD Quick is more trustworthy than the absolute numbers.
  - **All token-reduction percentages (74%, 80%, 82%) and the $847→$220/mo figure are self-reported by a single blogger with no published methodology.** I could not find independent measurement of BMAD's per-feature token consumption.
  - **CCPM's "89% less context-switching / 75% fewer bugs / 3× faster"** are vendor-published with no methodology. Treat as marketing.
  - **Star counts drift.** BMAD reads 51.6k on the repo today vs 49k reported in June analyses; GSD's 53.9k is the *upstream* count before the repo was locked, while the active fork sits at 7.9k. Do not compare those two numbers directly.
  - **Issue #446's specific bugs are against v4.35.3** and several may be fixed. I verified that its *structural* complaints (agent isolation, monolithic single-PRD model, shared output directory) are still reflected in v6 descriptions, but I did not verify them by running v6.
  - **"12+ personas"** is repeated across secondary sources; I did not enumerate the persona files in a v6.10 install. Notably, **v6.10's official workflow map is organized by *workflow*, not by *agent*** — which suggests BMAD may itself be de-emphasizing the persona framing. That reading is my inference from the docs structure, not a stated maintainer position.
  - **Reddit/HN primary criticism could not be retrieved.** Searches for abandonment threads returned no usable results; the negative evidence in §6 comes from GitHub issues, one hands-on benchmark, and third-party analyses instead. **The absence of retrievable "I quit BMAD" threads is a gap in this survey, not evidence that they don't exist.**
  - **GSD's "trusted by engineers at Amazon, Google, Shopify, Webflow"** is a vendor marketing claim from the pre-fork project. Unverifiable and now attached to a project with the governance history in §6.
  - **Agent OS v3's exact install command and current release number** were not retrieved from a primary source; v3.0.0 shipped Jan 2026 and later v3.x releases exist.

- **Open questions a decision-maker should test hands-on:**
  1. **Run BMAD Quick Flow (level 0–1) once on a real Doris-proxy feature** — e.g. "negotiate MySQL capability flags on handshake." Measure wall-clock, tokens, and whether the tech-spec captured the byte-level detail or flattened it into product prose. This is a ~1-day experiment that answers the whole question; do *not* start with BMAD Full.
  2. **Isolate the variable that matters:** does story sharding + fresh context per work item beat plain Claude Code plan mode on *your* codebase? If yes, you can buy that from OpenSpec or GSD without the persona relay.
  3. **Is `/bmad-party-mode` adversarial review actually good on a protocol design**, or does it produce generically-plausible product feedback? This is BMAD's strongest differentiator and the only one worth paying ceremony for. Test it on a design you already know the right answer to.
  4. **Does the Test Architect (TEA) module say anything useful about `proptest` / `cargo-mutants` / fuzzing a wire protocol?** Entirely unverified for Rust.
  5. **How badly does the nested-slash-command discovery issue (#773) bite in current Claude Code**, and does a BMAD minor upgrade break your command surface mid-project?
  6. **For CCPM specifically:** is the GitHub-Issues-as-database benefit real for a solo developer, or is it PR overhead with no audience? The parallel-worktree feature — its main selling point — has near-zero value at one agent.
