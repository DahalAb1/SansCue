import { isUuid } from './api';
import type { BeeState, TranscriptLine } from './api';

export type TransportState = 'connected' | 'reconnecting' | 'not_running';
export type LiveRecording = { source_conversation_id: string; first_seen_at: string; last_seen_at: string; utterances: number; bound_here: boolean };
export type BeeConnection = { transport: { state: TransportState; reported_at: string | null }; recordings: LiveRecording[] };
export type ConnectionStatus = { tone: 'idle' | 'waiting' | 'live' | 'warning'; label: string; detail: string };

/** Speech heard within this window counts as "receiving"; older means the speaker has gone quiet. */
export const RECEIVING_WINDOW_MS = 30_000;

function isTime(value: unknown): value is string {
  return typeof value === 'string' && !Number.isNaN(Date.parse(value));
}

export function parseBeeConnection(value: unknown): BeeConnection | null {
  const body = (value ?? {}) as { transport?: { state?: unknown; reported_at?: unknown }; recordings?: unknown };
  const state = body.transport?.state;
  const reportedAt = body.transport?.reported_at;
  if (state !== 'connected' && state !== 'reconnecting' && state !== 'not_running') return null;
  if (reportedAt !== null && !isTime(reportedAt)) return null;
  if (!Array.isArray(body.recordings)) return null;
  const recordings: LiveRecording[] = [];
  for (const item of body.recordings) {
    const r = (item ?? {}) as Record<string, unknown>;
    if (!isUuid(r.source_conversation_id) || !isTime(r.first_seen_at) || !isTime(r.last_seen_at)
      || !Number.isSafeInteger(r.utterances) || (r.utterances as number) < 0 || typeof r.bound_here !== 'boolean') return null;
    recordings.push({ source_conversation_id: r.source_conversation_id, first_seen_at: r.first_seen_at, last_seen_at: r.last_seen_at, utterances: r.utterances as number, bound_here: r.bound_here });
  }
  return { transport: { state, reported_at: reportedAt ?? null }, recordings };
}

export function ago(at: string, now: number): string {
  const seconds = Math.max(0, Math.round((now - Date.parse(at)) / 1000));
  if (seconds < 60) return `${seconds} s ago`;
  return `${Math.round(seconds / 60)} min ago`;
}

/**
 * What the presenter should believe about delivery right now. A running Bee
 * stream alone is not proof: only transcript that reached this room counts.
 */
export function connectionStatus(bee: BeeState | null | undefined, transport: TransportState | undefined, last: TranscriptLine | null | undefined, now: number): ConnectionStatus {
  if (!bee) return { tone: 'warning', label: 'Bee status unavailable', detail: 'Refresh room state.' };
  if (bee.status === 'unbound') return { tone: 'idle', label: 'No recording connected', detail: 'Choose the recording for this talk below.' };
  if (bee.status === 'binding') return { tone: 'waiting', label: 'Connecting the recording…', detail: 'Waiting for the Bee listener to confirm this room’s recording.' };
  if (bee.status === 'unbinding') return { tone: 'waiting', label: 'Disconnecting the recording…', detail: 'New speech is no longer routed to this room.' };
  if (transport === 'not_running') return { tone: 'warning', label: 'Bee listener not running', detail: 'Start the live Bee worker on the presenter’s computer. Published questions and replies keep working meanwhile.' };
  if (transport === 'reconnecting') return { tone: 'warning', label: 'Reconnecting to Bee…', detail: 'The Bee stream restarted. Speech during the interruption may be missing.' };
  if (last === undefined) return { tone: 'waiting', label: 'Connected — checking for speech…', detail: 'Loading the latest transcript.' };
  if (last === null) return { tone: 'waiting', label: 'Connected — waiting for speech', detail: 'Say a short phrase, such as “SansCue connection check”, and confirm it appears here.' };
  const age = now - Date.parse(last.spoken_at);
  if (age <= RECEIVING_WINDOW_MS) return { tone: 'live', label: 'Receiving transcript', detail: `Last speech ${ago(last.spoken_at, now)}.` };
  return { tone: 'waiting', label: 'Connected — no recent speech', detail: `Last speech ${ago(last.spoken_at, now)}. Speak to confirm delivery.` };
}
