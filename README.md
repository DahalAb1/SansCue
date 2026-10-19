# SansCue

A planned conference web app that uses Bee's spoken context to generate short audience questions and show speakers live feedback.

Built for the Bee track of the [Build, Ship, Shape: Amazon Developer Hackathon](https://amazonappdev2026.devpost.com/).

**Status:** Rooms, access, live updates, Bee CLI ingestion, staff transcripts,
moderator question publication, audience feedback, private Q&A and speaker
charts are implemented. Topics supports optional Claude generation through
Anthropic or Amazon Bedrock, with an explicit placeholder when unconfigured.

Sanskriti's [PR #2](https://github.com/DahalAb1/SansCue/pull/2) records local
Compose/Apple Watch delivery, question drafts and passing synthetic acceptance.
[PR #3](https://github.com/DahalAb1/SansCue/pull/3) records staff transcript
visibility and Bedrock connectivity; the account hit its daily token limit
before a live AI draft appeared. These reports do not establish model quality,
full recovery, public HTTPS, capacity or backup/restore acceptance. See the
[verification record](docs/environment-verification.md) and
[MVP status](docs/mvp-plan.md). EC2 deployment preparation is documented in the
[production runbook](deploy/PRODUCTION.md).

## Documents

- [Foundation contract and stage acceptance](docs/foundation-contract.md)
- [Environment verification status](docs/environment-verification.md)
- [MVP build plan](docs/mvp-plan.md)
- [Question generation design decision](docs/question-generation-design.md)
- [Bee device verification runbook](docs/bee-device-verification.md)
- [Model comparison worksheet](docs/model-evaluation-worksheet.md)
- [Submission checklist](docs/submission-requirements.md)
- [Developer product feedback](docs/product-feedback.md)
- [Bee streaming documentation](https://docs.bee.computer/docs/realtime)

Licensed under the [MIT License](LICENSE).
