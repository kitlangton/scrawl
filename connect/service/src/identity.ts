import {betterAuth, APIError} from 'better-auth';
import {genericOAuth} from 'better-auth/plugins';
import {decodeJwt} from 'jose';
import {oauthProvider,oauthDeviceAuthorization,DEVICE_CODE_GRANT_TYPE} from '@better-auth/oauth-provider';
import type {Env} from './types.ts';
import {admit,fail,hash,random} from './common.ts';

// Only the v1 adapters and the Google callback are exposed by the Worker.
// In particular, server-only client administration never becomes a public route.
export function identity(env: Env) {
 if(typeof env.BETTER_AUTH_SECRET!=='string'||env.BETTER_AUTH_SECRET.length<32)fail('auth_unavailable',503);
 return betterAuth({
  appName:'Apparent Connect',baseURL:env.API_ORIGIN,basePath:'/auth',secret:env.BETTER_AUTH_SECRET,
  database:env.DB,
  logger:{disabled:true},
  onAPIError:{throw:true},
  trustedOrigins:[env.SITE_ORIGIN,...env.LEGACY_SITE_ORIGIN?[env.LEGACY_SITE_ORIGIN]:[]],
  user:{modelName:'auth_user'},account:{modelName:'auth_account',accountLinking:{enabled:false},encryptOAuthTokens:true},
  session:{modelName:'auth_session',expiresIn:7*86400,cookieCache:{enabled:false}},
  verification:{modelName:'auth_verification'},
  databaseHooks:{
   // 1.7.15 encrypts provider access/refresh, but still stores ID tokens raw.
   // Verification has already finished; this app needs no retained ID token.
   account:{create:{before:async account=>({data:{...account,idToken:null}})},update:{before:async account=>({data:{...account,idToken:null}})}},
   session:{create:{after:async session=>{await admit(env,session.userId);}}},
  },
  advanced:{useSecureCookies:true,cookiePrefix:'apparent',defaultCookieAttributes:{httpOnly:true,secure:true,sameSite:'lax',path:'/'}},
  plugins:[genericOAuth({config:[{providerId:'google',clientId:env.GOOGLE_CLIENT_ID,clientSecret:env.GOOGLE_CLIENT_SECRET,
   discoveryUrl:env.OAUTH_DISCOVERY_URL||'https://accounts.google.com/.well-known/openid-configuration',
   redirectURI:env.API_ORIGIN+'/auth/google/callback',scopes:['openid','email','profile'],pkce:true,requireIdTokenVerification:true,
   async getUserInfo(tokens) {
    // In 1.7.15 genericOAuth verifies a present ID token (including nonce)
    // BEFORE this hook, but otherwise falls back to UserInfo. Require the token
    // here and read only those already-verified claims, before any admission.
    if(!tokens.idToken)return null;
    const profile=decodeJwt(tokens.idToken);
    if(profile.azp!==undefined&&profile.azp!==env.GOOGLE_CLIENT_ID)return null;
    if(typeof profile.email!=='string')return null;
    return {...profile,email:profile.email,emailVerified:profile.email_verified===true,
     name:typeof profile.name==='string'?profile.name:undefined,image:typeof profile.picture==='string'?profile.picture:undefined};
   },
   mapProfileToUser(profile) {
    if(profile.email_verified!==true||typeof profile.email!=='string'||profile.email.length>254)throw new APIError('UNAUTHORIZED',{message:'Verified email required'});
    return {name:typeof profile.name==='string'?profile.name:profile.email};
   }
  }]}),oauthProvider({
   loginPage:env.SITE_ORIGIN+'/connect',consentPage:env.SITE_ORIGIN+'/connect/device',
   scopes:['read','upload','offline_access'],grantTypes:['authorization_code',DEVICE_CODE_GRANT_TYPE,'refresh_token'],
   disableJwtPlugin:true,accessTokenExpiresIn:3600,refreshTokenExpiresIn:30*86400,
   refreshTokenReuseInterval:30,storeTokens:{hash:token=>hash(token)},
   allowDynamicClientRegistration:true,allowUnauthenticatedClientRegistration:true,clientPrivileges:()=>false,
  }),oauthDeviceAuthorization({expiresIn:'10m',interval:'5s',generateDeviceCode:()=>random(),generateUserCode:()=>random(6).toUpperCase(),
   verificationUri:env.SITE_ORIGIN+'/connect/device',schema:{deviceCode:{modelName:'auth_device'}}})],
 });
}
export {DEVICE_CODE_GRANT_TYPE};
