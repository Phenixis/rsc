---
feature: <feature-slug>
summary: <one line>
crates: [<crate>]
tests:
  - <crate>/src/<module>/tests.rs
  - <crate>/tests/<name>.rs
---

# <Feature title>

## Purpose
Why this exists, in a few lines. What the user or the calling code gets out of it.

## Public API
Exact signatures the tests use. DEV implements these names and shapes as written.

```rust
// module path, then the items
pub struct Example { /* fields the tests need */ }
impl Example {
    pub fn new(/* ... */) -> Self;
}
```

## Behaviors
Each one is observable and testable. Cover errors, edge cases and wire formats too.

- **B1.** <statement>
- **B2.** <statement>

## Test wiring
What DEV must add so that the tests compile (SPEC cannot edit application files):

- In `<crate>/src/<module>.rs`: `#[cfg(test)] mod tests;`
- Dev-dependencies: `<name> = "<version>"`

## Out of scope
What this feature deliberately does not do.

## Open questions
Anything unsettled. Remove entries when answered.

## Change log
- <YYYY-MM-DD>: initial spec and tests.
