# ADR 0002 — Extension model

## Status

Accepted

## Context

Raycast's value is its extension ecosystem, and its extension API is TypeScript/React in a
Node-like runtime. A cross-platform daimos needs an extension model that is cheap to start with,
language-agnostic, fast enough for per-keystroke queries, and safe enough to install from a
catalog. Once extensions exist, the model is nearly impossible to change.

## Decision

Two runtimes over one data model and one manifest format:

1. **Script protocol**: an executable in any language that speaks JSON Lines over stdin/stdout.
   One-shot or long-running. No isolation; labeled as such in the UI. This is the "write a
   five-line Python command" path.
2. **WebAssembly components** run by **wasmtime** with the component model. A WIT interface
   defines `query` and `action` exports and a small host API. Capabilities (filesystem, network
   hosts, clipboard, keyring entries) are declared in the manifest, shown at install, and enforced
   through WASI preopens and an allow-listed `wasi:http`. This is the catalog path.

Both produce the same items, actions, and views, so the UI and the conformance suite do not know
which runtime answered.

## Alternatives considered

- **Embedded JavaScript runtime (Deno, QuickJS, Boa)** with a Raycast-like API: the most familiar
  authoring, but it ties the ecosystem to one language, a React-like UI layer would have to be
  reimplemented, and sandboxing a JS runtime with native bindings is harder than sandboxing wasm.
  JavaScript authors are served through `componentize-js` instead.
- **Script protocol only** (Alfred, Udaimos, Script Kit): simplest, but no sandbox, process spawn
  per query on the slow path, and no basis for a catalog users can trust.
- **Wasm only**: safest and fastest, but a toolchain hurdle for a quick personal command.
- **Raycast API compatibility**: would require a React renderer and Node compatibility; rejected,
  but the data model stays close so manual ports are mechanical.

## Consequences

- `daimos-ext` has two hosts and one conformance suite; the suite is the M2 gate.
- The WIT interface is versioned from 0.1.0; breaking changes mean a new world, and the host keeps
  loading old worlds for at least one major version.
- Network access from wasm goes through the host's `wasi:http`, so the allow-list is enforced in
  one place and extensions cannot open raw sockets.
- Script extensions in the catalog need human review; wasm extensions need a capability review
  only.
