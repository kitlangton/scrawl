import { lstat } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { authorizeDevice, DeviceFlowError } from './device-flow.cjs';
import { AccountController } from './settings-controller.cjs';
import type { AccountBackend, Approval, Health, LocalState, Snapshot } from './settings-controller.cjs';
import type { Action, SafeError } from './settings-presentation.js';
import { presentConnect } from './settings-presentation.js';
import { LegacySettings, actions, browserUri, linksUri } from './legacy-settings.cjs';
export { LegacySettings, actions, browserUri };

type Status = 'error' | 'disabled' | 'disconnected' | 'cancelled' | 'unsupported';
class BridgeError extends Error { constructor(readonly status: Status) { super(status); } }
const bad = (): never => { throw new BridgeError('error'); };
const text = (value: unknown): value is string => typeof value === 'string' && Buffer.byteLength(value) <= 4096 && !/[\x00-\x1f\x7f]/.test(value);
const record = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : bad();
const errors = new Set<unknown>(['cancelled','offline','auth-required','rejected','expired','local-state','invalid-server','unverified-commit','unavailable']);
const safeError = (value: unknown): SafeError | null => value === null || errors.has(value) ? value as SafeError | null : bad();
const health = (value: unknown): Health => ['online','offline','auth-required'].includes(value as string) ? value as Health : bad();
const id = (value: unknown): number => typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : bad();
type DeviceKind = 'device-start'|'device-poll'|'device-commit';
type Event = { operation: number } & ({kind:'approval';approval:Approval} | {kind:'probe';health:Health} | {kind:'login'|'disable'|'disconnect';error:SafeError|null} | {kind:DeviceKind;reply:unknown;error:SafeError|null});
type Account = LocalState & { events: Event[]; operation?: number; activated?: boolean; deviceFlow?: 1 };

function decode(value: unknown, opening: boolean): Account {
  const response = record(value);
  if (response.error === 'feature_disabled') throw new BridgeError('disabled');
  if (opening && response.error === 'invalid_request') throw new BridgeError('unsupported');
  // A legacy-only peer may return its old opening shape. No mutation was sent.
  if (opening && response.account === undefined && response.connect !== undefined) throw new BridgeError('unsupported');
  const a = record(response.account);
  if (a.version !== 1) bad();
  const error = safeError(a.error);
  if (!(a.sharedLinksUrl === null || linksUri(a.sharedLinksUrl))) bad();
  let snapshot: Snapshot | null = null;
  if (a.snapshot !== null) {
    const s = record(a.snapshot);
    if (!['connected','uploadsEnabled','renewalRequired','reauthorizationRequired'].every(k => typeof s[k] === 'boolean') ||
        !(s.server === null || text(s.server) && browserUri(s.server, null)) ||
        !(s.accountInitial === null || typeof s.accountInitial === 'string' && /^[\p{L}\p{N}]{1,3}$/u.test(s.accountInitial))) bad();
    snapshot = {server:s.server as string|null,connected:s.connected as boolean,uploadsEnabled:s.uploadsEnabled as boolean,
      renewalRequired:s.renewalRequired as boolean,reauthorizationRequired:s.reauthorizationRequired as boolean,accountInitial:s.accountInitial as string|null};
  }
  if (snapshot === null && error === null) bad();
  if (!Array.isArray(a.events) || a.events.length > 64) bad();
  const events = (a.events as unknown[]).map(raw => {
    const e = record(raw), operation = id(e.operation);
    if (e.kind === 'approval') {
      const approval = record(e.approval);
      if (!(typeof approval.userCode === 'string' && /^[a-zA-Z0-9-]{1,128}$/.test(approval.userCode)) || !browserUri(approval.verificationUri, approval.userCode)) bad();
      return {operation,kind:'approval',approval:{verificationUri:approval.verificationUri,userCode:approval.userCode}} as Event;
    }
    if (['device-start','device-poll','device-commit'].includes(e.kind as string)) return {operation,kind:e.kind as DeviceKind,reply:e.reply,error:safeError(e.error)};
    if (e.kind === 'probe') return {operation,kind:'probe',health:health(e.health)} as Event;
    if (e.kind === 'login' || e.kind === 'disable' || e.kind === 'disconnect') return {operation,kind:e.kind,error:safeError(e.error)} as Event;
    return bad();
  });
  if (a.deviceFlow !== undefined && a.deviceFlow !== 1) bad();
  if (a.activated !== undefined && typeof a.activated !== 'boolean') bad();
  return {snapshot,error,sharedLinksUrl:a.sharedLinksUrl as string|null,events,...(a.deviceFlow === 1 ? {deviceFlow:1 as const} : {}),
    ...(a.operation === undefined ? {} : {operation:id(a.operation)}),
    ...(a.activated === undefined ? {} : {activated:a.activated as boolean})};
}

// One bounded request/receipt per owned stream. Internal cancellation commands
// share this queue; no ambiguous mutation is retried and no generic IPC escapes.
class Channel {
  private socket?: net.Socket;
  private opened = false;
  private generation = 0;
  private closed = false;
  private tail: Promise<unknown> = Promise.resolve();
  private finish?: (error: BridgeError) => void;
  constructor(private home: string, private timeout: () => number) {}
  get isClosed(): boolean { return this.closed; }
  close(): void {
    this.closed = true;
    this.generation += 1;
    this.socket?.destroy(); this.socket = undefined; this.opened = false;
    this.finish?.(new BridgeError('cancelled'));
  }
  request(command?: string, operation?: number, origin?: string, device?: {deviceCode?:string;timeoutMs?:number;site?:string}): Promise<Account> {
    const generation = this.generation;
    const result = this.tail.then(() => {
      if (this.closed || generation !== this.generation) throw new BridgeError('cancelled');
      return this.exchange(command, operation, origin, device);
    }).catch(error => { this.close(); throw error; });
    this.tail = result.catch(() => {});
    return result;
  }
  private async exchange(command?: string, operation?: number, origin?: string, device?: {deviceCode?:string;timeoutMs?:number;site?:string}): Promise<Account> {
    if (this.closed) throw new BridgeError('cancelled');
    const generation = this.generation;
    if (!this.socket) {
      if (command !== undefined) bad();
      const directory = path.join(this.home, '.config/apparent/agent-control');
      let dir, entry;
      try { [dir, entry] = await Promise.all([lstat(directory),lstat(path.join(directory,'capture.sock'))]); }
      catch (error) { throw new BridgeError((error as NodeJS.ErrnoException).code === 'ENOENT' ? 'disconnected' : 'error'); }
      if (generation !== this.generation) throw new BridgeError('cancelled');
      if (!dir.isDirectory() || dir.uid !== process.geteuid!() || dir.mode & 0o077 || !entry.isSocket() || entry.uid !== process.geteuid!() || (entry.mode & 0o777) !== 0o600) bad();
      this.socket = net.createConnection(path.join(directory,'capture.sock'));
      const socket = this.socket;
      const closed = () => { if (this.socket === socket) this.close(); };
      socket.on('error', closed); socket.on('end', closed); socket.on('close', closed);
    }
    const socket = this.socket, opening = !this.opened;
    const result = await new Promise<Account>((resolve,reject) => {
      let bytes = Buffer.alloc(0), finished = false;
      const done = (error?: BridgeError, value?: Account) => {
        if (finished) return; finished = true; clearTimeout(timer);
        socket.removeListener('data', data);
        if (this.finish === finish) this.finish = undefined;
        if (error) reject(error); else resolve(value!);
      };
      const finish = (error: BridgeError) => done(error);
      const data = (chunk: Buffer) => {
        bytes = Buffer.concat([bytes,chunk]);
        const newline = bytes.indexOf(10);
        if (bytes.length > 65536 || newline >= 0 && newline !== bytes.length - 1) { done(new BridgeError('error')); return; }
        if (newline < 0) return;
        try { done(undefined,decode(JSON.parse(bytes.subarray(0,newline).toString('utf8')),opening)); }
        catch (error) { done(error instanceof BridgeError ? error : new BridgeError('error')); }
      };
      const timer = setTimeout(() => { done(new BridgeError('cancelled')); this.close(); }, this.timeout());
      this.finish = finish; socket.on('data',data);
      socket.write(JSON.stringify({connect:opening?'backend':command?'backend-command':'backend-read',
        ...(opening ? {version:1} : {}),
        ...(command ? {command} : {}),...(operation === undefined ? {} : {operation}),
        ...(origin === undefined ? {} : {origin}),...device,
        deadline:Number(process.hrtime.bigint()/1000000n)+Math.min(this.timeout(),3000)})+'\n');
    });
    if (generation !== this.generation) throw new BridgeError('cancelled');
    this.opened = true;
    return result;
  }
}

type Pending = { kind: 'login'|'probe'|'disable'|'disconnect'; approval?: (value:Approval)=>void;
  finish: (value: SafeError|null|Health)=>void; abort?:()=>void; signal?:AbortSignal };
class Backend implements AccountBackend {
  private pending = new Map<number,Pending>();
  private successfulLogin?: number;
  private local?: LocalState;
  private deviceFlow = false;
  private deviceWait?: {operation:number;kind:DeviceKind;finish:(reply:unknown,error:SafeError|null)=>void};
  constructor(readonly channel: Channel) {}
  close(): void {
    this.channel.close();
    this.deviceWait?.finish(null,'cancelled'); this.deviceWait = undefined;
    for (const item of this.pending.values()) { this.retire(item); item.finish(item.kind === 'probe' ? 'offline' : 'cancelled'); }
    this.pending.clear(); this.successfulLogin = undefined;
  }
  private retire(item: Pending): void { if (item.abort) item.signal?.removeEventListener('abort',item.abort); }
  private consume(a: Account): void {
    if (a.deviceFlow === 1) this.deviceFlow = true;
    this.local = {snapshot:a.snapshot,error:a.error,sharedLinksUrl:a.sharedLinksUrl};
    for (const event of a.events) {
      if (event.kind === 'device-start' || event.kind === 'device-poll' || event.kind === 'device-commit') {
        const wait = this.deviceWait;
        if (wait && wait.operation === event.operation && wait.kind === event.kind) {
          this.deviceWait = undefined; wait.finish(event.reply,event.error);
        }
        continue;
      }
      const pending = this.pending.get(event.operation);
      if (!pending) continue;
      if (event.kind === 'approval') { if (pending.kind !== 'login') bad(); pending.approval?.(event.approval); continue; }
      if (pending.kind !== event.kind) bad();
      this.pending.delete(event.operation); this.retire(pending);
      if (event.kind === 'login' && event.error === null) this.successfulLogin = event.operation;
      pending.finish(event.kind === 'probe' ? event.health : event.error);
    }
  }
  async read(): Promise<LocalState> { const a = await this.channel.request(); this.consume(a); return this.local!; }
  private async operation(kind:Pending['kind'], signal?:AbortSignal, approval?:(value:Approval)=>void, origin?:string): Promise<SafeError|null|Health> {
    if (signal?.aborted) return kind === 'probe' ? 'offline' : 'cancelled';
    if (kind === 'disable' || kind === 'disconnect') this.successfulLogin = undefined;
    const a = await this.channel.request(kind,undefined,origin); this.consume(a);
    if (a.operation === undefined) return kind === 'probe' ? 'offline' : a.error ?? 'unavailable';
    const operation = a.operation;
    if (this.pending.has(operation)) bad();
    return new Promise(resolve => {
      const abort = () => {
        const item = this.pending.get(operation); if (!item) return;
        this.pending.delete(operation); this.retire(item);
        void this.channel.request('cancel').then(reply=>this.consume(reply)).catch(()=>this.close());
        resolve(kind === 'probe' ? 'offline' : 'cancelled');
      };
      this.pending.set(operation,{kind,approval,finish:resolve,signal,abort});
      signal?.addEventListener('abort',abort,{once:true});
      if (signal?.aborted) abort();
    });
  }
  private async deviceLogin(server:string, signal:AbortSignal, approval:(value:Approval)=>void):Promise<SafeError|null> {
    let operation: number | undefined;
    const step = async (kind:DeviceKind, command:string, fields?:{deviceCode?:string;timeoutMs?:number;site?:string}):Promise<unknown> => {
      if (signal.aborted) throw new DeviceFlowError('cancelled');
      const a = await this.channel.request(command,operation,command === 'device-login' ? server : undefined,fields);
      this.consume(a);
      if (signal.aborted) throw new DeviceFlowError('cancelled');
      if (a.operation === undefined) throw new DeviceFlowError(a.error ?? 'unavailable');
      if (operation !== undefined && operation !== a.operation) bad();
      operation = a.operation;
      return new Promise((resolve,reject) => {
        this.deviceWait = {operation:operation!,kind,finish:(reply,error)=> error ? reject(new DeviceFlowError(error)) : resolve(reply)};
      });
    };
    const abort = () => {
      const wait = this.deviceWait; this.deviceWait = undefined;
      wait?.finish(null,'cancelled');
      void this.channel.request('cancel').then(a=>this.consume(a)).catch(()=>this.close());
    };
    signal.addEventListener('abort',abort,{once:true});
    try {
      const result = await authorizeDevice({
        start:()=>step('device-start','device-login'),
        poll:(deviceCode,timeoutMs)=>step('device-poll','device-poll',{deviceCode,timeoutMs}),
        commit:async site=>{await step('device-commit','device-commit',{site});return null;},
      },signal,approval);
      if (result === null && !signal.aborted) this.successfulLogin = operation;
      else if (!signal.aborted) {
        // Retire a rejected/expired/malformed flow before admitting an explicit
        // retry on this lease; it cannot lend its marker or grant to another flow.
        try { const cancelled = await this.channel.request('cancel'); this.consume(cancelled); } catch { this.close(); }
      }
      return result;
    } finally { signal.removeEventListener('abort',abort); }
  }
  async login(server:string, signal:AbortSignal, approval:(value:Approval)=>void):Promise<SafeError|null> {
    if ((this.local?.snapshot?.server ?? 'https://hosted.tryapparent.com') !== server) return 'invalid-server';
    if (this.deviceFlow) return this.deviceLogin(server,signal,approval);
    return await this.operation('login',signal,approval,server) as SafeError|null;
  }
  async probe(signal:AbortSignal):Promise<Health> { return await this.operation('probe',signal) as Health; }
  async disable(logout:boolean):Promise<SafeError|null> { return await this.operation(logout?'disconnect':'disable') as SafeError|null; }
  async activate():Promise<boolean> {
    const id = this.successfulLogin; this.successfulLogin = undefined;
    if (id === undefined) return false;
    const reply = await this.channel.request('activate',id); this.consume(reply);
    return reply.activated === true;
  }
}

export class ConnectSettings {
  timeout: number;
  private backend: Backend;
  private controller: AccountController;
  private legacy?: InstanceType<typeof LegacySettings>;
  private observed = false;
  private busy = false;
  private generation = 0;
  constructor(private home:string, timeout=3500) {
    this.timeout = timeout;
    this.backend = new Backend(new Channel(home,()=>this.timeout));
    this.controller = new AccountController(this.backend);
  }
  close():void {
    this.generation += 1; this.controller.close(); this.backend.close(); this.legacy?.close();
    this.legacy = undefined; this.observed = false;
    this.backend = new Backend(new Channel(this.home,()=>this.timeout));
    this.controller = new AccountController(this.backend);
  }
  async request(action?:Action):Promise<unknown> {
    if (action !== undefined && !actions.has(action)) throw new Error('Invalid Connect action');
    if (this.busy) return {status:'error'};
    // A later local read may open a new lease after idle EOF. Never revive the
    // old request registry or retry an action on a replacement connection.
    if (action === undefined && !this.legacy && this.backend.channel.isClosed) this.close();
    if (action !== undefined && !this.observed) return {status:'error'};
    this.busy = true;
    const generation = this.generation;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      if (this.legacy) { this.legacy.timeout = this.timeout; return await this.legacy.request(action); }
      const result = await Promise.race([
        this.controller.request(action),
        new Promise<never>((_,reject)=>{timer=setTimeout(()=>{this.close();reject(new BridgeError('cancelled'));},this.timeout);}),
      ]);
      if (generation !== this.generation) return {status:'cancelled'};
      this.observed = true;
      return {status:'connected',...result,view:{...result.view,presentation:presentConnect(result.view.presentationState,result.view.buttons)}};
    } catch (error) {
      const status = error instanceof BridgeError ? error.status : 'error';
      // Fallback only after an unsupported read-only opening, before any action.
      if (status === 'unsupported' && action === undefined && !this.observed && generation === this.generation) {
        this.backend.close(); this.legacy = new LegacySettings(this.home,this.timeout);
        const result = await this.legacy.request(undefined);
        this.observed = record(result).status === 'connected';
        return result;
      }
      this.close(); return {status:status === 'unsupported' ? 'error' : status};
    } finally { if (timer) clearTimeout(timer); this.busy = false; }
  }
}
