# Survey: Commercial / Emerging SDD Products and the No-Tool Baseline

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** agent (commercial-and-minimal researcher)
> **Scope:** Covers Tessl, BuildBetter ZeroShot, CodeMySpec, and the "no framework" baseline (CLAUDE.md + `docs/specs/` + Claude Code native features). Deliberately excludes the open-source frameworks surveyed elsewhere in this directory (OpenSpec, Spec Kit, BMAD, Kiro) except where they are needed as a reference point.
>
> **Scope update 2026-08-08.** Project narrowed to an L7 Doris proxy doing SQL rewriting for tenant
> isolation, 1:1 backend connections, passthrough auth (`discussions/milestone-1/02-proxy-scope.md`). The
> commercial subjects were **eliminated** by the open-source-only constraint
> (`discussions/milestone-1/01-sdd-tool-selection.md`, R8) and that is unchanged. The **"no framework" baseline
> remains live**, and the narrower scope arguably strengthens it: with the artifact set now known
> concretely (allowlist, rewrite rules, compatibility matrix, invariants, unknowns), a plain
> `specs/` directory plus CLAUDE.md would deliver most of the value. Read this survey's baseline
> section before committing to a framework — the honest question is what a tool adds over
> hand-rolled discipline.

## 1. Snapshot

| Field | **Tessl** | **ZeroShot (BuildBetter)** | **CodeMySpec** | **No-tool baseline** |
|---|---|---|---|---|
| Name | Tessl Platform / Framework / Registry | ZeroShot, a.k.a. the `bb` CLI | CodeMySpec | Claude Code native + `docs/specs/` |
| Vendor / owner | Tessl Ltd. (London), founded 2024 by Guy Podjarny (Snyk founder) | BuildBetter (`buildbetter.sh` 308-redirects to `tryzeroshot.com`) | CodeMySpec — apparently founder-operated; no team page found | Anthropic (Claude Code); the specs are yours |
| License | Proprietary platform. SDD tile `tesslio/spec-driven-development-tile` is MIT | Proprietary CLI + cloud. Skills repo `buildbetter-app/skills` — README says MIT, GitHub API reports `NOASSERTION` | Not stated on site; links a GitHub org with no license info | n/a — your repo, your license |
| First release | Framework + Spec Registry announced 2025-09-16 (Framework closed beta) | ZeroShot skills repo created 2026-04-03 | Unknown; "early access" as of 2026 | Claude Code GA 2025; `/loop` shipped March 2026 |
| Latest version (as of survey date) | SDD tile last pushed 2026-03-30 (14 commits) | Skills repo last pushed 2026-07-13 (29 commits) | Unknown — no changelog found | Continuously shipped; docs reference v2.1.217 |
| Popularity signal | Company: $125M raised, ~59 employees (May 2026). **SDD tile: 49 stars.** Registry claims 10,000+ usage specs; homepage now says "3,000+ searchable skills" | **Skills repo: 22 stars.** Vendor claims Brex, Rappi, PostHog, AppFolio, Clay, Lufthansa, Procore, Macmillan — see §11, these appear recycled from a different product | No stars, no independent coverage found; nearly all search results are its own SEO blog | Claude Code is the developer's existing daily driver; no separate adoption decision |
| Maintenance signal | SDD tile: 0 open issues, ~4 months since last push, 14 commits total. Platform itself actively shipping — **but away from SDD** | Skills repo: 1 open issue, ~1 month since last push, small | Unknown | Anthropic-maintained, weekly-cadence releases |
| Primary interface | `tessl` CLI + web platform + registry; SDD delivered as an installable "tile" | `bb` CLI + desktop app (Mac/Win) + cloud dashboard; SDD delivered as skills | Claude Code plugin + local MCP server + web app | Claude Code CLI itself: skills, `.claude/rules/`, hooks, plan mode |
| Agent compatibility | Claims Claude Code, Cursor, Copilot, Gemini; tile targets "MCP-compatible" agents | Claims Claude Code, Cursor, Codex, Copilot, Gemini CLI, Windsurf, Amazon Q | Claude Code only | Claude Code only (portable to others via an `AGENTS.md` shim) |
| Language/stack neutrality | Neutral in principle; registry content skews JS/TS/Python OSS libraries | Neutral — it is telemetry + prompts, not codegen | **Phoenix/Elixir only.** Contexts, LiveView, Ecto, OTP first-class | Fully neutral |
| Greenfield vs brownfield bias | Framework is greenfield-flavored (`@generate` from spec); `@describe` covers brownfield | Brownfield — the value prop is mining existing team sessions for conventions | Greenfield Phoenix apps | Neutral; you choose |

## 2. One-paragraph thesis

**Tessl.** Podjarny's bet is that the durable artifact of software should be the specification, not the code — code becomes a compilation target that agents regenerate. Tessl Framework encodes this literally: markdown specs carry `@generate`, `@describe`, `@test`, and `@use` annotations, and the Spec Registry is "npm for knowledge," shipping version-accurate usage specs for open-source libraries so agents stop hallucinating APIs. The important caveat is chronological: that pitch is from September 2025. By January 2026 Tessl had repositioned as an "Agent Enablement Platform" under the banner "Skills are the new code. Treat them that way," and the homepage now sells governance, evals, and observability over an agent-skill registry. Spec-driven development survives as an installable tile, not as the company's thesis.

**ZeroShot (BuildBetter).** The bet is that individual agent productivity does not compound because context is not shared between teammates or tools. ZeroShot instruments every coding-agent session (tokens, time, tool calls), mines successful sessions and PR reviews into reusable markdown "skills," ships them to the team via a weekly PR, and then enforces skill usage on future PRs. It is best understood as **observability and convention-enforcement for agent fleets**, not as a spec-driven development framework. Its SDD surface is a `spec-workflow` skill pack — `bb-specify`, `bb-plan`, `bb-tasks`, `bb-clarify`, `bb-analyze`, `bb-checklist`, `bb-constitution`, `bb-implement` — which is GitHub Spec Kit's command set reproduced almost one-for-one, wrapped in telemetry.

**CodeMySpec.** The bet is that specs should be a *graph with prerequisites*, not a folder of documents: every spec, test, implementation, BDD scenario, and QA result is a node, and the harness computes what to work on next. Its two real differentiators are a mandatory BDD gate — Given/When/Then scenarios in a "Spex" DSL are required behavioral contracts, not advisory docs — and live verification, where a QA subagent boots the actual app, drives a real browser via Vibium, screenshots the result, and files severity-tagged issues. It is Phoenix and Elixir native, which makes the rest of this survey academic for a Rust project.

**The no-tool baseline.** The bet is that in 2026 Claude Code's own primitives already cover most of what an SDD framework provides, and that a framework mainly sells you *discipline* and *someone else's prompt engineering*. You write specs as markdown under `docs/specs/`, bind them to code with path-scoped `.claude/rules/`, encode the workflow as skills, gate with plan mode, and make the gate real with hooks. Notably, the official docs are explicit that CLAUDE.md and rules "shape Claude's behavior but are not a hard enforcement layer" — only hooks and settings are enforced by the client regardless of what the model decides. That sentence applies equally to every framework in this survey, and the baseline has the same hooks available to it for free.

## 3. Workflow / artifact model

### Tessl

Install the SDD capability as a tile onto the platform, then run a clarify → spec → approve → implement → review loop. Approved specs land as `.spec.md` files with YAML frontmatter, with requirements linked to tests via `[@test]` annotations.

```
your-repo/
├── .tessl/                          # tile files: skills, rules, docs, validators
│   └── scripts/
│       ├── validate-specs.sh
│       └── check-spec-links.sh
├── specs/
│   ├── connection-pool.spec.md      # frontmatter + capabilities + [@test] links
│   └── wire-protocol.spec.md
└── src/
```

**Command surface:** `tessl init`, `tessl install tessl-labs/spec-driven-development`, `make eval` (runs the tile's nine evaluation scenarios locally). In the Framework proper, spec-level annotations `@generate`, `@describe`, `@test`, `@use`.

**Artifact lifecycle:** agent interrogates requirements → writes spec → **pauses for human approval** → implements → review. Specs are long-lived documents in `specs/`, not per-change deltas; there is no archive/superseded model comparable to OpenSpec. Version control story is plain git — specs are ordinary markdown. The Registry adds a second, orthogonal artifact class: versioned third-party *usage* specs consumed as dependencies.

### ZeroShot

```
your-repo/
├── .claude/skills/                  # bb-* skills installed here for Claude Code
│   ├── bb-specify/SKILL.md
│   ├── bb-plan/SKILL.md
│   └── ...
└── (spec output location NOT documented — see §11)
```

**Command surface:** `bb` CLI (`bb agent-sessions resume` for cross-teammate handoff), plus slash-skills `/bb-constitution`, `/bb-specify`, `/bb-clarify`, `/bb-plan`, `/bb-tasks`, `/bb-analyze`, `/bb-checklist`, `/bb-implement`, `/bb-review`. Testing pack adds `app-navigator`, `trust-but-verify`, `generate-tests`.

**Artifact lifecycle:** the distinctive loop is not the spec loop — it is the *skill* loop. Sessions are captured → patterns auto-extracted as markdown skills → team reviews and merges them via a weekly PR → future runs auto-apply them → PR review cites conventions and enforcement blocks non-compliant code. ZeroShot's own blog states the spec should contain intent, constraints, acceptance criteria, conventions, and evidence links, and that "the spec is the durable artifact; the prompt is disposable" — but publishes no file format, no directory convention, and no example spec.

### CodeMySpec

**Command surface:** Claude Code plugin + local MCP server; no public command list found.

**Artifact lifecycle:** requirement graph — each artifact is a node with prerequisites, and the system computes the next available node. BDD scenarios gate implementation. QA subagent verifies against a running app in a real browser.

### No-tool baseline

```
your-repo/
├── CLAUDE.md                        # <200 lines: build/test commands, invariants, workflow rules
├── AGENTS.md                        # optional; CLAUDE.md pulls it in with @AGENTS.md
├── docs/specs/
│   ├── 0001-mysql-wire-handshake.md
│   ├── 0002-connection-pooling.md
│   └── archive/
└── .claude/
    ├── rules/
    │   ├── protocol.md              # paths: ["src/protocol/**/*.rs"]  ← spec↔code binding
    │   ├── testing.md               # proptest + cargo-mutants conventions
    │   └── unsafe.md                # paths: ["**/*.rs"] — invariants for unsafe blocks
    ├── skills/
    │   ├── spec-new/SKILL.md        # /spec-new  — draft a numbered spec
    │   ├── spec-review/SKILL.md     # /spec-review — ambiguity + testability pass
    │   └── spec-verify/SKILL.md     # /spec-verify — spec↔code drift check
    └── settings.json                # hooks: the only real enforcement layer
```

**Command surface:** `/spec-new`, `/spec-review`, `/spec-verify` (skills you author — `.claude/commands/x.md` and `.claude/skills/x/SKILL.md` are now equivalent and both produce `/x`); built-ins `/init`, `/memory`, `/context`, `/loop`, `/goal`, `/schedule`, `/doctor`; `claude --permission-mode plan` or `Shift+Tab` to cycle `default → acceptEdits → plan`; `claude --worktree <name>` for parallel isolated sessions; `claude -p` for headless CI checks.

**Artifact lifecycle:** you define it, in ~30 lines of CLAUDE.md. A workable convention: numbered spec in `docs/specs/`, plan mode to produce the implementation plan, implement, then move the spec to `docs/specs/archive/` when its acceptance criteria are covered by tests. The gap versus OpenSpec/Spec Kit is that nothing computes or validates the delta for you.

Three baseline mechanisms are worth calling out because they are what make the baseline competitive rather than merely cheap:

- **Path-scoped rules.** A file in `.claude/rules/` with `paths: ["src/protocol/**/*.rs"]` frontmatter loads *only* when Claude touches matching files. This is a genuine spec↔code binding mechanism, and it is stronger than what Spec Kit or ZeroShot offer.
- **Hooks.** `PreToolUse`, `PostToolUse`, `SessionStart`, `SubagentStop`, `InstructionsLoaded`. A `PreToolUse` hook on `Edit|Write` that greps for an open spec is a real gate; nothing in Tessl, ZeroShot, or Spec Kit is.
- **Auto memory.** `~/.claude/projects/<project>/memory/` with a `MEMORY.md` index (first 200 lines / 25KB loaded per session) and on-demand topic files. Useful, but **machine-local and not shared across machines** — it is not a team artifact and must not be confused with a spec.

## 4. What it enforces vs. what it suggests

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | **Tessl:** no — the tile ships "rules enforcing spec-before-code," but rules are context, not gates. **ZeroShot:** partially — PR-level enforcement blocks non-compliant code *after* the fact, in their cloud. **CodeMySpec:** yes for BDD — the BDD gate is described as mandatory and prerequisite-computed. **Baseline:** only if you write a `PreToolUse` hook (you can; ~20 lines) | Tessl, ZeroShot pre-PR, Baseline-by-default |
| Spec ↔ code traceability | **Tessl:** closest to real — `[@test]` annotations plus `check-spec-links.sh` mechanically verify requirement→test links. **Baseline:** `.claude/rules/` `paths:` frontmatter binds guidance to file globs (mechanical loading, not mechanical verification) | ZeroShot ("evidence links" — format undocumented), CodeMySpec (graph edges, unverified externally) |
| Test/acceptance criteria | **CodeMySpec:** BDD scenarios are required contracts. **Tessl:** `validate-specs.sh` in the tile | ZeroShot, Baseline (unless you add a hook or CI job) |
| Review gate | **Tessl:** the tile's workflow includes an explicit human approval pause. **Baseline:** plan mode is a real client-side gate — no edits reach disk until you approve | ZeroShot (`/bb-review` is a prompt), CodeMySpec |
| Drift detection | **ZeroShot:** its actual strength — continuous session/PR observability that surfaces divergence from stated conventions. **Tessl:** `check-spec-links.sh` catches broken spec→test links only | Baseline (you'd write `/spec-verify` yourself), CodeMySpec |

The honest summary: **none of these four enforce spec-before-code in a way a determined model cannot route around, except through hooks or CI — which the baseline has for free.** Anthropic's own documentation states it plainly: settings and hooks "are enforced by the client regardless of what Claude decides to do," whereas instruction files "shape Claude's behavior but are not a hard enforcement layer." Every framework in this survey delivers its "gates" as instruction files.

## 5. Strengths

**Tessl**
- The `[@test]` requirement→test annotation plus `check-spec-links.sh` is the single most useful primitive found in this survey: machine-checkable traceability from a written requirement to the test that covers it. Nothing else here has a mechanical link check.
- Specs are plain markdown in a plain `specs/` directory — genuinely portable, genuinely diffable, genuinely reviewable in a normal PR.
- The Spec Registry addresses a real and under-served failure mode (agents hallucinating library APIs and mixing versions across releases) that no methodology framework touches.
- Best-capitalized vendor in this survey by a wide margin: $125M raised, >$500M valuation, ~59 employees, named investors (Index, Accel, GV, boldstart). Insolvency is not the risk here.
- Free tier is real and card-free: $0/month with 1,000 credits and a single workspace.

**ZeroShot**
- It is solving a problem the other tools ignore: agent *usage* is invisible, so teams cannot tell which conventions are actually being applied or where tokens go. Session-level observability across tokens, time, and tool calls is legitimately differentiated.
- Cross-teammate session resume (`bb agent-sessions resume`) is a genuinely novel capability — picking up another engineer's agent session, in a different agent, on your machine.
- Skills are markdown and MIT-ish licensed; the extracted conventions survive cancellation of the product.
- Bring-your-own-keys at every paid tier, so you are not double-paying for tokens.
- Free tier is usable without an account for local-only operation.

**CodeMySpec**
- The requirement-graph model (prerequisites, computed next node) is more rigorous than the flat spec→plan→tasks pipeline everyone else ships.
- Live verification against a running app with browser screenshots and severity-tagged issue filing is the strongest verification story in this survey — for the applications it targets.
- Free forever "Build" tier with bring-your-own API keys.

**No-tool baseline**
- **Zero adoption cost and zero exit cost.** Everything is markdown in your own repo. It is not merely one option among four; it is the migration target for all three of the others.
- **The only option with a real enforcement primitive.** Hooks execute deterministically at lifecycle events regardless of model behavior. A `PreToolUse` hook matching `Edit|Write` that refuses when no spec is open is strictly stronger than any prompt-level "gate" in the commercial products.
- **Path-scoped rules give spec↔code binding that Spec Kit does not have.** Glob-scoped `paths:` frontmatter means your protocol invariants load exactly when Claude opens a protocol file, and cost nothing otherwise.
- **Best token economics.** CLAUDE.md is capped by convention at ~200 lines; skills load on demand only when invoked or judged relevant; path-scoped rules load only on match. Frameworks front-load a spec/plan/tasks trilogy per feature.
- **No vendor telemetry.** Nothing about your Doris proxy's internals leaves your machine beyond the model calls you already make.
- **Plan mode is a genuine client-enforced review gate**, already integrated with the permission system, with the plan editable in your own `$EDITOR`.
- **Composable with everything else.** Subagents get isolated context windows and optional worktree isolation; `/loop` and `/goal` give stop-condition-driven iteration; `claude -p` pipes into CI. None of this requires adopting anything.
- `/init` with `CLAUDE_CODE_NEW_INIT=1` runs an interactive multi-phase setup that proposes CLAUDE.md files, skills, and hooks and shows you a reviewable proposal before writing — which is, in effect, a scaffolder for the baseline.

## 6. Weaknesses / friction

**Tessl**
- **The company has moved on from the pitch you would be buying.** The SDD tile repo has 14 total commits, 49 stars, 0 open issues, and has not been pushed since 2026-03-30 — while the platform ships actively in a different direction. Adopting Tessl for SDD means adopting the least-maintained corner of an otherwise healthy product.
- The Framework appears never to have reached general availability; it was announced as closed beta in September 2025 and no GA announcement was found through August 2026.
- Numbers on the marketing surfaces do not reconcile: the 2025 launch claims "10,000+ specs" in the registry; the 2026 homepage advertises "3,000+ searchable skills." These may be different artifact classes, but the site does not say so.
- Credit-metered pricing (1,000/month free, 5,000 at $100/month) makes per-feature cost unpredictable, and BYOK is Enterprise-only — so on Free and Team you pay Tessl for tokens you may already be paying Anthropic for.
- Adopting it means running a second CLI, a second registry, and a second config directory (`.tessl/`) alongside `.claude/`.

**ZeroShot**
- **It is not really a spec-driven development framework**, despite ranking for the term. It is agent-session observability and skill enforcement. The spec workflow is a thin skill pack.
- **That skill pack is Spec Kit's command set re-skinned.** `bb-constitution`/`bb-specify`/`bb-clarify`/`bb-plan`/`bb-tasks`/`bb-analyze`/`bb-checklist`/`bb-implement` maps essentially one-for-one onto `speckit.constitution`/`specify`/`clarify`/`plan`/`tasks`/`analyze`/`checklist`/`implement`. You can have that workflow from a 125.8k-star MIT project without a $100/user/month subscription.
- **No published spec artifact format.** Their own SDD blog post names the five things a spec should contain and then never says whether it is markdown, YAML, or JSON, where it lives, or how Claude Code consumes it. The linked docs path 404s.
- **The adopter list does not survive scrutiny** — see §11. The one adopter case study that could be located (PostHog) is about BuildBetter's customer-feedback product, not about coding agents.
- The core value proposition requires uploading your agent sessions to their cloud; free hosted tier is capped at 3 members / 200 sessions per month.
- Skills repo is 22 stars and 29 commits. License is ambiguous: README says MIT, GitHub's license detector reports `NOASSERTION`.
- **Severe name collision.** An unrelated MIT project — `the-open-engine/zeroshot`, 1,695 stars, actively developed, an executor–verifier orchestrator on npm as `@the-open-engine/zeroshot` — shares the name. Search engines conflate the two; during this survey a search summarizer merged the two products' attributes into a single description. Any due diligence you delegate will likely return contaminated results.

**CodeMySpec**
- **Phoenix/Elixir only. This alone disqualifies it for a Rust project.** Its first-class concepts are Phoenix contexts, LiveView, Ecto, and OTP.
- Its verification centerpiece — booting the app and driving a real browser — is meaningless for a wire-protocol proxy with no UI.
- Essentially no independent coverage exists. Almost every search result describing CodeMySpec is CodeMySpec's own SEO blog, which also publishes head-to-head comparison articles in which it ranks itself. Treat all descriptive claims here as vendor-sourced.
- Apparently a solo operation (the custom tier is "quoted after consultation with the founder"), which is the highest bus-factor risk in this survey.

**No-tool baseline**
- **You lose the forcing function, and that is the real loss.** When nothing prints "no spec found for this change," you will skip the spec under deadline pressure. Every other weakness here is fixable in an afternoon; this one is a discipline problem and discipline is exactly what a solo greenfield project is worst at.
- **You lose a named, versioned change unit.** OpenSpec's `changes/<id>/` with proposal, delta, and tasks is a real artifact contract. You would invent your own numbering and directory convention, and it will be worse for the first month.
- **You lose the delta/archive lifecycle.** Nothing tells you a spec has been implemented and should be folded into a baseline spec or archived. Specs rot silently.
- **You lose someone else's prompt engineering.** Spec Kit's `/clarify` (ambiguity interrogation) and `/analyze` (cross-artifact consistency) encode real, non-obvious work. Writing equivalents yourself costs a day and the first versions will be noticeably worse.
- **You lose community improvement.** A framework's prompts get better while you sleep; yours do not.
- **CLAUDE.md is Claude-specific.** Claude Code does not read `AGENTS.md` natively; you need an `@AGENTS.md` import or a symlink. If the developer ever wants Codex or Cursor on this repo, the workflow (as opposed to the specs) does not port.
- **Auto memory is a trap if mistaken for a spec store.** It is machine-local, not shared across machines or with teammates, and only the first 200 lines of `MEMORY.md` load per session. Specs must live in `docs/specs/` under version control, not in auto memory.
- Setup is not zero: budget roughly half a day to write CLAUDE.md, two or three path-scoped rules, three skills, and one enforcement hook.

## 7. Fit signals

**Strong fit when:**

- *Tessl* — you want mechanical requirement→test traceability and are willing to run a lightly-maintained MIT tile that you may end up forking; **or** your actual pain is agents hallucinating third-party library APIs, in which case use the Registry alone and ignore the Framework; **or** you are an enterprise that needs governance, evals, and audit over an agent-skill sprawl you already have.
- *ZeroShot* — you run a team of five or more engineers on coding agents, you cannot see where tokens or time go, and you want team conventions mined from real sessions and enforced at PR time. It is a fleet-management purchase.
- *CodeMySpec* — you build Phoenix/Elixir web applications and want BDD gating plus live browser verification.
- *No-tool baseline* — a solo or small-team greenfield project; a domain (systems software) that no framework's templates were designed for; a developer who wants to understand the workflow before delegating it; a project where the specs matter more than the ceremony around them.

**Poor fit when:**

- *Tessl* — you are a solo developer on a Rust systems project. You would take on a second CLI, a second config directory, credit metering, and a stale tile, for one primitive (`[@test]`) you could reimplement in a `.claude/rules/` file plus a shell script.
- *ZeroShot* — you are a solo developer. There is no fleet to observe, no teammate to hand a session to, and no team convention drift to detect. The entire value proposition is inapplicable, and the SDD layer underneath it is Spec Kit.
- *CodeMySpec* — anything not Phoenix/Elixir. Full stop.
- *No-tool baseline* — you are onboarding several engineers who need a shared, named, enforced process; you want cross-agent workflow portability; or you know from experience that you will not maintain the discipline without a tool that nags you.

## 8. Rust / systems-software notes

None of the three commercial products has any Rust-specific affordance. More importantly, none has a place to put the things that actually matter for a Doris proxy.

**What a Doris proxy's load-bearing specs contain:** MySQL wire-protocol conformance (packet framing, capability-flag negotiation, auth-plugin handshakes), connection-pooling and backpressure invariants, latency and allocation budgets on the hot path, failover and query-routing semantics, and concurrency invariants around shared session state.

- **Protocol work.** No tool here has a primitive for a state machine or a wire format. Tessl's spec structure (description, capabilities-with-tests, API definition) is the closest fit because "capabilities with tests" maps acceptably onto "handshake states with conformance tests." ZeroShot has no published format at all. CodeMySpec's Given/When/Then targets user-visible behavior, which is the wrong altitude for packet framing.
- **Concurrency and invariants.** Nothing in this survey lets you write an invariant that anything checks. The baseline is not better in kind — but a `.claude/rules/` file scoped to `paths: ["src/pool/**/*.rs"]` stating the pooling invariants loads automatically whenever Claude opens a pool file, which is more than any of the commercial products do.
- **Performance budgets.** No tool here has a place to record "p99 under 2ms at 1k concurrent connections" that anything verifies. A criterion regression gate wired into a hook or CI does real work; a spec field labeled "performance" does not.
- **Property and mutation testing.** The project's `.gitignore` already anticipates `cargo-mutants`. **No tool in this survey integrates with `cargo-mutants`, `proptest`, or `quickcheck`, or knows what a mutation score is.** CodeMySpec is the only one with a native verification loop and it drives a browser. For this project, mutation score against a spec's acceptance criteria is a far stronger gate than any of these products offer, and wiring it is a `PostToolUse` hook or a CI job either way.
- **Language neutrality is not the same as language fitness.** Tessl and ZeroShot are neutral because they are format-and-telemetry layers that do not care what you write. That neutrality means they contribute nothing Rust-specific either.

**Unknown, stated as unknown:** whether Tessl's Registry contains meaningful usage specs for the Rust crates this project would depend on (`tokio`, `sqlx`, `bytes`, `mysql_async`). The registry's advertised content skews toward JavaScript/TypeScript and Python ecosystems, but no crate-level inventory was published and this was not verified.

## 9. Cost & lock-in

**Tessl**
- **Money:** Free $0/month (1,000 credits, single workspace, no card). Team $100/month (5,000 credits, role management, pay-as-you-go overage). Enterprise custom (platform fee + annual credit bundles, BYOK, SAML SSO, audit logs). Credit-based, not per-seat. Publishing/installing plugins is free at all tiers.
- **Token cost:** unclear, because credits mediate. On Free and Team you are paying Tessl for model calls; BYOK is Enterprise-only, so you cannot fold this into your existing Claude subscription without an enterprise contract.
- **Lock-in:** low on artifacts, moderate on workflow. Specs are `.spec.md` markdown in `specs/` — fully portable. The `@generate`/`@describe`/`@test`/`@use` annotations and the `[@test]` links are Tessl-specific syntax, but they are inert markdown that another agent can be taught to read.
- **Exit path:** easy. Delete `.tessl/`, keep `specs/`, replace `check-spec-links.sh` with your own script. The one thing you lose is the Registry's third-party usage specs.

**ZeroShot**
- **Money:** Free $0 (local app on Mac/Windows/CLI with no infrastructure use, no account required; or hosted for up to 3 members / 200 sessions per month). Pro $100/user/month with BYOK. Enterprise custom (SSO/SAML, audit logs, private VPC/on-prem).
- **Token cost:** BYOK at paid tiers, so you are not double-charged for tokens. The subscription buys the service layer.
- **Lock-in:** artifacts are low-risk (skills are markdown), but the *value* is high-lock-in — session history, the observability dashboard, extracted-skill provenance, and cross-teammate resume all live in their cloud. Cancel and you keep a folder of markdown skills and lose everything that justified the price.
- **Exit path:** trivially easy for a solo developer, precisely because there is nothing to exit from. For a team that has built its convention library inside ZeroShot, harder.

**CodeMySpec**
- **Money:** Build tier free forever with bring-your-own API keys. Operate & Grow $100/month/user. "Built with you" custom, $500 minimum, quoted after a call with the founder.
- **Token cost:** BYOK on the free tier, so it rides your existing subscription.
- **Lock-in:** unknown. The requirement graph lives in a local MCP server plus a web app; whether the graph is exportable was not determinable from public sources. The Spex BDD DSL is proprietary syntax.
- **Exit path:** unknown, and moot for this project.

**No-tool baseline**
- **Money:** $0 beyond the Claude Code subscription already in use.
- **Token cost:** the lowest of the four. CLAUDE.md is conventionally capped near 200 lines of always-on context; skills load only on invocation or relevance; path-scoped rules load only when a matching file is touched. Per-feature overhead is whatever your spec is, with no mandatory plan/tasks artifacts unless you choose to generate them.
- **Lock-in:** none. Every artifact is markdown in your own git repository.
- **Exit path:** n/a — this *is* the exit path for the other three. Any of them can be adopted later on top of a `docs/specs/` directory; the reverse migration is the one that costs something.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://tessl.io/blog/tessl-launches-spec-driven-framework-and-registry/ | vendor blog | 2025-09-23 | `@generate`/`@describe`/`@test`/`@use` annotations; spec = description + capabilities-with-tests + API definition; 10,000+ registry specs |
| 2 | https://tessl.io/blog/announcing-tessls-products-to-unlock-the-power-of-agents/ | vendor blog | 2025-09-16 | Framework announced as **closed beta**, Spec Registry as open beta |
| 3 | https://tessl.io/ | vendor site | fetched 2026-08-08 | Current positioning: Platform / Agent / Registry; "3,000+ searchable skills"; claims Claude Code, Cursor, Copilot, Gemini support |
| 4 | https://tessl.io/pricing | vendor site | fetched 2026-08-08 | Free $0 / 1,000 credits; Team $100/mo / 5,000 credits; Enterprise custom; credit-based not per-seat; BYOK Enterprise-only |
| 5 | https://docs.tessl.io/ | vendor docs | fetched 2026-08-08 | Six pillars: Registry & package manager, Governance, Evals, Observability, Inventory, Tessl Agent — skills-centric, no Framework spec docs on the index page |
| 6 | https://docs.tessl.io/use/spec-driven-development-with-tessl | vendor docs | fetched 2026-08-08 | `tessl init`; `tessl install tessl-labs/spec-driven-development`; specs in `specs/` as markdown; clarify → spec → **approval pause** → implement |
| 7 | https://github.com/tesslio/spec-driven-development-tile | repo | fetched 2026-08-08 | MIT; installs to `.tessl/`; `specs/*.spec.md` with YAML frontmatter and `[@test]` links; `validate-specs.sh`, `check-spec-links.sh`; nine eval scenarios; works with MCP-compatible agents incl. Claude Code |
| 8 | GitHub API `repos/tesslio/spec-driven-development-tile` | repo metadata | 2026-08-08 | 49 stars, 14 commits, 0 open issues, last push 2026-03-30, MIT, not archived |
| 9 | https://fortune.com/2024/11/14/tessl-funding-ai-software-development-platform | press | 2024-11-14 | $125M raise |
| 10 | https://tracxn.com/d/companies/tessl/ | data aggregator | accessed 2026-08-08 | $125M over 2 rounds, >$500M valuation, ~59 employees as of 2026-05-31, London, founded 2024, investors Index/Accel/GV/boldstart |
| 11 | https://tryzeroshot.com/ | vendor site | fetched 2026-08-08 | "BuildBetter ZeroShot"; session monitoring, skill extraction, PR enforcement; adopter list "Brex, PostHog, AppFolio, Rappi, Lufthansa, Procore"; links github.com/buildbetter-app/skills |
| 12 | https://tryzeroshot.com/pricing | vendor site | fetched 2026-08-08 | Free $0 (local, or hosted 3 members / 200 sessions per month); Pro $100/user/mo BYOK; Enterprise custom |
| 13 | https://tryzeroshot.com/blog/spec-driven-development-with-ai-coding-agents | vendor blog | 2026-06-19 | Spec = intent + constraints + acceptance criteria + conventions + evidence links; names `/bb-specify`, `/bb-plan`; **no file format, directory, or example given** |
| 14 | https://github.com/buildbetter-app/skills | repo | fetched 2026-08-08 | Skill packs core / spec-workflow / testing; `pip install bb-skills`; README states MIT |
| 15 | GitHub API `repos/buildbetter-app/skills` | repo metadata | 2026-08-08 | 22 stars, 29 commits, 1 open issue, created 2026-04-03, last push 2026-07-13, license `NOASSERTION` (conflicts with README's MIT) |
| 16 | https://new.buildbetter.ai/case-study/posthog | vendor case study | 2026 | **About BuildBetter's customer-feedback product, not coding agents.** Quotes Anna Szell on sorting calls and feedback and auto-linking issues in GitHub. No mention of ZeroShot, `bb`, or agent sessions |
| 17 | https://www.buildbetter.sh/ | vendor site | fetched 2026-08-08 | HTTP 308 permanent redirect to tryzeroshot.com — confirms `bb` CLI and ZeroShot are the same product |
| 18 | https://github.com/github/spec-kit | repo | fetched 2026-08-08 | Commands `speckit.constitution`, `.specify`, `.clarify`, `.plan`, `.tasks`, `.analyze`, `.checklist`, `.implement`, `.converge`, `.taskstoissues`; MIT; 125.8k stars — basis for the ZeroShot command-set comparison |
| 19 | GitHub API `repos/the-open-engine/zeroshot` | repo metadata | 2026-08-08 | 1,695 stars, MIT, 114 open issues, last push 2026-08-07 — **unrelated project sharing the ZeroShot name** |
| 20 | https://github.com/the-open-engine/zeroshot | repo | fetched 2026-08-08 | Executor–verifier orchestration; npm `@the-open-engine/zeroshot`; no mention of BuildBetter or tryzeroshot.com |
| 21 | https://codemyspec.com/ | vendor site | fetched 2026-08-08 | Build tier free forever (BYO API keys); Operate & Grow $100/mo/user; custom $500 minimum quoted by the founder |
| 22 | https://codemyspec.com/blog/spec-driven-development | vendor blog | 2026 | Self-description: full-lifecycle SDD harness for **Phoenix and Elixir**, Claude Code plugin + local MCP server + web app; requirement graph; Spex BDD DSL; Vibium browser QA; "**no Rust support**" |
| 23 | https://codemyspec.com/blog/tessl-review | competitor blog | 2026 | Claims Tessl Framework still not GA after ~9 months of closed beta, and that Tessl pivoted 2026-01-29 to "Skills are the new code." **Competitor-authored — see §11** |
| 24 | https://code.claude.com/docs/en/memory | official docs | fetched 2026-08-08 | CLAUDE.md hierarchy and load order; `@path` imports (depth 4); `.claude/rules/` with `paths:` glob frontmatter; **Claude Code reads CLAUDE.md, not AGENTS.md**; auto memory at `~/.claude/projects/<project>/memory/`, machine-local, 200-line/25KB index limit; "Settings rules are enforced by the client regardless of what Claude decides to do… CLAUDE.md instructions… are not a hard enforcement layer" |
| 25 | https://code.claude.com/docs/en/skills | official docs | fetched 2026-08-08 | `SKILL.md` layout; body loads on demand; `/skill-name` invocation; **custom commands merged into skills** — `.claude/commands/deploy.md` ≡ `.claude/skills/deploy/SKILL.md` |
| 26 | https://code.claude.com/docs/en/common-workflows | official docs | fetched 2026-08-08 | Plan mode via `--permission-mode plan` or `Shift+Tab` (`default → acceptEdits → plan`); subagent delegation; `claude --worktree`; `claude -p`; `/loop` scheduling table |
| 27 | https://www.explainx.ai/blog/claude-code-loops-official-guide-turn-goal-schedule-2026 | third-party blog | 2026 | `/loop` shipped March 2026; Anthropic published "Getting started with loops" 2026-07-07 covering `/goal` stop conditions, `/loop`, `/schedule` |
| 28 | Local filesystem `/Users/hclin/.claude/` | direct inspection | 2026-08-08 | Confirms live `settings.json` with `PreToolUse`/`PostToolUse` hooks (GitNexus), `skills/`, `commands/`, `hooks/`, `plugins/`, `projects/*/memory/` — the baseline's mechanisms are already installed and in use on this machine |
| 29 | https://specdriven.com/landscape/ | independent tracker | 2026 | 30+ frameworks tracked; adoption leaders Superpowers ~224k, Spec Kit ~111k, GSD ~64k, OpenSpec ~54k, BMAD ~49k. **Page returned HTTP 403 to direct fetch; figures are from search-result excerpts only** |

## 11. Confidence & gaps

**Confidence:** **High** for the no-tool baseline and for Tessl's artifact model, pricing, and maintenance state — all grounded in official documentation, live pricing pages, GitHub API metadata, and direct filesystem inspection. **Medium** for ZeroShot, where the artifact model is genuinely undocumented and the company's claims could not be corroborated. **Low** for CodeMySpec, where essentially every available source is the vendor's own SEO content — though this matters little, since Elixir-only support disqualifies it regardless of how good it is.

**Unverified claims:**

- **ZeroShot's adopter list is not verified and is probably misleading.** The homepage lists Brex, Rappi, PostHog, AppFolio, Clay, Lufthansa, Procore, and Macmillan. No engineering blog post, conference talk, or third-party report was found from any of them describing use of ZeroShot or the `bb` CLI. The single locatable case study — PostHog — is about BuildBetter's **customer-feedback product** (aggregating calls, Zendesk, Slack, and email into GitHub issues), quoting Anna Szell on reducing manual feedback triage. It contains no reference to coding agents, sessions, or skills. **The most probable reading is that ZeroShot's landing page reuses the logo wall from BuildBetter's earlier product line.** Treat every name on that list as unverified for coding-agent usage until a named engineer at one of those companies says otherwise.
- **ZeroShot's spec artifact format is undocumented.** Whether specs are markdown, where they live, and how Claude Code loads them could not be determined; the docs path referenced by their own blog (`docs.buildbetter.ai/pages/BuildBetter SH/overview`) returned HTTP 404.
- **ZeroShot's license is ambiguous.** The skills README states MIT; GitHub's license detector reports `NOASSERTION`. The proprietary/open boundary between the `bb` observability CLI and the `bb-skills` package was not established from primary sources.
- **"Tessl Framework never reached GA"** is asserted by CodeMySpec, a competitor with an SEO incentive. It is *consistent* with the primary evidence — the September 2025 announcement says closed beta, no GA announcement appears on tessl.io, and the SDD tile has 14 commits — but it is not independently confirmed.
- **The `3,000+ skills` vs `10,000+ specs` discrepancy** on Tessl's own surfaces is unexplained. These may be distinct artifact classes counted at different times; the site does not reconcile them.
- **specdriven.com star counts** (Superpowers ~224k, Spec Kit ~111k) come from search excerpts only; the site returned HTTP 403 to direct fetch, and the Spec Kit figure disagrees with the GitHub API's 125,795, so treat the whole set as approximate.
- **CodeMySpec's entire feature description** — requirement graph, Spex DSL, Vibium browser QA, mandatory BDD gate — is vendor-sourced with no independent confirmation.
- **Tessl Registry coverage for Rust crates** was not verified. No crate-level inventory is published.

**Open questions a decision-maker would still need to test hands-on:**

1. Does Tessl's `check-spec-links.sh` actually verify `[@test]` links against Rust `#[test]` functions and `#[cfg(test)]` modules, or does it only validate that the referenced path exists? This determines whether Tessl's one differentiating primitive works for this project at all — and it is a 30-minute experiment.
2. How much does the SDD tile's staleness bite in practice? Install it, run `make eval`, and see whether the nine eval scenarios still pass against current Claude Code.
3. Can a `PreToolUse` hook matching `Edit|Write` reliably implement a spec-before-code gate without becoming so annoying it gets disabled within a week? This is the pivotal question for the baseline, and it is a one-afternoon experiment that also derisks every other option.
4. How much of Spec Kit's `/clarify` and `/analyze` prompt value can be recovered by copying those two MIT-licensed prompt files into `.claude/skills/` — without adopting Spec Kit's directory model or its per-feature artifact ceremony? This is the highest-leverage unexplored option in this survey: the baseline plus two borrowed prompts.
5. For Tessl specifically: is the Spec Registry usable *standalone*, as a hallucination-reduction layer for Rust crates, without adopting the Framework or the SDD tile at all? That is a different and possibly better purchase than the one this survey was scoped to evaluate.
