import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync(new URL('../../../../.github/workflows/verification-platform.yml', import.meta.url), 'utf8');
const rust = workflow.slice(workflow.indexOf('\n  rust:'));

test('cold capture-client dependencies follow toolchain and cache before offline commands', () => {
  const toolchain = rust.indexOf('uses: dtolnay/rust-toolchain@stable');
  const cache = rust.indexOf('uses: Swatinem/rust-cache@v2');
  const fetch = rust.indexOf('cargo fetch --locked --manifest-path capture/agent/control-client/Cargo.toml');
  const tests = rust.indexOf('cargo test --locked --offline --manifest-path capture/agent/control-client/Cargo.toml');
  const build = rust.indexOf('cargo build --locked --offline --manifest-path capture/agent/control-client/Cargo.toml');
  assert(toolchain >= 0 && toolchain < cache && cache < fetch && fetch < tests && tests < build);
  assert.match(rust.slice(cache, fetch), /capture\/agent\/control-client -> target/);
  assert.match(rust.slice(fetch, tests), /inputs\.revision == github\.sha/);
});

test('capture-client packaging precedes attestation and exact-source upload', () => {
  const producer = rust.indexOf('node --experimental-strip-types capture/agent/control-client/delivery/package.ts pack');
  const attest = rust.indexOf('uses: actions/attest-build-provenance@v2', producer);
  const upload = rust.indexOf('uses: actions/upload-artifact@v4', attest);
  assert(producer >= 0 && producer < attest && attest < upload);
  assert(rust.slice(producer, attest).includes('"$GITHUB_SHA" "$target"'));
  assert(rust.slice(upload).includes('capture-client-${{ inputs.runner }}-${{ github.sha }}'));
});
