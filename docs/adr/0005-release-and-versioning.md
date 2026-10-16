# ADR 0005 — Release and versioning

## Status

Accepted

## Context

Daimos ships three platforms from one repository with a CI matrix that is expensive on Windows and
macOS. Versions have to be derived without a manual bump commit, pull requests need a cheap CI path
and releases need notes that do not have to be written by hand. The first two releases (v0.1.0,
v0.2.0) were pre-releases numbered `-beta.<n>`; from v0.3.0 the versions are stable.

## Decision

A release is the merge of a release branch into `main`. Everything after the merge is automatic.

- **Branches:** work for a release is squash-merged into `release/<x.y>` through one pull request
  per PBI, in dependency order. The release branch is finished with a `release: <x.y>` pull request
  into `main` that is merged with a merge commit, never squashed, so the squashed PBI commits stay
  visible on `main`.
- **Versions:** GitVersion in `ContinuousDeployment` mode numbers the merge. Stable versions are
  counted from the last stable tag: a merge is the next patch; `+semver: minor` or `+semver: major`
  in a commit message bumps that part. `next-version` in `GitVersion.yml` is used only to lift a
  pre-release to its first stable number and is removed afterwards. The manifests (`Cargo.toml`,
  `tauri.conf.json`, `package.json`) keep placeholder versions; the release workflow stamps the
  real one.
- **Notes and changelog:** git-cliff (`cliff.toml`) writes the release notes and `CHANGELOG.md`
  from the conventional-commit titles; `chore`, `ci`, `test`, `style` and `build` commits are left
  out unless marked breaking.
- **Pipeline:** `release.yml` calls the shared workflow `GravionLabs/ci/.github/workflows/tauri-app.yml`
  that builds the CLI archives (Linux x86_64 and arm64, macOS arm64, Windows x86_64) and the
  installers (`deb`, `nsis`, `dmg`), publishes the GitHub release `v<version>` and deletes the
  release branch.
- **CI cost:** pull requests into a release branch run the headless-sway Linux job, `cargo deny`
  and the readiness check only. The label `full-matrix` adds Windows and macOS, `x11` adds the X11
  job. Pull requests into `main` and pushes run everything.
- **Packages are unsigned** until the "Signed builds and packages" feature before v1.0; the
  documentation says so and tells users how to open the app anyway.

## Alternatives considered

- **Version bump commits (`cargo release`, `changesets`).** Rejected: a manual step per release on
  a one-person project is the step that gets skipped; the git history already carries the
  information.
- **Release on every merge to `main` (trunk-based).** Rejected: the budget and the manual platform
  checks need a place where a release is assembled and verified before it is numbered.
- **Tags as the release trigger.** Rejected: the tag is an output of the pipeline, not an input, so
  that the number always matches the history.

## Consequences

- `docs/releasing.md` is the operating manual of this decision; a change to the flow updates both.
- A PBI that lands in a release branch is closed by hand after the squash merge, because GitHub
  closes issues only on merges into the default branch.
- Patch releases happen whenever anything is merged into `main`, including pipeline chores
  (v0.3.1 and v0.3.2 contain no user-visible changes).
