export type RoomEvent = {
  type: 'room.changed';
  room_id: string;
  schema_version: 1;
  sequence: number;
};

export function parseRoomEvent(value: unknown, roomId: string, after: number): RoomEvent | undefined {
  if (typeof value !== 'object' || value === null) return;
  const event = value as Partial<RoomEvent>;
  if (event.type !== 'room.changed'
    || typeof event.room_id !== 'string'
    || event.room_id.toLowerCase() !== roomId.toLowerCase()
    || event.schema_version !== 1
    || !Number.isSafeInteger(event.sequence)
    || (event.sequence as number) <= after) return;
  return event as RoomEvent;
}

export function reconnectDelay(attempt: number): number {
  return Math.min(1000 * 2 ** Math.min(Math.max(0, attempt), 5), 30_000);
}
