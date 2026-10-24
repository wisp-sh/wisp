// Synchronous library reactor host. No inherited filesystem, process streams,
// arguments, environment, or network. Clocks and cryptographic randomness are real.
import { randomBytes } from 'node:crypto';
import { hrtime } from 'node:process';
const denied = () => { throw new Error('WISP_DENIED: reactor filesystem'); };
class Pollable {
  constructor(deadline = 0n) { this.deadline = deadline; }
  ready() { return hrtime.bigint() >= this.deadline; }
  block() {
    const wait = new Int32Array(new SharedArrayBuffer(4));
    while (!this.ready()) Atomics.wait(wait, 0, 0, Math.min(2147483647, Number((this.deadline - hrtime.bigint() + 999999n) / 1000000n)));
  }
  [Symbol.dispose]() {}
}
class IoError {
  toDebugString() { return 'reactor IO error'; }
  [Symbol.dispose]() {}
}
class InputStream {
  read() { throw { tag: 'closed' }; }
  blockingRead(len) { return this.read(len); }
  skip() { throw { tag: 'closed' }; }
  blockingSkip(len) { return this.skip(len); }
  subscribe() { return new Pollable(); }
  [Symbol.dispose]() {}
}
class OutputStream {
  allowance = 0n;
  checkWrite() { this.allowance = 65536n; return this.allowance; }
  write(bytes) {
    if (BigInt(bytes.length) > this.allowance) throw new Error('invalid reactor sink write allowance');
    this.allowance -= BigInt(bytes.length);
  }
  writeZeroes(len) {
    if (len > this.allowance) throw new Error('invalid reactor sink write allowance');
    this.allowance -= len;
  }
  flush() {}
  blockingFlush() {}
  blockingWriteAndFlush() {}
  blockingWriteZeroesAndFlush() {}
  splice(input, len) { input.read(len); return 0n; }
  blockingSplice(input, len) { return this.splice(input, len); }
  subscribe() { return new Pollable(); }
  [Symbol.dispose]() {}
}
// No descriptor is ever granted by this host. All filesystem operations require
// a descriptor capability, so none can reach a host path.
class Descriptor {
  readViaStream() { denied(); } writeViaStream() { denied(); }
  appendViaStream() { denied(); } advise() { denied(); } syncData() { denied(); }
  getFlags() { denied(); } getType() { denied(); } setSize() { denied(); }
  setTimes() { denied(); } read() { denied(); } write() { denied(); }
  readDirectory() { denied(); } sync() { denied(); } createDirectoryAt() { denied(); }
  stat() { denied(); } statAt() { denied(); } setTimesAt() { denied(); }
  linkAt() { denied(); } openAt() { denied(); } readlinkAt() { denied(); }
  removeDirectoryAt() { denied(); } renameAt() { denied(); }
  symlinkAt() { denied(); } unlinkFileAt() { denied(); } isSameObject() { denied(); }
  metadataHash() { denied(); } metadataHashAt() { denied(); }
  [Symbol.dispose]() {}
}
class DirectoryEntryStream { readDirectoryEntry() { denied(); } [Symbol.dispose]() {} }
export const environment = { getEnvironment: () => [], getArguments: () => [], initialCwd: () => undefined };
export const exit = {
  exit() { throw new Error('WISP_DENIED: reactor guest exit'); },
  exitWithCode() { throw new Error('WISP_DENIED: reactor guest exit'); },
};
export const stdin = { getStdin: () => new InputStream() };
export const stdout = { getStdout: () => new OutputStream() };
export const stderr = { getStderr: () => new OutputStream() };
export const error = { Error: IoError };
export const streams = { InputStream, OutputStream };
export const poll = {
  Pollable,
  poll(values) {
    if (!values.length) throw new Error('empty reactor poll list');
    let ready = values.flatMap((value, index) => value.ready() ? [index] : []);
    if (!ready.length) {
      values.reduce((a, b) => a.deadline <= b.deadline ? a : b).block();
      ready = values.flatMap((value, index) => value.ready() ? [index] : []);
    }
    return new Uint32Array(ready);
  },
};
export const monotonicClock = {
  now: () => hrtime.bigint(), resolution: () => 1n,
  subscribeInstant: instant => new Pollable(instant),
  subscribeDuration: duration => new Pollable(hrtime.bigint() + duration),
};
export const wallClock = {
  now() { const now = Date.now(); return { seconds: BigInt(Math.floor(now / 1000)), nanoseconds: (now % 1000) * 1000000 }; },
  resolution: () => ({ seconds: 0n, nanoseconds: 1000000 }),
};
export const random = {
  getRandomBytes: len => randomBytes(Number(len)),
  getRandomU64: () => randomBytes(8).readBigUInt64LE(),
};
export const insecureSeed = {
  insecureSeed() {
    const seed = randomBytes(16);
    return [seed.readBigUInt64LE(0), seed.readBigUInt64LE(8)];
  },
};
export const preopens = { getDirectories: () => [] };
export const types = { Descriptor, DirectoryEntryStream, filesystemErrorCode: () => undefined };

class TerminalInput { [Symbol.dispose]() {} }
class TerminalOutput { [Symbol.dispose]() {} }
export const terminalInput = { TerminalInput };
export const terminalOutput = { TerminalOutput };
export const terminalStdin = { getTerminalStdin: () => undefined };
export const terminalStdout = { getTerminalStdout: () => undefined };
export const terminalStderr = { getTerminalStderr: () => undefined };

// CPython retains WASI sockets imports even when the application has no socket
// API. Never create a host socket or resolver in the reactor profile. Result
// functions report the standard error code, so guests can handle denial.
const networkDenied = () => { throw 'access-denied'; };
class Network { [Symbol.dispose]() {} }
function unavailableSocketResource(methods) {
  class Resource { [Symbol.dispose]() {} }
  for (const method of methods.split(' '))
    Object.defineProperty(Resource.prototype, method, { value: networkDenied });
  return Resource;
}
const TcpSocket = unavailableSocketResource(
  'startBind finishBind startConnect finishConnect startListen finishListen accept '
  + 'localAddress remoteAddress isListening setListenBacklogSize keepAliveEnabled '
  + 'setKeepAliveEnabled keepAliveIdleTime setKeepAliveIdleTime keepAliveInterval '
  + 'setKeepAliveInterval keepAliveCount setKeepAliveCount hopLimit setHopLimit '
  + 'receiveBufferSize setReceiveBufferSize sendBufferSize setSendBufferSize subscribe shutdown');
const UdpSocket = unavailableSocketResource(
  'startBind finishBind stream localAddress remoteAddress unicastHopLimit '
  + 'setUnicastHopLimit receiveBufferSize setReceiveBufferSize sendBufferSize '
  + 'setSendBufferSize subscribe');
const IncomingDatagramStream = unavailableSocketResource('receive subscribe');
const OutgoingDatagramStream = unavailableSocketResource('checkSend send subscribe');
const ResolveAddressStream = unavailableSocketResource('resolveNextAddress subscribe');
export const network = { Network };
export const instanceNetwork = { instanceNetwork: () => new Network() };
export const tcp = { TcpSocket };
export const tcpCreateSocket = { createTcpSocket: networkDenied };
export const udp = { UdpSocket, IncomingDatagramStream, OutgoingDatagramStream };
export const udpCreateSocket = { createUdpSocket: networkDenied };
export const ipNameLookup = { ResolveAddressStream, resolveAddresses: networkDenied };
