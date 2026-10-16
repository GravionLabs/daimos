# ADR 0006 — Script extension security model

## Status

Accepted

## Context

ADR 0002 decided on two runtimes: wasm components that are sandboxed by declared capabilities, and
scripts in any language that run with the user's rights. The product spec promises "extensions are
sandboxed by default" and a "sandbox the user can see". Scripts shipped first (M2, v0.4) and cannot
be sandboxed portably, so the question was what the user sees and confirms before a script runs,
and where the secrets a script needs are kept.

## Decision

A script extension runs unsandboxed, and Daimos makes that visible and explicitly consented to.

- **Declared, displayed, not enforced.** The manifest declares capabilities (`network`,
  `filesystem`, `clipboard`, `keyring`) in the same format as a wasm extension. For a script they
  are a statement of intent that is shown to the user, not a boundary; a permanent "runs
  unsandboxed" label says so in the UI and in `daimos ext list`.
- **Consent per version, recorded outside the manifest.** On install the user sees the capabilities
  and confirms. The confirmation is written to `.consent.json` in the extension's directory, next
  to `extension.toml` but not inside it, so an update that replaces the manifest cannot grant
  itself anything. An extension whose manifest asks for more than its consent record is loaded in
  the state `needs-confirmation` and does not run until `daimos ext confirm` or the UI prompt
  accepts the new capabilities. Without a record everything is pending, even an empty declaration.
- **Secrets only in the OS keyring.** A `password` preference goes to the Secret Service, the
  Credential Manager or the Keychain under the service `daimos` and the user
  `<extension>/<preference>`, so the user can find and delete it by hand. Passwords are never
  written to SQLite, never logged, never part of `Debug` output and never listed; status reports
  only whether one is set. Other preference types are stored in `settings.db`.
- **Built-in actions stay in the host.** `open`, `copy`, `paste` and `run` are executed by Daimos,
  so a script never has to touch the clipboard or the shell to do the common things.
- **Process limits.** A long-running script is restarted after a crash with backoff and a limit,
  stopped after an idle timeout and cancelled per request, so a misbehaving script costs the user
  a toast, not the palette.

## Alternatives considered

- **Sandbox scripts with OS tools** (bubblewrap or Firejail on Linux, App Sandbox on macOS,
  AppContainer on Windows). Rejected for now: three different mechanisms with different holes, not
  available on every Linux distribution, and a sandboxed Python still needs the user's home for
  its interpreter. Wasm is the sandboxed runtime; scripts are the escape hatch.
- **Enforce the declared capabilities for scripts by proxying network and filesystem.** Rejected: a
  script can always bypass a proxy, so the enforcement would be a false promise.
- **No consent for scripts, since nothing is enforced.** Rejected: the user should know what a
  script says it does before it runs with their rights, and the diff of the declaration between
  versions is the one signal an update cannot hide.

## Consequences

- The capability prompt, the consent record and the keyring handling are shared with the wasm
  runtime; for wasm the same declaration becomes a boundary (ADR 0002, #56).
- A keyring that is not available (no Secret Service on a minimal Linux session) is an error with
  a hint, not a fallback to plain-text storage.
- The catalog (M4) can list script extensions only with a review, because the sandbox label is the
  user's main signal there.
