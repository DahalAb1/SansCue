# SansCue

A conference web app in development that uses Bee's spoken context to generate short audience questions and show speakers live feedback.

Built for the Bee track of the [Build, Ship, Shape: Amazon Developer Hackathon](https://amazonappdev2026.devpost.com/).

**Status (October 6, 2026):** Foundation code exists: a Preact frontend shell, Rust sessions health/readiness service, and local Compose packaging. Rooms, access, live feedback, Bee ingestion, and question generation are not implemented yet. Docker/runtime acceptance, AWS deployment, and the full MVP are not verified or complete.

Current implementation targets `integration/mvp-complete`; no `dev`/`main` merge or AWS deployment is authorized. The [MVP product contract](docs/mvp-contracts.md) records chosen defaults and acceptance expectations; the foundation contract's Stage 0 status text is historical.

## Local development

See [local runtime setup](deploy/README.md) for prerequisites, configuration, Compose startup, and direct-host development; its isolated-branch status notes describe original authoring, not this integrated tree. The existing stack runs only the foundation, not the conference flow.

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
