# Clicky-Tauri ↔ Cloudflare Worker API contracts

The desktop app calls only the Worker URL (configured in-app). API keys never ship in the binary.

## Coordinate system for `[POINT:x,y:label:screenN]`

- **Origin**: Virtual desktop coordinates: top-left of the bounding box that contains all monitors is `(0, 0)`. X grows right; Y grows down (same as Windows `GetSystemMetrics`/virtual screen origin conventions used by capture).
- **x, y**: Integer pixels in **virtual-desktop space**. The LLM receives labeled screenshots; each image’s user message specifies the monitor index and the **offset** `(offset_x, offset_y)` for that monitor in virtual space so the model can relate on-screen pixels to global points.
- **screenN**: **0-based** index into the ordered list returned by `capture_all_screens` / `enumerate_monitors` (stable for the session: primary first, then left-to-right, top-to-bottom by virtual position).

The system prompt instructs the model to emit tags only using this convention:

`[POINT:virtual_x,virtual_y:short_label:screen_index]`

`label` is a short human-readable cue (no unescaped `]`).

---

## `POST /chat`

Proxies to OpenRouter **`https://openrouter.ai/api/v1/chat/completions`**.

### Request headers (client → Worker)

| Header | Required | Description |
|--------|----------|-------------|
| `Content-Type` | yes | `application/json` |

### Request body

OpenAI-compatible **chat completions** JSON. The app sends:

- `model` — OpenRouter model id (e.g. `openai/gpt-4o`).
- `stream` — `true` for streamed responses.
- `messages` — includes a `system` message with POINT instructions, then `user` multimodal messages: `content` array with `{ "type": "text", "text": "..." }` and `{ "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,..." } }` per captured monitor **in index order**.
- Optional OpenRouter headers are passed through if the client sets them (e.g. `HTTP-Referer`, `X-Title`); the Worker may ignore unknown fields.

Worker adds `Authorization: Bearer <OPENROUTER_API_KEY>`.

### Response

Same as upstream: SSE (`text/event-stream`) when `stream: true`. Each `data:` line is JSON except `[DONE]`.

---

## `POST /tts`

Proxies to Deepgram **Speak** REST API.

### Request body (JSON)

```json
{
  "text": "Plain text for speech. POINT tags stripped before send.",
  "model": "aura-2-thalia-en"
}
```

`model` is optional; Worker may default from `vars`.

### Response

Raw audio body. Default upstream choice: **`audio/mpeg`** (MP3). Client decodes MP3 via `symphonia` / `rodio` in Rust.

Worker sets appropriate `Authorization: Token <DEEPGRAM_API_KEY>` (Deepgram REST uses `Token` scheme for API keys).

---

## `POST /deepgram-listen-token`

Issues a short-lived token for browser/desktop WebSocket clients that cannot safely hold the master API key.

### Request body

Optional JSON `{}` (reserved).

### Response

```json
{
  "access_token": "<jwt>",
  "expires_in": 30
}
```

Matches Deepgram `POST https://api.deepgram.com/v1/auth/grant` response shape. The app must open the listen WebSocket **immediately** after receiving the token.

---

## Deepgram Live (client → Deepgram, not Worker)

WebSocket URL (example):

`wss://api.deepgram.com/v1/listen?model=nova-2&encoding=linear16&sample_rate=16000&channels=1&interim_results=true`

Headers:

`Authorization: Bearer <access_token>`

Audio: raw **linear16** PCM mono little-endian frames (e.g. 20–40 ms) sent as binary WebSocket messages. Close the socket when PTT ends; use the last finalized transcript for the LLM turn.
