/**
 * Prompt ordering repair for steers.
 *
 * Messages arrive in serial order, and a steer lands mid-run, so by serial it
 * sits *between* assistant outputs:
 *
 *   user "explain X" → assistant "X is…" → user (steer) "in French" → assistant …
 *
 * Feeding that straight to a model has two problems. The prompt ends on an
 * assistant message, which several providers handle poorly when asked for a
 * completion, and the steer is buried behind assistant text that came after it,
 * so it loses salience exactly when it should carry most.
 *
 * Moving unresponded steers to the tail fixes both: the prompt ends on the
 * user's latest instruction.
 */

/**
 * Identifies a message for ordering purposes.
 *
 * Deliberately structural rather than tied to a codec: callers hold
 * `AI.UIMessage`, OpenAI items, or their own shape, and only the id and role
 * matter here.
 */
export interface OrderableMessage {
  /** Codec message id. */
  id: string;
  /** Message role. */
  role: string;
}

/** Options for {@link reorderUnrespondedSteers}. */
export interface SteerOrderingOptions {
  /**
   * Codec message ids of steers the agent has not yet responded to.
   *
   * "Responded to" is not the same as drained: a steer the agent drained but
   * whose step produced no assistant output afterwards is still unresponded, so
   * callers usually derive this from the steers drained since the last assistant
   * message rather than from every steer seen.
   */
  unrespondedSteerIds: readonly string[];
}

/**
 * Moves unresponded steers to the tail, preserving their relative order.
 *
 * Returns the input array unchanged (same reference) when there is nothing to
 * move, so callers can use it on every prompt build without allocating.
 *
 * Only trailing assistant messages are jumped: a steer stays put if it is
 * already at the tail, and messages that are not named as unresponded are never
 * reordered relative to each other.
 */
export function reorderUnrespondedSteers<TMessage extends OrderableMessage>(
  messages: readonly TMessage[],
  options: SteerOrderingOptions,
): readonly TMessage[] {
  const { unrespondedSteerIds } = options;
  if (unrespondedSteerIds.length === 0 || messages.length === 0) {
    return messages;
  }
  const targets = new Set(unrespondedSteerIds);
  const moved: TMessage[] = [];
  const kept: TMessage[] = [];
  for (const message of messages) {
    if (targets.has(message.id)) {
      moved.push(message);
    } else {
      kept.push(message);
    }
  }
  if (moved.length === 0) {
    return messages;
  }
  // Already at the tail in the same order: nothing to repair, so avoid
  // reallocating and avoid implying a change the caller might diff on.
  const tail = messages.slice(messages.length - moved.length);
  if (tail.length === moved.length && tail.every((message, index) => message === moved[index])) {
    return messages;
  }
  return [...kept, ...moved];
}

/**
 * Derives which steers are still unresponded.
 *
 * A steer counts as responded once an assistant message appears after it in
 * serial order — that is the only evidence on the wire that the agent's output
 * came *after* seeing it. Steers drained by a step that then failed, or that
 * produced no output, therefore stay unresponded and get moved to the tail on
 * the next prompt build.
 */
export function unrespondedSteerIds<TMessage extends OrderableMessage>(
  messages: readonly TMessage[],
  steerIds: readonly string[],
): readonly string[] {
  if (steerIds.length === 0) {
    return [];
  }
  const steers = new Set(steerIds);
  const unresponded: string[] = [];
  for (let index = 0; index < messages.length; index += 1) {
    const message = messages[index];
    if (message === undefined || !steers.has(message.id)) {
      continue;
    }
    const answered = messages
      .slice(index + 1)
      .some((later) => later.role === "assistant" || later.role === "tool");
    if (!answered) {
      unresponded.push(message.id);
    }
  }
  return unresponded;
}
