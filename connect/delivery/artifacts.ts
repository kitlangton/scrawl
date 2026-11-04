import {createHash} from 'node:crypto';
import {readFile,readdir,lstat,writeFile} from 'node:fs/promises';
import {resolve,join,relative} from 'node:path';
import {execFileSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';
export const ROUTES=['connect','connect/device','connect/shares','hosted','hosted/device','hosted/shares'];
const contract={version:1,browserOrigin:'https://app.tryapparent.com',apiOrigin:'https://hosted.tryapparent.com',routes:ROUTES};
const digest=(v:Buffer)=>createHash('sha256').update(v).digest('hex');
const canonical=(v:unknown):string=>JSON.stringify(v,(_,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.entries(x).sort()):x);
export async function inventory(root:string,kind:string){
 // Native recipient delivery tools are not browser/Worker build or admission inputs.
 // Keep the shared delivery sources and combined workflow/budget contracts below.
 const result:Record<string,string>={};
 async function walk(path:string){const stat=await lstat(join(root,path));if(stat.isSymbolicLink())throw Error(`symlink: ${path}`);if(stat.isDirectory()){if(path==='connect/delivery/recipient')return;for(const name of (await readdir(join(root,path))).sort()){if(path==='connect/web'&&name==='public')continue;if(['__pycache__','node_modules','dist','dist-ssr','.git','.generated'].includes(name)||name.startsWith('dist-')||name.endsWith('.tsbuildinfo'))continue;await walk(`${path}/${name}`)}}else if(stat.isFile())result[path]=digest(await readFile(join(root,path)));else throw Error('unsupported input');}
 for(const p of [kind==='connect-browser'?'connect/web':'connect/service','connect/delivery','shared/ui','shared/analytics',...(kind==='connect-worker'?['shared/assets']:[])])await walk(p);
 if(kind==='connect-browser'){
 const assets=await readFile(join(root,'connect/web/scripts/assets.mjs'),'utf8');
 const sources=[...assets.matchAll(/\['([^']+)',\s*'[^']+'\]/g)].map(m=>m[1]);if(sources.length!==5)throw Error('expected five approved assets');
 for(const p of sources)await walk(`public/site/public/assets/${p}`);
 }
 for(const p of ['.github/workflows/connect-delivery.yml','.github/actions/runtime-budget','enablement/verification/workflow-budget.ts','enablement/release_gate.py','enablement/verification/web/inputs.ts','enablement/verification/web/baseline.ts'])await walk(p);
 const tracked=new Set(execFileSync('git',['ls-files','-z'],{cwd:root,encoding:'utf8'}).split('\0'));
 const changed=execFileSync('git',['diff','HEAD','--name-only','--diff-filter=D'],{cwd:root,encoding:'utf8'}).trim().split('\n');
 for(const p of changed)if(p.startsWith(kind==='connect-browser'?'connect/web/':'connect/service/')||(p.startsWith('connect/delivery/')&&!p.startsWith('connect/delivery/recipient/'))||p.startsWith('shared/'))throw Error(`deleted input: ${p}`);
 for(const p of Object.keys(result)){if(!tracked.has(p))throw Error(`untracked input: ${p}`);const original=execFileSync('git',['show',`HEAD:${p}`],{cwd:root});if(digest(original)!==result[p])throw Error(`dirty input: ${p}`);}
 return result;
}
async function files(dir:string){const out:Record<string,string>={};async function walk(p:string){const s=await lstat(p);if(s.isSymbolicLink())throw Error('payload symlink');if(s.isDirectory())for(const n of await readdir(p))await walk(join(p,n));else if(s.isFile())out[relative(dir,p)]=digest(await readFile(p));else throw Error('unsupported payload');}await walk(dir);return out;}
export async function verifyOutput(dir:string,kind:string){const f=await files(join(dir,'dist'));if('manifest.json' in f)throw Error('served manifest');if(kind==='connect-browser'){if(Object.keys(f).some(p=>p.endsWith('.html')&&!ROUTES.some(r=>p===`${r}/index.html`)))throw Error('unexpected document');for(const r of ROUTES)if(!f[`${r}/index.html`])throw Error('missing route document');for(const r of ROUTES){const html=await readFile(join(dir,'dist',r,'index.html'),'utf8');if(!/<html[\s>]/i.test(html)||!html.trim())throw Error('invalid route document');for(const match of html.matchAll(/<(?:script|img|link)\b[^>]*?\b(?:src|href)=["']([^"']+)["']/gi)){const url=new URL(match[1],`https://artifact.invalid/${r}/`);if(url.origin!=='https://artifact.invalid'||!f[decodeURIComponent(url.pathname.slice(1))])throw Error('missing or external route resource');}}for(const p of Object.keys(f)){if(/\.(html|js|css|json)$/.test(p)){const s=await readFile(join(dir,'dist',p),'utf8');if(/https?:\/\/localhost|https?:\/\/127\.0\.0\.1|phc_synthetic|dist-test|https?:\/\/[^\s"'<>]*synthetic/i.test(s))throw Error('synthetic payload');}}}else if(!f['worker.js']||Object.keys(f).length!==1)throw Error('missing worker');return f;}
export function effectiveEnvironment(kind='connect-browser'){
 if(kind==='connect-worker')return {NODE_ENV:'production'};
 if(process.env.CONNECT_ASSET_SOURCE||process.env.CONNECT_BUILD_DIR)throw Error('noncanonical build override');
 const environment={VITE_HOSTED_API_ORIGIN:process.env.VITE_HOSTED_API_ORIGIN??contract.apiOrigin,PUBLIC_POSTHOG_ENABLED:process.env.PUBLIC_POSTHOG_ENABLED??'true',PUBLIC_POSTHOG_TOKEN:process.env.PUBLIC_POSTHOG_TOKEN??'',PUBLIC_POSTHOG_HOST:process.env.PUBLIC_POSTHOG_HOST??'https://eu.i.posthog.com',PUBLIC_POSTHOG_ENVIRONMENT:process.env.PUBLIC_POSTHOG_ENVIRONMENT??'production',NODE_ENV:process.env.NODE_ENV??'production'};
 if(!['true','false'].includes(environment.PUBLIC_POSTHOG_ENABLED))throw Error('invalid analytics switch');
 if(environment.VITE_HOSTED_API_ORIGIN!==contract.apiOrigin||environment.PUBLIC_POSTHOG_ENVIRONMENT!=='production'||environment.PUBLIC_POSTHOG_HOST!=='https://eu.i.posthog.com'||environment.NODE_ENV!=='production')throw Error('nonproduction environment');
 return environment;
}
export async function seal(root:string,dir:string,kind='connect-browser'){
 if(!['connect-browser','connect-worker'].includes(kind))throw Error('kind');
 const environment=effectiveEnvironment(kind);
 const m={schema:1,kind,sourceSha:execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim(),inputs:await inventory(root,kind),build:{platform:process.platform,arch:process.arch,npm:execFileSync('npm',['--version'],{encoding:'utf8'}).trim(),python:execFileSync('python3',['--version'],{encoding:'utf8'}).trim(),node:process.versions.node,bun:execFileSync('bun',['--version'],{encoding:'utf8'}).trim(),environment},contract,files:await verifyOutput(dir,kind)};
 await writeFile(join(dir,'manifest.json'),JSON.stringify(m,null,2)+'\n',{flag:'wx'});return m;
}
export async function validate(root:string,dir:string,kind='connect-browser'){
 const stat=await lstat(join(dir,'manifest.json'));if(!stat.isFile()||stat.isSymbolicLink())throw Error('manifest file');
 const m=JSON.parse(await readFile(join(dir,'manifest.json'),'utf8'));
 if(m.schema!==1||m.kind!==kind||canonical(m.contract)!==canonical(contract)||!/^[0-9a-f]{40}$/.test(m.sourceSha)||!m.build?.node||!m.build?.bun)throw Error('manifest contract');
 execFileSync('git',['merge-base','--is-ancestor',m.sourceSha,'HEAD'],{cwd:root});
 for(const [path,hash] of Object.entries(m.inputs??{})){if(digest(execFileSync('git',['show',`${m.sourceSha}:${path}`],{cwd:root}))!==hash)throw Error('source input mismatch');}
 const env=m.build.environment;
 if(kind==='connect-worker' ? canonical(env)!==canonical({NODE_ENV:'production'}) : !env||env.VITE_HOSTED_API_ORIGIN!==contract.apiOrigin||env.PUBLIC_POSTHOG_HOST!=='https://eu.i.posthog.com'||env.PUBLIC_POSTHOG_ENVIRONMENT!=='production'||env.NODE_ENV!=='production'||!['true','false'].includes(env.PUBLIC_POSTHOG_ENABLED)||typeof env.PUBLIC_POSTHOG_TOKEN!=='string'||(env.PUBLIC_POSTHOG_TOKEN!==''&&!/^phc_[a-zA-Z0-9_]+$/.test(env.PUBLIC_POSTHOG_TOKEN)))throw Error('build environment mismatch');
 if(canonical(m.inputs)!==canonical(await inventory(root,kind)))throw Error('input mismatch');if(canonical(m.files)!==canonical(await verifyOutput(dir,kind)))throw Error('payload mismatch');
 if(canonical((await readdir(dir)).sort())!==canonical(['dist','manifest.json']))throw Error('artifact extras');return m;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href){const [command,dir,kind='connect-browser',root=resolve(import.meta.dirname,'../..')]=process.argv.slice(2);if(!dir||!['seal','validate'].includes(command))throw Error('usage: artifacts.ts seal|validate ARTIFACT_DIR [connect-browser|connect-worker] [REPO_ROOT]');await (command==='seal'?seal:validate)(root,resolve(dir),kind);}
