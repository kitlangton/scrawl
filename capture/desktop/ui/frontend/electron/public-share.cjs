const {publicShare} = require('./runtime.cjs');
class PublicShare {
  constructor(home) { this.home=home; this.epoch=0; }
  close() {
    this.epoch++;
    const operation=this.operation; this.operation=undefined;
    if(operation) void publicShare(this.home,'cancel',operation).catch(()=>{});
  }
  async request(action, expected) {
    if(action==='cancel') { this.close(); return {status:'connected',state:'cancelled'}; }
    if(this.busy) return {status:'error',code:'app_busy'};
    if(action==='start' && (!expected || this.operation)) return {status:'error',code:'refresh_required'};
    if(action==='read' && !this.operation) return {status:'error',code:'no_operation'};
    const epoch=this.epoch; this.busy=true;
    try {
      let result=await publicShare(this.home,action,action==='start'?expected:this.operation);
      if(epoch!==this.epoch) {
        if(result.operation) void publicShare(this.home,'cancel',result.operation).catch(()=>{});
        return {status:'cancelled'};
      }
      if(result.operation) this.operation=result.operation;
      if(result.state==='prepared') {
        result=await publicShare(this.home,'deliver',this.operation);
        if(epoch!==this.epoch) return {status:'cancelled'};
      }
      if(result.status!=='connected' || result.state!=='preparing') this.close();
      const {operation:_,...projected}=result;
      return projected;
    } catch { this.close(); return {status:'error',code:'bridge_failed'}; }
    finally { this.busy=false; }
  }
}
module.exports={PublicShare};
