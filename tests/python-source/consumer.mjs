import assert from 'node:assert/strict';
import { api } from '@parity/python';

for (let i = 1; i <= 64; i++) {
  assert.equal(api.bump(), BigInt(i));
  assert.equal(api.greet('世界'), 'Hello, 世界!');
  assert.equal(api.total([20n, 22n]), 42n);
  assert.deepEqual(api.birthday({name: 'Ada', age: 41n}), {name: 'Ada', age: 42n});
  assert.equal(api.echoColor('dark-blue'), 'dark-blue');
  assert.deepEqual(api.echo(new Uint8Array([0, 128, 255])), new Uint8Array([0, 128, 255]));
  assert.equal(api.maybe(undefined), undefined);
  assert.equal(api.maybe('世界'), '世界');
  assert.equal(api.checked(21n), 42n);
  assert.throws(() => api.checked(-1n), error => error.payload === 'negative' || String(error).includes('negative'));
}
assert.equal(api.packageInfo(), '1.0:2026-01-02');
assert.equal(api.resource(), 'original resource');
assert.throws(() => api.explode());
console.log('python-installed-consumer-ok');
