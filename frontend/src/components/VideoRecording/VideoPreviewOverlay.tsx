import { useEffect, useRef, useState } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useVideoRecordingState } from './useVideoRecordingState';

interface PreviewFramePayload {
  width: number;
  height: number;
  bgra: number[];
}

export function VideoPreviewOverlay() {
  const state = useVideoRecordingState();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [position, setPosition] = useState({ x: 20, y: 20 });
  const draggingRef = useRef(false);
  const dragOffsetRef = useRef({ x: 0, y: 0 });
  const positionRef = useRef(position);
  positionRef.current = position;

  useEffect(() => {
    if (!state.is_recording) {
      const canvas = canvasRef.current;
      if (canvas) {
        const ctx = canvas.getContext('2d');
        if (ctx) ctx.clearRect(0, 0, canvas.width, canvas.height);
      }
      return;
    }

    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    (async () => {
      try {
        unlisten = await listen<PreviewFramePayload>('video-preview-frame', (event) => {
          if (cancelled) return;
          const { width, height, bgra } = event.payload;
          const canvas = canvasRef.current;
          if (!canvas) return;
          const ctx = canvas.getContext('2d');
          if (!ctx) return;
          const clamped = new Uint8ClampedArray(bgra);
          const imageData = new ImageData(clamped, width, height);
          ctx.putImageData(imageData, 0, 0);
        });
      } catch (e) {
        console.warn('VideoPreviewOverlay: failed to subscribe to video-preview-frame', e);
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [state.is_recording]);

  useEffect(() => {
    if (!state.is_recording) return;
    const onMove = (e: MouseEvent) => {
      if (!draggingRef.current) return;
      setPosition({
        x: e.clientX - dragOffsetRef.current.x,
        y: e.clientY - dragOffsetRef.current.y,
      });
    };
    const onUp = () => {
      draggingRef.current = false;
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
  }, [state.is_recording]);

  if (!state.is_recording) return null;

  return (
    <div
      style={{ left: position.x, top: position.y }}
      className="fixed z-40 bg-black rounded-lg shadow-lg p-2 cursor-move"
      onMouseDown={(e) => {
        draggingRef.current = true;
        dragOffsetRef.current = { x: e.clientX - positionRef.current.x, y: e.clientY - positionRef.current.y };
      }}
    >
      <canvas ref={canvasRef} width={240} height={180} className="rounded" />
      <p className="text-white text-xs text-center mt-1">Recording video (live preview)</p>
    </div>
  );
}
