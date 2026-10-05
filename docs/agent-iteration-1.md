# Project context for the first iteration

## What we are building

SansCue is the working name of a Bee-powered web app that turns spoken context and audience input into useful feedback for a speaker. A two-person team is building the conference MVP for the Amazon hackathon's Bee track. Audience members use their phone browsers.

In a large talk, few listeners can speak up. Speakers need to understand where people are struggling, and audience questions need to retain the context that prompted them.

## Intended experience

1. The speaker creates a room and displays its QR code.
2. Audience members open the room's webpage.
3. Bee supplies spoken context; a separate service generates short, relevant questions.
4. Audience members respond and can submit written questions in a separate Q&A tab.
5. The speaker's dashboard shows feedback, response counts, and context-linked questions. A TA can help manage the room.

Published questions stay fixed while people answer. Topic precision and question frequency are separate controls. Earlier discussion remains available within the current session; participant history across sessions is outside this MVP.

## Architecture direction

Microservices are required. One repository will contain the web frontend and three independently runnable backend services.

| Component | Responsibility |
|---|---|
| Web frontend | Audience tabs, speaker dashboard, and TA controls |
| Sessions-and-feedback | Rooms, access, published questions, responses, analytics, and live updates |
| Bee-connection | Original transcript events and their room association |
| Topics-and-questions | Session context, generation jobs, and questions linked to transcript evidence |

Each backend service owns its database and communicates through APIs or messages. Question generation runs separately from response handling so model delays do not block audience feedback.

The current stack recommendation is Preact, TypeScript, Vite, and CSS for the frontend; Rust, Axum, Tokio, PostgreSQL, and SQLx for the backend; Docker Compose and Caddy on EC2 for hosting. Exact versions and service message transport are not settled.

## Where we are now

As of October 5, 2026, the repository contains planning and submission documents. Application implementation has not started.

The first iteration concerns the frontend shell, sessions-service startup, database connectivity, health checks, and repeatable packaging. Its purpose is to establish a foundation that the team can inspect and extend. Rooms, access controls, live updates, Bee, and question generation follow incrementally in the [MVP plan](mvp-plan.md).

Agents can develop and verify this foundation locally. Local execution is a development environment; the product's hosting target remains AWS. The team will handle AWS account access and deployment settings. Cloud deployment is not verified, and MVP Step 1 remains incomplete until the deployed system works.

## What is still undecided

- Final UI design, question format, response options, topic boundaries, timers, and dashboard attention behavior.
- Model/provider selection, which follows testing with actual Bee excerpts.
- Bee authentication, live event behavior, ordering, and recovery, which have not been verified for this project.
- AWS capacity, expected audience size, latency, and cost targets; no performance claims are established.

## How to use this context

The user assigns each agent a specific task and branch. This document supplies project background; it does not authorize building every part described here.

Development style, incremental commits, and integration rules live in [development context](development-context.md). The [MVP plan](mvp-plan.md) remains the implementation roadmap, and the [submission checklist](submission-requirements.md) holds delivery requirements.
