# Development Context

Updated October 8, 2026. Team development guidance. Read alongside the [MVP plan](mvp-plan.md), which holds the implementation steps.

## Current implementation status

`dev` contains the web room/access flow, PostgreSQL-backed sessions service,
durable room updates, and local Compose packaging. Rust checks and tests against
the provisioned native PostgreSQL 17 database, frontend checks, and runtime
configuration unit/static checks pass. Docker/Compose build, proxy, real outage,
named-volume persistence, and backup/restore checks remain environment-blocked;
AWS deployment and Bee/model integrations are not complete. See the
[environment verification record](environment-verification.md) for the exact
container-only checklist.

## Implementation agreement

The [foundation contract](foundation-contract.md) records the agreed Stage 0/Stage 1 boundaries, ownership, and integration details. Use it for the immediate local foundation scope; the broader architecture and AWS goals below remain the product direction, not completed implementation.

## Working preferences

- Build one agreed step at a time. Discussion and requests for explanations are not permission to implement the whole product.
- Follow explicit instructions to wait for a green light. Once work is authorized, complete that scope without repeatedly asking for confirmation.
- During authorized implementation, agents make small, purposeful commits on their assigned task branches. The user handles pushes and integration unless explicitly delegated. An explicit request to leave work uncommitted takes precedence.
- Agent handoff documents explain the product, current state, boundaries, and unknowns. Keep development and commit rules here; the user assigns each agent's implementation scope separately.
- Read files before editing; the user also edits them. Preserve their changes and preferred structure.
- Use plain language, short tasks, and clear diagrams. Avoid roleplay scenarios, repeated explanations, unnecessary dependencies, and speculative scaffolding.
- Label proposals and unknowns. Never invent measurements, test results, device access, or completed features.

## Git workflow

| Branch | Purpose |
|---|---|
| `dev` | Integrate features and test them together. |
| `main` | Hold the source for tested, runnable milestones. |
| `feat/...`, `fix/...`, `docs/...` | One task per branch, created from current `dev`, with one owner. |

Feature branches are retained locally and on origin after integration. Keep automatic branch deletion disabled. Delete a branch only when the user explicitly requests it.

### Create `dev` once

If `dev` does not already exist, start it from updated `main`:

```bash
git switch main
git pull --ff-only origin main
git switch -c dev
git push -u origin dev
```

### Start a task

Start with a clean working tree. Agree on the task and owner; use a short descriptive branch name. The example below is for room creation:

```bash
git switch dev
git pull --ff-only origin dev
git switch -c feat/room-creation
```

Implement the task and make small, focused commits. Stage only relevant files. Publish the branch with:

```bash
git push -u origin feat/room-creation
```

### Incremental commits

- Commit each independently meaningful component, function, behavior, or fix before moving to the next change. Do not save an entire feature's work for one large final commit.
- A reusable component or type can be introduced in one commit, then connected to the application in another. Each commit must have a clear purpose and keep the project buildable; avoid empty objects and broken intermediate states solely to create more commits.
- Include the checks and tests relevant to that change with its implementation. Run the relevant checks before committing and report anything that could not be verified.
- Stage only the agent's own task changes. Keep unrelated edits and private context out of commits.
- Use short messages that name the actual change: `feat: add reusable tab navigation`, `feat: connect audience tabs to routes`, or `fix: preserve question draft when switching tabs`. Avoid vague messages such as `updates` or `finish frontend`.
- These detailed commits stay on the retained feature branch. Squash integration still gives `dev` one commit for the completed task.

### Update the feature branch

Before integration, rebase the active feature branch onto the latest `dev`:

```bash
git switch feat/room-creation
git fetch origin
git rebase origin/dev
```

This updates the feature branch; it does not add the feature to `dev`.

If conflicts occur, resolve them, stage the resolved files with `git add`, and run `git rebase --continue`. Use `git rebase --abort` to return to the pre-rebase state. Run relevant checks again after resolving conflicts.

If the branch was already pushed and rebasing rewrote its commits, update its remote copy with:

```bash
git push --force-with-lease origin feat/room-creation
```

Use this only for your own feature branch. Never force-push or rebase shared `dev` or `main`. If the lease rejects the push, inspect the remote changes before proceeding.

### Squash into `dev`

1. Open a PR with the feature branch as the source and `dev` as the target.
2. Have the other teammate review it and pass the relevant checks against current `dev`.
3. Use **Squash and merge**. Give the resulting commit a description of the completed change, such as `feat: add room creation`.
4. Keep the feature branch and its detailed commit history.

`dev` receives one new commit containing the combined feature changes. The feature branch keeps its individual commits; they are not added as individual commits or a merged history to `dev`.

For parallel work, integrate feature A, then rebase active feature B onto updated `dev`, retest, and squash B. Already integrated feature branches remain as history; start subsequent tasks from updated `dev` instead of routinely rebasing those retained branches.

### Promote a tested milestone to `main`

Test the combined work on `dev` against the MVP step's completion checks. Record the exact tested commit with `git rev-parse HEAD` while on that `dev` revision.

Replace `TESTED_DEV_COMMIT` below with that commit hash:

```bash
git fetch origin
git switch main
git pull --ff-only origin main
git merge --ff-only TESTED_DEV_COMMIT
git push origin main
```

This advances `main` to the tested commit without creating a merge commit or rewriting history. If fast-forwarding fails, inspect the divergence before changing either shared branch. Keep `dev` and all feature branches afterward.

## Team and scope

- Two-person team building for the Amazon hackathon's Bee track. Microservices are required by the team and professor.
- Working name: SansCue; it may change. Keep technical names and commit messages independent of branding.
- Conference web app first. Audience members use phone browsers; no native mobile application.
- AWS hosting only for the current scope. Local-network hosting has been removed.
- Team submission target: October 22, 2026. Deadline: October 23, 2026, 2 PM America/Chicago. Keep submission details in the [checklist](../docs/submission-requirements.md).
- A small external pilot is desired, subject to scheduling. No pilot or performance results exist yet.

## Product behavior

The speaker creates a room and displays a QR code. Audience members join its webpage. Bee supplies spoken context; generated questions gather audience feedback; the speaker sees topic-level feedback and context-linked questions.

- Audience: generated questions and response controls, with written questions restricted to a separate Q&A tab.
- Speaker: a dashboard understandable while speaking, plus question management and controls. This is a dashboard, not a speaking/recording tab.
- TA: access to Questions; further permissions remain undecided. The speaker must also be able to operate alone.
- Topic precision and question frequency are separate controls.
- Published questions stay fixed while people answer. Preserve their supporting transcript context after the discussion moves on.
- Display response counts alongside understanding ratings where applicable.
- Session-only context includes earlier discussion within that session. Participant history across future sessions is deferred.

## Architecture direction

Current stack recommendation: **Preact + TypeScript + Vite + CSS**, **Rust + Axum + Tokio**, **PostgreSQL + SQLx**, **Docker Compose on EC2**, and **Caddy HTTPS**.

One repository contains the frontend and three independently runnable backend services:

| Service | Owns |
|---|---|
| Sessions-and-feedback | Rooms, access, published question copies, responses, analytics, live updates |
| Bee-connection | Original transcript events and room associations |
| Topics-and-questions | Context, generation jobs, generated questions, evidence references |

Each service owns its database, credentials, migrations, and contracts. One PostgreSQL server can host the three databases initially; capacity remains untested. Services communicate through authenticated APIs or messages and do not query each other's databases or import each other's business logic. The frontend is not a backend microservice.

Proposed layout; create files only when their implementation is needed:

```text
web/src/
  app/                         routing and page composition
  features/                    rooms, responses, questions, dashboard
  ui/                          reusable visual controls
  lib/                         HTTP and live-connection clients
services/<service>/
  src/
    main.rs                    process startup
    app.rs                     dependency wiring
    config.rs                  validated settings
    <feature>/                 feature-owned code
  contracts/                   API and event definitions
  migrations/                  database changes
  tests/                       service integration tests
  Cargo.toml
  Dockerfile
deploy/                        Compose and Caddy configuration
docs/                          public plan and submission documents
Cargo.toml                     Rust workspace
Cargo.lock
```

Within a feature, separate HTTP handling, business rules, types/errors, and SQL when needed. Use small Rust traits at storage, Bee, model, and messaging boundaries. Avoid creating a shared business-model crate that couples the services.

## Code and scaling rules

- Write simple, clean, concise code with clear names. Prefer straightforward control flow over clever shortcuts, verbose boilerplate, or speculative abstractions.
- Apply SOLID to responsibilities and dependencies. Use reusable components, functions, modules, or types when they remove real duplication or define a needed boundary. SOLID does not require a class for every feature; choose the idiomatic construct for TypeScript or Rust.
- Keep each component and function focused. Separate reusable behavior from its application wiring when useful, and commit those changes incrementally as described above.

| SOLID principle | Practical rule |
|---|---|
| Single responsibility | Keep request parsing, product rules, and database code separate. |
| Open/closed | Extend external integrations through stable interfaces. |
| Liskov substitution | Implementations preserve the interface's behavior, errors, and side effects. |
| Interface segregation | Define small interfaces for specific operations. |
| Dependency inversion | Product logic depends on interfaces; startup code supplies implementations. |

- Keep generation jobs separate from response handling so a slow model cannot freeze room feedback.
- Store important state durably. Keep socket connections in bounded memory; reload current room state on reconnect.
- Use request/event IDs and database constraints to prevent duplicate effects. Handle ordering explicitly; arrival order alone does not prove speech order.
- For reliable service delivery, save outgoing events with their database changes, then retry delivery. Consumers must handle duplicates.
- Limit model concurrency, outgoing queues, and database pools; use timeouts and controlled retries.
- Before adding sessions-service replicas, provide room broadcasts across instances and reconnect recovery. Job distribution and broadcasting need different delivery behavior.
- Evolve contracts compatibly. Test product rules, database behavior, integration contracts, and the full flow as those parts are built.
- Start with the required services and measure demand before adding infrastructure. No user-capacity or latency claims are established.

## Development sequence and open decisions

- The local implementation covers the frontend shell, sessions service, room creation/join/access, and live updates. Local Compose packaging exists but awaits Docker verification; AWS account/domain readiness and deployment are unverified.
- Next product milestone: complete Step 6 reconnect and room-ending behavior, then agree on the listed Bee integration decisions before Step 7. Continue native PostgreSQL tests for every relevant service change.
- Build general infrastructure, connect the services, then optimize the complete flow. Add timing measurements during integration; fix issues that block useful testing immediately.
- Access to a Bee-enabled Apple Watch must be scheduled. Authentication, data access, live delivery, and performance have not been verified for this project.
- Recorded transcripts can support repeatable development tests. Validate the final flow with actual Bee data.
- Step 8: decide an initial question format, topic behavior, and context selection; connect a provisional model.
- Step 9: compare models using the same transcripts and instructions; choose based on quality, complete-question delay, and cost. Record model and prompt versions. Bedrock is a candidate, not a finalized choice.
- Step 10: decide response options, publishing rules, timers, Q&A visibility/moderation, dashboard attention, and TA permissions.
- Service message transport, exact AWS capacity, and performance targets remain open. SQS is a candidate, not an installed requirement.

Update these notes when decisions change. Never add credentials, participant data, private recordings, or machine-specific paths.
