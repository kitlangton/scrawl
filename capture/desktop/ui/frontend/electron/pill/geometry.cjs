'use strict';
function inset(y,width,height,radius=28){
 const r=Math.min(radius,width/2,height/2);
 const edge=Math.min(y+.5,height-y-.5);
 return edge>=r?0:Math.ceil(r-Math.sqrt(Math.max(0,r*r-(r-edge)**2)));
}
function segments(width,height,rowY=0,rowHeight=height){
 const row={y:rowY,width,height:rowHeight,radius:28};
 if(height<=rowHeight)return [row];
 return [row,{y:rowY===0?rowHeight+4:0,width,height:height-rowHeight-4,radius:16}];
}
function shape(width,height,rowY=0,rowHeight=height){
 return segments(width,height,rowY,rowHeight).flatMap(part=>Array.from({length:part.height},(_,y)=>{
 const x=inset(y,part.width,part.height,part.radius);return {x,y:y+part.y,width:Math.max(1,width-2*x),height:1};}));
}
function contains(point,bounds,rowY=0,rowHeight=bounds.height){
 return segments(bounds.width,bounds.height,rowY,rowHeight).some(part=>{
 const x=point.x-bounds.x,y=point.y-bounds.y-part.y;
 if(y<0||y>=part.height)return false;
 const edge=inset(Math.floor(y),part.width,part.height,part.radius);
 return x>=edge&&x<part.width-edge;
 });
}
// Keep the leading row anchored when a pocket opens, closes or changes direction.
function layout(before,area,width,height,rowHeight,previousRowY=0){
 const anchor=before.y+previousRowY;
 const up=height>rowHeight && anchor+height>area.y+area.height && anchor>=area.y+height-rowHeight;
 const rowY=up?height-rowHeight:0;
 const bounds={x:Math.round(before.x+(before.width-width)/2),y:anchor-rowY,width,height};
 return {bounds:{...bounds,...clamp(bounds,area)},rowY,rowHeight,side:up?'up':'down'};
}
function clamp(bounds,area){
 return {x:Math.min(Math.max(bounds.x,area.x),area.x+Math.max(0,area.width-bounds.width)),
 y:Math.min(Math.max(bounds.y,area.y),area.y+Math.max(0,area.height-bounds.height))};
}
module.exports={shape,contains,clamp,layout};
