// Original geometric icon, rendered directly to PNG/ICO without external packages.
const fs=require('node:fs'),zlib=require('node:zlib');
fs.mkdirSync('assets',{recursive:true});
function crc32(b){let c=0xffffffff;for(const x of b){c^=x;for(let i=0;i<8;i++)c=(c>>>1)^((c&1)?0xedb88320:0);}return(c^0xffffffff)>>>0;}
function chunk(type,data){const t=Buffer.from(type),len=Buffer.alloc(4),crc=Buffer.alloc(4);len.writeUInt32BE(data.length);crc.writeUInt32BE(crc32(Buffer.concat([t,data])));return Buffer.concat([len,t,data,crc]);}
const n=256,raw=Buffer.alloc(n*(1+n*4));
for(let y=0;y<n;y++)for(let x=0;x<n;x++){const p=y*(1+n*4)+1+x*4;const rounded=(x>=36&&x<220)||(y>=36&&y<220)||((x<36?36:219)-x)**2+((y<36?36:219)-y)**2<36**2;const stem=x>62&&x<85&&y>64&&y<192;const ring=((x-114)/62)**2+((y-128)/64)**2<1&&((x-114)/39)**2+((y-128)/41)**2>1&&x>76;const dot=(x-197)**2+(y-186)**2<12**2;raw.set(dot?[153,211,200,255]:stem||ring?[245,249,252,255]:[15,43,70,rounded?255:0],p);}
const ihdr=Buffer.alloc(13);ihdr.writeUInt32BE(n);ihdr.writeUInt32BE(n,4);ihdr[8]=8;ihdr[9]=6;
const png=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',ihdr),chunk('IDAT',zlib.deflateSync(raw)),chunk('IEND',Buffer.alloc(0))]);fs.writeFileSync('assets/icon.png',png);
const header=Buffer.alloc(22);header.writeUInt16LE(1,2);header.writeUInt16LE(1,4);header.writeUInt16LE(1,10);header.writeUInt16LE(32,12);header.writeUInt32LE(png.length,14);header.writeUInt32LE(22,18);fs.writeFileSync('assets/icon.ico',Buffer.concat([header,png]));
