// Real Edge/WebView-compatible MP3 decoding and MediaSource streaming, no network.
const {chromium}=require('playwright');
const fs=require('node:fs'),http=require('node:http'),assert=require('node:assert/strict');
(async()=>{
 const audio=fs.readFileSync('.tools/edge-speech-de.mp3').toString('base64');
 const server=http.createServer((req,res)=>{
  if(req.url==='/speech.js'){res.setHeader('content-type','text/javascript');res.end(fs.readFileSync('frontend/speech.js'));return;}
  res.setHeader('content-type','text/html');res.end('<button id="play">Play</button><script type="module">import {createSpeechController} from "/speech.js";window.controller=createSpeechController({invoke:window.invoke,createChannel:()=>({})});document.querySelector("button").onclick=()=>window.controller.speak("source","Synthetic test","DE");</script>');
 });await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});const page=await browser.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(({audio})=>{
   const NativeAudio=window.Audio;window.Audio=class extends NativeAudio{constructor(src){super(src);window.player=this;}};
   window.requestFinished=false;window.receivedChunks=0;
   window.invoke=async(name,args)=>{
    if(name==='begin_speech_session')return 1;
    if(name!=='stream_speech')return;
    window.requestFinished=false;args.onAudio.onmessage({event:'started',data:{voice:'de-DE-ConradNeural',fallback:false}});
    const bytes=Uint8Array.from(atob(audio),c=>c.charCodeAt(0));
    for(let i=0;i<bytes.length;i+=768){args.onAudio.onmessage({event:'audio',data:btoa(String.fromCharCode(...bytes.subarray(i,i+768)))});window.receivedChunks++;await new Promise(r=>setTimeout(r,65));}
    args.onAudio.onmessage({event:'finished'});window.requestFinished=true;return {};
   };
  },{audio});
  await page.goto(`http://127.0.0.1:${server.address().port}`);await page.waitForFunction(()=>window.controller);await page.click('#play');
  await page.waitForFunction(()=>window.player?.currentTime>0.05&&!window.requestFinished);
  const result=await page.evaluate(()=>({playbackTime:window.player.currentTime,receivedChunks:window.receivedChunks,finished:window.requestFinished}));
  assert.equal(result.finished,false);assert(result.playbackTime>0);assert.equal(await page.evaluate(()=>window.controller.state().busy),false);
  await page.evaluate(()=>window.controller.stop());assert.equal(await page.evaluate(()=>window.player.paused),true);
  await page.waitForFunction(()=>window.requestFinished);assert.equal(await page.evaluate(()=>window.controller.state().active),null);assert.deepEqual(errors,[]);
  await page.click('#play');await page.waitForFunction(()=>window.requestFinished);await page.waitForFunction(()=>window.controller.state().active===null);assert.equal(await page.evaluate(()=>window.player.paused),true);assert.deepEqual(errors,[]);
  console.log('Real MP3 streaming passed:',JSON.stringify(result),'stop, late chunks and natural completion verified.');
 }finally{await browser?.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
