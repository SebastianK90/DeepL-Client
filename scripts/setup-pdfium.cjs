// Build-time only. Exact publisher archives are SHA-256 pinned; runtime never downloads code.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),zlib=require('node:zlib'),{execFileSync}=require('node:child_process');
(async()=>{
 const hostPlatform=process.platform==='win32'?'win':process.platform==='linux'?'linux':null;
 const args=process.argv.slice(2),options={},seen=new Set();
 while(args.length){const key=args.shift(),value=args.shift();if(!['--platform','--output'].includes(key)||!value||seen.has(key))throw Error('Usage: node scripts/setup-pdfium.cjs [--platform win|linux] [--output directory]');seen.add(key);options[key]=value;}
 const platform=options['--platform']||hostPlatform;
 if(!['win','linux'].includes(platform)||process.arch!=='x64')throw Error('PDFium setup supports Windows/Linux x64.');
 if(platform!==hostPlatform&&!options['--output'])throw Error('Cross-platform setup requires --output to keep host payloads separate.');
 const version='8125',name=`pdfium-${platform}-x64.tgz`,dir=path.resolve(options['--output']||'assets/pdfium');
 const hashes={win:'9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b',linux:'e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659'};
 const url=`https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/${version}/${name}`;
 fs.mkdirSync(dir,{recursive:true});
 const data=Buffer.from(await fetch(url).then(r=>{if(!r.ok)throw Error(r.status);return r.arrayBuffer()}));
 const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
 if(sha(data)!==hashes[platform])throw Error('PDFium archive checksum mismatch');
 const archive=path.join(dir,name);fs.writeFileSync(archive,data);
 const library=platform==='win'?'pdfium.dll':'libpdfium.so',relative=(platform==='win'?'bin/':'lib/')+library;
 execFileSync('tar',['-xzf',archive,'-C',dir,relative,'LICENSE','licenses']);fs.unlinkSync(archive);
 fs.copyFileSync(path.join(dir,relative),path.join(dir,library));
 const raw=fs.readFileSync(path.join(dir,library));
 const provenance={version,asset:url,sha256:hashes[platform],library_sha256:sha(raw),raw_bytes:raw.length};
 fs.writeFileSync(path.join(dir,'SOURCE.json'),JSON.stringify(provenance,null,2));
 const compress=b=>zlib.brotliCompressSync(b,{params:{[zlib.constants.BROTLI_PARAM_QUALITY]:11,[zlib.constants.BROTLI_PARAM_SIZE_HINT]:b.length}});
 const packed=compress(raw);fs.writeFileSync(path.join(dir,library+'.br'),packed);
 let notices='Embedded PDFium and modified msedge-tts notices\n\n'+JSON.stringify(provenance,null,2);
 const append=file=>{const data=fs.readFileSync(file);let text;try{text=new TextDecoder('utf-8',{fatal:true}).decode(data);}catch{text=new TextDecoder('windows-1252').decode(data);}notices+='\n\n=== '+path.basename(file)+' ===\n'+text;};
 append(path.join(dir,'LICENSE'));
 const walk=folder=>{for(const name of fs.readdirSync(folder).sort()){const file=path.join(folder,name);if(fs.statSync(file).isDirectory())walk(file);else append(file);}};
 walk(path.join(dir,'licenses'));append('vendor/msedge-tts/LICENSE');
 fs.writeFileSync(path.join(dir,'licenses.br'),compress(Buffer.from(notices)));
 console.log(`PDFium ${version} embedded payload prepared: ${raw.length} -> ${packed.length} bytes.`);
})().catch(e=>{console.error(e);process.exitCode=1;});

