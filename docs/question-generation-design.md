# How check-ins are generated

Generated check-ins are one way SansCue makes participation easier. Someone
who would not raise a hand or write a question can still indicate whether the
current topic is clear. Together, these responses give the speaker information
to consider while the discussion is happening.

Bee's transcript supplies the conversational context. AI prepares a short
clarity question with supporting evidence; a moderator decides what the
audience sees. Private written questions provide a separate path for needs
that a quick response cannot express.

## From transcript to draft

1. The presenter selects the room's Bee recording. The Bee worker sends its
   accepted transcript events to the Topics service.
2. Topics assembles a recent window in speech order. It checks context length,
   freshness, pauses, repetition, and the interval since earlier drafts before
   requesting generation.
3. The model returns a question and supporting quote, or declines to generate.
   The hosted configuration uses Amazon Bedrock Nova Micro.
4. Topics validates the output and matches the quote to one contributing
   utterance. It supplies the recording and event identifiers itself.
5. The draft reaches Sessions, where only room staff can inspect it. A moderator
   can name its topic, edit, dismiss, or publish it.

The source quote lets the reviewer inspect what prompted the question. A
matching quote does not prove the question is relevant or correctly interprets
the explanation. The presenter has read-only access to drafts; publication is
restricted to the moderator role, called `ta` in the code.

## Generation and review defaults

| Control | Default |
| --- | --- |
| Minimum context | 60 words |
| Maximum window duration | 180 seconds |
| Silence boundary | 5 seconds |
| Maximum transcript age | 45 seconds |
| Minimum interval between drafts | 90 seconds |
| Minimum interval between attempts | 15 seconds of speech |
| Recent draft context | Last 5 drafts within 30 minutes |
| Generated question | One line, at most 25 words |
| Nova output limit | 512 tokens |
| Visible review queue | At most 3 drafts |
| Draft expiry | 10 minutes after arrival or latest saved edit |

These are starting settings for rehearsal. They do not promise a question at
fixed intervals. See the [Topics runtime guide](../services/topics-and-questions/README.md)
for configuration and the [local runtime guide](../deploy/README.md) for the
settings passed through Compose. Without model credentials, a labeled,
deterministic placeholder exercises the pipeline without AI inference.

## Publication and audience responses

Only one check-in is active at a time. Publishing another preserves the earlier
question's history. With a topic and no custom wording, publication asks
“How clear is {topic}?” Corrections create a new wording version; saved answers
retain their recorded version. Dismissed or expired drafts cannot be published.

Audience members answer Clear, Partly clear, or Need help. The presenter chart
uses each room membership's latest answer per normalized topic:

```text
confusion score = round(100 × (Need help + 0.5 × Partly clear) / respondents)
```

A topic with no responses has no score. The score summarizes self-reported
clarity among respondents. It helps a speaker decide what to revisit; it does
not establish the whole room's knowledge or make that decision for them.

## Context and private questions

Audience members can submit their own written questions. When a check-in is
active, the question stores a reference to it. This connects the submission to
that check-in, but does not automatically attach a transcript passage. The
thread is visible to its author and authorized room staff.

Moderators can send private replies; speakers can inspect the threads and
respond aloud. Check-in generation is automatic, while publication and written
replies require a moderator. Flagging and deleting audience questions are not
implemented. Dismissing a generated draft is a separate action.

## Reliability and remaining evaluation

Generation jobs use time-limited ownership, timeouts, and bounded retries.
Candidate creation and its pending delivery are saved together; repeated
messages are checked to avoid duplicate effects. Model failures therefore need
not stop existing audience feedback.

[Recorded checks](environment-verification.md) include actual Nova inference
with synthetic speech. Before making quality, latency, or scale claims, test
representative live explanations and measure relevance, repetition, delay,
moderator edits, and request cost. No comparative model benchmark is claimed.

Implementation: [generation policy](../services/topics-and-questions/src/lib.rs),
[model adapter](../services/topics-and-questions/src/claude.rs),
[publication and permissions](../services/sessions/src/access.rs).
