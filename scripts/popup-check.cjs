// DOM/IPC tests only. Actual window placement is tested separately with Rust/native UI.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend');
 const server=http.createServer((req,res)=>{const p=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname);if(!p.startsWith(root+path.sep)){res.writeHead(403).end();return;}fs.readFile(p,(error,data)=>{if(error){res.writeHead(404).end();return;}res.setHeader('content-type',p.endsWith('.js')?'text/javascript':p.endsWith('.css')?'text/css':'text/html');res.end(data);});});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});
  const page=await browser.newPage({viewport:{width:280,height:450},deviceScaleFactor:1.5});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{window.calls=[];window.listeners={};window.__TAURI__={event:{listen:async(name,handler)=>{window.listeners[name]=handler;}},core:{invoke:async(name,args)=>{window.calls.push({name,args});if(name==='language_popup_state')return{revision:1,open:true,mode:'translate',selected:'DE',ui_language:'de',theme:'dark'};return null;}}};});
  await page.goto(`http://127.0.0.1:${server.address().port}/language-popup.html`);
  await page.locator('#popup-search').waitFor();await page.waitForFunction(()=>document.activeElement?.id==='popup-search');
  assert.equal(await page.locator('#language-DE').getAttribute('aria-selected'),'true');
  await page.locator('#popup-search').fill('Eng');assert.equal(await page.locator('.popup-option').count(),3);
  await page.locator('#popup-search').dispatchEvent('keydown',{key:'Enter',isComposing:true});
  assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='choose_quick_language').length),0,'Composing Enter must not commit a language');
  await page.locator('#popup-search').press('ArrowDown');await page.locator('#popup-search').press('Enter');
  assert.deepEqual(await page.evaluate(()=>window.calls.find(c=>c.name==='choose_quick_language').args),{revision:1,code:'EN-GB'});
  await page.locator('#popup-search').press('Escape');assert.ok(await page.evaluate(()=>window.calls.some(c=>c.name==='close_language_popup')));
  await page.evaluate(()=>window.listeners['language-popup-opened']({payload:{revision:2,open:true,mode:'write',selected:'FR',ui_language:'fr',theme:'dark',placement:{above:true}}}));
  assert.equal(await page.evaluate(()=>document.querySelector('.popup-search').getBoundingClientRect().top>=document.getElementById('popup-list').getBoundingClientRect().bottom),true,'Upward popup keeps search closest to the app at the bottom');
  assert.equal(await page.locator('#popup-search').inputValue(),'');assert.equal(await page.locator('#language-NL').count(),0);assert.equal(await page.locator('#language-AUTO').count(),0);
  assert.equal(await page.locator('#language-FR').getAttribute('aria-selected'),'true');
  await page.locator('#popup-search').fill('jap');assert.equal(await page.locator('.popup-option').count(),1);assert.match(await page.locator('.popup-option').innerText(),/japonais/i);
  await page.locator('#popup-search').fill('<img src=x onerror=alert(1)>');assert.equal(await page.locator('.popup-option').count(),0);assert.equal(await page.locator('#popup-empty').innerText(),'Aucune langue trouvée.');assert.equal(await page.locator('img').count(),0);
  await page.locator('#popup-search').fill('');await page.screenshot({path:'docs/screenshots/language-popup-dark.png'});
  await page.evaluate(()=>{const sequence=window.calls.filter(c=>c.name==='resize_language_popup').at(-1).args.sequence;window.listeners['language-popup-placement']({payload:[2,sequence,{above:false}]});window.listeners['language-popup-placement']({payload:[2,sequence-1,{above:true}]});});
  assert.equal(await page.evaluate(()=>document.querySelector('.popup-search').getBoundingClientRect().bottom<=document.getElementById('popup-list').getBoundingClientRect().top),true,'Downward menu keeps search at the top and ignores older placement updates');
  await page.evaluate(()=>window.listeners['language-popup-opened']({payload:{revision:1,open:true,mode:'translate',selected:'DE',ui_language:'de',theme:'light'}}));
  assert.equal(await page.locator('html').getAttribute('lang'),'fr','Old open events cannot replace a newer menu');
  await page.evaluate(()=>window.listeners['language-popup-opened']({payload:{revision:3,open:true,mode:'translate',selected:'DE',ui_language:'de',theme:'light'}}));
  await page.screenshot({path:'docs/screenshots/language-popup-light.png'});assert.deepEqual(errors,[]);
  console.log('Popup checks passed: search, language filtering, localized labels, keyboard/Enter/Escape, selected state, revision binding, safe text, light/dark.');
 }finally{if(browser)await browser.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
