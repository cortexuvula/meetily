import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useVideoRecordingState } from './useVideoRecordingState';

interface UseVideoClickHandlerOpts {
  meetingId: string;
  savePath: string;
  defaultScreenId?: string;
  defaultCameraId?: string;
  onJitSelection: (kind: 'screen' | 'camera', available: unknown[]) => void;
}

export function useVideoClickHandler(opts: UseVideoClickHandlerOpts) {
  const state = useVideoRecordingState();

  return useCallback(async () => {
    if (state.is_recording || state.is_stopping) {
      try {
        await invoke('stop_video_recording');
      } catch (e) {
        console.error('stop_video_recording failed', e);
      }
      return;
    }
    try {
      await invoke('start_video_recording', {
        meetingId: opts.meetingId,
        savePath: opts.savePath,
        screenId: opts.defaultScreenId ?? null,
        cameraId: opts.defaultCameraId ?? null,
      });
    } catch (e: unknown) {
      const msg = typeof e === 'string' ? e : (e as { message?: string })?.message ?? JSON.stringify(e);
      if (msg.includes('Multiple screens detected')) {
        const screens = await invoke<unknown[]>('list_video_screens');
        opts.onJitSelection('screen', screens);
      } else if (msg.includes('Multiple cameras detected')) {
        const cameras = await invoke<unknown[]>('list_video_cameras');
        opts.onJitSelection('camera', cameras);
      } else {
        console.error('start_video_recording failed', e);
      }
    }
  }, [state.is_recording, state.is_stopping, opts]);
}
