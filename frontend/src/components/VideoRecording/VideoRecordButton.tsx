import { useVideoRecordingState } from './useVideoRecordingState';
import { invoke } from '@tauri-apps/api/core';

interface VideoRecordButtonProps {
  meetingId: string;
  savePath: string;
  defaultScreenId?: string;
  defaultCameraId?: string;
  onJitSelection: (kind: 'screen' | 'camera', available: unknown[]) => void;
}

export function VideoRecordButton({
  meetingId,
  savePath,
  defaultScreenId,
  defaultCameraId,
  onJitSelection,
}: VideoRecordButtonProps) {
  const state = useVideoRecordingState();

  const handleClick = async () => {
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
        meetingId,
        savePath,
        screenId: defaultScreenId ?? null,
        cameraId: defaultCameraId ?? null,
      });
    } catch (e: unknown) {
      const msg = typeof e === 'string' ? e : (e as { message?: string })?.message ?? JSON.stringify(e);
      if (msg.includes('Multiple screens detected')) {
        const screens = await invoke<unknown[]>('list_video_screens');
        onJitSelection('screen', screens);
      } else if (msg.includes('Multiple cameras detected')) {
        const cameras = await invoke<unknown[]>('list_video_cameras');
        onJitSelection('camera', cameras);
      } else {
        console.error('start_video_recording failed', e);
      }
    }
  };

  const label = state.is_recording ? 'Stop Video' : 'Record Video';
  const disabled = state.is_starting;

  return (
    <button
      onClick={handleClick}
      disabled={disabled}
      className={`w-full flex items-center justify-center px-3 py-2 text-sm font-medium text-white ${
        state.is_recording ? 'bg-red-500' : 'bg-blue-500 hover:bg-blue-600'
      } ${disabled ? 'opacity-50 cursor-not-allowed' : ''} rounded-lg transition-colors shadow-sm`}
      title={disabled ? 'Starting...' : label}
    >
      {state.is_starting ? 'Starting...' : state.is_stopping ? 'Stopping...' : label}
    </button>
  );
}
