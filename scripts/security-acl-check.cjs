// Verify every registered app command is explicitly enrolled in Tauri's ACL.
const fs = require('node:fs');
const assert = require('node:assert/strict');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const read = name => fs.readFileSync(path.join(root, name), 'utf8');
const source = read('src/ui/mod.rs');
const handler = source.match(/tauri::generate_handler!\[([\s\S]*?)\]/)[1];
const commands = handler.split(',').map(s => s.trim().split('::').pop()).filter(Boolean).sort();
assert.match(read('build.rs'), /\.app_manifest\(/, 'Custom commands must be enrolled in Tauri ACL');
const policy = read('src/command_policy.rs');
const declared = [...policy.matchAll(/^\s*\("([a-z_]+)",/gm)].map(m => m[1]).sort();
assert.deepEqual(declared, commands, 'Every handler must have exactly one policy entry');
assert.match(source, /command_policy::allows\(/, 'Runtime dispatch must enforce the window policy');
const capabilities = fs.readdirSync(path.join(root, 'capabilities')).filter(f => f.endsWith('.json')).map(f => JSON.parse(read('capabilities/' + f)));
const expected = {
 main: ['bootstrap','translate','write','remember_mode','save_settings','save_api_key','delete_api_key','toggle_favorite','clear_history','delete_history_entry','read_clipboard','copy_text','share_text','capture_screenshot','recognize_image','translate_document','cancel_document','save_document','third_party_licenses','main_window_action','begin_speech_session','cancel_speech','synthesize_speech','stream_speech','lookup_alternatives','apply_word_alternative','lookup_dictionary'],
 quick: ['bootstrap','translate','write','remember_mode','save_settings','begin_quick','replace_selection','copy_text','hide_quick','open_main','start_quick_drag','start_quick_resize','open_language_popup','close_language_popup'],
 'quick-language': ['language_popup_state','close_language_popup','choose_quick_language','resize_language_popup'],
 'capture-select': ['capture_area_state','capture_area_finish']
};
for (const cap of capabilities) {
 assert.equal(cap.remote, undefined, 'Remote origins must not receive capabilities');
 assert.equal(cap.local ?? true, true);
 assert(!cap.webviews, 'Use exact window identities');
 for (const label of cap.windows) assert(Object.hasOwn(expected,label), 'No wildcard/unrecognized windows');
}
for (const [label, allowed] of Object.entries(expected)) {
 const actual = capabilities.filter(c=>c.windows.includes(label)).flatMap(c=>c.permissions).filter(p=>!p.startsWith('core:'));
 assert.deepEqual([...new Set(actual)].sort(), allowed.map(c=>'allow-'+c.replaceAll('_','-')).sort(), label+' permissions');
}
for (const cap of capabilities) for (const permission of cap.permissions) {
 assert.equal(typeof permission,'string');
 assert(permission.startsWith('allow-') || ['core:event:allow-listen','core:event:allow-unlisten'].includes(permission), 'No broad default/plugin permissions');
}
console.log('ACL verified: '+commands.length+' commands; exact main/quick/language/capture permissions; no remote grants.');
