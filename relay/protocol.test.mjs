import {test} from 'node:test';
import assert from 'node:assert/strict';
import {randomBytes} from 'node:crypto';
import {once} from 'node:events';
test('owner selects folder, invited friend reads an uploaded version, other groups are isolated',async t=>{
  const origin='https://groups.example';
  Object.assign(process.env,{PORT:'0',DATABASE:':memory:',STORE_KEY:randomBytes(32).toString('hex'),PUBLIC_ORIGIN:origin,PROJECT_NUMBER:'123',GOOGLE_CLIENT_ID:'test',GOOGLE_CLIENT_SECRET:'test',PICKER_KEY:'test'});
  const realFetch=globalThis.fetch;
  const zip=Buffer.alloc(32);zip.writeUInt32LE(0x04034b50);
  t.mock.method(globalThis,'fetch',async(url,options)=>{
    if(url==='https://oauth2.googleapis.com/token') return Response.json({access_token:'access',refresh_token:'refresh'});
    assert.equal(options.headers.Authorization,'Bearer access');
    if(url.includes('/upload/')) {assert.ok(options.body.includes(zip));return Response.json({id:'save-id'});}
    if(url.includes('alt=media')) return new Response(zip);
    if(url.includes('/folder-id?')) return Response.json({mimeType:'application/vnd.google-apps.folder',capabilities:{canAddChildren:true}});
    throw Error('unexpected request');
  });
  const {server}=await import('./server.mjs');
  if(!server.listening)await once(server,'listening');
  t.after(()=>server.close());
  const request=(path,opts={})=>realFetch('http://localhost:'+server.address().port+path,{...opts,redirect:'manual'});
  async function create(){
    const start=await request('/create');
    const cookie=start.headers.get('set-cookie').split(';')[0];
    const state=new URL(start.headers.get('location')).searchParams.get('state');
    assert.equal((await request('/callback?state=wrong&code=x',{headers:{cookie}})).status,400);
    assert.equal((await request('/callback?state='+state+'&code=x',{headers:{cookie}})).status,303);
    const r=await request('/folder',{method:'POST',headers:{cookie,origin},body:'folder-id'});
    const invite=(await r.json()).invite;
    return {Authorization:'Bearer '+new URL(invite).hash.slice(1)};
  }
  const headers=await create(), other=await create();
  const name='save-001234567890123-abc.zip',path='/versions/'+name;
  assert.equal((await request('/versions')).status,401);
  assert.equal((await request(path,{method:'PUT',headers,body:zip})).status,201);
  assert.deepEqual(await (await request('/versions',{headers})).json(),[name]);
  assert.deepEqual(Buffer.from(await(await request(path,{headers})).arrayBuffer()),zip);
  assert.equal((await request(path,{headers:other})).status,404);
});
