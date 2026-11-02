const fs=require('node:fs');
const path=require('node:path');
const assert=require('node:assert/strict');
function checkWorkflow(text) {
  for(const match of text.matchAll(/^\s*(?:-\s*)?uses:\s*["']?([^\s"'#]+).*$/gm)) {
    const ref=match[1];
    assert(/^[\w.-]+\/[\w./-]+@[0-9a-f]{40}$/.test(ref),`Action must use an immutable full commit: ${ref}`);
  }
  for(const match of text.matchAll(/^\s*(?:node-version|toolchain):\s*["']?([^\s"'#]+)/gm)) assert(/^\d+\.\d+\.\d+$/.test(match[1]),`Tool version must be exact: ${match[1]}`);
  for(const line of text.split('\n').filter(l=>/cargo install\s/.test(l))) assert(/--version\s+['"]?=?\d+\.\d+\.\d+['"]?(?:\s|$)/.test(line)&&/--locked(?:\s|$)/.test(line),`cargo install needs an exact version and --locked: ${line}`);
}
function main() {
 const dir=path.resolve(__dirname,'../.github/workflows');
 const files=fs.readdirSync(dir).filter(f=>/\.ya?ml$/.test(f));
 assert(files.length,'No workflows found');
 for(const file of files)checkWorkflow(fs.readFileSync(path.join(dir,file),'utf8'));
 console.log(`Supply chain guard passed for ${files.length} workflow files.`);
}
module.exports={checkWorkflow};
if(require.main===module)main();
