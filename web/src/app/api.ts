export type Role = 'speaker' | 'ta' | 'audience';
export type BeeState = {
  binding_id: string | null;
  status: 'unbound' | 'binding' | 'bound' | 'unbinding';
  has_gaps: boolean;
  connectivity_revision: number;
  /** The Bee conversation feeding this room; null while a listening room waits for one. */
  source_conversation_id?: string | null;
};
export type State = {
  room: { id: string; title: string; status: 'open' | 'ended' };
  membership: { id: string; role: Role };
  revision: number;
  sequence: number;
  active_question: { id: string; version: number; text: string; topic?: string; evidence?: unknown; versions?: Array<{ version: number; text: string; created_at: string }> } | null;
  /** Speaker-only ranking of topics by confusion. Absent for every other role. */
  topics?: TopicScore[] | null;
  my_response?: 'clear' | 'partly_clear' | 'need_help' | null;
  /** Wording version that was active when this audience member last responded. */
  my_response_version?: number | null;
  dashboard?: {
    respondents: number;
    counts: { clear: number; partly_clear: number; need_help: number };
    percentages: { clear: number | null; partly_clear: number | null; need_help: number | null };
    /** Respondents whose saved rating predates the current wording. */
    earlier_version_respondents?: number;
  } | null;
  join_link_enabled?: boolean;
  question_candidates?: QuestionCandidate[] | null;
  bee?: BeeState | null;
};
export type TopicScore = { label: string; confusion: number; respondents: number; collecting: boolean };
export type TranscriptLine = { text: string; spoken_at: string };
export type QuestionCandidate = {
  candidate_id: string;
  event_id: string;
  /** Current draft wording: the moderator's saved edit, or the generated text. */
  text: string;
  original_text?: string;
  edited?: boolean;
  edited_at?: string | null;
  generator_version: string;
  evidence: { conversation_id: string; ingest_ordinal: number; received_at: string; excerpt: string; window_start_time?: string | null; window_end_time?: string | null };
  created_at: string;
  expires_at?: string;
  published: false;
};
export type QaReply = { reply_id: string; body: string; created_at: string; from: 'moderator' };
export type WrittenQuestion = { message_id: string; question_id: string | null; body: string; created_at: string; author_label?: string; replies: QaReply[] };
export type Page<T> = { items: T[]; next_cursor: string | null };
export type Invitation = {
  invitation_id: string;
  status: 'pending' | 'redeemed' | 'revoked' | 'expired';
  membership_id: string | null;
  grant_id: string | null;
  expires_at: string;
};
export type Membership = {
  membership_id: string;
  label: string;
  role: Role;
  status: 'active' | 'revoked';
  last_seen_at: string | null;
  current_grant_id: string | null;
};
export type LinkResult = { join_url?: string | null; invite_url?: string | null; link_unavailable?: boolean };

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function isUuid(value: unknown): value is string {
  return typeof value === 'string'
    && UUID.test(value)
    && value.toLowerCase() !== '00000000-0000-0000-0000-000000000000';
}

export function segment(value: string): string {
  return encodeURIComponent(value);
}

export const paths = {
  bootstrap: '/bootstrap',
  login: '/operator/login',
  operatorSession: '/operator/session',
  rooms: '/rooms',
  state: (id: string) => '/rooms/' + segment(id) + '/state',
  events: (id: string, after: number) => '/rooms/' + segment(id) + '/events?after=' + encodeURIComponent(String(after)),
  join: (token: string) => '/join/' + segment(token),
  redeem: (token: string) => '/invitations/' + segment(token) + '/redeem',
  invitations: (id: string) => '/rooms/' + segment(id) + '/invitations',
  memberships: (id: string) => '/rooms/' + segment(id) + '/memberships',
  revokeInvitation: (id: string, invitationId: string) => '/rooms/' + segment(id) + '/invitations/' + segment(invitationId) + '/revoke',
  revokeMembership: (id: string, membershipId: string) => '/rooms/' + segment(id) + '/memberships/' + segment(membershipId) + '/revoke',
  rotateJoin: (id: string) => '/rooms/' + segment(id) + '/join-link/rotate',
  revokeJoin: (id: string) => '/rooms/' + segment(id) + '/join-link/revoke',
  end: (id: string) => '/rooms/' + segment(id) + '/end',
  bindBee: (id: string) => '/rooms/' + segment(id) + '/bee/bind',
  beeConnection: (id: string) => '/rooms/' + segment(id) + '/bee-connection',
  unbindBee: (id: string) => '/rooms/' + segment(id) + '/bee/unbind',
  qa: (id: string) => '/rooms/' + segment(id) + '/qa',
  transcript: (id: string) => '/rooms/' + segment(id) + '/transcript',
  publishQuestion: (id: string, candidateId: string) => '/rooms/' + segment(id) + '/questions/' + segment(candidateId) + '/publish',
  saveDraft: (id: string, candidateId: string) => '/rooms/' + segment(id) + '/questions/' + segment(candidateId) + '/draft',
  dismissDraft: (id: string, candidateId: string) => '/rooms/' + segment(id) + '/questions/' + segment(candidateId) + '/dismiss',
  reply: (id: string, messageId: string) => '/rooms/' + segment(id) + '/qa/' + segment(messageId) + '/replies',
  questionVersion: (id: string, questionId: string) => '/rooms/' + segment(id) + '/questions/' + segment(questionId) + '/versions',
  respond: (id: string, questionId: string) => '/rooms/' + segment(id) + '/questions/' + segment(questionId) + '/response',
  page(path: string, cursor?: string | null): string | null {
    if (!cursor) return path + '?limit=50';
    // Cursors are opaque: pass them through unchanged with no length cap, only encoded for a query value.
    if (typeof cursor !== 'string') return null;
    try { return path + '?limit=50&after=' + encodeURIComponent(cursor); }
    catch { return null; }
  },
};

export class ApiError extends Error {
  constructor(public status: number, public code: string) { super(code); this.name = 'ApiError'; }
}

/** Accepts only a list of bounded text lines with timestamps; anything else is rejected. */
export function parseTranscript(value: unknown): TranscriptLine[] | null {
  const lines = (value as { lines?: unknown } | null)?.lines;
  if (!Array.isArray(lines)) return null;
  const parsed: TranscriptLine[] = [];
  for (const line of lines) {
    const { text, spoken_at } = (line ?? {}) as Record<string, unknown>;
    if (typeof text !== 'string' || typeof spoken_at !== 'string' || text.length > 16000) return null;
    parsed.push({ text, spoken_at });
  }
  return parsed;
}

/** Names the generator behind a candidate in words a speaker recognizes. */
export function generatorLabel(version: string): string {
  if (version.includes('stub')) return 'Placeholder generator, not AI';
  const model = version.split('/')[0].replace(/^(us|global|eu|apac|jp)\./, '').replace(/^anthropic\./, '').replace(/-\d{8}-v\d+:\d+$/, '');
  const match = /^claude-([a-z]+)-(\d+)-(\d+)$/.exec(model);
  if (!match) return 'Drafted by ' + model;
  return 'Drafted by Claude ' + match[1][0].toUpperCase() + match[1].slice(1) + ' ' + match[2] + '.' + match[3];
}

export function errorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) return 'Unable to reach the sessions service or read its response. Retry the same action; its request key is retained.';
  if (error.status === 401) return 'Your session has expired. Reconnect, then use a valid link or sign in as the operator again.';
  if (error.code === 'room_ended') return 'This room has ended. New joins and participation are closed.';
  if (error.status === 404) return 'Not found or access unavailable. The link may be invalid, expired or revoked.';
  if (error.code === 'invite_used') return 'This invitation has already been used or cannot change your current role. Ask the speaker for access.';
  if (error.code === 'request_in_progress') return 'This request is still in progress. Retry to recover its result.';
  if (error.code === 'idempotency_conflict') return 'That retry did not match the original request. Submit it again as a new action.';
  if (error.code === 'transcript_unavailable') return 'The live transcript is unavailable right now. It retries automatically.';
  if (error.code === 'draft_dismissed') return 'This draft was dismissed and can no longer be published.';
  if (error.code === 'draft_expired') return 'This draft expired before it was published. Wait for a fresh draft.';
  if (error.code === 'question_already_published') return 'This draft has already been published.';
  if (error.code === 'bee_already_bound') return 'Unbind the current Bee conversation before binding another one.';
  if (error.code === 'bee_source_already_bound') return 'That Bee conversation is already bound to another room.';
  if (error.status === 403) return 'Access denied. Your access or security session may have changed. Reconnect or contact the speaker.';
  if (error.status === 429) return 'Too many attempts. Wait before trying again.';
  if (error.status === 409) return 'The room or request has changed. Refresh its state before continuing.';
  if (error.status === 400) return 'The request was not accepted. Check the form values.';
  return 'The sessions service is unavailable. Retry when it recovers.';
}

export function deniesRoomRead(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.status === 404);
}

function isTime(value: unknown): value is string {
  return typeof value === 'string' && !Number.isNaN(Date.parse(value));
}

function isCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

/** Accepts only well-formed private Q&A threads; anything else is rejected. */
export function parseWrittenQuestions(value: unknown): WrittenQuestion[] | null {
  const items = (value as { items?: unknown } | null)?.items;
  if (!Array.isArray(items)) return null;
  const parsed: WrittenQuestion[] = [];
  for (const item of items) {
    const { message_id, question_id, body, created_at, author_label, replies } = (item ?? {}) as Record<string, unknown>;
    if (!isUuid(message_id) || (question_id !== null && !isUuid(question_id)) || typeof body !== 'string' || !isTime(created_at)) return null;
    if (author_label !== undefined && typeof author_label !== 'string') return null;
    const thread = replies === undefined ? [] : replies;
    if (!Array.isArray(thread)) return null;
    const parsedReplies: QaReply[] = [];
    for (const reply of thread) {
      const r = (reply ?? {}) as Record<string, unknown>;
      if (!isUuid(r.reply_id) || typeof r.body !== 'string' || !isTime(r.created_at) || r.from !== 'moderator') return null;
      parsedReplies.push({ reply_id: r.reply_id, body: r.body, created_at: r.created_at, from: 'moderator' });
    }
    parsed.push({ message_id, question_id: question_id as string | null, body, created_at, ...(author_label === undefined ? {} : { author_label }), replies: parsedReplies });
  }
  return parsed;
}

export function parseRoomState(value: unknown, expectedId?: string): State | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
  const state = value as State;
  const room = state.room;
  const membership = state.membership;
  if (typeof room !== 'object' || room === null || typeof membership !== 'object' || membership === null) return null;
  if (typeof room.id !== 'string' || !UUID.test(room.id)) return null;
  if (expectedId && room.id.toLowerCase() !== expectedId.toLowerCase()) return null;
  if (room.status !== 'open' && room.status !== 'ended') return null;
  if (typeof room.title !== 'string' || room.title.trim() === '' || [...room.title].length > 200) return null;
  if (typeof membership.id !== 'string' || !UUID.test(membership.id)) return null;
  if (membership.role !== 'speaker' && membership.role !== 'ta' && membership.role !== 'audience') return null;
  if (!Number.isSafeInteger(state.revision) || state.revision < 0) return null;
  if (!Number.isSafeInteger(state.sequence) || state.sequence < 0) return null;
  if (state.active_question !== null && (typeof state.active_question !== 'object' || state.active_question === null || Array.isArray(state.active_question))) return null;
  if (state.active_question !== null && (typeof state.active_question.id !== 'string' || !UUID.test(state.active_question.id) || !Number.isSafeInteger(state.active_question.version) || state.active_question.version < 1 || typeof state.active_question.text !== 'string' || state.active_question.text.trim() === '')) return null;
  if (state.membership.role === 'audience' && state.active_question !== null && Object.keys(state.active_question).some(key => !['id','version','text'].includes(key))) return null;
  if (state.active_question?.topic !== undefined && (state.membership.role === 'audience' || typeof state.active_question.topic !== 'string' || state.active_question.topic.trim() === '' || [...state.active_question.topic].length > 80)) return null;
  if (state.membership.role !== 'speaker' && state.topics != null) return null;
  if (Array.isArray(state.topics) && state.topics.some(topic => !topic || typeof topic.label !== 'string' || topic.label.trim() === '' || [...topic.label].length > 80 || !isCount(topic.confusion) || topic.confusion > 100 || !isCount(topic.respondents) || topic.respondents < 1 || typeof topic.collecting !== 'boolean')) return null;
  if (state.topics != null && !Array.isArray(state.topics)) return null;
  if (state.my_response !== undefined && state.my_response !== null && !['clear','partly_clear','need_help'].includes(state.my_response)) return null;
  if (state.membership.role === 'audience' && state.dashboard !== undefined && state.dashboard !== null) return null;
  if (state.membership.role !== 'audience' && state.dashboard !== undefined && state.dashboard !== null && (typeof state.dashboard.respondents !== 'number' || !Number.isSafeInteger(state.dashboard.respondents) || !state.dashboard.counts || !state.dashboard.percentages)) return null;
  if (state.join_link_enabled !== undefined && typeof state.join_link_enabled !== 'boolean') return null;
  if (state.question_candidates !== undefined && state.membership.role === 'audience' && state.question_candidates !== null) return null;
  if (state.question_candidates !== undefined && state.membership.role !== 'audience' && state.question_candidates === null) return null;
  if (state.question_candidates !== undefined && state.question_candidates !== null && (!Array.isArray(state.question_candidates) || state.question_candidates.some(c => !c || typeof c !== 'object' || typeof c.candidate_id !== 'string' || !UUID.test(c.candidate_id) || typeof c.event_id !== 'string' || !UUID.test(c.event_id) || typeof c.text !== 'string' || c.text.trim() === '' || typeof c.generator_version !== 'string' || c.generator_version.trim() === '' || c.generator_version.length > 128 || /[\u0000-\u001f\u007f]/.test(c.generator_version) || c.published !== false || typeof c.created_at !== 'string' || Number.isNaN(Date.parse(c.created_at)) || !c.evidence || typeof c.evidence !== 'object' || typeof c.evidence.conversation_id !== 'string' || !UUID.test(c.evidence.conversation_id) || !Number.isSafeInteger(c.evidence.ingest_ordinal) || c.evidence.ingest_ordinal < 1 || typeof c.evidence.received_at !== 'string' || Number.isNaN(Date.parse(c.evidence.received_at)) || typeof c.evidence.excerpt !== 'string' || c.evidence.excerpt.trim() === ''))) return null;
  if (state.question_candidates?.some(c => (c.original_text !== undefined && typeof c.original_text !== 'string')
    || (c.edited !== undefined && typeof c.edited !== 'boolean')
    || (c.edited_at !== undefined && c.edited_at !== null && !isTime(c.edited_at))
    || (c.expires_at !== undefined && !isTime(c.expires_at)))) return null;
  if (state.dashboard?.earlier_version_respondents !== undefined && !isCount(state.dashboard.earlier_version_respondents)) return null;
  if (state.my_response_version !== undefined && state.my_response_version !== null && !(isCount(state.my_response_version) && state.my_response_version >= 1)) return null;
  if (state.membership.role === 'audience' && state.bee !== undefined && state.bee !== null) return null;
  if (state.membership.role !== 'audience') {
    if (!state.bee || typeof state.bee !== 'object') return null;
    if (state.bee.binding_id !== null && !isUuid(state.bee.binding_id)) return null;
    if (!['unbound', 'binding', 'bound', 'unbinding'].includes(state.bee.status)) return null;
    if (typeof state.bee.has_gaps !== 'boolean') return null;
    if (!Number.isSafeInteger(state.bee.connectivity_revision) || state.bee.connectivity_revision < 0) return null;
  }
  return state;
}

export function preferState(previous: State | undefined, incoming: State): State {
  if (!previous || incoming.revision >= previous.revision) return incoming;
  return previous;
}

// Request keys and the CSRF value stay on this instance, never in web storage.
export class SessionsApi {
  #csrf: string | undefined;
  #bootstrapPending: Promise<void> | undefined;
  #keys = new Map<string, string>();

  async bootstrap(): Promise<void> {
    if (!this.#bootstrapPending) {
      this.#bootstrapPending = this.read<{ csrf_token?: unknown }>(paths.bootstrap).then(body => {
        if (typeof body?.csrf_token !== 'string' || body.csrf_token === '') throw new Error('Invalid bootstrap');
        this.#csrf = body.csrf_token;
      }).finally(() => { this.#bootstrapPending = undefined; });
    }
    return this.#bootstrapPending;
  }

  private async request<T>(path: string, init: RequestInit, status: number): Promise<T> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 15000);
    try {
      const response = await fetch('/api/sessions' + path, {
        ...init,
        credentials: 'same-origin',
        cache: 'no-store',
        referrerPolicy: 'no-referrer',
        signal: controller.signal,
      });
      const text = await response.text();
      let body: { code?: unknown } | null = null;
      if (text !== '') {
        try { body = JSON.parse(text) as { code?: unknown }; }
        catch { throw new ApiError(response.ok ? 502 : response.status, 'unavailable'); }
      }
      if (!response.ok) throw new ApiError(response.status, typeof body?.code === 'string' ? body.code : 'unavailable');
      if (response.status !== status || body === null) throw new Error('Unexpected response status');
      return body as T;
    } finally { clearTimeout(timer); }
  }

  read<T>(path: string): Promise<T> {
    return this.request(path, { headers: { Accept: 'application/json' } }, 200);
  }

  async mutate<T>(path: string, body: object = {}, status = 200): Promise<T> {
    if (!this.#csrf) await this.bootstrap();
    const json = JSON.stringify(body);
    const bytes = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(path + '\n' + json));
    const fingerprint = Array.from(new Uint8Array(bytes), byte => byte.toString(16).padStart(2, '0')).join('');
    const key = this.#keys.get(fingerprint) ?? crypto.randomUUID();
    this.#keys.set(fingerprint, key);
    const result = await this.request<T>(path, {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
        'X-CSRF-Token': this.#csrf!,
        'Idempotency-Key': key,
      },
      body: json,
    }, status);
    this.#keys.delete(fingerprint);
    return result;
  }
}

export const api = new SessionsApi();
