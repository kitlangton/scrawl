import type {Env} from './types.ts';
import {ApiError,fail,trustedSiteOrigin} from './common.ts';
import {authRoutes} from './auth.ts';
import {captureRoutes,cleanup} from './captures.ts';
import {shareRoutes} from './shares.ts';
import {renderShare,shareViewScript,shareViewStyles,shareWordmark,shareFont} from './share-view.ts';
import {flushAnalytics} from './analytics.ts';
import {cleanupDailyActivity} from './reporting/aggregate-dau.ts';

export default {
 async fetch(request: Request,env: Env,context?:ExecutionContext) {
  let response;
  try {
   const url=new URL(request.url);
   if(['/share-view.js','/share-view.css','/share-wordmark.svg','/share-font.ttf'].includes(url.pathname)&&['GET','HEAD'].includes(request.method)) {
    let body: BodyInit, type: string;
    if(url.pathname==='/share-view.js'){body=shareViewScript;type='text/javascript; charset=utf-8';}
    else if(url.pathname==='/share-view.css'){body=shareViewStyles;type='text/css; charset=utf-8';}
    else if(url.pathname==='/share-wordmark.svg'){body=shareWordmark;type='image/svg+xml';}
    else{body=Uint8Array.from(atob(shareFont),c=>c.charCodeAt(0));type='font/ttf';}
    response=new Response(request.method==='HEAD'?null:body,{headers:{'Content-Type':type}});
   }else if(request.method==='OPTIONS') {
    if(!trustedSiteOrigin(request.headers.get('Origin'),env))fail('forbidden',403);
    response=new Response(null,{status:204,headers:{'Access-Control-Allow-Methods':'GET, POST, PUT, DELETE, OPTIONS','Access-Control-Allow-Headers':'Authorization, Content-Type, X-CSRF-Token','Access-Control-Max-Age':'600'}});
   }else response=await authRoutes(request,env,url)||await shareRoutes(request,env,url,renderShare,context)||await captureRoutes(request,env,url,context);
   if(!response)fail('not_found',404);
  }catch(e){response=Response.json({error:{code:e instanceof ApiError?e.code:'internal_error',message:e instanceof ApiError?e.message:'Request failed'}},{status:e instanceof ApiError?e.status:500});}
  const headers=new Headers(response.headers);
  headers.set('Cache-Control','no-store');headers.set('X-Content-Type-Options','nosniff');headers.set('Referrer-Policy','no-referrer');headers.set('Vary','Origin');
  headers.set('X-Robots-Tag','noindex, nofollow, noarchive');
  const origin=request.headers.get('Origin');
  if(trustedSiteOrigin(origin,env)){headers.set('Access-Control-Allow-Origin',origin);headers.set('Access-Control-Allow-Credentials','true');}
  return new Response(response.body,{status:response.status,headers});
 },
 async scheduled(_controller: ScheduledController,env: Env) {
  await Promise.all([(async()=>{await cleanup(env);await flushAnalytics(env);})(),cleanupDailyActivity(env)]);
 }
};
