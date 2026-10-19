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

export function respondentSummary(dashboard: Dashboard): string {
  if (dashboard.respondents === 0) return 'No responses yet';
  return `${dashboard.respondents} audience response${dashboard.respondents === 1 ? '' : 's'}`;
}
