import {test} from 'node:test';
import {strict as assert} from 'node:assert';
import {mkdtemp,rm,mkdir,writeFile,readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';

test('packed CLI installs offline outside checkout and inspects without loading native dependencies',async()=>{
  const root=await mkdtemp(join(tmpdir(),'agent-package-'));
  try {
    const npm=process.env.npm_execpath!; assert(npm);
    const run=(args:string[],cwd=root)=>{
      const result=spawnSync(process.execPath,args,{cwd,encoding:'utf8',timeout:30000,env:{...process.env,APPARENT_ENGINE_EXECUTABLE:'/not-installed/engine',APPARENT_ENGINE_RUNTIME:'/not-installed/runtime'}});
      assert.equal(result.status,0,result.stderr);return result.stdout;
    };
    const source=fileURLToPath(new URL('..',import.meta.url));
    const metadata=JSON.parse(run([npm,'pack','--ignore-scripts','--json','--pack-destination',root],source));
    run([npm,'install','--offline','--ignore-scripts','--no-audit','--no-fund','--prefix',root,join(root,metadata[0].filename)]);
    const cli=resolve(root,'node_modules/@apparent/agent/dist/cli.js');
    assert.equal(run(['--input-type=module','-e',"await import('@apparent/agent')"]),'');
    assert.match(run([cli,'--help']),/Local Apparent/);assert.equal(run([cli,'--version']),'0.1.0\n');
    const bundle=join(root,'fixture.apparent');await mkdir(bundle);
    const raw=JSON.stringify({version:1,id:'fixture',createdAtMs:1,durationMs:1,target:{mode:'display',app:null,bundleId:null,windowTitle:null,windowId:null,displayId:0,frame:{x:0,y:0,width:800,height:600,scale:1}},transcript:{text:'hello',segments:[]},strokes:[],marks:[],events:[],keyframes:[],contactSheet:'sheet.png'});
    await writeFile(join(bundle,'apparent.json'),raw);await writeFile(join(bundle,'summary.md'),'hello\n');
    assert.match(run([cli,'inspect',bundle]),/“hello”/);assert.equal(run([cli,'inspect',bundle,'--stored-summary']),'hello\n');assert.equal(run([cli,'inspect',bundle,'--json']),raw);
    assert.equal(await readFile(join(bundle,'apparent.json'),'utf8'),raw);
  }finally{await rm(root,{recursive:true,force:true});}
});
