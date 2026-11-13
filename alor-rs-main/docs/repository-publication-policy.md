# Repository publication policy

## Canonical branch

`origin/main` is the canonical engineering and deployment branch. Changes intended
for publication are prepared directly on top of `origin/main`; a second sanitized
repository or a long-lived export branch is not required.

The full local development and live-soak history may be retained in local-only
archive branches. Those branches must not be pushed to a shared remote.

## Content allowed in `origin/main`

- application source code and tests;
- deployable configuration templates without credentials;
- architecture documents, technical specifications, runbooks, and checklists;
- deterministic test fixtures and bootstrap artifacts required by the runtime;
- redacted incident reports that are needed to explain an engineering decision.

## Local-only content

- raw broker ledgers and account exports;
- detailed live-soak observation journals and economics;
- VPS inventories, addresses, credentials, and operator shell transcripts;
- runtime logs, replay output, review bundles, and generated reports;
- local `.env` files, keys, tokens, and `expect` automation.

Store local-only material under an ignored directory such as `private/`,
`private_docs/`, `runtime_artifacts/`, `review_artifacts/`, or `reports/`.

Files already tracked in `origin/main` are not hidden by `.gitignore`. Updating
one of them still requires an explicit content and secret review.

Distribution ZIP/tar archives and duplicate source snapshots are transferred
separately, not added to the engineering tree. Keep only the portable model/
adapter contract and reproducible verification instructions in published docs.

Before reconciling an older mixed local tree, retain a local-only safety ref
and verify both tracked and untracked changes. An engineering-only tree import
on top of `origin/main` must not merge archive branch ancestry, which can
contain historical operational notes. Never force-push those local histories.

## Pre-push checks

1. Start from the latest `origin/main` and require a fast-forward push.
2. Review `git diff --name-status origin/main...HEAD`.
3. Confirm that no local environment file, broker export, runtime log, VPS
   inventory, token, password, or private key is present.
4. Run formatting, tests, and lint checks for affected Rust crates.
5. Push only the reviewed integration branch to `origin/main`.
