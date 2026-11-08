import { useState } from 'react';
import { Button } from '@apparent/ui/button';
import './gesture-guide.css';

function recordKeys(shortcut: string): string[] {
  const mac = shortcut.match(/^([⌃⌥⇧⌘]+)(.+)$/u);
  if (mac) return [...Array.from(mac[1]), mac[2]];
  const modifiers: string[] = [];
  let key = shortcut.trim(), match;
  while ((match = key.match(/^(Ctrl|Control|Alt|Option|Shift|Super|Meta|Command|Cmd)\s*\+\s*/i))) {
    modifiers.push(match[1]);
    key = key.slice(match[0].length);
  }
  // After modifier separators, the remaining '+' is a literal key.
  return modifiers.length && key ? [...modifiers, key] : [shortcut];
}

/** Presentation only: these illustrations never send keys, draw, or advance setup. */
export function GestureGuide({ kind, shortcut, platform }: {
  kind: 'record' | 'draw' | 'note' | 'paste'; shortcut: string; platform: 'macos' | 'linux';
}) {
  const [replay, setReplay] = useState(0);
  if (kind === 'record' && !shortcut.trim()) return null;
  const label = kind === 'note' ? 'Hold Shift and click a spot' : kind === 'draw' ? 'Hold Shift and move your pointer to draw' :
    kind === 'paste' ? `Paste with ${platform === 'linux' ? 'Control' : 'Command'} V` : `Record with ${shortcut}`;
  const keys = kind === 'paste' ? [platform === 'linux' ? 'Ctrl' : '⌘', 'V'] :
    recordKeys(shortcut);
  return <figure className="native-gesture" aria-label={label} data-kind={kind}>
    <div key={replay} className="native-gesture-motion" aria-hidden="true">
      {kind === 'note' ? <>
        <kbd className="native-gesture-modifier">⇧ Shift</kbd>
        <svg viewBox="0 0 156 56" className="native-gesture-draw">
          <circle className="native-note-click" cx="70" cy="27" r="13" />
          <g className="native-note-cursor"><path d="M0 0V18L5 13 10 22 13 20 8 11H16Z" /></g>
          <text className="native-note-label" x="105" y="34">Click</text>
        </svg>
      </> : kind === 'draw' ? <>
        <kbd className="native-gesture-modifier">⇧ Shift</kbd>
        <svg viewBox="0 0 156 56" className="native-gesture-draw">
          <path className="native-gesture-ink" pathLength="1" d="M12 44C42 5 112 5 142 44" />
          <g className="native-gesture-cursor"><path d="M0 0V18L5 13 10 22 13 20 8 11H16Z" /></g>
        </svg>
      </> : <div className="native-gesture-keys">{keys.map((key, index) =>
        <kbd key={index} data-last={index === keys.length - 1}>{key}</kbd>)}</div>}
    </div>
    <Button variant="ghost" size="icon" className="native-gesture-replay" aria-label={`Replay ${kind === 'note' ? 'note' : kind === 'draw' ? 'drawing' : kind === 'paste' ? 'paste' : 'shortcut'} guide`} onClick={() => setReplay(value => value + 1)}>↻</Button>
  </figure>;
}
