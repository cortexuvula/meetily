import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface VideoRecordingStateDto {
  is_recording: boolean;
  is_starting: boolean;
  is_stopping: boolean;
  is_paused: boolean;
  current_meeting_id: string | null;
  final_path: string | null;
  last_error: string | null;
}

const EMPTY: VideoRecordingStateDto = {
  is_recording: false,
  is_starting: false,
  is_stopping: false,
  is_paused: false,
  current_meeting_id: null,
  final_path: null,
  last_error: null,
};

export function useVideoRecordingState() {
  if (typeof window !== 'undefined') {
    console.log('[useVideoRecordingState] hook called');
  }
  const [state, setState] = useState<VideoRecordingStateDto>(EMPTY);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    (async () => {
      try {
        const initial = await invoke<VideoRecordingStateDto>('get_video_recording_state');
        if (!cancelled) setState(initial);
      } catch (e) {
        console.error('useVideoRecordingState: initial fetch failed', e);
      }

      try {
        unlisten = await listen<VideoRecordingStateDto>('video-state-changed', (event) => {
          if (!cancelled) setState(event.payload);
        });
      } catch (e) {
        console.error('useVideoRecordingState: listen failed', e);
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return state;
}
