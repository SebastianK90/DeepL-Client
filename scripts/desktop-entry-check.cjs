// Exercise the real desktop parser with isolated, harmless local executables.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const assert = require('node:assert/strict');
const {spawnSync} = require('node:child_process');

(async () => {
  if (process.platform !== 'linux') {
    console.log('Desktop entry checks apply to Linux only.');
    return;
  }
  for (const command of ['gio', 'desktop-file-validate']) {
    assert.equal(spawnSync('which', [command]).status, 0, `${command} is required`);
  }
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'deepl-desktop-check-'));
  const failures = [];
  try {
    const names = ['plain AppImage', 'quote"name', 'back\\slash', 'dollar$`name', 'percent%name', "Unicode-ä's;name"];
    for (const [index, name] of names.entries()) {
      const folder = path.join(root, String(index));
      fs.mkdirSync(folder);
      const executable = path.join(folder, name);
      const output = path.join(folder, 'argv.json');
      fs.writeFileSync(executable, '#!/usr/bin/env node\n' +
        `require('node:fs').writeFileSync(${JSON.stringify(output)}, JSON.stringify(process.argv.slice(1)));\n`, {mode: 0o755});
      const env = {...process.env, XDG_DATA_HOME: path.join(folder, 'data')};
      const setup = spawnSync(process.execPath, ['scripts/install-desktop-entry.cjs', executable], {env, encoding: 'utf8'});
      try {
        assert.equal(setup.status, 0, setup.stderr);
        const entry = path.join(env.XDG_DATA_HOME, 'applications/dev.local.deepl-client.desktop');
        const validation = spawnSync('desktop-file-validate', [entry], {encoding: 'utf8'});
        assert.equal(validation.status, 0, validation.stdout + validation.stderr);
        const launch = spawnSync('gio', ['launch', entry], {env, encoding: 'utf8', timeout: 5000});
        assert.equal(launch.status, 0, launch.stdout + launch.stderr);
        for (let attempt = 0; attempt < 100 && !fs.existsSync(output); attempt++) {
          await new Promise(resolve => setTimeout(resolve, 20));
        }
        assert(fs.existsSync(output), 'The intended executable must run.');
        assert.deepEqual(JSON.parse(fs.readFileSync(output, 'utf8')), [executable]);
        fs.writeFileSync(entry, '# User-owned desktop entry\n');
        const preserved = spawnSync(process.execPath, ['scripts/install-desktop-entry.cjs', executable], {env, encoding: 'utf8'});
        assert.notEqual(preserved.status, 0);
        assert.equal(fs.readFileSync(entry, 'utf8'), '# User-owned desktop entry\n');
      } catch (error) {
        failures.push(`${name}: ${error.message}`);
      }
    }
    for (const name of ['newline\nname', 'equal=name']) {
      const executable = path.join(root, name);
      fs.writeFileSync(executable, '', {mode: 0o755});
      const env = {...process.env, XDG_DATA_HOME: path.join(root, 'rejected')};
      const result = spawnSync(process.execPath, ['scripts/install-desktop-entry.cjs', executable], {env, encoding: 'utf8'});
      assert.notEqual(result.status, 0, 'Unsupported paths must fail clearly.');
      assert(!fs.existsSync(path.join(env.XDG_DATA_HOME, 'applications/dev.local.deepl-client.desktop')));
    }
    assert.equal(failures.length, 0, failures.join('\n'));
    console.log(`Desktop entry checks passed: ${names.length} real launches, exact paths/arguments, custom entries preserved.`);
  } finally {
    fs.rmSync(root, {recursive: true, force: true});
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
