'use strict';
const actions = new Set(['record','cancel','finish','whiteboard','paste','copy-link','inspect','dismiss',
  'select-area','select-screen','select-iphone','switch-area','switch-screen','switch-iphone']);
function view(value) {
  if (!value || !Number.isSafeInteger(value.revision) || value.revision < 0 ||
      !['hidden','selecting','recording','processing','completed'].includes(value.phase) ||
      typeof value.clock !== 'string' || value.clock.length > 20 ||
      typeof value.label !== 'string' || value.label.length > 300 ||
      !(value.warning === null || typeof value.warning === 'string' && value.warning.length <= 300) ||
      !(value.inputLevel == null || Number.isFinite(value.inputLevel) && value.inputLevel >= 0 && value.inputLevel <= 1) ||
      !(value.notice == null || typeof value.notice === 'string' && value.notice.length<=120) ||
      typeof value.reducedMotion !== 'boolean' || typeof value.draggable !== 'boolean' ||
      !Array.isArray(value.actions) || value.actions.length > 10) throw Error('Invalid pill state');
  const seen = new Set();
  const controls = value.actions.map(a => {
    if (!a || !actions.has(a.id) || seen.has(a.id) || typeof a.label !== 'string' || a.label.length > 60 ||
        ['enabled','selected','primary'].some(k => typeof a[k] !== 'boolean')) throw Error('Invalid pill control');
    seen.add(a.id);
    return {id:a.id,label:a.label,enabled:a.enabled,selected:a.selected,primary:a.primary};
  });
  let pocket=null;
  if(value.pocket!=null){
    const p=value.pocket;
    if(!['info','warning','failure'].includes(p.tone) || typeof p.title!=='string' || !p.title.length || p.title.length>80 ||
       typeof p.description!=='string' || p.description.length>300 || !Array.isArray(p.actionIds) || p.actionIds.length>3 ||
       new Set(p.actionIds).size!==p.actionIds.length || p.actionIds.some(id=>!controls.some(a=>a.id===id)))throw Error('Invalid pill pocket');
    pocket={tone:p.tone,title:p.title,description:p.description,actionIds:[...p.actionIds]};
  }
  return {revision:value.revision,phase:value.phase,clock:value.clock,label:value.label,warning:value.warning,notice:value.notice??null,
    reducedMotion:value.reducedMotion,draggable:value.draggable,inputLevel:value.inputLevel??null,pocket,actions:controls};
}
function command(current, revision, action) {
  if (!current || revision !== current.revision || !actions.has(action) ||
      !current.actions.some(a => a.id === action && a.enabled)) throw Error('Stale or unavailable pill action');
  return {revision,action};
}
module.exports = {view,command};
