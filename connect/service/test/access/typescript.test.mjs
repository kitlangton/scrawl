import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile,readdir} from 'node:fs/promises';

test('strict TypeScript checks gate the deployed Worker build',async()=>{
 const pkg=JSON.parse(await readFile('package.json','utf8'));
 const config=JSON.parse(await readFile('tsconfig.json','utf8'));
 assert.equal(config.compilerOptions.strict,true);
 assert.equal(config.compilerOptions.noEmit,true);
 assert.equal(pkg.scripts.typecheck,'tsc --noEmit');
 assert.match(pkg.scripts.build,/^npm run build:recipient && npm run typecheck && esbuild src\/worker\.ts\b/);
 assert.equal(pkg.scripts['build:recipient'],'node scripts/build-recipient.mjs');
 assert.match(pkg.scripts.build,/--outfile=dist\/worker\.js\b/);
 for(const name of ['test','dev','deploy'])assert.match(pkg.scripts[name],/^npm run build && /);
 assert.deepEqual((await readdir('src')).filter(name=>name.endsWith('.js')),[]);
});
