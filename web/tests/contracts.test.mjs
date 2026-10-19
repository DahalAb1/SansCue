import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { createServer } from 'vite';

const server = await createServer({ server: { middlewareMode: true }, appType: 'custom', logLevel: 'silent' });
const apiModule = await server.ssrLoadModule('/src/app/api.ts');
const navigation = await server.ssrLoadModule('/src/app/navigation.ts');
const memory = await server.ssrLoadModule('/src/app/pageMemory.ts');
const live = await server.ssrLoadModule('/src/app/live.ts');
const feedback = await server.ssrLoadModule('/src/app/feedback.ts');
const connection = await server.ssrLoadModule('/src/app/connection.ts');
await server.close();

const { SessionsApi, ApiError, errorMessage, paths, parseRoomState, preferState, deniesRoomRead, parseTranscript, generatorLabel } = apiModule;
const { parseRoute, tabsFor, selectedSection, sharePath } = navigation;
const { keepJoin, memoryJoin, keepInvite, memoryInvite, keepDraft, memoryDraft, draftKey, keepPublishedQuestionDraft, memoryPublishedQuestionDraft, clearPublishedQuestionDraft } = memory;
const { parseRoomEvent, reconnectDelay } = live;
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const roomId = '00000000-0000-4000-8000-000000000001';
const memberId = '00000000-0000-4000-8000-000000000002';
const token = 'ab'.repeat(32);

function files(dir) {
  return readdirSync(dir).flatMap(name => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

function state(role = 'audience') {
  return {
    room: { id: roomId, title: 'Talk', status: 'open' },
    membership: { id: memberId, role },
    revision: 3,
    sequence: 3,
    active_question: null,
    my_response: null,
    dashboard: null,
    bee: role === 'audience' ? null : { binding_id: null, status: 'unbound', has_gaps: false, connectivity_revision: 0 },
    ...(role === 'speaker' ? { join_link_enabled: true } : {}),
  };
}

test('frontend routes accept opaque path-segment tokens and role sections', () => {
  assert.deepEqual(parseRoute('/rooms/' + roomId + '/qa'), { kind: 'room', id: roomId, tab: 'qa' });
  assert.deepEqual(parseRoute('/join/' + token), { kind: 'join', token });
  assert.deepEqual(parseRoute('/invite/' + token + '/'), { kind: 'invite', token });
  const opaque = 'Zm9v-_.~YmFy+Z=:@!$&\'()*,;';
  const encoded = encodeURIComponent(opaque);
  assert.deepEqual(parseRoute('/join/' + opaque), { kind: 'join', token: opaque });
  assert.deepEqual(parseRoute('/invite/' + encoded), { kind: 'invite', token: opaque });
  assert.deepEqual(parseRoute('/join/' + token.toUpperCase()), { kind: 'join', token: token.toUpperCase() });
  assert.deepEqual(parseRoute('/join/a%25b%2Bc'), { kind: 'join', token: 'a%b+c' });
  const longToken = 'Ab0-_.~'.repeat(2000);
  assert.deepEqual(parseRoute('/join/' + longToken), { kind: 'join', token: longToken });
  assert.deepEqual(parseRoute('/invite/' + encodeURIComponent(longToken + '+=')), { kind: 'invite', token: longToken + '+=' });
  assert.equal(parseRoute('/join/' + longToken + '%2F').kind, 'missing');
  assert.equal(parseRoute('/join/' + longToken + '%00').kind, 'missing');
  assert.equal(parseRoute('/join/' + longToken + '%E0%A4%A').kind, 'missing');
  for (const bad of ['%2e', '%2e%2e', '%2F', 'a%2Fb', 'a%5Cb', 'a%00b', 'a%0Ab', 'a%20b', '%E0%A4%A', '%', 'a b', 'a\\b', 'caf\u00e9']) {
    assert.equal(parseRoute('/join/' + bad).kind, 'missing', bad);
    assert.equal(parseRoute('/invite/' + bad).kind, 'missing', bad);
  }
  assert.equal(parseRoute('/join/' + token + '/extra').kind, 'missing');
  assert.equal(parseRoute('/api/sessions/rooms').kind, 'missing');
  assert.equal(parseRoute('/join/').kind, 'missing');
  assert.equal(parseRoute('/join/%2e%2e').kind, 'missing');
  assert.deepEqual(tabsFor('audience'), ['response', 'qa']);
  assert.deepEqual(tabsFor('ta'), ['dashboard', 'questions', 'transcript', 'qa']);
  assert.deepEqual(tabsFor('speaker'), ['dashboard', 'questions', 'transcript', 'qa', 'access']);
  assert.equal(selectedSection('audience', 'access').selected, 'response');
  assert.equal(selectedSection('audience', 'access').redirected, true);
  assert.equal(selectedSection('ta', 'access').selected, 'dashboard');
  assert.equal(selectedSection('speaker', 'access').selected, 'access');
  assert.equal(selectedSection('speaker').selected, 'dashboard');
});

test('question candidates are staff-only development previews with traceable evidence', () => {
  const candidate = {
    candidate_id: memberId,
    event_id: memberId,
    text: 'What is the key implication?',
    generator_version: 'stub-v1',
    published: false,
    created_at: '2026-10-09T00:00:00Z',
    evidence: {
      conversation_id: roomId,
      ingest_ordinal: 1,
      received_at: '2026-10-09T00:00:00Z',
      excerpt: 'A transcript excerpt',
    },
  };
  assert.deepEqual(parseRoomState({ ...state('speaker'), question_candidates: [candidate] }, roomId)?.question_candidates, [candidate]);
  const modelCandidate = { ...candidate, generator_version: 'provider:model@prompt-v1' };
  assert.deepEqual(parseRoomState({ ...state('speaker'), question_candidates: [modelCandidate] }, roomId)?.question_candidates, [modelCandidate]);
  assert.equal(parseRoomState({ ...state('speaker'), question_candidates: [{ ...candidate, generator_version: 'bad\nversion' }] }, roomId), null);
  assert.equal(parseRoomState({ ...state('audience'), question_candidates: [candidate] }, roomId), null);
  assert.equal(parseRoomState({ ...state('ta'), question_candidates: [{ ...candidate, published: true }] }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), question_candidates: [{ ...candidate, evidence: { ...candidate.evidence, ingest_ordinal: 0 } }] }, roomId), null);
});

test('share links are same-origin path tokens and nothing else', () => {
  const join = '/join/' + token;
  const invite = '/invite/' + token;
  assert.equal(sharePath('join', join), join);
  assert.equal(sharePath('invite', invite), invite);
  for (const value of ['https://evil.example' + join, '//' + 'evil.example' + join, join + '?x=1', join + '#x', join + '/', join + '/extra', '/invite/' + token, 'javascript:alert(1)', '/join/', '/join/%2e%2e', '/join/a%2Fb', '/join/a%00b', '/join/%zz', '/join/a b', ' ' + join, join + '\n', null, undefined]) {
    assert.equal(sharePath('join', value), null, String(value));
  }
  const opaque = 'Zm9v-_.~YmFy+Z=:@!$&\'()*,;';
  assert.equal(sharePath('join', '/join/' + opaque), '/join/' + opaque);
  assert.equal(sharePath('invite', '/invite/' + encodeURIComponent(opaque)), '/invite/' + encodeURIComponent(opaque));
  assert.equal(sharePath('join', '/join/' + token.toUpperCase()), '/join/' + token.toUpperCase());
  const longToken = 'Ab0-_.~'.repeat(2000);
  assert.equal(sharePath('join', '/join/' + longToken), '/join/' + longToken);
  assert.equal(sharePath('invite', '/invite/' + longToken), '/invite/' + longToken);
  assert.equal(sharePath('join', '/join/' + longToken + '%2F'), null);
  assert.equal(sharePath('invite', join), null);
});

test('room feature paths match the sessions publication, feedback and written Q&A contract', () => {
  assert.equal(paths.bootstrap, '/bootstrap');
  assert.equal(paths.login, '/operator/login');
  assert.equal(paths.rooms, '/rooms');
  assert.equal(paths.state(roomId), '/rooms/' + roomId + '/state');
  assert.equal(paths.events(roomId, 17), '/rooms/' + roomId + '/events?after=17');
  assert.equal(paths.join(token), '/join/' + token);
  assert.equal(paths.redeem(token), '/invitations/' + token + '/redeem');
  assert.equal(paths.invitations(roomId), '/rooms/' + roomId + '/invitations');
  assert.equal(paths.memberships(roomId), '/rooms/' + roomId + '/memberships');
  assert.equal(paths.revokeInvitation(roomId, memberId), '/rooms/' + roomId + '/invitations/' + memberId + '/revoke');
  assert.equal(paths.revokeMembership(roomId, memberId), '/rooms/' + roomId + '/memberships/' + memberId + '/revoke');
  assert.equal(paths.rotateJoin(roomId), '/rooms/' + roomId + '/join-link/rotate');
  assert.equal(paths.revokeJoin(roomId), '/rooms/' + roomId + '/join-link/revoke');
  assert.equal(paths.end(roomId), '/rooms/' + roomId + '/end');
  assert.equal(paths.bindBee(roomId), '/rooms/' + roomId + '/bee/bind');
  assert.equal(paths.unbindBee(roomId), '/rooms/' + roomId + '/bee/unbind');
  assert.equal(paths.qa(roomId), '/rooms/' + roomId + '/qa');
  assert.equal(paths.publishQuestion(roomId, memberId), '/rooms/' + roomId + '/questions/' + memberId + '/publish');
  assert.equal(paths.questionVersion(roomId, memberId), '/rooms/' + roomId + '/questions/' + memberId + '/versions');
  assert.equal(paths.respond(roomId, memberId), '/rooms/' + roomId + '/questions/' + memberId + '/response');
  assert.equal(paths.page(paths.invitations(roomId)), paths.invitations(roomId) + '?limit=50');
  assert.equal(paths.page(paths.memberships(roomId), memberId), paths.memberships(roomId) + '?limit=50&after=' + memberId);
  const opaqueCursor = 'eyJ0IjoiMjAyNi0xMC0wOCIsImlkIjoiYSJ9.sig/+=?&# %';
  assert.equal(paths.page(paths.invitations(roomId), opaqueCursor), paths.invitations(roomId) + '?limit=50&after=' + encodeURIComponent(opaqueCursor));
  assert.equal(new URL('http://x' + paths.page(paths.memberships(roomId), opaqueCursor)).searchParams.get('after'), opaqueCursor);
  assert.equal(paths.page(paths.invitations(roomId), '../secrets'), paths.invitations(roomId) + '?limit=50&after=..%2Fsecrets');
  assert.equal(paths.page(paths.invitations(roomId), 'a&limit=1'), paths.invitations(roomId) + '?limit=50&after=a%26limit%3D1');
  assert.equal(paths.page(paths.invitations(roomId), null), paths.invitations(roomId) + '?limit=50');
  assert.equal(paths.page(paths.invitations(roomId), '\ud800'), null);
  const longCursor = 'a/+=?&# %'.repeat(5000);
  assert.equal(paths.page(paths.invitations(roomId), longCursor), paths.invitations(roomId) + '?limit=50&after=' + encodeURIComponent(longCursor));
  assert.equal(new URL('http://x' + paths.page(paths.memberships(roomId), longCursor)).searchParams.get('after'), longCursor);
  assert.equal(paths.page(paths.invitations(roomId), 'a'.repeat(5000) + '\ud800'), null);
  const listed = [
    paths.bootstrap, paths.login, paths.rooms, paths.state(roomId), paths.join(token), paths.redeem(token),
    paths.invitations(roomId), paths.memberships(roomId), paths.revokeInvitation(roomId, memberId),
    paths.revokeMembership(roomId, memberId), paths.rotateJoin(roomId), paths.revokeJoin(roomId), paths.end(roomId),
    paths.bindBee(roomId), paths.unbindBee(roomId),
    paths.qa(roomId), paths.publishQuestion(roomId, memberId), paths.questionVersion(roomId, memberId), paths.respond(roomId, memberId),
  ].join('\n');
  assert.match(listed, /\/questions/);
  assert.match(listed, /\/qa/);
  assert.match(listed, /\/response/);
});

test('published question state supports public text while role-specific data stays partitioned', () => {
  const active = { id: memberId, version: 2, text: 'Published text' };
  assert.deepEqual(parseRoomState({ ...state('audience'), active_question: active, my_response: 'clear' }, roomId)?.active_question, active);
  assert.equal(parseRoomState({ ...state('audience'), active_question: { ...active, evidence: { excerpt: 'secret' } }, question_candidates: [] }, roomId), null);
  assert.equal(parseRoomState({ ...state('audience'), my_response: 'other' }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), active_question: { ...active, evidence: { excerpt: 'private source copy' } }, dashboard: { respondents: 1, counts: { clear: 1, partly_clear: 0, need_help: 0 }, percentages: { clear: 100, partly_clear: 0, need_help: 0 } } }, roomId)?.dashboard.respondents, 1);
});

test('live events are scoped and replayed strictly after the accepted room sequence', () => {
  const event = { type: 'room.changed', room_id: roomId, schema_version: 1, sequence: 4 };
  assert.deepEqual(parseRoomEvent(event, roomId, 3), event);
  assert.equal(parseRoomEvent(event, roomId, 4), undefined);
  assert.equal(parseRoomEvent({ ...event, room_id: memberId }, roomId, 3), undefined);
  assert.equal(parseRoomEvent({ ...event, type: 'room.other' }, roomId, 3), undefined);
  assert.equal(parseRoomEvent({ ...event, schema_version: 2 }, roomId, 3), undefined);
  assert.equal(parseRoomEvent({ ...event, sequence: Number.MAX_SAFE_INTEGER + 1 }, roomId, 3), undefined);
});

test('live reconnect backoff grows and is capped', () => {
  assert.deepEqual([0, 1, 2, 3, 4, 5, 9].map(reconnectDelay), [1000, 2000, 4000, 8000, 16000, 30000, 30000]);
});

test('room state is accepted only for the requested room and a server role', () => {
  assert.equal(parseRoomState(state('speaker'), roomId).membership.role, 'speaker');
  assert.equal(parseRoomState(state(), roomId.toUpperCase()).room.id, roomId);
  assert.equal(parseRoomState({ ...state(), membership: { id: memberId, role: 'operator' } }, roomId), null);
  assert.equal(parseRoomState(state(), '00000000-0000-4000-8000-000000000099'), null);
  assert.equal(parseRoomState({ ...state(), revision: 1.5 }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), bee: null }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), bee: { binding_id: null, status: 'other', has_gaps: false, connectivity_revision: 0 } }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), bee: { binding_id: 'not-a-uuid', status: 'bound', has_gaps: false, connectivity_revision: 1 } }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), bee: { binding_id: '00000000-0000-0000-0000-000000000000', status: 'bound', has_gaps: false, connectivity_revision: 1 } }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), bee: { binding_id: memberId, status: 'bound', has_gaps: false, connectivity_revision: 1 } }, roomId)?.bee?.status, 'bound');
  assert.equal(parseRoomState({ ...state('audience'), bee: state('speaker').bee }, roomId), null);
  const older = state();
  older.revision = 2;
  const newer = state();
  newer.revision = 4;
  newer.room = { ...newer.room, title: 'Newer' };
  assert.equal(preferState(newer, older).room.title, 'Newer');
  assert.equal(preferState(older, newer).room.title, 'Newer');
  assert.equal(deniesRoomRead(new ApiError(404, 'not_found')), true);
  assert.equal(deniesRoomRead(new ApiError(403, 'forbidden')), false);
});

test('mutations bootstrap and send exact same-origin security headers and body', async () => {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return json(String(url).endsWith('/bootstrap') ? { csrf_token: 'test-csrf' } : { operator: true });
  };
  const api = new SessionsApi();
  await api.mutate(paths.login, { credential: 'synthetic-secret' });
  assert.equal(JSON.stringify(api).includes('synthetic-secret'), false);
  assert.equal(JSON.stringify(api).includes('test-csrf'), false);
  assert.equal(calls.length, 2);
  assert.equal(calls[0].url, '/api/sessions/bootstrap');
  assert.equal(calls[0].init.credentials, 'same-origin');
  assert.equal(calls[0].init.cache, 'no-store');
  assert.equal(calls[0].init.referrerPolicy, 'no-referrer');
  const { url, init } = calls[1];
  assert.equal(url, '/api/sessions/operator/login');
  assert.equal(init.method, 'POST');
  assert.equal(init.credentials, 'same-origin');
  assert.equal(init.cache, 'no-store');
  assert.equal(init.referrerPolicy, 'no-referrer');
  assert.equal(init.headers['X-CSRF-Token'], 'test-csrf');
  assert.equal(init.headers['Content-Type'], 'application/json');
  assert.match(init.headers['Idempotency-Key'], /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  assert.deepEqual(JSON.parse(init.body), { credential: 'synthetic-secret' });
});

test('reads are unauthorized-free GETs and join redemption posts an empty body', async () => {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    if (String(url).endsWith('/bootstrap')) return json({ csrf_token: 'csrf' });
    if (String(url).endsWith('/state')) return json(state());
    return json({ state: state() });
  };
  const api = new SessionsApi();
  const loaded = await api.read(paths.state(roomId));
  assert.equal(parseRoomState(loaded, roomId).room.title, 'Talk');
  assert.equal(calls[0].url, '/api/sessions' + paths.state(roomId));
  assert.equal(calls[0].init.method, undefined);
  assert.equal(calls[0].init.body, undefined);
  assert.equal(calls[0].init.headers['X-CSRF-Token'], undefined);
  assert.equal(calls[0].init.headers.Accept, 'application/json');
  await api.mutate(paths.join(token));
  await api.mutate(paths.redeem(token));
  assert.equal(calls[1].url, '/api/sessions/bootstrap');
  assert.equal(calls[2].url, '/api/sessions' + paths.join(token));
  assert.equal(calls[2].init.method, 'POST');
  assert.equal(calls[2].init.body, '{}');
  assert.equal(calls[3].url, '/api/sessions' + paths.redeem(token));
  assert.equal(calls[3].init.body, '{}');
  assert.notEqual(calls[2].init.headers['Idempotency-Key'], calls[3].init.headers['Idempotency-Key']);
});

test('lost response retries keep key; changed payload and completed operations get new keys', async () => {
  const calls = [];
  let fail = true;
  globalThis.fetch = async (url, init) => {
    if (String(url).endsWith('/bootstrap')) return json({ csrf_token: 'csrf' });
    calls.push(init);
    if (fail) { fail = false; throw new TypeError('lost response'); }
    return json({ state: state('speaker'), join_url: null, link_unavailable: true }, 201);
  };
  const api = new SessionsApi();
  await assert.rejects(api.mutate(paths.rooms, { title: 'Talk' }, 201));
  await api.mutate(paths.rooms, { title: 'Other' }, 201);
  const replay = await api.mutate(paths.rooms, { title: 'Talk' }, 201);
  await api.mutate(paths.rooms, { title: 'Talk' }, 201);
  assert.notEqual(calls[0].headers['Idempotency-Key'], calls[1].headers['Idempotency-Key']);
  assert.equal(calls[0].headers['Idempotency-Key'], calls[2].headers['Idempotency-Key']);
  assert.notEqual(calls[2].headers['Idempotency-Key'], calls[3].headers['Idempotency-Key']);
  assert.equal(replay.link_unavailable, true);
  assert.equal(JSON.stringify(replay).includes(token), false);
});

test('error responses cannot echo secrets into UI and retries preserve keys', async () => {
  const secret = 'synthetic-secret';
  const keys = [];
  globalThis.fetch = async (url, init) => {
    if (String(url).endsWith('/bootstrap')) return json({ csrf_token: 'csrf' });
    keys.push(init.headers['Idempotency-Key']);
    return json({ code: 'request_in_progress', message: secret, request_id: roomId }, 409);
  };
  const api = new SessionsApi();
  for (let i = 0; i < 2; i++) {
    await assert.rejects(api.mutate(paths.join(token)), reason => reason instanceof ApiError && !errorMessage(reason).includes(secret) && !errorMessage(reason).includes(token));
  }
  assert.equal(keys[0], keys[1]);
  assert.match(errorMessage(new ApiError(404, 'not_found')), /invalid, expired or revoked/);
  assert.match(errorMessage(new ApiError(409, 'room_ended')), /ended/);
  assert.match(errorMessage(new ApiError(409, 'idempotency_conflict')), /did not match the original request/);
  assert.match(errorMessage(new ApiError(409, 'bee_already_bound')), /Unbind the current/);
  assert.match(errorMessage(new ApiError(409, 'bee_source_already_bound')), /another room/);
  assert.equal(errorMessage(new ApiError(409, 'idempotency_conflict')).includes(secret), false);
  assert.match(errorMessage(new TypeError('http://127.0.0.1/join/' + token)), /request key is retained/);
  assert.equal(errorMessage(new TypeError('http://127.0.0.1/join/' + token)).includes(token), false);
});

test('rebootstrap refreshes CSRF without losing pending request identity', async () => {
  let csrf = 0;
  let fail = true;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    if (String(url).endsWith('/bootstrap')) return json({ csrf_token: 'csrf-' + ++csrf });
    calls.push(init);
    if (fail) { fail = false; return json({ code: 'csrf_failed' }, 403); }
    return json({ state: state('ta') });
  };
  const api = new SessionsApi();
  await assert.rejects(api.mutate(paths.redeem(token)));
  await api.bootstrap();
  await api.mutate(paths.redeem(token));
  assert.equal(calls[0].headers['Idempotency-Key'], calls[1].headers['Idempotency-Key']);
  assert.equal(calls[1].headers['X-CSRF-Token'], 'csrf-2');
  assert.equal(calls[1].body, '{}');
  assert.equal(JSON.stringify(api).includes('csrf-2'), false);
});

test('success status mismatch is not treated as a successful mutation', async () => {
  globalThis.fetch = async url => json(String(url).endsWith('/bootstrap') ? { csrf_token: 'csrf' } : {});
  await assert.rejects(new SessionsApi().mutate(paths.rooms, { title: 'Talk' }, 201));
});

test('issued links and unsent drafts stay in page memory only', () => {
  const key = draftKey(roomId, memberId, 'audience');
  keepJoin(roomId, '/join/' + token);
  assert.equal(memoryJoin(roomId), '/join/' + token);
  keepJoin(roomId, null);
  assert.equal(memoryJoin(roomId), null);
  keepInvite(roomId, { id: memberId, expires: '2026-10-09T00:00:00Z', link: null });
  assert.equal(memoryInvite(roomId).id, memberId);
  assert.equal(memoryInvite(roomId).link, null);
  keepInvite(roomId, undefined);
  assert.equal(memoryInvite(roomId), undefined);
  keepDraft(key, 'Please explain the example');
  assert.equal(memoryDraft(key), 'Please explain the example');
  assert.equal(memoryDraft(draftKey(roomId, memberId, 'ta')), '');
  keepDraft(key, '');
  assert.equal(memoryDraft(key), '');
});

test('written Q&A drafts are keyed by membership and survive section unmounts', () => {
  const key = draftKey(roomId, memberId, 'audience');
  keepDraft(key, 'Question draft before opening another tab');
  // QaPanel remounts from the same page-memory entry when its tab is selected again.
  assert.equal(memoryDraft(draftKey(roomId, memberId, 'audience')), 'Question draft before opening another tab');
  assert.equal(memoryDraft(draftKey(roomId, roomId, 'audience')), '');
  assert.equal(memoryDraft(draftKey(roomId, memberId, 'ta')), '');
  keepDraft(key, '');
});

test('speaker version drafts survive tab changes but reset when publication version changes', () => {
  const key = draftKey(roomId, memberId, 'question-publication');
  keepPublishedQuestionDraft(key, memberId, 1, 'Unsaved speaker edit');
  // Returning to Questions remounts the panel with the same room, member and active version.
  assert.equal(memoryPublishedQuestionDraft(key, memberId, 1, 'Published v1'), 'Unsaved speaker edit');
  // A new publication/version invalidates the stale unsaved draft and uses the published baseline.
  assert.equal(memoryPublishedQuestionDraft(key, memberId, 2, 'Published v2'), 'Published v2');
  keepPublishedQuestionDraft(key, memberId, 2, 'Published v2');
  assert.equal(memoryPublishedQuestionDraft(key, memberId, 2, 'fallback'), 'Published v2');
  assert.equal(memoryPublishedQuestionDraft(draftKey(roomId, roomId, 'question-publication'), memberId, 2, 'Other member'), 'Other member');
  clearPublishedQuestionDraft(key);
  assert.equal(memoryPublishedQuestionDraft(key, memberId, 2, 'Published v2'), 'Published v2');
});

test('client source does not persist secrets or log them', () => {
  const banned = /localStorage|sessionStorage|document\.cookie|console\.(log|debug|info|warn|error)/;
  const sources = files('src').filter(path => path.endsWith('.ts') || path.endsWith('.tsx'));
  assert.ok(sources.length >= 6);
  for (const path of sources) assert.doesNotMatch(readFileSync(path, 'utf8'), banned, path);
});

test('speaker chart rows use respondent shares and stay empty without responses', () => {
  const { chartRows, formatPercent, respondentSummary } = feedback;
  const empty = { respondents: 0, counts: { clear: 0, partly_clear: 0, need_help: 0 }, percentages: { clear: null, partly_clear: null, need_help: null } };
  assert.deepEqual(chartRows(empty).map(row => [row.value, row.count, row.percent, row.width]), [
    ['clear', 0, null, 0], ['partly_clear', 0, null, 0], ['need_help', 0, null, 0],
  ]);
  assert.equal(respondentSummary(empty), 'No responses yet');
  // A malformed 100% with no respondents must not render as "everyone is clear".
  assert.equal(chartRows({ ...empty, percentages: { ...empty.percentages, clear: 100 } })[0].width, 0);
  const mixed = { respondents: 3, counts: { clear: 2, partly_clear: 0, need_help: 1 }, percentages: { clear: 200 / 3, partly_clear: 0, need_help: 100 / 3 } };
  const rows = chartRows(mixed);
  assert.deepEqual(rows.map(row => row.label), ['Clear', 'Partly clear', 'Need help']);
  assert.deepEqual(rows.map(row => row.count), [2, 0, 1]);
  assert.deepEqual(rows.map(row => formatPercent(row.percent)), ['66.7%', '0.0%', '33.3%']);
  assert.equal(rows[1].width, 0);
  assert.equal(respondentSummary(mixed), '3 audience responses');
  assert.equal(respondentSummary({ ...mixed, respondents: 1 }), '1 audience response');
  assert.equal(chartRows({ ...mixed, percentages: { ...mixed.percentages, clear: 140 } })[0].width, 100);
});

test('moderator review paths, draft memory and wording notes', () => {
  const { draftExpiry, earlierWordingNote } = feedback;
  assert.equal(paths.saveDraft(roomId, memberId), '/rooms/' + roomId + '/questions/' + memberId + '/draft');
  assert.equal(paths.dismissDraft(roomId, memberId), '/rooms/' + roomId + '/questions/' + memberId + '/dismiss');
  assert.equal(paths.reply(roomId, memberId), '/rooms/' + roomId + '/qa/' + memberId + '/replies');
  assert.equal(navigation.roleLabel('ta'), 'Moderator (TA)');
  const { keepReviewDraft, memoryReviewDraft } = memory;
  const key = draftKey(roomId, memberId, 'review-draft\n' + memberId);
  keepReviewDraft(key, 'How clear is X?', 'How clear is X now?');
  assert.equal(memoryReviewDraft(key, 'How clear is X?'), 'How clear is X now?');
  // Another moderator saved new wording: the stale local edit is not resurrected.
  assert.equal(memoryReviewDraft(key, 'Saved elsewhere'), 'Saved elsewhere');
  keepReviewDraft(key, 'How clear is X?', 'How clear is X?');
  assert.equal(memoryReviewDraft(key, 'How clear is X?'), 'How clear is X?');
  const now = Date.parse('2026-10-19T12:00:00Z');
  assert.deepEqual(draftExpiry('2026-10-19T12:04:01Z', now), { expired: false, label: 'Expires in 5 min' });
  assert.deepEqual(draftExpiry('2026-10-19T11:59:59Z', now), { expired: true, label: 'Expired' });
  assert.deepEqual(draftExpiry(undefined, now), { expired: false, label: '' });
  const dashboard = { respondents: 4, counts: { clear: 4, partly_clear: 0, need_help: 0 }, percentages: { clear: 100, partly_clear: 0, need_help: 0 }, earlier_version_respondents: 3 };
  assert.equal(earlierWordingNote(dashboard, 2), '3 of 4 responses were given before the wording changed to version 2.');
  assert.equal(earlierWordingNote({ ...dashboard, earlier_version_respondents: 0 }, 2), null);
  assert.equal(earlierWordingNote(dashboard, 1), null);
  assert.match(errorMessage(new ApiError(410, 'draft_expired')), /expired/);
  assert.match(errorMessage(new ApiError(409, 'draft_dismissed')), /dismissed/);
});

test('review queue state accepts moderator fields and rejects malformed ones', () => {
  const candidate = {
    candidate_id: memberId, event_id: memberId, text: 'How clear is the edited idea?', original_text: 'How clear is the idea?',
    edited: true, edited_at: '2026-10-19T12:00:00Z', generator_version: 'stub-v1', published: false,
    created_at: '2026-10-19T11:59:00Z', expires_at: '2026-10-19T12:10:00Z',
    evidence: { conversation_id: roomId, ingest_ordinal: 1, received_at: '2026-10-19T11:59:00Z', excerpt: 'the idea' },
  };
  assert.equal(parseRoomState({ ...state('ta'), question_candidates: [candidate] }, roomId)?.question_candidates[0].edited, true);
  assert.equal(parseRoomState({ ...state('ta'), question_candidates: [{ ...candidate, edited: 'yes' }] }, roomId), null);
  assert.equal(parseRoomState({ ...state('ta'), question_candidates: [{ ...candidate, expires_at: 'later' }] }, roomId), null);
  const dashboard = { respondents: 1, counts: { clear: 1, partly_clear: 0, need_help: 0 }, percentages: { clear: 100, partly_clear: 0, need_help: 0 } };
  assert.equal(parseRoomState({ ...state('speaker'), dashboard: { ...dashboard, earlier_version_respondents: -1 } }, roomId), null);
  assert.equal(parseRoomState({ ...state('speaker'), dashboard: { ...dashboard, earlier_version_respondents: 1 } }, roomId)?.dashboard.earlier_version_respondents, 1);
  assert.equal(parseRoomState({ ...state('audience'), my_response_version: 0 }, roomId), null);
  assert.equal(parseRoomState({ ...state('audience'), my_response_version: 2 }, roomId)?.my_response_version, 2);
});

test('private Q&A threads parse replies attributed to the moderator', () => {
  const { parseWrittenQuestions } = apiModule;
  const thread = { message_id: memberId, question_id: null, body: 'Can you repeat that?', created_at: '2026-10-19T12:00:00Z', replies: [{ reply_id: roomId, body: 'Yes, next slide.', created_at: '2026-10-19T12:01:00Z', from: 'moderator' }] };
  assert.deepEqual(parseWrittenQuestions({ items: [thread] }), [thread]);
  assert.deepEqual(parseWrittenQuestions({ items: [{ ...thread, author_label: 'Participant 2' }] })[0].author_label, 'Participant 2');
  assert.deepEqual(parseWrittenQuestions({ items: [{ ...thread, replies: undefined }] })[0].replies, []);
  assert.equal(parseWrittenQuestions({ items: [{ ...thread, replies: [{ ...thread.replies[0], from: 'speaker' }] }] }), null);
  assert.equal(parseWrittenQuestions({ items: [{ ...thread, message_id: 'x' }] }), null);
  assert.equal(parseWrittenQuestions({}), null);
});

test('Bee connection states never treat a running stream as proof of delivery', () => {
  const { connectionStatus, parseBeeConnection, ago } = connection;
  assert.equal(paths.beeConnection(roomId), '/rooms/' + roomId + '/bee-connection');
  const now = Date.parse('2026-10-19T12:00:00Z');
  const bound = { binding_id: memberId, status: 'bound', has_gaps: false, connectivity_revision: 2, source_conversation_id: roomId };
  const line = at => ({ text: 'SansCue connection check.', spoken_at: at });
  assert.equal(connectionStatus({ ...bound, status: 'unbound' }, 'connected', undefined, now).label, 'No recording connected');
  assert.equal(connectionStatus({ ...bound, status: 'binding' }, 'connected', undefined, now).tone, 'waiting');
  assert.equal(connectionStatus(bound, 'not_running', line('2026-10-19T11:59:59Z'), now).label, 'Bee listener not running');
  assert.equal(connectionStatus(bound, 'reconnecting', null, now).tone, 'warning');
  assert.equal(connectionStatus(bound, 'connected', null, now).label, 'Connected — waiting for speech');
  assert.deepEqual(connectionStatus(bound, 'connected', line('2026-10-19T11:59:50Z'), now), { tone: 'live', label: 'Receiving transcript', detail: 'Last speech 10 s ago.' });
  assert.equal(connectionStatus(bound, 'connected', line('2026-10-19T11:55:00Z'), now).label, 'Connected — no recent speech');
  assert.equal(ago('2026-10-19T11:57:00Z', now), '3 min ago');
  const report = { transport: { state: 'connected', reported_at: '2026-10-19T12:00:00Z' }, recordings: [{ source_conversation_id: roomId, first_seen_at: '2026-10-19T11:58:00Z', last_seen_at: '2026-10-19T11:59:58Z', utterances: 4, bound_here: false }] };
  assert.deepEqual(parseBeeConnection(report), report);
  assert.equal(parseBeeConnection({ ...report, transport: { state: 'listening', reported_at: null } }), null);
  assert.equal(parseBeeConnection({ ...report, recordings: [{ ...report.recordings[0], text: undefined, utterances: -1 }] }), null);
  assert.equal(parseBeeConnection({ transport: { state: 'not_running', reported_at: null }, recordings: [] }).transport.state, 'not_running');
});

test('transcript lines and generator labels are parsed defensively', () => {
  assert.equal(paths.transcript(roomId), '/rooms/' + roomId + '/transcript');
  assert.deepEqual(parseTranscript({ lines: [{ text: 'Hello.', spoken_at: '2026-10-18T20:00:00Z' }] }), [{ text: 'Hello.', spoken_at: '2026-10-18T20:00:00Z' }]);
  assert.equal(parseTranscript({ lines: [{ text: 7, spoken_at: 'x' }] }), null);
  assert.equal(parseTranscript({}), null);
  assert.equal(generatorLabel('topic-window-stub-v1'), 'Placeholder generator, not AI');
  assert.equal(generatorLabel('us.anthropic.claude-haiku-4-5-20251001-v1:0/topic-check-v1'), 'Drafted by Claude Haiku 4.5');
  assert.equal(generatorLabel('claude-opus-5-5/topic-check-v1'), 'Drafted by Claude Opus 5.5');
  assert.match(errorMessage(new ApiError(503, 'transcript_unavailable')), /retries automatically/);
});
