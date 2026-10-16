# ADR 0004 — License

## Status

Accepted

## Context

The product spec listed the license as an open decision: `MIT OR Apache-2.0` as in MCP Studio, or a
copyleft license to keep forks open. The decision had to be taken before the first public commit.
The workspace `Cargo.toml` has carried `license = "MIT OR Apache-2.0"` since the first release, so
the choice was made in practice but never recorded.

## Decision

Daimos is licensed under `MIT OR Apache-2.0`, the dual license that is the convention of the Rust
ecosystem.

## Alternatives considered

- **Copyleft (GPL-3.0 or MPL-2.0)** to keep forks open. Rejected: it would stop the code being
  reused in the other GravionLabs projects under permissive licenses, and extension authors would
  ask whether their extensions are affected.
- **MIT only or Apache-2.0 only.** Rejected: the dual license is what Rust crates expect, and
  Apache-2.0 adds the patent grant that MIT lacks.

## Consequences

- Every crate and package inherits the license from the workspace manifest; `cargo deny` checks that
  dependencies are compatible with it.
- The repository still lacks `LICENSE-MIT` and `LICENSE-APACHE` files and a license line in the
  README; adding them is a follow-up of this ADR.
- Extensions are separate works: the license of an extension is its author's choice, and the
  catalog (M4) records it per entry.
