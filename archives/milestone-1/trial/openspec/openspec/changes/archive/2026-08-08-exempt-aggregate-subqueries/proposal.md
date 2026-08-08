## Why

`add-limit-injection` shipped with a known-wrong case, documented and accepted at the time: `SELECT COUNT(*) FROM (SELECT id FROM events) t` returns 200 instead of the true count. The rewrite records added by that change now let us see how often it happens, and it is the single most common shape in the BI traffic - roughly one in nine rewritten statements caps a derived table whose parent does nothing but aggregate over it.

Those queries return one row. Capping their inner scan buys no protection the outer bound does not already provide, and it produces confidently wrong numbers on dashboards. The evidence we said we would wait for has arrived.

## What Changes

- A derived table, CTE body, or set-operation branch whose parent consumes it **only** through aggregate functions is no longer capped.
- The exemption is deliberately narrow. If the parent's projection contains any non-aggregate output column, or the parent has a `GROUP BY`, the sub-query is still capped as before - a grouped query can return arbitrarily many rows and is exactly the case the cap exists for.
- **Not addressed here**: an exempt sub-query is now genuinely unbounded, which is a real load vector. Doris' own query timeout is the only remaining backstop. A cost-based ceiling for exempt scans is a separate problem.
- The rewrite record gains the exempted nodes and the reason, so an exemption is as auditable as a cap.

## Capabilities

### New Capabilities

<!-- None. This change adjusts requirements in an existing capability. -->

### Modified Capabilities

- `query-limit-injection`: sub-query capping gains an aggregate-only exemption, and the audit record must report exemptions alongside caps.

## Impact

- Rewrite stage only: a new classification pass over the parent query before deciding whether to cap a child node. No configuration, connection, or authentication behaviour changes.
- Behaviour change visible to clients: aggregates over derived tables start returning correct results, which will move numbers on existing dashboards. Anyone who has built on the capped values will see a jump.
- Removes task 6.3 from the previous change, which pinned the wrong count as tested behaviour.
