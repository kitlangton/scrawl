import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Storage } from './storage';
import { ConnectSettingsCard } from './connect-settings';
import './style.css';
import wordmark from '../../../../../shared/assets/brand/apparent-wordmark.svg';

const panes = ['General', 'Recording', 'Storage', 'Permissions', 'Agents', 'About'] as const;

function Settings() {
  const connect = new URLSearchParams(location.search).get('surface') === 'connect';
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
      <div className="settings-brand"><img src={wordmark} alt="Apparent" /></div>
      <nav>{panes.map(pane => <button key={pane} type="button" disabled={navigating}
        aria-current={!connect && pane === 'Storage' ? 'page' : undefined}
        onClick={() => { if (connect || pane !== 'Storage') void navigate(pane); }}>{pane}</button>)}</nav>
    </aside>
    <main className="settings-content">
      <header><h1>{connect ? 'Connect' : 'Storage'}</h1>{!connect && <p>Choose where captures live and how long to keep them.</p>}</header>
      {error && <p role="alert">{error}</p>}
      <>{connect ? <ConnectSettingsCard /> : <Storage settings />}</>
    </main>
  </div>;
}

createRoot(document.getElementById('root')!).render(<Settings />);
