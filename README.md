# SansCue

A conference web app in development that uses Bee's spoken context to generate short audience questions and show speakers live feedback.

Built for the Bee track of the [Build, Ship, Shape: Amazon Developer Hackathon](https://amazonappdev2026.devpost.com/).

**Status (October 8, 2026):** The local `dev` branch contains the Preact room-navigation UI and PostgreSQL-backed sessions room/access API, alongside the sessions health service and local Compose packaging. Frontend typechecking, contract tests, and build pass. Live updates, Bee ingestion, evidence-linked question generation, responses, and the speaker dashboard are not implemented yet. Database-backed service verification, Compose/runtime acceptance, AWS deployment, and the full MVP are not verified or complete.

Current development targets `dev` via small feature branches rebased onto its latest commit and squash-integrated after review. Pushes, changes to `main`, AWS deployment, and paid model calls are not authorized. The [MVP product contract](docs/mvp-contracts.md) records chosen defaults and acceptance expectations; the foundation contract's Stage 0 status text is historical.

## Local development

See [local runtime setup](deploy/README.md) for prerequisites, configuration, Compose startup, and direct-host development; its isolated-branch status notes describe original authoring, not this integrated tree. Room and access flows are implemented in the code, but end-to-end runtime behavior has not yet been verified.

Frontend checks: `(cd web && npm ci && npm run typecheck && npm run build)`. See [sessions verification](services/sessions/tests/README.md) for Rust checks and the required disposable PostgreSQL test database. [MVP verification expectations](docs/mvp-contracts.md#local-verification-and-completion-evidence) distinguish local checks from actual Bee/model and deployment evidence. These instructions are not a claim that checks passed.

## Documents

- [MVP product contract](docs/mvp-contracts.md)
- [Development workflow](docs/development-context.md)
- [First-iteration context](docs/agent-iteration-1.md)
- [Foundation contract and stage acceptance](docs/foundation-contract.md)
- [MVP build plan](docs/mvp-plan.md)
- [Submission checklist](docs/submission-requirements.md)
- [Developer product feedback](docs/product-feedback.md)
- [Bee streaming documentation](https://docs.bee.computer/docs/realtime)

Licensed under the [MIT License](LICENSE).
