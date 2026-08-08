# Survey: {Title}

> **Template version:** 1.0 · **Surveyed:** {YYYY-MM-DD} · **Author:** {agent/human}
> **Scope:** {one line — what this survey covers and deliberately excludes}

## 1. Snapshot

| Field | Value |
|---|---|
| Name | |
| Vendor / owner | |
| License | |
| First release | |
| Latest version (as of survey date) | |
| Popularity signal | {stars, downloads, notable adopters} |
| Maintenance signal | {commit cadence, open issue ratio, release cadence} |
| Primary interface | {CLI / IDE / slash commands / MCP / web} |
| Agent compatibility | {Claude Code, Cursor, Copilot, Codex, …} |
| Language/stack neutrality | |
| Greenfield vs brownfield bias | |

## 2. One-paragraph thesis

{What problem does it claim to solve, and what is its core bet? Written so a
reader who knows nothing about it understands the pitch in 60 seconds.}

## 3. Workflow / artifact model

{The actual loop a developer runs. Name the commands, the files produced, and
where they live in the repo. Include a fenced block showing the artifact tree.}

```
{artifact tree}
```

**Command surface:** {list the primary commands/slash commands}

**Artifact lifecycle:** {how specs are created → reviewed → implemented →
archived/retired. Is there a delta model? Version control story?}

## 4. What it enforces vs. what it suggests

| Concern | Enforced (tooling blocks you) | Suggested (prompt-level only) |
|---|---|---|
| Spec exists before code | | |
| Spec ↔ code traceability | | |
| Test/acceptance criteria | | |
| Review gate | | |
| Drift detection | | |

## 5. Strengths

- {bullet — concrete, not marketing}

## 6. Weaknesses / friction

- {bullet — including ceremony cost, token cost, failure modes observed by users}

## 7. Fit signals

**Strong fit when:** {bullets}

**Poor fit when:** {bullets}

## 8. Rust / systems-software notes

{Does it assume web/CRUD? How well does it handle protocol work, concurrency,
performance budgets, invariants, property/mutation testing? Explicit "unknown"
is acceptable and preferred over speculation.}

## 9. Cost & lock-in

- **Money:** {free / paid tiers}
- **Token cost:** {rough per-feature overhead}
- **Lock-in:** {what happens if you stop using it — are artifacts plain markdown?}
- **Exit path:** {how hard to migrate off}

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | {url} | {docs/repo/blog/hands-on report} | | |

## 11. Confidence & gaps

- **Confidence:** {high / medium / low} — {why}
- **Unverified claims:** {what could not be confirmed from primary sources}
- **Open questions:** {what a decision-maker would still need to test hands-on}
