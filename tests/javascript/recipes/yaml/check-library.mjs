import assert from 'node:assert/strict';
import { parseYaml, parseDocuments, stringifyJson } from './library/node/index.js';
const unpack = s => JSON.parse(s);
assert.deepEqual(unpack(parseYaml('base: &x {name: 世界}\ncopy: *x\n')).value,
 {base:{name:'世界'},copy:{name:'世界'}});
assert.deepEqual(unpack(parseDocuments('---\na: 1\n---\n- 二\n')).value,[{a:1},['二']]);
assert.equal(unpack(parseYaml('a: [')).ok,false);
assert.equal(unpack(parseYaml('a: 1\na: 2')).ok,false);
assert.equal(unpack(parseDocuments('---\na: 1\n---\na: [')).ok,false);
assert.equal(unpack(parseYaml('a: &a [*a]')).ok,false);
assert.deepEqual(unpack(parseYaml('yes: on\nnumber: 0x10')).value,{yes:'on',number:16});
assert.equal(unpack(stringifyJson('x')).ok,false);
for(let i=0;i<100;i++) assert.deepEqual(unpack(parseYaml(`i: ${i}\ntext: 🦀`)).value,{i,text:'🦀'});
assert.equal(unpack(parseYaml(unpack(stringifyJson('{"a":[1,true,"世界"]}')).value)).value.a[2],'世界');
console.log('yaml: aliases, multi-document, errors, Unicode, round trip, 100 repeated calls passed');
