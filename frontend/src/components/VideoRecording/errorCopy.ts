export interface ErrorCopy {
  title: string;
  hint: string;
}

export function videoErrorCopy(error: string): ErrorCopy {
  if (error.includes('No camera detected')) {
    return { title: 'No camera detected', hint: 'Connect a webcam and try again.' };
  }
  if (error.includes('Camera permission denied')) {
    return {
      title: 'Camera permission denied',
      hint: 'Open System Settings → Privacy & Security → Camera and grant access to Meetily.',
    };
  }
  if (error.includes('Screen recording permission denied')) {
    return {
      title: 'Screen recording permission denied',
      hint: 'Open System Settings → Privacy & Security → Screen Recording and grant access to Meetily.',
    };
  }
  if (error.includes('No screen available')) {
    return { title: 'No screen available', hint: 'Connect a display and try again.' };
  }
  if (error.includes('FFmpeg process exited')) {
    return {
      title: 'Video encoder error',
      hint: 'Restart the app and try again. If it persists, the bundled ffmpeg binary may be missing.',
    };
  }
  if (error.includes('Screen capture stream error')) {
    return { title: 'Screen capture failed', hint: 'Try stopping and starting the recording again.' };
  }
  if (error.includes('Camera capture stream error')) {
    return {
      title: 'Camera capture failed',
      hint: 'Try disconnecting and reconnecting the webcam.',
    };
  }
  if (error.includes('Audio tap error')) {
    return {
      title: 'Audio sync error',
      hint: 'The audio could not be written to the video file. The recording has been stopped.',
    };
  }
  if (error.includes('Failed to write output')) {
    return {
      title: 'Could not save the video file',
      hint: 'Check that the meeting folder is writable and has free disk space.',
    };
  }
  return { title: 'Video recording error', hint: 'See the developer console for details.' };
}
