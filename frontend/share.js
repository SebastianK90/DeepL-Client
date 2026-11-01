export function createShareController({invoke,t,notify,isOverlay,getText}) {
  const $=id=>document.getElementById(id);
  const dialog=$('share-dialog'),button=$('share');
  let busy=false,supported=true,snapshot='';
  const targets=[$('share-teams'),$('share-outlook')];
  function refresh(available=supported){
    supported=available;
    button.disabled=isOverlay||!getText()?.trim()||!supported||busy;
    button.setAttribute('aria-busy',String(busy));
    button.title=t('Ergebnis teilen');button.setAttribute('aria-label',button.title);
    targets.forEach(el=>{el.disabled=busy;el.setAttribute('aria-busy',String(busy));});
  }
  function close(){dialog.close();if(!button.disabled)button.focus();}
  button.onclick=()=>{
    if(button.disabled)return;
    snapshot=getText();$('share-preview').textContent=snapshot;
    $('share-error').textContent='';$('share-error').hidden=true;
    dialog.showModal();targets[0].focus();
  };
  $('share-close').onclick=close;
  dialog.addEventListener('close',()=>{if(!button.disabled)button.focus();});
  dialog.addEventListener('click',e=>{if(e.target!==dialog)return;const b=dialog.getBoundingClientRect();if(e.clientX<b.left||e.clientX>b.right||e.clientY<b.top||e.clientY>b.bottom)close();});
  for(const target of ['teams','outlook'])$('share-'+target).onclick=async()=>{
    if(busy||!snapshot)return;
    const text=snapshot;
    busy=true;$('share-error').hidden=true;refresh();
    try{
      await invoke('share_text',{target,text});
      dialog.close();
      notify(t(target==='teams'
        ?'Text kopiert. In Teams einen Chat wählen und mit Strg+V einfügen.'
        :'Text kopiert – bei Bedarf im Entwurf einfügen.'));
    }catch(error){
      $('share-error').textContent=t(error instanceof Error?error.message:String(error));
      $('share-error').hidden=false;
    }finally{busy=false;refresh();if(!dialog.open&&!button.disabled)button.focus();}
  };
  return {refresh};
}
