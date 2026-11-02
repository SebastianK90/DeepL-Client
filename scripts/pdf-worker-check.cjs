// Keep the UI document route out of the in-process native PDF parser.
const fs=require('node:fs'),assert=require('node:assert/strict'),path=require('node:path');
const root=path.resolve(__dirname,'..'),read=f=>fs.readFileSync(path.join(root,f),'utf8');
assert.match(read('src/document.rs'),/"pdf"\s*=>\s*pdf_worker::translate\(/,'PDF document imports must use the child broker');
assert.match(read('src/main.rs'),/"--pdf-worker"/,'The child entry point must run before UI startup');
assert(read('src/main.rs').indexOf('"--pdf-worker"')<read('src/main.rs').indexOf('ui::run()'));
assert.match(read('src/document/pdf_worker.rs'),/env_clear\(/,'Do not inherit API credentials/environment secrets');
console.log('PDF worker route verified: subprocess broker, early CLI entry, cleared environment.');
