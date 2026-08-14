# Tool Calling

Server-executed tools stream inline. Approval-only client input resumes the suspended run. A
client-executed tool result instead opens a new assistant reply fork, carries the suspended run's
projection as a fork seed, and stamps the `supersedes` transport header. This prevents concurrent
browser tabs from folding different tool results into one shared suspended run.

`createChatTransport` performs this automatically. Callers using `view.sendInput` directly can use
`createToolResultFork` from `@sockudo/ai-transport/vercel`; pass its returned `input` and
`sendOptions` together. The helper validates that the suspended projection owns the tool call and
intentionally leaves `runId` unset so the send creates a new fork.
