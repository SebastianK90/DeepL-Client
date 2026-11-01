import assert from 'node:assert/strict';
import {createSpeechController} from '../frontend/speech.js';
let channel,resolveRequest,plays=0,pauses=0;
globalThis.MediaSource=class {
 static isTypeSupported(type){return type==='audio/mpeg';}
 constructor(){this.readyState='closed';this.listeners={};queueMicrotask(()=>{this.readyState='open';this.listeners.sourceopen?.();});}
 addEventListener(name,fn){this.listeners[name]=fn;}
 removeEventListener(name){delete this.listeners[name];}
 addSourceBuffer(){return {updating:false,appendBuffer(){},addEventListener(){},removeEventListener(){},abort(){}};}
 endOfStream(){this.readyState='ended';}
};
globalThis.Audio=class {async play(){plays++;}pause(){pauses++;}};
URL.createObjectURL=()=> 'blob:test';URL.revokeObjectURL=()=>{};
const controller=createSpeechController({createChannel:()=>channel={},invoke:async(name)=>{
 if(name==='begin_speech_session')return 1;
 if(name==='stream_speech')return new Promise(r=>resolveRequest=r);
}});
const pending=controller.speak('source','Hello','EN');await new Promise(r=>setTimeout(r,0));
assert(channel,'native streaming channel must be created');
channel.onmessage({event:'started',data:{voice:'en-GB-RyanNeural',fallback:false}});
channel.onmessage({event:'audio',data:'SUQz'});await new Promise(r=>setTimeout(r,0));
assert.equal(plays,1,'playback must start before synthesis finishes');
assert.equal(controller.state().busy,false);
controller.stop();assert(pauses>0);
channel.onmessage({event:'audio',data:'SUQz'});
resolveRequest({audio_base64:'',voice:'en-GB-RyanNeural'});await pending;
assert.equal(plays,1,'late streaming chunks must not restart stopped playback');
console.log('Speech streaming checks passed: playback before completion, cancellation, stale chunks.');
