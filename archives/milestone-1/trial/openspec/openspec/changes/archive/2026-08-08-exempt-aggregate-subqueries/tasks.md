## 1. Classifier

- [ ] 1.1 Add a parent-shape classifier returning aggregate-only / not / undetermined
- [ ] 1.2 Treat a projection of aggregate calls and literals as aggregate-only
- [ ] 1.3 Disqualify a parent carrying `GROUP BY`
- [ ] 1.4 Disqualify a parent carrying a `WINDOW` clause or window functions in its projection
- [ ] 1.5 Map the undetermined result to "cap it", so unknown shapes keep the old behaviour

## 2. Rewriter integration

- [ ] 2.1 Pass the parent context down the AST walk so a child node can be classified
- [ ] 2.2 Skip cap insertion for derived tables classified aggregate-only
- [ ] 2.3 Apply the same skip to CTE bodies and set-operation branches under an aggregate-only parent
- [ ] 2.4 Confirm the top-level cap is still applied when an inner node is exempted

## 3. Observability

- [ ] 3.1 Add exempted nodes and their reason to the rewrite record
- [ ] 3.2 Add a counter for aggregate-only exemptions, separable from write-statement exemptions

## 4. Verification

- [ ] 4.1 One test per scenario in the `Classify a sub-query as aggregate-only` requirement
- [ ] 4.2 One test per scenario in the modified `Cap row-producing sub-queries` requirement
- [ ] 4.3 Replace the previous change's test that asserted `SELECT COUNT(*) FROM (SELECT id FROM big) t` returns the cap; it now asserts the true count
- [ ] 4.4 Regression pass: every previously-capped shape that is not aggregate-only still carries its cap
- [ ] 4.5 Re-run the captured production query log and report the exemption rate against the predicted one in nine
