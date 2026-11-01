export function createHeader({invoke,notify,isOverlay}){
 const $=id=>document.getElementById(id),menu=$('app-menu');
 const close=()=>{menu.hidden=true;$('app-menu-toggle').setAttribute('aria-expanded','false');};
 $('app-menu-toggle').onclick=()=>{menu.hidden=!menu.hidden;$('app-menu-toggle').setAttribute('aria-expanded',String(!menu.hidden));};
 menu.addEventListener('click',e=>{if(e.target.closest('button'))close();});
 document.addEventListener('pointerdown',e=>{if(!menu.hidden&&!menu.contains(e.target)&&!e.target.closest('#app-menu-toggle'))close();});
 document.addEventListener('keydown',e=>{if(e.key==='Escape'&&!menu.hidden){close();$('app-menu-toggle').focus();}});
 const action=action=>{if(!isOverlay)invoke('main_window_action',{action}).catch(e=>notify(String(e)));};
 for(const [id,name]of[['window-minimize','minimize'],['window-maximize','maximize'],['window-close','close']])$(id).onclick=()=>action(name);
 const header=document.querySelector('.app-topbar');header.addEventListener('mousedown',e=>{if(e.button===0&&e.detail===1&&!e.target.closest('button,a,input,nav'))action('drag');});header.addEventListener('dblclick',e=>{if(!e.target.closest('button,a,input,nav'))action('maximize');});

 return{close};
}

