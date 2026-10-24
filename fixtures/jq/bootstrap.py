#!/usr/bin/env python3
"""Fetch pinned archives, verify SHA-256 BEFORE extraction. No global installs."""
import pathlib, urllib.request, json, hashlib, subprocess, concurrent.futures
ROOT=pathlib.Path(__file__).resolve().parent
SPECS=[
 ('WebAssembly/wasi-sdk','wasi-sdk-31','wasi-sdk-31.0-x86_64-linux.tar.gz','wasi-sdk','e1cdf3f851ba090353c255627b1da997db57e0efc27631c5da55089c8bfb6a5d'),
 ('bytecodealliance/wit-bindgen','v0.61.0','wit-bindgen-0.61.0-x86_64-linux.tar.gz','wit-bindgen','51ad45fac01d2e412d06a0ca290e26d449fa06a62904a3a9878da180becc08a1'),
 ('bytecodealliance/wasm-tools','v1.245.0','wasm-tools-1.245.0-x86_64-linux.tar.gz','wasm-tools','811edaad5ca41b95fe747826a033bbf9c4bb636ad0e51ece0b69631dfd8b8f50'),
 ('jqlang/jq','jq-1.8.1','jq-1.8.1.tar.gz','jq','2be64e7129cecb11d5906290eba10af694fb9e3e7f9fc208a311dc33ca837eb0'),
 ('bytecodealliance/wasmtime','v50.0.0','wasi_snapshot_preview1.reactor.wasm',None,'73b01f85e42e99c890efbbf619cd765e04e5809da36d976b87fc64abd87f5812'),
]
def install(s):
 repo,tag,name,dest,expected=s
 url=f'https://github.com/{repo}/releases/download/{tag}/{name}'
 archive=ROOT/'downloads'/name
 if not archive.exists():
  temporary=archive.with_suffix(archive.suffix+'.partial')
  urllib.request.urlretrieve(url,temporary); temporary.rename(archive)
 digest=hashlib.sha256(archive.read_bytes()).hexdigest()
 if digest!=expected: raise RuntimeError(f'Hash mismatch: {name}: {digest}')
 if dest:
  target=ROOT/'tools'/dest; target.mkdir(exist_ok=True)
  subprocess.run(['tar','xf',str(archive),'--strip-components=1','-C',str(target)],check=True)
 else: (ROOT/'tools'/name).write_bytes(archive.read_bytes())
 print(name,digest,flush=True)
 return dict(repo=repo,tag=tag,asset=name,url=url,sha256=digest)
if __name__=='__main__':
 for d in ['downloads','tools','evidence','tmp','build']: (ROOT/d).mkdir(exist_ok=True)
 with concurrent.futures.ThreadPoolExecutor(4) as pool: results=list(pool.map(install,SPECS))
 (ROOT/'downloads.lock.json').write_text(json.dumps(results,indent=2)+'\n')
