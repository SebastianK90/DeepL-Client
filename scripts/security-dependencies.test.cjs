const {test}=require('node:test');
const assert=require('node:assert/strict');
let scanner;
try { scanner=require('./security-dependencies.cjs'); } catch (error) { if(error.code!=='MODULE_NOT_FOUND') throw error; }
test('dependency policy implementation exists',()=>assert.ok(scanner,'Missing dependency scanner'));
if(scanner) {
 const {parseLock,assessFinding}=scanner;
 const pkg={name:'example',version:'1.2.3'};
 const maintenance={id:'RUSTSEC-2026-0001',affected:[{database_specific:{informational:'unmaintained'}}]};
 const exception={...pkg,advisory:maintenance.id,kind:'unmaintained',expires:'2027-01-01',reason:'Tracked migration'};
 test('scans registry and local backports, excluding only the root application',()=>assert.deepEqual(parseLock('version = 4\n[[package]]\nname = "deepl-client"\nversion = "0.1.2"\n[[package]]\nname = "glib"\nversion = "0.18.5"\n[[package]]\nname = "example"\nversion = "1.2.3"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n'),[{name:'glib',version:'0.18.5'},{name:'example',version:'1.2.3'}]));
 test('fails on any unreviewed advisory',()=>assert.equal(assessFinding(pkg,maintenance,[],{today:'2026-11-02'}),'block'));
 test('permits only an exact, current maintenance exception',()=>{
  assert.equal(assessFinding(pkg,maintenance,[exception],{today:'2026-11-02'}),'maintenance');
  for(const change of [{version:'1.2.4'},{name:'other'},{advisory:'other'},{expires:'2026-11-01'}]) assert.equal(assessFinding(pkg,maintenance,[{...exception,...change}],{today:'2026-11-02'}),'block');
 });
 test('maintenance exception cannot hide a security vulnerability',()=>assert.equal(assessFinding(pkg,{...maintenance,affected:[]},[exception],{today:'2026-11-02'}),'block'));
 test('GLib advisory requires verified backport and exact version',()=>{
  const advisory={id:'GHSA-wrw7-89jp-8q8g',aliases:['RUSTSEC-2024-0429']};
  assert.equal(assessFinding({name:'glib',version:'0.18.5'},advisory,[],{backportVerified:true}),'backport');
  assert.equal(assessFinding({name:'glib',version:'0.18.5'},advisory,[],{}),'block');
  assert.equal(assessFinding({name:'glib',version:'0.18.4'},advisory,[],{backportVerified:true}),'block');
 });
}
