// Run against a freshly built production CLI, outside Rust's cfg(test) overrides:
// APPARENT_BIN=/absolute/path/to/apparent node --experimental-strip-types --test connect/client/tests/encryption-default.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, isAbsolute } from 'node:path';
import { spawnSync } from 'node:child_process';

test('production Connect defaults to encryption without granting upload consent or rewriting config', () => {
  const binary = process.env.APPARENT_BIN;
  assert.ok(binary && isAbsolute(binary), 'APPARENT_BIN must name the freshly built production CLI');
  const directory = mkdtempSync(join(tmpdir(), 'encryption-default-'));
  const config = join(directory, 'config.json');
  const cache = join(directory, 'cache');
  mkdirSync(cache, { mode: 0o700 });
  function status() {
    const result = spawnSync(binary!, ['connect', 'status'], {
      encoding: 'utf8', timeout: 15_000,
      env: { ...process.env, APPARENT_CONFIG: config, XDG_CACHE_HOME: cache },
    });
    assert.equal(result.status, 0, result.stderr);
    const status = JSON.parse(result.stdout);
    assert.equal(status.enabled, false);
    assert.equal(status.uploadConsent, false);
    assert.equal(status.uploadSignedIn, false);
    assert.equal(status.readSignedIn, false);
    assert.deepEqual(status.queue, []);
    return status;
  }
  try {
    assert.equal(status().encryption, true);
    assert.equal(existsSync(config), false, 'readback must not create config');
    for (const enabled of [false, true]) {
      const content = JSON.stringify({ experiments: { hostedEncryption: enabled, unrelated: true }, privateUnknown: 'preserve' });
      writeFileSync(config, content, { mode: 0o600 });
      assert.equal(status().encryption, enabled);
      assert.equal(readFileSync(config, 'utf8'), content);
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
