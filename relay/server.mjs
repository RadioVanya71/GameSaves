import http from 'node:http';
import { DatabaseSync } from 'node:sqlite';
import { randomBytes, createHash, createCipheriv, createDecipheriv } from 'node:crypto';
import { readFileSync } from 'node:fs';

const env = process.env;
for (const key of ['PUBLIC_ORIGIN','GOOGLE_CLIENT_ID','GOOGLE_CLIENT_SECRET','PICKER_KEY','PROJECT_NUMBER','STORE_KEY','DATABASE']) {
  if (!env[key]) throw Error('Missing ' + key);
}
const origin = new URL(env.PUBLIC_ORIGIN).origin;
if (!origin.startsWith('https://')) throw Error('PUBLIC_ORIGIN requires HTTPS');
const secret = Buffer.from(env.STORE_KEY, 'hex');
if (secret.length !== 32) throw Error('STORE_KEY requires 32 random bytes in hex');
const db = new DatabaseSync(env.DATABASE);
db.exec(`PRAGMA journal_mode=WAL;
  CREATE TABLE IF NOT EXISTS groups (id TEXT PRIMARY KEY, folder TEXT NOT NULL, credential TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS versions (grp TEXT NOT NULL, name TEXT NOT NULL, file TEXT NOT NULL, PRIMARY KEY(grp,name));`);
const sessions = new Map();
const nonce = () => randomBytes(32).toString('base64url');
const hash = value => createHash('sha256').update(value).digest('hex');
const limit = 32 * 1024 * 1024;
const namePattern = /^save-[0-9]{15}-[A-Za-z0-9]{1,32}\.zip$/;
const callback = origin + '/callback';
const drive = 'https://www.googleapis.com/drive/v3/files';
const picker = readFileSync(new URL('picker.html', import.meta.url),'utf8');
function seal(token) {
  const iv = randomBytes(12);
  const cipher = createCipheriv('aes-256-gcm',secret,iv);
  const data = Buffer.concat([cipher.update(token,'utf8'),cipher.final()]);
  return Buffer.concat([iv,cipher.getAuthTag(),data]).toString('base64');
}
function open(value) {
  const b = Buffer.from(value,'base64');
  const cipher = createDecipheriv('aes-256-gcm',secret,b.subarray(0,12));
  cipher.setAuthTag(b.subarray(12,28));
  return Buffer.concat([cipher.update(b.subarray(28)),cipher.final()]).toString();
}
function fail(status) { throw Object.assign(Error('Request failed'),{status}); }
async function google(url, opts) {
  const result = await fetch(url,{...opts,signal:AbortSignal.timeout(120000)});
  if (!result.ok) fail(result.status === 403 ? 403 : 502);
  return result;
}
async function tokens(fields) {
  const body = new URLSearchParams({...fields,client_id:env.GOOGLE_CLIENT_ID,client_secret:env.GOOGLE_CLIENT_SECRET});
  return (await google('https://oauth2.googleapis.com/token',{method:'POST',body})).json();
}
async function body(req,max) {
  if (Number(req.headers['content-length']) > max) fail(413);
  const chunks = []; let size = 0;
  for await (const chunk of req) { size += chunk.length; if (size > max) fail(413); chunks.push(chunk); }
  return Buffer.concat(chunks);
}
setInterval(()=>{for(const [key,s] of sessions) if(s.until<Date.now()) sessions.delete(key);},60000).unref();

async function serve(req,res) {
  const url = new URL(req.url,origin);
  res.setHeader('Cache-Control','no-store');
  res.setHeader('Referrer-Policy','no-referrer');
  res.setHeader('X-Content-Type-Options','nosniff');
  res.setHeader('X-Frame-Options','DENY');
  const json = data => { res.setHeader('Content-Type','application/json');res.end(JSON.stringify(data)); };
  const page = data => { res.setHeader('Content-Type','text/html; charset=utf-8');res.end(data); };
  if (url.pathname === '/join') return page('<meta charset=utf-8><p>Скопируй полный адрес этой страницы в поле приглашения GameSaves.</p>');
  if (url.pathname === '/create' && req.method === 'GET') {
    if(sessions.size >= 5000) fail(429);
    const key=nonce(),state=nonce();
    sessions.set(key,{state,until:Date.now()+600000});
    res.setHeader('Set-Cookie','gs='+key+'; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=600');
    const query=new URLSearchParams({client_id:env.GOOGLE_CLIENT_ID,redirect_uri:callback,response_type:'code',scope:'https://www.googleapis.com/auth/drive.file',state,access_type:'offline',prompt:'consent'});
    res.writeHead(302,{Location:'https://accounts.google.com/o/oauth2/v2/auth?'+query});return res.end();
  }
  const sid=(req.headers.cookie||'').match(/(?:^|; )gs=([A-Za-z0-9_-]{43})(?:;|$)/)?.[1];
  const s=sessions.get(sid);
  if (['/callback','/picker','/folder'].includes(url.pathname)) {
    if (!s || s.until < Date.now()) fail(401);
    if(url.pathname === '/callback') {
      if(!s.state || s.state!==url.searchParams.get('state') || !url.searchParams.get('code')) fail(400);
      delete s.state;
      s.oauth=await tokens({code:url.searchParams.get('code'),redirect_uri:callback,grant_type:'authorization_code'});
      if(!s.oauth.refresh_token) fail(401);
      res.writeHead(303,{Location:'/picker'});return res.end();
    }
    if(!s.oauth) fail(401);
    if(url.pathname === '/picker') {
      res.setHeader('Referrer-Policy','strict-origin');
      const config=JSON.stringify([env.PROJECT_NUMBER,env.PICKER_KEY,s.oauth.access_token]).replaceAll('<','\\u003c');
      return page(picker.replace('/*CONFIG*/',()=>config));
    }
    if(req.method!=='POST' || req.headers.origin!==origin) fail(403);
    const folder=(await body(req,256)).toString();
    if(!/^[a-zA-Z0-9_-]{1,200}$/.test(folder)) fail(400);
    const metadata=await (await google(drive+'/'+folder+'?supportsAllDrives=true&fields=mimeType,trashed,capabilities(canAddChildren)',{headers:{Authorization:'Bearer '+s.oauth.access_token}})).json();
    if(metadata.trashed || metadata.mimeType!=='application/vnd.google-apps.folder' || !metadata.capabilities?.canAddChildren) fail(403);
    const token=nonce();
    db.prepare('INSERT INTO groups VALUES(?,?,?)').run(hash(token),folder,seal(s.oauth.refresh_token));
    sessions.delete(sid);
    return json({invite:origin+'/join#'+token});
  }
  const parts=url.pathname.split('/');
  if(parts[1]!=='versions' || parts.length>3) fail(404);
  const token=req.headers.authorization?.match(/^Bearer ([a-zA-Z0-9_-]{43})$/)?.[1];
  if(!token) fail(401);
  const group=db.prepare('SELECT * FROM groups WHERE id=?').get(hash(token));
  if(!group) fail(403);
  if(parts.length===2 && req.method==='GET') return json(db.prepare('SELECT name FROM versions WHERE grp=? ORDER BY name DESC').all(group.id).map(row=>row.name));
  const name=parts[2];
  if(!namePattern.test(name||'')) fail(400);
  const existing=db.prepare('SELECT file FROM versions WHERE grp=? AND name=?').get(group.id,name);
  if(req.method==='GET' && !existing) fail(404);
  if(req.method==='PUT' && existing) fail(409);
  if(!['GET','PUT'].includes(req.method)) fail(405);
  const {access_token}=await tokens({grant_type:'refresh_token',refresh_token:open(group.credential)});
  const headers={Authorization:'Bearer '+access_token};
  if(req.method==='GET') {
    const response=await google(drive+'/'+existing.file+'?alt=media&supportsAllDrives=true',{headers});
    const chunks=[];let size=0;
    for await(const b of response.body){size+=b.length;if(size>limit) fail(413);chunks.push(b);}
    res.setHeader('Content-Type','application/zip');return res.end(Buffer.concat(chunks));
  }
  const archive=await body(req,limit);
  if(archive.length<22 || archive.readUInt32LE(0)!==0x04034b50) fail(400);
  const boundary=nonce();
  const meta={name,parents:[group.folder],mimeType:'application/zip'};
  const pre='--'+boundary+'\r\nContent-Type: application/json\r\n\r\n'+JSON.stringify(meta)+'\r\n--'+boundary+'\r\nContent-Type: application/zip\r\n\r\n';
  const multi=Buffer.concat([Buffer.from(pre),archive,Buffer.from('\r\n--'+boundary+'--\r\n')]);
  const saved=await (await google('https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&supportsAllDrives=true&fields=id',{method:'POST',headers:{...headers,'Content-Type':'multipart/related; boundary='+boundary},body:multi})).json();
  db.prepare('INSERT INTO versions VALUES(?,?,?)').run(group.id,name,saved.id);
  res.writeHead(201);res.end();
}
export const server=http.createServer((req,res)=>serve(req,res).catch(e=>{res.writeHead(e.status||500);res.end('Не удалось выполнить запрос.');})).listen(Number(env.PORT||8080));
