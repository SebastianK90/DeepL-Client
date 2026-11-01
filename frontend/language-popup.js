import {t,applyLocale,captureStaticText,currentLocale,localizedError} from './i18n.js';
import {languageChoices} from './languages.js';
captureStaticText();
const $=id=>document.getElementById(id);
const native=Boolean(window.__TAURI__);
const invoke=(name,args={})=>native?window.__TAURI__.core.invoke(name,args):Promise.resolve();
let context={revision:0,open:false,mode:'translate',selected:'DE',ui_language:'de',theme:'light'},visible=[],active=0,busy=false,loading=true,deferred=null,measuredRows=-1,resizeSequence=0;
function theme(){document.documentElement.dataset.theme=context.theme==='system'?(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'):context.theme;}
function orient(placement){document.querySelector('.language-popup-panel').dataset.direction=placement?.above?'above':'below';}
matchMedia('(prefers-color-scheme: dark)').addEventListener('change',theme);
function render(){
 visible=languageChoices(context.mode,currentLocale(),$('popup-search').value);active=Math.max(0,Math.min(active,visible.length-1));
 $('popup-list').replaceChildren(...visible.map((item,index)=>{const button=document.createElement('button');button.className='popup-option';button.id=`language-${item.code}`;button.type='button';button.tabIndex=-1;button.role='option';button.setAttribute('aria-selected',String(item.code===context.selected));button.classList.toggle('active',index===active);const mark=document.createElement('span');mark.className='popup-check';mark.textContent=item.code===context.selected?'✓':'';mark.setAttribute('aria-hidden','true');button.append(mark,document.createTextNode(item.label));button.onclick=()=>choose(item.code);return button;}));
 $('popup-empty').hidden=visible.length!==0;$('popup-empty').textContent=t('Keine Sprache gefunden.');
 $('popup-search').setAttribute('aria-activedescendant',visible[active]?`language-${visible[active].code}`:'');
 if(context.open&&visible.length!==measuredRows){measuredRows=visible.length;invoke('resize_language_popup',{revision:context.revision,rows:visible.length,sequence:++resizeSequence}).catch(()=>{});}
}
function show(next){if(next.revision<context.revision)return;context=next;orient(next.placement);busy=false;measuredRows=-1;applyLocale(context.ui_language);theme();$('popup-search').disabled=false;$('popup-search').value='';$('popup-error').hidden=true;visible=languageChoices(context.mode,currentLocale());active=Math.max(0,visible.findIndex(item=>item.code===context.selected));render();if(context.open)requestAnimationFrame(()=>{$('popup-search').focus();document.querySelector('.popup-option.active')?.scrollIntoView({block:'nearest'});});}
async function choose(code){if(busy||!context.open)return;const revision=context.revision;busy=true;$('popup-search').disabled=true;try{await invoke('choose_quick_language',{revision,code});}catch(error){if(context.revision===revision){$('popup-error').textContent=localizedError(String(error));$('popup-error').hidden=false;}}finally{if(context.revision===revision){busy=false;$('popup-search').disabled=false;}}}
$('popup-search').oninput=()=>{active=0;render();};
document.addEventListener('keydown',event=>{
 if(event.isComposing||event.keyCode===229)return;
 if(event.key==='Escape'){event.preventDefault();invoke('close_language_popup',{revision:context.revision});return;}
 if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)&&(!['Home','End'].includes(event.key)||event.ctrlKey)){event.preventDefault();active=event.key==='Home'?0:event.key==='End'?visible.length-1:Math.max(0,Math.min(visible.length-1,active+(event.key==='ArrowDown'?1:-1)));render();document.querySelector('.popup-option.active')?.scrollIntoView({block:'nearest'});}
 if(event.key==='Enter'&&visible[active]){event.preventDefault();choose(visible[active].code);}
});
async function init(){
 if(!native){show({...context,open:true});return;}
 await window.__TAURI__.event.listen('language-popup-opened',event=>{if(loading)deferred=event.payload;else show(event.payload);});
 await window.__TAURI__.event.listen('language-popup-placement',event=>{const [revision,sequence,placement]=event.payload;if(revision===context.revision&&sequence===resizeSequence)orient(placement);});
 const initial=await invoke('language_popup_state');loading=false;show(deferred&&deferred.revision>=initial.revision?deferred:initial);
}
init().catch(error=>{$('popup-error').textContent=String(error);$('popup-error').hidden=false;});
