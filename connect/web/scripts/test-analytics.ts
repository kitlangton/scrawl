// Cross-owner integration proof. Separate synthetic outputs never replace dist.
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {resolve} from 'node:path'
const web=fileURLToPath(new URL('../',import.meta.url)),root=resolve(web,'../..')
function run(command:string,args:string[],cwd:string,extra:Record<string,string>={}) {
 const env={...process.env,...extra}
 delete env.PUBLIC_POSTHOG_ENABLED // Exercise the enabled default, not an explicit test opt-in.
 const result=spawnSync(command,args,{cwd,env,stdio:'inherit'})
 if(result.error)throw result.error
 if(result.status!==0)throw new Error(`${command} ${args.join(' ')} failed (${result.status})`)
}
const synthetic={PUBLIC_POSTHOG_TOKEN:'phc_synthetic',PUBLIC_POSTHOG_HOST:'https://eu.i.posthog.com',PUBLIC_POSTHOG_ENVIRONMENT:'synthetic'}
run(process.execPath,['--test','shared/analytics/config.test.ts'],root)
run('npm',['run','build'],web,{...synthetic,VITE_HOSTED_API_ORIGIN:'',CONNECT_BUILD_DIR:'dist-analytics-test'})
const site=resolve(root,'public/site')
run(resolve(site,'node_modules/.bin/tsc'),['-b'],site,synthetic)
run(resolve(site,'node_modules/.bin/vite'),['build','--outDir','dist-analytics-test'],site,{...synthetic,APPARENT_PUBLIC_SITE:'1'})
run(process.execPath,['test/analytics-transport.browser.ts'],web,synthetic)
