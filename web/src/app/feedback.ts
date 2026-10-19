import type { State } from './api';

export const responseOptions = [
  { value: 'clear', label: 'Clear' },
  { value: 'partly_clear', label: 'Partly clear' },
  { value: 'need_help', label: 'Need help' },
] as const;

export type ResponseValue = typeof responseOptions[number]['value'];
export type Dashboard = NonNullable<State['dashboard']>;
export type ChartRow = { value: ResponseValue; label: string; count: number; percent: number | null; width: number };

function finite(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function formatPercent(value: number | null): string {
  return finite(value) ? `${value.toFixed(1)}%` : '—';
}

/** Bars are sized by share of respondents, so zero responses leave every bar empty. */
export function chartRows(dashboard: Dashboard): ChartRow[] {
  return responseOptions.map(({ value, label }) => {
    const count = Math.max(0, dashboard.counts[value] ?? 0);
    const reported = dashboard.respondents > 0 ? dashboard.percentages[value] : null;
    const percent = finite(reported) ? Math.min(100, Math.max(0, reported)) : null;
    return { value, label, count, percent, width: percent ?? 0 };
  });
}

/** A draft without a known expiry stays reviewable; the server still enforces it. */
export function draftExpiry(expiresAt: string | undefined, now: number): { expired: boolean; label: string } {
  const at = expiresAt ? Date.parse(expiresAt) : Number.NaN;
  if (Number.isNaN(at)) return { expired: false, label: '' };
  const remaining = at - now;
  if (remaining <= 0) return { expired: true, label: 'Expired' };
  const minutes = Math.ceil(remaining / 60_000);
  return { expired: false, label: `Expires in ${minutes} min` };
}

export function earlierWordingNote(dashboard: Dashboard, version: number): string | null {
  const earlier = dashboard.earlier_version_respondents ?? 0;
  if (earlier <= 0 || version <= 1) return null;
  return `${earlier} of ${dashboard.respondents} response${dashboard.respondents === 1 ? ' was' : 's were'} given before the wording changed to version ${version}.`;
}

/** Chart label for a check-in. Other wording keeps its own words, without the question mark. */
export function topicFromQuestion(text: string): string {
  const trimmed = text.trim().replace(/\?+$/u, '').trim().replace(/\s+/g, ' ');
  const match = /^(?:how clear is|how well do you understand)\s+/i.exec(trimmed);
  const topic = (match ? trimmed.slice(match[0].length) : trimmed).trim();
  return [...topic].slice(0, 80).join('');
}

/** Need help counts fully and Partly clear counts half. No answers is not a score of 0. */
export function confusionScore(needHelp: number, partlyClear: number, respondents: number): number | null {
  if (!Number.isFinite(respondents) || respondents <= 0) return null;
  const score = Math.round(((needHelp + 0.5 * partlyClear) / respondents) * 100);
  return Math.min(100, Math.max(0, score));
}

export function answerCount(respondents: number): string {
  return `${respondents} ${respondents === 1 ? 'answer' : 'answers'}`;
}

export function respondentSummary(dashboard: Dashboard): string {
  if (dashboard.respondents === 0) return 'No responses yet';
  return `${dashboard.respondents} audience response${dashboard.respondents === 1 ? '' : 's'}`;
}
