import { useEffect, useRef, useState } from 'preact/hooks';
import QRCode from 'qrcode';
import { ApiError, api, deniesRoomRead, errorMessage, generatorLabel, isUuid, parseRoomState, parseTranscript, parseWrittenQuestions, paths } from './api';
import type { BeeState, State, Invitation, Membership, Page, LinkResult, QuestionCandidate, TranscriptLine, WrittenQuestion } from './api';
import { followLink, roleLabel, selectedSection, sharePath, tabLabel } from './navigation';
import { clearPublishedQuestionDraft, draftKey, keepDraft, keepInvite, keepJoin, keepPublishedQuestionDraft, keepReviewDraft, memoryDraft, memoryInvite, memoryJoin, memoryPublishedQuestionDraft, memoryReviewDraft } from './pageMemory';
import { parseRoomEvent, reconnectDelay } from './live';
import { chartRows, draftExpiry, earlierWordingNote, formatPercent, respondentSummary, responseOptions } from './feedback';
type LiveStatus = 'connecting' | 'connected' | 'disconnected' | 'ended';

function when(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'Unknown time' : date.toLocaleString();
}

export function ShareLink({ value, kind }: { value: string; kind: 'join' | 'invite' }) {
  const safe = sharePath(kind, value);
  const url = safe ? window.location.origin + safe : '';
  const [qr, setQr] = useState('');
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let current = true;
    setQr('');
    setFailed(false);
    if (!url) return;
    QRCode.toDataURL(url, { width: 280, margin: 4, errorCorrectionLevel: 'M' })
      .then(data => { if (current) setQr(data); })
      .catch(() => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [url]);
  if (!safe) return <p role="alert">The service returned an unusable link. Issue a new link.</p>;
  const label = kind === 'join' ? 'Audience join URL' : 'Private moderator invitation URL';
  return <section class="share-link" aria-label={kind === 'join' ? 'Audience join link' : 'Private moderator invitation'}>
    {qr && <img src={qr} width="280" height="280" alt={kind === 'join' ? 'QR code for the audience join link below' : 'QR code for the private moderator invitation below'} />}
    {failed && <p role="status">QR could not be generated. Use the text link instead.</p>}
    <label>{label}<input readOnly value={url} onFocus={event => event.currentTarget.select()} /></label>
    <a href={safe} target="_blank" rel="noreferrer noopener">Open {kind === 'join' ? 'audience link' : 'moderator invitation'}</a>
    <p>Copy this link now. It is kept only in this page’s memory.{kind === 'invite' && ' Share only with your intended moderator; this invitation is single-use.'}</p>
  </section>;
}

function TranscriptPanel({ roomId }: { roomId: string }) {
  const [lines, setLines] = useState<TranscriptLine[] | null>(null);
  const [error, setError] = useState('');
  const box = useRef<HTMLOListElement>(null);
  useEffect(() => {
    let current = true;
    const load = () => api.read<unknown>(paths.transcript(roomId))
      .then(result => {
        if (!current) return;
        const parsed = parseTranscript(result);
        if (parsed) { setLines(parsed); setError(''); }
        else setError('The transcript response was not understood.');
      })
      .catch(reason => { if (current) setError(errorMessage(reason)); });
    void load();
    // Polling keeps the panel independent of the room event stream.
    const timer = window.setInterval(() => void load(), 3000);
    return () => { current = false; window.clearInterval(timer); };
  }, [roomId]);
  useEffect(() => {
    if (box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines?.length]);
  return <section aria-label="Live transcript">
    <p>What Bee heard, newest last. Only the speaker and moderators see this.</p>
    {error && <p role="status">{error}</p>}
    {lines === null && !error && <p role="status">Loading the transcript…</p>}
    {lines?.length === 0 && <p role="status">Nothing yet. Speech appears here a few seconds after Bee hears it.</p>}
    {lines && lines.length > 0 && <ol class="transcript" ref={box} aria-live="polite">
      {lines.map((line, index) => <li key={index}><time dateTime={line.spoken_at}>{new Date(line.spoken_at).toLocaleTimeString()}</time> {line.text}</li>)}
    </ol>}
  </section>;
}

/** Re-renders on an interval and returns the time of this render. */
function useNow(intervalMs: number): number {
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setTick(tick => tick + 1), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return Date.now();
}

function evidenceTime(candidate: QuestionCandidate): string {
  const at = candidate.evidence.window_end_time ?? candidate.evidence.received_at;
  const date = new Date(at);
  return Number.isNaN(date.getTime()) ? '' : 'Spoken around ' + date.toLocaleTimeString();
}

type Run = (task: () => Promise<void>) => Promise<void>;

function DraftCard({ state, candidate, now, busy, run, replacing }: { state: State; candidate: QuestionCandidate; now: number; busy: boolean; run: Run; replacing: boolean }) {
  const key = draftKey(state.room.id, state.membership.id, 'review-draft\n' + candidate.candidate_id);
  const [text, setText] = useState(() => memoryReviewDraft(key, candidate.text));
  useEffect(() => setText(memoryReviewDraft(key, candidate.text)), [key, candidate.text]);
  const moderator = state.membership.role === 'ta' && state.room.status === 'open';
  const expiry = draftExpiry(candidate.expires_at, now);
  const trimmed = text.trim();
  const changed = trimmed !== candidate.text;
  const invalid = trimmed === '' || [...trimmed].length > 1000;
  const fieldId = 'draft-' + candidate.candidate_id;
  return <li class="draft">
    <p class="draft-meta">
      <span>{generatorLabel(candidate.generator_version)}</span>
      {candidate.edited && <span>Edited by moderator</span>}
      {expiry.label && <span class={expiry.expired ? 'draft-expired' : undefined}>{expiry.label}</span>}
    </p>
    {moderator ? <>
      <label for={fieldId} class="visually-hidden">Question wording</label>
      <textarea id={fieldId} class="draft-text" rows={2} maxLength={1000} value={text} disabled={expiry.expired} onInput={event => {
        const value = event.currentTarget.value;
        setText(value);
        keepReviewDraft(key, candidate.text, value);
      }} />
    </> : <p class="draft-text">{candidate.text}</p>}
    {candidate.edited && candidate.original_text && <p class="draft-original">Generated wording: {candidate.original_text}</p>}
    <figure class="evidence">
      <blockquote>{candidate.evidence.excerpt}</blockquote>
      <figcaption>Transcript evidence{evidenceTime(candidate) && ' · ' + evidenceTime(candidate)}</figcaption>
    </figure>
    {moderator && <div class="actions">
      <button type="button" class="primary" disabled={busy || expiry.expired || invalid} onClick={() => void run(async () => {
        await api.mutate(paths.publishQuestion(state.room.id, candidate.candidate_id), changed ? { text: trimmed } : {});
        keepReviewDraft(key, text, text);
      })}>{replacing ? 'Publish and replace current' : 'Publish'}</button>
      <button type="button" disabled={busy || expiry.expired || invalid || !changed} onClick={() => void run(async () => {
        await api.mutate(paths.saveDraft(state.room.id, candidate.candidate_id), { text: trimmed });
        keepReviewDraft(key, text, text);
      })}>Save edit</button>
      <button type="button" disabled={busy} onClick={() => {
        if (!window.confirm('Dismiss this draft? It will not come back.')) return;
        void run(async () => {
          await api.mutate(paths.dismissDraft(state.room.id, candidate.candidate_id));
          keepReviewDraft(key, text, text);
        });
      }}>Dismiss</button>
    </div>}
  </li>;
}

function QuestionsPanel({ state, refresh, ended }: { state: State; refresh: () => Promise<boolean>; ended: boolean }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const now = useNow(15_000);
  const moderator = state.membership.role === 'ta';
  const owner = draftKey(state.room.id, state.membership.id, 'question-publication');
  const active = state.active_question;
  const [draft, setDraft] = useState(() => active
    ? memoryPublishedQuestionDraft(owner, active.id, active.version, active.text)
    : '');
  useEffect(() => {
    setDraft(active ? memoryPublishedQuestionDraft(owner, active.id, active.version, active.text) : '');
  }, [owner, active?.id, active?.version]);
  const candidates = state.question_candidates ?? [];
  async function run(task: () => Promise<void>) {
    setBusy(true); setError('');
    try { await task(); await refresh(); }
    catch (reason) { setError(errorMessage(reason)); await refresh(); }
    finally { setBusy(false); }
  }
  async function addVersion() {
    if (!state.active_question || draft.trim() === '') return;
    await run(async () => {
      const result = await api.mutate<{ question_id?: string; version?: number }>(paths.questionVersion(state.room.id, state.active_question!.id), { text: draft });
      if (result.question_id && Number.isSafeInteger(result.version)) keepPublishedQuestionDraft(owner, result.question_id, result.version!, draft);
      else clearPublishedQuestionDraft(owner);
    });
  }
  return <>
    {error && <p role="alert" class="error">{error}</p>}
    <h3>Published question</h3>
    {active ? <>
      <p class="published-text">{active.text} <small>Version {active.version}</small></p>
      {active.versions && active.versions.length > 1 && <ul aria-label="Immutable published versions">{active.versions.map(item => <li key={item.version}><strong>Version {item.version}</strong>: {item.text} <small>{when(item.created_at)}</small></li>)}</ul>}
      {moderator && !ended && <details class="version-editor">
        <summary>Correct the published wording</summary>
        <p>This creates version {active.version + 1}. Responses already given stay attached, and the speaker’s chart shows how many were given to earlier wording.</p>
        <label>New immutable version<textarea rows={3} maxLength={1000} value={draft} onInput={e => {
          const value = e.currentTarget.value;
          setDraft(value);
          keepPublishedQuestionDraft(owner, active.id, active.version, value);
        }} /></label>
        <button type="button" disabled={busy || draft.trim() === '' || draft.trim() === active.text} onClick={() => void addVersion()}>Publish new version</button>
      </details>}
    </> : <p>No question is published.</p>}
    <h3>Question drafts</h3>
    <p>{moderator
      ? 'Drafted automatically from the live transcript and visible only to room staff. Check the quoted evidence, edit if needed, then publish. The audience sees a question only after you publish it.'
      : 'Drafted automatically from the live transcript. Your moderator reviews, edits and publishes them; you see them here read-only.'}</p>
    {candidates.length === 0
      ? <p role="status">No drafts right now. Drafts appear after the speaker explains an idea; greetings, pauses, repeated ideas and the minimum gap between drafts can delay them. Unpublished drafts expire after a few minutes.</p>
      : <ul class="drafts" aria-label="Drafts awaiting review">{candidates.map(candidate => <DraftCard key={candidate.candidate_id} state={state} candidate={candidate} now={now} busy={busy} run={run} replacing={active !== null} />)}</ul>}
  </>;
}

function FeedbackPanel({ state, ended, refresh }: { state: State; ended: boolean; refresh: () => Promise<boolean> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const responses = responseOptions;
  async function respond(value: typeof responses[number]['value']) {
    if (!state.active_question) return;
    setBusy(true); setError('');
    try { await api.mutate(paths.respond(state.room.id, state.active_question.id), { response: value }); await refresh(); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }
  return <>
    {error && <p role="alert" class="error">{error}</p>}
    {state.active_question ? <>
      <h3>{state.active_question.text}</h3>
      <p>How clear is this question?</p>
      <div class="actions">{responses.map(item => <button key={item.value} aria-pressed={state.my_response === item.value} disabled={busy || ended} onClick={() => void respond(item.value)}>{item.label}</button>)}</div>
      {state.my_response && <p role="status">Your response: {responses.find(item => item.value === state.my_response)?.label}</p>}
      {state.my_response && typeof state.my_response_version === 'number' && state.my_response_version < state.active_question.version
        && <p class="notice">The wording changed after you responded. Choose again if your answer is different.</p>}
    </> : <p>No active question to respond to.</p>}
  </>;
}

function DashboardPanel({ state }: { state: State }) {
  const dashboard = state.dashboard;
  if (!dashboard) return <p role="status">Response dashboard unavailable.</p>;
  const percent = formatPercent;
  return <>
    <p>{dashboard.respondents} audience response{dashboard.respondents === 1 ? '' : 's'}</p>
    <dl>
      <dt>Clear</dt><dd>{dashboard.counts.clear} ({percent(dashboard.percentages.clear)})</dd>
      <dt>Partly clear</dt><dd>{dashboard.counts.partly_clear} ({percent(dashboard.percentages.partly_clear)})</dd>
      <dt>Need help</dt><dd>{dashboard.counts.need_help} ({percent(dashboard.percentages.need_help)})</dd>
    </dl>
  </>;
}

function SpeakerDashboard({ state, liveStatus }: { state: State; liveStatus: LiveStatus }) {
  const dashboard = state.dashboard;
  const question = state.active_question;
  const ended = state.room.status === 'ended';
  if (!dashboard) return <p role="status">Response results are unavailable. Refresh room state.</p>;
  const rows = chartRows(dashboard);
  return <section class="response-chart" aria-labelledby="chart-question">
    {!ended && liveStatus !== 'connected' && <p role="status" class="notice">
      {liveStatus === 'connecting' ? 'Connecting to live updates…' : 'Live updates are reconnecting. These results may be out of date.'}
    </p>}
    {!question ? <>
      <h3 id="chart-question" class="chart-question">No question is published</h3>
      <p>{ended ? 'This room has ended without an active question.' : 'Your moderator reviews drafts and publishes check-in questions. Results appear here as the audience responds.'}</p>
    </> : <>
      <p class="chart-kicker">{ended ? 'Final results · room ended' : 'Live results'} · Version {question.version}</p>
      <h3 id="chart-question" class="chart-question">{question.text}</h3>
      <p class="chart-total" aria-live="polite">{respondentSummary(dashboard)}</p>
      <ul class="chart-bars" aria-label="Responses by rating">
        {rows.map(row => <li key={row.value} class={'chart-row chart-row--' + row.value}>
          <span class="chart-label">{row.label}</span>
          <span class="chart-track" aria-hidden="true"><span class="chart-fill" style={{ width: row.width + '%' }} /></span>
          <span class="chart-value">
            {row.count}<span class="visually-hidden"> {row.count === 1 ? 'response' : 'responses'}</span>
            {row.percent === null
              ? <span aria-hidden="true"> · —</span>
              : <> <span aria-hidden="true">·</span> {formatPercent(row.percent)}</>}
          </span>
        </li>)}
      </ul>
      {earlierWordingNote(dashboard, question.version) && <p class="notice">{earlierWordingNote(dashboard, question.version)}</p>}
      <p class="chart-note">Percentages are shares of audience members who responded, not of everyone in the room.</p>
    </>}
  </section>;
}

function ReplyForm({ state, item, busy, send }: { state: State; item: WrittenQuestion; busy: boolean; send: (item: WrittenQuestion, body: string) => Promise<boolean> }) {
  const key = draftKey(state.room.id, state.membership.id, 'qa-reply\n' + item.message_id);
  const [text, setText] = useState(() => memoryDraft(key));
  useEffect(() => setText(memoryDraft(key)), [key]);
  const fieldId = 'reply-' + item.message_id;
  return <form class="reply-form" onSubmit={async event => {
    event.preventDefault();
    if (!text.trim() || busy) return;
    if (await send(item, text)) { setText(''); keepDraft(key, ''); }
  }}>
    <label for={fieldId}>Reply privately to {item.author_label ?? 'this participant'}</label>
    <textarea id={fieldId} rows={2} maxLength={2000} value={text} onInput={event => { const value = event.currentTarget.value; setText(value); keepDraft(key, value); }} />
    <button disabled={busy || !text.trim()}>Send reply</button>
  </form>;
}

function QaPanel({ state, ended, revision }: { state: State; ended: boolean; revision: number }) {
  const [items, setItems] = useState<WrittenQuestion[]>([]);
  const owner = draftKey(state.room.id, state.membership.id, state.membership.role);
  const [draft, setDraft] = useState(() => memoryDraft(owner));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const role = state.membership.role;
  async function load() {
    const parsed = parseWrittenQuestions(await api.read<unknown>(paths.qa(state.room.id)));
    if (!parsed) throw new Error('Invalid Q&A');
    setItems(parsed);
  }
  useEffect(() => setDraft(memoryDraft(owner)), [owner]);
  useEffect(() => {
    let current = true;
    api.read<unknown>(paths.qa(state.room.id)).then(result => {
      if (!current) return;
      const parsed = parseWrittenQuestions(result);
      if (parsed) setItems(parsed);
      else setError('The Q&A response was not understood.');
    }).catch(reason => { if (current) setError(errorMessage(reason)); });
    return () => { current = false; };
  }, [state.room.id, revision]);
  async function submit() {
    if (!draft.trim()) return;
    setBusy(true); setError('');
    try {
      await api.mutate(paths.qa(state.room.id), { body: draft, question_id: state.active_question?.id ?? null });
      setDraft('');
      keepDraft(owner, '');
      await load();
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }
  async function sendReply(item: WrittenQuestion, body: string): Promise<boolean> {
    setBusy(true); setError('');
    try {
      await api.mutate(paths.reply(state.room.id, item.message_id), { body });
      await load();
      return true;
    } catch (reason) { setError(errorMessage(reason)); return false; }
    finally { setBusy(false); }
  }
  return <>
    {error && <p role="alert" class="error">{error}</p>}
    {role === 'audience' && <>
      <label>Write a question<textarea rows={4} maxLength={4000} value={draft} readOnly={ended} onInput={e => { const value = e.currentTarget.value; setDraft(value); keepDraft(owner, value); }} /></label>
      <button disabled={busy || ended || !draft.trim()} onClick={() => void submit()}>Send question</button>
      <p>Your written questions are private to you and the room staff. Replies from the moderator appear under your question.</p>
    </>}
    {role === 'ta' && <p>Questions are private to their author. Your reply is visible only to that participant and the room staff.</p>}
    {items.length === 0 ? <p role="status">No written questions.</p> : <ul class="threads">{items.map(item => <li key={item.message_id} class="thread">
      <p class="thread-meta">{item.author_label && <strong>{item.author_label}</strong>} <small>{when(item.created_at)}</small></p>
      <p class="thread-body">{item.body}</p>
      {item.replies.length > 0 && <ol class="thread-replies" aria-label="Replies">{item.replies.map(reply => <li key={reply.reply_id}>
        <p class="thread-meta"><strong>Moderator reply</strong> <small>{when(reply.created_at)}</small></p>
        <p class="thread-body">{reply.body}</p>
      </li>)}</ol>}
      {role === 'ta' && !ended && <ReplyForm state={state} item={item} busy={busy} send={sendReply} />}
    </li>)}</ul>}
  </>;
}

type IssuedInvite = { id: string; expires: string; link: string | null };

function beeLabel(bee: BeeState): string {
  if (bee.status === 'bound' && !bee.source_conversation_id) return 'listening for a Bee recording';
  if (bee.status === 'bound') return 'live';
  return bee.status;
}

function joinStatus(state: State): string {
  if (state.room.status === 'ended') return 'This room has ended; links no longer admit new members.';
  if (state.join_link_enabled === true) return 'The audience link is enabled.';
  if (state.join_link_enabled === false) return 'New audience joins are disabled.';
  return 'Audience link status is unavailable. Refresh room state.';
}

function Access({ state, refresh, loseAccess }: { state: State; refresh: () => Promise<boolean>; loseAccess: () => void }) {
  const roomId = state.room.id;
  const ended = state.room.status === 'ended';
  const [invitations, setInvitations] = useState<Page<Invitation>>();
  const [members, setMembers] = useState<Page<Membership>>();
  const [joinLink, setJoinLink] = useState<string | null>(memoryJoin(roomId));
  const [issued, setIssued] = useState<IssuedInvite | undefined>(memoryInvite(roomId));
  const [beeConversation, setBeeConversation] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [listError, setListError] = useState('');
  const listGeneration = useRef(0);

  useEffect(() => {
    setJoinLink(memoryJoin(roomId));
    setIssued(memoryInvite(roomId));
  }, [roomId, state.revision, state.room.status]);

  async function load() {
    const generation = ++listGeneration.current;
    const invitationsPath = paths.page(paths.invitations(roomId));
    const membersPath = paths.page(paths.memberships(roomId));
    if (!invitationsPath || !membersPath) throw new Error('Invalid list');
    const [nextInvitations, nextMembers] = await Promise.all([
      api.read<Page<Invitation>>(invitationsPath),
      api.read<Page<Membership>>(membersPath),
    ]);
    if (generation !== listGeneration.current) return;
    setInvitations(nextInvitations);
    setMembers(nextMembers);
    setListError('');
  }

  useEffect(() => {
    let current = true;
    load().catch(reason => {
      if (!current) return;
      setListError(errorMessage(reason));
      if (deniesRoomRead(reason)) loseAccess();
    });
    return () => {
      current = false;
      listGeneration.current += 1;
    };
  }, [roomId, state.revision, state.room.status]);

  async function run(task: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    setError('');
    setNotice('');
    try { await task(); }
    catch (reason) {
      setError(errorMessage(reason));
      if (reason instanceof ApiError && reason.status === 401) loseAccess();
    }
    finally { setBusy(false); }
  }

  function rememberJoin(link: string | null) {
    const safe = sharePath('join', link);
    keepJoin(roomId, safe);
    setJoinLink(safe);
    return safe;
  }

  return <section aria-labelledby="access-title" aria-busy={busy}>
    <h2 id="access-title">Room access</h2>
    {error && <p role="alert" class="error">{error}</p>}
    {listError && <p role="alert" class="error">{listError}</p>}
    {notice && <p role="status">{notice}</p>}
    <button type="button" disabled={busy} onClick={() => void run(load)}>Refresh access lists</button>
    <h3>Bee live transcript</h3>
    {!state.bee ? <p role="status">Bee binding status is unavailable.</p> : <>
      <p>Status: <strong>{beeLabel(state.bee)}</strong>{state.bee.has_gaps ? ' · transcript gaps detected' : ''}</p>
      {state.bee.status === 'unbound' ? <>
        <p>Click Start listening, then start recording on your Bee or Apple Watch. SansCue connects the next new Bee conversation to this room.</p>
        <button type="button" disabled={busy || ended} onClick={() => void run(async () => {
          await api.mutate(paths.bindBee(roomId), { listen: true });
          await refresh();
          setNotice('Listening for your next Bee recording.');
        })}>Start listening</button>
        <details>
          <summary>Connect a specific Bee conversation</summary>
          <label>Bee conversation UUID
            <input
              autoComplete="off"
              placeholder="00000000-0000-0000-0000-000000000000"
              value={beeConversation}
              onInput={event => setBeeConversation(event.currentTarget.value)}
            />
          </label>
          <p>Use this when several rooms listen at once. Copy <code>conversation_uuid</code> from <code>bee stream --json</code>.</p>
          <button type="button" disabled={busy || ended || !isUuid(beeConversation.trim())} onClick={() => void run(async () => {
            await api.mutate(paths.bindBee(roomId), { source_conversation_id: beeConversation.trim() });
            setBeeConversation('');
            await refresh();
            setNotice('Bee conversation binding requested.');
          })}>Bind Bee conversation</button>
        </details>
      </> : <button type="button" disabled={busy || state.bee.status === 'unbinding'} onClick={() => {
        if (!window.confirm('Stop routing this Bee conversation to the room?')) return;
        void run(async () => {
          await api.mutate(paths.unbindBee(roomId));
          await refresh();
          setNotice('Bee conversation unbinding requested.');
        });
      }}>{state.bee.status === 'bound' && !state.bee.source_conversation_id ? 'Stop listening' : 'Unbind Bee conversation'}</button>}
    </>}
    <h3>Audience link</h3>
    <p>{joinStatus(state)} Existing members keep their access.</p>
    {!ended && joinLink && state.join_link_enabled === true && <ShareLink value={joinLink} kind="join" />}
    {!ended && state.join_link_enabled === true && !joinLink && <p>The current URL cannot be retrieved. Rotate to issue a new one; the old URL will stop working.</p>}
    <div class="actions">
      <button type="button" disabled={busy || ended} onClick={() => {
        if (!window.confirm('Issue a new audience link? The previous link will stop working.')) return;
        void run(async () => {
          const result = await api.mutate<LinkResult>(paths.rotateJoin(roomId));
          const safe = rememberJoin(result.join_url ?? null);
          setNotice(result.link_unavailable || !safe ? 'Rotation was recovered, but its link is unavailable. Rotate again to get a new link.' : 'New audience link issued.');
          await refresh();
        });
      }}>Rotate audience link</button>
      <button type="button" disabled={busy || ended || state.join_link_enabled !== true} onClick={() => {
        if (!window.confirm('Disable new audience joins? Existing memberships stay active.')) return;
        void run(async () => {
          await api.mutate(paths.revokeJoin(roomId));
          rememberJoin(null);
          await refresh();
          setNotice('Audience link revoked.');
        });
      }}>Revoke audience link</button>
    </div>
    <h3>Moderator invitations</h3>
    <p>Invite the person who will review drafts, publish questions and reply to the audience. Speakers cannot publish questions.</p>
    <button type="button" disabled={busy || ended} onClick={() => void run(async () => {
      const result = await api.mutate<LinkResult & { invitation_id?: string; expires_at?: string }>(paths.invitations(roomId), {}, 201);
      const link = sharePath('invite', result.invite_url ?? null);
      const id = result.invitation_id;
      if (typeof id !== 'string' || id === '') throw new Error('Invalid invitation');
      const next = { id, expires: result.expires_at ?? '', link };
      keepInvite(roomId, next);
      setIssued(next);
      setNotice(result.link_unavailable || !link ? 'Invitation created, but its link was lost. Revoke the identified invitation below and create another.' : 'Invitation created.');
      await load();
    })}>Create moderator invitation</button>
    {issued && <p>Issued invitation: <code>{issued.id}</code> · Expires {issued.expires ? when(issued.expires) : 'Unknown time'}</p>}
    {issued?.link && !ended && <ShareLink value={issued.link} kind="invite" />}
    {!invitations ? <p role="status">{listError ? 'Invitation list not loaded.' : 'Loading invitations…'}</p> : <>
      {invitations.items.length === 0 && <p>No invitations.</p>}
      <ul class="access-list">{invitations.items.map(item => <li key={item.invitation_id}>
        <div><code>{item.invitation_id}</code><p>{item.status} · Expires {when(item.expires_at)}</p></div>
        {item.status !== 'revoked' && item.status !== 'expired' && <button type="button" disabled={busy} onClick={() => {
          if (!window.confirm('Revoke this invitation? A redeemed invitation also revokes access if its grant is still current.')) return;
          void run(async () => {
            await api.mutate(paths.revokeInvitation(roomId, item.invitation_id));
            if (issued?.id === item.invitation_id) { keepInvite(roomId, undefined); setIssued(undefined); }
            await load();
            await refresh();
            setNotice('Access updated.');
          });
        }}>Revoke invitation</button>}
      </li>)}</ul>
      {invitations.next_cursor && <button type="button" disabled={busy} onClick={() => void run(async () => {
        const nextPath = paths.page(paths.invitations(roomId), invitations.next_cursor);
        if (!nextPath) { setError('The invitation list cannot be continued.'); return; }
        const next = await api.read<Page<Invitation>>(nextPath);
        setInvitations({ ...next, items: [...invitations.items, ...next.items] });
      })}>More invitations</button>}
    </>}
    <h3>Memberships</h3>
    <p>Room-local aliases, not personal identities. Last activity is not a presence guarantee.</p>
    {!members ? <p role="status">{listError ? 'Membership list not loaded.' : 'Loading memberships…'}</p> : <>
      {members.items.length === 0 && <p>No memberships.</p>}
      <ul class="access-list">{members.items.map(item => <li key={item.membership_id}>
        <div><strong>{item.label}</strong><p>{roleLabel(item.role)} · {item.status} · Last activity: {item.last_seen_at ? when(item.last_seen_at) : 'Not recorded'}</p></div>
        {item.role !== 'speaker' && item.status === 'active' && <button type="button" disabled={busy} onClick={() => {
          if (!window.confirm('Revoke access for this member? This takes effect immediately.')) return;
          void run(async () => {
            await api.mutate(paths.revokeMembership(roomId, item.membership_id));
            await load();
            await refresh();
            setNotice('Access updated.');
          });
        }}>Revoke access</button>}
      </li>)}</ul>
      {members.next_cursor && <button type="button" disabled={busy} onClick={() => void run(async () => {
        const nextPath = paths.page(paths.memberships(roomId), members.next_cursor);
        if (!nextPath) { setError('The membership list cannot be continued.'); return; }
        const next = await api.read<Page<Membership>>(nextPath);
        setMembers({ ...next, items: [...members.items, ...next.items] });
      })}>More memberships</button>}
    </>}
  </section>;
}

export function Room({ id, tab }: { id: string; tab?: string }) {
  const [state, setState] = useState<State>();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [liveStatus, setLiveStatus] = useState<LiveStatus>('connecting');
  const seenRevision = useRef(-1);
  const liveSequence = useRef(0);
  const refreshLive = useRef<(() => Promise<boolean>) | undefined>(undefined);

  function show(next: State) {
    if (next.revision < seenRevision.current) return;
    seenRevision.current = next.revision;
    liveSequence.current = Math.max(liveSequence.current, next.sequence);
    setState(next);
  }

  async function refresh(): Promise<boolean> {
    setBusy(true);
    setError('');
    try {
      const body = await api.read<unknown>(paths.state(id));
      const next = parseRoomState(body, id);
      if (!next) throw new Error('Invalid state');
      show(next);
      return true;
    } catch (reason) {
      setError(errorMessage(reason));
      if (deniesRoomRead(reason)) setState(undefined);
      return false;
    } finally { setBusy(false); }
  }

  refreshLive.current = refresh;

  useEffect(() => {
    seenRevision.current = -1;
    liveSequence.current = 0;
    setState(undefined);
    void refresh();
  }, [id]);

  useEffect(() => {
    if (!state || state.room.id.toLowerCase() !== id.toLowerCase()) return;
    if (state.room.status === 'ended') {
      setLiveStatus('ended');
      return;
    }
    let current = true;
    let refreshing = false;
    let pendingRefresh = false;
    let pendingSequence = liveSequence.current;
    let attempt = 0;
    let socket: WebSocket | undefined;
    let reconnectTimer: number | undefined;
    setLiveStatus('connecting');
    function scheduleReconnect() {
      if (!current || reconnectTimer !== undefined) return;
      reconnectTimer = window.setTimeout(() => {
        reconnectTimer = undefined;
        connect();
      }, reconnectDelay(attempt++));
    }
    function connect() {
      if (!current) return;
      const socketUrl = new URL('/api/sessions' + paths.events(id, liveSequence.current), window.location.href);
      socketUrl.protocol = socketUrl.protocol === 'https:' ? 'wss:' : 'ws:';
      const connection = new WebSocket(socketUrl);
      socket = connection;
      connection.addEventListener('open', () => { if (current) setLiveStatus('connected'); });
      connection.addEventListener('message', event => {
        if (typeof event.data !== 'string') return;
        let update: unknown;
        try { update = JSON.parse(event.data); } catch { return; }
        const message = parseRoomEvent(update, id, liveSequence.current);
        if (!message) return;
        pendingSequence = Math.max(pendingSequence, message.sequence);
        pendingRefresh = true;
        if (refreshing) return;
        refreshing = true;
        void (async () => {
          while (current && pendingRefresh) {
            pendingRefresh = false;
            if (!await refreshLive.current?.()) {
              connection.close();
              break;
            }
            // refresh() advances the replay cursor only after a valid snapshot
            // is received, so reconnecting replays any unreflected changes.
            if (liveSequence.current < pendingSequence) {
              connection.close();
              break;
            }
          }
        })().finally(() => { refreshing = false; });
      });
      connection.addEventListener('close', () => {
        if (!current) return;
        setLiveStatus('disconnected');
        const refreshAfterDisconnect = refreshLive.current;
        if (refreshAfterDisconnect) void refreshAfterDisconnect().finally(scheduleReconnect);
        else scheduleReconnect();
      });
      connection.addEventListener('error', () => connection.close());
    }
    connect();
    return () => {
      current = false;
      if (reconnectTimer !== undefined) window.clearTimeout(reconnectTimer);
      socket?.close(1000, 'room view changed');
    };
  }, [id, state?.membership.id, state?.room.status]);

  if (!state) {
    return <section class="panel"><h1>Room</h1>
      {busy ? <p role="status">Loading room…</p> : <>
        <p role="alert" class="error">{error}</p>
        <button type="button" onClick={() => void refresh()}>Retry room state</button>
        <p><a href="/operator" onClick={event => followLink(event, '/operator')}>Operator sign-in</a> · Ask the speaker for a valid join or invitation link.</p>
      </>}
    </section>;
  }

  const { tabs, selected, redirected } = selectedSection(state.membership.role, tab);
  const ended = state.room.status === 'ended';
  return <section class="room">
    <p class="eyebrow">{roleLabel(state.membership.role)} · {state.room.status}</p>
    <h1>{state.room.title}</h1>
    {ended && <p role="status" class="notice">Room ended. Saved state is read-only. Ending a room does not delete its data.</p>}
    {error && <p role="alert" class="error">{error}</p>}
    <div class="actions">
      <button type="button" disabled={busy} onClick={() => void refresh()}>{busy ? 'Refreshing…' : 'Refresh room state'}</button>
      {state.membership.role === 'speaker' && !ended && <button type="button" disabled={busy} onClick={async () => {
        if (!window.confirm('End this room? New joins and participation stop permanently. Saved data is retained.')) return;
        setBusy(true);
        setError('');
        try {
          const result = await api.mutate<{ revision?: unknown }>(paths.end(id));
          keepJoin(id, null);
          keepInvite(id, undefined);
          if (state) {
            const reported = result.revision;
            const revision = typeof reported === 'number' && Number.isSafeInteger(reported) && reported >= state.revision ? reported : state.revision;
            show({
              ...state,
              room: { ...state.room, status: 'ended' },
              join_link_enabled: false,
              active_question: null,
              revision,
              sequence: revision,
            });
          }
          await refresh();
        } catch (reason) {
          setError(errorMessage(reason));
          if (deniesRoomRead(reason)) setState(undefined);
        } finally { setBusy(false); }
      }}>End room</button>}
    </div>
    <p class="service-note" role="status">{liveStatus === 'connected' ? 'Live updates connected.' : liveStatus === 'connecting' ? 'Connecting to live updates…' : liveStatus === 'ended' ? 'Room ended. Live updates are closed.' : 'Live updates disconnected. Reconnecting…'}</p>
    <nav class="room-tabs" aria-label="Room sections">{tabs.map(name => <a key={name} href={'/rooms/' + id + '/' + name} aria-current={selected === name ? 'page' : undefined} onClick={event => followLink(event, '/rooms/' + id + '/' + name)}>{tabLabel(name)}</a>)}</nav>
    {redirected && <p role="status">That section is unavailable for your role. Showing {tabLabel(selected)}.</p>}
    <div class="panel">
      {selected === 'access' && state.membership.role === 'speaker'
        ? <Access state={state} refresh={refresh} loseAccess={() => { setError('Room access changed. Refresh or sign in again.'); setState(undefined); }} />
        : <>
          <h2>{tabLabel(selected)}</h2>
          {selected === 'qa' ? <QaPanel state={state} ended={ended} revision={state.revision} />
            : selected === 'response' ? <FeedbackPanel state={state} ended={ended} refresh={refresh} />
            : selected === 'dashboard' && state.membership.role === 'speaker' ? <SpeakerDashboard state={state} liveStatus={liveStatus} />
            : selected === 'dashboard' ? <DashboardPanel state={state} />
            : selected === 'questions' && state.membership.role !== 'audience' ? <QuestionsPanel state={state} refresh={refresh} ended={ended} />
            : selected === 'transcript' && state.membership.role !== 'audience' ? <TranscriptPanel roomId={state.room.id} />
            : <p>{selected === 'questions' ? 'Candidate preview is not available to audience members.' : 'No active question is available.'}</p>}
        </>}
    </div>
  </section>;
}
