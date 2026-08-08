# Discussion: Choosing an SDD tool for this project

**Date:** 2026-08-08 · **Status:** in progress (surveys complete, trial written, tool not yet chosen)

## Decision framing

The repo was originally named `study-openspec-doris-proxy`, which presumed the
answer. It was renamed to **`study-sdd-doris-proxy`** on 2026-08-08 to keep the
tool choice genuinely open.

The interview also inverted the framing:

> **SDD is the deliverable. The Doris proxy is the exercise.**

This changes the selection criteria. Normally you pick the SDD tool with the
least friction that still prevents agent drift. Here, a tool that *hides* the
method teaches nothing — the mechanics need to be visible.

## Requirements gathered

| # | Requirement | Answer | Implication for tool choice |
|---|---|---|---|
| R1 | Project goal | Learn SDD; practice via Doris proxy | Pedagogical clarity > raw efficiency |
| R2 | Team shape | Solo | No human review gate; specs are agent context + own memory |
| R3 | Tooling | Claude Code CLI; wants multi-agent portability | Must be plain markdown + slash commands. **Rules out IDE-locked tools.** |
| R4 | Prior pain | None — proactive adoption | ⚠️ Highest abandonment risk factor. See below. |
| R5 | Learning mode | One tool, deeply, end-to-end | No tool-hopping. Commit to one for the whole build. |
| R6 | Ceremony budget | Moderate — spec+plan+tasks OK; personas and 7-file features too much | **Rules out multi-persona frameworks.** Spec Kit is borderline. |
| R7 | Proxy scope | Undecided — wants help scoping | Scoping becomes the first SDD exercise (good: it's a real spec task) |
| R8 | Cost stance | Open source only | **Rules out all commercial entrants.** |

## Candidates eliminated before research completed

| Tool | Eliminated by | Reason |
|---|---|---|
| AWS Kiro | R3, R8 | IDE-native, account-gated, paid tiers |
| Google Antigravity | R3, R8 | IDE-native |
| Cursor spec features | R3 | IDE-bound; user is CLI-first |
| Augment Cosmos | R8 | Commercial |
| Tessl | R8 | Commercial |
| ZeroShot | R8 | Commercial |
| CodeMySpec | R8 | Commercial |
| BMAD-METHOD | R6 | Multi-persona ceremony exceeds stated budget |

**Surviving shortlist:** OpenSpec · GitHub Spec Kit · "no framework" baseline
(CLAUDE.md + `docs/specs/` + Claude Code plan mode / skills)

## Open tension to resolve

**R1 vs R6.** Learning SDD deeply argues for explicit, visible phase
structure — you cannot study an anatomy you never see. Spec Kit's
constitution → specify → clarify → plan → tasks → implement makes every
organ of the method legible. But its per-feature file count is exactly what
R6 flags as "too much."

OpenSpec's delta model is lighter and more sustainable solo, but it is
designed as brownfield-first and fluid — it deliberately *removes* the phase
gates that are arguably the thing worth learning.

**R4 is the real risk.** Adopting process without a felt pain is the classic
setup for abandonment around week three. Whatever is chosen should be paired
with a deliberately-chosen first milestone painful enough to justify it.

## Decided

**D1 — The unspecifiable half gets ADRs, not specs.** (2026-08-08)
Roughly half the project — async task topology, cancellation safety, buffer
ownership, error-enum shape — is discovered against the borrow checker, and a
spec written for it is wrong by the second refactor. These get **Architecture
Decision Records written after the fact** in `docs/adr/`: immutable, one file
per decision, superseded rather than edited. Because they record history rather
than intent, they cannot drift. Specs stay for external contracts only.

## Still open

**D2 — the tool itself.** User asked to see a trial before committing.
`archives/milestone-1/trial/` now holds the same real spec (MySQL capability-flag negotiation)
written in both OpenSpec and Spec Kit form. Findings recorded in
`archives/milestone-1/trial/README.md`; the three that were not visible from the surveys alone:

1. Spec Kit's mandatory user-story form dilutes invariants into personas.
2. Its mandated `data-model.md` and `contracts/` are near-useless for wire
   protocol work — the real model is a state machine, better expressed as Rust
   typestate than as Markdown.
3. The most valuable artifact in this project — `unknowns.md`, recording
   protocol constraints discovered from packet captures with provenance — is
   shipped by **neither** tool and must be hand-rolled either way.

Overarching finding: **the spec layer enforces nothing in either tool.** The
tests keep the code honest; the spec keeps the tests honest. Choose on which
artifacts will still get written at change fifteen, not on which sounds
stricter.

**D3 — first milestone.** Deferred until D2 is settled. Candidate favored:
handshake + auth negotiation, now **stronger** than when first proposed —
under passthrough auth the handshake is load-bearing for the entire isolation
story (salt relay, C2 in `02-proxy-scope.md`), not merely protocol table-stakes.

## Resolved since

**D4 — proxy scope.** Settled 2026-08-08; see `discussions/milestone-1/02-proxy-scope.md`.
L7 MySQL proxy, SQL rewriting for **tenant isolation + schema remapping**,
**1:1** backend connections, **passthrough** auth, **full parse with an
allowlist** and fail-closed on parse failure.

### How the scope changes D2

The narrowed scope moves the evidence, and it moves it in one direction.

- **The spec-shaped fraction went up**, from ~50–60% to ~70–75% by surface area.
  Dropping pooling, routing and read/write splitting removed the work that most
  resisted specification; adding the rewrite catalogue and statement allowlist
  added the work that most invites it.
- **The living-spec/delta case got stronger.** There are now *three* documents
  that grow forever rather than one: the compatibility matrix, the statement
  allowlist, and the rewrite-rule catalogue. Spec Kit has no spec lifecycle and
  no delta model; its regenerate cascade (#1059) is a worse fit than first
  assessed.
- **The constitution case also got stronger.** Tenant isolation introduces
  exactly the kind of non-negotiable cross-cutting rule a constitution exists to
  hold — fail closed on parse failure; rewrites must not change placeholder
  count, order, or result shape. This remains OpenSpec's one real gap.
- **A new requirement appeared that neither tool meets:** the ability to mark a
  requirement **security-relevant**, so a reviewer can see at a glance which
  rules cannot be relaxed. A house convention is needed regardless of choice.

Net: the recommendation of `01`'s earlier analysis — OpenSpec as the base with
the constitution concept grafted on — is *reinforced*, not changed. But the
grafted invariants file is now load-bearing rather than a nicety.
