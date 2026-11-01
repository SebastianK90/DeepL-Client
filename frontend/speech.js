// One playback at a time. Request ordering is also checked by the Rust backend.
export function createSpeechController({invoke,enabled=true,changed=()=>{},failed=()=>{},voice=()=>{},t=value=>value,createChannel=()=>typeof window!=='undefined'&&window.__TAURI__?.core.Channel?new window.__TAURI__.core.Channel():null}) {
 let sequence=0,active=null,busy=false,audio=null,url=null,stream=null;
 const session=enabled?invoke('begin_speech_session').catch(()=>null):Promise.resolve(null);
 function dispose(){stream?.dispose();stream=null;if(audio){audio.onended=null;audio.onerror=null;audio.pause();audio.src='';audio=null;}if(url){URL.revokeObjectURL(url);url=null;}}
 function stop(cancel=true){const order=++sequence;dispose();active=null;busy=false;changed();if(cancel&&enabled)session.then(id=>{if(id!==null)return invoke('cancel_speech',{session:id,sequence:order});}).catch(()=>{});}
 async function speak(side,text,language){
  if(active===side){stop();return;}
  stop();const order=++sequence;active=side;busy=true;changed();
  try{
   const id=await session;if(order!==sequence)return;
   if(id===null)throw new Error(t('Sprachausgabe konnte nicht initialisiert werden.'));
   const channel=typeof MediaSource!=='undefined'&&MediaSource.isTypeSupported('audio/mpeg')?createChannel():null;
   let metadata=null;
   const playbackError=()=>{if(order===sequence){stop();failed(t('Audio konnte nicht abgespielt werden.'));}};
   const attachAudio=()=>{audio.onended=()=>{if(order===sequence)stop(false);};audio.onerror=playbackError;};
   if(channel)channel.onmessage=message=>{
    if(order!==sequence)return;
    try{
     if(message.event==='started'){metadata=message.data;return;}
     if(message.event==='finished'){if(!stream)throw new Error('missing audio');stream.finish();return;}
     if(message.event!=='audio')return;
     if(!stream){
      const media=new MediaSource();url=URL.createObjectURL(media);audio=new Audio(url);attachAudio();
      stream=createMpegStream(media,playbackError);
      audio.play().then(()=>{if(order!==sequence)return;busy=false;if(metadata)voice(metadata);changed();}).catch(playbackError);
     }
     stream.push(Uint8Array.from(atob(message.data),char=>char.charCodeAt(0)));
    }catch{playbackError();}
   };
   const response=await invoke(channel?'stream_speech':'synthesize_speech',{session:id,sequence:order,text,language,...(channel?{onAudio:channel}:{})});if(order!==sequence)return;
   if(channel)return;
   const bytes=Uint8Array.from(atob(response.audio_base64),char=>char.charCodeAt(0));
   url=URL.createObjectURL(new Blob([bytes],{type:'audio/mpeg'}));audio=new Audio(url);
   audio.onended=()=>{if(order===sequence)stop(false);};
   audio.onerror=()=>{if(order===sequence){stop(false);failed(t('Audio konnte nicht abgespielt werden.'));}};
   try{await audio.play();}catch{throw new Error(t('Audio konnte nicht abgespielt werden.'));}if(order!==sequence)return;busy=false;voice(response);changed();
  }catch(error){if(order!==sequence)return;stop(false);failed(error instanceof Error?error.message:String(error));}
 }
 return {speak,stop,state:()=>({active,busy})};
}

// Ordered MP3 frames are appended as they arrive. Unsupported WebViews keep the
// complete-file path above; no system/browser voice substitution is involved.
function createMpegStream(media,failed){
 let buffer=null,queue=[],finished=false,disposed=false;
 function pump(){if(disposed||!buffer||buffer.updating||media.readyState!=='open')return;try{if(queue.length)buffer.appendBuffer(queue.shift());else if(finished)media.endOfStream();}catch{failed();}}
 const opened=()=>{if(disposed)return;try{buffer=media.addSourceBuffer('audio/mpeg');buffer.addEventListener('updateend',pump);buffer.addEventListener('error',failed);pump();}catch{failed();}};
 media.addEventListener('sourceopen',opened);
 return {push(bytes){if(disposed)return;queue.push(bytes);pump();},finish(){finished=true;pump();},dispose(){disposed=true;queue=[];media.removeEventListener('sourceopen',opened);if(buffer){buffer.removeEventListener('updateend',pump);buffer.removeEventListener('error',failed);try{if(buffer.updating)buffer.abort();}catch{}}}};
}
