import { useEffect, useRef, useState } from 'preact/hooks';
import QRCode from 'qrcode';
import { ApiError, api, deniesRoomRead, errorMessage, parseRoomState, paths } from './api';
import type { State, Invitation, Membership, Page, LinkResult } from './api';
import { followLink, selectedSection, sharePath, tabLabel } from './navigation';
import { draftKey, keepDraft, keepInvite, keepJoin, memoryDraft, memoryInvite, memoryJoin } from './pageMemory';

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
  const label = kind === 'join' ? 'Audience join URL' : 'Private TA invitation URL';
  return <section class="share-link" aria-label={kind === 'join' ? 'Audience join link' : 'Private TA invitation'}>
    {qr && <img src={qr} width="280" height="280" alt={kind === 'join' ? 'QR code for the audience join link below' : 'QR code for the private TA invitation below'} />}
    {failed && <p role="status">QR could not be generated. Use the text link instead.</p>}
    <label>{label}<input readOnly value={url} onFocus={event => event.currentTarget.select()} /></label>
    <a href={safe} target="_blank" rel="noreferrer noopener">Open {kind === 'join' ? 'audience link' : 'TA invitation'}</a>
    <p>Copy this link now. It is kept only in this page’s memory.{kind === 'invite' && ' Share only with your intended TA; this invitation is single-use.'}</p>
  </section>;
}

type IssuedInvite = { id: string; expires: string; link: string | null };

function joinStatus(state: State): string {
  if (state.room.status === 'ended') return 'This room has ended; links no longer admit new members.';
  if (state.join_link_enabled === true) return 'The audience link is enabled.';
  if (state.join_link_enabled === false) return 'New audience joins are disabled.';
  return 'Audience link status is unavailable. Refresh room state.';
}

function Access({ state, refresh, loseAccess }: { state: State; refresh: () => Promise<void>; loseAccess: () => void }) {
  const roomId = state.room.id;
  const ended = state.room.status === 'ended';
  const [invitations, setInvitations] = useState<Page<Invitation>>();
  const [members, setMembers] = useState<Page<Membership>>();
  const [joinLink, setJoinLink] = useState<string | null>(memoryJoin(roomId));
  const [issued, setIssued] = useState<IssuedInvite | undefined>(memoryInvite(roomId));
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
    <h3>TA invitations</h3>
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
    })}>Create TA invitation</button>
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
        <div><strong>{item.label}</strong><p>{item.role} · {item.status} · Last activity: {item.last_seen_at ? when(item.last_seen_at) : 'Not recorded'}</p></div>
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
  const [draft, setDraft] = useState('');
  const seenRevision = useRef(-1);
  const draftOwner = useRef('');

  function show(next: State) {
    if (next.revision < seenRevision.current) return;
    seenRevision.current = next.revision;
    const owner = draftKey(id, next.membership.id, next.membership.role);
    if (draftOwner.current !== owner) {
      draftOwner.current = owner;
      setDraft(memoryDraft(owner));
    }
    setState(next);
  }

  async function refresh() {
    setBusy(true);
    setError('');
    try {
      const body = await api.read<unknown>(paths.state(id));
      const next = parseRoomState(body, id);
      if (!next) throw new Error('Invalid state');
      show(next);
    } catch (reason) {
      setError(errorMessage(reason));
      if (deniesRoomRead(reason)) setState(undefined);
    } finally { setBusy(false); }
  }

  useEffect(() => { void refresh(); }, [id]);

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
  const owner = draftKey(id, state.membership.id, state.membership.role);
  return <section class="room">
    <p class="eyebrow">{state.membership.role} · {state.room.status}</p>
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
    <p class="service-note">State is a snapshot. Live updates are not implemented in this stage; refresh to see changes.</p>
    <nav class="room-tabs" aria-label="Room sections">{tabs.map(name => <a key={name} href={'/rooms/' + id + '/' + name} aria-current={selected === name ? 'page' : undefined} onClick={event => followLink(event, '/rooms/' + id + '/' + name)}>{tabLabel(name)}</a>)}</nav>
    {redirected && <p role="status">That section is unavailable for your role. Showing {tabLabel(selected)}.</p>}
    <div class="panel">
      {selected === 'access' && state.membership.role === 'speaker'
        ? <Access state={state} refresh={refresh} loseAccess={() => { setError('Room access changed. Refresh or sign in again.'); setState(undefined); }} />
        : <>
          <h2>{tabLabel(selected)}</h2>
          {selected === 'qa' ? state.membership.role === 'audience' ? <>
            <p>Written Q&A is private to you and the speaker/TAs. Submission is not implemented yet.</p>
            <label>Your unsent question draft
              <textarea rows={5} maxLength={4000} value={draft} readOnly={ended} aria-describedby="draft-help" onInput={event => {
                const value = event.currentTarget.value;
                setDraft(value);
                keepDraft(owner, value);
              }} />
            </label>
            <p id="draft-help">{ended ? 'This room is read-only.' : 'Draft only — not sent or saved to the server.'} Your draft survives section changes in this page, but not a page reload.</p>
          </> : <p>Private Q&A review is not implemented in this stage. No participant questions are loaded here.</p> : <>
            <p>{ended ? 'The room has ended. There is no active question.' : state.active_question ? 'A published question exists. Question display and participation arrive in a later stage.' : 'No active question. Waiting for the speaker to publish one.'}</p>
            <p>{selected === 'dashboard' ? 'Response metrics are not implemented in this stage.' : selected === 'questions' ? 'Generation, review and publication are not implemented in this stage.' : 'Response submission is not implemented in this stage.'}</p>
          </>}
        </>}
    </div>
  </section>;
}
