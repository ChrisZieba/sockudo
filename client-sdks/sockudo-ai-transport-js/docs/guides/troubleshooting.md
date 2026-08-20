# Troubleshooting

| Symptom                          | Likely cause                                              | Fix                                                                      |
| -------------------------------- | --------------------------------------------------------- | ------------------------------------------------------------------------ |
| Empty assistant message          | Mutable messages disabled; server returns `93002`.        | Enable the mutable-message namespace for the AI channel.                 |
| History load fails               | Missing `history` capability.                             | Issue a token with `history` for the session channel.                    |
| Run never ends                   | Agent did not publish `ai-run-end`.                       | Call `end()` in `finally`; Vercel paths should use `vercelRunEndReason`. |
| Duplicate runs                   | React strict mode or edit-mid-stream double send.         | Keep session handles stable and cancel before editing active output.     |
| Cross-client cancellation denied | Local client id was trusted instead of verified identity. | Authorize against server-verified `clientId`.                            |
| Suspended state does not resume  | Continuation was published without `run-continue=true`.   | Continue the same `run-id` with a new `invocation-id`.                   |
