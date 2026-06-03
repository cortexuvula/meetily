import { useVideoRecordingState } from './useVideoRecordingState';
import { videoErrorCopy } from './errorCopy';

export function VideoErrorBanner() {
  const state = useVideoRecordingState();
  if (!state.last_error) return null;
  const { title, hint } = videoErrorCopy(state.last_error);
  return (
    <div className="bg-red-50 border border-red-200 text-red-800 rounded-lg p-3 mb-2">
      <p className="font-semibold text-sm">{title}</p>
      <p className="text-xs mt-1">{hint}</p>
    </div>
  );
}
