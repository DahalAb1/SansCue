# Building SansCue

SansCue began with a question: how can one person communicate with a large
audience while still responding to different people's needs? We wanted to use
live conversational context to connect what a speaker says with what listeners
find clear, confusing, or worth asking about.

For the first release, we connected Bee context, AI-generated check-ins,
audience feedback, and private Q&A in one room. Speakers receive signals they
can act on, audience members can participate without taking the microphone,
and moderators can provide individual support. This is our first step toward
making communication more personal at scale.

We separated Bee ingestion, question generation, and room interactions into
three services. These boundaries let us extend each part independently and keep
model processing separate from audience participation as the product develops.

## An agent team on AWS

Alongside the application, we built an environment for AI coding agents on
Amazon EC2—a cloud computer where development could continue between our local
sessions. OpenClaw coordinated tasks. Cursor supplied coding sessions connected
through the Agent Client Protocol (ACP), an interface for controlling those
sessions from another tool.

The intended workflow was:

```text
Inspect the code → plan a bounded task → implement → test → review → integrate
```

A coordinator assigned work; a planner defined the task and its checks;
implementation agents changed code; reviewers examined the result. Supporting
roles handled research, environment problems, and preserving context. Separate
Git worktrees gave parallel tasks their own working directories. These were
assigned responsibilities, not proof that every role ran for every change.

The agents used an operating-system account without administrator privileges.
The database and coordination service stayed private. We initially planned to
reuse the development machine for deployment; the recorded product deployment
later used a separate EC2 instance. The coding agents are development tools.
The application itself uses Bedrock to draft questions.

## What made this difficult

**A role name did not identify a working coding session.** Asking for a
“builder” described a job, but the coordinator also needed the correct route
to Cursor. Directly starting Cursor worked while delegation failed. A wrapper
entry connected the two, and a small repository-reading task verified the
complete route before we entrusted it with implementation.

**Background work could wait on an invisible permission prompt.** Workers
running without an interactive terminal could not answer prompts. We configured
their tool permissions for that environment while retaining the unprivileged
operating-system account.

**Different failures looked like an agent that had stopped working.** Full
conversation context, exhausted provider quotas, missing tool integration, and
an incorrectly targeted session needed different fixes. Repeated restarts did
not distinguish them. Inspecting the failing session and testing one boundary
at a time was more useful.

**Tests needed the worker's actual environment.** Docker was unavailable to the
agent account at an early stage. Native PostgreSQL 17 let database integration
checks proceed. Credentials and environment variables had to be available in
the worker's own shell; success in an administrator's shell was insufficient.

**A simulated input could not settle a device question.** Transcript replay
kept integration work moving when hardware was unavailable. Sanskriti's Apple
Watch runs then checked real delivery. Hosted Nova inference was tested
separately. The [verification record](environment-verification.md) preserves
those distinctions.

## Team contributions

We supplied product direction, integration decisions, testing, and deployment
work alongside agent-assisted implementation. These Git records illustrate
contributions; commit counts do not measure who wrote each line unaided.

| Contributor | Examples |
| --- | --- |
| Abhinesh Dahal | [EC2 deployment and Bee bridge](https://github.com/DahalAb1/SansCue/commit/9c330e6), [room access and request limits](https://github.com/DahalAb1/SansCue/commit/94b7422), [Nova integration](https://github.com/DahalAb1/SansCue/commit/7f41b0c) |
| Sanskriti Poudel | [Live Bee ingestion](https://github.com/DahalAb1/SansCue/commit/c0c1fe1), [Bedrock integration](https://github.com/DahalAb1/SansCue/commit/6116ae1), [staff transcript view](https://github.com/DahalAb1/SansCue/commit/5af0f1d), [Apple Watch testing](https://github.com/DahalAb1/SansCue/pull/2) |
| OpenClaw Builder | [Transcript delivery](https://github.com/DahalAb1/SansCue/commit/20a916b), [question pipeline](https://github.com/DahalAb1/SansCue/commit/31fb60b), [generation job reliability](https://github.com/DahalAb1/SansCue/commit/411fba0) |
| Cursor Agent, with team direction | [Moderator publication and private replies](https://github.com/DahalAb1/SansCue/commit/5e50ef4), [recording selection](https://github.com/DahalAb1/SansCue/commit/d2addee), [topic feedback chart](https://github.com/DahalAb1/SansCue/commit/f514ae1) |

The main lesson was that useful agent work needs more than a prompt. It needs a
clear task, a working environment, access to the right context, and checks that
show what actually happened. We had to engineer that process as deliberately
as the product.
