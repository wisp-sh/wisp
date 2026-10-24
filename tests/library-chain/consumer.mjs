import assert from 'node:assert/strict';
import { arithmetic } from '@wisp/library-chain';

assert.equal(arithmetic.total({ quantity: 3, unitPrice: 7n }), 21n);
console.log('ok(21)');
assert.throws(
  () => arithmetic.total({ quantity: 2, unitPrice: 18446744073709551615n }),
  error => error.payload === 'overflow',
);
console.log('err(overflow)');
assert.equal(arithmetic.total({ quantity: 0, unitPrice: 18446744073709551615n }), 0n);
console.log('ok(0)');
