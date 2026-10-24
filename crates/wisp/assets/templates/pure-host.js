// Pure host v1. New resource classes and objects for every component instance.
// No preview2-shim imports, ambient process handles or global setters.
export function pureImports() {
  const denied = what => { throw new Error(`WISP_DENIED: ${what}`); };
  class IoError { toDebugString() { return 'pure host IO error'; } [Symbol.dispose]() {} }
  class InputStream {
    read(len) { throw {tag:'closed'}; }
    blockingRead(len) { return this.read(len); }
    [Symbol.dispose]() {}
  }
  class OutputStream {
    #allowance=0n;
    checkWrite() { this.#allowance=65536n; return this.#allowance; }
    write(bytes) { if(BigInt(bytes.length)>this.#allowance)denied('invalid sink write allowance');this.#allowance-=BigInt(bytes.length); }
    blockingWriteAndFlush(bytes) { /* Discard without retaining or touching host streams. */ }
    blockingFlush() {}
    [Symbol.dispose]() {}
  }
  class TerminalInput { [Symbol.dispose]() {} }
  class TerminalOutput { [Symbol.dispose]() {} }
  class Descriptor {
    readViaStream(){denied('filesystem');} writeViaStream(){denied('filesystem');}
    appendViaStream(){denied('filesystem');} getFlags(){denied('filesystem');}
    getType(){denied('filesystem');} stat(){denied('filesystem');}
    statAt(){denied('filesystem');} openAt(){denied('filesystem');}
    readlinkAt(){denied('filesystem');} metadataHash(){denied('filesystem');}
    metadataHashAt(){denied('filesystem');} [Symbol.dispose]() {}
  }
  return {
    'wasi:cli/environment':{getEnvironment:()=>[]},
    'wasi:cli/exit':{exit:()=>denied('guest exit')},
    'wasi:cli/stdin':{getStdin:()=>new InputStream()},
    'wasi:cli/stdout':{getStdout:()=>new OutputStream()},
    'wasi:cli/stderr':{getStderr:()=>new OutputStream()},
    'wasi:cli/terminal-input':{TerminalInput},
    'wasi:cli/terminal-output':{TerminalOutput},
    'wasi:cli/terminal-stdin':{getTerminalStdin:()=>undefined},
    'wasi:cli/terminal-stdout':{getTerminalStdout:()=>undefined},
    'wasi:cli/terminal-stderr':{getTerminalStderr:()=>undefined},
    'wasi:clocks/monotonic-clock':{now:()=>denied('monotonic clock')},
    'wasi:clocks/wall-clock':{now:()=>denied('wall clock')},
    'wasi:filesystem/preopens':{getDirectories:()=>[]},
    'wasi:filesystem/types':{Descriptor,filesystemErrorCode:()=>undefined},
    'wasi:io/error':{Error:IoError},
    'wasi:io/streams':{InputStream,OutputStream},
  };
}
