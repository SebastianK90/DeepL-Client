// Scan every locked crate (including vendored backports) against current OSV data.
// No platform filter: Windows and Linux dependencies must both remain covered.
const fs = require('node:fs');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const root = path.resolve(__dirname, '..');
function parseLock(text) {
  const packages = text.split(/^\[\[package\]\]\s*$/m).slice(1).map(block => {
    const value = key => block.match(new RegExp(`^${key} = "([^"\\n]+)"`, 'm'))?.[1];
    return {name: value('name'), version: value('version')};
  });
  if (!packages.length || packages.some(p => !p.name || !p.version)) throw new Error('Invalid or empty Cargo.lock');
  return packages.filter(p => p.name !== 'deepl-client');
}
function assessFinding(pkg, advisory, exceptions, {today = new Date().toISOString().slice(0,10), backportVerified = false} = {}) {
  const ids = [advisory.id, ...(advisory.aliases || [])];
  if (backportVerified && pkg.name === 'glib' && pkg.version === '0.18.5' && ids.includes('RUSTSEC-2024-0429')) return 'backport';
  const affected = advisory.affected || [];
  if (affected.length && affected.every(a => a.database_specific?.informational === 'unmaintained') && exceptions.some(e =>
    e.name === pkg.name && e.version === pkg.version && e.advisory === advisory.id && e.kind === 'unmaintained' &&
    /^\d{4}-\d{2}-\d{2}$/.test(e.expires) && e.expires >= today && e.reason?.trim())) return 'maintenance';
  return 'block';
}
async function request(endpoint, body) {
  const response = await fetch(`https://api.osv.dev/v1/${endpoint}`, {
    method: body ? 'POST' : 'GET', headers: {'Content-Type':'application/json'},
    ...(body ? {body: JSON.stringify(body)} : {}), signal: AbortSignal.timeout(60000)
  });
  if (!response.ok) throw new Error(`OSV ${endpoint}: HTTP ${response.status}`);
  return response.json();
}
async function main() {
  // Fail closed before considering the sole security exception.
  execFileSync(process.execPath, [path.join(__dirname, 'check-glib-backport.cjs')], {cwd:root, stdio:'inherit'});
  const packages = parseLock(fs.readFileSync(path.join(root,'Cargo.lock'),'utf8'));
  const exceptions = JSON.parse(fs.readFileSync(path.join(root,'.github/security-maintenance-exceptions.json'),'utf8'));
  const findings = [];
  const details = new Map();
  for (let offset=0; offset<packages.length; offset+=100) {
    const batch=packages.slice(offset,offset+100);
    const result=await request('querybatch',{queries:batch.map(p=>({package:{name:p.name,ecosystem:'crates.io'},version:p.version}))});
    if (!Array.isArray(result.results) || result.results.length!==batch.length) throw new Error('Incomplete OSV batch response');
    for(let i=0;i<batch.length;i++) {
      if (result.results[i].next_page_token) throw new Error('Paginated OSV result requires scanner update; refusing incomplete scan');
      for(const summary of result.results[i].vulns || []) {
        if(!details.has(summary.id)) details.set(summary.id,await request(`vulns/${encodeURIComponent(summary.id)}`));
        const advisory=details.get(summary.id);
        if(advisory.withdrawn) continue;
        const status=assessFinding(batch[i],advisory,exceptions,{backportVerified:true});
        findings.push({package:batch[i],advisory:advisory.id,status,summary:advisory.summary});
      }
    }
  }
  console.log(JSON.stringify({scanned:packages.length,findings},null,2));
  if(findings.some(f=>f.status==='block')) throw new Error('Unreviewed dependency advisory; update dependency or review a narrowly scoped policy change');
}
module.exports={parseLock,assessFinding};
if(require.main===module) main().catch(error=>{console.error(error.message);process.exitCode=1;});
