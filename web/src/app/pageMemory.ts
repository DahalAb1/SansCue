// Page-lifetime copies of issued links and unsent drafts. Reloading the page drops them.
// Do not copy this module into web storage or logs.

type IssuedInvite = { id: string; expires: string; link: string | null };

const joins = new Map<string, string>();
const invites = new Map<string, IssuedInvite>();
const drafts = new Map<string, string>();

export function memoryJoin(roomId: string): string | null {
  return joins.get(roomId) ?? null;
}

export function keepJoin(roomId: string, link: string | null) {
  if (link) joins.set(roomId, link);
  else joins.delete(roomId);
}

export function memoryInvite(roomId: string): IssuedInvite | undefined {
  return invites.get(roomId);
}

export function keepInvite(roomId: string, invite: IssuedInvite | undefined) {
  if (invite) invites.set(roomId, invite);
  else invites.delete(roomId);
}

export function draftKey(roomId: string, membershipId: string, role: string): string {
  return roomId + '\n' + membershipId + '\n' + role;
}

export function memoryDraft(key: string): string {
  return drafts.get(key) ?? '';
}

export function keepDraft(key: string, value: string) {
  if (value) drafts.set(key, value);
  else drafts.delete(key);
}
