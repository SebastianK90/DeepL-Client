export function relativeHistoryTime(timestamp,locale,now=Date.now()){
 const seconds=Math.max(0,Math.floor(now/1000)-timestamp);
 if(seconds<60)return new Intl.RelativeTimeFormat(locale,{numeric:'auto'}).format(-seconds,'second');
 const [size,unit]=seconds<3600?[60,'minute']:seconds<86400?[3600,'hour']:seconds<2592000?[86400,'day']:seconds<31536000?[2592000,'month']:[31536000,'year'];
 return new Intl.RelativeTimeFormat(locale,{numeric:'always'}).format(-Math.max(1,Math.floor(seconds/size)),unit);
}
export function trashIcon(){const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('viewBox','0 0 24 24');svg.setAttribute('aria-hidden','true');const path=document.createElementNS(svg.namespaceURI,'path');path.setAttribute('d','M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7');svg.append(path);return svg;}
export function createHistoryCard({entry,t,locale,nameOf,open:openEntry,favorite,remove}){
 const el=document.createElement('article');el.className='history-card';el.dataset.historyId=entry.id;
 const open=document.createElement('button');open.type='button';open.className='history-open';open.title=t('Übersetzung öffnen');
 const make=(tag,cls,text)=>{const node=document.createElement(tag);node.className=cls;if(text!==undefined)node.textContent=text;return node;};
 const count=text=>t('{count} Zeichen',{count:new Intl.NumberFormat(locale).format(Array.from(text).length)});
 const sourceLanguage=entry.mode==='write'?entry.request.target:(entry.translation.detected_source||entry.request.source);
 const source=make('span','history-meta history-source-meta');source.append(make('span','history-source-label',t('Von:')),make('strong','',nameOf(sourceLanguage)),make('span','history-count',count(entry.request.text)));
 if(entry.mode==='write')source.append(make('span','history-mode','Write'));
 const time=make('time','history-time',relativeHistoryTime(entry.created_at,locale));time.dateTime=new Date(entry.created_at*1000).toISOString();time.title=new Intl.DateTimeFormat(locale,{dateStyle:'medium',timeStyle:'short'}).format(new Date(entry.created_at*1000));
 const target=make('span','history-meta history-target-meta');target.append(make('span','history-target-label',t('Nach:')),make('strong','',nameOf(entry.request.target==='DE'?'DE-DE':entry.request.target)),make('span','history-count',count(entry.translation.text)));
 open.append(source,make('span','original history-preview',entry.request.text),make('span','history-divider'),target,make('span','translated history-preview',entry.translation.text));open.onclick=openEntry;
 const star=make('button','favorite-button',entry.favorite?'★':'☆');star.type='button';star.title=t(entry.favorite?'Favorit entfernen':'Als Favorit speichern');star.setAttribute('aria-label',star.title);star.setAttribute('aria-pressed',String(entry.favorite));star.onclick=async()=>{if(star.disabled)return;star.disabled=true;try{await favorite();}finally{star.disabled=false;}};
 const del=make('button','history-delete');del.type='button';del.title=t('Verlaufseintrag löschen');del.setAttribute('aria-label',del.title);del.append(trashIcon());del.onclick=async()=>{if(del.disabled)return;del.disabled=true;try{await remove();}finally{del.disabled=false;}};
 el.append(open,time,star,del);return el;
}
