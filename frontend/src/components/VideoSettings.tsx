'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Save } from 'lucide-react';
import { toast } from 'sonner';

type QualityPreset = 'Low' | 'Medium' | 'High' | 'Custom';
type Resolution = 'P720' | 'P1080' | 'Native';
type PipPosition = 'TopLeft' | 'TopRight' | 'BottomLeft' | 'BottomRight';
type PipSize = 'Small' | 'Medium' | 'Large';

interface VideoPreferences {
  quality: QualityPreset;
  resolution: Resolution;
  fps: number;
  bitrate_kbps: number;
  pip_position: PipPosition;
  pip_size: PipSize;
  default_screen: string | null;
  default_camera: string | null;
}

interface ScreenInfo {
  id: string;
  name: string;
  width: number;
  height: number;
}

interface CameraInfo {
  id: string;
  name: string;
}

const QUALITY_PRESETS: QualityPreset[] = ['Low', 'Medium', 'High', 'Custom'];
const PIP_POSITIONS: PipPosition[] = ['TopLeft', 'TopRight', 'BottomLeft', 'BottomRight'];
const PIP_SIZES: PipSize[] = ['Small', 'Medium', 'Large'];

const PRESET_VALUES: Record<Exclude<QualityPreset, 'Custom'>, { resolution: Resolution; fps: number; bitrate_kbps: number }> = {
  Low: { resolution: 'P720', fps: 24, bitrate_kbps: 2000 },
  Medium: { resolution: 'P1080', fps: 30, bitrate_kbps: 4000 },
  High: { resolution: 'P1080', fps: 30, bitrate_kbps: 8000 },
};

export function VideoSettings() {
  const [prefs, setPrefs] = useState<VideoPreferences | null>(null);
  const [screens, setScreens] = useState<ScreenInfo[]>([]);
  const [cameras, setCameras] = useState<CameraInfo[]>([]);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const [p, s, c] = await Promise.all([
          invoke<VideoPreferences>('get_video_preferences'),
          invoke<ScreenInfo[]>('list_video_screens'),
          invoke<CameraInfo[]>('list_video_cameras'),
        ]);
        setPrefs(p);
        setScreens(s);
        setCameras(c);
      } catch (e) {
        console.error('Failed to load video preferences', e);
        toast.error('Failed to load video preferences');
      }
    })();
  }, []);

  const update = (patch: Partial<VideoPreferences>) => {
    setPrefs((prev) => (prev ? { ...prev, ...patch } : prev));
  };

  const handleQualityChange = (quality: QualityPreset) => {
    if (quality === 'Custom') {
      update({ quality });
      return;
    }
    const p = PRESET_VALUES[quality];
    update({ quality, resolution: p.resolution, fps: p.fps, bitrate_kbps: p.bitrate_kbps });
  };

  const save = async () => {
    if (!prefs) return;
    setSaving(true);
    try {
      await invoke('set_video_preferences', { preferences: prefs });
      toast.success('Video preferences saved');
    } catch (e) {
      console.error('Failed to save video preferences', e);
      toast.error('Failed to save video preferences');
    } finally {
      setSaving(false);
    }
  };

  if (!prefs) {
    return (
      <div className="animate-pulse space-y-4">
        <div className="h-4 bg-gray-200 rounded w-1/4"></div>
        <div className="h-8 bg-gray-200 rounded w-1/2"></div>
        <div className="h-8 bg-gray-200 rounded w-1/3"></div>
      </div>
    );
  }

  const isCustom = prefs.quality === 'Custom';

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-semibold mb-2">Video Settings</h3>
        <p className="text-sm text-gray-600">
          Configure how video recordings are captured during meetings.
        </p>
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">Quality preset</label>
        <div className="flex gap-2">
          {QUALITY_PRESETS.map((q) => (
            <button
              key={q}
              onClick={() => handleQualityChange(q)}
              className={`px-3 py-1.5 text-sm rounded-md border transition-colors ${
                prefs.quality === q
                  ? 'bg-blue-500 text-white border-blue-500'
                  : 'bg-white text-gray-700 border-gray-300 hover:bg-gray-50'
              }`}
            >
              {q}
            </button>
          ))}
        </div>
        <p className="text-xs text-gray-500 mt-1">
          {isCustom
            ? 'Custom mode lets you set FPS and bitrate manually.'
            : 'Selecting a preset locks the resolution, FPS, and bitrate to recommended values.'}
        </p>
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">Resolution</label>
        <select
          value={prefs.resolution}
          onChange={(e) => update({ resolution: e.target.value as Resolution })}
          className="border border-gray-300 rounded-md px-3 py-1.5 text-sm bg-white"
        >
          <option value="P720">720p (1280×720)</option>
          <option value="P1080">1080p (1920×1080)</option>
          <option value="Native">Native (capped at 1080p)</option>
        </select>
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">FPS</label>
        <select
          value={prefs.fps}
          onChange={(e) => update({ fps: parseInt(e.target.value, 10) })}
          disabled={!isCustom}
          className="border border-gray-300 rounded-md px-3 py-1.5 text-sm bg-white disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <option value="24">24</option>
          <option value="30">30</option>
          <option value="60">60</option>
        </select>
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">Bitrate (kbps)</label>
        <input
          type="number"
          min={500}
          max={50000}
          step={500}
          value={prefs.bitrate_kbps}
          onChange={(e) => update({ bitrate_kbps: parseInt(e.target.value, 10) || 4000 })}
          disabled={!isCustom}
          className="border border-gray-300 rounded-md px-3 py-1.5 text-sm w-32 disabled:opacity-50 disabled:cursor-not-allowed"
        />
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">Picture-in-picture position</label>
        <div className="flex gap-2">
          {PIP_POSITIONS.map((p) => (
            <button
              key={p}
              onClick={() => update({ pip_position: p })}
              className={`px-3 py-1.5 text-xs rounded-md border transition-colors ${
                prefs.pip_position === p
                  ? 'bg-blue-500 text-white border-blue-500'
                  : 'bg-white text-gray-700 border-gray-300 hover:bg-gray-50'
              }`}
            >
              {p}
            </button>
          ))}
        </div>
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">Picture-in-picture size</label>
        <div className="flex gap-2">
          {PIP_SIZES.map((s) => (
            <button
              key={s}
              onClick={() => update({ pip_size: s })}
              className={`px-3 py-1.5 text-xs rounded-md border transition-colors ${
                prefs.pip_size === s
                  ? 'bg-blue-500 text-white border-blue-500'
                  : 'bg-white text-gray-700 border-gray-300 hover:bg-gray-50'
              }`}
            >
              {s}
            </button>
          ))}
        </div>
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">Default camera</label>
        <select
          value={prefs.default_camera ?? ''}
          onChange={(e) => update({ default_camera: e.target.value || null })}
          className="border border-gray-300 rounded-md px-3 py-1.5 text-sm bg-white w-80"
        >
          <option value="">Prompt each time</option>
          {cameras.map((c) => (
            <option key={c.id} value={c.id}>{c.name}</option>
          ))}
        </select>
      </div>

      <div>
        <label className="block text-sm font-medium mb-2">Default screen</label>
        <select
          value={prefs.default_screen ?? ''}
          onChange={(e) => update({ default_screen: e.target.value || null })}
          className="border border-gray-300 rounded-md px-3 py-1.5 text-sm bg-white w-80"
        >
          <option value="">Prompt each time</option>
          {screens.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name} ({s.width}×{s.height})
            </option>
          ))}
        </select>
      </div>

      <div className="pt-4 border-t flex items-center gap-2">
        <button
          onClick={save}
          disabled={saving}
          className="flex items-center gap-2 px-4 py-2 bg-blue-500 text-white text-sm rounded-md hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Save className="w-4 h-4" />
          {saving ? 'Saving…' : 'Save'}
        </button>
      </div>
    </div>
  );
}
