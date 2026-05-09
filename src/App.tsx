import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";


import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { streamOpenRouterChatCompletion } from "./lib/openrouter";
import { companionSystemPrompt } from "./lib/prompt";
import { extractPointTags, stripPointTagsForSpeech } from "./lib/points";
import "./index.css";

type Settings = {
  proxyBaseUrl: string;
  openrouterModel: string;
  deepgramTtsModel: string;
  showBuddyAlways: boolean;
};

type CapturedScreen = {
  index: number;
  jpegBase64: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
};

export default function App() {
  const [settings, setSettings] = useState<Settings>({
    proxyBaseUrl: "",
    openrouterModel: "openai/gpt-4o-mini",
    deepgramTtsModel: "aura-2-thalia-en",
    showBuddyAlways: true,
  });
  const [status, setStatus] = useState<string>("Idle");
  const [lastError, setLastError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const persistedDraft = useMemo(() => ({ ...settings }), [settings]);

  useEffect(() => {
    let cancelled = false;
    void invoke<Settings>("companion_load_settings").then((loaded) => {
      if (cancelled) {
        return;
      }
      setSettings({
        proxyBaseUrl: loaded.proxyBaseUrl ?? "",
        openrouterModel: loaded.openrouterModel ?? "openai/gpt-4o-mini",
        deepgramTtsModel:
          loaded.deepgramTtsModel ?? "aura-2-thalia-en",
        showBuddyAlways: loaded.showBuddyAlways ?? true,
      });
    });

    async function bootstrap() {
      await listen<{ state: string }>("buddy-listen-state", async (evt) => {
        await emit("overlay-listen-state", { state: evt.payload.state }).catch(
          () => undefined,
        );
        setStatus(
          evt.payload.state === "listening" ? "Listening (mic)" : "Idle",
        );
      });

      await listen<{ message: string }>("listen-error", (evt) => {
        setLastError(evt.payload.message);
        setStatus("Error");
      });

      await listen<{ transcript: string }>("listen-done", async (evt) => {
        setLastError(null);
        const transcriptTrimmed = evt.payload.transcript.trim();
        await emit("overlay-assistant-reset", {}).catch(() => undefined);
        if (!transcriptTrimmed) {
          return;
        }
        await pipelineTurn(transcriptTrimmed);
      });

      await listen<{ text: string }>("stt-partial", async (evt) => {
        await emit("overlay-transcript-live", {
          partial: evt.payload.text,
        }).catch(() => undefined);
      });
    }

    void bootstrap();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pipelineTurn = useCallback(
    async (userSpeechTranscript: string) => {
      if (!settings.proxyBaseUrl.trim()) {
        setLastError("Set Cloudflare Worker base URL.");
        return;
      }
      abortRef.current?.abort();
      abortRef.current = new AbortController();
      setStatus("Thinking");
      await invoke("companion_overlay_show", { show: true }).catch(() => undefined);

      let screenshots: CapturedScreen[] = [];
      try {
        screenshots = await invoke<CapturedScreen[]>(
          "companion_capture_all_screens_command",
        );
      } catch (screenCaptureIssue) {
        setLastError(String(screenCaptureIssue));
        setStatus("Capture failed");
        return;
      }

      const multimodalContent: unknown[] = [
        {
          type: "text",
          text:
            "You receive sequential JPEG screenshots in ascending monitor index. Each line before an image describes that image's virtual desktop offsets.",
        },
      ];

      for (const screenshot of screenshots) {
        multimodalContent.push({
          type: "text",
          text: `Monitor index=${screenshot.index}; virtual offset_x=${screenshot.x}; offset_y=${screenshot.y}; width=${screenshot.width}; height=${screenshot.height}; scale=${screenshot.scaleFactor}`,
        });
        multimodalContent.push({
          type: "image_url",
          image_url: {
            url: `data:image/jpeg;base64,${screenshot.jpegBase64}`,
          },
        });
      }

      multimodalContent.push({
        type: "text",
        text: `User voice transcript:\n${userSpeechTranscript}`,
      });

      let assistantAccumulator = "";

      try {
        assistantAccumulator = await streamOpenRouterChatCompletion({
          proxyBaseUrl: settings.proxyBaseUrl.trim(),
          model: settings.openrouterModel.trim(),
          messages: [
            { role: "system", content: companionSystemPrompt },
            {
              role: "user",
              content: multimodalContent,
            },
          ],
          signal: abortRef.current.signal,
          onDelta: async (fragment) =>
            emit("overlay-assistant-append", { incrementalText: fragment }).catch(
              () => undefined,
            ),
        });
      } catch (chatFailure) {
        setLastError(String(chatFailure));
        setStatus("Chat failed");
        return;
      }

      await emit("overlay-assistant-final", {
        assistantTextComplete: assistantAccumulator,
      }).catch(() => undefined);

      const narrationPlain = stripPointTagsForSpeech(assistantAccumulator);

      try {
        if (narrationPlain.length > 0) {
          setStatus("Speaking");
          await invoke("companion_play_tts", { textPlain: narrationPlain });
        }
      } catch (ttsFailure) {
        setLastError(String(ttsFailure));
      }

      const pointsFound = extractPointTags(assistantAccumulator);
      await emit("overlay-point-sequence", {
        pointsDetected: pointsFound,
      }).catch(() => undefined);

      setStatus("Idle");
    },
    [settings.openrouterModel, settings.proxyBaseUrl],
  );

  const saveClicked = async () => {
    setLastError(null);
    await invoke("companion_save_settings", { payload: persistedDraft }).catch((e) => {
      setLastError(String(e));
    });
    await invoke("companion_set_proxy_url", {
      proxyBaseUrl: persistedDraft.proxyBaseUrl.trim(),
    }).catch(() => undefined);
    if (persistedDraft.showBuddyAlways) {
      await invoke("companion_overlay_show", { show: true }).catch(() => undefined);
    } else {
      await invoke("companion_overlay_show", { show: false }).catch(() => undefined);
    }
    setStatus("Saved");
    window.setTimeout(() => setStatus("Idle"), 1200);
  };

  return (
    <div style={{ padding: 16 }}>
      <div className="card" style={{ display: "grid", gap: 10 }}>
        <div className="pill">Clicky Tauri • Windows-ready</div>
        <div>
          <strong style={{ fontSize: "1rem" }}>Clicky panel</strong>
          <p className="muted" style={{ margin: "6px 0 0", fontSize: "0.8rem" }}>
            Hold <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>Space</kbd> to talk — release to send your
            question.
          </p>
        </div>

        <div style={{ fontSize: "0.75rem" }}>Status: {status}</div>

        <label className="field">
          Cloudflare Worker base URL
          <input
            value={settings.proxyBaseUrl}
            placeholder="https://your-worker.workers.dev"
            onChange={(e) =>
              setSettings({
                ...settings,
                proxyBaseUrl: e.target.value,
              })
            }
          />
        </label>

        <label className="field">
          OpenRouter model id
          <input
            value={settings.openrouterModel}
            onChange={(e) =>
              setSettings({
                ...settings,
                openrouterModel: e.target.value,
              })
            }
          />
        </label>

        <label className="field">
          Deepgram TTS voice
          <input
            value={settings.deepgramTtsModel}
            onChange={(e) =>
              setSettings({
                ...settings,
                deepgramTtsModel: e.target.value,
              })
            }
          />
        </label>

        <label style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input
            type="checkbox"
            checked={settings.showBuddyAlways}
            onChange={(e) =>
              setSettings({
                ...settings,
                showBuddyAlways: e.target.checked,
              })
            }
          />
          Always show buddy overlay between turns
        </label>

        <button
          type="button"
          onClick={() => void saveClicked()}
          style={{
            padding: "0.5rem",
            borderRadius: 10,
            border: "1px solid #4d63ff",
            background: "#3c52eb",
            color: "#fff",
          }}
        >
          Save
        </button>

        <button
          type="button"
          onClick={() =>
            invoke("companion_overlay_show", { show: true }).catch(() => undefined)
          }
          style={{
            padding: "0.45rem",
            borderRadius: 10,
            border: "1px solid #2c2f3d",
            background: "#1a1b24",
            color: "#eaeaf0",
          }}
        >
          Show overlay
        </button>

        {lastError !== null ? (
          <div
            style={{
              fontSize: "0.72rem",
              padding: "0.55rem",
              borderRadius: 10,
              background: "#3b1419",
              color: "#fdc6cc",
            }}
          >
            {lastError}
          </div>
        ) : null}
      </div>
    </div>
  );
}
