// UI speech routing/lifecycle with explicit IPC and Audio doubles; no remote text is sent.
const {chromium}=require('playwright');const fs=require('node:fs'),path=require('node:path'),http=require('node:http'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend');const server=http.createServer((req,res)=>{const name=new URL(req.url,'http://localhost').pathname;const file=path.resolve(root,'.'+(name==='/'?'/index.html':name));if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}fs.readFile(file,(err,data)=>{if(err){res.writeHead(404).end();return;}res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(data);});});await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});const page=await browser.newPage({viewport:{width:974,height:611}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{
   window.calls=[];window.listeners={};window.players=[];window.playCount=0;window.pauseCount=0;window.revoked=0;
   const originalRevoke=URL.revokeObjectURL.bind(URL);URL.revokeObjectURL=url=>{window.revoked++;originalRevoke(url);};
   window.Audio=class{constructor(src){this.src=src;window.players.push(this);}async play(){if(window.failPlay)throw new Error('decoder failure');window.playCount++;}pause(){window.pauseCount++;}};
   let config={provider:'api_free',source:'AUTO',target:'DE',theme:'light',hotkey_ms:350,history_enabled:true,write_language:'DE',write_style:'simple',ui_language:'de',last_mode:'translate',quick_mode_policy:'remember_last',quick_position_policy:'remember_last'};
   window.__TAURI__={event:{listen:async(name,fn)=>{window.listeners[name]=fn;}},core:{invoke:async(name,args={})=>{window.calls.push({name,args});
    if(name==='bootstrap')return{config,history:[],key_present:false,quick_text:'',hotkey_status:'test'};
    if(name==='begin_speech_session')return 7;
    if(name==='save_settings'){config={...args.config};window.listeners['settings-changed']?.({payload:config});}
    if(name==='remember_mode'){config.last_mode=args.mode;window.listeners['settings-changed']?.({payload:config});}
    if(name==='translate')return{text:'Guten Tag.',detected_source:window.detectedLanguage||'EN'};
    if(name==='write')return{text:'Ein guter Text.',detected_source:'DE'};
    if(name==='synthesize_speech'){await new Promise(r=>setTimeout(r,window.speechDelay||40));if(window.failSpeech)throw 'Edge-Sprachausgabe ist derzeit nicht erreichbar.';const french=args.language==='FR';return{audio_base64:'SUQz',voice:french?'fr-FR-RemyMultilingualNeural':args.language==='EN-US'?'en-US-BrianNeural':'de-DE-ConradNeural',preferred_voice:french?'fr-FR-Remy:DragonHDLatestNeural':'de-DE-ConradNeural',locale:french?'fr-FR':'de-DE',fallback:french};}
    return null;
   }}};
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);await page.waitForFunction(()=>window.listeners['speech-stop']);
  assert.equal(await page.locator('#speak-source').isDisabled(),true);assert.equal(await page.locator('#speak-result').isDisabled(),true);
  await page.locator('#source-text').fill('Hello.');assert.equal(await page.locator('#speak-source').isDisabled(),true,'AUTO requires a detected source, never a guessed UI language');
  const source=async code=>{await page.locator('#source-language').click();await page.locator('.language-option').filter({has:page.getByText(code,{exact:true})}).click();};
  await source('EN-US');await page.locator('#speak-source').click();await page.waitForFunction(()=>window.playCount===1);
  let call=await page.evaluate(()=>window.calls.filter(c=>c.name==='synthesize_speech').at(-1).args);assert.equal(call.language,'EN-US');assert.equal(call.text,'Hello.');assert.equal(call.session,7);
  await page.locator('#speak-source').click();assert.equal(await page.locator('#speak-source').getAttribute('aria-pressed'),'false');assert.ok(await page.evaluate(()=>window.pauseCount>0&&window.revoked===1));
  await page.keyboard.press('Control+Enter');await page.locator('#speak-result').click();await page.waitForFunction(()=>window.playCount===2);call=await page.evaluate(()=>window.calls.filter(c=>c.name==='synthesize_speech').at(-1).args);assert.equal(call.language,'DE');assert.equal(call.text,'Guten Tag.');
  await page.locator('#source-text').fill('Bonjour.');assert.equal(await page.locator('#speak-result').getAttribute('aria-pressed'),'false');assert.equal(await page.locator('#speak-result').isDisabled(),true);
  await source('AUTO');await page.evaluate(()=>window.detectedLanguage='FR');await page.keyboard.press('Control+Enter');await page.locator('#speak-source').click();await page.waitForFunction(()=>window.playCount===3);assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='synthesize_speech').at(-1).args.language),'FR');assert.match(await page.locator('#speech-status').innerText(),/Alternative.*RemyMultilingual/);
  await page.evaluate(()=>window.listeners['speech-stop']({payload:null}));await page.evaluate(()=>window.speechDelay=200);await page.locator('#speak-source').click();await page.locator('#speak-source').click();await page.waitForTimeout(250);assert.equal(await page.evaluate(()=>window.playCount),3,'A stopped pending response never starts audio');
  await page.locator('#speak-source').click();await page.locator('#speak-result').click();await page.waitForTimeout(260);assert.equal(await page.evaluate(()=>window.playCount),4);assert.equal(await page.locator('#speak-result').getAttribute('aria-pressed'),'true');assert.equal(await page.locator('#speak-source').getAttribute('aria-pressed'),'false');
  await page.evaluate(()=>window.players.at(-1).onended());assert.equal(await page.locator('#speak-result').getAttribute('aria-pressed'),'false');assert.equal(await page.locator('#speech-status').isVisible(),false);
  await page.evaluate(()=>{window.failSpeech=true;window.speechDelay=1;});await page.locator('#speak-source').click();await page.waitForTimeout(60);assert.match(await page.locator('#toast').innerText(),/derzeit nicht erreichbar/);assert.equal(await page.locator('#speak-source').getAttribute('aria-pressed'),'false');
  await page.evaluate(()=>{window.failSpeech=false;window.failPlay=true;});await page.locator('#speak-source').click();await page.waitForTimeout(60);assert.match(await page.locator('#toast').innerText(),/Audio konnte nicht abgespielt/);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1||document.documentElement.scrollHeight>innerHeight+1),false);assert.deepEqual(errors,[]);
  console.log('Speech UI checks passed: explicit/detected/target languages, source/result routing, stop/cancel, stale responses, fallback disclosure, ended/error cleanup, compact layout.');
 }finally{if(browser)await browser.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
