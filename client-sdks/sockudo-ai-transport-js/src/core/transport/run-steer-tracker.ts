import { HEADER_STEER_CODEC_MESSAGE_IDS } from "../../constants.js";

/**
 * Maximum steer ids one stamp may carry.
 *
 * The Sockudo server bounds `steer-codec-message-ids` at 2 KiB, and a
 * UUID-shaped codec-message-id costs roughly 42 bytes inside the JSON array.
 * Overflow rejects the whole assistant output publish rather than dropping the
 * stamp, so the cap is deliberately well under the ceiling.
 *
 * Consequence worth knowing: past the cap the oldest drained ids are omitted,
 * so their `steer()` outcome reports `consumed: false` even though the agent
 * did see them. Losing outcome precision beats losing the message.
 */
export const MAX_STEER_IDS_PER_STAMP = 32;

/**
 * Per-run steer bookkeeping on the agent side.
 *
 * Steers arrive as client inputs while a run is already executing. The agent's
 * loop drains them at a step boundary via {@link drain}; whatever was drained
 * is then stamped onto that step attempt's outputs so the client can settle
 * outcomes.
 */
export class RunSteerTracker {
  private pending: string[] = [];
  private drained: string[] = [];

  /** Records a steer input that arrived for this run. */
  public add(codecMessageId: string): void {
    if (this.pending.includes(codecMessageId) || this.drained.includes(codecMessageId)) {
      return;
    }
    this.pending.push(codecMessageId);
  }

  /** Whether unclaimed steer input is waiting. */
  public get hasPending(): boolean {
    return this.pending.length > 0;
  }

  /**
   * Moves pending steers into the drained set and returns them.
   *
   * Called when the agent's loop is about to run an inference, so "drained"
   * means "visible to the iteration that produced the next output".
   */
  public drain(): readonly string[] {
    if (this.pending.length === 0) {
      return [];
    }
    const claimed = this.pending;
    this.pending = [];
    this.drained = [...this.drained, ...claimed];
    return claimed;
  }

  /** Every steer drained so far, newest last, capped for the wire. */
  public get stampIds(): readonly string[] {
    return this.drained.length <= MAX_STEER_IDS_PER_STAMP
      ? this.drained
      : this.drained.slice(-MAX_STEER_IDS_PER_STAMP);
  }

  /**
   * Transport headers stamping the drained steers, or an empty map when none.
   *
   * Omitted rather than empty when nothing was drained: the server rejects an
   * empty value for this key.
   */
  public stampHeaders(): Record<string, string> {
    const ids = this.stampIds;
    if (ids.length === 0) {
      return {};
    }
    return { [HEADER_STEER_CODEC_MESSAGE_IDS]: JSON.stringify(ids) };
  }

  /** Forgets all state for this run. */
  public clear(): void {
    this.pending = [];
    this.drained = [];
  }
}
