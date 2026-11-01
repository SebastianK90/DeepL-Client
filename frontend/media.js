import {createDocuments} from './documents.js';
export function createMedia({invoke,t,notify,config,isNative,isOverlay,onText,capture,onView}){
 const $=id=>document.getElementById(id);let currentView='translate',revision=0,preview=null,busy=false;
 const documents=createDocuments({invoke,t,notify,config});
 function refresh(){documents.refresh();const c=config();$('image-upload').disabled=busy;$('image-status').textContent=busy?t('Bildtext wird erkannt …'):'';$('image-source-language').textContent=c.sourceLabel;$('image-target-language').textContent=c.targetLabel;}
 async function image(file){if(!file||busy)return;const token=++revision;
  if(!['image/png','image/jpeg'].includes(file.type)){notify(t('Bitte ein PNG- oder JPEG-Bild auswählen.'));return;}
  if(file.size>8*1024*1024){notify(t('Das Bild ist zu groß (maximal 8 MB).'));return;}
  if(preview)URL.revokeObjectURL(preview);preview=URL.createObjectURL(file);$('image-preview').src=preview;$('image-preview').hidden=false;$('image-filename').textContent=file.name;
  busy=true;refresh();const language=config().source;
  try{const encoded=await new Promise((resolve,reject)=>{const reader=new FileReader();reader.onload=()=>resolve(String(reader.result).split(',')[1]);reader.onerror=()=>reject(t('Das Bild konnte nicht gelesen werden.'));reader.readAsDataURL(file);});const text=await invoke('recognize_image',{encoded,language});if(token!==revision||currentView!=='images')return;if(language!==config().source)throw t('Die Sprache wurde geändert. Bitte das Bild erneut auswählen.');await onText(text);}
  catch(e){if(token===revision)notify(e instanceof Error?e.message:String(e));}finally{if(token===revision){busy=false;refresh();}$('image-input').value='';}
 }
 $('image-upload').onclick=()=>$('image-input').click();$('image-input').onchange=()=>image($('image-input').files[0]);$('image-capture').onclick=capture;
 const drop=$('image-drop');for(const event of ['dragover','dragleave','drop'])drop.addEventListener(event,e=>{e.preventDefault();drop.classList.toggle('dragging',event==='dragover');if(event==='drop')image(e.dataTransfer.files[0]);});
 window.addEventListener('beforeunload',()=>{if(preview)URL.revokeObjectURL(preview);});
 return{refresh,async nativeDrop(payload){if(isOverlay)return;try{const bytes=Uint8Array.from(atob(payload.encoded),c=>c.charCodeAt(0));const file=new File([bytes],payload.name,{type:payload.mime});if(payload.mime.startsWith('image/')){onView('images');await image(file);}else{onView('files');documents.choose(file);}}catch(e){notify(e instanceof Error?e.message:String(e));}},viewChanged(next){if(next!=='images'){revision++;busy=false;}currentView=next;refresh();},progress:documents.progress};
}

