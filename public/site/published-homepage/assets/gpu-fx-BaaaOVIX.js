import{n as e,r as t,t as n}from"./index-DQGXji3Z.js";var r=t(),i=e(),a=32,o=34,s=1100,c=360,l=1.85,u=`
struct U { data: array<vec4<f32>, ${o}> };
@group(0) @binding(0) var<uniform> u: U;

struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) p: vec2<f32> };

@vertex
fn vsMain(@builtin(vertex_index) vid: u32) -> VsOut {
  var c = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  let ndc = c[vid];
  var o: VsOut;
  o.pos = vec4<f32>(ndc, 0.0, 1.0);
  o.p = vec2<f32>((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5) * u.data[0].xy;
  return o;
}

@fragment
fn fsMain(i: VsOut) -> @location(0) vec4<f32> {
  let p = i.p;
  let box = u.data[1];
  if (p.x < box.x || p.y < box.y || p.x > box.z || p.y > box.w) { return vec4<f32>(0.0); }
  let n = i32(u.data[0].w);
  var best = 1e9;
  var c = vec2<f32>(0.0);
  var wet = 0.0;
  for (var k = 0; k < 31; k = k + 1) {
    if (k + 1 >= n) { break; }
    let a = u.data[2 + k];
    let b = u.data[3 + k];
    if (b.w <= 0.0) { continue; }
    let ab = b.xy - a.xy;
    let h = clamp(dot(p - a.xy, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0);
    let q = a.xy + ab * h;
    let d = length(p - q);
    if (d < best) { best = d; c = q; wet = mix(a.z, b.z, h); }
  }
  let hw = ${l.toFixed(2)};
  if (best > hw + 1.0) { return vec4<f32>(0.0); }
  let off = (p - c) / hw;
  let r = min(length(off), 1.0);
  let nrm = vec3<f32>(off * (r / max(length(off), 1e-4)), sqrt(max(0.0, 1.0 - r * r)));
  let l = normalize(vec3<f32>(-0.45, -0.8, 0.9));
  let hv = normalize(l + vec3<f32>(0.0, 0.0, 1.0));
  let spec = pow(max(dot(nrm, hv), 0.0), 24.0);
  let cover = smoothstep(hw + 0.5, hw - 0.5, best);
  let a = spec * wet * cover * u.data[0].z;
  return vec4<f32>(a, a * 0.96, a * 0.92, a);
}
`,d=`#version 300 es
precision highp float;
uniform vec4 u_data[${o}];
out vec2 v_p;
void main() {
  vec2 c[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  vec2 ndc = c[gl_VertexID];
  gl_Position = vec4(ndc, 0.0, 1.0);
  v_p = vec2((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5) * u_data[0].xy;
}
`,f=`#version 300 es
precision highp float;
uniform vec4 u_data[${o}];
in vec2 v_p;
out vec4 o;
void main() {
  vec2 p = v_p;
  vec4 box = u_data[1];
  if (p.x < box.x || p.y < box.y || p.x > box.z || p.y > box.w) { o = vec4(0.0); return; }
  int n = int(u_data[0].w);
  float best = 1e9;
  vec2 c = vec2(0.0);
  float wet = 0.0;
  for (int k = 0; k < 31; k++) {
    if (k + 1 >= n) break;
    vec4 a = u_data[2 + k];
    vec4 b = u_data[3 + k];
    if (b.w <= 0.0) continue;
    vec2 ab = b.xy - a.xy;
    float h = clamp(dot(p - a.xy, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0);
    vec2 q = a.xy + ab * h;
    float d = length(p - q);
    if (d < best) { best = d; c = q; wet = mix(a.z, b.z, h); }
  }
  float hw = ${l.toFixed(2)};
  if (best > hw + 1.0) { o = vec4(0.0); return; }
  vec2 off = (p - c) / hw;
  float r = min(length(off), 1.0);
  vec3 nrm = vec3(off * (r / max(length(off), 1e-4)), sqrt(max(0.0, 1.0 - r * r)));
  vec3 l = normalize(vec3(-0.45, -0.8, 0.9));
  vec3 hv = normalize(l + vec3(0.0, 0.0, 1.0));
  float spec = pow(max(dot(nrm, hv), 0.0), 24.0);
  float cover = smoothstep(hw + 0.5, hw - 0.5, best);
  float a = spec * wet * cover * u_data[0].z;
  o = vec4(a, a * 0.96, a * 0.92, a);
}
`;async function p(e){if(typeof navigator>`u`||!(`gpu`in navigator))return null;try{let t=navigator.gpu,n=await t.requestAdapter();if(!n)return null;let r=await n.requestDevice(),i=e.getContext(`webgpu`);if(!i)return null;let a=t.getPreferredCanvasFormat();i.configure({device:r,format:a,alphaMode:`premultiplied`});let o=r.createShaderModule({code:u}),s=r.createBuffer({size:544,usage:72}),c=r.createBindGroupLayout({entries:[{binding:0,visibility:3,buffer:{type:`uniform`}}]}),l=r.createBindGroup({layout:c,entries:[{binding:0,resource:{buffer:s}}]}),d=r.createRenderPipeline({layout:r.createPipelineLayout({bindGroupLayouts:[c]}),vertex:{module:o,entryPoint:`vsMain`},fragment:{module:o,entryPoint:`fsMain`,targets:[{format:a,blend:{color:{srcFactor:`one`,dstFactor:`one-minus-src-alpha`,operation:`add`},alpha:{srcFactor:`one`,dstFactor:`one-minus-src-alpha`,operation:`add`}}}]},primitive:{topology:`triangle-list`}}),f=e=>{let t=r.createCommandEncoder(),n=t.beginRenderPass({colorAttachments:[{view:i.getCurrentTexture().createView(),clearValue:{r:0,g:0,b:0,a:0},loadOp:`clear`,storeOp:`store`}]});e&&(n.setPipeline(d),n.setBindGroup(0,l),n.draw(3)),n.end(),r.queue.submit([t.finish()])};return{backend:`webgpu`,render(e){r.queue.writeBuffer(s,0,e.buffer,e.byteOffset,e.byteLength),f(!0)},clear:()=>f(!1),dispose(){try{s.destroy(),r.destroy()}catch{}}}}catch{return null}}function m(e){try{let t=e.getContext(`webgl2`,{alpha:!0,premultipliedAlpha:!0,antialias:!1,depth:!1,stencil:!1,powerPreference:`low-power`});if(!t)return null;let n=(e,n)=>{let r=t.createShader(e);return t.shaderSource(r,n),t.compileShader(r),t.getShaderParameter(r,t.COMPILE_STATUS)?r:null},r=n(t.VERTEX_SHADER,d),i=n(t.FRAGMENT_SHADER,f);if(!r||!i)return null;let a=t.createProgram();if(t.attachShader(a,r),t.attachShader(a,i),t.linkProgram(a),!t.getProgramParameter(a,t.LINK_STATUS))return null;let o=t.getUniformLocation(a,`u_data`),s=t.createVertexArray();t.enable(t.BLEND),t.blendFunc(t.ONE,t.ONE_MINUS_SRC_ALPHA);let c=()=>{t.viewport(0,0,e.width,e.height),t.clearColor(0,0,0,0),t.clear(t.COLOR_BUFFER_BIT)};return{backend:`webgl2`,render(e){c(),t.bindVertexArray(s),t.useProgram(a),t.uniform4fv(o,e),t.drawArrays(t.TRIANGLES,0,3)},clear:c,dispose(){t.deleteProgram(a),s&&t.deleteVertexArray(s)}}}catch{return null}}function h(){if(typeof window>`u`)return null;if(window.__apparentForceFx)return window.__apparentForceFx;let e=new URLSearchParams(window.location.search).get(`fx`);return e===`webgpu`||e===`webgl2`||e===`css`?e:null}var g=e=>e<.5?Math.cbrt(e/4):1-Math.cbrt(2*(1-e))/2;function _(e,t,n){let r=0,i=1/0,o=1/0,u=-1/0,d=-1/0;for(let f of e.strokes){if(t<=f.t0||t>f.t1+s)continue;let e=f.points.length-1,p=t=>f.t0+g(t/e)*(f.t1-f.t0),m=[];for(let n=0;n<=e;n++){let e=p(n);if(e>t){let r=f.points[n-1],i=f.points[n],a=(t-p(n-1))/Math.max(.001,e-p(n-1));r&&i&&m.push([r[0]+(i[0]-r[0])*a,r[1]+(i[1]-r[1])*a,1]);break}let r=t-e;r<s&&m.push([f.points[n][0],f.points[n][1],Math.exp(-r/c)*Math.min(1,(s-r)/200)])}if(m.length<2)continue;let h=a-r;if(h<2)break;let _=Math.max(1,Math.ceil(m.length/h));m.filter((e,t)=>t%_===0||t===m.length-1).forEach(([e,t,a],s)=>{let c=(2+r)*4;n[c]=e,n[c+1]=t,n[c+2]=a,n[c+3]=s===0?-1.85:l,i=Math.min(i,e),o=Math.min(o,t),u=Math.max(u,e),d=Math.max(d,t),r++})}return n[4]=i-4,n[5]=o-4,n[6]=u+4,n[7]=d+4,r}function v({t:e,script:t,intensity:a,stageRef:o}){let s=(0,r.useRef)(null),c=(0,r.useRef)(null),[l,u]=(0,r.useState)(`initializing`),d=(0,r.useRef)(new Float32Array(136)),f=(0,r.useRef)(!1),g=(0,r.useRef)(!1),v=(0,r.useRef)(!1),y=(0,r.useRef)([]),b=(0,r.useRef)({backend:`initializing`,drawCalls:0,points:0,avgRenderMs:0,reducedMotion:!1,lastT:0});return(0,r.useEffect)(()=>{window.__apparentGpu=()=>b.current;let e=window.matchMedia?.(`(prefers-reduced-motion: reduce)`),t=()=>{g.current=!!e?.matches,b.current.reducedMotion=g.current};return t(),e?.addEventListener?.(`change`,t),()=>e?.removeEventListener?.(`change`,t)},[]),(0,r.useEffect)(()=>{let e=!1,t=s.current;if(t)return(async()=>{let n=h(),r=null;if(n!==`css`&&(n!==`webgl2`&&(r=await p(t)),!r&&n!==`webgpu`&&(r=m(t))),e)return r?.dispose();c.current=r;let i=r?.backend??`css`;u(i),b.current.backend=i})(),()=>{e=!0,c.current?.dispose(),c.current=null}},[]),(0,r.useEffect)(()=>{let e=s.current,t=o.current;if(!e||!t)return;let r=()=>{let r=t.getBoundingClientRect().width/n.w;v.current=r<.55;let i=Math.min(window.devicePixelRatio||1,2),a=Math.max(1,Math.round(n.w*r*i)),o=Math.max(1,Math.round(n.h*r*i));(e.width!==a||e.height!==o)&&(e.width=a,e.height=o,f.current=!1)};r();let i=new ResizeObserver(r);return i.observe(t),()=>i.disconnect()},[o,l]),(0,r.useEffect)(()=>{b.current.lastT=e;let r=c.current;if(!r)return;let i=d.current,o=!t||g.current||v.current||a<=0?0:_(t,e,i);if(b.current.points=o,o<2){f.current||r.clear(),f.current=!0;return}f.current=!1,i[0]=n.w,i[1]=n.h,i[2]=a,i[3]=o;let s=performance.now();r.render(i);let l=y.current;l.push(performance.now()-s),l.length>120&&l.shift(),b.current.avgRenderMs=+(l.reduce((e,t)=>e+t,0)/l.length).toFixed(3),b.current.drawCalls++},[e,t,a]),l===`css`?null:(0,i.jsx)(`canvas`,{ref:s,className:`stage-gpu-canvas`,"aria-hidden":!0})}export{v as StageGpuOverlay,v as default};