const {test}=require('node:test');
const assert=require('node:assert/strict');
let policy;
try {policy=require('./security-supply-chain.cjs');} catch(error) {if(error.code!=='MODULE_NOT_FOUND')throw error;}
test('supply chain guard exists',()=>assert.ok(policy,'Missing supply chain guard'));
if(policy){
 test('rejects floating actions and reusable workflows',()=>{
  for(const text of ['- uses: actions/checkout@v4','uses: owner/repo/.github/workflows/build.yml@main']) assert.throws(()=>policy.checkWorkflow(text),/immutable/);
 });
 test('accepts full commit refs',()=>assert.doesNotThrow(()=>policy.checkWorkflow('- uses: actions/checkout@'+'a'.repeat(40))));
 test('rejects floating compiler, Node and Tauri versions',()=>{
  for(const text of ['node-version: 22','toolchain: stable',"run: cargo install tauri-cli --version '^2' --locked"]) assert.throws(()=>policy.checkWorkflow(text),/exact/);
 });
}
