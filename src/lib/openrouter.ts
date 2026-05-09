export type StreamChatParams = {
  proxyBaseUrl: string;
  model: string;
  messages: unknown[];
  onDelta: (chunk: string) => void;
  signal?: AbortSignal;
};

/**
 * POST {proxy}/chat with OpenRouter-compatible JSON; parse SSE text/event-stream.
 */
export async function streamOpenRouterChatCompletion(
  params: StreamChatParams,
): Promise<string> {
  const url = `${params.proxyBaseUrl.replace(/\/$/, "")}/chat`;
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model: params.model,
      stream: true,
      messages: params.messages,
    }),
    signal: params.signal,
  });

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`OpenRouter proxy ${response.status}: ${errorBody}`);
  }

  const reader = response.body?.getReader();
  if (!reader) {
    throw new Error("Missing response body stream");
  }

  const decoder = new TextDecoder();
  let buffer = "";
  let assembledAssistant = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed.startsWith("data:")) {
        continue;
      }
      const dataPart = trimmed.slice(5).trim();
      if (dataPart === "[DONE]") {
        continue;
      }
      try {
        const parsed = JSON.parse(dataPart) as {
          choices?: Array<{ delta?: { content?: string | null } }>;
        };
        const piece = parsed.choices?.[0]?.delta?.content;
        if (typeof piece === "string" && piece.length > 0) {
          assembledAssistant += piece;
          params.onDelta(piece);
        }
      } catch {
        // Incomplete JSON chunk across lines — ignore lone fragments
      }
    }
  }

  return assembledAssistant;
}
