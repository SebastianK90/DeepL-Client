// Exercise the real overlay layout and mouse handling with isolated native IPC.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend'),server=http.createServer((req,res)=>{
  const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname.replace(/\/$/,'/index.html'));
  if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}
  fs.readFile(file,(error,bytes)=>{if(error){res.writeHead(404).end();return;}res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(bytes);});
 });
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});
  for(const scale of [1,1.25,1.5,2]){
   const context=await browser.newContext({viewport:{width:492,height:292},deviceScaleFactor:scale}),page=await context.newPage();
   await page.addInitScript(()=>{
    window.calls=[];
    window.__TAURI__={event:{listen:async()=>{}},core:{invoke:async(name,args)=>{
     window.calls.push({name,args});
     if(name==='bootstrap')return{config:{provider:'web',ui_language:'en'},history:[],key_present:false,hotkey_status:'test'};
    }}};
   });
   await page.goto(`http://127.0.0.1:${server.address().port}/?overlay=1`);
   const layout = await page.evaluate(() => {
    const ids = ['quick', 'quick-close', 'quick-settings', 'quick-copy', 'quick-main', 'quick-replace', 'quick-output'];
    return ids.map(id => {const r = document.getElementById(id).getBoundingClientRect();return {id, x:r.x, y:r.y, right:r.right, bottom:r.bottom, width:r.width, height:r.height};});
   });
   for (const r of layout) {
    assert(r.width > 0 && r.height > 0, `Invisible Quick Assist control: ${r.id}`);
    assert(r.x >= -1 && r.y >= -1 && r.right <= 493 && r.bottom <= 293, `Clipped Quick Assist control: ${JSON.stringify(r)}`);
   }
   const grip=page.locator('#quick-resize');
   assert.equal(await grip.count(),1,'Quick Assist needs a visible resize grip');
   await grip.waitFor({state:'visible'});
   const edges={north:'ns-resize',north_east:'nesw-resize',east:'ew-resize',south_east:'nwse-resize',south:'ns-resize',south_west:'nesw-resize',west:'ew-resize',north_west:'nwse-resize'};
   for(const [edge,cursor] of Object.entries(edges)){
    const handle=page.locator(`[data-quick-resize="${edge}"]`);
    assert.equal(await handle.count(),1,`Missing resize handle: ${edge}`);
    assert.equal(await handle.evaluate(e=>getComputedStyle(e).cursor),cursor);
    await page.evaluate(()=>window.calls=[]);
    await handle.click();
    assert.deepEqual(await page.evaluate(()=>window.calls.find(c=>c.name==='start_quick_resize')?.args),{edge},`Wrong resize direction: ${edge}`);
    assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='start_quick_drag').length),0,'Resizing must not start moving the title bar');
   }
   for(const viewport of [{width:492,height:292},{width:560,height:340},{width:800,height:550}]){
    await page.setViewportSize(viewport);
    for(const theme of ['light','dark']){
     await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
     const box=await grip.boundingBox();
     assert(box.width>=12&&box.height>=12,'Grip must be easy to hit');
     assert(Math.abs(box.x+box.width-viewport.width)<=2&&Math.abs(box.y+box.height-viewport.height)<=2,'Grip must stay in the bottom right corner');
     assert.equal(await grip.evaluate(e=>getComputedStyle(e).cursor),'nwse-resize');
     const replace=await page.locator('#quick-replace').boundingBox();
     assert(replace.x+replace.width<=box.x||replace.y+replace.height<=box.y,'Grip must not overlap Replace');
     await page.evaluate(()=>window.calls=[]);
     await grip.click({button:'right'});
     assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='start_quick_resize').length),0,'Right click must not resize');
     await grip.click();
     assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='start_quick_resize').length),1,'Left press must start native resizing');
    }
   }
   await context.close();
  }
  console.log('Quick Assist resize grip passed: primary mouse gesture, corner placement, no action overlap, both themes and 100%/125%/150%/200% DPI.');
 }finally{await browser?.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
