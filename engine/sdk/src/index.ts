import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { randomBytes, randomUUID } from 'node:crypto';
import { createInterface } from 'node:readline';

export type State = 'starting'|'recording'|'stopping'|'finalizing'|'saved'|'cancelled'|'failed'|'rejected';
export type ErrorCode = 'malformed_request'|'unsupported_version'|'identity_conflict'|'not_ready'|'not_found'|'busy'|'permission_denied'|'unavailable'|'internal';
export interface Handshake { protocolVersion: 1; implementation: 'native'|'test-double'; capabilities: { screenCapture: boolean; audioCapture: boolean; windowCapture: boolean }; readiness: 'ready'|'unavailable'; }
export interface Receipt { protocolVersion: 1; requestId: string; bundleId: string; state: State; bundlePath?: string; artifactVersion?: 1; error?: { code: ErrorCode; message: string }; }
export interface StartRequest { protocolVersion: 1; clientId: string; requestId: string; method: 'start'; ownerToken: string; bundleId: string; bundlePath: string; }
export interface HandshakeRequest { protocolVersion: 1; clientId: string; requestId: string; method: 'handshake'; }
export interface ControlRequest { protocolVersion: 1; clientId: string; requestId: string; method: 'inspect'|'stop'|'cancel'; ownerToken: string; }
export interface Response { protocolVersion: 1; requestId: string; result?: Handshake|Receipt; error?: { code: ErrorCode; message: string }; }
export interface StartResult { receipt: Receipt; requestId: string; ownerToken: string; }
export type Request = StartRequest|HandshakeRequest|ControlRequest;
const MAX_LINE = 64 * 1024;
const states = new Set<State>(['starting','recording','stopping','finalizing','saved','cancelled','failed','rejected']);
const codes = new Set<ErrorCode>(['malformed_request','unsupported_version','identity_conflict','not_ready','not_found','busy','permission_denied','unavailable','internal']);
export class EngineRpcError extends Error { readonly code:ErrorCode; constructor(code:ErrorCode,message:string){super(message);this.code=code;this.name='EngineRpcError';} }
function record(x: unknown): Record<string, unknown> { if (!x || typeof x !== 'object' || Array.isArray(x)) throw new Error('invalid protocol object'); return x as Record<string, unknown>; }
function exact(x: Record<string, unknown>, keys: string[]): void { if (Object.keys(x).some(k => !keys.includes(k))) throw new Error('unknown protocol field'); }
function nonempty(x: unknown): x is string { return typeof x === 'string' && x.length > 0 && x.length <= 4096; }
function checkedResponse(value:Response,requestId:string):Response {
  const x=record(value); exact(x,['protocolVersion','requestId','result','error']);
  if(x.protocolVersion!==1||x.requestId!==requestId||((x.result===undefined)===(x.error===undefined))) throw new Error('invalid Engine response envelope');
  if(x.error!==undefined){const e=record(x.error);exact(e,['code','message']);if(!codes.has(e.code as ErrorCode)||typeof e.message!=='string'||e.message.length>512)throw new Error('invalid Engine error');throw new EngineRpcError(e.code as ErrorCode,e.message);}
  return value;
}
export function validateHandshake(value: unknown): Handshake {
  const x=record(value); exact(x,['protocolVersion','implementation','capabilities','readiness']);
  const c=record(x.capabilities); exact(c,['screenCapture','audioCapture','windowCapture']);
  if(x.protocolVersion!==1 || !['native','test-double'].includes(String(x.implementation)) || !['ready','unavailable'].includes(String(x.readiness)) || ![c.screenCapture,c.audioCapture,c.windowCapture].every(v=>typeof v==='boolean')) throw new Error('invalid handshake');
  if(x.implementation==='test-double' && (c.screenCapture || x.readiness==='ready')) throw new Error('test double cannot claim native readiness');
  return x as unknown as Handshake;
}
export function validateReceipt(value: unknown): Receipt {
  const x=record(value); exact(x,['protocolVersion','requestId','bundleId','state','bundlePath','artifactVersion','error']);
  if(x.protocolVersion!==1 || !nonempty(x.requestId) || !nonempty(x.bundleId) || !states.has(x.state as State)) throw new Error('invalid receipt');
  if(x.state==='saved' && (!nonempty(x.bundlePath) || x.artifactVersion!==1)) throw new Error('saved receipt missing validated artifact');
  if(x.state!=='saved' && (x.bundlePath!==undefined || x.artifactVersion!==undefined)) throw new Error('non-saved receipt has artifact');
  if(x.error!==undefined) { const e=record(x.error); exact(e,['code','message']); if(!codes.has(e.code as ErrorCode)||typeof e.message!=='string'||e.message.length>512) throw new Error('invalid structured error'); }
  return x as unknown as Receipt;
}
export function encodeRequest(request: Request): string {
  const x=record(request); exact(x, request.method==='start'?['protocolVersion','clientId','requestId','method','ownerToken','bundleId','bundlePath']:request.method==='handshake'?['protocolVersion','clientId','requestId','method']:['protocolVersion','clientId','requestId','method','ownerToken']);
  if(x.protocolVersion!==1 || !nonempty(x.clientId)||!nonempty(x.requestId)) throw new Error('invalid request identity');
  if(request.method==='start' && (!nonempty(request.ownerToken)||!nonempty(request.bundleId)||!nonempty(request.bundlePath))) throw new Error('invalid start request');
  if(request.method!=='start' && request.method!=='handshake' && !nonempty(request.ownerToken)) throw new Error('missing owner authority');
  const line=JSON.stringify(request); if(Buffer.byteLength(line)>MAX_LINE) throw new Error('request exceeds protocol limit'); return line+'\n';
}
export interface Transport { request(request: Request, timeoutMs?: number): Promise<Response>; close(): Promise<void>; }
export class EngineClient {
  readonly clientId=randomUUID();
  private readonly transport: Transport;
  constructor(transport: Transport) { this.transport=transport; }
  handshake(timeoutMs=5000): Promise<Handshake> { const id=randomUUID();return this.transport.request({protocolVersion:1,clientId:this.clientId,requestId:id,method:'handshake'},timeoutMs).then(r=>validateHandshake(checkedResponse(r,id).result)); }
  prepareStart(bundleId:string,bundlePath:string):StartRequest { return {protocolVersion:1,clientId:this.clientId,requestId:randomUUID(),method:'start',ownerToken:randomBytes(32).toString('hex'),bundleId,bundlePath}; }
  start(request:StartRequest,timeoutMs=5000): Promise<StartResult> { encodeRequest(request);return this.transport.request(request,timeoutMs).then(r=>({receipt:validateReceipt(checkedResponse(r,request.requestId).result),requestId:request.requestId,ownerToken:request.ownerToken})); }
  control(method:'inspect'|'stop'|'cancel',requestId:string,ownerToken:string,timeoutMs=5000): Promise<Receipt> { return this.transport.request({protocolVersion:1,clientId:this.clientId,requestId,method,ownerToken},timeoutMs).then(r=>validateReceipt(checkedResponse(r,requestId).result)); }
  close(): Promise<void> { return this.transport.close(); }
}
/** Spawn only a caller-verified, explicit executable path; never resolve via PATH. */
export function launchEngine(executablePath:string): ChildProcessTransport {
  if(!executablePath.startsWith('/')) throw new Error('Engine path must be absolute');
  const child=spawn(executablePath,[],{stdio:['pipe','pipe','pipe'],shell:false}); return new ChildProcessTransport(child);
}
export class ChildProcessTransport implements Transport {
  private readonly lines; private pending=new Map<string,{resolve:(r:Response)=>void;reject:(e:Error)=>void;timer:ReturnType<typeof setTimeout>}>(); private closed=false;
  private readonly child:ChildProcessWithoutNullStreams;
  constructor(child:ChildProcessWithoutNullStreams) {
    this.child=child;
    this.lines=createInterface({input:child.stdout,crlfDelay:Infinity});
    this.lines.on('line',line=>{ if(Buffer.byteLength(line)>MAX_LINE) return this.fail(new Error('Engine response exceeds protocol limit')); let value:unknown; try{value=JSON.parse(line)}catch{return this.fail(new Error('malformed Engine response'))}; try{const x=record(value); exact(x,['protocolVersion','requestId','result','error']); if(x.protocolVersion!==1||!nonempty(x.requestId))throw new Error('invalid Engine envelope'); const p=this.pending.get(x.requestId); if(!p)return; clearTimeout(p.timer);this.pending.delete(x.requestId);p.resolve(x as unknown as Response);}catch(e){this.fail(e instanceof Error?e:new Error('invalid Engine response'));} });
    child.on('error',e=>this.fail(e)); child.on('exit',()=>this.fail(new Error('Engine process disconnected')));
  }
  request(request:Request,timeoutMs=5000):Promise<Response>{if(this.closed)return Promise.reject(new Error('Engine disconnected'));const line=encodeRequest(request);return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{this.pending.delete(request.requestId);reject(new Error('Engine request timed out; operation was not cancelled'));},timeoutMs);this.pending.set(request.requestId,{resolve,reject,timer});this.child.stdin.write(line,e=>{if(e){clearTimeout(timer);this.pending.delete(request.requestId);reject(e);}});});}
  private fail(error:Error){if(this.closed)return;this.closed=true;for(const p of this.pending.values()){clearTimeout(p.timer);p.reject(error)}this.pending.clear();}
  async close(){if(this.closed)return;this.child.stdin.end();this.fail(new Error('Engine transport closed'));}
}
