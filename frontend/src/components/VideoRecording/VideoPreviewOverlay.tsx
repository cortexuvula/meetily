import { useEffect, useRef, useState } from 'react';
import { useVideoRecordingState } from './useVideoRecordingState';

const PREVIEW_FPS = 5;

export function VideoPreviewOverlay() {
  const state = useVideoRecordingState();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [position, setPosition] = useState({ x: 20, y: 20 });
  const [dragging, setDragging] = useState(false);
  const dragOffset = useRef({ x: 0, y: 0 });
  const streamRef = useRef<MediaStream | null>(null);

  useEffect(() => {
    if (!state.is_recording) {
      streamRef.current?.getTracks().forEach((t) => t.stop());
      streamRef.current = null;
      return;
    }

    let cancelled = false;
    const ctx = canvasRef.current?.getContext('2d');

    (async () => {
      try {
        const display = await (navigator.mediaDevices as unknown as {
          getDisplayMedia: (c: MediaStreamConstraints) => Promise<MediaStream>;
        }).getDisplayMedia({ video: true, audio: false });
        const camera = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
        if (cancelled) {
          display.getTracks().forEach((t) => t.stop());
          camera.getTracks().forEach((t) => t.stop());
          return;
        }
        streamRef.current = new MediaStream([
          ...display.getVideoTracks(),
          ...camera.getVideoTracks(),
        ]);
        const video = document.createElement('video');
        video.srcObject = streamRef.current;
        video.muted = true;
        video.play();
        const interval = window.setInterval(() => {
          if (!ctx || !canvasRef.current) return;
          if (video.readyState >= 2) {
            ctx.drawImage(video, 0, 0, canvasRef.current.width, canvasRef.current.height);
          }
        }, 1000 / PREVIEW_FPS);
        (streamRef.current as unknown as { _intervalId?: number })._intervalId = interval;
      } catch (e) {
        console.warn('VideoPreviewOverlay: could not start preview streams', e);
      }
    })();

    return () => {
      cancelled = true;
      if (streamRef.current) {
        const id = (streamRef.current as unknown as { _intervalId?: number })._intervalId;
        if (id) window.clearInterval(id);
        streamRef.current.getTracks().forEach((t) => t.stop());
        streamRef.current = null;
      }
    };
  }, [state.is_recording]);

  if (!state.is_recording) return null;

  return (
    <div
      style={{ left: position.x, top: position.y }}
      className="fixed z-40 bg-black rounded-lg shadow-lg p-2 cursor-move"
      onMouseDown={(e) => {
        setDragging(true);
        dragOffset.current = { x: e.clientX - position.x, y: e.clientY - position.y };
      }}
      onMouseUp={() => setDragging(false)}
      onMouseLeave={() => setDragging(false)}
      onMouseMove={(e) => {
        if (!dragging) return;
        setPosition({ x: e.clientX - dragOffset.current.x, y: e.clientY - dragOffset.current.y });
      }}
    >
      <canvas ref={canvasRef} width={240} height={180} className="rounded" />
      <p className="text-white text-xs text-center mt-1">Recording video (preview)</p>
    </div>
  );
}
