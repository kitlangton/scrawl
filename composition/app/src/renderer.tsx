import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Button } from '@apparent/ui/button';
import { parseStreamManifest, type StreamModule, type StreamManifest } from '@apparent/app-contracts';

type Loaded = { manifest: StreamManifest; module: StreamModule };
class Boundary extends React.Component<React.PropsWithChildren, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() { return this.state.failed ? <p role="alert">This section could not open.</p> : this.props.children; }
}
function Shell({ streams }: { streams: Loaded[] }) {
  const [selected, select] = React.useState(streams[0]?.manifest.id);
  const [settings, showSettings] = React.useState(false);
  const active = streams.find(stream => stream.manifest.id === selected);
  return <>
    <header><h1>Apparent Foundation</h1><p>Synthetic modules · no recording or account access</p></header>
    <nav aria-label="Streams">{streams.map(({ manifest }) =>
      <Button key={manifest.id} aria-pressed={!settings && selected === manifest.id}
        onClick={() => { select(manifest.id); showSettings(false); }}>{manifest.label}</Button>)}
      <Button aria-pressed={settings} onClick={() => showSettings(true)}>Settings</Button>
    </nav>
    <main>{settings ? <><h2>Settings</h2>{streams.map(({ manifest, module }) =>
      <section key={manifest.id} aria-label={`${manifest.label} settings`}>
        <h3>{manifest.label}</h3><Boundary><module.Settings /></Boundary>
      </section>)}</> : active ? <Boundary key={active.manifest.id}><active.module.Screen /></Boundary> : <p>No streams installed.</p>}</main>
  </>;
}
async function start() {
  // Registry is generated from pinned artifacts, not discovered from user input.
  const registry = await import('./registry.js');
  const streams: Loaded[] = await Promise.all(registry.streams.map(async (input: unknown) => {
    const manifest = parseStreamManifest(input);
    const entry = `./streams/${manifest.id}/index.js`;
    const module = (await import(entry)).default as StreamModule;
    if (!module || module.id !== manifest.id || typeof module.Screen !== 'function' || typeof module.Settings !== 'function') {
      throw new Error('Invalid stream exports');
    }
    return { manifest, module };
  }));
  createRoot(document.getElementById('root')!).render(<Shell streams={streams} />);
}
start().catch(error => {
  console.error('Foundation startup failed', error);
  document.getElementById('root')!.textContent = 'The application components could not load.';
});
