// Verify the fixed source and Cargo's actual resolution, not just a version label.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict'),{execFileSync}=require('node:child_process');
const root=path.resolve(__dirname,'..'),vendor=path.join(root,'vendor/glib');
const metadata=JSON.parse(fs.readFileSync(path.join(vendor,'BACKPORT.json'),'utf8'));
assert.equal(metadata.registry_archive_sha256,'233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5');
assert.equal(metadata.upstream_merge_commit,'05dff0ee696f9bcd8617cd48c4b812d046d440cb');
assert.equal(metadata.advisory,'RUSTSEC-2024-0429');
const digest=crypto.createHash('sha256').update(fs.readFileSync(path.join(vendor,'src/variant_iter.rs'))).digest('hex');
assert.equal(digest,'a0f5ee8acb8faa089bcdfbc9a57372609fce7654026ccef7d9a224d05a654ccc','The reviewed GLib backport source changed');
assert.equal(digest,metadata.patched_file_sha256);
const cargo=JSON.parse(execFileSync('cargo',['metadata','--locked','--format-version','1'],{cwd:root,encoding:'utf8',maxBuffer:32*1024*1024}));
const glib=cargo.packages.filter(p=>p.name==='glib');
assert.equal(glib.length,1,'Review additional GLib versions instead of leaving an unpatched copy');
assert.equal(glib[0].version,'0.18.5');assert.equal(glib[0].source,null,'Cargo must use the patched local source');
const canonical=p=>{const value=fs.realpathSync(p);return process.platform==='win32'?value.toLowerCase():value;};
assert.equal(canonical(glib[0].manifest_path),canonical(path.join(vendor,'Cargo.toml')));
assert(!cargo.workspace_members.includes(glib[0].id),'The vendored dependency must not become a workspace test target');
assert(cargo.resolve.nodes.some(node=>node.id===glib[0].id),'The patched crate must be in the resolved dependency graph');
console.log('GLib backport verified: reviewed upstream fix, unchanged 0.18.5 API and one patched source for the dependency graph.');
