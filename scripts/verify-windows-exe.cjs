// Inspect the actual PE import table; do not infer single-file delivery from a build target.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const file = process.argv[2];
assert(file && process.argv.length === 3, 'Usage: node scripts/verify-windows-exe.cjs path/to/client.exe');
const bytes = fs.readFileSync(file);
assert(bytes.length >= 64 && bytes.toString('ascii', 0, 2) === 'MZ', 'Expected a Windows executable.');
const pe = bytes.readUInt32LE(0x3c);
assert.equal(bytes.toString('ascii', pe, pe + 4), 'PE\0\0');
assert.equal(bytes.readUInt16LE(pe + 4), 0x8664, 'Expected Windows x64.');
const optional = pe + 24;
assert.equal(bytes.readUInt16LE(optional), 0x20b, 'Expected PE32+.');
const sectionTable = optional + bytes.readUInt16LE(pe + 20);
const sections = Array.from({length: bytes.readUInt16LE(pe + 6)}, (_, i) => {
  const at = sectionTable + i * 40;
  return {address: bytes.readUInt32LE(at + 12), size: Math.max(bytes.readUInt32LE(at + 8), bytes.readUInt32LE(at + 16)), offset: bytes.readUInt32LE(at + 20)};
});
function offset(rva) {
  const section = sections.find(s => rva >= s.address && rva < s.address + s.size);
  assert(section, `Unmapped PE address: ${rva}`);
  const result = section.offset + rva - section.address;
  assert(result < bytes.length, 'PE address exceeds file size.');
  return result;
}
const imports = [];
const table = offset(bytes.readUInt32LE(optional + 120));
for (let i = 0; ; i++) {
  assert(i < 256, 'Unterminated PE import table.');
  const at = table + i * 20;
  if (bytes.subarray(at, at + 20).every(value => value === 0)) break;
  const name = offset(bytes.readUInt32LE(at + 12));
  const end = bytes.indexOf(0, name);
  assert(end >= name && end - name < 512, 'Invalid PE import name.');
  imports.push(bytes.toString('ascii', name, end));
}
const extraDlls = imports.filter(name => /^(webview2loader|pdfium|vcruntime\w*|msvcp\w*|libgcc.*|libstdc\+\+.*|libwinpthread.*)\.dll$/i.test(name));
assert.deepEqual(extraDlls, [], `The deliverable requires additional DLLs: ${extraDlls.join(', ')}`);
console.log(JSON.stringify({file: path.basename(file), bytes: bytes.length, imports, singleFileImports: true}, null, 2));
