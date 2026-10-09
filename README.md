# SansCue

A planned conference web app that uses Bee's spoken context to generate short audience questions and show speakers live feedback.

Built for the Bee track of the [Build, Ship, Shape: Amazon Developer Hackathon](https://amazonappdev2026.devpost.com/).

**Status:** The `dev` branch now includes room/access and live updates, a
device-independent Bee replay/binding/outbox service, deterministic
transcript-to-candidate preview, and question publication, audience ratings,
written Q&A, and staff response counts. Candidate generation is the explicit
`stub-v1` placeholder, not a model integration. The new
[hardware-independent acceptance procedure](services/acceptance/README.md)
exercises this flow with a synthetic fixture and three separate PostgreSQL
databases.

The synthetic end-to-end procedure and the relevant service PostgreSQL suites
passed on `test/mvp-acceptance-matrix` at `707e23c` against native PostgreSQL
17.11; this is not Docker verification. See the
[verification record](docs/environment-verification.md) for exact check scope.
Live Bee/API, AWS, model quality, Docker/Compose runtime, and production
resilience/performance remain distinct unverified gates. See the
[development context](docs/development-context.md) for implementation
boundaries.

## Documents

- [Foundation contract and stage acceptance](docs/foundation-contract.md)
- [Environment verification status](docs/environment-verification.md)
- [MVP build plan](docs/mvp-plan.md)
- [Submission checklist](docs/submission-requirements.md)
- [Developer product feedback](docs/product-feedback.md)
- [Bee streaming documentation](https://docs.bee.computer/docs/realtime)

Licensed under the [MIT License](LICENSE).
