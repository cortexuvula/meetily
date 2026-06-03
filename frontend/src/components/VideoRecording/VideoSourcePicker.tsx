import { useState } from 'react';

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

export interface SelectionResult {
  kind: 'screen' | 'camera';
  selectedId: string;
}

interface VideoSourcePickerProps {
  kind: 'screen' | 'camera';
  available: unknown[];
  onSelect: (result: SelectionResult) => void;
  onCancel: () => void;
}

export function VideoSourcePicker({ kind, available, onSelect, onCancel }: VideoSourcePickerProps) {
  const [selected, setSelected] = useState<string | null>(null);

  const label = kind === 'screen' ? 'Pick a screen to record' : 'Pick a camera to record';
  const items = (kind === 'screen' ? (available as ScreenInfo[]) : (available as CameraInfo[]));
  const idKey = 'id' as const;
  const nameKey = 'name' as const;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg p-6 w-96 max-w-full">
        <h2 className="text-lg font-semibold mb-4">{label}</h2>
        <ul className="max-h-80 overflow-y-auto">
          {items.map((item) => (
            <li key={item[idKey]}>
              <button
                className={`w-full text-left px-3 py-2 rounded ${
                  selected === item[idKey] ? 'bg-blue-100' : 'hover:bg-gray-100'
                }`}
                onClick={() => setSelected(item[idKey])}
              >
                {item[nameKey]}
                {kind === 'screen' && ` — ${(item as ScreenInfo).width}×${(item as ScreenInfo).height}`}
              </button>
            </li>
          ))}
        </ul>
        <div className="flex justify-end gap-2 mt-4">
          <button className="px-4 py-2 text-sm" onClick={onCancel}>
            Cancel
          </button>
          <button
            className="px-4 py-2 text-sm bg-blue-500 text-white rounded disabled:opacity-50"
            disabled={!selected}
            onClick={() => onSelect({ kind, selectedId: selected! })}
          >
            Start Recording
          </button>
        </div>
      </div>
    </div>
  );
}
