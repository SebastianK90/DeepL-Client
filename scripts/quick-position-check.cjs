// Verify position-mode indicators from initial settings and live configuration events.
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
  const labels={mouse_cursor:'At the mouse pointer',centered:'Always center on screen',remember_last:'Remember last position'};
  for(const initial of Object.keys(labels)){
   const page=await browser.newPage({viewport:{width:492,height:292}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
   await page.addInitScript(policy=>{
    window.listeners={};window.__TAURI__={event:{listen:async(name,handler)=>{window.listeners[name]=handler;}},core:{invoke:async name=>{
     if(name==='bootstrap')return{config:{provider:'web',ui_language:'en',quick_position_policy:policy},history:[],key_present:false,hotkey_status:'test'};
    }}};
   },initial);
   await page.goto(`http://127.0.0.1:${server.address().port}/?overlay=1`);
   const indicator=page.locator('#quick-position-indicator');
   assert.equal(await indicator.count(),1,'Quick Assist title bar needs a position-mode indicator');
   await page.waitForFunction(()=>document.documentElement.lang==='en');
   async function check(policy){
    assert.equal(await indicator.getAttribute('title'),`Quick Assist position: ${labels[policy]}`);
    assert.equal(await indicator.getAttribute('aria-label'),await indicator.getAttribute('title'));
    const visible=indicator.locator('[data-position-policy]:visible');
    assert.equal(await visible.count(),1);assert.equal(await visible.getAttribute('data-position-policy'),policy);
    assert((await indicator.boundingBox()).width>=14);
   }
   await check(initial);
   for(const policy of Object.keys(labels)){
    await page.evaluate(policy=>window.listeners['settings-changed']({payload:{quick_position_policy:policy}}),policy);
    await check(policy);
   }
   await page.evaluate(()=>window.listeners['settings-changed']({payload:{ui_language:'de',quick_position_policy:'centered'}}));
   assert.equal(await indicator.getAttribute('title'),'Quick-Assist-Position: Immer mittig zentrieren');
   assert.deepEqual(errors,[]);await page.close();
  }
  console.log('Quick Assist position indicator passed: all three initial modes, live settings updates and localized accessible tooltips.');
 }finally{await browser?.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
