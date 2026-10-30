import assert from 'node:assert/strict';
import { api } from '@example/ocaml-values';
assert.equal(api.add(20,20),42);
assert.equal(api.wide(9007199254740993n),9007199254740994n);
assert.equal(api.maybe(undefined),undefined);
assert.equal(api.maybe('λ🌍'),'λ🌍!');
assert.deepEqual(api.points([{x:1.25,y:-2.5}]),[{x:1.75,y:-3}]);
assert.equal(api.safeDivide(84,2),42);
assert.throws(()=>api.safeDivide(1,0),e=>e.payload==='division by zero');
assert.equal(api.check(true),undefined);
assert.throws(()=>api.check(false),e=>e.payload==='false');
for(let i=1;i<=200;i++) {
  assert.equal(api.next(),i);
  const input={name:'λ\0🌍',payload:new Uint8Array([0,255,128,1]),mode:'loud'};
  assert.deepEqual(api.echo(input),{...input,name:input.name+'!'});
  api.collect();
}
assert.throws(()=>api.explode());
console.log('ocaml-installed-node-ok');
