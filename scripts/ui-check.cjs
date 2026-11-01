// Run with a Playwright installation in NODE_PATH. No remote translation is used.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),assert=require('node:assert/strict'),path=require('node:path');
(async()=>{
 const server=http.createServer((req,res)=>{const pathname=new URL(req.url,'http://localhost').pathname;const p=path.join(process.cwd(),'frontend',pathname==='/'?'index.html':pathname);if(!p.startsWith(path.join(process.cwd(),'frontend'))){res.writeHead(403).end();return;}fs.readFile(p,(e,b)=>{if(e){res.writeHead(404).end();return;}res.setHeader('content-type',p.endsWith('.js')?'text/javascript':p.endsWith('.css')?'text/css':'text/html');res.end(b);});});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 let browser;
 try { browser=await chromium.launch({headless:true, ...(process.platform==='win32'?{channel:'msedge'}:{})}); }
 catch(e) { server.close();throw e; }
 try {
  const page=await browser.newPage({viewport:{width:1180,height:800},deviceScaleFactor:1});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  const base=`http://127.0.0.1:${server.address().port}`;
  await page.goto(base);await page.waitForSelector('#source-text');
  assert.equal(await page.locator('#write-style option[value="default"]').count(),0,'Only the four actual writing styles are selectable');
  assert.equal(await page.locator('#write-style').inputValue(),'simple','Simple is selected by default');
  fs.mkdirSync('docs/screenshots',{recursive:true});
  await page.screenshot({path:'docs/screenshots/light.png',fullPage:true});
  assert.equal(await page.locator('#translate').count(),0);
  await page.locator('#source-text').fill('Hallo 🌍');
  assert.match(await page.locator('#char-count').innerText(),/7/);
  await page.locator('#target-language').click();await page.locator('#language-search').fill('Japan');
  assert.equal(await page.locator('.language-option').count(),1);
  await page.locator('.language-option').click();assert.match(await page.locator('#target-language').innerText(),/Japanisch/);
  await page.locator('#app-menu-toggle').click();await page.locator('#settings-open').click();assert.equal(await page.locator('#settings-dialog').isVisible(),true);
  await page.locator('[data-close="settings-dialog"]').click();
  await page.locator('#app-menu-toggle').click();await page.locator('#theme-toggle').click();await page.locator('#toast').waitFor();await page.locator('#toast').evaluate(el=>el.hidden=true);
  await page.locator('#clear').click();await page.waitForTimeout(250);await page.screenshot({path:'docs/screenshots/dark.png',fullPage:true});
  const darkPalette=await page.evaluate(()=>({background:getComputedStyle(document.body).backgroundColor,ink:getComputedStyle(document.body).color,result:getComputedStyle(document.querySelector('.result-panel')).backgroundColor,line:getComputedStyle(document.querySelector('.translation-workspace')).borderTopColor,action:getComputedStyle(document.querySelector('#settings-form .primary-button')).backgroundColor}));
  await page.goto(base+'/?overlay=1');assert.equal(await page.locator('#app').isVisible(),false);assert.equal(await page.locator('#quick').isVisible(),true);
  assert.equal(await page.locator('#quick-original').count(),0,'Quick Assist must show only the result');
  assert.equal(await page.locator('#quick-write-style').count(),0,'Write settings belong in the full app');
  assert.equal(await page.locator('#quick-replace').count(),1,'The original layout includes Replace');
  assert.equal(await page.locator('#quick-target').getAttribute('aria-haspopup'),'listbox','Quick Assist opens a searchable owned popup');
  await page.setViewportSize({width:492,height:292});await page.waitForTimeout(250);await page.screenshot({path:'docs/screenshots/quick.png'});
  await page.evaluate(()=>document.documentElement.dataset.theme='dark');
  await page.waitForTimeout(250);
  const quickDarkPalette=await page.evaluate(()=>({background:getComputedStyle(document.getElementById('quick')).backgroundColor,ink:getComputedStyle(document.getElementById('quick')).color,result:getComputedStyle(document.getElementById('quick-output')).backgroundColor,line:getComputedStyle(document.getElementById('quick-output')).borderTopColor,action:getComputedStyle(document.getElementById('quick-replace')).backgroundColor}));
  assert.deepEqual(quickDarkPalette,darkPalette,'Main and Quick Assist share the complete dark palette');
  await page.screenshot({path:'docs/screenshots/quick-dark.png'});await page.evaluate(()=>document.documentElement.dataset.theme='light');
  // Explicit IPC test double: test async races and safe rendering, never claim a native run.
  const nativeMock=()=>{
    window.testCalls=[];window.listeners={};
    const defaults={provider:'api_free',source:'AUTO',target:'DE',theme:'light',hotkey_ms:350,history_enabled:true,instance_id:'00000000-0000-4000-8000-000000000001',last_mode:'translate',quick_mode_policy:'remember_last',write_language:'DE'};
    const settings=()=>({...defaults,...JSON.parse(localStorage.getItem('test-config')||'{}')});
    window.__TAURI__={event:{listen:async(name,handler)=>{window.listeners[name]=handler;return()=>{};}},core:{invoke:async(name,args)=>{
      window.testCalls.push({name,args});
      if(name==='bootstrap')return{config:settings(),history:[],hotkey_status:'test',quick_text:'',key_present:false};
      if(name==='begin_quick'){const c=settings();window.quickSession=(window.quickSession||0)+1;return{mode:c.quick_mode_policy==='always_write'?'write':c.quick_mode_policy==='always_translate'?'translate':c.last_mode,session:window.quickSession,can_replace:window.canReplace!==false};}
      if(name==='save_settings'||name==='remember_mode'){if(name==='save_settings'&&window.settingsSaveDelay)await new Promise(r=>setTimeout(r,window.settingsSaveDelay));const c=name==='remember_mode'?{...settings(),last_mode:args.mode}:{...args.config,last_mode:settings().last_mode};localStorage.setItem('test-config',JSON.stringify(c));window.listeners['settings-changed']?.({payload:c});return null;}
      if(name==='translate'){await new Promise(r=>setTimeout(r,120));return{text:'<script>window.injected=true</script>',detected_source:'EN'};}
      if(name==='write'){const text=window.testWriteTextByStyle?.[args.request.style]||'Das ist eine gute Idee. Ich freue mich auf Ihre Antwort.';await new Promise(r=>setTimeout(r,120));return{text,detected_source:'DE'};}
      return null;
    }}};
  };
  await page.addInitScript(nativeMock);
  await page.setViewportSize({width:1180,height:800});await page.goto(base);await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));
  await page.locator('#source-text').fill('Hello');
  assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='translate').length),0);
  await page.keyboard.press('Control+Enter');await page.waitForTimeout(200);
  assert.match(await page.locator('#result-text').inputValue(),/<script>/);assert.equal(await page.evaluate(()=>window.injected),undefined);
  await page.keyboard.press('Control+Enter');await page.locator('#source-text').fill('Changed');await page.waitForTimeout(200);assert.equal(await page.locator('#result-text').isVisible(),false);
  await page.locator('#main-mode-write').click();
  await page.locator('#source-text').fill('Das ist ein gute Idee. Ich freue mich auf ihre antwort.');
  await page.keyboard.press('Control+Enter');await page.waitForFunction(()=>document.querySelector('#result-text').value.includes('eine gute Idee'));
  assert.match(await page.locator('#result-text').inputValue(),/eine gute Idee/);
  assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='write').at(-1).args.request.text),'Das ist ein gute Idee. Ich freue mich auf ihre antwort.');
  assert.equal(await page.locator('#source-language').isDisabled(),true);
  for(const style of ['simple','business','academic','casual']) {
    await page.locator(`[data-style="${style}"]`).click();await page.waitForTimeout(200);
    assert.equal(await page.locator('#write-style').inputValue(),style);
    assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='write').at(-1).args.request.style),style);
    assert.equal(await page.locator(`[data-style="${style}"]`).getAttribute('aria-pressed'),'true');
  }
  assert.equal(await page.locator('#write-tools').isVisible(),true);
  await page.evaluate(()=>{window.testWriteTextByStyle={simple:'Simple output',business:'Business output'};});
  await page.locator('[data-style="simple"]').click();await page.locator('[data-style="business"]').click();await page.waitForTimeout(450);
  assert.equal(await page.locator('#result-text').inputValue(),'Business output');
  await page.evaluate(()=>delete window.testWriteTextByStyle);
  await page.locator('[data-style="casual"]').click();await page.waitForTimeout(200);
  await page.locator('[data-view="history"]').click();assert.equal(await page.locator('#history-view').isVisible(),true);
  await page.locator('#main-mode-write').click();assert.equal(await page.locator('#translate-view').isVisible(),true);
  await page.locator('#target-language').click();await page.locator('#language-search').fill('Nieder');assert.equal(await page.locator('.language-option').count(),0);await page.locator('[data-close="language-dialog"]').click();
  await page.keyboard.press('Control+Enter');await page.waitForTimeout(200);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollHeight>innerHeight+1),false);
  await page.screenshot({path:'docs/screenshots/write-main.png',fullPage:true});
  await page.setViewportSize({width:760,height:600});await page.waitForTimeout(100);assert.equal(await page.evaluate(()=>document.documentElement.scrollHeight>innerHeight+1),false);await page.screenshot({path:'docs/screenshots/write-compact.png'});
  await page.setViewportSize({width:1180,height:800});
  await page.goto(base+'/?overlay=1');await page.setViewportSize({width:492,height:292});await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));
  const hotkey=()=>page.evaluate(()=>window.listeners['quick-text']({payload:'Das ist ein gute Idee.'}));
  await hotkey();assert.equal(await page.locator('#quick-mode-write').getAttribute('aria-selected'),'true');assert.match(await page.locator('#quick-output').innerText(),/eine gute Idee/);
  assert.equal(await page.locator('#quick-write-style').count(),0);
  assert.equal(await page.locator('#quick-original').count(),0);
  assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='write').at(-1).args.request.style),'casual');
  assert.equal(await page.locator('#quick-replace').isEnabled(),true);
  await page.locator('#quick-copy').click();assert.match(await page.locator('#quick-copy').innerText(),/Kopiert/);assert.equal(await page.locator('#quick-copy svg').count(),1);
  await page.locator('#quick-main').click();let editCall=await page.evaluate(()=>window.testCalls.filter(x=>x.name==='open_main').at(-1));assert.equal(editCall.args.request.text,'Das ist ein gute Idee.');assert.equal(editCall.args.mode,'write');assert.equal(editCall.args.translation.text,'Das ist eine gute Idee. Ich freue mich auf Ihre Antwort.');
  await page.locator('#quick-settings').click();assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='open_main').at(-1).args.settings),true);assert.equal(await page.locator('#quick-preferences').count(),0);
  await page.locator('#quick-drag-handle').dispatchEvent('mousedown',{button:0,clientX:280,clientY:30});assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='start_quick_drag').length),1);
  await page.locator('#quick-replace').click();assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='replace_selection').at(-1).args.displayed),'Das ist eine gute Idee. Ich freue mich auf Ihre Antwort.');assert.equal(await page.locator('#quick-replace').isDisabled(),true);
  await hotkey();
  await page.locator('#quick-copy').evaluate(el=>el.blur());
  await page.waitForTimeout(1350);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1),false,'Compact Quick Assist fits the native viewport');
  await page.screenshot({path:'docs/screenshots/write-quick.png'});
  const referenceDpi=await browser.newContext({viewport:{width:492,height:292},deviceScaleFactor:1.5,storageState:await page.context().storageState()});
  const referencePage=await referenceDpi.newPage();await referencePage.addInitScript(nativeMock);await referencePage.goto(base+'/?overlay=1');await referencePage.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));
  await referencePage.evaluate(()=>window.listeners['quick-text']({payload:'Das ist ein gute Idee.'}));
  const referenceShot=await referencePage.screenshot({path:'docs/screenshots/write-quick-150.png'});assert.equal(referenceShot.readUInt32BE(16),738);assert.equal(referenceShot.readUInt32BE(20),438);await referenceDpi.close();
  const setPolicy=async value=>{await page.goto(base);await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));await page.locator('#app-menu-toggle').click();await page.locator('#settings-open').click();await page.locator('#quick-mode-policy').selectOption(value);await page.locator('#settings-form button[type="submit"]').click();await page.goto(base+'/?overlay=1');await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));};
  await page.locator('#quick-mode-translate').click();await page.waitForTimeout(200);await hotkey();assert.equal(await page.locator('#quick-mode-translate').getAttribute('aria-selected'),'true');
  await setPolicy('always_write');
  await hotkey();assert.equal(await page.locator('#quick-mode-write').getAttribute('aria-selected'),'true');
  await page.locator('#quick-mode-translate').click();await page.waitForTimeout(200);await hotkey();assert.equal(await page.locator('#quick-mode-write').getAttribute('aria-selected'),'true');
  await setPolicy('always_translate');
  await hotkey();assert.equal(await page.locator('#quick-mode-translate').getAttribute('aria-selected'),'true');
  await setPolicy('remember_last');
  await page.locator('#quick-mode-write').click();await page.waitForTimeout(200);await page.reload();await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));await hotkey();assert.equal(await page.locator('#quick-mode-write').getAttribute('aria-selected'),'true');assert.equal(await page.evaluate(()=>window.testCalls.filter(x=>x.name==='write').at(-1).args.request.style),'casual');
  // A slow old Translate response must never overwrite a newer Write result.
  await page.evaluate(()=>{window.listeners['quick-text']({payload:'Old text'});document.getElementById('quick-mode-translate').click();document.getElementById('quick-mode-write').click();});
  await page.waitForTimeout(450);assert.match(await page.locator('#quick-output').innerText(),/eine gute Idee/);
  await page.evaluate(()=>window.canReplace=false);await hotkey();assert.equal(await page.locator('#quick-replace').isDisabled(),true);assert.equal(await page.locator('#quick-copy').isEnabled(),true);
  await page.evaluate(()=>{window.canReplace=true;window.settingsSaveDelay=400;});await hotkey();
  await page.locator('#quick-target').click();assert.equal(await page.locator('#quick-target').getAttribute('aria-expanded'),'true');
  const popupCall=await page.evaluate(()=>window.testCalls.filter(x=>x.name==='open_language_popup').at(-1));assert.equal(popupCall.args.mode,'write');assert.ok(popupCall.args.anchor.height>0);
  await page.evaluate(()=>{const c=JSON.parse(localStorage.getItem('test-config'));c.write_language='EN-GB';localStorage.setItem('test-config',JSON.stringify(c));window.listeners['settings-changed']({payload:c});window.listeners['quick-language-popup-closed']({payload:null});window.listeners['quick-language-selected']({payload:'write'});});
  assert.equal(await page.locator('#quick-copy').isDisabled(),true);assert.equal(await page.locator('#quick-replace').isDisabled(),true);assert.match(await page.locator('#quick-output').innerText(),/Wird verbessert/);
  await page.waitForTimeout(650);assert.equal(await page.locator('#quick-replace').isEnabled(),true);
  // Full interface language changes must not alter either original text or service output.
  await page.setViewportSize({width:1180,height:800});await page.goto(base);await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));
  await page.locator('#app-menu-toggle').click();await page.locator('#settings-open').click();
  for(const position of ['mouse_cursor','centered','remember_last']) {
    await page.locator('#quick-position-policy').selectOption(position);
    await page.locator('#settings-form button[type="submit"]').click();
    await page.reload();await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));
    await page.locator('#app-menu-toggle').click();await page.locator('#settings-open').click();assert.equal(await page.locator('#quick-position-policy').inputValue(),position);
  }
  await page.locator('[data-close="settings-dialog"]').click();
  await page.locator('#main-mode-write').click();await page.locator('#source-text').fill('Einstellungen');
  await page.evaluate(()=>{window.testWriteTextByStyle={casual:'Kopieren'};});await page.keyboard.press('Control+Enter');await page.waitForTimeout(200);
  await page.locator('#app-menu-toggle').click();await page.locator('#settings-open').click();
  const titles={en:'Settings',de:'Einstellungen',cs:'Nastavení',fr:'Paramètres',ja:'設定',it:'Impostazioni'};
  const historyNames={en:'History',de:'Verlauf',cs:'Historie',fr:'Historique',ja:'履歴',it:'Cronologia'};
  for(const locale of ['en','de','cs','fr','ja','it']){
    await page.locator('#ui-language').selectOption(locale);await page.waitForTimeout(60);
    assert.equal(await page.locator('html').getAttribute('lang'),locale);
    assert.equal(await page.locator('#settings-dialog h2').innerText(),titles[locale]);
    assert.match(await page.locator('[data-view="history"]').innerText(),new RegExp(historyNames[locale]));
    assert.equal(await page.locator('#source-text').inputValue(),'Einstellungen');assert.equal(await page.locator('#result-text').inputValue(),'Kopieren');
    if(locale==='ja')await page.screenshot({path:'docs/screenshots/settings-japanese.png'});
  }
  await page.locator('#settings-form button[type="submit"]').click();assert.equal(await page.locator('#result-text').inputValue(),'Kopieren');await page.reload();await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));assert.equal(await page.locator('html').getAttribute('lang'),'it');
  await page.goto(base+'/?overlay=1');await page.setViewportSize({width:492,height:292});await page.waitForFunction(()=>window.testCalls.some(x=>x.name==='bootstrap'));await hotkey();assert.equal(await page.locator('#quick-copy').innerText(),'Copia');assert.equal(await page.locator('#quick-write-style').count(),0);assert.equal(await page.locator('#quick-replace').innerText(),'Sostituisci');
  assert.deepEqual(errors,[]);
  console.log('UI checks passed: six UI languages, localized settings/overlay/styles, persisted locale, unchanged user text, editor layout, styles, startup policies, races, safe rendering.');
 } finally {await browser.close();server.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});



