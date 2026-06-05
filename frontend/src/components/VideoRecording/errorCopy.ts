export interface ErrorCopy {
  title: string;
  hint: string;
}

function toMessage(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === 'object' && 'message' in error) {
    return String((error as { message: unknown }).message);
  }
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

export function videoErrorCopy(error: unknown): ErrorCopy {
  const message = toMessage(error);

  if (message.includes('No camera detected')) {
    return { title: 'No camera detected', hint: 'Connect a webcam and try again.' };
  }
  if (message.includes('Camera permission denied')) {
    return {
      title: 'Camera permission denied',
      hint: 'Open System Settings → Privacy & Security → Camera and grant access to Meetily.',
    };
  }
  if (message.includes('Screen recording permission denied')) {
    return {
      title: 'Screen recording permission denied',
      hint: 'Open System Settings → Privacy & Security → Screen Recording and grant access to Meetily.',
    };
  }
  if (message.includes('No screen available')) {
    return { title: 'No screen available', hint: 'Connect a display and try again.' };
  }
  if (message.includes('Multiple screens detected') || message.includes('Multiple cameras detected')) {
    return {
      title: 'Multiple sources detected',
      hint: 'Pick which one to record and try again.',
    };
  }
  if (message.includes('FFmpeg process exited')) {
    // The Rust side formats as "FFmpeg process exited with code {code}: {message}"
    // where {message} is the truncated stderr tail. Surface the last
    // 200 chars of that in the hint so the user has something to act on.
    const colonIdx = message.indexOf(': ');
    const detail = colonIdx >= 0 ? message.substring(colonIdx + 2).trim() : '';
    if (detail) {
      const truncated = detail.length > 200 ? detail.substring(0, 200) + '…' : detail;
      return {
        title: 'Video encoder error',
        hint: `ffmpeg: ${truncated}`,
      };
    }
    return {
      title: 'Video encoder error',
      hint: 'Restart the app and try again. If it persists, the bundled ffmpeg binary may be missing.',
    };
  }
  if (message.includes('Screen capture stream error')) {
    return { title: 'Screen capture failed', hint: 'Try stopping and starting the recording again.' };
  }
  if (message.includes('Camera capture stream error')) {
    return {
      title: 'Camera capture failed',
      hint: 'Try disconnecting and reconnecting the webcam.',
    };
  }
  if (message.includes('Audio tap error')) {
    return {
      title: 'Audio sync error',
      hint: 'The audio could not be written to the video file. The recording has been stopped.',
    };
  }
  if (message.includes('Failed to write output')) {
    return {
      title: 'Could not save the video file',
      hint: 'Check that the meeting folder is writable and has free disk space.',
    };
  }
  if (message.includes('IO error')) {
    return {
      title: 'Video file system error',
      hint: 'Check disk space, file permissions, and that the meeting folder is reachable.',
    };
  }
  if (message.includes('Video recording is already in progress')) {
    return {
      title: 'Already recording',
      hint: 'A video recording is already in progress.',
    };
  }
  if (message.includes('Video recording is not in progress')) {
    return {
      title: 'Not recording',
      hint: 'No video recording was in progress to stop.',
    };
  }
  return { title: 'Video recording error', hint: 'See the developer console for details.' };
}
