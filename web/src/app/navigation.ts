import type { Role } from './api';

export type Route =
  | { kind: 'home' | 'operator' | 'health' | 'missing' }
  | { kind: 'join' | 'invite'; token: string }
  | { kind: 'room'; id: string; tab?: string };

const TOKEN = /^[0-9a-f]{64}$/;

export function parseRoute(path: string): Route {
  if (path === '/') return { kind: 'home' };
  if (path === '/operator') return { kind: 'operator' };
  if (path === '/health') return { kind: 'health' };
  const link = /^\/(join|invite)\/([^/]+)\/?$/.exec(path);
  if (link) {
    let token = link[2];
    try { token = decodeURIComponent(token); } catch { return { kind: 'missing' }; }
    if (!TOKEN.test(token)) return { kind: 'missing' };
    return { kind: link[1] as 'join' | 'invite', token };
  }
  const room = /^\/rooms\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})(?:\/([a-z-]+))?\/?$/i.exec(path);
  if (room) return { kind: 'room', id: room[1], tab: room[2] };
  return { kind: 'missing' };
}

export function tabsFor(role: Role): string[] {
  if (role === 'audience') return ['response', 'qa'];
  if (role === 'speaker') return ['dashboard', 'questions', 'qa', 'access'];
  return ['dashboard', 'questions', 'qa'];
}

export function selectedSection(role: Role, tab?: string): { tabs: string[]; selected: string; redirected: boolean } {
  const tabs = tabsFor(role);
  const redirected = Boolean(tab && !tabs.includes(tab));
  return { tabs, selected: redirected || !tab ? tabs[0] : tab, redirected };
}

export function tabLabel(tab: string): string {
  return ({ response: 'Response', qa: 'Q&A', dashboard: 'Dashboard', questions: 'Questions', access: 'Room access' } as Record<string, string>)[tab] ?? tab;
}

export function sharePath(kind: 'join' | 'invite', value: string | null | undefined): string | null {
  if (typeof value !== 'string' || !new RegExp('^/' + kind + '/[0-9a-f]{64}$').test(value)) return null;
  return value;
}

export function navigate(path: string, replace = false) {
  if (replace) history.replaceState(null, '', path);
  else history.pushState(null, '', path);
  window.dispatchEvent(new PopStateEvent('popstate'));
}

export function followLink(event: { preventDefault(): void; button: number; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }, path: string) {
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || event.button !== 0) return;
  event.preventDefault();
  navigate(path);
}
