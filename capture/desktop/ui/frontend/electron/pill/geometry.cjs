'use strict';
function inset(y,width,height,radius=28){
 const r=Math.min(radius,width/2,height/2);
 const edge=Math.min(y+.5,height-y-.5);
 return edge>=r?0:Math.ceil(r-Math.sqrt(Math.max(0,r*r-(r-edge)**2)));
}
function shape(width,height){
 return Array.from({length:height},(_,y)=>{const x=inset(y,width,height);return {x,y,width:Math.max(1,width-2*x),height:1};});
}
function contains(point,bounds){
 const x=point.x-bounds.x,y=point.y-bounds.y;
 if(y<0||y>=bounds.height)return false;
 const edge=inset(Math.floor(y),bounds.width,bounds.height);
 return x>=edge&&x<bounds.width-edge;
}
function clamp(bounds,area){
 return {x:Math.min(Math.max(bounds.x,area.x),area.x+Math.max(0,area.width-bounds.width)),
 y:Math.min(Math.max(bounds.y,area.y),area.y+Math.max(0,area.height-bounds.height))};
}
module.exports={shape,contains,clamp};
