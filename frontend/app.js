import {t, localizedError, captureStaticText, applyLocale, currentLocale} from './i18n.js';
import {createSpeechController} from './speech.js';
import {createWordLookup} from './word-lookup.js';
import {createShareController} from './share.js';
import {createHistoryCard,trashIcon} from './history.js';
import {createHeader} from './header.js';
import {createMedia} from './media.js';
captureStaticText();
const $ = id => document.getElementById(id);
const isOverlay = new URLSearchParams(location.search).has('overlay');
const native = Boolean(window.__TAURI__);
const invoke = (command, args = {}) => native ? window.__TAURI__.core.invoke(command, args) : Promise.reject(new Error('Bitte die Desktop-App mit cargo run starten. Die Browseransicht zeigt nur die Oberfläche.'));
import {languages, writeLanguages, languageChoices} from './languages.js';
let config = {provider:'api_free',source:'AUTO',target:'DE',theme:'light',history_enabled:true,start_in_tray:true,hotkey_ms:350,quick_mode_policy:'remember_last',last_mode:'translate',write_language:'DE',write_style:'simple',ui_language:'de',quick_position_policy:'remember_last'};
const writingStyles={default:['Standard','Grammatik und Formulierungen verbessern, ohne einen bestimmten Schreibstil vorzugeben.'],simple:['Einfach','Kurze Sätze, verständliche Worte und eine klare Aussage.'],business:['Geschäftlich','Professionell formuliert – für E-Mails, Angebote und deinen Arbeitsalltag.'],academic:['Akademisch','Präzise und sachlich formuliert – mit einer strukturierten Ausdrucksweise.'],casual:['Locker','Unkompliziert, natürlich und nahbar. So, wie du dich unterhältst.']};
const styleLanguages=['DE','EN','EN-GB','EN-US','FR','ES','IT','PT','PT-BR','PT-PT'];
const effectiveStyle=()=>styleLanguages.includes(config.write_language)?config.write_style:'default';
const selectedStyle=style=>['simple','business','academic','casual'].includes(style)?style:'simple';
let styleGeneration=0,pendingMain=false,mainTimer=null,composing=false;

let mainMode = 'translate', quickMode = 'translate', mainBusy = false, quickOpenGeneration = 0;
let ready = false, deferredQuick = null, deferredTranslation = null, keyAvailable = false;
let quickOriginal='',quickSessionId=0,quickCanReplace=false,quickReplacing=false;
let hotkeyStatus='Hotkey wird initialisiert …';
let entries = [], view = 'translate', selectionSide = 'source', result = null, lastDetected = null, generation = 0, quickGeneration = 0, toastTimer;
let saveQueue = Promise.resolve();
let lastSpeechVoice=null;
let shareSupported=true;
const shareController=createShareController({invoke,t,notify,isOverlay,getText:()=>result?.text});
let capturingScreen=false;
const speech=createSpeechController({invoke,enabled:native&&!isOverlay,t,changed:refreshSpeechButtons,failed:notify,voice:info=>{lastSpeechVoice=info;refreshSpeechStatus();}});
const wordLookup=createWordLookup({invoke,t,error:notify,changed:value=>showResult(value),speak:(id,text,language)=>speech.speak(id,text,language),speechState:()=>speech.state(),stopSpeech:()=>speech.stop(),context:()=>({enabled:!isOverlay&&view==='translate'&&Boolean(result),source_enabled:!isOverlay&&mainMode==='translate'&&view==='translate',source_text:$('source-text').value,target_text:result?.text||'',source_language:mainMode==='write'?config.write_language:config.source==='AUTO'?(lastDetected||''):config.source,target_language:mainMode==='write'?config.write_language:config.target,mode:mainMode,style:effectiveStyle(),ui_language:config.ui_language})});
let wordLookupLocale=null;
const mediaController=createMedia({invoke,t,notify,onView:setView,isNative:native,isOverlay,config:()=>({...config,sourceLabel:nameOf(config.source),targetLabel:nameOf(config.target)}),capture:()=>$('capture-screen').click(),onText:async text=>{const switching=selectMode('translate',true,false),token=generation;await switching;if(token!==generation)return;$('source-text').value=text;invalidate();scheduleMain(true);}});
const headerController=createHeader({invoke,notify,isOverlay});
function refreshSpeechStatus(){const status=$('speech-status'),state=speech.state();status.hidden=!lastSpeechVoice||!state.active||state.busy;if(status.hidden)return;const info=lastSpeechVoice;status.textContent=t(info.fallback?'Alternative Edge-Stimme: {voice}':'Edge-TTS: {voice}',{voice:info.voice});status.title=info.fallback?`${info.preferred_voice} → ${info.voice}`:info.voice;}
function sourceSpeechLanguage(){return mainMode==='write'?config.write_language:config.source==='AUTO'?lastDetected:config.source;}
function refreshSpeechButtons(){
  const state=speech.state();
  for(const side of ['source','result']){
    const button=$('speak-'+side),isActive=state.active===side;
    const language=side==='source'?sourceSpeechLanguage():(mainMode==='write'?config.write_language:config.target);
    const text=side==='source'?$('source-text').value:result?.text;
    button.disabled=!isActive&&(!text?.trim()||!language);
    const label=isActive?t(state.busy?'Vorlesen abbrechen':'Wiedergabe stoppen'):!language&&side==='source'?t('Zum Vorlesen zuerst übersetzen oder eine Quellsprache wählen.'):t(side==='source'?'Original vorlesen':'Ergebnis vorlesen');
    button.title=label;button.setAttribute('aria-label',label);button.setAttribute('aria-pressed',String(isActive));button.setAttribute('aria-busy',String(isActive&&state.busy));button.querySelector('span').textContent=isActive?t('Stoppen'):t('Vorlesen');
  }
  refreshSpeechStatus();
  wordLookup.refreshSpeech();
}
const nameOf = code => {if(!code)return '';if(code.toUpperCase()==='AUTO')return t('Automatisch erkennen');try{return new Intl.DisplayNames([currentLocale()],{type:'language'}).of(code.toLowerCase());}catch{return code;}};
const modes = {api_free:'API Free',api_pro:'API Pro',web:t('Web · experimentell')};
function errorText(e) { return e instanceof Error ? e.message : String(e); }
function notify(text) { $('toast').textContent = localizedError(text); $('toast').hidden = false; clearTimeout(toastTimer); toastTimer = setTimeout(() => $('toast').hidden = true, 6000); }
function applyTheme() { document.documentElement.dataset.theme = config.theme === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : config.theme; }
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', applyTheme);
function syncLabels() {
  applyLocale(config.ui_language);
  $('app').classList.toggle('history-active',['history','favorites'].includes(view));
  $('app').classList.toggle('media-active',['images','files'].includes(view));
  mediaController.refresh();
  if(wordLookupLocale!==config.ui_language){wordLookupLocale=config.ui_language;wordLookup.refreshLocale();}
  $('source-language').replaceChildren(document.createTextNode(mainMode === 'write' ? t('Originaltext') : nameOf(config.source)), Object.assign(document.createElement('span'),{textContent:mainMode === 'write' ? '' : '⌄'}));
  $('source-language').disabled = mainMode === 'write'; $('swap').disabled = mainMode === 'write';
  $('target-language').replaceChildren(document.createTextNode(nameOf(mainMode === 'write' ? config.write_language : config.target)), Object.assign(document.createElement('span'),{textContent:'⌄'}));
  $('quick-target-label').textContent = nameOf(quickMode === 'write' ? config.write_language : config.target);
  syncQuickLanguages();
  $('app').classList.toggle('write-active',mainMode==='write' && view==='translate');
  $('write-tools').hidden=mainMode!=='write';
  const supported=styleLanguages.includes(config.write_language);
  $('write-style').value=supported?selectedStyle(config.write_style):'simple';$('write-style').disabled=!supported;
  document.querySelectorAll('[data-style]').forEach(btn=>{btn.disabled=!supported;btn.setAttribute('aria-pressed',String(btn.dataset.style===effectiveStyle()));});
  $('style-language-note').textContent=supported?'':t('Für {language} ist nur die Standardverbesserung verfügbar.',{language:nameOf(config.write_language)});
  $('style-description').textContent=t(writingStyles[effectiveStyle()][1]);
  $('result-panel-label').textContent=mainMode==='write'?t('VERBESSERTER TEXT'):t('ÜBERSETZUNG');
  $('result-display').title=t(mainMode==='write'?'Wortklick: Wort austauschen und Satz umformulieren.':'Wortklick: Alternativen und Wörterbuch. Dabei werden Original und Übersetzung an den DeepL-Webdienst gesendet.');
  $('result-style-badge').textContent=t(writingStyles[effectiveStyle()][0]);
  $('result-style-badge').hidden=mainMode!=='write'||effectiveStyle()==='default';
  $('use-result').hidden=mainMode!=='write';$('use-result').disabled=!result;
  $('quick-edit-label').textContent=t('Bearbeiten in DeepL');
  $('quick-copy').querySelector('[data-button-label]').textContent=t('Kopieren');
  $('quick-replace').querySelector('[data-button-label]').textContent=t('Ersetzen');
  const positionLabels={mouse_cursor:'Am Mauszeiger',centered:'Immer mittig zentrieren',remember_last:'Letzte Position merken'};
  const positionPolicy=Object.hasOwn(positionLabels,config.quick_position_policy)?config.quick_position_policy:'remember_last';
  const positionHint=t('Quick-Assist-Position: {position}',{position:t(positionLabels[positionPolicy])});
  $('quick-position-indicator').title=positionHint;$('quick-position-indicator').setAttribute('aria-label',positionHint);
  for(const icon of $('quick-position-indicator').querySelectorAll('[data-position-policy]'))icon.toggleAttribute('hidden',icon.dataset.positionPolicy!==positionPolicy);
  updateReplaceButton();
  $('mode-explanation').textContent = mainMode === 'write' ? t('Klarer formuliert. In deiner Sprache.') : t('In einer anderen Sprache auf den Punkt.');
  $('keyboard-hint').textContent=mainMode==='write'?t('Text wird automatisch verbessert.'):t('Text wird automatisch übersetzt.');
  $('source-text').placeholder = mainMode === 'write' ? t('Welchen Text möchtest du verbessern?') : t('Was möchtest du übersetzen?');
  $('result-placeholder').querySelector('p').textContent = mainMode === 'write' ? t('Dein Text. Klarer und stilsicherer.') : t('Hier entsteht deine Übersetzung.');
  for (const area of ['main','quick']) for (const mode of ['translate','write']) $(area+'-mode-'+mode).setAttribute('aria-selected',String((area === 'main' ? mainMode : quickMode) === mode && (area==='quick'||view==='translate')));
  $('quick').classList.toggle('write-active',quickMode === 'write');
  $('translation-status').textContent=mainBusy?t(mainMode==='write'?'Wird verbessert …':'Wird übersetzt …'):'';
  $('result-display').setAttribute('aria-busy',String(mainBusy));
  $('mode-label').textContent = t(modes[config.provider]);
  $('request-hint').replaceChildren(Object.assign(document.createElement('span'),{className:'status-dot'}), document.createTextNode(config.provider === 'web' ? t('Web-Backend · experimentell') : t('Offizielle DeepL API')));
  $('privacy-note').textContent = config.history_enabled ? t('Verlauf bleibt auf diesem Gerät.') : t('Verlauf ist ausgeschaltet.');
  $('detected-language').textContent=result?t('{language} erkannt',{language:nameOf(result.detected_source)}):t('Bereit, wenn du es bist.');
  $('page-title').textContent=view==='favorites'?t('Worte, die bleiben.'):t('Ein Blick zurück.');
  $('page-description').textContent=view==='favorites'?t('Deine gespeicherten Lieblingsübersetzungen.'):t('Alle Übersetzungen. Auf deinem Gerät.');
  $('key-status').textContent=keyAvailable?t('✓ Key im Schlüsselspeicher vorhanden.'):t('Kein Key gespeichert.');
  $('hotkey-status').textContent=localizedError(hotkeyStatus);
  applyTheme();
  refreshSpeechButtons();
  refreshShareButton();
}
function updateReplaceButton(){
  $('quick-replace').disabled=!quickResult||!quickCanReplace||quickReplacing;
  $('quick-replace').title=quickCanReplace?'':t('Ersetzen ist auf diesem Desktop nicht verfügbar. Bitte Kopieren verwenden.');
}
async function selectStyle(style) {
  if(!writingStyles[style]||style===config.write_style)return;
  const token=++styleGeneration;
  config.write_style=style;invalidate();quickGeneration++;quickResult=null;$('quick-copy').disabled=true;syncLabels();
  try{await persist();if(token!==styleGeneration)return;if(isOverlay && quickMode==='write' && quickOriginal)await translateQuick(quickOriginal);else if(!isOverlay && mainMode==='write')scheduleMain(true);}catch(e){notify(errorText(e));}
}
async function selectMode(mode, remember = true, process = true) {
  const selection = ++quickOpenGeneration;
  if(isOverlay&&quickLanguageOpen)await invoke('close_language_popup');
  if (isOverlay) {quickMode=mode;quickGeneration++;quickResult=null;$('quick-copy').disabled=true;}
  else {mainMode=mode;invalidate();setView('translate');}
  syncLabels();
  if (remember) { config.last_mode=mode; if(native) await invoke('remember_mode',{mode}); }
  if (selection !== quickOpenGeneration) return;
  if (isOverlay && quickOriginal) await translateQuick(quickOriginal);
  else if(!isOverlay && process)scheduleMain(true);
}
async function openQuick(text) {
  const opening = ++quickOpenGeneration;
  invalidateQuick();
  // Resolve on every hotkey, not only once when the hidden WebView starts.
  const context = native ? await invoke('begin_quick') : {mode:config.quick_mode_policy === 'always_write' ? 'write' : config.quick_mode_policy === 'always_translate' ? 'translate' : config.last_mode,session:0,can_replace:false};
  if(opening !== quickOpenGeneration) return;
  quickSessionId=context.session;quickCanReplace=context.can_replace;quickMode = context.mode || 'translate'; syncLabels(); await translateQuick(text);
}
function persist() {
  const snapshot = {...config};
  saveQueue = saveQueue.catch(() => {}).then(() => invoke('save_settings',{config:snapshot}));
  return saveQueue;
}
function setView(next) {
  if(next!=='translate'){cancelMain();generation++;}
  wordLookup.close();
  view = next;
  $('translate-view').hidden = next !== 'translate'; $('history-view').hidden = !['history','favorites'].includes(next);
  $('images-view').hidden=next!=='images';$('files-view').hidden=next!=='files';headerController.close();
  document.querySelectorAll('[data-view]').forEach(el => el.classList.toggle('active', el.dataset.view === next));
  $('page-title').textContent = next === 'translate' ? t('Raum für klare Worte.') : next === 'favorites' ? t('Worte, die bleiben.') : t('Ein Blick zurück.');
  $('page-description').textContent = next === 'translate' ? t('Deine Gedanken. In jeder Sprache.') : next === 'favorites' ? t('Deine gespeicherten Lieblingsübersetzungen.') : t('Alle Übersetzungen. Auf deinem Gerät.');
  $('clear-history').hidden = next === 'favorites'; renderHistory();
  syncLabels();mediaController.viewChanged(next);
}
function card(entry) {
  return createHistoryCard({entry,t,locale:currentLocale(),nameOf,
    open:()=>{ cancelMain();generation++;mainMode=entry.mode||'translate';if(mainMode==='write'){config.write_language=entry.request.target;config.write_style=selectedStyle(entry.request.style);}else{config.source = entry.request.source; config.target = entry.request.target;} $('source-text').value = entry.request.text; showResult(entry.translation); count(); syncLabels(); setView('translate'); },
    favorite:async()=>{try{await invoke('toggle_favorite',{id:entry.id});}catch(e){notify(errorText(e));}},
    remove:async()=>{try{await invoke('delete_history_entry',{id:entry.id});}catch(e){notify(errorText(e));}}
  });
}
function empty(text, hint) { const el = document.createElement('div'); el.className = 'empty-history'; const small = document.createElement('small'); small.textContent = hint; el.append(document.createTextNode(text),small); return el; }
function renderHistory() {
  $('history-count').textContent = entries.length;
  $('clear-history').disabled=entries.length===0;
  const recent = entries.slice(0,2); $('recent-list').replaceChildren(...(recent.length ? recent.map(card) : [empty(t('Ein guter Anfang wartet auf dich.'),t('Deine letzten Übersetzungen erscheinen hier.'))]));
  const q = $('history-search').value.toLocaleLowerCase();
  const list = entries.filter(e => (view !== 'favorites' || e.favorite) && `${e.request.text} ${e.translation.text}`.toLocaleLowerCase().includes(q));
  $('history-list').replaceChildren(...(list.length ? list.map(card) : [empty(q ? t('Keine passenden Übersetzungen.') : view === 'favorites' ? t('Noch keine Favoriten.') : t('Dein Verlauf ist noch leer.'),view === 'favorites' ? t('Mit dem Stern im Verlauf speicherst du Lieblingsübersetzungen.') : t('Hier findest du deine lokal gespeicherten Übersetzungen.'))]));
}
function count() { const n = Array.from($('source-text').value).length; $('char-count').textContent = `${n.toLocaleString(currentLocale())} / ${(5000).toLocaleString(currentLocale())}`; $('char-count').style.color = n>5000 ? '#c65454' : '';$('copy-source').disabled=n===0;$('capture-screen').disabled=capturingScreen||mainBusy;refreshSpeechButtons(); }
function showResult(value) {
  speech.stop();
  result = value; lastDetected = value?.detected_source || null;
  $('result-text').value = value?.text || ''; $('result-text').hidden = true; $('result-display').hidden = !value;wordLookup.render(value?.text||'',mainMode==='write'?config.write_language:config.target);$('result-placeholder').hidden = Boolean(value); $('copy').disabled = !value;
  $('use-result').disabled=!value;
  refreshShareButton();
  $('detected-language').textContent = value ? t('{language} erkannt',{language:nameOf(value.detected_source)}) : t('Bereit, wenn du es bist.');
  refreshSpeechButtons();
}
function cancelMain(){clearTimeout(mainTimer);mainTimer=null;pendingMain=false;}
function canProcessMain(){const text=$('source-text').value;return native&&ready&&!isOverlay&&!composing&&view==='translate'&&Boolean(text.trim())&&Array.from(text).length<=5000;}
function scheduleMain(immediate=false){
  clearTimeout(mainTimer);mainTimer=null;
  if(!canProcessMain())return;
  if(immediate){void doTranslate();return;}
  mainTimer=setTimeout(()=>{mainTimer=null;void doTranslate();},350);
}
function invalidate() { cancelMain();generation++; showResult(null); count(); }
function refreshShareButton(){shareController.refresh(shareSupported);}
async function doTranslate() {
  clearTimeout(mainTimer);mainTimer=null;
  if (!canProcessMain()) return;
  if(mainBusy){pendingMain=true;return;}
  pendingMain=false;
  const token = ++generation; const text = $('source-text').value;
  const request = {text,source:mainMode==='write'?'AUTO':config.source,target:mainMode==='write'?config.write_language:config.target,style:mainMode==='write'?effectiveStyle():'default'};
  mainBusy=true;syncLabels();count();$('result-placeholder').classList.add('loading');
  try { const response = await invoke(mainMode,{request}); if (token === generation) showResult(response); }
  catch(e) { if (token === generation) notify(errorText(e)); }
  finally { mainBusy=false;syncLabels(); $('result-placeholder').classList.remove('loading'); count();if(pendingMain){pendingMain=false;void doTranslate();} }
}
async function copy(text,button) {
  if (!text) return;
  try { await invoke('copy_text',{text}); const label=button.querySelector('[data-button-label]')||button.querySelector('span')||button;label.textContent = t('✓ Kopiert'); button.classList.add('copy-success'); setTimeout(()=> {label.textContent = t(button.id==='copy-source'?'Original kopieren':'Kopieren');button.classList.remove('copy-success');},1300); }
  catch(e) { notify(errorText(e)); }
}
function openLanguage(side) { wordLookup.close();selectionSide = side; $('language-search').value = ''; renderLanguages(); $('language-dialog').showModal(); $('language-search').focus(); }
let quickLanguageOpen=false,inlineQuickLanguages=false;
function syncQuickLanguages(){ $('quick-target').setAttribute('aria-expanded',String(quickLanguageOpen)); }
async function toggleQuickLanguage(){
 if(inlineQuickLanguages){
  if(quickLanguageOpen){await invoke('close_language_popup');return;}
  const rect=$('quick-target').getBoundingClientRect();quickLanguageOpen=true;syncQuickLanguages();
  try{await invoke('open_language_popup',{mode:quickMode,anchor:{x:rect.x,y:rect.y,width:rect.width,height:rect.height},choices:languageChoices(quickMode,currentLocale()).map(({code,label})=>({code,label})),placeholder:t('Sprache suchen …')});}
  catch(e){quickLanguageOpen=false;syncQuickLanguages();openLanguage(quickMode==='write'?'write_language':'target');}
  return;
 }
 if(quickLanguageOpen){await invoke('close_language_popup');return;}
 const rect=$('quick-target').getBoundingClientRect();quickLanguageOpen=true;syncQuickLanguages();
 try{await invoke('open_language_popup',{mode:quickMode,anchor:{x:rect.x,y:rect.y,width:rect.width,height:rect.height}});}catch(e){quickLanguageOpen=false;syncQuickLanguages();notify(errorText(e));}
}
function invalidateQuick() {quickGeneration++;pendingQuick=null;quickResult=null;$('quick-copy').disabled=true;updateReplaceButton();$('quick-output').textContent='';}
function renderLanguages() {
  const q = $('language-search').value.toLocaleLowerCase();
  const options = languages.filter(([code,name])=>(selectionSide !== 'write_language' || writeLanguages.includes(code)) && (selectionSide !== 'target' || code !== 'AUTO') && `${name} ${code} ${nameOf(code)}`.toLocaleLowerCase().includes(q));
  $('language-list').replaceChildren(...options.map(([code])=> { const btn = document.createElement('button'); btn.className = 'language-option';btn.classList.toggle('selected',config[selectionSide] === code); const label = document.createElement('small');label.textContent = code;btn.append(document.createTextNode(nameOf(code)),label);btn.addEventListener('click', async()=> {config[selectionSide] = code;if(isOverlay)invalidateQuick();syncLabels();invalidate();$('language-dialog').close();try{await persist();if(isOverlay && quickOriginal) await translateQuick(quickOriginal);else if(!isOverlay)scheduleMain(true);}catch(e){notify(errorText(e));}});return btn; }));
}
async function swap() {
  const source = config.source === 'AUTO' ? (lastDetected || 'EN') : config.source;
  const sourceBase = source.toUpperCase();
  config.source = config.target;
  config.target = languages.some(([code])=>code===sourceBase) ? sourceBase : 'EN';
  if (result) $('source-text').value = result.text;
  syncLabels(); invalidate();
  try { await persist();scheduleMain(true); } catch(e) {notify(errorText(e));}
}

let pendingQuick = null, quickRunning = false, quickResult = null;
async function translateQuick(text) {
  quickResult = null;
  pendingQuick = {text, source:quickMode==='write'?'AUTO':config.source, target:quickMode==='write'?config.write_language:config.target,style:quickMode==='write'?effectiveStyle():'default', mode:quickMode,session:quickSessionId, token:++quickGeneration};
  quickOriginal = text; $('quick-output').textContent = quickMode==='write'?t('Wird verbessert …'):t('Wird übersetzt …'); $('quick-copy').disabled = true;updateReplaceButton();
  if (quickRunning) return;
  quickRunning = true;
  try {
    while (pendingQuick) {
      const {token,mode,session,...request} = pendingQuick; pendingQuick = null;
      try { const response = await invoke(mode,{request,quickSession:session}); if (token === quickGeneration) {quickResult = response;lastDetected = response.detected_source;$('quick-output').textContent = response.text;$('quick-copy').disabled = false;updateReplaceButton();} }
      catch(e) { if (token === quickGeneration) $('quick-output').textContent = localizedError(errorText(e)); }
    }
  } finally { quickRunning = false; }
}

document.querySelectorAll('[data-view]').forEach(el=>el.addEventListener('click',()=>setView(el.dataset.view)));
document.querySelectorAll('[data-close]').forEach(el=>el.addEventListener('click',() => $(el.dataset.close).close()));
document.querySelectorAll('dialog').forEach(dialog=>dialog.addEventListener('click',e=>{if(e.target===dialog){const r=dialog.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)dialog.close();}}));
$('source-language').onclick = () => openLanguage('source'); $('target-language').onclick = () => openLanguage(mainMode==='write'?'write_language':'target');
$('quick-target').onclick = ()=>toggleQuickLanguage().catch(e=>notify(errorText(e)));
for(const area of ['main','quick'])for(const mode of ['translate','write'])$(area+'-mode-'+mode).onclick=()=>selectMode(mode).catch(e=>notify(errorText(e)));
$('language-search').oninput = renderLanguages; $('history-search').oninput = renderHistory;
$('swap').onclick = swap;
$('document-source-language').onclick=()=>openLanguage('source');$('document-target-language').onclick=()=>openLanguage('target');
$('image-source-language').onclick=()=>openLanguage('source');$('image-target-language').onclick=()=>openLanguage('target');
$('source-text').oninput = e=>{invalidate();if(!e.isComposing&&!composing)scheduleMain(e.inputType==='insertFromPaste'||e.inputType==='insertFromDrop');};
$('source-text').addEventListener('compositionstart',()=>{composing=true;invalidate();});
$('source-text').addEventListener('compositionend',()=>{composing=false;invalidate();scheduleMain();});
$('speak-source').onclick=()=>speech.speak('source',$('source-text').value,sourceSpeechLanguage());
$('speak-result').onclick=()=>{if(result)speech.speak('result',result.text,mainMode==='write'?config.write_language:config.target);};
window.addEventListener('beforeunload',()=>speech.stop());
$('write-style').onchange=()=>selectStyle($('write-style').value);
document.querySelectorAll('[data-style]').forEach(btn=>btn.onclick=()=>selectStyle(btn.dataset.style));
$('use-result').onclick=()=>{if(result){$('source-text').value=result.text;invalidate();$('source-text').focus();scheduleMain(true);}};
$('clear-history').prepend(trashIcon());
$('history-search-toggle').onclick=()=>{const search=$('history-search');search.hidden=!search.hidden;$('history-search-toggle').setAttribute('aria-expanded',String(!search.hidden));if(!search.hidden)search.focus();else{search.value='';renderHistory();}};
$('back-editor').onclick=()=>setView('translate');
$('clear').onclick = ()=> {$('source-text').value = '';invalidate();$('source-text').focus();};
$('paste').onclick = async()=> {const token=generation,resume=mainTimer!==null||pendingMain;cancelMain();try{const text=await invoke('read_clipboard');if(token!==generation)return;$('source-text').value=text;invalidate();$('source-text').focus();scheduleMain(true);}catch(e){if(token===generation){notify(errorText(e));if(resume)scheduleMain();}}};
$('copy').onclick = ()=>copy(result?.text,$('copy')); $('quick-copy').onclick = ()=>copy(quickResult?.text,$('quick-copy'));
$('copy-source').onclick=()=>copy($('source-text').value,$('copy-source'));
$('capture-screen').onclick=async()=>{
  if(isOverlay||capturingScreen||mainBusy)return;
  let token=generation;capturingScreen=true;speech.stop();wordLookup.close();$('capture-screen').setAttribute('aria-busy','true');count();
  try{const text=await invoke('capture_screenshot',{language:config.source,hint:t('Bereich ziehen · Esc zum Abbrechen')});if(token!==generation||text===null||text===undefined)return;if(typeof text!=='string'||!text.trim())throw new Error(t('Im Screenshot wurde kein Text erkannt.'));if(Array.from(text).length>5000)throw new Error(t('Der Screenshot enthält mehr als 5.000 Zeichen.'));const switching=selectMode('translate',true,false);token=generation;await switching;if(token!==generation)return;$('source-text').value=text;invalidate();$('source-text').focus();await doTranslate();}
  catch(e){if(token===generation)notify(errorText(e));}
  finally{capturingScreen=false;$('capture-screen').setAttribute('aria-busy','false');count();}
};
$('show-history').onclick = ()=>setView('history');
$('clear-history').onclick = async()=> {if(!entries.length)return;if(confirm(t('Den gesamten Übersetzungsverlauf einschließlich Favoriten löschen?')))try{await invoke('clear_history');}catch(e){notify(errorText(e));}};
$('licenses-open').onclick=async()=>{try{const text=await invoke('third_party_licenses');$('licenses-text').textContent=text;$('licenses-dialog').showModal();}catch(e){notify(errorText(e));}};
$('theme-toggle').onclick = async()=> {config.theme = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';applyTheme();try{await persist();}catch(e){notify(errorText(e));}};
function providerInfo() { $('provider-info').textContent = $('provider').value === 'web' ? t('Translate und Write über das experimentelle OneShot-Backend ohne API-Key. Server können Anfragen ablehnen.') : t('Offizielle REST-API. Write benötigt eine entsprechende API-Berechtigung; ein Übersetzungs-Key allein garantiert keinen Write-Zugang.'); }
$('settings-open').onclick = ()=> {wordLookup.close();$('ui-language').value=config.ui_language;$('provider').value=config.provider;$('theme').value=config.theme;$('hotkey-ms').value=config.hotkey_ms;$('quick-mode-policy').value=config.quick_mode_policy;$('quick-position-policy').value=config.quick_position_policy;$('history-enabled').checked=config.history_enabled;$('start-in-tray').checked=config.start_in_tray;providerInfo();$('settings-dialog').showModal();};
$('ui-language').onchange=async()=>{const previous=config.ui_language;config.ui_language=$('ui-language').value;syncLabels();count();renderHistory();providerInfo();try{await persist();notify(t('Einstellungen gespeichert.'));}catch(e){config.ui_language=previous;$('ui-language').value=previous;syncLabels();renderHistory();notify(errorText(e));}};
$('quick-settings').onclick=()=>invoke('open_main',{request:null,translation:null,mode:null,settings:true}).catch(e=>notify(errorText(e)));
$('provider').onchange = providerInfo;
$('key-save').onclick = async()=> {try{await invoke('save_api_key',{key:$('api-key').value});keyAvailable=true;$('api-key').value='';$('key-status').textContent=t('✓ Sicher gespeichert.');notify(t('API-Key im Betriebssystem-Tresor gespeichert.'));}catch(e){notify(errorText(e));}};
$('key-delete').onclick = async()=> {try{await invoke('delete_api_key');keyAvailable=false;$('key-status').textContent=t('Kein Key gespeichert.');notify(t('API-Key entfernt.'));}catch(e){notify(errorText(e));}};
$('settings-form').onsubmit = async e=> {e.preventDefault();const next={...config,provider:$('provider').value,theme:$('theme').value,hotkey_ms:Number($('hotkey-ms').value),history_enabled:$('history-enabled').checked,start_in_tray:$('start-in-tray').checked,quick_mode_policy:$('quick-mode-policy').value,quick_position_policy:$('quick-position-policy').value};const changedProvider=next.provider!==config.provider;try{await invoke('save_settings',{config:next});config=next;syncLabels();if(changedProvider){invalidate();scheduleMain();}$('settings-dialog').close();notify(t('Einstellungen gespeichert.'));}catch(err){notify(errorText(err));}};
$('quick-close').onclick = ()=>{quickGeneration++;quickOpenGeneration++;pendingQuick=null;invoke('hide_quick').catch(e=>notify(errorText(e)));};
$('quick-main').onclick = ()=>invoke('open_main',{request:{text:quickOriginal,source:quickMode==='write'?'AUTO':config.source,target:quickMode==='write'?config.write_language:config.target,style:quickMode==='write'?effectiveStyle():'default'},translation:quickResult,mode:quickMode,settings:false}).catch(e=>notify(errorText(e)));
$('quick-replace').onclick=async()=>{if(!quickResult||!quickCanReplace||quickReplacing)return;const session=quickSessionId;quickReplacing=true;updateReplaceButton();try{await invoke('replace_selection',{session,displayed:quickResult.text});if(session===quickSessionId)quickCanReplace=false;}catch(e){notify(errorText(e));}finally{quickReplacing=false;updateReplaceButton();}};
$('quick').querySelector('[data-quick-drag]').addEventListener('mousedown',e=>{if(e.button===0 && !e.target.closest('button,a,input,select') && native){e.preventDefault();invoke('start_quick_drag',{grabX:e.clientX,grabY:e.clientY}).catch(err=>notify(errorText(err)));}});
for(const handle of $('quick').querySelectorAll('[data-quick-resize]'))handle.addEventListener('mousedown',e=>{if(e.button===0&&native){e.preventDefault();e.stopPropagation();invoke('start_quick_resize',{edge:handle.dataset.quickResize}).catch(err=>notify(errorText(err)));}});
document.addEventListener('keydown',e=> {if(e.key==='Escape' && isOverlay && !document.querySelector('dialog[open]')){if(quickLanguageOpen)invoke('close_language_popup').catch(()=>{});else invoke('hide_quick').catch(()=>{});}if(e.ctrlKey && e.key==='Enter' && !isOverlay && !document.querySelector('dialog[open]')){e.preventDefault();doTranslate();}});

function loadTranslation(payload) {const [request,translation,mode]=payload;cancelMain();generation++;mainMode=mode||'translate';if(mainMode==='write'){config.write_language=request.target;config.write_style=selectedStyle(request.style);}else{config.source=request.source;config.target=request.target;}$('source-text').value=request.text;showResult(translation);syncLabels();count();setView('translate');if(!translation)scheduleMain(true);}

async function init() {
  $('app').hidden = isOverlay; $('quick').hidden = !isOverlay; document.body.classList.toggle('overlay',isOverlay);
  syncLabels();count();renderHistory();
  if (!native) {hotkeyStatus='Oberflächenvorschau · Desktop-Funktionen benötigen cargo run';$('hotkey-status').textContent=t(hotkeyStatus);return;}
  // Register listeners before reading initial state so a hotkey during startup cannot be lost.
  const listen = window.__TAURI__.event.listen;
  if(!isOverlay)await listen('open-settings',()=>$('settings-open').click());
  if(!isOverlay)await listen('speech-stop',()=>{speech.stop();wordLookup.close();});
  await listen('notice',e=>notify(e.payload));
  if(!isOverlay){await listen('document-progress',e=>mediaController.progress(e.payload));await listen('native-file-drop',e=>mediaController.nativeDrop(e.payload));}
  await listen('hotkey-status',e=>{hotkeyStatus=e.payload;$('hotkey-status').textContent=localizedError(hotkeyStatus);});
  await listen('settings-changed',e=> {const keys=['provider','source','target','write_language','write_style'];const changed=keys.some(k=>e.payload[k]!==undefined && config[k]!==e.payload[k]);const languageChanged=e.payload.ui_language && config.ui_language!==e.payload.ui_language;config={...config,...e.payload};if(changed){invalidate();scheduleMain();quickGeneration++;quickResult=null;$('quick-copy').disabled=true;}syncLabels();if(languageChanged){count();renderHistory();if($('settings-dialog').open)providerInfo();if($('language-dialog').open)renderLanguages();}});
  if (!isOverlay) await listen('load-translation',e=> {if(!ready){deferredTranslation=e.payload;return;}loadTranslation(e.payload);});
  if(!isOverlay)await listen('history-changed',async()=> {try{entries=(await invoke('bootstrap')).history;renderHistory();}catch(e){notify(errorText(e));}});
  if(isOverlay){await listen('quick-language-popup-closed',()=>{quickLanguageOpen=false;syncQuickLanguages();});await listen('quick-language-selected',async e=>{if(e.payload===quickMode&&quickOriginal)await translateQuick(quickOriginal);});}
  if (isOverlay) await listen('quick-text',e=>{if(!ready){deferredQuick=e.payload;return;}return openQuick(e.payload).catch(err=>notify(errorText(err)));});
  const state = await invoke('bootstrap');inlineQuickLanguages=state.inline_quick_languages===true;config={...config,...state.config};keyAvailable=state.key_present;hotkeyStatus=state.hotkey_status;entries=state.history;shareSupported=state.share_supported!==false;syncLabels();count();renderHistory();$('key-status').textContent=state.key_present?t('✓ Key im Schlüsselspeicher vorhanden.'):t('Kein Key gespeichert.');
  ready=true;
  if(!isOverlay&&state.startup_notice){
    const showRecovery=()=>notify(t(state.startup_notice));
    if(document.hidden){const onVisible=()=>{if(!document.hidden){document.removeEventListener('visibilitychange',onVisible);showRecovery();}};document.addEventListener('visibilitychange',onVisible);}
    else showRecovery();
  }
  if(!isOverlay){if(deferredTranslation){loadTranslation(deferredTranslation);deferredTranslation=null;}else scheduleMain();}
  if (isOverlay && (deferredQuick || state.quick_text) && !quickRunning) await openQuick(deferredQuick || state.quick_text);
}
init().catch(e=>notify(errorText(e)));



