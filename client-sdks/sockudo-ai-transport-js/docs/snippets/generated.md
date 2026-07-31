# Generated Demo Snippets

Do not edit by hand. Run `pnpm docs:snippets:update` after changing demo snippet markers.

<!-- prettier-ignore-start -->

## core-branch-views

Source: `demo/app.vue`

<!-- snippet:core-branch-views -->

```vue
const primaryView = useView({ session });
const compareView = useCreateView({ session });
const rawMessages = useSockudoMessages({ session });
const activeRuns = useActiveRuns({ session });
const tree = useTree({ session });
const channel = realtime.channels.get(channelName, {
  params: { rewind: { count: 100 } },
});
```

## core-route

Source: `demo/server/api/chat.post.ts`

<!-- snippet:core-route -->

```ts
async function runTurn(
  turn: ReturnType<ReturnType<typeof createAgentSession>["createRun"]>,
  body: Record<string, unknown>,
  model: string,
): Promise<void> {
  await turn.start();
  try {
    const stream = hasGatewayKey()
      ? await liveGatewayStream(body, model, turn.abortSignal)
      : demoUiMessageStream(latestText(body));
    await turn.streamResponse(stream);
    await turn.end("complete");
  } catch (error) {
    await turn.streamResponse(errorStream(error));
    await turn.end("error");
  }
}
```

## usechat-provider

Source: `demo/app.vue`

<!-- snippet:usechat-provider -->

```vue
const transportProvider = provideChatTransport({
  api: "/api/chat",
  channelName,
  client: realtime,
  clientId,
  chatOptions: {
    prepareSendMessagesRequest(context) {
      return {
        headers: {
          "x-demo-client-id": clientId,
          "x-demo-trigger": context.trigger,
        },
        body: {
          channelName,
          demoHistoryCount: context.history.length,
          demoMessageCount: context.messages.length,
          demoParent: context.parent,
          demoForkOf: context.forkOf,
        },
      };
    },
  },
});
```

## usechat-route

Source: `demo/server/api/chat.post.ts`

<!-- snippet:usechat-route -->

```ts
  const session = createAgentSession({
    client: realtimeClient(),
    channelName,
  });
  const turn = session.createRun({
    runId,
    invocationId,
    inputEventId,
    ...(clientId === undefined ? {} : { clientId }),
    onCancel(request) {
      return request.filter.all === true || request.runOwners.get(runId) === clientId;
    },
    onError(error) {
      console.error("[sockudo-ai-transport-demo] turn failed", error.message);
    },
  });
```

## usechat-sync

Source: `demo/app.vue`

<!-- snippet:usechat-sync -->

```vue
watch(
  () => primaryView.messages.value,
  (next) => {
    const nextMessages = safeArray(next);
    if (nextMessages.length > messages.value.length && chat.status.value !== "streaming") {
      chat.messages.value = [...nextMessages] as typeof chat.messages.value;
    }
  },
);
```

<!-- prettier-ignore-end -->
