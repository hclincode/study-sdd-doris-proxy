# Study: OpenSpec vs. GitHub Spec Kit

> **Date:** 2026-08-08 · **Author:** agent (Claude Opus 5)
> **Scope:** A head-to-head comparison of the two surviving shortlist candidates, from
> overview down to pros and cons across three scenario types. Synthesized entirely from
> material already in this repo — `surveys/openspec.md`, `surveys/github-spec-kit.md`,
> `surveys/domain-fit-rust-proxy.md`, and `archives/milestone-1/trial/`. No new research was performed; every
> figure traces back to those documents. **This study informs D2 in
> `discussions/milestone-1/01-sdd-tool-selection.md`; it does not settle it.**
>
> **Scope update 2026-08-08 (after this study was written).** The proxy scope narrowed to
> an L7 proxy doing **SQL rewriting for tenant isolation and schema remapping**, with 1:1
> backend connections, passthrough auth, and full parsing behind a statement allowlist that
> **fails closed** — see `discussions/milestone-1/02-proxy-scope.md`. Every comparison below still holds;
> three things shift, all in the same direction:
>
> 1. **The delta model matters more.** There are now three documents that grow forever, not
>    one: the compatibility matrix, the statement allowlist, and the rewrite-rule catalogue.
>    §1's distinction — snapshots vs. current truth — is the whole decision.
> 2. **The constitution matters more.** Tenant isolation introduces non-negotiable
>    cross-cutting rules (fail closed on parse failure; rewrites must not change placeholder
>    count, order, or result column shape). This is Spec Kit's one durable advantage and the
>    thing worth grafting onto OpenSpec.
> 3. **§5 gains a fourth gap** — see below.

## 1. The one-sentence difference

**Spec Kit is a phase pipeline over a feature; OpenSpec is a delta model over a living
spec store.**

Everything else follows from that. Spec Kit's unit of work is a numbered feature folder
that is created, filled through a fixed command sequence, and then never revisited.
OpenSpec's unit of work is a *change* that declares `ADDED / MODIFIED / REMOVED`
requirements against a canonical spec, and is merged and archived when done. One
accumulates snapshots; the other maintains a current truth.

## 2. Side-by-side overview

| | **OpenSpec** | **GitHub Spec Kit** |
|---|---|---|
| Owner | Fission AI (`Fission-AI/OpenSpec`) | GitHub, Inc. (`github/spec-kit`) |
| License | MIT | MIT |
| Version at survey | v1.8.0 (2026-08-05) | v0.16.1 (2026-08-07) |
| Popularity | 64,224 stars; 351k npm downloads/wk | 125,795 stars |
| Runtime dependency | Node ≥ 20.19 (global npm install) | Python via `uv` |
| Interface | CLI + Claude Code skills (`/opsx:*`); **no MCP** | CLI (`specify`) + Claude Code skills (`/speckit-*`) |
| Bias | Brownfield-first | Greenfield-first |
| Core mechanism | **Delta specs** merged into `openspec/specs/` at archive | **Phase chain** `constitution → specify → clarify → plan → tasks → implement` |
| Unique artifact | `changes/<name>/specs/**` delta + `changes/archive/` | `.specify/memory/constitution.md` |
| Lifecycle | Change → merge → archive; spec store stays current | Feature folder created, never retired |
| Files per unit of work | 3–4 | 7–9 |
| Hard enforcement | Markdown *structure* linting + merge engine | Script checking *file existence in order* |

### Artifact shape

```
OPENSPEC                                SPEC KIT
openspec/                               .specify/
├── config.yaml                         │   ├── memory/constitution.md   ← the good part
├── specs/<domain>/spec.md   ← truth    │   ├── scripts/                 ← the only gate
└── changes/<name>/                     │   └── templates/
    ├── proposal.md                     specs/001-<slug>/
    ├── design.md      (optional)       ├── spec.md      (user stories, FR-###, SC-###)
    ├── tasks.md                        ├── plan.md      (+ Constitution Check)
    ├── specs/<domain>/spec.md ← delta  ├── research.md  ┐
    └── → archive/<date>-<name>/        ├── data-model.md├ mandated by /speckit.plan
                                        ├── contracts/   ┘
                                        ├── quickstart.md
                                        └── tasks.md
```

## 3. What each actually enforces

This is where most comparisons go wrong, so state it plainly. Verified against source in
both surveys — `src/core/validation/validator.ts` for OpenSpec,
`scripts/bash/check-prerequisites.sh` for Spec Kit.

| Concern | OpenSpec | Spec Kit |
|---|---|---|
| Spec before code | Partial — `validate` errors on a change with no delta sections; nothing stops the agent editing source directly | Partial — `/plan` refuses without a feature dir, `/implement` without `tasks.md`; nothing stops direct source edits |
| Spec ↔ code traceability | **None.** Validator never reads your source tree | **None.** `FR-###` IDs are convention; `/analyze` is an LLM opinion |
| Test criteria | Structural only — errors if a requirement has zero `#### Scenario:` blocks | **None.** Template asks for Given/When/Then; nothing checks |
| Review gate | **None** | **None** — `/clarify`, `/checklist`, `/analyze` all documented optional |
| Drift detection | Weak and late (issue #1112: a `MODIFIED` against a non-existent header passes `--strict`, fails weeks later at archive) | **None deterministic** — `/analyze`, `/converge` are LLM comparisons with no exit code |
| Principles compliance | No slot for project-wide principles at all | Constitution Check re-run pre-Phase-0 and post-Phase-1 — but violations are *documented*, not blocked |

**Both tools enforce approximately nothing that matters.** OpenSpec's CLI is a Markdown
structure linter plus a merge engine. Spec Kit's only hard gate is a turnstile checking
that files exist in a given order. Quality, traceability, testing, and principle
compliance are prompt text in both.

The trial reached the same conclusion independently (`archives/milestone-1/trial/README.md`): the only two
lines in either artifact with real teeth were the `proptest` and `cargo-mutants`
annotations — and both point *out* of the spec system into Rust's toolchain.

> **The spec layer enforces nothing. The tests keep the code honest, and the spec keeps
> the tests honest.**

## 4. Pros and cons by scenario type

Three scenario types, chosen because they are the three the domain-fit survey's §3a table
already splits this project into. The tools invert their ranking between the first two,
and both lose the third.

---

### Scenario A — A long-lived external contract that accretes

*Examples from this project: the packet codec, the `COM_*` coverage matrix, the
compatibility matrix, error mapping. Rows 1–8 of domain-fit §3a. The defining property is
that the artifact is edited many times over the project's life and must always state
current truth.*

**OpenSpec — strong fit**

| Pros | Cons |
|---|---|
| The delta model is built for exactly this: change #15 writes `## MODIFIED Requirements` against a living table, not a rewrite | Issue **#1246** — two changes modifying the same requirement, archived sequentially, silently overwrote the first's scenarios. Closed, but the survey did not verify the fix |
| `openspec/specs/` stays the single current answer to "what does the proxy support today?" | The spec corpus grows without bound and nothing prunes it — OpenSpec's own repo carries ~60k tokens of accumulated spec after one year |
| `### Requirement:` / `#### Scenario:` maps ~1:1 onto integration tests with no translation loss (trial finding) | Issue **#1112** — a delta targeting a header that doesn't exist passes `validate --strict` and fails only at archive, often weeks later |
| Small artifacts: 3 files / 131 lines in the trial | Issue **#805** — agents don't flip `- [ ]` to `- [x]`, so task progress reporting is unreliable |

**Spec Kit — poor fit**

| Pros | Cons |
|---|---|
| `plan.md`'s Technical Context has real fields for performance goals and constraints | **No delta model and no archive step.** After 40 features you have 40 folders of stale plans with no signal about which describe current behavior |
| | Issue **#1059** (open) — no way to update a plan after the spec changes; you regenerate and lose hand edits. On a protocol project the understanding changes weekly |
| | The mandated `data-model.md` and `contracts/` are near-useless here. `contracts/` is documented for OpenAPI and WebSocket events; our contract is a byte layout |
| | The mandatory user-story form dilutes invariants into personas — see the trial's two `spec.md` files side by side |

**Verdict for A: OpenSpec, clearly.** Its central mechanism is the one this scenario
needs; Spec Kit's central mechanism actively fights it.

---

### Scenario B — A net-new, well-understood, feature-shaped increment

*Examples: config schema, admin/observability API, routing policy — anything where the
risk is "building the wrong thing" rather than "not knowing how to build it," and the
artifact is written once.*

**Spec Kit — strong fit**

| Pros | Cons |
|---|---|
| **The constitution is the single best artifact in either tool.** Project-wide invariants, re-checked per feature. OpenSpec has no equivalent slot — this is the piece worth stealing even if you reject the rest | Output-volume-to-value ratio is the central documented complaint: 689 lines of code alongside **2,577 lines of Markdown**, 33.5 min agent + **3.5 h human review** vs. a 24 min iterative baseline (Scott Logic, Nov 2025) — and it still shipped a bug |
| Every organ of the SDD method is legible: constitution, specify, clarify, plan, tasks, implement. Pedagogically the strongest — directly relevant to R1 | 7–9 files per feature; R6's stated ceremony budget is "spec+plan+tasks OK, 7-file features too much" |
| Real ordering enforcement — the agent cannot `/implement` without `tasks.md` | Standing context tax: ~32k tokens of command prompts, though Claude Code's skill-based progressive loading should blunt this (**unverified — cheapest high-value experiment available**) |
| `/speckit.analyze` demonstrably catches real contradictions (Rust hands-on report: a stateless-design principle violated by an in-memory feature, caught pre-implementation) | Discussion #1784 — "creates the illusion of work"; polished Markdown manufacturing false confidence |
| The `lean` preset is an official acknowledgement of the verbosity problem — 4 artifacts, "just the prompt, just the artifact" | Unresolved `CLAUDE.md` vs `constitution.md` overlap (issue #609, 24 reactions, no authoritative answer) |
| GitHub-backed; low bus-factor risk | Command surface is in flux — most blog posts describe an older one |

**OpenSpec — adequate fit**

| Pros | Cons |
|---|---|
| Far cheaper: ~3,500–8,000 tokens of artifact per change vs. Spec Kit's several-times-implementation cost | The delta model has nothing to bite on when there is no base spec — its central innovation idles on greenfield day one |
| Lower ceremony matches R6 directly | "Fluid not rigid" deliberately removes the phase gates, which for R1 is precisely the anatomy you were trying to study |
| | No constitution equivalent. Project-wide invariants have nowhere to live except `config.yaml` rules, which the docs admit are "injected into the AI prompt… without being enforceable checks" |

**Verdict for B: Spec Kit, if and only if you adopt the `lean` preset and skip the
optional gates.** At full ceremony the Scott Logic numbers are hard to argue with. Note
that R2 (solo) removes the main thing the heavier artifacts buy — a shared statement of
intent across people.

---

### Scenario C — Exploratory systems work discovered against the compiler

*Examples: async task topology, cancellation safety, buffer ownership and backpressure,
error-enum shape, allocation avoidance. Rows 11–14 of domain-fit §3a — roughly half the
project by surface area.*

**Both tools: poor fit. This is the clearest finding in the repo.**

| | OpenSpec | Spec Kit |
|---|---|---|
| Vocabulary for concurrency invariants | None. `writing-specs.md` offers zero guidance on throughput, latency percentiles, or resource ceilings | None. `data-model.md` is entity/field/relationship-shaped; the interesting model is a state machine and a task graph |
| Performance budgets | No slot | Technical Context has Performance Goals / Constraints / Scale fields — **a real advantage, but nothing verifies them** |
| Property / mutation testing | No test-runner awareness of any kind | No reference to `proptest`, `quickcheck`, fuzzing, `cargo-mutants`, or `loom` anywhere in templates |
| Documented Rust/systems use | Essentially none; what exists is inverted (Rust tools that *parse* OpenSpec, not Rust projects using it) | One data point — an Axum/Tokio service. It was a BMI calculator, i.e. CRUD-shaped |

Given/When/Then can express a functional protocol behavior cleanly. It expresses "no task
holds the connection lock across an await point" badly or not at all — that is a property
over all executions, not a scenario.

**Verdict for C: neither.** This is already decided as **D1** — the unspecifiable half
gets ADRs written after the fact in `docs/adr/`, immutable and therefore drift-proof. That
decision stands regardless of which tool wins D2, and it is the reason the tool choice
matters less than it first appears: it only governs ~50–60% of the surface area and
~25–35% of the effort.

---

## 5. What neither tool ships, and you need anyway

Four gaps, both tools — the first three from the trial, the fourth added by the narrowed scope:

1. **`unknowns.md`** — empirically-discovered protocol constraints with provenance
   (packet capture, MySQL source line). The documented example: handshake salt bytes must
   avoid ASCII 36 and stay within `[1,35] ∪ [37,127]`, a constraint absent from published
   docs and visible only in MySQL's C source. Arguably the single most valuable file in
   the project. Hand-rolled either way.
2. **A slot for invariants and budgets that anything checks.** Spec Kit has the fields;
   nothing verifies them. OpenSpec doesn't have the fields.
3. **Any link between a requirement and the test that proves it.** Neither tool has a
   traceability model. Both surveys confirm this independently.
4. **A way to mark a requirement *security-relevant*.** Once rewriting enforces tenant
   isolation, some requirements cannot be relaxed and others can, and a reviewer needs to
   tell them apart at a glance. Neither tool has a severity or criticality concept. A house
   convention is required regardless of choice — either a tag in the requirement text or a
   dedicated `isolation` spec domain that carries the rule by location.

   This gap is sharper than it looks. The whole isolation guarantee reduces to "no
   allowlisted statement reaches the backend un-predicated" — a *negative* requirement over
   all executions. Neither Given/When/Then nor the user-story form expresses negative
   universal properties well; both are built around enumerating positive cases. Expect to
   state it as an invariant and prove it with `cargo-mutants` plus negative tests, not with
   scenarios.

## 6. Extension points, if you want to close the gaps yourself

| Need | OpenSpec path | Spec Kit path |
|---|---|---|
| Add a `test-plan` or `unknowns` artifact | `openspec schema fork spec-driven <name>` — add artifacts with `requires:` deps, as the community `anvil` schema does | `.specify/templates/overrides/` — highest precedence in a documented layering order |
| Cut ceremony | Already low | `specify preset add lean` |
| Project-wide invariants | `config.yaml` rules (prompt injection only) | `constitution.md` (prompt-level, but re-checked per phase) |
| Actual enforcement | Your own CI: `openspec validate --all --strict` + `cargo clippy` + `cargo mutants` | Your own CI. Same. |

Note the symmetry of the last row. Whichever tool you pick, the enforcement you actually
get comes from `cargo`, not from the SDD tool.

## 7. Cost and exit

| | OpenSpec | Spec Kit |
|---|---|---|
| Money | Free, MIT, no account | Free, MIT, no account |
| Tokens per unit of work | ~3,500–8,000 typical; +1,000–4,200 per skill invocation; rising as the spec corpus grows | Several times the cost of just implementing the feature; `lean` cuts it by an unquantified amount |
| Lock-in | Very low — plain Markdown + one YAML | Very low — plain Markdown + scripts you own |
| Exit path | `npm uninstall -g`, delete `.claude/skills/openspec-*`; `openspec/` remains as docs | Delete `.specify/` and skill dir; `specs/` remains as docs |

Both are cheap to abandon. A three-month trial of either risks almost nothing but time —
which, given R4 (no felt pain, highest abandonment risk), is the resource actually at
stake.

## 8. Reading this against the open decision

D2 is still open in `discussions/milestone-1/01-sdd-tool-selection.md`, and this study does not close
it. What it does is sharpen the tension already recorded there:

- **R1 (learn SDD deeply) points at Spec Kit.** Its phase chain makes the method visible;
  the constitution is a genuinely good idea; you cannot study an anatomy you never see.
- **R6 (moderate ceremony) and the domain both point at OpenSpec.** The living-spec-store
  shape matches a compatibility matrix that will be edited for the project's whole life,
  and the trial showed Spec Kit's two mandated artifacts are the two least useful ones
  here.

The honest synthesis is that these are not fully in conflict, because the two tools'
strongest features are in **different layers**: Spec Kit's constitution is project-wide,
OpenSpec's delta model is per-change. A constitution is ~2 KB of Markdown and a house rule
that every proposal re-checks it — it does not require adopting Spec Kit's pipeline. That
combination is worth evaluating explicitly when D2 is decided, rather than treating the
choice as strictly either/or.

Whatever is chosen, **D3 still matters more than D2**: pick a first milestone painful
enough to justify the ceremony (handshake + auth negotiation is the favored candidate),
because R4 — adopting process without a felt pain — is the documented failure mode around
week three.

## 9. Confidence and gaps

**High confidence** on mechanism, artifact shape, enforcement semantics, versions, and
cost — all read from primary sources in the two surveys (repo docs, validator source,
GitHub/npm APIs, filed issues), plus the trial's own measurements on real domain content.

**Carried-forward gaps** (from the source surveys, not resolved here):

1. Claude Code's *actual* resident context cost for Spec Kit post-skills-migration. The
   ~32k figure is a byte-count derivation, not a measurement. Cheapest high-value
   experiment available.
2. Whether OpenSpec's #1246 silent-overwrite bug is genuinely fixed — the issue is closed
   but no closing commit or regression test was traced. Severity is spec data loss.
3. The `lean` preset's real output reduction is unquantified by any source.
4. No evidence of either tool used on a network proxy, wire protocol, or
   performance-critical systems project. Fit for this project is inferred throughout, not
   observed. **You would be an early adopter either way.**

## 10. Sources

All from within this repo; each carries its own primary-source citations.

| Document | Supplies |
|---|---|
| `surveys/openspec.md` | §§1–4, 6, 7 OpenSpec columns; issues #1112/#1246/#805; token figures |
| `surveys/github-spec-kit.md` | §§1–4, 6, 7 Spec Kit columns; issues #1401/#1059/#752/#609; Scott Logic and 40tude reports |
| `surveys/domain-fit-rust-proxy.md` §3a | The scenario split in §4; the ~50–60% / ~25–35% figures |
| `archives/milestone-1/trial/README.md` + `archives/milestone-1/trial/**` | §3's "enforces nothing" finding; §4A/B trial measurements; §5's three gaps |
| `discussions/milestone-1/01-sdd-tool-selection.md` | R1–R8; D1; the open D2/D3 framing in §8 |
