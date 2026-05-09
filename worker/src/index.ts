/**
 * Clicky-Tauri proxy Worker
 *
 * Routes:
 * POST /chat → OpenRouter chat completions (streaming SSE)
 * POST /tts → Deepgram /v1/speak (MP3)
 * POST /deepgram-listen-token → Deepgram /v1/auth/grant (temporary listen JWT)
 */

interface Env {
  OPENROUTER_API_KEY: string;
  DEEPGRAM_API_KEY: string;
  DEFAULT_DEEPGRAM_TTS_MODEL: string;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (request.method !== "POST") {
      return new Response("Method not allowed", { status: 405 });
    }

    try {
      if (url.pathname === "/chat") {
        return await handleChat(request, env);
      }
      if (url.pathname === "/tts") {
        return await handleTts(request, env);
      }
      if (url.pathname === "/deepgram-listen-token") {
        return await handleDeepgramListenToken(env);
      }
    } catch (error) {
      console.error(`[${url.pathname}]`, error);
      return new Response(JSON.stringify({ error: String(error) }), {
        status: 500,
        headers: { "content-type": "application/json" },
      });
    }

    return new Response("Not found", { status: 404 });
  },
};

async function handleChat(request: Request, env: Env): Promise<Response> {
  const body = await request.text();

  const upstream = await fetch("https://openrouter.ai/api/v1/chat/completions", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${env.OPENROUTER_API_KEY}`,
      "Content-Type": "application/json",
    },
    body,
  });

  if (!upstream.ok) {
    const errorBody = await upstream.text();
    console.error("[/chat] OpenRouter error", upstream.status, errorBody);
    return new Response(errorBody, {
      status: upstream.status,
      headers: { "content-type": upstream.headers.get("content-type") || "application/json" },
    });
  }

  return new Response(upstream.body, {
    status: upstream.status,
    headers: {
      "content-type": upstream.headers.get("content-type") || "text/event-stream",
      "cache-control": "no-cache",
    },
  });
}

type TtsRequestBody = {
  text?: string;
  model?: string;
};

async function handleTts(request: Request, env: Env): Promise<Response> {
  const parsed = JSON.parse(await request.text()) as TtsRequestBody;
  const text = parsed.text ?? "";
  if (!text.trim()) {
    return new Response(JSON.stringify({ error: "text is required" }), {
      status: 400,
      headers: { "content-type": "application/json" },
    });
  }

  const model = parsed.model || env.DEFAULT_DEEPGRAM_TTS_MODEL || "aura-2-thalia-en";
  const qs = new URLSearchParams({
    model,
    encoding: "mp3",
  });

  const upstream = await fetch(`https://api.deepgram.com/v1/speak?${qs}`, {
    method: "POST",
    headers: {
      Authorization: `Token ${env.DEEPGRAM_API_KEY}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ text }),
  });

  if (!upstream.ok) {
    const errorBody = await upstream.text();
    console.error("[/tts] Deepgram error", upstream.status, errorBody);
    return new Response(errorBody, {
      status: upstream.status,
      headers: { "content-type": "application/json" },
    });
  }

  return new Response(upstream.body, {
    status: upstream.status,
    headers: {
      "content-type": upstream.headers.get("content-type") || "audio/mpeg",
      "cache-control": "no-store",
    },
  });
}

async function handleDeepgramListenToken(env: Env): Promise<Response> {
  const upstream = await fetch("https://api.deepgram.com/v1/auth/grant", {
    method: "POST",
    headers: {
      Authorization: `Token ${env.DEEPGRAM_API_KEY}`,
      "Content-Type": "application/json",
    },
    body: "{}",
  });

  const bodyText = await upstream.text();
  if (!upstream.ok) {
    console.error("[/deepgram-listen-token]", upstream.status, bodyText);
    return new Response(bodyText, {
      status: upstream.status,
      headers: { "content-type": "application/json" },
    });
  }

  return new Response(bodyText, {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}
