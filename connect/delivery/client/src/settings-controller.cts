import type { Action, Button, PresentationState, SafeError } from './settings-presentation.js';

export type Snapshot = NonNullable<PresentationState['snapshot']> & { server: string | null };
export type Approval = { verificationUri: string; userCode: string };
export type Health = NonNullable<PresentationState['health']>;
export type LocalState = { snapshot: Snapshot | null; error: SafeError | null; sharedLinksUrl: string | null };
// Temporary backend: Rust executes auth and durable mutations through its sole
// writer. No credential, config path, or generic mutation reaches this interface.
export interface AccountBackend {
  read(): Promise<LocalState>;
  login(server: string, signal: AbortSignal, approval: (value: Approval) => void): Promise<SafeError | null>;
  probe(signal: AbortSignal): Promise<Health>;
  disable(logout: boolean): Promise<SafeError | null>;
  activate(): Promise<boolean>;
}
export type View = {
  title: string; status: string; detail: string; note: string;
  server: string | null; verificationUri: string | null; userCode: string | null;
  buttons: Button[]; presentationState: PresentationState;
};
type Operation = 'login' | 'probe' | 'disable';
type Request = { operation: Operation; server: string; logout: boolean; abort: AbortController };
type Receipt = { request: Request } & (
  { kind: 'approval'; approval: Approval } | { kind: 'login' | 'disable'; error: SafeError | null } |
  { kind: 'probe'; health: Health }
);
const DEFAULT_SERVER = 'https://hosted.tryapparent.com';

export class AccountController {
  private snapshot: Snapshot | null = null;
  private localError: SafeError | null = null;
  private error: SafeError | null = null;
  private health: Health | null = null;
  private consent = false;
  private approval: Approval | null = null;
  private pending: Request | null = null;
  private receipts: Receipt[] = [];
  private closed = false;
  private retryDisable: boolean | null = null;
  private confirmation: boolean | null = null;
  private sharedLinksUrl: string | null = null;
  private revision = 0;
  private afterDisableError: SafeError | null = null;

  constructor(private backend: AccountBackend) {}

  private async refresh(): Promise<void> {
    const revision = this.revision;
    const local = await this.backend.read();
    if (revision !== this.revision) return;
    if (JSON.stringify(this.snapshot) !== JSON.stringify(local.snapshot) || local.error) this.health = null;
    this.snapshot = local.error ? null : local.snapshot;
    this.localError = local.error;
    this.sharedLinksUrl = local.sharedLinksUrl;
  }

  private cancel(): void {
    this.revision += 1;
    this.pending?.abort.abort();
    this.pending = null;
    this.approval = null;
    this.consent = false;
  }

  private get disabling(): boolean { return this.pending?.operation === 'disable'; }

  private start(operation: Operation, server = '', logout = false): void {
    this.cancel();
    this.error = null;
    this.health = null;
    const request: Request = { operation, server, logout, abort: new AbortController() };
    this.pending = request;
    if (operation === 'login') {
      void this.backend.login(server, request.abort.signal, approval => {
        if (!request.abort.signal.aborted) this.receipts.push({ request, kind: 'approval', approval });
      }).then(error => this.receipts.push({ request, kind: 'login', error }),
        () => this.receipts.push({ request, kind: 'login', error: 'unavailable' }));
    } else if (operation === 'probe') {
      void this.backend.probe(request.abort.signal).then(health => this.receipts.push({ request, kind: 'probe', health }),
        () => this.receipts.push({ request, kind: 'probe', health: 'offline' }));
    } else {
      // Revocation settles even if its window closes. It deliberately has no
      // cancellation signal, matching the existing durable transaction contract.
      void this.backend.disable(logout).then(error => this.receipts.push({ request, kind: 'disable', error }),
        () => this.receipts.push({ request, kind: 'disable', error: 'local-state' }));
    }
  }

  close(): void {
    if (this.closed) return;
    if (this.pending?.operation === 'login') this.start('disable');
    this.closed = true;
    this.approval = null;
    this.consent = false;
    if (!this.disabling) this.cancel();
  }

  private async command(action: Action): Promise<string | undefined> {
    if (this.closed || this.disabling) return;
    if (action === 'request-disable' || action === 'request-disconnect') {
      this.confirmation = action === 'request-disconnect'; return;
    }
    if (action === 'cancel' && this.confirmation !== null) { this.confirmation = null; return; }
    if (action !== 'cancel') this.confirmation = null;
    if (action === 'open-shared-links') return this.sharedLinksUrl ?? undefined;
    if (action === 'open-browser') return this.pending?.operation === 'login' && !this.pending.abort.signal.aborted
      ? this.approval?.verificationUri : undefined;
    if (action === 'disable' || action === 'disconnect') {
      this.retryDisable = action === 'disconnect';
      this.start('disable', '', this.retryDisable); return;
    }
    if (action === 'cancel' && this.consent) { this.cancel(); await this.refresh(); return; }
    if (action === 'cancel') {
      if (this.pending?.operation === 'login') { this.retryDisable = false; this.start('disable'); return; }
      this.cancel(); this.error = 'cancelled'; await this.refresh(); return;
    }
    if (this.pending) return;
    if (action === 'connect') {
      await this.refresh();
      if (!this.closed && this.snapshot && this.retryDisable === null) { this.error = null; this.consent = true; }
    } else if (action === 'allow' && this.consent && this.retryDisable === null) {
      await this.refresh();
      if (!this.closed && this.snapshot) this.start('login', this.snapshot.server ?? DEFAULT_SERVER);
    } else if (action === 'check' && !this.consent) {
      await this.refresh();
      if (!this.closed && this.snapshot) this.start('probe');
    }
  }

  private async drain(): Promise<void> {
    while (this.receipts.length) {
      const receipt = this.receipts.shift()!;
      if (receipt.request !== this.pending || receipt.request.abort.signal.aborted || this.closed && !this.disabling) continue;
      if (receipt.kind === 'approval') {
        if (receipt.request.operation === 'login' && !this.closed) this.approval = receipt.approval;
        continue;
      }
      this.cancel();
      await this.refresh();
      if (receipt.kind === 'login') {
        if (receipt.error) this.error = receipt.error;
        else if (!this.closed && this.snapshot?.connected && this.snapshot.uploadsEnabled &&
            (this.snapshot.server ?? DEFAULT_SERVER) === receipt.request.server && !this.localError && await this.backend.activate()) {
          this.error = null;
        } else {
          // A successful backend login without authoritative readback must not
          // remain pending indefinitely or lend consent to a later retry.
          this.afterDisableError = 'unverified-commit';
          this.retryDisable = false;
          this.start('disable');
        }
      } else if (receipt.kind === 'probe') this.health = receipt.health;
      else if (receipt.error) this.error = receipt.error;
      else { this.retryDisable = null; this.error = this.afterDisableError; this.afterDisableError = null; }
    }
  }

  async request(action?: Action): Promise<{ view: View; openUri?: string }> {
    if (this.closed) return { view: this.view() };
    await this.refresh();
    // Explicit cancellation/revocation wins over queued completion receipts.
    const openUri = action ? await this.command(action) : undefined;
    await this.drain();
    return { view: this.view(), ...(openUri === undefined ? {} : { openUri }) };
  }

  view(): View {
    const buttons: Button[] = [];
    const button = (command: Action, title: string, enabled = true) => buttons.push({ command, title, enabled: !this.closed && enabled });
    const s = this.snapshot;
    if (this.pending?.operation === 'login') {
      if (this.approval) button('open-browser', 'Open browser');
      button('cancel', 'Cancel sign-in'); button('disable', 'Disable uploads'); button('disconnect', 'Disconnect');
    } else if (this.pending?.operation === 'probe') {
      button('cancel', 'Cancel check'); button('disable', 'Disable uploads'); button('disconnect', 'Disconnect');
    } else if (this.pending?.operation === 'disable') button(this.pending.logout ? 'disconnect' : 'disable', 'Please wait…', false);
    else if (this.consent) { button('allow', 'Allow and continue', !!s); button('cancel', 'Not now'); }
    else {
      const reauthorize = !!s?.reauthorizationRequired || this.health === 'auth-required';
      if (!s?.uploadsEnabled && !s?.renewalRequired || reauthorize) button('connect', reauthorize ? 'Sign in again…' : s?.connected ? 'Enable uploads…' : 'Sign in to Connect…', !!s && this.retryDisable === null);
      if (s?.connected || s?.renewalRequired || reauthorize || this.error || this.localError) button('check', 'Check connection', !!s);
      if (s?.uploadsEnabled || s?.renewalRequired || this.retryDisable === false || this.localError) button('disable', this.retryDisable === false ? 'Retry disabling uploads' : 'Disable uploads');
      if (s?.connected || s?.renewalRequired || reauthorize || this.retryDisable === true || this.localError) button('disconnect', this.retryDisable === true ? 'Retry disconnecting' : 'Disconnect');
    }
    return {
      title: 'Apparent Connect', status: this.pending ? 'Working…' : s?.uploadsEnabled ? 'Uploads on' : s?.connected ? 'Connected · uploads off' : 'Not connected',
      detail: 'Local recording remains available.', note: 'Existing captures are not uploaded. Sharing is a separate action.',
      server: s ? s.server ?? DEFAULT_SERVER : null,
      verificationUri: this.closed ? null : this.approval?.verificationUri ?? null,
      userCode: this.closed ? null : this.approval?.userCode ?? null, buttons,
      presentationState: {
        snapshot: s ? { connected: s.connected, uploadsEnabled: s.uploadsEnabled,
          renewalRequired: s.renewalRequired, reauthorizationRequired: s.reauthorizationRequired, accountInitial: s.accountInitial } : null,
        error: this.error, localError: this.localError, health: this.health,
        consent: this.consent, approval: this.approval !== null, operation: this.pending?.operation ?? null,
        logout: this.pending?.logout ?? false, retryDisable: this.retryDisable, confirmation: this.confirmation,
        closed: this.closed, sharedLinksUrl: this.sharedLinksUrl,
      },
    };
  }
}
