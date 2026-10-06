import { useEffect, useRef, useState } from 'preact/hooks';

type Check = 'healthz' | 'readyz';
type Result = { kind: 'idle' | 'checking' | 'success' | 'warning' | 'error'; label: string; detail: string };
const initial: Result = { kind: 'idle', label: 'Not checked', detail: 'Run a check to see the current service status.' };

async function checkService(check: Check, signal: AbortSignal): Promise<Result> {
  const response = await fetch('/api/sessions/' + check, { signal, cache: 'no-store', headers: { Accept: 'application/json' } });
  if (response.headers.get('content-type')?.split(';')[0].trim().toLowerCase() !== 'application/json') {
    throw new Error('Unexpected response (HTTP ' + response.status + '). Expected JSON from the sessions service.');
  }
  let body: unknown;
  try { body = await response.json(); } catch { throw new Error('The service returned invalid JSON (HTTP ' + response.status + ').'); }
  const status = typeof body === 'object' && body !== null && !Array.isArray(body) && Object.keys(body).length === 1 && 'status' in body ? body.status : undefined;
  if (check === 'healthz' && response.status === 200 && status === 'ok') {
    return { kind: 'success', label: 'Alive', detail: 'The sessions service is responding. This does not check the database.' };
  }
  if (check === 'readyz' && response.status === 200 && status === 'ready') {
    return { kind: 'success', label: 'Ready', detail: 'The sessions service can reach its database.' };
  }
  if (check === 'readyz' && response.status === 503 && status === 'not_ready') {
    return { kind: 'warning', label: 'Not ready', detail: 'The service responded, but its database check did not pass. Retry after the database recovers.' };
  }
  throw new Error('Unexpected service response (HTTP ' + response.status + '). The status did not match the agreed contract.');
}

function ServiceCheck({ check, title, description }: { check: Check; title: string; description: string }) {
  const [result, setResult] = useState<Result>(initial);
  const [checkedAt, setCheckedAt] = useState<string>();
  const pending = useRef<AbortController | null>(null);
  useEffect(() => () => { pending.current?.abort(); pending.current = null; }, []);

  async function run() {
    if (pending.current) return;
    const controller = new AbortController();
    pending.current = controller;
    setResult({ kind: 'checking', label: 'Checking…', detail: 'Waiting for the sessions service.' });
    const timeout = window.setTimeout(() => controller.abort(), 5000);
    let next: Result;
    try { next = await checkService(check, controller.signal); }
    catch (error) {
      next = { kind: 'error', label: controller.signal.aborted ? 'Timed out' : 'Check failed', detail: controller.signal.aborted ? 'No complete response within 5 seconds. Check that the local service is running, then retry.' : error instanceof TypeError ? 'Could not reach the sessions service. Check the local service and proxy, then retry.' : error instanceof Error ? error.message : 'Unable to check the service. Please retry.' };
    } finally { window.clearTimeout(timeout); }
    if (pending.current !== controller) return;
    pending.current = null;
    setResult(next);
    setCheckedAt(new Date().toLocaleTimeString());
  }

  return (
    <article class="check-card" aria-labelledby={check + '-title'}>
      <div class="check-heading"><h3 id={check + '-title'}>{title}</h3><code>/{check}</code></div>
      <p class="check-description">{description}</p>
      <div class="result" role="status" aria-live="polite" aria-atomic="true">
        <span class={'status status--' + result.kind}><span aria-hidden="true" class="status-dot" />{result.label}</span>
        <p>{result.detail}</p>
      </div>
      <div class="check-footer">
        <button type="button" disabled={result.kind === 'checking'} onClick={run} aria-label={(checkedAt ? 'Retry ' : 'Check ') + title.toLowerCase()}>{result.kind === 'checking' ? 'Checking…' : checkedAt ? 'Retry check' : 'Run check'}<span aria-hidden="true">↗</span></button>
        <span class="timestamp">{checkedAt ? 'Last completed ' + checkedAt : 'Manual check'}</span>
      </div>
    </article>
  );
}

export function App() {
  return (
    <div class="shell">
      <a class="skip-link" href="#main">Skip to content</a>
      <header class="site-header"><a class="brand" href="./" aria-label="SansCue home"><span class="brand-mark" aria-hidden="true">s.</span>SansCue</a><span class="environment">Local development</span></header>
      <main id="main" tabIndex={-1}>
        <section class="intro" aria-labelledby="page-title">
          <p class="eyebrow"><span aria-hidden="true" /> Stage 1 / Foundation</p>
          <h1 id="page-title">A starting point.<br /><span>Not a live event.</span></h1>
          <p class="intro-copy">SansCue is a conference tool in development. This starter shell provides a place to verify the local sessions service — nothing more, yet.</p>
          <div class="scope-note"><span aria-hidden="true">↳</span><p>This is a development shell, not a working conference experience.</p></div>
        </section>
        <section class="service-section" aria-labelledby="service-title">
          <div class="section-heading"><div><p class="eyebrow">Local connection</p><h2 id="service-title">Sessions service</h2></div><p>Two independent checks.<br />Run or retry each when you need it.</p></div>
          <div class="checks"><ServiceCheck check="healthz" title="Liveness" description="Is the service responding?" /><ServiceCheck check="readyz" title="Readiness" description="Can the service reach its database?" /></div>
          <p class="service-note">Checks use the same-origin <code>/api/sessions</code> proxy. Results are snapshots, not continuous monitoring.</p>
        </section>
      </main>
      <footer class="site-footer"><span>SansCue</span><span>Local foundation · No conference functionality</span></footer>
    </div>
  );
}
