// Pixel regression: square Quick Assist corners must be completely opaque, in both themes and at 150% DPI.
const {chromium}=require('playwright'),fs=require('node:fs'),http=require('node:http'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const root=path.resolve('frontend'),server=http.createServer((req,res)=>{const file=path.resolve(root,'.'+new URL(req.url,'http://localhost').pathname.replace(/\/$/,'/index.html'));if(!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}fs.readFile(file,(e,b)=>{if(e){res.writeHead(404).end();return;}res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(b);});});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'msedge'}:{})});
 for(const scale of [1,1.5]){const context=await browser.newContext({viewport:{width:492,height:292},deviceScaleFactor:scale});const page=await context.newPage();await page.goto(`http://127.0.0.1:${server.address().port}/?overlay=1`);await page.waitForSelector('body.overlay');
 for(const theme of ['light','dark']){await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;document.getElementById('quick-output').textContent='Eckiger Quick Assist ohne ausgesparte Ecken.';},theme);const shot=await page.screenshot({omitBackground:true});const pixels=await page.evaluate(async b64=>{const img=new Image();img.src='data:image/png;base64,'+b64;await img.decode();const canvas=document.createElement('canvas');canvas.width=img.width;canvas.height=img.height;const ctx=canvas.getContext('2d');ctx.drawImage(img,0,0);return [[0,0],[img.width-1,0],[0,img.height-1],[img.width-1,img.height-1],[img.width/2|0,img.height/2|0]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data]);},shot.toString('base64'));
 for(const pixel of pixels.slice(0,4))assert.equal(pixel[3],255,`${theme} at ${scale}: transparent corner ${pixel}`);assert.equal(pixels[4][3],255,'Panel itself stays opaque');fs.mkdirSync('docs/screenshots',{recursive:true});fs.writeFileSync(`docs/screenshots/quick-corners-${theme}-${scale}.png`,shot);
 }await context.close();}
 console.log('Quick Assist corner pixels passed: light/dark, 100%/150% DPI, four opaque square corners and opaque panel.');
 }finally{await browser?.close();server.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});

