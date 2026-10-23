# Developer tool feedback

SansCue uses live conversational context to connect speakers, audiences, and
moderators. This feedback describes the tools we used to build that experience,
what worked in our setup, and where we encountered friction. Test results refer
to the recorded runs, rather than every future deployment.

## Bee on Apple Watch and the Bee CLI

**Use:** provide the live conversational context for audience interaction.
The selected recording supplies the transcript from which SansCue prepares
relevant check-ins. Audience members respond and ask private questions through
their browsers without connecting a Bee account.

**What worked:** Sanskriti's local tests delivered Apple Watch speech to the
room and displayed it for the presenter and moderator. See [PR #2](https://github.com/DahalAb1/SansCue/pull/2)
and [PR #3](https://github.com/DahalAb1/SansCue/pull/3).

**Onboarding and difficulty:** we had to map recording identifiers, timestamps,
and emitted events into our own room model. A running listener did not by
itself prove the intended room was receiving speech. Hardware availability
also limited when we could test the real connection; synthetic replay helped
with development but could not verify device behavior.

**Suggested improvement:** a versioned example showing recording selection,
transcript events, disconnects, and available recovery behavior would help
integrators validate the complete path.

**Would we use it again? Yes.** Continuous spoken context is central to the
speaker-and-audience experience we want to build. We would keep explicit
recording selection and verify device recovery as part of each deployment.

## Amazon Bedrock

**Use:** prepare check-ins that follow the discussion without requiring someone
to write every poll in advance. Nova Micro turns recent transcript excerpts
into short clarity questions with supporting quotes. The configured model is
`amazon.nova-micro-v1:0` in `us-east-1`.

**What worked:** actual Nova requests produced questions on three synthetic
explanations. A hosted test then completed generation, moderator publication,
an audience response, and presenter counts. The model runs through an EC2
instance role without copying personal AWS keys to the server.

**Onboarding and difficulty:** we needed timely responses while managing
limited credits and uncertain request volume. An earlier Bedrock Claude test
reached AWS but hit the account's daily token limit. Nova provided a working
MVP path; we have not measured it against other models for quality or speed.

**Suggested improvement:** a preflight view combining model access, remaining
quota, and applicable credits would make account readiness easier to establish.
Authentication alone did not establish usable inference capacity.

**Would we use it again? Yes.** Bedrock gave us a working AI path for the MVP.
We would check model access and quota early, and continue measuring response
delay and cost against a live discussion.

## AWS hosting and access

**Use:** EC2 hosts the application and was also used for the separate coding-agent
environment. The product uses encrypted EBS storage, an Elastic IP for its HTTPS
address, and an IAM instance role limited to Nova invocation. EventBridge
Scheduler controls planned start/stop times. The Bee worker stays on a trusted
Mac and reaches private services through SSH tunnels.

**What worked:** public HTTPS, automatic recovery after an EC2 stop/start, and
same-host database dump/restore passed their recorded checks. Instance-role
credentials could be refreshed without restarting the model service.

**Onboarding and difficulty:** deployment required coordinating private ports,
database roles, file ownership, credential refresh, and the Mac connection.
One credential-file fix concerned the container user's actual UID rather than
an assumed group ID. Scheduled execution and off-instance recovery still need
their own observations; configured schedules are not proof of execution.

## Coding agents: OpenClaw, Cursor, and OpenAI/Codex tools

**Use:** task coordination, planning, implementation, review, and documentation.
OpenClaw coordinated specialist roles; Cursor supplied coding sessions through
ACP, the Agent Client Protocol. OpenAI/Codex tools also supported planning and
development. The [development story](development-story.md) explains the setup
and links representative human and agent contributions.

**What worked:** a small delegation test eventually reached the coding session
and read the repository. Scoped implementation and review work then contributed
to the product alongside the team's changes.

**Onboarding and difficulty:** logical role names and coding-runtime identifiers
were different. Background workers encountered inaccessible permission prompts;
missing integrations, context overflow, and provider quotas caused separate
failures. Native PostgreSQL allowed database checks while Docker was unavailable
to the worker account.

**Suggested improvement:** show the requested role, selected coding tool,
active session, permissions, and failing step together when delegation fails.
That would reduce trial-and-error configuration changes.

**Would we use this approach again? Yes.** Scoped coding and review work was
useful, but setting up the agent system was substantial work. We would start
with a verified delegation path and clear task boundaries before expanding
into more roles.

## Application and development tools

| Tool | Use and observed experience |
| --- | --- |
| Docker and Compose | Package services and databases. Hosted builds passed; early agent development needed native databases while Docker was unavailable. |
| Caddy | Serve the frontend and proxy HTTPS requests. Public routing and certificate checks passed on the recorded deployment. |
| PostgreSQL 17 and SQLx | Store each service's data, apply migrations, and support durable delivery. Integration tests and same-host restore checks passed in their recorded scope. |
| Rust, Axum, and Tokio | Implement the services and asynchronous workers. Workspace tests exercised permissions, event delivery, and generation behavior. |
| Preact, TypeScript, Vite, and qrcode | Build the browser interface and audience QR links. Frontend tests and production builds passed in the verification record. |
| Git and GitHub | Separate feature work, review changes, and preserve human and agent contributions. Worktrees gave tasks independent working directories. |
| tmux and Telegram | Keep remote development sessions running and monitor progress. A custom watchdog could report false alarms; it did not automatically repair failures. |

Package versions are recorded in [Cargo.lock](../Cargo.lock) and
[web/package-lock.json](../web/package-lock.json). These records support the
observations above; they do not establish reliability or performance for every
future deployment. Detailed results are in the
[verification record](environment-verification.md).
