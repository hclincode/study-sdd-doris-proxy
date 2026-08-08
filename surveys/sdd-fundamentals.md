# Survey: Spec-Driven Development (SDD) as a Methodology

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** research agent (Claude Opus 5)
> **Scope update 2026-08-08.** This survey is about the methodology and is deliberately
> project-independent; the narrowing of the proxy scope (`discussions/milestone-1/02-proxy-scope.md`) does not
> change any finding here. One connection worth drawing: this survey's central caution — that SDD
> degenerates into ceremony when specs are *guesses about product behavior* — does not apply to
> this project, where requirements are dictated by an external protocol and a stated isolation
> policy. That is the strongest structural argument for SDD in this repo.
>
> **Scope:** SDD as a *methodology* — its lineage, core claim, evidence base, economics, and design
> axes. Deliberately excludes tool-by-tool evaluation (OpenSpec, Spec Kit, Kiro, BMAD are cited only
> as evidence about the methodology; each gets its own survey).

## 1. Snapshot

*(Section adapted for a methodology survey per the brief — tool-specific rows replaced.)*

| Field | Value |
|---|---|
| Name | Spec-Driven Development (SDD); also "specification-driven development", "spec-first" |
| Origin (term) | Academic: Ostroff, Makalsky & Paige, *Agile Specification-Driven Development*, XP 2004 — a synthesis of TDD and Design by Contract [16]. The 2025 AI usage is a re-coinage, largely disconnected from that literature. |
| Origin (current wave) | AWS **Kiro** (July 2025) and GitHub **Spec Kit** (Sept 2025) popularized the term for agent workflows; Böckeler's Oct 2025 martinfowler.com piece gave it its working vocabulary [1] |
| Core claim | The specification is the source of truth; code is a derived/regenerable artifact. Intent is expressed once, in natural language, and durably. |
| Canonical taxonomy | **spec-first** (spec precedes code, then discarded) → **spec-anchored** (spec persists and is maintained alongside code) → **spec-as-source** (human edits only the spec; code is regenerated) [1][3] |
| Maturity | **Low-to-medium.** Term is ~12 months old in its current sense, definition still contested ("semantic diffusion" per Böckeler [1]). No standardized metrics; DORA has acknowledged the topic but published no SDD-specific findings [21]. |
| Notable proponents | GitHub / Microsoft Developer Division [7]; AWS (Kiro); Thoughtworks (qualified — Böckeler is a careful describer, not an advocate); Deepak Babu Piskala (arXiv survey [3]) |
| Notable critics | François Zaninotto / Marmelab ("The Waterfall Strikes Back") [8]; Nils Kjellman [10]; Isoform [5]; Augment Code [6]; Simon Martinelli (enterprise) [11]; a broad plurality of Hacker News commenters [13][14][15] |
| Ecosystem size (proxy) | GitHub Spec Kit ≈125.8k stars as of 2026-08 [2] — a strong *awareness* signal, a weak *retention* signal (see §6) |
| Evidence base quality | **Weak.** No RCT of SDD itself. Best available: one multivocal literature review with a 14-engineer uncontrolled pilot [4], one process taxonomy paper [17], and a large volume of n=1 practitioner reports. |
| Adoption trajectory (2026) | Peaked ~Q4 2025; visible deflation through 2026. Spec Kit went a month without commits by Feb 2026 [14]; an Aug 2026 "What happened to SDD?" thread drew agreement that "all the SDD tools have kinda fallen off" [13]. The *practices* survived; the *frameworks* largely did not. |
| Greenfield vs brownfield bias | Strongly greenfield-biased in tooling and in every tutorial; brownfield is where practitioners report the *most* residual value but where full-system specs are infeasible [11][21] |

## 2. One-paragraph thesis

SDD's bet is that the bottleneck in agent-assisted software development is no longer typing code but
*conveying and preserving intent*. Coding agents can produce plausible code faster than a human can
specify what they want, and they lose the thread across sessions, so the argument goes: stop throwing
prompts away. Write the intent down as a structured, version-controlled, natural-language artifact —
requirements, constraints, acceptance criteria, edge cases — review *that* instead of reviewing a
thousand lines of diff, and let the agent derive the implementation from it. In its strong form the
claim is an abstraction claim: the spec is to the code what the C source is to the assembly, and we are
witnessing a rise in the level at which humans program. In its weak (and far more defensible) form it
is a claim about *context engineering*: agents behave better with persistent, structured, curated
context than with ephemeral chat, and a repo-resident markdown spec is a cheap way to supply it. Almost
all the credible reported benefit belongs to the weak form; almost all the credible criticism targets
the strong form.

## 3. Workflow / artifact model

The methodology-level loop, stripped of tool branding, has five stages. Microsoft's seven-step framing
(Constitution → Specify → Clarify → Plan → Tasks → Implement → Validate) [7] and Spec Kit's command set
[2] are the most-copied instantiation; Kiro compresses to three documents (Requirements → Design →
Tasks) [1].

1. **Ground rules** — a persistent, project-wide document of principles, stack constraints, and
   non-negotiables ("constitution", `AGENTS.md`, `CLAUDE.md`). Written once, amended rarely.
2. **Specify** — a per-change artifact describing *what* and *why*, not *how*. Behavior, acceptance
   criteria, edge cases, out-of-scope.
3. **Clarify / Plan** — the agent surfaces ambiguities and proposes a technical design against the
   existing codebase. This is the stage practitioners rate highest (see §5).
4. **Decompose** — the plan becomes an ordered, checkable task list, providing resumability across
   context windows.
5. **Implement & validate** — the agent executes tasks; tests and acceptance criteria gate completion;
   the spec is either archived (spec-first), updated (spec-anchored), or re-run (spec-as-source).

```
repo/
├─ AGENTS.md | CLAUDE.md | .specify/memory/constitution.md   # ground rules, persistent
├─ specs/                                                     # the contested directory
│  ├─ 001-feature-name/            # "full spec" model (Spec Kit, Kiro)
│  │  ├─ spec.md                   #   what & why
│  │  ├─ plan.md                   #   technical design
│  │  ├─ research.md               #   investigation notes
│  │  ├─ data-model.md
│  │  ├─ contracts/                #   API/protocol contracts
│  │  ├─ quickstart.md
│  │  └─ tasks.md                  #   ordered checklist
│  └─ ...
├─ openspec/                       # "delta spec" model (OpenSpec)
│  ├─ specs/                       #   current truth, capability-scoped
│  ├─ changes/<id>/                #   ONLY the diff: proposal.md, tasks.md, specs/*-delta.md
│  └─ changes/archive/             #   merged deltas, historical lineage
└─ src/                            # the derived artifact (allegedly)
```

**Command surface (methodology-level):** the near-universal verb set is
`init/constitution` · `specify` · `clarify` · `plan` · `tasks` · `implement` · `validate/analyze` ·
`archive`. Tools differ in which are enforced, which are optional, and whether they are slash commands,
CLI subcommands, or plain prompt conventions.

**Artifact lifecycle — the central design decision.** Böckeler's three levels [1] are the right lens:

- **Spec-first**: spec is scaffolding for one change, then discarded or left to rot. Cheapest; this is
  what most teams actually do regardless of what their tool aspires to. Böckeler notes Spec Kit
  "creates a branch for every spec", which is *behaviorally* spec-first even though its rhetoric is
  spec-anchored [1].
- **Spec-anchored**: specs persist and are maintained as the system evolves. This is where the value
  proposition lives and where the maintenance tax lands.
- **Spec-as-source**: humans never touch code. Only Tessl and a few startups (e.g. Specific) genuinely
  attempt this [1]. Böckeler explicitly warns it risks combining "inflexibility *and*
  non-determinism" — the failure mode of Model-Driven Development, plus LLM nondeterminism [1].

The **delta vs. full** split is the second lifecycle axis. Full-spec tools restate the world per
feature and regenerate whole documents when an upstream phase changes; delta-spec tools describe only
the diff against a maintained baseline and archive merged deltas. Delta models measure roughly half the
token cost for equal or better outcomes (§9) and are the only tractable approach in brownfield [21].

## 4. What it enforces vs. what it suggests

**The honest methodology-level answer: SDD as a methodology enforces nothing.** It is a discipline
expressed in markdown that a nondeterministic executor is asked to honor. Enforcement is entirely a
property of the tool and the CI around it, and even then it is thin. Böckeler's finding after working
through three tools is the one to internalize: despite specifications, constitutions and checklists,
"the agent ultimately [does] not follow all the instructions" — agents simultaneously ignore
constraints and over-apply rules [1]. Zaninotto observed agents marking verification steps complete
without having written the unit tests [8].

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | Weak. Some CLIs refuse `/plan` before `/specify`, and OpenSpec ships a `validate --strict` schema check. Nothing stops a human or agent editing `src/` directly. | Universally. The whole "spec first" norm is social. |
| Spec ↔ code traceability | Essentially never at methodology level. Only spec-as-source tools with 1:1 spec↔file mapping attempt it [1]. | Task checklists referencing spec sections; commit-message conventions. |
| Test/acceptance criteria | Only insofar as your own CI runs the tests. The *presence* of acceptance criteria in the doc is template-enforced; their *truth* is not. | "Every requirement must have acceptance criteria" — template prompt. |
| Review gate | Human review of the spec is a workflow step, never a blocker. Practitioners routinely rubber-stamp it — reviewers "treat the spec phase as a loading screen" [19]. | Yes, entirely. |
| Drift detection | **Nothing in mainstream SDD detects spec-code drift.** This is the single largest unenforced concern and is named as a recurring risk by the one comparative academic study [17]. | "Update the spec when you change the code." Böckeler's proposed remedy is periodic agent "garbage collection" runs to hunt inconsistencies — an admission that drift is otherwise unchecked [18]. |

## 5. Strengths

These are the claims that survive contact with skeptical sources.

- **The clarify/plan step is the load-bearing one.** Across otherwise hostile threads, the consistently
  praised behavior is forcing the agent to explore the codebase and ask questions before writing.
  One practitioner: the agent's questions "are usually 80% useless but those 20% often do point me to
  stuff I have not considered" [14]. This is cheap and does not require a framework.
- **Durable context beats ephemeral prompts for multi-session work.** A written plan survives context
  compaction, session boundaries, and model switches. This is the "weak form" of SDD and it is largely
  uncontested — even critics keep a `plan.md` [13].
- **Review shifts left, where defects are cheaper.** Reviewing a 1-page intent document before 2,000
  lines of generated diff catches whole-approach errors, not typos. Faros AI data shows AI adoption
  producing 98% more merged PRs alongside a 91% increase in review time [4] — anything that moves
  review earlier attacks the actual bottleneck.
- **It suppresses unrequested features.** A recurring, specific report: agents "are getting very good
  also on inventing features… Specs are great to identify what has been implemented without explicit
  control" [13]. Bounding scope in writing is a real mechanism, not marketing.
- **Task decomposition gives resumability and parallelism.** An ordered checklist is a work queue that
  a fresh context can pick up, and it is a prerequisite for running multiple agents without collision.
- **Modest but real delivery-metric movement, in one pilot.** A Spec Kit pilot reported lead time
  8–12 → 6–9 days, code churn 12–18% → 6–10% of changed lines, rollbacks 2–4/mo → 0–1/mo [4]. Read
  §11 before weighting this: n=14, one org, no control group.
- **Onboarding.** Microsoft reports a brownfield project cutting onboarding from 2–3 weeks to days [7].
  Vendor-sourced, but plausible and consistent with the general value of good docs.
- **It legitimizes writing things down.** A soft benefit that shows up repeatedly: the discipline gets
  teams to produce architecture notes and acceptance criteria they would otherwise skip, and LLMs
  dramatically lower the cost of producing them [14].

## 6. Weaknesses / friction

This is the section that matters. The critical literature is more numerous, more specific, and better
evidenced than the promotional literature.

- **Ceremony wildly out of proportion to task size.** The canonical examples: Kiro turning a small bug
  fix into 4 user stories and 16 acceptance criteria, including "As a developer, I want the
  transformation function to handle edge cases gracefully" [1][15]; Marmelab generating **8 files and
  1,300 lines of Markdown to add a single date field** [8]. Practitioner verdict: "a sledgehammer to
  crack a nut" [15]. Neither Kiro nor Spec Kit right-sizes its workflow to problem size [1].
- **Double review.** Specs contain design decisions and often code; you review them in the spec, then
  review them again in the implementation [8]. SDD does not remove review work, it duplicates and
  relocates it — and it produces *more* prose to read, which is what humans skip.
- **Rubber-stamping collapses the entire value proposition.** If the human does not genuinely read and
  argue with the spec, SDD is strictly worse than prompting: you paid the token and latency cost for a
  gate that did not gate [19]. This failure is the default human behavior, not an edge case.
- **Spec-code drift is unsolved and agent-amplified.** "The only documentation you can 100% trust is
  the code itself… A stale spec misleads agents that don't know any better. They'll execute a plan that
  no longer matches reality, confidently" [6]. The structural argument is strong: every
  documentation-first initiative in software history has failed because it asks for continuous
  maintenance work that nobody sees and nobody rewards [6]. The comparative academic study lists
  spec-code drift first among recurring risks and finds **no framework** covers all six process
  dimensions [17].
- **Token cost is 2× for the heavy variants, with no measured quality gain.** Head-to-head on identical
  features: 57,740 vs 120,947 tokens (Test 1) and 91,729 vs 181,040 tokens (Test 2) for delta-spec vs
  full-spec workflows — and the more expensive one produced the *worse* result [12]. Independent
  supporting evidence: ETH Zurich's study of repo context files found LLM-generated context files
  **reduced task success ~3% while increasing inference cost >20%**, with agents spending 14–22% more
  reasoning tokens parsing documentation instead of solving the problem [20]. More instructions ≠
  better code.
- **It fails hardest where it's most needed: large codebases.** Zaninotto's sixth failure mode is
  diminishing returns as codebases grow [8]. Enterprise critique is blunter: these tools assume
  greenfield, a single developer, a free choice of stack, and no pre-existing requirements embedded in
  code and schemas [11]. Generating full specs for a large existing system exceeds practical context
  limits.
- **The waterfall objection is not merely rhetorical.** Detailed upfront specification locks in a
  design before the implementation has taught you anything; when reality forces spec revision, the
  claimed efficiency evaporates [8]. Böckeler's Model-Driven Development parallel is the sharpest
  version: MDD failed for reasons SDD has not addressed, and spec-as-source adds nondeterminism on
  top [1]. The best counterargument is honest but limited — a feedback loop measured in minutes is not
  waterfall in the way a 6-month one was [9] — and it only rescues the *lightweight* variants.
- **Specs record *what*, not *why*.** They cannot capture the reasoning behind assumptions or the
  constraints discovered mid-implementation, which is precisely the knowledge that makes future changes
  safe [5]. Detailed specs also create a false sense of completeness that discourages iteration [5].
- **Wrong abstraction level.** Current tools drift into field definitions, schemas, and signatures —
  producing code that is "structurally correct but misaligned with the actual intent" [5].
- **The abstraction claim doesn't hold.** The most incisive HN critique: you don't read assembly because
  the C→assembly abstraction is *complete and deterministic*. Natural-language spec → code is neither.
  "There's no way to consistently reconcile the spec with the code… it seems like most people seem to
  agree [code is] the only source of truth" [13]. Corollary: if the spec truly determined the code, it
  would be a programming language, and you would be back to learning what good code looks like — only
  now for specs, and nobody knows what a good spec looks like [15].
- **Team ergonomics are an afterthought.** Sequentially-numbered spec directories collide when two
  engineers create specs concurrently, and there is no automated check that merged code satisfies
  multiple concurrent specs [22]. SDD tooling was built for a solo developer.
- **Ecosystem deflation is itself evidence.** Spec Kit went >1 month without commits by Feb 2026 [14];
  by Aug 2026 the framing of the discussion had become "what happened to SDD?" [13]. What replaced the
  frameworks is instructive: hand-rolled `plan.md` files, `AGENTS.md`, `/plan` mode, and custom slash
  commands [13][14][15]. The practices diffused; the ceremony was discarded.
- **The reported-success cases are heavily n=1 and self-selected**, and the reported-failure cases are
  equally so. One documented failure: 10 days of Spec Kit execution ending with most tests failing and
  a broken build, followed by an equally long remediation, leaving the author with "low confidence" in
  the code [15]. Another: a developer reportedly burned $620 in credits and 310+ hours on a Kiro
  project estimated at 20–30 hours, reaching 50% completion [19] — single-source, treat as anecdote.

## 7. Fit signals

**Strong fit when:**

- The work is genuinely novel and under-determined, where the cost of the agent guessing wrong is high
  and a 30-minute clarification conversation is cheap insurance.
- Multi-session features that will outlive a context window, where a durable plan prevents re-derivation.
- A durable contract exists that multiple parties depend on — protocols, wire formats, public APIs.
  This is SDD's genuinely best domain: the spec is not redundant with the code because it constrains
  *other* implementations too.
- The team already writes design docs. SDD then costs nearly nothing incremental and adds agent
  readability.
- Onboarding humans or agents onto an unfamiliar codebase [7].
- You have deliberately picked **spec-anchored** and staffed the maintenance, or deliberately picked
  **spec-first** and accepted the specs are disposable. Böckeler's point stands: if you don't know which
  level you're practicing, you get the cost of the higher level and the benefit of the lower one.

**Poor fit when:**

- Single-line fixes, small refactors, dependency bumps. The pipeline cost exceeds the work [1][11].
- Exploratory work where you don't yet know what you're building; premature specification constrains
  learning [5].
- Throwaway prototypes and spikes.
- Requirements that are obvious and hard to misinterpret (routine CRUD) — elaborate specs add cost
  without reducing ambiguity.
- Very large existing codebases, without a delta-spec model. Full-system spec generation is
  impractical [11][21].
- Any situation where the human will not actually read the spec. Be honest about this one.
- Solo, short-lived projects with no maintenance horizon — the drift tax never gets repaid because
  there's no future to pay it back.

## 8. Rust / systems-software notes

**Direct evidence is thin.** A targeted search for SDD experience reports in Rust, embedded, or protocol
work returned nothing substantive. Essentially the entire practitioner corpus is web/CRUD: Razor Pages
UI modernization [23], chat app streaming/sessions [12], a date field on a blog [8], todo apps [15].
Every claim below is inference from the methodology's structure, not observed practice — treat it as
hypothesis to test, not finding.

**Where systems work should favor SDD:**

- **Protocol and wire-format work is SDD's natural home.** A Doris/MySQL protocol proxy has an external
  specification that genuinely exists independently of your code, that other implementations must
  interoperate with, and that is not derivable from reading your `src/`. Here "spec as source of truth"
  is literally true and predates the methodology — this is the OpenAPI/contract-first tradition, which
  has a decades-long track record that the AI wave inherits rather than invents [3].
- **Invariants and property-based testing are a better spec surface than prose acceptance criteria.**
  A `proptest`/`quickcheck` property is an executable, machine-checkable specification with no drift
  problem — it fails when the code diverges. The BDD counter-tradition raised exactly this on HN:
  a living specification "documents the features as code rather than verbose markdown files no one will
  read" [15]. For a Rust proxy, the strongest form of SDD available is: prose spec for *intent*,
  property tests and type-level constraints for *enforcement*.
- **Rust's compiler is a drift detector that markdown can't be.** Types, `#[non_exhaustive]`, exhaustive
  matching, and `unsafe` boundaries encode constraints the agent cannot silently violate. Böckeler's
  "harness engineering" framing — deterministic linters and structural tests as sensors on agent
  output [18] — is much cheaper to realize in Rust than in a dynamic language. `cargo clippy`,
  `cargo mutants`, and a strict `#![deny(warnings)]` posture do more real enforcement than any
  `constitution.md`.

**Where systems work should disfavor SDD:**

- **Performance and concurrency requirements resist natural-language specification.** "Handle 10k
  concurrent connections with p99 < 5ms" is a benchmark, not a spec paragraph; an agent cannot verify
  it by reading. Memory-layout choices, lock granularity, and async runtime decisions are exactly the
  "why" that specs structurally omit [5], and exactly where a plausible-looking implementation can be
  catastrophically wrong.
- **`unsafe`, lifetimes, and FFI boundaries need code-level review regardless.** SDD's promise of
  "review the spec instead of the diff" does not transfer; you must read the diff.
- **Rust's compile-cycle economics change the loop.** The "iterate fast with the agent" alternative to
  SDD [8][10] is cheaper in JS/Python than in Rust, where a failed build costs real wall-clock. This is
  a genuine argument *for* more upfront planning in Rust than the critics assume — the cost of a wrong
  approach discovered late is higher.
- **Unknown:** whether current agents can maintain protocol-level invariants (state machine
  correctness, packet framing, backpressure) from a prose spec across a multi-file Rust codebase. No
  evidence found either way. This is the single most important thing to test hands-on before
  committing.

## 9. Cost & lock-in

*(Section adapted for a methodology survey.)*

- **Money (methodology-level):** the methodology is free; the tools are overwhelmingly free and
  open-source (Spec Kit, OpenSpec, BMAD). Kiro and Tessl are commercial. The real spend is inference,
  not licenses.
- **Token cost:** the best-measured figure is **roughly 20–40% higher API spend per feature** for
  lightweight SDD versus ad-hoc prompting, because the agent re-reads spec + plan + tasks on many turns.
  Heavy full-spec workflows are worse: measured **~2× the tokens of delta-spec workflows on identical
  features (57.7k → 120.9k; 91.7k → 181.0k), with worse output** [12]. Reported wall-clock/dollar for
  one real feature: 3–4h planning ($25–30) plus ~1 day implementation ($45–70) [24]. BMAD-style
  multi-agent workflows report ~31.7k tokens per run and $800–2,000+/month per developer on large
  projects [12] — verify independently before believing.
- **Human review cost — the dominant and most-underestimated line item.** SDD adds a spec-review pass
  *without removing* the code-review pass [8]. If a spec review takes 20 minutes and does not shorten
  code review, SDD is net-negative on human time regardless of token economics. The break-even
  condition is narrow and specific: **the spec review must catch approach-level errors that would
  otherwise have been caught only after implementation.** Track that hit rate; if it's low, stop.
- **Drift maintenance burden:** unbounded and compounding, and the reason spec-anchored is rarely
  achieved in practice. Grows with system complexity [5], is invisible and unrewarded work [6], and its
  failure mode is worse than no documentation because agents follow stale specs confidently [6]. Budget
  for it explicitly or commit to spec-first (disposable specs) and stop pretending.
- **Lock-in:** **low, by construction.** Every mainstream SDD artifact is plain markdown in your repo
  under version control. This is the methodology's most underrated property and the strongest argument
  for a cheap trial: you keep the specs whatever you do next. Exceptions: Kiro (IDE-bound), Tessl
  (spec-as-source, where abandoning the tool leaves you owning generated code you never reviewed —
  genuine lock-in).
- **Exit path:** for markdown-based SDD, exit is deleting slash commands and keeping the documents; the
  observed 2026 industry trajectory *is* this exit, toward hand-rolled `plan.md` and `AGENTS.md`
  [13][14]. The real lock-in risk is **methodological**: a team that has offloaded design thinking to a
  spec-generating agent and rubber-stamped the output has lost the skill, not the files. One critic
  practices "Manual Mondays" — one AI-free day a week — specifically to prevent this atrophy [10]; the
  literature review independently reports skill atrophy in juniors who gain 30–40% speed [4].

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://martinfowler.com/articles/exploring-gen-ai/sdd-3-tools.html | analysis (Böckeler, Thoughtworks) | 2025-10-15 | **Most important single source.** Origin of spec-first/anchored/as-source taxonomy; MDD parallel; "semantic diffusion"; sledgehammer critique |
| 2 | https://github.com/github/spec-kit | repo/docs | fetched 2026-08-08 | ≈125.8k stars; 7-command workflow; no stated "when not to use" |
| 3 | https://arxiv.org/abs/2602.00180 | preprint (Piskala) | 2026-01-30 | *SDD: From Code to Contract in the Age of AI Coding Assistants.* Three rigor levels; decision framework; BDD→Spec Kit lineage. Abstract-level read only |
| 4 | https://arxiv.org/html/2605.01160v1 | preprint (Farrag, Univ. East London) | 2026-05 | Multivocal review, 67 sources, Jan 2022–Apr 2026. METR −19%; DORA 25% adoption ↔ −7.2% stability; Faros +98% PRs / +91% review time; Spec Kit pilot metrics. **Self-declared limits: single-reviewer, n=14, no control group** |
| 5 | https://isoform.ai/blog/the-limits-of-spec-driven-development | critique | 2026 | Four failure modes: maintenance tax, missing "why", false completeness, wrong abstraction level. No quantitative data |
| 6 | https://www.augmentcode.com/blog/what-spec-driven-development-gets-wrong | critique (vendor) | 2026 | "The only documentation you can 100% trust is the code itself"; stale specs mislead agents confidently. Vendor-motivated but argument stands |
| 7 | https://developer.microsoft.com/blog/spec-driven-development-ai-native-engineering/ | vendor advocacy | 2026-06-10 | 7-step lifecycle; onboarding 2–3 weeks → days; concedes "right-size", "not every change needs the full lifecycle" |
| 8 | https://marmelab.com/blog/2025/11/12/spec-driven-development-waterfall-strikes-back.html | hands-on critique (Zaninotto) | 2025-11-12 | **Best critical writeup.** 8 files / 1,300 lines of markdown for one field; six named failure modes; "Natural Language Development" alternative |
| 9 | https://news.ycombinator.com/item?id=45935763 | HN discussion (191 comments) | 2025-11 | Counterarguments to [8]: short-iteration SDD isn't waterfall; "20x ROI on well defined, comprehensive, end-to-end acceptance tests" |
| 10 | https://augmentengineer.com/your-spec-driven-workflow-is-just-waterfall-with-extra-steps/ | critique (Kjellman) | 2026 | "artifact sprawl"; `/reason` `/question` `/tdd` as lightweight replacement; "Manual Mondays" skill-atrophy practice |
| 11 | https://martinelli.ch/why-spec-driven-development-tools-fail-in-the-enterprise/ | critique | 2026 | Five enterprise failure modes: greenfield assumption, single-dev model, stack inflexibility, spec-from-scratch, narrow scope |
| 12 | https://medium.com/it-chronicles/is-your-safe-choice-burning-your-budget-1cfddf8782e4 | measured comparison | 2026 | Token counts on identical features (GPT-5.2/Codex): 57.7k vs 120.9k; 91.7k vs 181.0k. "More structure increased cost without improving outcomes" |
| 13 | https://news.ycombinator.com/item?id=49182353 | HN (Ask HN) | **2026-08-05** | "All the SDD tools have kinda fallen off but all of the complaints remain." Best statement of the abstraction critique. **Only 3 points — low engagement, weight accordingly** |
| 14 | https://news.ycombinator.com/item?id=46864948 | HN (Ask HN) | 2026-02-03 | Spec Kit ">1 month without commits"; brownfield still-users; ~750-line file sweet spot; cites Cockburn and Ostroff as prior art |
| 15 | https://news.ycombinator.com/item?id=45610996 | HN (128 pts) on [1] | 2025-10-16 | Richest practitioner thread. 10-day Spec Kit run ending in failing tests; "sledgehammer to crack a nut"; Kiro deleting code; BDD-as-living-spec counterproposal; "if the spec is truly the source… you've invented a formal programming language" |
| 16 | Ostroff, Makalsky & Paige, *Agile Specification-Driven Development*, XP 2004 | peer-reviewed (via Wikipedia) | 2004 | Original academic coinage: tests + contracts as complementary specifications. Not read in full; cited via https://en.wikipedia.org/wiki/Spec-driven_development |
| 17 | https://arxiv.org/abs/2606.04967 | preprint (de Macedo) | 2026-06-03 | Six-dimension process taxonomy across Spec Kit, OpenSpec, BMAD, GSD, Spec Kitty, Reversa. **"No framework strongly covers all six dimensions."** Risks: spec-code drift, over-reliance, platform dependence, no process benchmarks |
| 18 | https://martinfowler.com/articles/exploring-gen-ai/harness-engineering-memo.html | analysis (Böckeler) | 2026-02-17 | Post-SDD framing: deterministic linters/structural tests as sensors; agent "garbage collection" to fight documentation entropy. Implies specs alone are insufficient |
| 19 | https://blog.vibecoder.me/amazon-kiro-spec-driven-development-reviewed | review | 2026 | Spec review treated as "a loading screen"; $620 / 310h anecdote. **Single-source, low reliability** |
| 20 | https://www.marktechpost.com/2026/02/25/new-eth-zurich-study-proves-your-ai-coding-agents-are-failing-because-your-agents-md-files-are-too-detailed/ | secondary report of ETH Zurich study | 2026-02-25 | LLM-generated context files: ~−3% success, >20% cost, 14–22% more reasoning tokens. **Read via secondary source; primary paper not fetched** |
| 21 | https://www.augmentcode.com/guides/spec-driven-development-brownfield-codebases | vendor guide | 2026 | Change-level vs full-system specs in brownfield; notes DORA has published no SDD-specific findings and no standardized metrics exist |
| 22 | https://github.com/github/spec-kit/discussions/2116 · /497 | issue tracker | 2026 | Concurrent-development pain: sequential spec numbering collides; no automated check that merged code satisfies concurrent specs |
| 23 | https://dev.to/incomplete_developer/openspec-spec-driven-development-failed-my-experiment-instructionsmd-was-simpler-and-faster-3a5d | hands-on report | 2026 | ~2h SDD run produced near-identical-to-original UI; a plain `Instructions.md` was faster, cheaper, better |
| 24 | https://ypyl.github.io/programming/2026/06/03/openspec-vs-spec-kit-sdd.html | hands-on comparison | 2026-06-03 | Real feature: 3h/$25 + 1d/$70 vs 4h/$30 + 1d/$45; delta (~4 files) vs full (7+ files) artifact volume |
| 25 | https://martinfowler.com/articles/structured-prompt-driven/ | analysis (Zhang & Xia) | 2026-04-28 | SPDD as successor framing; explicitly adds bidirectional sync because SDD lacks it |

## 11. Confidence & gaps

- **Confidence: medium on the shape of the debate, low on any effect size.** The qualitative picture is
  consistent across ~15 independent sources with opposing incentives, which is the strongest signal
  available. Directionally I am confident that (a) the clarify/plan step is the value, (b) heavy
  ceremony is net-negative for small changes, (c) drift is unsolved, and (d) the framework wave
  deflated during 2026 while the practices persisted. I am **not** confident in any number in this
  document.

- **Unverified / weakly-sourced claims (do not act on these alone):**
  - The Spec Kit pilot metrics in [4] come from an uncontrolled 14-engineer single-org study whose
    author lists the absence of a control group and single-reviewer screening as limitations. Treat as
    illustrative, not evidence.
  - The ETH Zurich context-file numbers [20] were read via a secondary report; the primary paper was
    not retrieved and the figures are not independently confirmed.
  - Vendor claims surfaced in search but not verified against primary sources — GitHub's
    "order-of-magnitude fewer regenerate-from-scratch cycles" and AWS Kiro's "40-hour features in
    under 8 hours" — are **excluded from this survey's body** for that reason. They should not be cited.
  - The $620 / 310-hour Kiro figure [19] is single-source and uncorroborated.
  - Token comparisons [12][24] are n=1 or n=2 per tool, on web-app tasks, with models (GPT-5.2/Codex)
    that differ from what this project would use. Ratios are more trustworthy than absolutes.
  - Ostroff/Makalsky/Paige (2004) [16] was cited via Wikipedia and HN, not read.
  - The two Aug-2026 HN threads are low-engagement (3 and 6 points); "SDD has fallen off" is a
    plausible reading of the ecosystem, not a measured fact.

- **Open questions a decision-maker must test hands-on:**
  1. **Does spec review actually catch approach-level errors on *this* codebase?** Instrument it: for
     10 changes, record whether the spec review changed the approach. If under ~30%, the ceremony is
     not paying for itself.
  2. **Can an agent hold protocol-level invariants (MySQL wire protocol framing, connection state
     machine, backpressure) from a prose spec across a multi-file Rust codebase?** No evidence exists
     either way. This is the highest-value experiment for a Doris proxy.
  3. **Does a property-test suite (`proptest`) plus strict types substitute for a prose spec as the
     enforcement layer** — i.e., can we get spec-anchored *behavior* without spec-anchored *documents*?
     This is the most promising Rust-specific hypothesis and nobody appears to have tested it publicly.
  4. **What is the real drift rate?** Ship 5 features spec-anchored, then measure how many specs are
     stale 4 weeks later without deliberate intervention.
  5. **Delta vs. full specs at equal quality** — the ~2× token gap [12] is measured on web apps only;
     confirm it holds where the spec must encode protocol constraints.
  6. **Solo-developer economics.** Nearly all reported benefit is coordination benefit. If this project
     stays single-developer, does anything survive besides `plan.md` and good `CLAUDE.md`? The 2026
     trajectory suggests: not much.
