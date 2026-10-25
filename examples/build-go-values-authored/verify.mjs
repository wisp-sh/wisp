import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

if (!process.argv[2]) throw new Error('usage: node verify.mjs /path/to/index.js');
const { api } = await import(pathToFileURL(resolve(process.argv[2])).href);

for (const labels of [[], [''], ['hello', '🦀 世界', '\0'], Array(128).fill('repeat')]) {
  assert.deepEqual(api.labels(labels), labels);
}
for (const mode of [undefined, 'fast', 'careful']) {
  assert.equal(api.selected(mode), mode);
}
for (const modes of [[], ['fast'], ['careful', 'fast', 'careful']]) {
  assert.deepEqual(api.modes(modes), modes);
}
for (const choices of [[], [undefined], ['fast', undefined, 'careful']]) {
  assert.deepEqual(api.choices(choices), choices);
}
const packet = {
  label: 'hello 🦀', data: new Uint8Array([0, 128, 255]),
  sequence: 9007199254740994n, enabled: true,
};
assert.deepEqual(api.packetValue(packet), packet);
assert.deepEqual(api.checked(true), packet);
assert.throws(() => api.checked(false), error => error.payload === 'denied' || String(error).includes('denied'));
assert.deepEqual(api.resultValue({ tag: 'ok', val: packet }), packet);
assert.throws(() => api.resultValue({ tag: 'err', val: 'denied' }));
for (let i = 0; i < 64; i++) {
  const batch = { labels: [`hello ${i}`, '世界'], selected: i % 2 ? 'careful' : undefined, modes: ['fast', 'careful'] };
  assert.deepEqual(api.batchValue(batch), batch);
}
console.log('go-upstream-values-ok');
