import { useEffect, useState } from 'preact/hooks';
import { api, errorMessage, parseRoomState, paths } from './api';
import type { LinkResult, State } from './api';
import { followLink, navigate, parseRoute, sharePath } from './navigation';
import { keepJoin } from './pageMemory';
import { Room } from './Room';
import { ServiceChecks } from './ServiceChecks';

function Retention() {
  return <p class="notice">Local data, including questions, responses and any captured transcripts, is retained until the operator explicitly deletes the dataset and backups. Room end and access revocation are not deletion. Do not use real recordings if indefinite local retention is unacceptable.</p>;
}

function Operator({ created }: { created: (state: State) => void }) {
  const [credential, setCredential] = useState('');
  const [authenticated, setAuthenticated] = useState(false);
  const [title, setTitle] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [roomId, setRoomId] = useState('');
  return <section class="panel">
    <h1>Host operator</h1>
    <p>Only the host-configured operator can create rooms. Audience links never grant speaker access.</p>
    {error && <p role="alert" class="error">{error}</p>}
    {!authenticated ? <form onSubmit={async event => {
      event.preventDefault();
      if (busy) return;
      setBusy(true);
      setError('');
      const secret = credential;
      setCredential('');
      try {
        const result = await api.mutate<{ operator?: boolean }>(paths.login, { credential: secret });
        if (result.operator !== true) throw new Error('Invalid operator response');
        setAuthenticated(true);
      } catch (reason) {
        setError(errorMessage(reason) + ' Re-enter the credential to retry.');
      } finally { setBusy(false); }
    }}>
      <label>Operator credential
        <input type="password" autoComplete="off" required value={credential} onInput={event => setCredential(event.currentTarget.value)} />
      </label>
      <p>The credential is sent only to this origin’s sessions service and is not saved in this browser.</p>
      <button disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</button>
    </form> : <>
      <p role="status">Operator authenticated for this session.</p>
      <Retention />
      <form onSubmit={async event => {
        event.preventDefault();
        const trimmed = title.trim();
        if (busy || trimmed === '' || [...trimmed].length > 200) return;
        setBusy(true);
        setError('');
        try {
          const result = await api.mutate<{ state?: unknown } & LinkResult>(paths.rooms, { title: trimmed }, 201);
          const state = parseRoomState(result.state);
          if (!state || state.membership.role !== 'speaker') throw new Error('Invalid room');
          const link = sharePath('join', result.join_url);
          keepJoin(state.room.id, link);
          created(state);
        } catch (reason) {
          setError(errorMessage(reason));
        } finally { setBusy(false); }
      }}>
        <label>Room title
          <input required maxLength={200} value={title} onInput={event => setTitle(event.currentTarget.value)} />
        </label>
        <button disabled={busy || title.trim() === ''}>{busy ? 'Creating…' : 'Create room'}</button>
      </form>
      <h2>Return to an existing room</h2>
      <p>Use its saved room URL, or enter its room ID. Signing in restores the operator’s existing speaker memberships.</p>
      <form onSubmit={event => { event.preventDefault(); navigate('/rooms/' + roomId.trim()); }}>
        <label>Room ID
          <input required pattern="[a-fA-F0-9-]{36}" value={roomId} onInput={event => setRoomId(event.currentTarget.value)} />
        </label>
        <button>Open room</button>
      </form>
    </>}
  </section>;
}

function Landing({ kind, token }: { kind: 'join' | 'invite'; token: string }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  return <section class="panel">
    <h1>{kind === 'join' ? 'Join the audience' : 'Accept TA invitation'}</h1>
    <p>{kind === 'join' ? 'This link grants audience membership only. Existing speaker or TA membership is preserved.' : 'This single-use invitation grants TA access, not speaker or operator authority.'}</p>
    <Retention />
    {error && <p role="alert" class="error">{error}</p>}
    <form onSubmit={async event => {
      event.preventDefault();
      if (busy) return;
      setBusy(true);
      setError('');
      try {
        const result = await api.mutate<{ state?: unknown }>(kind === 'join' ? paths.join(token) : paths.redeem(token));
        const state = parseRoomState(result.state);
        if (!state) throw new Error('Invalid room');
        navigate('/rooms/' + state.room.id, true);
      } catch (reason) {
        setError(errorMessage(reason));
      } finally { setBusy(false); }
    }}>
      <button disabled={busy}>{busy ? 'Joining…' : kind === 'join' ? 'Join room' : 'Accept invitation'}</button>
    </form>
    <p>Invalid, expired or revoked link? Ask the speaker for a fresh link.</p>
  </section>;
}

export function App() {
  const [path, setPath] = useState(window.location.pathname);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState('');
  const route = parseRoute(path);
  async function connect() {
    setBusy(true);
    setError('');
    try {
      await api.bootstrap();
      setReady(true);
    } catch (reason) {
      setReady(false);
      setError(errorMessage(reason));
    } finally { setBusy(false); }
  }
  useEffect(() => {
    const update = () => setPath(window.location.pathname);
    window.addEventListener('popstate', update);
    void connect();
    return () => window.removeEventListener('popstate', update);
  }, []);
  useEffect(() => {
    const title = route.kind === 'room' ? 'Room'
      : route.kind === 'operator' ? 'Host operator'
        : route.kind === 'health' ? 'Service checks'
          : route.kind === 'join' ? 'Join room'
            : route.kind === 'invite' ? 'Invitation'
              : route.kind === 'missing' ? 'Not found'
                : 'Rooms';
    document.title = 'SansCue · ' + title;
  }, [path]);
  return <div class="shell">
    <a class="skip-link" href="#main">Skip to content</a>
    <header class="site-header">
      <a class="brand" href="/" onClick={event => followLink(event, '/')}>
        <span class="brand-mark" aria-hidden="true">s.</span>SansCue
      </a>
      <span class="environment">Rooms · local MVP</span>
    </header>
    <main id="main" tabIndex={-1}>
      {route.kind === 'health' ? <ServiceChecks /> : !ready ? <section class="panel">
        <h1>Connect to SansCue</h1>
        {busy ? <p role="status">Preparing your secure browser session…</p> : <>
          <p role="alert" class="error">{error}</p>
          <button type="button" onClick={() => void connect()}>Retry connection</button>
        </>}
      </section> : route.kind === 'room' ? <Room key={route.id} id={route.id} tab={route.tab} /> : route.kind === 'operator' ? <Operator created={state => navigate('/rooms/' + state.room.id + '/access')} /> : route.kind === 'join' || route.kind === 'invite' ? <Landing key={path} kind={route.kind} token={route.token} /> : route.kind === 'missing' ? <section class="panel">
        <h1>Page not found</h1>
        <p>This is not a valid room or invitation route.</p>
        <a href="/" onClick={event => followLink(event, '/')}>Return home</a>
      </section> : <section class="intro">
        <p class="eyebrow">Rooms and access</p>
        <h1>A shared room.<br /><span>Your own perspective.</span></h1>
        <p class="intro-copy">Join using the speaker’s QR code or audience link. Invited TAs use their private invitation.</p>
        <p><a href="/operator" onClick={event => followLink(event, '/operator')}>Host operator sign-in and room creation</a></p>
        <p class="scope-note">Questions, responses, live updates and Bee integration are coming in later stages.</p>
      </section>}
    </main>
    <footer class="site-footer">
      <a href="/health" onClick={event => followLink(event, '/health')}>Service checks</a>
      <button type="button" disabled={busy} onClick={() => { setReady(false); void connect(); }}>Reconnect browser session</button>
      <span>Same-origin sessions · No accounts</span>
    </footer>
  </div>;
}
