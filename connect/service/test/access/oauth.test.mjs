import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {generateKeyPair,exportJWK,SignJWT} from 'jose';
import {fixture,digest} from '../helpers.mjs';

async function provider() {
 const {privateKey,publicKey}=await generateKeyPair('RS256'),other=await generateKeyPair('RS256');
 const jwk={...await exportJWK(publicKey),kid:'test',alg:'RS256',use:'sig'},codes=new Map(),issued=new Map(),profiles=new Map();let issuer;
 const server=createServer(async(req,res)=>{
  if(req.url==='/.well-known/openid-configuration'){res.setHeader('Content-Type','application/json');res.end(JSON.stringify({issuer,authorization_endpoint:issuer+'/authorize',token_endpoint:issuer+'/token',userinfo_endpoint:issuer+'/userinfo',jwks_uri:issuer+'/jwks',id_token_signing_alg_values_supported:['RS256']}));return;}
  if(req.url==='/jwks'){res.setHeader('Content-Type','application/json');res.end(JSON.stringify({keys:[jwk]}));return;}
  if(req.url==='/userinfo'){
   const profile=profiles.get(req.headers.authorization);
   res.writeHead(profile?200:401,{'Content-Type':'application/json'});res.end(JSON.stringify(profile||{}));return;
  }
  if(req.url==='/token'){
   let raw='';for await(const c of req)raw+=c;
   const params=new URLSearchParams(raw),c=codes.get(params.get('code'));codes.delete(params.get('code'));
   if(!c||params.get('client_secret')!=='controlled-secret'||!params.get('code_verifier')){res.writeHead(400);res.end('{}');return;}
   const claims={nonce:c.nonce,email:c.email||c.sub+'@example.com',email_verified:true,...c.claims};
   const token=await new SignJWT(claims).setProtectedHeader({alg:'RS256',kid:'test'}).setIssuer(c.issuer||issuer).setSubject(c.sub).setAudience(c.aud||'test-client').setIssuedAt().setExpirationTime(c.exp||'5m').sign(c.badKey?other.privateKey:privateKey);
   const accessToken=c.missingIdToken?'controlled-userinfo-access-'+c.sub:'controlled-provider-access';
   profiles.set('Bearer '+accessToken,{sub:c.sub,...claims});
   issued.set(c.sub,token);res.setHeader('Content-Type','application/json');res.end(JSON.stringify({...c.missingIdToken?{}:{id_token:token},access_token:accessToken,refresh_token:'controlled-provider-refresh',expires_in:3600,token_type:'Bearer'}));return;
  }
  res.writeHead(404);res.end();
 });
 await new Promise(r=>server.listen(0,'127.0.0.1',r));issuer='http://127.0.0.1:'+server.address().port;
 return {bindings:{GOOGLE_CLIENT_ID:'test-client',GOOGLE_CLIENT_SECRET:'controlled-secret',OAUTH_DISCOVERY_URL:issuer+'/.well-known/openid-configuration'},codes,issued,close:()=>new Promise(r=>server.close(r))};
}
async function login(f,provider,sub,options={}) {
 const start=await f.call('/auth/google/start?returnTo='+encodeURIComponent(options.returnTo||'/hosted/device?code=ABCDEF123456'));
 assert.equal(start.status,302);const u=new URL(start.headers.get('Location'));
 assert.equal(u.searchParams.get('code_challenge_method'),'S256');
 const code='code-'+sub;provider.codes.set(code,{sub,nonce:u.searchParams.get('nonce'),...options});
 const cookie=start.headers.get('Set-Cookie').split(';')[0];
 if(options.storedReturnTo!==undefined){
  const state=await f.db.prepare('SELECT * FROM auth_verification WHERE identifier=?').bind('auth-state:'+u.searchParams.get('state')).first();
  const value=JSON.parse(state.value);value.callbackURL=options.storedReturnTo.startsWith('/')&&!options.storedReturnTo.startsWith('//')?'https://app.tryapparent.com'+options.storedReturnTo:new URL(options.storedReturnTo,'https://app.tryapparent.com').href;
  await f.db.prepare('UPDATE auth_verification SET value=? WHERE id=?').bind(JSON.stringify(value),state.id).run();
 }
 const callback='/auth/google/callback?state='+u.searchParams.get('state')+'&code='+code;
 const response=await f.call(callback,{cookie});
 return {response,callback,cookie};
}
const sessionCookie=r=>r.headers.get('Set-Cookie').match(/__Secure-apparent\.session_token=[^;, ]+/)[0];

test('Connect canonical and legacy OAuth destinations normalize, including library-owned stored state',async()=>{
 const p=await provider(),f=await fixture({...p.bindings,SITE_ORIGIN:'https://app.tryapparent.com',API_ORIGIN:'https://hosted.tryapparent.com',LEGACY_SITE_ORIGIN:'https://hosted.tryapparent.com'});try{
 const start=await f.call('/auth/google/start');
 assert.equal(new URL(start.headers.get('Location')).searchParams.get('redirect_uri'),'https://hosted.tryapparent.com/auth/google/callback');
 let n=0;
 for(const path of ['/connect','/hosted','/connect/device?code=ABCDEF123456','/hosted/device?code=ABCDEF123456']) {
  for(const stored of [false,true]) {
   const {response}=await login(f,p,'route'+n++,stored?{storedReturnTo:path}:{returnTo:path});
   assert.equal(response.status,302);
   assert.equal(response.headers.get('Location'),'https://app.tryapparent.com'+path.replace('/hosted','/connect'));
   assert.doesNotMatch(response.headers.get('Set-Cookie'),/Domain=/i);
  }
 }
 for(const path of ['https://evil.example/connect','//evil.example/connect','/connect?next=evil','/connect#bad','/connect/device?code=A&next=evil','/connect/device?code=A&code=B','/connect/../connect']) {
  assert.equal((await f.call('/auth/google/start?returnTo='+encodeURIComponent(path))).status,400,path);
  assert.equal((await login(f,p,'bad'+n++,{storedReturnTo:path})).response.status,401,path);
 }
 for(const scope of ['read','upload']) {
  const device=await(await f.call('/v1/device/start',{method:'POST',body:{scope}})).json();
  assert.equal(device.verificationUri,'https://app.tryapparent.com/connect/device?code='+device.userCode);
 }
 }finally{await f.close();await p.close();}
});

test('Connect CORS and CSRF share exact explicit origins; device scope remains immutable',async()=>{
 const canonical='https://app.tryapparent.com',legacy='https://hosted.tryapparent.com';
 const f=await fixture({SITE_ORIGIN:canonical,LEGACY_SITE_ORIGIN:legacy});try{
 const a=await f.identity('connect');
 for(const origin of [canonical,legacy,'https://evil.example','null',undefined,canonical+'/',canonical+'.evil.example','http://app.tryapparent.com']) {
  const allowed=[canonical,legacy].includes(origin);
  const preflight=await f.call('/v1/waitlist',{method:'OPTIONS',origin});
  assert.equal(preflight.status,allowed?204:403,String(origin));
  for(const response of [preflight,await f.call('/v1/session',{origin}),await f.call('/v1/waitlist',{...a,origin,method:'POST',body:{interestedAtFiveDollars:true}})]) {
   assert.equal(response.headers.get('Access-Control-Allow-Origin'),allowed?origin:null,String(origin));
   assert.equal(response.headers.get('Access-Control-Allow-Credentials'),allowed?'true':null);
  }
  assert.equal((await f.call('/v1/waitlist',{...a,origin,method:'POST',body:{interestedAtFiveDollars:false}})).status,allowed?200:403);
  for(const path of ['/auth/logout','/v1/waitlist','/v1/device/approve','/v1/device/reject']) {
   // Even a trusted origin needs CSRF; an untrusted origin cannot use valid CSRF.
   assert.equal((await f.call(path,{...a,origin,csrf:allowed?'wrong':a.csrf,method:'POST',body:{userCode:'ABCDEF123456',interestedAtFiveDollars:true}})).status,403,path+' '+String(origin));
  }
 }
 for(const origin of [canonical,legacy])for(const scope of ['read','upload']) {
  const d=await(await f.call('/v1/device/start',{method:'POST',body:{scope}})).json();
  assert.equal((await f.call('/v1/device/approve',{...a,origin,method:'POST',body:{userCode:d.userCode,scope:scope==='read'?'upload':'read'}})).status,200);
  const token=await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:d.deviceCode}})).json();
  assert.equal(token.scope,scope);
  assert.deepEqual(JSON.parse((await f.db.prepare('SELECT scopes FROM oauthAccessToken WHERE token=?').bind(digest(token.accessToken)).first()).scopes),[scope,'offline_access']);
 }
 }finally{await f.close();}
 const only=await fixture({SITE_ORIGIN:canonical});try{assert.equal((await only.call('/v1/session',{method:'OPTIONS',origin:legacy})).status,403);}finally{await only.close();}
});

test('controlled OAuth signed provider: concurrent admission max20, overflow waitlist, cookie session and no auto-approval',async()=>{
 const p=await provider(),f=await fixture(p.bindings);try{
 const pending=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'upload'}})).json();
 const results=await Promise.all(Array.from({length:22},(_,i)=>login(f,p,'person'+i)));
 for(const {response} of results)assert.equal(response.status,302);
 const states=await Promise.all(results.map(({response})=>f.call('/v1/session',{cookie:sessionCookie(response)}).then(r=>r.json())));
 assert.equal(states.filter(s=>s.account.beta==='active').length,20);assert.equal(states.filter(s=>s.account.beta==='waitlist').length,2);
 assert.deepEqual(states[0].slots,{used:20,limit:20,remaining:0});
 assert.equal((await f.db.prepare('SELECT status FROM auth_device').first()).status,'pending');
 const index=states.findIndex(s=>s.account.beta==='waitlist'),cookie=sessionCookie(results[index].response),csrf=states[index].csrfToken;
 assert.equal((await f.call('/v1/waitlist',{cookie,csrf,origin:'https://site.example',method:'POST',body:{interestedAtFiveDollars:true}})).status,200);
 assert.equal((await(await f.call('/v1/session',{cookie})).json()).account.waitlist.interestedAtFiveDollars,true);
 assert.equal((await f.call('/v1/device/approve',{cookie,csrf,origin:'https://site.example',method:'POST',body:{userCode:pending.userCode}})).status,403);
 const replay=await f.call(results[0].callback,{cookie:results[0].cookie});assert.equal(replay.status,401);
 assert.match(results[0].response.headers.get('Set-Cookie'),/HttpOnly/i);assert.match(results[0].response.headers.get('Set-Cookie'),/Secure/i);assert.match(results[0].response.headers.get('Set-Cookie'),/SameSite=Lax/i);
 assert.equal(results[0].response.headers.get('Location'),'https://site.example/connect/device?code=ABCDEF123456');
 }finally{await f.close();await p.close();}
});

test('controlled OAuth rejects unverified email, wrong nonce/issuer/audience/expiry/signature and state',async()=>{
 const p=await provider(),f=await fixture(p.bindings);try{
 let n=0;
 for(const options of [{claims:{email_verified:false}},{claims:{nonce:'wrong'}},{issuer:'https://evil.example'},{aud:'other-client'},{exp:'-1h'},{badKey:true}]) {
  const {response}=await login(f,p,'invalid'+n++,options);await rejectedLogin(f,response);
 }
 assert.equal((await f.call('/auth/google/callback?state=wrong&code=stolen',{cookie:'__Host-apparent_oauth=other'})).status,401);
 assert.equal((await f.call('/auth/google/start?returnTo=https://evil.example/hosted')).status,400);
 assert.equal((await f.call('/auth/google/start?returnTo=/hosted?token=oops')).status,400);
 assert.equal((await f.db.prepare('SELECT count(*) AS n FROM accounts').first()).n,0);
 const anonymous=await(await f.call('/v1/session')).json();assert.deepEqual(anonymous,{authenticated:false,csrfToken:null,account:null,slots:{used:0,limit:20,remaining:20}});
 }finally{await f.close();await p.close();}
});

async function rejectedLogin(f,response) {
 assert.doesNotMatch(response.headers.get('Set-Cookie')||'',/__Secure-apparent\.session_token=/);
 for(const table of ['auth_session','auth_user','auth_account','accounts'])assert.equal((await f.db.prepare('SELECT count(*) AS n FROM '+table).first()).n,0,table+' must remain empty');
 assert.equal((await(await f.call('/v1/session')).json()).authenticated,false);
 assert.equal(response.status,401);assert.deepEqual(await response.json(),{error:{code:'oauth_failed',message:'oauth failed'}});
}

for(const [name,options] of [
 ['missing ID token despite authenticated UserInfo',{missingIdToken:true}],
 ['wrong authorized party with multiple audiences',{aud:['test-client','another-client'],claims:{azp:'another-client'}}],
 ['wrong authorized party with single audience',{claims:{azp:'another-client'}}],
])test('controlled OAuth rejects '+name+' before admission',async()=>{
 const p=await provider(),f=await fixture(p.bindings);try{
  const {response}=await login(f,p,'policy-invalid',options);await rejectedLogin(f,response);
 }finally{await f.close();await p.close();}
});

test('controlled OAuth accepts signed nonce-bound tokens with absent or matching authorized party',async()=>{
 const p=await provider(),f=await fixture(p.bindings);try{
  let n=0;
  for(const options of [{},{claims:{azp:'test-client'}},{aud:['test-client','another-client'],claims:{azp:'test-client'}}]) {
   const {response}=await login(f,p,'policy-valid'+n++,options);assert.equal(response.status,302);
   assert.equal((await(await f.call('/v1/session',{cookie:sessionCookie(response)})).json()).authenticated,true);
  }
  for(const table of ['auth_session','auth_user','auth_account','accounts'])assert.equal((await f.db.prepare('SELECT count(*) AS n FROM '+table).first()).n,3,table);
 }finally{await f.close();await p.close();}
});

test('existing issuer/subject retains account, captures, preference and budgets through real library login',async()=>{
 const p=await provider(),f=await fixture(p.bindings);try{
 const account=digest('https://accounts.google.com/incumbent'),capture='c'.repeat(32),now=Date.now();
 await f.db.prepare("INSERT INTO accounts(id,email,beta,interest,created_at) VALUES(?,?,'active',1,1)").bind(account,'old@example.com').run();
 const manifest=JSON.stringify([{path:'summary.md',size:0,sha256:digest('')}]);
 await f.db.prepare('INSERT INTO captures(id,account_id,manifest,bytes,expires_at,pending_until) VALUES(?,?,?,0,?,?)').bind(capture,account,manifest,now+86400000,now+86400000).run();
 await f.db.exec('UPDATE r2_budget SET a_used=7,b_used=11');const budget=await f.db.prepare('SELECT * FROM r2_budget').first();
 const {response}=await login(f,p,'incumbent',{claims:{email:'new@example.com'}});assert.equal(response.status,302);
 const providerAccount=await f.db.prepare("SELECT idToken,accessToken,refreshToken FROM auth_account WHERE providerId='google'").first();
 assert.ok(p.issued.get('incumbent'));assert.equal(providerAccount.idToken,null,'discard the verified ID token after use');
 assert.ok(providerAccount.accessToken);assert.notEqual(providerAccount.accessToken,'controlled-provider-access');assert.ok(providerAccount.refreshToken);assert.notEqual(providerAccount.refreshToken,'controlled-provider-refresh');
 const cookie=sessionCookie(response),s=await(await f.call('/v1/session',{cookie})).json();assert.equal(s.authenticated,true);assert.equal(s.account.email,'new@example.com');
 assert.deepEqual(await f.db.prepare('SELECT * FROM accounts WHERE id=?').bind(account).first(),{id:account,email:'new@example.com',beta:'active',interest:1,created_at:1});
 assert.equal((await f.db.prepare('SELECT count(*) n FROM accounts').first()).n,1);
 const d=await(await f.call('/v1/device/start',{method:'POST',body:{scope:'read'}})).json();
 assert.equal((await f.call('/v1/device/approve',{cookie,csrf:s.csrfToken,origin:'https://site.example',method:'POST',body:{userCode:d.userCode}})).status,200);
 const t=await(await f.call('/v1/device/poll',{method:'POST',body:{deviceCode:d.deviceCode}})).json();
 assert.equal((await f.call('/v1/captures/'+capture,{token:t.accessToken})).status,200);
 assert.equal((await f.db.prepare('SELECT account_id FROM captures WHERE id=?').bind(capture).first()).account_id,account);
 assert.deepEqual(await f.db.prepare('SELECT * FROM r2_budget').first(),budget);
 }finally{await f.close();await p.close();}
});
