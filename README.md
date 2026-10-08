# SansCue

A planned conference web app that uses Bee's spoken context to generate short audience questions and show speakers live feedback.

Built for the Bee track of the [Build, Ship, Shape: Amazon Developer Hackathon](https://amazonappdev2026.devpost.com/).

**Status:** MVP implementation is underway on `dev`. The repository now contains
the web app shell and room/access flow, a PostgreSQL-backed Rust sessions service,
live room updates, and a documented local Compose runtime.

Native PostgreSQL and frontend checks pass. Docker/Compose image, routing,
outage/recovery, persistence, and backup/restore verification remain pending;
AWS deployment, Bee integration, and generated-question workflows are not yet
verified or complete. See the [verification status](docs/environment-verification.md)
for exact remaining environment checks.

## Documents

- [Foundation contract and stage acceptance](docs/foundation-contract.md)
- [Environment verification status](docs/environment-verification.md)
- [MVP build plan](docs/mvp-plan.md)
- [Submission checklist](docs/submission-requirements.md)
- [Developer product feedback](docs/product-feedback.md)
- [Bee streaming documentation](https://docs.bee.computer/docs/realtime)

Licensed under the [MIT License](LICENSE).
