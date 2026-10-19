# Question generation design decision

## Decision

Prefer **evidence-first, human-reviewed question generation** over free-form
generation from an entire transcript. A candidate must carry an exact source
quote from the originating transcript event; the Topics service verifies that
the quote is a contiguous substring before it can be persisted or delivered.
The question text and evidence quote are distinct fields: the quote is
provenance, not a claim that the transcript already answers the question.

The next model-backed stage should select a question-worthy claim or unresolved
point from a coherent topic window, then phrase one concise, self-contained
follow-up grounded in that anchor. It should suppress weakly grounded,
already-answered, repetitive, or low-value suggestions and allow abstention.
The current feature branch implements provisional rolling windows, silence
boundaries, cadence, and deterministic abstention. These defaults are
development policy, not accepted question quality; representative targets and
model evaluation remain open.

## Why this approach

- A full-transcript prompt makes it difficult to see which passage motivated a
  candidate and encourages unsupported details. An exact quote makes the
  proposed anchor inspectable.
- The service, not the model, builds conversation ID, event ordinal, timestamp,
  and excerpt metadata. The generator cannot invent event provenance.
- Quote validation is a cheap deterministic guardrail. It does **not** prove
  semantic entailment; the speaker/TA review and the model evaluation must still
  check relevance, correctness, clarity, redundancy, and usefulness.
- Generating against a selected anchor gives evaluation a stable basis for
  checking groundedness and answerability. Research on meeting-transcript
  dialogs similarly uses human verification and evidence attribution; question
  generation evaluations also separate answerability, non-redundancy, and
  coverage. See [the meeting-transcript study](https://arxiv.org/abs/2405.01121)
  and [AURA-QG](https://aclanthology.org/2025.ijcnlp-long.159/).

## Implemented boundary and remaining work

- `GeneratedQuestion` now includes an evidence excerpt. Before a candidate is
  accepted, Topics checks that the question is non-empty, bounded, phrased as a
  question, and that its bounded evidence excerpt is an exact substring of
  this event's transcript text.
- Candidate evidence uses that validated excerpt. Sessions and the web client
  accept bounded, non-empty generator-version labels rather than hardcoding
  `stub-v1`, so a later evaluated provider can identify its generator version.
- With `ANTHROPIC_API_KEY` set, Topics uses `claude-opus-5-5` (override with
  `TOPICS_MODEL`) after the deterministic topic-window gate passes. The model
  receives the window as numbered transcript lines and returns structured JSON:
  abstain, or one Clear / Partly clear / Need help check-in plus an evidence
  quote. The quote is matched back to the transcript's exact spelling; an
  unmatched quote abstains as `weakly_grounded`. Speech older than 45 seconds
  abstains as `stale_transcript`, so a backlog never produces late questions.
  Candidates are labelled `<model>/topic-check-v1`.
- Without a key, `topic-window-stub-v1` remains the runtime generator. It is a
  deterministic placeholder, not an LLM or product-quality question generator.
- A two-model comparison on consented excerpts has not been run; see the
  [model comparison worksheet](model-evaluation-worksheet.md).
- Before enabling a model, validate or revise the provisional window/cadence
  policy; compare two providers on the same frozen excerpts; record the exact
  model and prompt; and keep speaker/TA approval before publishing.
