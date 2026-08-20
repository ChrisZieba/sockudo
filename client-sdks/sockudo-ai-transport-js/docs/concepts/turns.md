# Runs

A run starts with `ai-run-start`, streams output through mutable `sockudo:message.*` operations, and
ends with `ai-run-end`. End reasons are `complete`, `cancelled`, `error`, and `suspended`.

Agents must call `end()` in a `finally` block or map stream results through `vercelRunEndReason` so
clients do not keep runs active forever. Pre-3.0 turn-named wire fields remain readable only for
history migration.
