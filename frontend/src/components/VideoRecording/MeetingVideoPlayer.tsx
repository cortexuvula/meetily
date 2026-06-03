import { useEffect, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';

interface MeetingVideoPlayerProps {
  meetingFolder: string;
}

export function MeetingVideoPlayer({ meetingFolder }: MeetingVideoPlayerProps) {
  const [exists, setExists] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { exists } = await import('@tauri-apps/plugin-fs');
        const path = `${meetingFolder}/video.mp4`;
        const result = await exists(path);
        if (!cancelled) setExists(result);
      } catch (e) {
        if (!cancelled) setExists(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [meetingFolder]);

  if (exists !== true) return null;
  const src = convertFileSrc(`${meetingFolder}/video.mp4`);
  return (
    <div className="mt-4">
      <h3 className="text-sm font-semibold mb-2">Recording</h3>
      <video src={src} controls className="w-full rounded-lg" />
    </div>
  );
}
