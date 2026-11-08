const actions = new Set(['continue','back','later','close','settings','change-shortcut','use-desktop-shortcut','skip','watched','practiced','reset-shortcut','cancel-shortcut','practice-paste','practice-locate','practice-pasted','practice-finish']);
function parseAction(value) {
 if (typeof value === 'string') {
  if(actions.has(value)||/^pen:(coral|purple|blue|green)$/.test(value)) return {action:value};
  if(/^grant:(screen-recording|microphone|accessibility|camera|speech-recognition)$/.test(value)) return {action:'grant',permission:value.slice(6)};
 } else if(value && typeof value==='object' && !Array.isArray(value) && Object.keys(value).sort().join(',')==='action,key,keyCode,kind,modifiers,reserved' && value.action==='shortcut' && value.kind==='record' && Number.isInteger(value.keyCode) && value.keyCode>=0 && value.keyCode<=255 && Number.isInteger(value.modifiers) && value.modifiers>0 && (value.modifiers & ~6912)===0 && typeof value.key==='string' && /^[a-z0-9]$/.test(value.key) && value.reserved===false) return value;
 throw Error('Forbidden onboarding command');
}
function lines(consume) {let buffer='';return chunk=>{buffer+=chunk;let end;while((end=buffer.indexOf('\n'))>=0){if(end>65536)throw Error('Oversized onboarding line');const line=buffer.slice(0,end);buffer=buffer.slice(end+1);try{consume(JSON.parse(line))}catch(e){if(e instanceof SyntaxError)continue;throw e}}if(buffer.length>65536)throw Error('Oversized onboarding line')}}
function parentWriter(stream, onClosed) {
 let closed=false;
 const close=()=>{if(!closed){closed=true;onClosed();}};
 stream.on('error',close);
 return value=>{if(closed)return;try{stream.write(JSON.stringify(value)+'\n');}catch{close();}};
}
module.exports={parseAction,lines,parentWriter};
