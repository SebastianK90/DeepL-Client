// Explicit IPC double; validates interaction, safe rendering and response ordering.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend');const server=http.createServer((req,res)=>{
  const name=new URL(req.url,'http://localhost').pathname;
  if(name==='/app'){res.setHeader('content-type','text/html');res.end(fs.readFileSync(path.join(root,'index.html')));return;}
  if(name==='/'){res.setHeader('content-type','text/html');res.end('<link rel="stylesheet" href="/styles.css"><link rel="stylesheet" href="/word-lookup.css"><div id="result-display" role="textbox" aria-readonly="true" tabindex="0" style="position:absolute;left:40px;top:80px;width:400px;height:320px"></div><button id="outside" style="position:absolute;top:20px;right:20px">Outside</button><script type="module">import {createWordLookup} from "/word-lookup.js";window.lookup=createWordLookup({invoke:window.invoke,t:s=>s,context:()=>window.lookupContext,changed:v=>{window.applied=v;window.lookupContext.target_text=v.text;lookup.render(v.text,"en");},error:e=>window.errors.push(String(e))});lookup.render(window.lookupContext.target_text,"en");</script>');return;}
  const file=path.resolve(root,'.'+name);if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}
  fs.readFile(file,(error,data)=>{if(error){res.writeHead(404).end();return;}res.setHeader('content-type',file.endsWith('.js')?'text/javascript':'text/css');res.end(data);});
 });await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});const page=await browser.newPage({viewport:{width:974,height:611}});const pageErrors=[];page.on('pageerror',e=>pageErrors.push(e.message));
  await page.addInitScript(()=>{
   window.errors=[];window.calls=[];window.lookupContext={enabled:true,source_text:'Ein Hauptfenster und ein Fenster.',target_text:'A main window and another window.',source_language:'DE',target_language:'EN-GB',ui_language:'de'};
   window.invoke=async(name,args)=>{window.calls.push({name,args});const delay=window.delay||0;if(delay)await new Promise(r=>setTimeout(r,delay));
    if(name==='lookup_alternatives')return{ticket:'ticket-'+args.request.offset,alternatives:['screen at …','<img src=x onerror=window.injected=true>']};
    if(name==='apply_word_alternative')return{text:'A display and another window.',detected_source:'DE'};
    if(name==='lookup_dictionary')return{query:args.request.query,supported:!window.unsupported,entries:[{word:'window',word_type:'noun',translations:[{text:'Fenster',word_type:'Substantiv',examples:[{source:'Open the window.',target:'Öffne das Fenster.'}]}]}]};
   };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);await page.waitForFunction(()=>window.lookup);
  assert.equal(await page.locator('#result-display').innerText(),'A main window and another window.');
  await page.locator('[data-word-start="26"]').click();await page.locator('.word-alternative').first().waitFor();
  let call=await page.evaluate(()=>window.calls.find(c=>c.name==='lookup_alternatives'));assert.equal(call.args.request.offset,26,'Exact repeated-word occurrence, UTF-16 offset');assert.equal(call.args.request.target_text,'A main window and another window.');
  assert.equal(await page.locator('#word-lookup img').count(),0);assert.equal(await page.evaluate(()=>window.injected),undefined);
  await page.locator('#word-tab-dictionary').click();await page.getByText('Öffne das Fenster.',{exact:true}).waitFor();
  call=await page.evaluate(()=>window.calls.find(c=>c.name==='lookup_dictionary'));assert.equal(call.args.request.query,'window');assert.equal(call.args.request.source_language,'EN-GB');assert.equal(call.args.request.target_language,'DE');
  await page.locator('#word-tab-alternatives').click();await page.locator('.word-alternative').first().click();await page.waitForFunction(()=>window.applied);
  assert.deepEqual(await page.evaluate(()=>window.calls.find(c=>c.name==='apply_word_alternative').args),{ticket:'ticket-26',selection:'screen at …'});assert.equal(await page.locator('#result-display').innerText(),'A display and another window.');assert.equal(await page.locator('#word-lookup').isVisible(),false);
  await page.evaluate(()=>{window.delay=120;lookupContext.target_text='First second';lookup.render(lookupContext.target_text,'en');});await page.locator('[data-word-start="0"]').click();await page.locator('[data-word-start="6"]').click();await page.waitForTimeout(160);await page.locator('.word-alternative').first().click();await page.waitForFunction(()=>window.calls.filter(c=>c.name==='apply_word_alternative').length===2);assert.equal(await page.evaluate(()=>window.calls.filter(c=>c.name==='apply_word_alternative').at(-1).args.ticket),'ticket-6');
  await page.evaluate(()=>{window.delay=120;lookupContext.target_text='Stale text';lookup.render('Stale text','en');});await page.locator('[data-word-start="0"]').click();await page.evaluate(()=>{lookupContext.target_text='Changed';lookup.render('Changed','en');});await page.waitForTimeout(160);assert.equal(await page.locator('#word-lookup').isVisible(),false);
  await page.evaluate(()=>{window.delay=0;lookupContext.target_text='😀 日本語の文章 café';lookup.render(lookupContext.target_text,'ja');});assert.equal(await page.locator('#result-display').innerText(),'😀 日本語の文章 café');assert.ok(await page.locator('[data-word-start]').count()>1);
  await page.locator('[data-word-start]').last().click();await page.keyboard.press('Escape');assert.equal(await page.locator('#word-lookup').isVisible(),false);assert.equal(await page.locator('#result-display').evaluate(el=>el===document.activeElement),true);
  await page.locator('[data-word-start]').last().click();await page.locator('#outside').click();assert.equal(await page.locator('#word-lookup').isVisible(),false);
  await page.evaluate(()=>{document.documentElement.dataset.theme='dark';const display=document.querySelector('#result-display');display.style.cssText='position:absolute;right:8px;bottom:8px;width:240px;height:90px';lookupContext.target_text='Window';lookup.render('Window','en');});await page.locator('[data-word-start]').click();await page.locator('.word-alternative').first().waitFor();
  const bounds=await page.locator('#word-lookup').boundingBox();assert(bounds.x>=0&&bounds.y>=0&&bounds.x+bounds.width<=975&&bounds.y+bounds.height<=612);assert.equal(await page.locator('#word-lookup').evaluate(el=>getComputedStyle(el).backgroundColor),'rgb(23, 37, 50)');
  await page.evaluate(()=>window.unsupported=true);await page.locator('#word-tab-dictionary').click();await page.getByText('Für dieses Sprachpaar ist kein Wörterbuch verfügbar.',{exact:true}).waitFor();
  await page.evaluate(()=>{lookup.close();lookupContext.target_text='Main window';lookup.render('Main window','en');});await page.locator('#result-display').focus();await page.keyboard.press('ArrowRight');await page.keyboard.press('Enter');await page.locator('.word-alternative').first().waitFor();assert.equal(await page.locator('#word-tab-alternatives').evaluate(el=>el===document.activeElement),true);await page.keyboard.press('ArrowRight');await page.locator('#word-tab-dictionary[aria-selected=true]').waitFor();await page.keyboard.press('Escape');
  await page.evaluate(()=>{window.unsupported=false;const display=document.querySelector('#result-display'),range=document.createRange();range.selectNodeContents(display);getSelection().removeAllRanges();getSelection().addRange(range);});await page.locator('#result-display').press('Enter');await page.locator('#word-tab-dictionary').click();await page.waitForFunction(()=>window.calls.filter(c=>c.name==='lookup_dictionary').at(-1).args.request.query==='Main window');
  await page.evaluate(()=>{lookup.close();lookupContext.enabled=false;getSelection().removeAllRanges();});await page.locator('[data-word-start]').first().click();assert.equal(await page.locator('#word-lookup').isVisible(),false);
  assert.deepEqual(pageErrors,[]);
  // Real app integration: canonical output, rich display, speech/copy and transfers.
  const app=await browser.newPage({viewport:{width:974,height:611}});const appErrors=[];app.on('pageerror',e=>appErrors.push(e.message));
  await app.addInitScript(()=>{
   window.calls=[];window.listeners={};window.players=[];window.Audio=class{constructor(){window.players.push(this);}async play(){}pause(){this.paused=true;}};
   let config={provider:'api_free',source:'AUTO',target:'EN-GB',theme:'light',history_enabled:true,hotkey_ms:350,quick_mode_policy:'remember_last',last_mode:'translate',write_language:'DE',write_style:'simple',ui_language:'de',quick_position_policy:'remember_last'};
   const history=[{id:'history-1',created_at:1760000000,favorite:false,mode:'translate',request:{text:'Historisches Original',source:'DE',target:'EN-GB',style:'default'},translation:{text:'Historic window.',detected_source:'DE'}}];
   window.__TAURI__={event:{listen:async(name,handler)=>window.listeners[name]=handler},core:{invoke:async(name,args={})=>{window.calls.push({name,args});
    if(name==='bootstrap')return {config,history,key_present:false,quick_text:'',hotkey_status:'test'};
    if(name==='begin_speech_session')return 1;
    if(name==='save_settings'){config={...args.config};return;}
    if(name==='lookup_alternatives')return {ticket:'app-ticket',alternatives:['display …']};
    if(name==='lookup_dictionary')return {query:args.request.query,supported:true,entries:[{word:args.request.query,word_type:'Substantiv',translations:[{text:'main window',word_type:'noun',examples:[]}]}]};
    if(name==='apply_word_alternative')return {text:'A contextual display.',detected_source:'DE'};
    if(name==='synthesize_speech')return {audio_base64:'SUQz',voice:'en-GB-RyanNeural',preferred_voice:'en-GB-RyanNeural',fallback:false};
   }}};
  });
  await app.goto(`http://127.0.0.1:${server.address().port}/app`);await app.waitForFunction(()=>window.listeners['load-translation']);
  const loadResult=()=>app.evaluate(()=>window.listeners['load-translation']({payload:[{text:'Das Hauptfenster.',source:'AUTO',target:'EN-GB',style:'default'},{text:'The main window.',detected_source:'DE'},'translate']}));
  await loadResult();assert.equal(await app.locator('#result-display').innerText(),'The main window.');assert.equal(await app.locator('#result-display').isVisible(),true);assert.equal(await app.locator('#result-text').isVisible(),false);
  const sourceDictionary=()=>app.locator('#source-text').evaluate(el=>{el.focus();el.setSelectionRange(4,16);const r=el.getBoundingClientRect();el.dispatchEvent(new MouseEvent('dblclick',{bubbles:true,clientX:r.left+60,clientY:r.top+15}));});
  await sourceDictionary();await app.locator('.word-dictionary-entry').waitFor({timeout:2000});
  let sourceCall=await app.evaluate(()=>window.calls.filter(c=>c.name==='lookup_dictionary').at(-1));assert.equal(sourceCall.args.request.query,'Hauptfenster');assert.equal(sourceCall.args.request.source_language,'DE');assert.equal(sourceCall.args.request.target_language,'EN-GB');assert.equal(await app.locator('#word-tab-alternatives').isVisible(),false);assert.equal(await app.locator('#source-text').inputValue(),'Das Hauptfenster.');
  const dictionaryAudio=app.locator('.word-dictionary-speak');assert.equal(await dictionaryAudio.count(),2);
  await dictionaryAudio.first().click();await app.waitForFunction(()=>window.calls.some(c=>c.name==='synthesize_speech'&&c.args.text==='Hauptfenster'&&c.args.language==='DE'));assert.equal(await dictionaryAudio.first().getAttribute('aria-pressed'),'true');
  await dictionaryAudio.last().click();await app.waitForFunction(()=>window.calls.some(c=>c.name==='synthesize_speech'&&c.args.text==='main window'&&c.args.language==='EN-GB'));assert.equal(await dictionaryAudio.first().getAttribute('aria-pressed'),'false');assert.equal(await dictionaryAudio.last().getAttribute('aria-pressed'),'true');
  await dictionaryAudio.last().click();assert.equal(await dictionaryAudio.last().getAttribute('aria-pressed'),'false');
  await dictionaryAudio.first().click();await app.keyboard.press('Escape');assert.equal(await app.locator('#word-lookup').isVisible(),false);assert.equal(await app.locator('#source-text').evaluate(el=>el===document.activeElement),true);assert.equal(await app.locator('#speak-source').getAttribute('aria-pressed'),'false');
  await sourceDictionary();await app.locator('.word-dictionary-entry').waitFor();await app.locator('#source-text').fill('Changed');assert.equal(await app.locator('#word-lookup').isVisible(),false);await loadResult();
  await app.locator('#result-display .result-word').filter({hasText:'window'}).click();await app.locator('.word-alternative').click();await app.waitForFunction(()=>document.querySelector('#result-text').value==='A contextual display.');assert.equal(await app.locator('#result-display').innerText(),'A contextual display.');
  let appCall=await app.evaluate(()=>window.calls.find(c=>c.name==='lookup_alternatives'));assert.equal(appCall.args.request.source_language,'DE');assert.equal(appCall.args.request.target_language,'EN-GB');
  await app.locator('#result-display .result-word').last().click();await app.locator('#word-tab-dictionary').click();await app.locator('.word-dictionary-entry').waitFor();
  await app.locator('.word-dictionary-speak').first().click();await app.waitForFunction(()=>window.calls.filter(c=>c.name==='synthesize_speech').at(-1)?.args.language==='EN-GB');
  await app.locator('.word-dictionary-speak').last().click();await app.waitForFunction(()=>window.calls.filter(c=>c.name==='synthesize_speech').at(-1)?.args.language==='DE');
  await app.keyboard.press('Escape');
  await app.locator('#copy').click();assert.equal(await app.evaluate(()=>window.calls.filter(c=>c.name==='copy_text').at(-1).args.text),'A contextual display.');
  await app.locator('#speak-result').click();await app.waitForFunction(()=>window.calls.some(c=>c.name==='synthesize_speech'));appCall=await app.evaluate(()=>window.calls.filter(c=>c.name==='synthesize_speech').at(-1));assert.equal(appCall.args.text,'A contextual display.');assert.equal(appCall.args.language,'EN-GB');
  await app.locator('#result-display .result-word').last().click();await app.locator('#source-text').fill('Changed');assert.equal(await app.locator('#result-display').isVisible(),false);assert.equal(await app.locator('#word-lookup').isVisible(),false);assert.equal(await app.locator('#speak-result').getAttribute('aria-pressed'),'false');
  await loadResult();await app.locator('#result-display .result-word').last().click();await app.locator('#app-menu-toggle').click();await app.locator('#settings-open').click();assert.equal(await app.locator('#word-lookup').isVisible(),false);await app.locator('#ui-language').selectOption('en');await app.locator('[data-close="settings-dialog"]').click();await app.locator('#result-display .result-word').last().click();assert.equal(await app.locator('#word-tab-dictionary').innerText(),'Dictionary');await app.keyboard.press('Escape');
  await app.locator('[data-view="history"]').click();await app.locator('#history-list .history-open').click();assert.equal(await app.locator('#result-display').innerText(),'Historic window.');assert.equal(await app.locator('#result-text').inputValue(),'Historic window.');
  await app.locator('#result-display .result-word').last().click();await app.locator('#target-language').click();assert.equal(await app.locator('#word-lookup').isVisible(),false);await app.locator('[data-close="language-dialog"]').click();
  await app.locator('#main-mode-write').click();await app.evaluate(()=>window.listeners['load-translation']({payload:[{text:'Write original',source:'AUTO',target:'DE',style:'simple'},{text:'Write result',detected_source:'DE'},'write']}));await app.locator('#result-display .result-word').first().click();assert.equal(await app.locator('#word-lookup').isVisible(),true);assert.equal(await app.locator('#word-tab-alternatives').innerText(),'Replace word');await app.keyboard.press('Escape');
  assert.equal(await app.evaluate(()=>document.documentElement.scrollHeight>innerHeight+1||document.documentElement.scrollWidth>innerWidth+1),false);assert.deepEqual(appErrors,[]);await app.close();
  console.log('Word lookup checks passed: source double-click, dictionary direction, headword/translation audio in both directions, switch/stop/close, exact offsets, contextual apply, dictionary/examples, safe rendering, races, Unicode, keyboard/dismissal, dark bounds; canonical display, copy/speech coherence, settings, locale, history/Quick transfer and Write action integration.');
 }finally{await browser?.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
