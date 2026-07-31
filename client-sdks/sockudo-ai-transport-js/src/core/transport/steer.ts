import { HEADER_RUN_ID, HEADER_STEER_CODEC_MESSAGE_IDS } from "../../constants.js";
import { ErrorCode, ErrorInfo } from "../../errors.js";
import type { Serial } from "../../realtime/types.js";
import type { HeaderMap } from "../../utils.js";
import type { RunEndReason } from "./tree.js";

/**
 * Outcome of a steering publish, derived from the agent's responses.
 *
 * Each agent response carries a `steer-codec-message-ids` header listing the
 * steers its loop had drained when the stamping step attempt opened. The union
 * of those lists per run is what decides membership.
 */
export interface SteerOutcome {
  /**
   * Whether the steer landed inside the run's consumed-input window.
   *
   * `true` means the agent's loop had it visible at the iteration it drained
   * on — not that the agent's prompt actually used it. The wire cannot report
   * more than visibility, so neither does this.
   */
  consumed: boolean;
  /**
   * Terminal reason of the run, present when the outcome was settled by a run
   * end. Absent when settled by a suspend, since a later resume may still
   * consume the steer.
   */
  runTerminalReason?: RunEndReason;
}

/**
 * Result of {@link ClientRun.steer}.
 */
export interface SteerResult {
  /** Resolves when the steer publish lands, carrying the assigned serial. */
  published: Promise<{ serial: Serial | undefined }>;
  /** Resolves with consumed/not-consumed at the run's next terminal event. */
  outcome: Promise<SteerOutcome>;
}

interface PendingSteer {
  codecMessageId: string;
  resolve(outcome: SteerOutcome): void;
  reject(error: ErrorInfo): void;
}

/**
 * Reads the steer stamp off a message's transport headers.
 *
 * The wire value is a JSON array. A malformed value is treated as absent
 * rather than thrown: a stamp is an observation used to settle outcomes, and
 * one unreadable stamp must not break folding of the message it rode on.
 */
export function readSteerStamp(headers: HeaderMap): readonly string[] {
  const raw = headers[HEADER_STEER_CODEC_MESSAGE_IDS];
  if (raw === undefined || raw.length === 0) {
    return [];
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter((value): value is string => typeof value === "string");
  } catch {
    return [];
  }
}

/**
 * Tracks in-flight steers per run and settles their outcomes.
 *
 * One coordinator per client session. Outcomes settle when a run reaches a
 * terminal lifecycle event; anything still pending when the session closes is
 * rejected rather than left hanging.
 */
export class SteerCoordinator {
  private readonly pendingByRun = new Map<string, PendingSteer[]>();
  private readonly stampsByRun = new Map<string, Set<string>>();

  /** Registers a steer awaiting its outcome. */
  public track(runId: string, codecMessageId: string): Promise<SteerOutcome> {
    return new Promise<SteerOutcome>((resolve, reject) => {
      const pending = this.pendingByRun.get(runId) ?? [];
      pending.push({ codecMessageId, resolve, reject });
      this.pendingByRun.set(runId, pending);
    });
  }

  /**
   * Folds a stamp observed on an agent response into the run's union.
   *
   * Accumulating rather than replacing matters: a run's responses each report
   * only what was drained by the step attempt that produced them, so the
   * consumed window is the union across the run, not the latest stamp.
   */
  public observe(headers: HeaderMap): void {
    const runId = headers[HEADER_RUN_ID];
    if (runId === undefined) {
      return;
    }
    const stamped = readSteerStamp(headers);
    if (stamped.length === 0) {
      return;
    }
    const union = this.stampsByRun.get(runId) ?? new Set<string>();
    for (const id of stamped) {
      union.add(id);
    }
    this.stampsByRun.set(runId, union);
  }

  /**
   * Settles every steer pending on a run.
   *
   * A suspend settles outcomes without a terminal reason and keeps the stamp
   * union, because a resume can still consume steers that have not landed yet.
   * A run end drops the union.
   */
  public settle(runId: string, reason: RunEndReason | undefined, terminal: boolean): void {
    const pending = this.pendingByRun.get(runId);
    if (pending === undefined || pending.length === 0) {
      if (terminal) {
        this.stampsByRun.delete(runId);
      }
      return;
    }
    const union = this.stampsByRun.get(runId);
    const settled: PendingSteer[] = [];
    for (const steer of pending) {
      const consumed = union?.has(steer.codecMessageId) === true;
      // A suspend only settles what it can prove consumed; the rest stay
      // pending so a resume can still claim them.
      if (!terminal && !consumed) {
        continue;
      }
      steer.resolve({
        consumed,
        ...(terminal && reason !== undefined ? { runTerminalReason: reason } : {}),
      });
      settled.push(steer);
    }
    const remaining = pending.filter((steer) => !settled.includes(steer));
    if (remaining.length === 0) {
      this.pendingByRun.delete(runId);
    } else {
      this.pendingByRun.set(runId, remaining);
    }
    if (terminal) {
      this.stampsByRun.delete(runId);
    }
  }

  /** Rejects everything still pending. Called on close and continuity loss. */
  public drain(message: string): void {
    for (const pending of this.pendingByRun.values()) {
      for (const steer of pending) {
        steer.reject(
          new ErrorInfo({
            code: ErrorCode.SessionClosed,
            message,
          }),
        );
      }
    }
    this.pendingByRun.clear();
    this.stampsByRun.clear();
  }
}
