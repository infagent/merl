# Synthetic visible-package fixture

The planner supplied these value-sanitized templates and generator as an interface sample for #131. All prompts, source text, answers, identities, reviewer strings and hashes are invented. They demonstrate two questions, three question-cutoff units and two shared prefixes. No real evaluation package or held-out material is included.

`generate.py OUTPUT` creates a new deterministic package. Historical pending/zero proposal fields intentionally remain unchanged; the separately bound scope overlay records simulated approval. Preparation examples contain non-SQLite placeholder authorities and illustrative usage, not benchmark attestations. The categorical response example proposes the shape now implemented by the harness; the approved package did not previously register an executable JSON grader protocol.

`rebind.py PACKAGE` is an adversarial test helper. It updates transport references after a mutation while preserving the inventory/rules pins, so tests exercise semantic integrity checks beyond an outer checksum failure. Never use it to repair an approved package.
