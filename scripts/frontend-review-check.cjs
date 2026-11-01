// Reproduce asynchronous UI races with isolated IPC; no external service or clipboard calls.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend');
 const server=http.createServer((req,res)=>{const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname.replace(/\/$/,'/index.html'));if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}fs.readFile(file,(error,bytes)=>{if(error){res.writeHead(404).end();return;}res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(bytes);});});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});
  const failures=[];
  for(const scenario of ['image-handoff','clipboard-handoff','clipboard-debounce','clipboard-error','startup-load','tray-setting']){
   const page=await browser.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
   try{
    await page.addInitScript(scenario=>{
     window.listeners={};window.calls=[];
     window.__TAURI__={event:{listen:async(name,handler)=>window.listeners[name]=handler},core:{invoke:async(name,args={})=>{
      window.calls.push({name,args});
      if(name==='bootstrap'){if(scenario==='startup-load')await new Promise(resolve=>window.resolveBootstrap=resolve);return{config:{provider:'web',source:'DE',target:'EN-GB',write_language:'DE',write_style:'simple',ui_language:'de',theme:'light',...(scenario==='tray-setting'?{start_in_tray:localStorage.getItem('tray-setting')!=='false'}:{})},history:[],hotkey_status:'test',key_present:false};}
      if(name==='save_settings'&&scenario==='tray-setting'){localStorage.setItem('tray-setting',String(args.config.start_in_tray));return;}
      if(name==='begin_speech_session')return 1;
      if(name==='recognize_image')return 'Older image text';
      if(name==='remember_mode'&&window.deferRemember)return new Promise(resolve=>window.resolveRemember=resolve);
      if(name==='read_clipboard')return new Promise((resolve,reject)=>{window.resolveClipboard=resolve;window.rejectClipboard=reject;});
      if(name==='translate')return{text:'Result: '+args.request.text,detected_source:'DE'};
     }}};
    },scenario);
    await page.goto(`http://127.0.0.1:${server.address().port}`);await page.waitForFunction(()=>window.calls.some(call=>call.name==='bootstrap'));
    if(scenario==='image-handoff'){
     await page.evaluate(()=>window.deferRemember=true);await page.locator('[data-view="images"]').click();
     await page.locator('#image-input').setInputFiles({name:'old.png',mimeType:'image/png',buffer:Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aK9sAAAAASUVORK5CYII=','base64')});
     await page.waitForFunction(()=>window.resolveRemember);await page.locator('#source-text').fill('Newer editor text');
     await page.evaluate(()=>window.resolveRemember());await page.waitForFunction(()=>document.getElementById('image-input').value==='');
     assert.equal(await page.locator('#source-text').inputValue(),'Newer editor text','An image handoff must not overwrite edits made while remembering the mode');
    }else if(scenario==='clipboard-handoff'){
     await page.locator('#paste').click();await page.waitForFunction(()=>window.resolveClipboard);
     await page.locator('#source-text').fill('Newer editor text');
     await page.evaluate(async()=>{window.resolveClipboard('Older clipboard text');await new Promise(resolve=>setTimeout(resolve,0));});
     assert.equal(await page.locator('#source-text').inputValue(),'Newer editor text','A delayed clipboard read must not overwrite newer edits');
    }
    if(scenario==='clipboard-debounce'){
     await page.locator('#source-text').fill('Previously typed text');
     await page.locator('#paste').click();await page.waitForFunction(()=>window.resolveClipboard);
     await page.waitForTimeout(450);
     await page.evaluate(async()=>{window.resolveClipboard('Requested clipboard text');await new Promise(resolve=>setTimeout(resolve,0));});
     assert.equal(await page.locator('#source-text').inputValue(),'Requested clipboard text','A pending automatic translation must not discard a requested paste');
    }
    if(scenario==='clipboard-error'){
     await page.locator('#source-text').fill('Previously typed text');
     await page.locator('#paste').click();await page.waitForFunction(()=>window.rejectClipboard);
     await page.evaluate(()=>window.rejectClipboard('Clipboard unavailable'));
     await page.waitForFunction(()=>document.getElementById('result-text').value==='Result: Previously typed text',{},{timeout:2000});
    }
    if(scenario==='startup-load'){
     await page.waitForFunction(()=>window.resolveBootstrap);
     await page.evaluate(()=>window.listeners['load-translation']({payload:[{text:'Saved English text',source:'EN',target:'FR',style:'default'},{text:'Traduction déjà prête',detected_source:'EN'},'translate']}));
     await page.evaluate(async()=>{window.resolveBootstrap();await new Promise(resolve=>setTimeout(resolve,450));});
     assert.equal(await page.locator('#result-text').inputValue(),'Traduction déjà prête','Loading during bootstrap must preserve the supplied result and target');
     assert.equal(await page.locator('#target-language').innerText(),'Französisch⌄');
     assert.equal(await page.evaluate(()=>window.calls.filter(call=>call.name==='translate').length),0,'A supplied translation must not be submitted again at startup');
    }
    if(scenario==='tray-setting'){
     const openSettings=async()=>{await page.locator('#app-menu-toggle').click();await page.locator('#settings-open').click();};
     await openSettings();const toggle=page.getByRole('switch',{name:'Im Tray starten',exact:true});
     assert.equal(await toggle.isChecked(),true,'Tray startup is enabled by default');
     await toggle.focus();await page.keyboard.press('Space');
     await page.locator('#settings-form button[type="submit"]').click();
     await page.waitForFunction(()=>window.calls.some(c=>c.name==='save_settings'&&c.args.config.start_in_tray===false));
     await page.reload();await page.waitForFunction(()=>window.calls.some(c=>c.name==='bootstrap'));
     await openSettings();assert.equal(await toggle.isChecked(),false,'Opt-out survives reloading');
     await toggle.check();await page.locator('#settings-form button[type="submit"]').click();
     await openSettings();assert.equal(await toggle.isChecked(),true);
     await toggle.scrollIntoViewIfNeeded();fs.mkdirSync('.tools/start-in-tray',{recursive:true});await page.screenshot({path:'.tools/start-in-tray/settings.png'});
    }
    assert.deepEqual(errors,[]);console.log(`${scenario}: passed`);
   }catch(error){failures.push(`${scenario}: ${error.message}`);}finally{await page.close();}
  }
  assert.deepEqual(failures,[]);
 }finally{await browser?.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
