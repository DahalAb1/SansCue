export type Role = 'speaker' | 'ta' | 'audience';
export type State = {
  room: { id: string; title: string; status: 'open' | 'ended' };
  membership: { id: string; role: Role };
  revision: number;
  sequence: number;
  active_question: { id: string; text?: string } | null;
  join_link_enabled?: boolean;
};
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

export function segment(value: string): string {
  return encodeURIComponent(value);
}

export const paths = {
  bootstrap: '/bootstrap',
  login: '/operator/login',
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

export function errorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) return 'Unable to reach the sessions service or read its response. Retry the same action; its request key is retained.';
  if (error.status === 401) return 'Your session has expired. Reconnect, then use a valid link or sign in as the operator again.';
  if (error.code === 'room_ended') return 'This room has ended. New joins and participation are closed.';
  if (error.status === 404) return 'Not found or access unavailable. The link may be invalid, expired or revoked.';
  if (error.code === 'invite_used') return 'This invitation has already been used or cannot change your current role. Ask the speaker for access.';
  if (error.code === 'request_in_progress') return 'This request is still in progress. Retry to recover its result.';
  if (error.code === 'idempotency_conflict') return 'That retry did not match the original request. Submit it again as a new action.';
  if (error.status === 403) return 'Access denied. Your access or security session may have changed. Reconnect or contact the speaker.';
  if (error.status === 429) return 'Too many attempts. Wait before trying again.';
  if (error.status === 409) return 'The room or request has changed. Refresh its state before continuing.';
  if (error.status === 400) return 'The request was not accepted. Check the form values.';
  return 'The sessions service is unavailable. Retry when it recovers.';
}

export function deniesRoomRead(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 401 || error.status === 404);
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
  if (state.join_link_enabled !== undefined && typeof state.join_link_enabled !== 'boolean') return null;
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
