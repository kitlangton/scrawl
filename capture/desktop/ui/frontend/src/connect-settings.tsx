import { useEffect, useRef, useState } from 'react';
import { Button } from '@apparent/ui/button';
import { StatusCard } from '@apparent/ui/status-card';
import { DisclosureMenu } from '@apparent/ui/disclosure-menu';
import { UserRound } from 'lucide-react';
import '@apparent/ui/status.css';
import frogs from '../../../../../shared/assets/brand/connect-frogs.png';
import font from '../../../../../shared/assets/fonts/HostGrotesk-Variable.ttf?url';
import type { ConnectAction, ConnectSettings, ConnectView } from './bridge';

export function ConnectSettingsCard() {
  const [result, setResult] = useState<ConnectSettings>({ status: 'disabled' });
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const polling = useRef(false);
  async function request(action?: ConnectAction) {
    const current = ++generation.current;
    setBusy(true);
    try {
      const bridge = window.apparent;
      const handlers = bridge && {
        connect: bridge.connectSettings, allow: bridge.allowConnectUploads, cancel: bridge.cancelConnectSettings,
        check: bridge.checkConnectConnection, disable: bridge.disableConnectUploads,
        disconnect: bridge.disconnectConnectSettings, 'open-browser': bridge.openConnectBrowser,
        'request-disable': bridge.requestDisableConnectUploads,
        'request-disconnect': bridge.requestDisconnectConnectSettings,
        'open-shared-links': bridge.openConnectSharedLinks,
      };
      const next = !bridge ? { status: 'disconnected' } as const : action
        ? await handlers![action]() : await bridge.readConnectSettings();
      if (current === generation.current) setResult(next);
    } catch {
      if (current === generation.current) setResult({ status: 'error' });
    } finally {
      if (current === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    void request();
    return () => { generation.current += 1; };
  }, []);
  useEffect(() => {
    if (result.status !== 'connected') return;
    // Local-only snapshots renew this window's controller lease and collect
    // background receipts. They never perform a health probe or authorize login.
    const timer = setInterval(() => {
      if (busy || polling.current) return;
      polling.current = true;
      void request().finally(() => { polling.current = false; });
    }, 1000);
    return () => clearInterval(timer);
  }, [result.status, busy]);

  if (result.status === 'disabled') return null;
  const view = result.status === 'connected' ? result.view : undefined;
  const p = view?.presentation;
  const buttons = p?.buttons ?? view?.buttons ?? [];
  const menu = new Set(p?.menuActions ?? []);
  const controls = buttons.filter(button => menu.has(button.command));
  const cardButtons = buttons.filter(button => !menu.has(button.command));
  function renderButton(button: typeof buttons[number], close?: () => void) {
    const destructive = button.command === 'disconnect' || button.command === 'request-disconnect';
    return <Button key={button.command} variant="ghost"
      className={close ? destructive ? 'disconnect-action' : undefined
        : destructive ? 'destructive-action' : button.command === 'cancel' ? 'secondary-action' : 'primary-action'}
      disabled={busy || !button.enabled} onClick={() => { close?.(); void request(button.command); }}>{button.title}</Button>;
  }
  return <section aria-label="Apparent Connect" className="apparent-connect-workflow">
    <style>{`@font-face { font-family: 'Connect Host Grotesk'; src: url('${font}'); }`}</style>
    {p?.accountMenu && <div className="flex justify-end pb-4">
      <DisclosureMenu label="Account and connection options" trigger={p?.accountInitial ?? <UserRound size={18} aria-hidden="true" />}>
        {close => <>
          <p className="menu-context">This computer{view?.server && <><br />{view.server}</>}</p>
          {controls.map(button => renderButton(button, close))}
          <p className="menu-context">Disconnecting does not delete uploaded captures.</p>
        </>}
      </DisclosureMenu>
    </div>}
    <StatusCard tone={p?.tone ?? (view ? 'neutral' : busy ? 'progress' : 'failure')}
      headline={p?.headline ?? view?.status ?? (busy ? 'Reading connection settings…' : 'Connection unavailable.')}
      compact={p?.compact} artwork={p?.artwork ? <img src={frogs} alt="" /> : undefined}>
      <p>{p?.detail ?? view?.detail ?? 'Open the current Apparent app, then refresh. Local recording remains available.'}</p>
      {p?.note && <p className="status-note">{p.note}</p>}
      {cardButtons.length > 0 && <div className="status-actions">{cardButtons.map(button => renderButton(button))}</div>}
      {!view && <Button className="primary-action" disabled={busy} onClick={() => void request()}>Refresh connection</Button>}
    </StatusCard>
  </section>;
}
