import { invoke } from "@tauri-apps/api/core";
import { listen, type Event } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { ParsedPoint } from "./lib/points";
import "./overlay.css";

type VirtualBoundsPayload = {
  origin: { x: number; y: number };
  width: number;
  height: number;
};

export default function OverlayAppRoot() {
  const [listenGlow, setListenGlow] = useState(false);
  const [bubble, setBubble] = useState("");
  const [viewportPiece, setViewportPiece] = useState<VirtualBoundsPayload | null>(
    null,
  );
  const [buddyPos, setBuddyPos] = useState({ leftPx: 120, topPx: 400 });

  const viewportRef = useRef<VirtualBoundsPayload | undefined>(undefined);

  useEffect(() => {
    viewportRef.current = viewportPiece ?? undefined;
  }, [viewportPiece]);

  useEffect(() => {
    void invoke("companion_overlay_set_click_through", {
      ignoreCursorEvents: true,
    }).catch(() => undefined);
  }, []);

  useEffect(() => {
    let assistantDraft = "";
    const unsubs: Array<() => void> = [];

    void (async () => {
      unsubs.push(
        await listen<VirtualBoundsPayload>(
          "virtual-desktop-bounds",
          (evt: Event<VirtualBoundsPayload>) => {
            setViewportPiece(evt.payload);
          },
        ),
      );

      unsubs.push(
        await listen<{ state: string }>(
          "overlay-listen-state",
          (evt: Event<{ state: string }>) => {
            setListenGlow(evt.payload.state === "listening");
          },
        ),
      );

      unsubs.push(
        await listen("overlay-assistant-reset", () => {
          assistantDraft = "";
          setBubble("");
        }),
      );

      unsubs.push(
        await listen<{ partial: string }>(
          "overlay-transcript-live",
          (evt: Event<{ partial: string }>) => {
            setBubble(evt.payload.partial);
          },
        ),
      );

      unsubs.push(
        await listen<{ incrementalText: string }>(
          "overlay-assistant-append",
          (evt: Event<{ incrementalText: string }>) => {
            assistantDraft += evt.payload.incrementalText;
            setBubble(assistantDraft);
          },
        ),
      );

      unsubs.push(
        await listen<{ assistantTextComplete: string }>(
          "overlay-assistant-final",
          (evt: Event<{ assistantTextComplete: string }>) => {
            assistantDraft = evt.payload.assistantTextComplete;
            setBubble(assistantDraft);
          },
        ),
      );

      unsubs.push(
        await listen<{ pointsDetected: ParsedPoint[] }>(
          "overlay-point-sequence",
          async (evt: Event<{ pointsDetected: ParsedPoint[] }>) => {
            const vp = viewportRef.current;
            await invoke("companion_overlay_show", { show: true }).catch(
              () => undefined,
            );
            if (!vp) {
              return;
            }
            for (const p of evt.payload.pointsDetected) {
              setBuddyPos({
                leftPx: p.x - vp.origin.x,
                topPx: p.y - vp.origin.y,
              });
              await new Promise<void>((r) => window.setTimeout(r, 760));
            }
          },
        ),
      );
    })();

    return () => unsubs.forEach((u) => u());
  }, []);

  useEffect(() => {
    if (!viewportPiece) {
      return;
    }
    setBuddyPos((prev) =>
      prev.leftPx === 120 && prev.topPx === 400
        ? { leftPx: 140, topPx: Math.max(120, viewportPiece.height - 220) }
        : prev,
    );
  }, [viewportPiece]);

  return (
    <div className="overlayRootMeasurement">
      <div className={listenGlow ? "buddyGlow listening" : "buddyGlow"} />

      <div
        className="bubbleOverlayMeasurement"
        style={{
          transform: `translate(${Math.min(buddyPos.leftPx + 44, 900)}px, ${Math.max(
            buddyPos.topPx - 90,
            12,
          )}px)`,
        }}
      >
        {bubble.length === 0 ? "\u00a0" : bubble}
      </div>

      <div
        className="buddyOrbMeasurement"
        style={{ left: buddyPos.leftPx, top: buddyPos.topPx }}
      />
    </div>
  );
}
