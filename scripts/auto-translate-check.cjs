// Isolated IPC doubles: no network request or clipboard access.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend');const server=http.createServer((req,res)=>{const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname.replace(/\/$/,'/index.html'));if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}fs.readFile(file,(e,b)=>{if(e){res.writeHead(404).end();return;}res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(b);});});await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});const page=await browser.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{window.calls=[];window.listeners={};window.waiters=[];window.__TAURI__={event:{listen:async(n,h)=>window.listeners[n]=h},core:{invoke:async(name,args={})=>{window.calls.push({name,args});if(name==='bootstrap')return{config:{provider:'web',source:'DE',target:'EN-GB',ui_language:'de',theme:'light',write_style:'simple',write_language:'DE'},history:[],hotkey_status:'test',key_present:false};if(name==='begin_speech_session')return 1;if(name==='read_clipboard')return 'Clipboard text';if(['translate','write'].includes(name)){if(window.defer)return new Promise(r=>window.waiters.push(()=>r({text:'Result: '+args.request.text,detected_source:'DE'})));if(window.fail)throw 'Synthetic failure';return{text:'Result: '+args.request.text,detected_source:'DE'};}}}};});
  const base=`http://127.0.0.1:${server.address().port}`;await page.goto(base);await page.waitForFunction(()=>window.listeners['load-translation']);
  assert.equal(await page.locator('#translate').count(),0,'Manual translate button is removed');
  const count=()=>page.evaluate(()=>window.calls.filter(c=>['translate','write'].includes(c.name)).length);
  const result=text=>page.waitForFunction(text=>document.querySelector('#result-text').value==='Result: '+text,text);
  const paste=text=>page.locator('#source-text').evaluate((el,text)=>{el.value=text;el.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertFromPaste'}));},text);
  await page.locator('#paste').click();await result('Clipboard text');assert.equal(await count(),1);
  await paste('Pasted 🌍');await result('Pasted 🌍');assert.equal(await count(),2);
  await page.locator('#source-text').fill('Typing');await page.waitForTimeout(120);await page.locator('#source-text').fill('Typing final');await page.waitForTimeout(120);assert.equal(await count(),2);await result('Typing final');assert.equal(await count(),3);
  await page.evaluate(()=>window.defer=true);await paste('Old');await page.waitForFunction(()=>window.waiters.length===1);
  await paste('Intermediate');await paste('Newest');assert.equal(await count(),4,'Only one request in flight');
  await page.evaluate(()=>window.waiters.shift()());await page.waitForFunction(()=>window.waiters.length===1);assert.equal(await page.locator('#result-text').inputValue(),'','Stale result stays hidden');
  await page.evaluate(()=>{window.defer=false;window.waiters.shift()();});await result('Newest');assert.equal(await count(),5);
  await page.locator('#source-text').fill('Will be cleared');await page.locator('#clear').click();await page.waitForTimeout(430);assert.equal(await count(),5);
  await paste('界'.repeat(5001));await page.waitForTimeout(430);assert.equal(await count(),5,'Character limit applies to automatic requests');
  await page.locator('#source-text').evaluate(el=>{el.dispatchEvent(new CompositionEvent('compositionstart'));el.value='日本語';el.dispatchEvent(new InputEvent('input',{bubbles:true,isComposing:true}));});await page.waitForTimeout(430);assert.equal(await count(),5);
  await page.locator('#source-text').dispatchEvent('compositionend');await result('日本語');assert.equal(await count(),6);
  await page.locator('#main-mode-write').click();await page.waitForFunction(()=>window.calls.filter(c=>c.name==='write').length===1);await paste('Write paste');await result('Write paste');
  assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='write').at(-1).args.request.style),'simple');
  await page.locator('[data-style="business"]').click();await page.waitForFunction(()=>window.calls.filter(c=>c.name==='write').at(-1).args.request.style==='business');
  await page.locator('#target-language').click();await page.locator('#language-search').fill('EN-US');await page.locator('.language-option').filter({hasText:'EN-US'}).click();await page.waitForFunction(()=>window.calls.filter(c=>c.name==='write').at(-1).args.request.target==='EN-US');
  let before=await count();await page.evaluate(()=>window.listeners['settings-changed']({payload:{ui_language:'fr',theme:'dark'}}));await page.waitForTimeout(430);assert.equal(await count(),before,'Presentation settings do not send text');
  await page.evaluate(()=>window.fail=true);await paste('Failure');await page.waitForTimeout(430);assert.equal(await count(),before+1,'Errors do not cause automatic retry loops');
  await page.evaluate(()=>window.fail=false);await paste('Recovered');await result('Recovered');
  before=await count();await page.locator('#source-text').fill('Pending edit');await page.evaluate(()=>window.listeners['load-translation']({payload:[{text:'Saved',source:'DE',target:'EN-GB'},{text:'Saved result',detected_source:'DE'},'translate']}));await page.waitForTimeout(430);assert.equal(await count(),before);assert.equal(await page.locator('#result-text').inputValue(),'Saved result');
  await page.goto(base+'/?overlay=1');await page.waitForFunction(()=>document.querySelector('#quick').hidden===false);await page.waitForTimeout(430);assert.equal(await count(),0,'Hidden main editor never auto-runs in Quick Assist');assert.deepEqual(errors,[]);
  console.log('Automatic processing passed: clipboard and paste, typing debounce, coalescing/stale results, empty/limit/IME, Write/style/language, presentation-only settings, failure/recovery, loaded results and Quick Assist isolation.');
 }finally{await browser?.close();server.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
