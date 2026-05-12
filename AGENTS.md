# DW Trial Branch Policy

This repository keeps the `dw` trial work in two origin-tracked branches:
`kcptun-support` and `dw-trial`.

## `kcptun-support`

- `kcptun-support` always tracks the feature work intended for an upstream PR.
- Keep the branch as a single squashed commit ahead of the relevant upstream
  base.
- When the PR-facing feature changes, update this branch by replacing that
  single commit instead of stacking follow-up commits.
- Keep `origin/kcptun-support` synchronized with the local branch.

## `dw-trial`

- `dw-trial` always starts from the latest stable upstream release tag, not from
  `upstream/dev`.
- Avoid release candidates for the base unless explicitly requested; prefer the
  latest stable tag such as `vX.Y.Z`.
- Keep `dw-trial` as exactly two commits ahead of the upstream release base:
  1. the squashed `kcptun-support` change;
  2. one temporary trial commit for DW-only build and trial customizations.
- Put any `dw-trial`-only maintenance notes, including this file, in the second
  temporary trial commit so the branch remains two commits ahead.
- Keep `origin/dw-trial` synchronized with the local branch. Because the branch
  is intentionally rebased onto newer stable releases, update the remote with
  `git push --force-with-lease origin dw-trial`.

Before updating either branch, confirm the target base and the local/remote
relationship with `git status --short --branch`, `git log --oneline --decorate`,
and the relevant `origin/*` and `upstream/*` refs.
