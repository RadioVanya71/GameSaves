import {test} from 'node:test';
import assert from 'node:assert/strict';
import {randomBytes} from 'node:crypto';
import {once} from 'node:events';
test('owner selects folder, invited friend reads an uploaded version, other groups are isolated',async t=>{
  const origin='https://groups.example';
  Object.assign(process.env,{PORT:'0',DATABASE:':memory:',STORE_KEY:randomBytes(32).toString('hex'),PUBLIC_ORIGIN:origin,PROJECT_NUMBER:'123',GOOGLE_CLIENT_ID:'test',GOOGLE_CLIENT_SECRET:'test',PICKER_KEY:'test'});
  const realFetch=globalThis.fetch;
  const comment=Buffer.from('GameSaves/author=Ваня');
  const zip=Buffer.alloc(32+22+comment.length);
  zip.writeUInt32LE(0x04034b50);zip.writeUInt32LE(0x06054b50,32);
  zip.writeUInt16LE(comment.length,52);comment.copy(zip,54);
  t.mock.method(globalThis,'fetch',async(url,options)=>{
    if(url==='https://oauth2.googleapis.com/token') return Response.json({access_token:'access',refresh_token:'refresh'});
    assert.equal(options.headers.Authorization,'Bearer access');
    if(url.includes('/upload/')) {assert.ok(options.body.includes(zip));return Response.json({id:'save-id'});}
    if(url.includes('alt=media')) return new Response(zip);
    if(url.includes('/folder-id?')) return Response.json({ownedByMe:true,mimeType:'application/vnd.google-apps.folder',capabilities:{canAddChildren:true}});
    throw Error('unexpected request');
  });
  const {server}=await import('./server.mjs');
  if(!server.listening)await once(server,'listening');
  t.after(()=>server.close());
  const request=(path,opts={})=>realFetch('http://localhost:'+server.address().port+path,{...opts,redirect:'manual'});
  async function device(folder,nick){
    const r=await request('/devices',{method:'POST',body:JSON.stringify({folder,nick})});
    assert.equal(r.status,200);
    return {Authorization:'Bearer '+(await r.json()).token};
  }
  const status=async headers=>(await request('/membership',{headers})).json();
  const owner=await device('folder-id','Владелец'), headers=await device('folder-id','Друг');
  const other=await device('other-folder','Друг');
  const own=await status(owner),pending=await status(headers),outside=await status(other);
  assert.equal(pending.role,'pending');
  assert.equal((await request('/versions',{headers})).status,403);
  assert.equal((await request('/versions/save-001234567890123-abc.zip',{method:'PUT',headers,body:zip})).status,403);
  const start=await request('/create?device='+own.id);
  const cookie=start.headers.get('set-cookie').split(';')[0];
  const state=new URL(start.headers.get('location')).searchParams.get('state');
  assert.equal((await request('/callback?state=wrong&code=x',{headers:{cookie}})).status,400);
  assert.equal((await request('/callback?state='+state+'&code=x',{headers:{cookie}})).status,303);
  const bind=proof=>request('/folder',{method:'POST',headers:{cookie,origin},body:JSON.stringify({folder:'folder-id',proof})});
  assert.equal((await bind('bad')).status,403);
  assert.equal((await bind(own.proof)).status,200);
  assert.equal((await status(owner)).role,'owner');
  assert.deepEqual((await status(owner)).requests,[{id:pending.id,nick:'Друг'}]);
  const approve=(who,id)=>request('/decision/'+id+'/allow',{method:'POST',headers:who});
  assert.equal((await approve(headers,pending.id)).status,403);
  assert.equal((await approve(owner,outside.id)).status,404);
  assert.equal((await approve(owner,pending.id)).status,200);
  assert.equal((await status(headers)).role,'member');
  const name='save-001234567890123-abc.zip',path='/versions/'+name;
  assert.equal((await request('/versions')).status,401);
  assert.equal((await request(path,{method:'PUT',headers,body:zip})).status,201);
  assert.deepEqual(await (await request('/versions',{headers})).json(),[name]);
  assert.deepEqual(await (await request('/versions?authors=1',{headers})).json(),[{name,author:'Ваня'}]);
  assert.deepEqual(Buffer.from(await(await request(path,{headers})).arrayBuffer()),zip);
  assert.equal((await request(path,{headers:other})).status,403);
});
