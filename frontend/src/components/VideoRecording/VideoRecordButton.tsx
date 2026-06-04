import { Video } from 'lucide-react';
import { useVideoRecordingState } from './useVideoRecordingState';
import { useVideoClickHandler } from './useVideoClickHandler';

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
  const handleClick = useVideoClickHandler({
    meetingId,
    savePath,
    defaultScreenId,
    defaultCameraId,
    onJitSelection,
  });

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
      <Video className="w-4 h-4 mr-2" />
      {state.is_starting ? 'Starting...' : state.is_stopping ? 'Stopping...' : label}
    </button>
  );
}
