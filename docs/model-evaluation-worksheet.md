# Model comparison worksheet

**Status: no completed comparison recorded.** Anthropic and Bedrock generation
are implemented, with a deterministic placeholder when unconfigured.
[PR #3](https://github.com/DahalAb1/SansCue/pull/3) records a real Bedrock request
that hit the daily token limit before a live AI draft appeared. Connectivity is
not model-quality acceptance. Use this worksheet after agreeing on excerpts and
quality/latency/cost targets. Do not enter credentials here.

## Evaluation setup

```text
Evaluation owner/date:
SansCue commit:
Question format and topic boundaries:
Context-selection rule (recent passages / earlier same-session context):
Consent and data-handling approval/reference:
Dataset version and number of excerpts:
Prompt text/version (store exact prompt in a separate non-secret artifact):
Decoding/settings shared by candidates:
Candidate A: provider / exact model ID / region:
Candidate B: provider / exact model ID / region:
Quality target and acceptance rule (agree before running):
Latency target and percentile (agree before running):
Cost target and unit (agree before running):
Repeat count per excerpt/model:
Reviewer(s), blinding/randomization method:
```

If any target is blank, results can still be exploratory, but they cannot
establish Step 9 success or justify a production model choice.

## Dataset manifest

Use stable, non-identifying excerpt IDs. Keep raw audio/transcripts and any
identifying mapping in the approved private store, not this worksheet or Git.
Include representative speech patterns and context lengths; record exclusions
and why. Freeze the manifest before candidate generation.

| Excerpt ID | Session context included | Language / speech conditions | Consent reference | Exclusions / notes |
|---|---|---|---|---|
|  |  |  |  |  |

## Blind output scoring

Use identical excerpts, instructions, and settings for both candidates. Hide
provider/model labels from quality reviewers until scoring is complete. The
1–5 scale below is a worksheet convention, not an agreed acceptance threshold:
1 = unusable, 3 = mixed/needs revision, 5 = strong. Agree on whether to use it
and on the target before the run.

| Excerpt ID | Blind candidate ID | Generated question | Evidence reference(s) | Grounded / correct (1–5) | Relevant / precise (1–5) | Clear / answerable (1–5) | Unsupported claim or privacy issue | Reviewer notes |
|---|---|---|---|---:|---:|---:|---|---|
|  |  |  |  |  |  |  |  |  |

Evidence references should point to the approved private excerpt and its
minimal supporting span, not duplicate transcript content in this report.
Record unusable output, refusal, timeout, or invalid evidence explicitly; do
not omit failed calls from the summary.

## Runtime and cost observations

Record one row per attempt, including failures and retries. Use provider-reported
usage and pricing captured for the evaluation date; do not estimate from model
names alone.

| Excerpt ID | Blind candidate ID | Attempt | End-to-end latency (ms) | Input tokens | Output tokens | Provider cost | Currency / pricing date | Error / retry |
|---|---|---:|---:|---:|---:|---:|---|---|
|  |  |  |  |  |  |  |  |  |

## Summary and decision

| Candidate | Exact provider/model ID | Quality summary | P50 / target latency | P95 / target latency | Cost per agreed unit / target | Failure rate | Decision / rationale |
|---|---|---|---|---|---|---|---|
| A |  |  |  |  |  |  |  |
| B |  |  |  |  |  |  |  |

Document the aggregation method and how disagreements were resolved. Decide
whether each candidate meets every pre-agreed target; do not trade quality,
latency, or cost against each other after seeing results unless the decision
rule is explicitly revised and the comparison rerun. If neither candidate
passes, record that outcome and revise the prompt, targets, or candidate set
before another evaluation.

```text
Decision (A / B / neither / inconclusive):
Exact selected model ID and settings (if any):
Prompt/version selected:
Unresolved quality or safety issues:
Evaluation artifact location (private if it contains sensitive data):
Reviewer sign-off/date:
```
