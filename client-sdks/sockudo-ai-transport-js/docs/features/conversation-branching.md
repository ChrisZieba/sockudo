# Conversation Branching

Edits and regenerations create forks through `parent`, `fork-of`, and `msg-regenerate` transport
headers. Views select siblings without mutating the underlying tree.

When a client resolves a tool call from a suspended assistant run, the resolution opens a new
assistant fork and carries `supersedes=<suspended-run-id>`. A single resolution therefore stays
linear in the visible view, while concurrent forks remain selectable siblings. The superseded trunk
is retained for history and direct tree lookup.
