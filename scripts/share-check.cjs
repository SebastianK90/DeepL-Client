// Isolated IPC doubles: opens no external application and never sends a message.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend');const server=http.createServer((req,res)=>{const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname.replace(/\/$/,'/index.html'));if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}fs.readFile(file,(e,b)=>{if(e){res.writeHead(404).end();return;}res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(b);});});await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});const page=await browser.newPage({viewport:{width:974,height:611}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{window.calls=[];window.listeners={};window.__TAURI__={event:{listen:async(n,h)=>window.listeners[n]=h},core:{invoke:async(name,args={})=>{window.calls.push({name,args});if(name==='bootstrap')return{share_supported:!location.search.includes('unsupported'),config:{provider:'web',source:'DE',target:'EN-GB',ui_language:'de',theme:'light',write_style:'simple',write_language:'DE',last_mode:'translate'},history:[],hotkey_status:'test',key_present:false};if(name==='begin_speech_session')return 1;if(name==='share_text'){if(window.shareError)throw window.shareError;if(window.deferShare)return new Promise(r=>window.resolveShare=r);return{copied:true,desktop:true};}}}};});
  const base=`http://127.0.0.1:${server.address().port}`;await page.goto(base);await page.waitForFunction(()=>window.listeners['load-translation']);
  assert.equal(await page.locator('#share').isDisabled(),true);
  const load=()=>page.evaluate(()=>window.listeners['load-translation']({payload:[{text:'Original secret',source:'DE',target:'EN-GB'},{text:'Shared 🌍\n日本語 <b>plain text</b>',detected_source:'DE'},'translate']}));
  await load();await page.locator('#copy').click();assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='copy_text').at(-1).args.text),'Shared 🌍\n日本語 <b>plain text</b>');assert.equal(await page.locator('#copy svg').count(),1);
  await page.locator('#share').click();assert.equal(await page.locator('#share-dialog').isVisible(),true);assert.equal(await page.evaluate(()=>window.calls.some(c=>c.name==='share_text')),false);assert.equal(await page.locator('#share-preview b').count(),0);assert.equal(await page.locator('#share-preview').innerText(),'Shared 🌍\n日本語 <b>plain text</b>');
  await page.evaluate(()=>window.deferShare=true);await page.locator('#share-teams').click();await page.waitForFunction(()=>window.resolveShare);assert.equal(await page.locator('#share-teams').isDisabled(),true);assert.equal(await page.locator('#share-outlook').isDisabled(),true);assert.deepEqual(await page.evaluate(()=>window.calls.filter(c=>c.name==='share_text').at(-1).args),{target:'teams',text:'Shared 🌍\n日本語 <b>plain text</b>'});
  await page.evaluate(()=>window.resolveShare({copied:true,prefilled:false,desktop:true}));await page.waitForFunction(()=>!document.querySelector('#share-dialog').open);assert.equal(await page.locator('#source-text').inputValue(),'Original secret');
  assert.equal(await page.locator('#toast').innerText(),'Text kopiert. In Teams einen Chat wählen und mit Strg+V einfügen.');
  assert.equal(await page.locator('#share-teams small').innerText(),'Text kopieren und Teams öffnen');
  await page.evaluate(()=>{window.deferShare=false;window.shareError='Ziel-App konnte nicht geöffnet werden. Der Text wurde kopiert.';});await page.locator('#share').click();await page.locator('#share-outlook').click();assert.equal(await page.locator('#share-dialog').isVisible(),true);assert.match(await page.locator('#share-error').innerText(),/Ziel-App/);assert.equal(await page.locator('#share-outlook').isDisabled(),false);assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='share_text').at(-1).args.target),'outlook');
  await page.keyboard.press('Escape');assert.equal(await page.locator('#share-dialog').isVisible(),false);assert.equal(await page.locator('#share').evaluate(el=>document.activeElement===el),true);
  await page.evaluate(()=>window.shareError=null);await page.locator('#share').click();await page.evaluate(()=>window.listeners['load-translation']({payload:[{text:'New input',source:'DE',target:'EN-GB'},{text:'New result',detected_source:'DE'},'translate']}));await page.locator('#share-outlook').click();assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='share_text').at(-1).args.text),'Shared 🌍\n日本語 <b>plain text</b>');
  await load();await page.locator('#share').click();
  for(const [locale,label] of [['en','Share result'],['de','Ergebnis teilen'],['cs','Sdílet výsledek'],['fr','Partager le résultat'],['ja','結果を共有'],['it','Condividi risultato']]){
   await page.evaluate(ui_language=>window.listeners['settings-changed']({payload:{ui_language}}),locale);
   assert.equal(await page.locator('#share-heading').innerText(),label);
   assert.equal(await page.locator('#share-preview').innerText(),'Shared 🌍\n日本語 <b>plain text</b>');
  }
  await page.evaluate(()=>{window.listeners['settings-changed']({payload:{ui_language:'de'}});document.querySelector('#toast').hidden=true;});
  const beforeTranslate=await page.evaluate(()=>window.calls.filter(c=>c.name==='translate').length);
  await page.keyboard.press('Control+Enter');assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='translate').length),beforeTranslate);
  fs.mkdirSync('docs/screenshots',{recursive:true});await page.screenshot({path:'docs/screenshots/share-light.png'});
  await page.evaluate(()=>document.documentElement.dataset.theme='dark');await page.waitForTimeout(250);
  assert.equal(await page.locator('#share-teams').evaluate(el=>getComputedStyle(el).backgroundColor),await page.locator('#share-dialog').evaluate(el=>getComputedStyle(el).backgroundColor));
  await page.screenshot({path:'docs/screenshots/share-dark.png'});
  await page.mouse.click(2,2);assert.equal(await page.locator('#share-dialog').isVisible(),false);
  // Reproduce the reported overlap with long text, in both modes and themes.
  for(const mode of ['translate','write'])for(const theme of ['light','dark'])for(const [width,height] of [[760,540],[974,611],[1240,690],[1500,850]]){
   await page.setViewportSize({width,height});await page.evaluate(({mode,theme})=>{document.documentElement.dataset.theme=theme;window.listeners['load-translation']({payload:[{text:'Original text with many lines.\n'.repeat(80),source:'DE',target:'EN-GB'},{text:'Translated text with many lines.\n'.repeat(80),detected_source:'DE'},mode]});},{mode,theme});
   const g=await page.evaluate(()=>{const r=s=>{const b=document.querySelector(s).getBoundingClientRect();return{top:b.top,bottom:b.bottom};};const b=document.querySelector('#share').getBoundingClientRect();return{source:r('#source-text'),result:r('#result-display'),sf:r('.source-panel .panel-footer'),rf:r('.result-panel .panel-footer'),actions:r('.workspace-actions'),button:r('#share'),onTop:Boolean(document.elementFromPoint(b.x+b.width/2,b.y+b.height/2)?.closest('#share'))};});const context=JSON.stringify({width,height,mode,theme,g});
   assert.ok(g.source.bottom<=g.sf.top+1&&g.result.bottom<=g.rf.top+1,context);assert.ok(g.sf.bottom<=g.actions.top+1&&g.rf.bottom<=g.actions.top+1,context);assert.ok(g.button.bottom<=height&&g.onTop,context);
  }
  await page.goto(base+'/?unsupported=1');await page.waitForFunction(()=>window.listeners['load-translation']);await load();assert.equal(await page.locator('#share').isDisabled(),true);
  await page.goto(base+'/?overlay=1');await page.waitForFunction(()=>document.querySelector('#quick').hidden===false);assert.equal(await page.locator('#share').isVisible(),false);assert.equal(await page.locator('#share-dialog').isVisible(),false);assert.deepEqual(errors,[]);
  console.log('Share checks passed: own Teams/Outlook dialog, no handoff before target choice, Unicode snapshot, pending/error/retry/Escape/focus, unchanged Quick Assist; 16 long-text resize scenarios.');
 }finally{await browser?.close();server.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
