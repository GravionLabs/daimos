# Daimos — agent guide

Daimos is a cross-platform command palette (Linux on X11 and Wayland, Windows, macOS) built with
Tauri 2, a Rust core, and a SolidJS frontend. Read `docs/specs/product-spec.md` first, then
`docs/specs/architecture.md` and `docs/specs/extensions.md`. Decisions live in `docs/adr/`.

## Issue tracking

Work is tracked on GitHub as Epic → Feature → PBI → Task (Bug → Task), using GitHub's **native
sub-issues** (not manual checklists) for parent/child links — see `.github/ISSUE_TEMPLATE/`. When
creating child issues, link them via the sub-issues panel/API, not a markdown checklist. The build
order is the milestone table in `docs/specs/product-spec.md`.

### Issue conventions

These apply to every issue, including issues created with `gh` or the API (which skip the issue
forms). Write them in US English.

- **Labels:** exactly one level label (`epic`, `feature`, `pbi`, `task`, `bug`) and one `area:*`
  label (`area:core`, `area:platform`, `area:ext`, `area:frontend`, `area:cli`, `area:ci`).
- **Title:** `[Epic] `, `[Feature] `, `[PBI] `, `[Task] ` or `[Bug] ` matching the level label.
  PBIs and tasks continue with a conventional-commit title: `[PBI] feat: hotkey on Wayland via the
  XDG GlobalShortcuts portal`, `[Task] test: matcher orders exact prefix first`.
- **PBI body:** `### Parent Feature`, `### Acceptance criteria`, `### Depends on`,
  `### Verification` (commands run from the repo root), `### Sub-issues`. Add a
  `### How to work on this` section when the order of tasks or the documents to read are not
  obvious.
- **Task body:** `### Parent PBI or Bug`, `### Implementation notes`, `### Files`, `### Done when`
  (checkboxes ending with the PBI's verification). A task that needs a check on a real desktop adds
  `### Manual check` with the platform, how to start the app, and the exact steps.
- **Platforms:** a PBI that touches `daimos-platform` says in its acceptance criteria what happens
  on each of X11, Wayland (KDE and GNOME), Windows, and macOS, including where it degrades.
- **Links:** parents and children as native sub-issues; dependencies between PBIs as native
  "blocked by" dependencies, and also listed under `### Depends on`. Refer to other issues by
  number.
- **ADRs:** never fix an ADR number in an issue; write "the next free ADR number".
- **Decisions:** an issue whose outcome is a product or architecture decision gets
  `needs-decision`. Prepare it fully and write the ADR with **Status: Proposed**; the owner accepts
  it and removes the label.
- **Milestones:** features, PBIs and tasks get their milestone (`M0 Foundation` … `v1.0 Release`),
  epics none.

**Definition of ready:** a PBI or task can be started when it has no `needs-refinement` or
`needs-decision` label and nothing it is blocked by is still open.

## Verification

From the repo root, unless a PBI says otherwise:

```sh
cargo fmt --all --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
pnpm lint && pnpm test
```

## Performance budget

The budget in `docs/specs/product-spec.md` is a CI check from M1. A change that breaks it does not
merge; raise the budget only with an ADR.
