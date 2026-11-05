import type { Handshake, Receipt, Request, Response, State, Transport } from './index.ts';
import { encodeRequest } from './index.ts';

/** Protocol-only fake. Never creates media or claims production readiness. */
export class ReferenceEngine implements Transport {
  readonly handshake: Handshake={protocolVersion:1,implementation:'test-double',capabilities:{screenCapture:false,audioCapture:false,windowCapture:false},readiness:'unavailable'};
  private receipts=new Map<string,Receipt>(); private owners=new Map<string,string>(); private closed=false;
  async request(request:Request):Promise<Response>{
    encodeRequest(request);
    if(this.closed)throw new Error('test reference peer closed');
    if(request.method==='handshake')return {protocolVersion:1,requestId:request.requestId,result:this.handshake};
    if(request.method==='start'){
      const old=this.receipts.get(request.requestId); if(old){if(old.bundleId!==request.bundleId)return this.error(request.requestId,'identity_conflict','request identity reused');return {protocolVersion:1,requestId:request.requestId,result:old};}
      const ownerToken=request.ownerToken;this.owners.set(request.requestId,ownerToken);
      const receipt:Receipt={protocolVersion:1,requestId:request.requestId,bundleId:request.bundleId,state:'rejected',error:{code:'unavailable',message:'test reference peer does not record media'}};this.receipts.set(request.requestId,receipt);
      return {protocolVersion:1,requestId:request.requestId,result:receipt};
    }
    const receipt=this.receipts.get(request.requestId);if(!receipt)return this.error(request.requestId,'not_found','unknown request');
    if(this.owners.get(request.requestId)!==request.ownerToken)return this.error(request.requestId,'permission_denied','owner authority rejected');
    if(request.method==='cancel' && ['starting','recording','stopping','finalizing'].includes(receipt.state)){receipt.state='cancelled';}
    return {protocolVersion:1,requestId:request.requestId,result:receipt};
  }
  async close(){this.closed=true;}
  private error(requestId:string,code:Response['error'] extends infer T ? T extends {code:infer C}?C:never:never,message:string):Response{return {protocolVersion:1,requestId,error:{code,message}};}
  /** Test-only scenario control; not an Engine RPC and cannot assert saved media. */
  scenarioState(requestId:string,state:Extract<State,'starting'|'recording'|'stopping'|'finalizing'|'failed'|'cancelled'>){const receipt=this.receipts.get(requestId);if(!receipt)throw new Error('missing scenario receipt');receipt.state=state;}
}
