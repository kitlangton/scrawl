import { useEffect, useLayoutEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { About } from './about/about';
import { Storage } from './storage';
import './style.css';
import wordmark from '../../../../../shared/assets/brand/apparent-wordmark.svg';

const panes = ['General', 'Recording', 'Storage', 'Permissions', 'Agents', 'About'] as const;
// Match the native sidebar symbols without adding a renderer dependency.
const icons = [
  <><path d="M12 8v8M8 12h8" /><circle cx="12" cy="12" r="9" /></>,
  <><circle cx="12" cy="12" r="9" /><circle cx="12" cy="12" r="3" /></>,
  <><path d="m5 4-3 9v7h20v-7l-3-9ZM2 13h20M6 17h.01M10 17h.01" /></>,
  <><path d="m12 2 9 4v6c0 5-9 10-9 10S3 17 3 12V6ZM8 12l3 3 5-6" /></>,
  <><path d="m4 5 6 6-6 6M13 19h7" /></>,
  <><circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7h.01" /></>,
];

declare global {
  interface Window {
    apparentSettingsMounted?: () => Promise<void>;
    apparentSettingsPane?: (callback: (pane: 'storage' | 'about') => void) => () => void;
  }
}

function Settings() {
  const [current, setCurrent] = useState<'Storage' | 'About'>(() => location.hash === '#about' ? 'About' : 'Storage');
  useLayoutEffect(() => {
    // React's DOM is committed here. The preload owns the first-frame wait;
    // a hidden page's animation scheduling must not hold up this acknowledgement.
    void window.apparentSettingsMounted?.().catch(() => {});
  }, []);
  useEffect(() => window.apparentSettingsPane?.(pane => {
    history.replaceState(null, '', pane === 'about' ? '#about' : location.pathname);
    setCurrent(pane === 'about' ? 'About' : 'Storage');
  }), []);
  const [error, setError] = useState('');
  const [navigating, setNavigating] = useState(false);
  async function navigate(pane: string) {
    setNavigating(true);
    try {
      await window.apparent?.navigateSettings(pane.toLowerCase());
    } catch { setError('Unable to open that Settings page. Please try again.'); }
    finally { setNavigating(false); }
  }
  return <div className="settings-shell">
    <aside className="settings-sidebar" aria-label="Settings pages">
      <div className="settings-brand"><img src={wordmark} alt="Apparent" /><p>CAPTURE SETTINGS</p></div>
      <nav>{panes.map((pane, index) => <button key={pane} type="button" disabled={navigating}
        aria-current={pane === current ? 'page' : undefined}
        onClick={() => { if (pane !== current) void navigate(pane); }}>
        <svg aria-hidden="true" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">{icons[index]}</svg>{pane}</button>)}</nav>
    </aside>
    <main className="settings-content">
      <header><h1>{current}</h1>{current === 'Storage' && <p>Choose where captures live and how long to keep them.</p>}</header>
      {error && <p role="alert">{error}</p>}
      {current === 'About' ? <About /> : <Storage settings />}
    </main>
  </div>;
}

createRoot(document.getElementById('root')!).render(<Settings />);
