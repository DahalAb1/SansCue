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

Do not infer that the acceptance procedure or every database gate has run:
record execution status for the current host/commit separately. Live Bee/API,
AWS, model quality, Docker/Compose runtime, and production resilience/performance
remain distinct unverified gates. See [verification status](docs/environment-verification.md)
for Docker checks and [development context](docs/development-context.md) for
current implementation boundaries.

## Documents

- [Foundation contract and stage acceptance](docs/foundation-contract.md)
- [Environment verification status](docs/environment-verification.md)
- [MVP build plan](docs/mvp-plan.md)
- [Submission checklist](docs/submission-requirements.md)
- [Developer product feedback](docs/product-feedback.md)
- [Bee streaming documentation](https://docs.bee.computer/docs/realtime)

Licensed under the [MIT License](LICENSE).
