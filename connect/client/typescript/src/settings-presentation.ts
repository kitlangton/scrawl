// Connect owns presentation policy. Desktop supplies the shell and native effects.
export type Action = 'connect' | 'allow' | 'cancel' | 'check' | 'disable' | 'disconnect' |
  'open-browser' | 'request-disable' | 'request-disconnect' | 'open-shared-links';
export type Button = { command: Action; title: string; enabled: boolean };
export type SafeError = 'cancelled' | 'offline' | 'auth-required' | 'rejected' | 'expired' |
  'local-state' | 'invalid-server' | 'unverified-commit' | 'unavailable';
export type PresentationState = {
  snapshot: { connected: boolean; uploadsEnabled: boolean; renewalRequired: boolean;
    reauthorizationRequired: boolean; accountInitial: string | null } | null;
  health: 'online' | 'offline' | 'auth-required' | null;
  error: SafeError | null; localError: SafeError | null;
  consent: boolean; approval: boolean; operation: 'login' | 'probe' | 'disable' | null;
  logout: boolean; retryDisable: boolean | null; confirmation: boolean | null;
  closed: boolean; sharedLinksUrl: string | null;
};
export type Presentation = {
  tone: 'neutral' | 'success' | 'progress' | 'warning' | 'failure';
  headline: string; detail: string; note: string; artwork: boolean; compact: boolean;
  accountMenu: boolean; accountInitial?: string; buttons: Button[];
  menuActions: Action[]; sharedLinksUrl: string | null;
};
const errors: Record<SafeError, [string, string]> = {
  cancelled: ['Connection cancelled.', 'Cancelled. Check the current local upload settings before reconnecting.'],
  offline: ['Connect is offline.', 'Could not reach Connect. Check your connection and try again.'],
  'auth-required': ['Sign in again.', 'Sign in again to authorize this device.'],
  rejected: ['Sign-in wasn’t approved.', 'Sign-in was not approved. You can try again.'],
  expired: ['That sign-in expired.', 'The sign-in code expired. Connect again for a new code.'],
  'local-state': ['Connection settings unavailable.', 'Apparent could not read or save Connect state. Check local storage and try again.'],
  'invalid-server': ['Check your Connect server.', 'The configured Connect server is invalid. Correct it before connecting.'],
  'unverified-commit': ['Connection not confirmed.', 'Sign-in finished, but Apparent could not verify uploads are enabled for this server. Check local state before trying again.'],
  unavailable: ['Connect is unavailable.', 'Connect is unavailable. Try again.'],
};

export function presentConnect(state: PresentationState, buttons: readonly Button[]): Presentation {
  const s = state.snapshot;
  const disabling = state.operation === 'disable';
  const p: Presentation = {
    tone: 'neutral', headline: 'Help others see what you mean.',
    detail: 'Share your capture with people or agents.', note: '', artwork: true,
    compact: false, accountMenu: false, buttons: buttons.map(b => ({ ...b })),
    menuActions: [], sharedLinksUrl: state.sharedLinksUrl,
    ...(s?.accountInitial ? { accountInitial: s.accountInitial } : {}),
  };
  const failure = (headline: string, detail: string) => {
    Object.assign(p, { tone: 'failure', headline, detail, artwork: false, compact: false });
  };
  if (s?.uploadsEnabled) Object.assign(p, { headline: 'You’re connected.', detail: 'New captures upload privately.', tone: 'success', compact: true });
  else if (s?.connected) Object.assign(p, { headline: 'Uploads are off.', detail: 'You’re still signed in. New captures stay local.', artwork: false });
  if (s?.renewalRequired) Object.assign(p, { headline: 'Connection needs a refresh.', detail: "You're still signed in. Check connection to renew this computer's access. Local recording still works.", tone: 'warning', compact: false, artwork: false });
  if (state.health === 'offline') failure('Connect is offline.', 'Connect could not be reached. Local recording still works; your upload permission has not changed.');
  if (state.health === 'auth-required' || s?.reauthorizationRequired) failure('Sign in again.', 'Your authorization expired or was revoked. Local recording still works.');
  if (state.error) {
    failure(...errors[state.error]);
    if (state.error === 'cancelled') p.tone = 'neutral';
  }
  if (state.consent) Object.assign(p, { headline: 'Allow new capture uploads?', detail: 'New captures upload privately, including recordings, images, audio, transcripts, context and other capture assets.', note: 'Older captures stay local. Sharing is a separate action.', tone: 'neutral', artwork: true, compact: false });
  if (state.operation) {
    Object.assign(p, { tone: 'progress', compact: false, artwork: false });
    if (state.operation === 'login' && state.approval) Object.assign(p, { headline: 'Finish connecting in your browser.', detail: 'Sign in and approve this computer.', note: 'Waiting for approval. Uploads haven’t started.', artwork: true });
    else if (state.operation === 'login') Object.assign(p, { headline: 'Getting sign-in ready…', detail: 'Local recording still works.' });
    else if (state.operation === 'probe') Object.assign(p, { headline: 'Checking connection…', detail: 'Local recording still works.' });
    else Object.assign(p, { headline: state.logout ? 'Disconnecting…' : 'Turning uploads off…', detail: 'Saving your choice and cancelling unfinished uploads.' });
  }
  if (state.localError) failure('Connection settings unavailable.', errors[state.localError][1]);
  p.accountMenu = !!(s && (s.connected || s.reauthorizationRequired || s.renewalRequired)) || buttons.some(b => b.command === 'disable' || b.command === 'disconnect');
  if (state.sharedLinksUrl) {
    p.buttons.push({ command: 'open-shared-links', title: 'Manage shared links', enabled: true });
    p.menuActions.push('open-shared-links'); p.accountMenu = true;
  }
  for (const b of p.buttons) {
    if (disabling) continue;
    if (b.command === 'connect') b.title = state.health === 'auth-required' || s?.reauthorizationRequired ? 'Sign in again' : s?.connected ? 'Enable uploads' : 'Connect';
    if (b.command === 'allow') b.title = 'Allow & continue';
    if (b.command === 'cancel') b.title = state.consent ? 'Not now' : 'Cancel';
    if (b.command === 'disable') b.title = state.retryDisable === false ? 'Retry disabling uploads' : 'Turn off uploads';
    if (b.command === 'disconnect') b.title = state.retryDisable === true ? 'Retry disconnecting' : 'Disconnect this computer';
    if (state.retryDisable === null) {
      if (b.command === 'disable') b.command = 'request-disable';
      if (b.command === 'disconnect') b.command = 'request-disconnect';
    }
    if (['check', 'request-disable', 'request-disconnect', 'disable', 'disconnect'].includes(b.command) && !(s?.renewalRequired && b.command === 'check')) p.menuActions.push(b.command);
  }
  if (state.confirmation !== null) {
    const logout = state.confirmation;
    Object.assign(p, { headline: logout ? 'Disconnect this computer?' : 'Turn uploads off?',
      detail: logout ? 'Unfinished uploads stop. Already uploaded captures and shared links are not deleted.' : 'Unfinished uploads stop. You stay signed in, and already uploaded captures remain.',
      note: '', tone: 'warning', artwork: false, compact: false });
    p.buttons = [
      { command: logout ? 'disconnect' : 'disable', title: logout ? 'Disconnect' : 'Turn off uploads', enabled: true },
      { command: 'cancel', title: logout ? 'Keep connected' : 'Keep uploads on', enabled: true },
    ];
    p.menuActions = [];
  }
  p.accountMenu ||= p.menuActions.length > 0;
  if (state.closed) for (const button of p.buttons) button.enabled = false;
  return p;
}
