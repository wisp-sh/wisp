// Modified by Wisp: disable optional filesystem advice so this network-only
// host has no undeclared native @bytecodealliance/jco-node-fs dependency.
import fs, { createReadStream, createWriteStream, read } from "node:fs";
import process, { hrtime, nextTick, stderr, stdout } from "node:process";
import { PassThrough, Readable } from "node:stream";
import { parentPort, workerData } from "node:worker_threads";
import { Agent, createServer, request } from "node:http";
import { Agent as Agent$1, request as request$1 } from "node:https";
import { promisify } from "node:util";
import { Server, Socket, isIP } from "node:net";
import { lookup } from "node:dns/promises";
import { createSocket } from "node:dgram";
import { ALL, BADFAMILY, CANCELLED, CONNREFUSED, NODATA, NOMEM, NONAME, NOTFOUND, REFUSED, SERVFAIL, TIMEOUT, V4MAPPED } from "node:dns";
import { EACCES, EADDRINUSE, EADDRNOTAVAIL, EALREADY, EBADF, ECONNABORTED, ECONNREFUSED, ECONNRESET, EHOSTUNREACH, EINVAL, EMFILE, EMSGSIZE, ENETDOWN, ENETUNREACH, ENFILE, ENOBUFS, ENOMEM, ENOTCONN, ENOTSUP, EPERM, EWOULDBLOCK } from "node:constants";
//#region \0rolldown/runtime.js
var __defProp = Object.defineProperty;
var __exportAll = (all, no_symbols) => {
	let target = {};
	for (var name in all) __defProp(target, name, {
		get: all[name],
		enumerable: true
	});
	if (!no_symbols) __defProp(target, Symbol.toStringTag, { value: "Module" });
	return target;
};
//#endregion
//#region dist/synckit/index.js
function extractProperties(object) {
	if (object && typeof object === "object") {
		const properties = {};
		for (const key in object) properties[key] = object[key];
		return properties;
	}
}
function runAsWorker(fn) {
	if (!workerData) return;
	const { workerPort, debug } = workerData;
	try {
		parentPort?.on("message", ({ sharedBuffer, cid, args }) => {
			(async () => {
				const sharedBufferView = new Int32Array(sharedBuffer);
				let msg;
				try {
					msg = {
						cid,
						result: await fn(...args)
					};
				} catch (error) {
					msg = {
						cid,
						error,
						properties: extractProperties(error)
					};
				}
				workerPort.postMessage(msg);
				Atomics.add(sharedBufferView, 0, 1);
				Atomics.notify(sharedBufferView, 0);
			})();
		});
	} catch (error) {
		parentPort?.on("message", ({ sharedBuffer, cid }) => {
			const sharedBufferView = new Int32Array(sharedBuffer);
			workerPort.postMessage({
				cid,
				error,
				properties: extractProperties(error)
			});
			Atomics.add(sharedBufferView, 0, 1);
			Atomics.notify(sharedBufferView, 0);
		});
	}
	return debug;
}
//#endregion
//#region dist/io/calls.js
var calls_exports = /* @__PURE__ */ __exportAll({
	CALL_MASK: () => CALL_MASK,
	CALL_SHIFT: () => 24,
	CALL_TYPE_MASK: () => CALL_TYPE_MASK,
	CLOCKS: () => CLOCKS,
	CLOCKS_DURATION_SUBSCRIBE: () => CLOCKS_DURATION_SUBSCRIBE,
	CLOCKS_INSTANT_SUBSCRIBE: () => CLOCKS_INSTANT_SUBSCRIBE,
	FILE: () => FILE,
	FILESYSTEM_DESCRIPTOR_ADVISE: () => FILESYSTEM_DESCRIPTOR_ADVISE,
	FILESYSTEM_DESCRIPTOR_CLOSE: () => FILESYSTEM_DESCRIPTOR_CLOSE,
	FILESYSTEM_DESCRIPTOR_GET_TYPE: () => FILESYSTEM_DESCRIPTOR_GET_TYPE,
	FILESYSTEM_DESCRIPTOR_METADATA_HASH: () => FILESYSTEM_DESCRIPTOR_METADATA_HASH,
	FILESYSTEM_DESCRIPTOR_OPEN: () => FILESYSTEM_DESCRIPTOR_OPEN,
	FILESYSTEM_DESCRIPTOR_READ: () => FILESYSTEM_DESCRIPTOR_READ,
	FILESYSTEM_DESCRIPTOR_SET_SIZE: () => FILESYSTEM_DESCRIPTOR_SET_SIZE,
	FILESYSTEM_DESCRIPTOR_SET_TIMES: () => FILESYSTEM_DESCRIPTOR_SET_TIMES,
	FILESYSTEM_DESCRIPTOR_STAT: () => FILESYSTEM_DESCRIPTOR_STAT,
	FILESYSTEM_DESCRIPTOR_SYNC: () => FILESYSTEM_DESCRIPTOR_SYNC,
	FILESYSTEM_DESCRIPTOR_SYNC_DATA: () => FILESYSTEM_DESCRIPTOR_SYNC_DATA,
	FILESYSTEM_DESCRIPTOR_WRITE: () => FILESYSTEM_DESCRIPTOR_WRITE,
	FUTURE_DISPOSE: () => FUTURE_DISPOSE,
	FUTURE_SUBSCRIBE: () => FUTURE_SUBSCRIBE,
	FUTURE_TAKE_VALUE: () => FUTURE_TAKE_VALUE,
	HTTP: () => HTTP,
	HTTP_CREATE_REQUEST: () => HTTP_CREATE_REQUEST,
	HTTP_OUTGOING_BODY_DISPOSE: () => HTTP_OUTGOING_BODY_DISPOSE,
	HTTP_OUTPUT_STREAM_FINISH: () => HTTP_OUTPUT_STREAM_FINISH,
	HTTP_SERVER_CLEAR_OUTGOING_RESPONSE: () => HTTP_SERVER_CLEAR_OUTGOING_RESPONSE,
	HTTP_SERVER_GET_ADDRESS: () => HTTP_SERVER_GET_ADDRESS,
	HTTP_SERVER_INCOMING_HANDLER: () => HTTP_SERVER_INCOMING_HANDLER,
	HTTP_SERVER_SET_OUTGOING_RESPONSE: () => HTTP_SERVER_SET_OUTGOING_RESPONSE,
	HTTP_SERVER_START: () => HTTP_SERVER_START,
	HTTP_SERVER_STOP: () => HTTP_SERVER_STOP,
	INPUT_STREAM_BLOCKING_READ: () => INPUT_STREAM_BLOCKING_READ,
	INPUT_STREAM_BLOCKING_SKIP: () => INPUT_STREAM_BLOCKING_SKIP,
	INPUT_STREAM_CREATE: () => INPUT_STREAM_CREATE,
	INPUT_STREAM_DISPOSE: () => INPUT_STREAM_DISPOSE,
	INPUT_STREAM_READ: () => INPUT_STREAM_READ,
	INPUT_STREAM_SKIP: () => INPUT_STREAM_SKIP,
	INPUT_STREAM_SUBSCRIBE: () => INPUT_STREAM_SUBSCRIBE,
	OUTPUT_STREAM_BLOCKING_FLUSH: () => OUTPUT_STREAM_BLOCKING_FLUSH,
	OUTPUT_STREAM_BLOCKING_SPLICE: () => OUTPUT_STREAM_BLOCKING_SPLICE,
	OUTPUT_STREAM_BLOCKING_WRITE_AND_FLUSH: () => OUTPUT_STREAM_BLOCKING_WRITE_AND_FLUSH,
	OUTPUT_STREAM_BLOCKING_WRITE_ZEROES_AND_FLUSH: () => OUTPUT_STREAM_BLOCKING_WRITE_ZEROES_AND_FLUSH,
	OUTPUT_STREAM_CHECK_WRITE: () => OUTPUT_STREAM_CHECK_WRITE,
	OUTPUT_STREAM_CREATE: () => OUTPUT_STREAM_CREATE,
	OUTPUT_STREAM_DISPOSE: () => OUTPUT_STREAM_DISPOSE,
	OUTPUT_STREAM_FLUSH: () => OUTPUT_STREAM_FLUSH,
	OUTPUT_STREAM_GET_TOTAL_BYTES: () => OUTPUT_STREAM_GET_TOTAL_BYTES,
	OUTPUT_STREAM_SPLICE: () => OUTPUT_STREAM_SPLICE,
	OUTPUT_STREAM_SUBSCRIBE: () => OUTPUT_STREAM_SUBSCRIBE,
	OUTPUT_STREAM_WRITE: () => OUTPUT_STREAM_WRITE,
	OUTPUT_STREAM_WRITE_ZEROES: () => OUTPUT_STREAM_WRITE_ZEROES,
	POLL_POLLABLE_BLOCK: () => POLL_POLLABLE_BLOCK,
	POLL_POLLABLE_DISPOSE: () => POLL_POLLABLE_DISPOSE,
	POLL_POLLABLE_READY: () => POLL_POLLABLE_READY,
	POLL_POLL_LIST: () => POLL_POLL_LIST,
	SOCKET_DATAGRAM_STREAM_DISPOSE: () => SOCKET_DATAGRAM_STREAM_DISPOSE,
	SOCKET_DATAGRAM_STREAM_SUBSCRIBE: () => SOCKET_DATAGRAM_STREAM_SUBSCRIBE,
	SOCKET_GET_DEFAULT_RECEIVE_BUFFER_SIZE: () => SOCKET_GET_DEFAULT_RECEIVE_BUFFER_SIZE,
	SOCKET_GET_DEFAULT_SEND_BUFFER_SIZE: () => SOCKET_GET_DEFAULT_SEND_BUFFER_SIZE,
	SOCKET_INCOMING_DATAGRAM_STREAM_RECEIVE: () => SOCKET_INCOMING_DATAGRAM_STREAM_RECEIVE,
	SOCKET_OUTGOING_DATAGRAM_STREAM_CHECK_SEND: () => SOCKET_OUTGOING_DATAGRAM_STREAM_CHECK_SEND,
	SOCKET_OUTGOING_DATAGRAM_STREAM_SEND: () => SOCKET_OUTGOING_DATAGRAM_STREAM_SEND,
	SOCKET_RESOLVE_ADDRESS_CREATE_REQUEST: () => SOCKET_RESOLVE_ADDRESS_CREATE_REQUEST,
	SOCKET_RESOLVE_ADDRESS_DISPOSE_REQUEST: () => SOCKET_RESOLVE_ADDRESS_DISPOSE_REQUEST,
	SOCKET_RESOLVE_ADDRESS_SUBSCRIBE_REQUEST: () => SOCKET_RESOLVE_ADDRESS_SUBSCRIBE_REQUEST,
	SOCKET_RESOLVE_ADDRESS_TAKE_REQUEST: () => SOCKET_RESOLVE_ADDRESS_TAKE_REQUEST,
	SOCKET_TCP: () => SOCKET_TCP,
	SOCKET_TCP_ACCEPT: () => SOCKET_TCP_ACCEPT,
	SOCKET_TCP_BIND_FINISH: () => SOCKET_TCP_BIND_FINISH,
	SOCKET_TCP_BIND_START: () => SOCKET_TCP_BIND_START,
	SOCKET_TCP_CONNECT_FINISH: () => SOCKET_TCP_CONNECT_FINISH,
	SOCKET_TCP_CONNECT_START: () => SOCKET_TCP_CONNECT_START,
	SOCKET_TCP_CREATE_HANDLE: () => SOCKET_TCP_CREATE_HANDLE,
	SOCKET_TCP_DISPOSE: () => SOCKET_TCP_DISPOSE,
	SOCKET_TCP_GET_LOCAL_ADDRESS: () => SOCKET_TCP_GET_LOCAL_ADDRESS,
	SOCKET_TCP_GET_REMOTE_ADDRESS: () => SOCKET_TCP_GET_REMOTE_ADDRESS,
	SOCKET_TCP_IS_LISTENING: () => SOCKET_TCP_IS_LISTENING,
	SOCKET_TCP_LISTEN_FINISH: () => SOCKET_TCP_LISTEN_FINISH,
	SOCKET_TCP_LISTEN_START: () => SOCKET_TCP_LISTEN_START,
	SOCKET_TCP_SET_KEEP_ALIVE: () => SOCKET_TCP_SET_KEEP_ALIVE,
	SOCKET_TCP_SET_LISTEN_BACKLOG_SIZE: () => SOCKET_TCP_SET_LISTEN_BACKLOG_SIZE,
	SOCKET_TCP_SHUTDOWN: () => SOCKET_TCP_SHUTDOWN,
	SOCKET_TCP_SUBSCRIBE: () => SOCKET_TCP_SUBSCRIBE,
	SOCKET_UDP: () => SOCKET_UDP,
	SOCKET_UDP_BIND_FINISH: () => SOCKET_UDP_BIND_FINISH,
	SOCKET_UDP_BIND_START: () => SOCKET_UDP_BIND_START,
	SOCKET_UDP_CREATE_HANDLE: () => SOCKET_UDP_CREATE_HANDLE,
	SOCKET_UDP_DISPOSE: () => SOCKET_UDP_DISPOSE,
	SOCKET_UDP_GET_LOCAL_ADDRESS: () => SOCKET_UDP_GET_LOCAL_ADDRESS,
	SOCKET_UDP_GET_RECEIVE_BUFFER_SIZE: () => SOCKET_UDP_GET_RECEIVE_BUFFER_SIZE,
	SOCKET_UDP_GET_REMOTE_ADDRESS: () => SOCKET_UDP_GET_REMOTE_ADDRESS,
	SOCKET_UDP_GET_SEND_BUFFER_SIZE: () => SOCKET_UDP_GET_SEND_BUFFER_SIZE,
	SOCKET_UDP_GET_UNICAST_HOP_LIMIT: () => SOCKET_UDP_GET_UNICAST_HOP_LIMIT,
	SOCKET_UDP_SET_RECEIVE_BUFFER_SIZE: () => SOCKET_UDP_SET_RECEIVE_BUFFER_SIZE,
	SOCKET_UDP_SET_SEND_BUFFER_SIZE: () => SOCKET_UDP_SET_SEND_BUFFER_SIZE,
	SOCKET_UDP_SET_UNICAST_HOP_LIMIT: () => SOCKET_UDP_SET_UNICAST_HOP_LIMIT,
	SOCKET_UDP_STREAM: () => SOCKET_UDP_STREAM,
	SOCKET_UDP_SUBSCRIBE: () => SOCKET_UDP_SUBSCRIBE,
	STDERR: () => STDERR,
	STDIN: () => STDIN,
	STDOUT: () => STDOUT,
	reverseMap: () => reverseMap
});
let call_id = 0;
const CALL_MASK = 4278190080;
const CALL_TYPE_MASK = 16777215;
let cnt = 0;
const STDIN = ++cnt;
const STDOUT = ++cnt;
const STDERR = ++cnt;
const FILE = ++cnt;
const HTTP = ++cnt;
const SOCKET_TCP = ++cnt;
const SOCKET_UDP = ++cnt;
const CLOCKS = ++cnt;
const INPUT_STREAM_CREATE = ++call_id << 24;
const INPUT_STREAM_READ = ++call_id << 24;
const INPUT_STREAM_BLOCKING_READ = ++call_id << 24;
const INPUT_STREAM_SKIP = ++call_id << 24;
const INPUT_STREAM_BLOCKING_SKIP = ++call_id << 24;
const INPUT_STREAM_SUBSCRIBE = ++call_id << 24;
const INPUT_STREAM_DISPOSE = ++call_id << 24;
const OUTPUT_STREAM_CREATE = ++call_id << 24;
const OUTPUT_STREAM_CHECK_WRITE = ++call_id << 24;
const OUTPUT_STREAM_WRITE = ++call_id << 24;
const OUTPUT_STREAM_BLOCKING_WRITE_AND_FLUSH = ++call_id << 24;
const OUTPUT_STREAM_FLUSH = ++call_id << 24;
const OUTPUT_STREAM_BLOCKING_FLUSH = ++call_id << 24;
const OUTPUT_STREAM_WRITE_ZEROES = ++call_id << 24;
const OUTPUT_STREAM_BLOCKING_WRITE_ZEROES_AND_FLUSH = ++call_id << 24;
const OUTPUT_STREAM_SPLICE = ++call_id << 24;
const OUTPUT_STREAM_BLOCKING_SPLICE = ++call_id << 24;
const OUTPUT_STREAM_SUBSCRIBE = ++call_id << 24;
const OUTPUT_STREAM_DISPOSE = ++call_id << 24;
const OUTPUT_STREAM_GET_TOTAL_BYTES = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_OPEN = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_CLOSE = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_ADVISE = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_SYNC_DATA = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_GET_TYPE = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_SET_SIZE = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_SET_TIMES = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_READ = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_WRITE = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_SYNC = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_STAT = ++call_id << 24;
const FILESYSTEM_DESCRIPTOR_METADATA_HASH = ++call_id << 24;
const POLL_POLLABLE_READY = ++call_id << 24;
const POLL_POLLABLE_BLOCK = ++call_id << 24;
const POLL_POLLABLE_DISPOSE = ++call_id << 24;
const POLL_POLL_LIST = ++call_id << 24;
const FUTURE_DISPOSE = ++call_id << 24;
const FUTURE_TAKE_VALUE = ++call_id << 24;
const FUTURE_SUBSCRIBE = ++call_id << 24;
const HTTP_CREATE_REQUEST = ++call_id << 24;
const HTTP_OUTPUT_STREAM_FINISH = ++call_id << 24;
const HTTP_SERVER_START = ++call_id << 24;
const HTTP_SERVER_STOP = ++call_id << 24;
const HTTP_SERVER_INCOMING_HANDLER = ++call_id << 24;
const HTTP_SERVER_SET_OUTGOING_RESPONSE = ++call_id << 24;
const HTTP_SERVER_CLEAR_OUTGOING_RESPONSE = ++call_id << 24;
const HTTP_SERVER_GET_ADDRESS = ++call_id << 24;
const HTTP_OUTGOING_BODY_DISPOSE = ++call_id << 24;
const CLOCKS_DURATION_SUBSCRIBE = ++call_id << 24;
const CLOCKS_INSTANT_SUBSCRIBE = ++call_id << 24;
const SOCKET_TCP_CREATE_HANDLE = ++call_id << 24;
const SOCKET_TCP_BIND_START = ++call_id << 24;
const SOCKET_TCP_BIND_FINISH = ++call_id << 24;
const SOCKET_TCP_CONNECT_START = ++call_id << 24;
const SOCKET_TCP_CONNECT_FINISH = ++call_id << 24;
const SOCKET_TCP_SUBSCRIBE = ++call_id << 24;
const SOCKET_TCP_LISTEN_START = ++call_id << 24;
const SOCKET_TCP_LISTEN_FINISH = ++call_id << 24;
const SOCKET_TCP_IS_LISTENING = ++call_id << 24;
const SOCKET_TCP_ACCEPT = ++call_id << 24;
const SOCKET_TCP_GET_LOCAL_ADDRESS = ++call_id << 24;
const SOCKET_TCP_GET_REMOTE_ADDRESS = ++call_id << 24;
const SOCKET_TCP_SET_KEEP_ALIVE = ++call_id << 24;
const SOCKET_TCP_SET_LISTEN_BACKLOG_SIZE = ++call_id << 24;
const SOCKET_TCP_SHUTDOWN = ++call_id << 24;
const SOCKET_TCP_DISPOSE = ++call_id << 24;
const SOCKET_UDP_CREATE_HANDLE = ++call_id << 24;
const SOCKET_UDP_BIND_START = ++call_id << 24;
const SOCKET_UDP_BIND_FINISH = ++call_id << 24;
const SOCKET_UDP_STREAM = ++call_id << 24;
const SOCKET_UDP_SUBSCRIBE = ++call_id << 24;
const SOCKET_UDP_DISPOSE = ++call_id << 24;
const SOCKET_UDP_GET_LOCAL_ADDRESS = ++call_id << 24;
const SOCKET_UDP_GET_RECEIVE_BUFFER_SIZE = ++call_id << 24;
const SOCKET_UDP_GET_REMOTE_ADDRESS = ++call_id << 24;
const SOCKET_UDP_GET_SEND_BUFFER_SIZE = ++call_id << 24;
const SOCKET_UDP_GET_UNICAST_HOP_LIMIT = ++call_id << 24;
const SOCKET_UDP_SET_RECEIVE_BUFFER_SIZE = ++call_id << 24;
const SOCKET_UDP_SET_SEND_BUFFER_SIZE = ++call_id << 24;
const SOCKET_UDP_SET_UNICAST_HOP_LIMIT = ++call_id << 24;
const SOCKET_INCOMING_DATAGRAM_STREAM_RECEIVE = ++call_id << 24;
const SOCKET_OUTGOING_DATAGRAM_STREAM_CHECK_SEND = ++call_id << 24;
const SOCKET_OUTGOING_DATAGRAM_STREAM_SEND = ++call_id << 24;
const SOCKET_DATAGRAM_STREAM_SUBSCRIBE = ++call_id << 24;
const SOCKET_DATAGRAM_STREAM_DISPOSE = ++call_id << 24;
const SOCKET_GET_DEFAULT_SEND_BUFFER_SIZE = ++call_id << 24;
const SOCKET_GET_DEFAULT_RECEIVE_BUFFER_SIZE = ++call_id << 24;
const SOCKET_RESOLVE_ADDRESS_CREATE_REQUEST = ++call_id << 24;
const SOCKET_RESOLVE_ADDRESS_TAKE_REQUEST = ++call_id << 24;
const SOCKET_RESOLVE_ADDRESS_SUBSCRIBE_REQUEST = ++call_id << 24;
const SOCKET_RESOLVE_ADDRESS_DISPOSE_REQUEST = ++call_id << 24;
const reverseMap = {};
for (const name of Object.getOwnPropertyNames(calls_exports)) {
	if (name === "reverseMap") continue;
	reverseMap[calls_exports[name]] = name;
}
//#endregion
//#region dist/io/worker-http.js
const agentOptions = { keepAlive: true };
const httpAgent = new Agent(agentOptions);
const httpsAgent = new Agent$1(agentOptions);
const servers = /* @__PURE__ */ new Map();
let responseCnt = 0;
const responses = /* @__PURE__ */ new Map();
async function stopHttpServer(id) {
	await new Promise((resolve) => servers.get(id).close(resolve));
}
function clearOutgoingResponse(id) {
	responses.delete(id);
}
async function getHttpServerAddress(id) {
	return servers.get(id).address();
}
async function setOutgoingResponse(id, { statusCode, headers, streamId }) {
	const response = responses.get(id);
	const textDecoder = new TextDecoder();
	response.writeHead(statusCode, Object.fromEntries(headers.map(([key, val]) => [key, textDecoder.decode(val)])));
	response.flushHeaders();
	const { stream } = getStreamOrThrow(streamId);
	stream.pipe(response);
	responses.delete(id);
}
async function startHttpServer(id, { port, host }) {
	const server = createServer((req, res) => {
		if (!parentPort) throw {
			tag: "internal-error",
			val: "Unable to post message. Likely running in main thread."
		};
		const streamId = createReadableStream(req);
		const responseId = ++responseCnt;
		parentPort.postMessage({
			type: HTTP_SERVER_INCOMING_HANDLER,
			id,
			payload: {
				responseId,
				method: req.method,
				host: req.headers.host || host || "localhost",
				pathWithQuery: req.url,
				headers: Object.entries(req.headersDistinct).flatMap(([key, val]) => val?.map((val) => [key, val])),
				streamId
			}
		});
		responses.set(responseId, res);
	});
	await new Promise((resolve, reject) => {
		server.listen(port, host, resolve);
		server.on("error", reject);
	});
	servers.set(id, server);
}
async function createHttpRequest(method, scheme, authority, pathWithQuery, headers, bodyId, connectTimeout, betweenBytesTimeout, firstByteTimeout) {
	let stream = null;
	if (bodyId) try {
		({stream} = getStreamOrThrow(bodyId));
	} catch (e) {
		if (e.tag === "closed") throw {
			tag: "internal-error",
			val: "Unexpected closed body stream"
		};
		if (e.tag === "last-operation-failed") throw {
			tag: "internal-error",
			val: e.val
		};
		throw e;
	}
	try {
		let req;
		switch (scheme) {
			case "http:":
				req = request({
					agent: httpAgent,
					method,
					host: authority.split(":")[0],
					port: authority.split(":")[1],
					path: pathWithQuery,
					timeout: connectTimeout && Number(connectTimeout / 1000000n)
				});
				break;
			case "https:":
				req = request$1({
					agent: httpsAgent,
					method,
					host: authority.split(":")[0],
					port: authority.split(":")[1],
					path: pathWithQuery,
					timeout: connectTimeout && Number(connectTimeout / 1000000n)
				});
				break;
			default: throw { tag: "HTTP-protocol-error" };
		}
		for (const [key, value] of headers) req.appendHeader(key, value);
		req.flushHeaders();
		if (stream) stream.pipe(req);
		else req.end();
		const res = await new Promise((resolve, reject) => {
			req.once("timeout", () => {
				reject({ tag: "connection-timeout" });
				req.destroy();
			});
			req.once("response", resolve);
			req.once("close", () => reject);
			req.once("error", reject);
		});
		if (firstByteTimeout) res.setTimeout(Number(firstByteTimeout / 1000000n));
		if (betweenBytesTimeout) res.once("readable", () => {
			res.setTimeout(Number(betweenBytesTimeout / 1000000n));
		});
		const bodyStreamId = createReadableStream(res);
		return {
			status: res.statusCode,
			headers: Array.from(Object.entries(res.headers)),
			bodyStreamId
		};
	} catch (e) {
		if (e?.tag) throw e;
		const err = getFirstError(e);
		switch (err.code) {
			case "ECONNRESET": throw { tag: "HTTP-protocol-error" };
			case "ENOTFOUND": throw {
				tag: "DNS-error",
				val: {
					rcode: err.code,
					infoCode: err.errno < 0 ? -err.errno : err.errno
				}
			};
			case "ECONNREFUSED": throw { tag: "connection-refused" };
		}
		throw {
			tag: "internal-error",
			val: err.toString()
		};
	}
}
function getFirstError(e) {
	if (typeof e !== "object" || e === null) return e;
	if (e.cause) return getFirstError(e.cause);
	if (e instanceof AggregateError) return getFirstError(e.errors[0]);
	return e;
}
//#endregion
//#region dist/io/worker-filesystem.js
const closeAsync = promisify(fs.close);
const fdatasyncAsync = promisify(fs.fdatasync);
const fstatAsync = promisify(fs.fstat);
const fsyncAsync = promisify(fs.fsync);
const ftruncateAsync = promisify(fs.ftruncate);
const futimesAsync = promisify(fs.futimes);
const openAsync = promisify(fs.open);
const readAsync = promisify(fs.read);
const writeAsync = promisify(fs.write);
let nextFileResourceId = 0;
const fileResources = /* @__PURE__ */ new Map();
function lookupType(obj) {
	if (obj.isFile()) return "regular-file";
	if (obj.isSocket()) return "socket";
	if (obj.isSymbolicLink()) return "symbolic-link";
	if (obj.isFIFO()) return "fifo";
	if (obj.isDirectory()) return "directory";
	if (obj.isCharacterDevice()) return "character-device";
	if (obj.isBlockDevice()) return "block-device";
	return "unknown";
}
function getFileResource(id) {
	const resource = fileResources.get(id);
	if (!resource) {
		const error = /* @__PURE__ */ new Error(`Invalid filesystem descriptor resource: ${id}`);
		error.code = "EBADF";
		throw error;
	}
	return resource;
}
async function openFileResource(path, flags) {
	const fd = await openAsync(path, flags);
	let type;
	try {
		type = lookupType(await fstatAsync(fd, { bigint: true }));
	} catch (error) {
		await closeAsync(fd);
		throw error;
	}
	const id = ++nextFileResourceId;
	fileResources.set(id, {
		fd,
		refs: 1
	});
	return {
		id,
		type
	};
}
function retainFileResource(id) {
	getFileResource(id).refs++;
	return id;
}
async function releaseFileResource(id) {
	const resource = getFileResource(id);
	if (--resource.refs === 0) {
		fileResources.delete(id);
		await closeAsync(resource.fd);
	}
}
function fileResourceFd(id) {
	return getFileResource(id).fd;
}
function adviseFileResource(id, offset, length, advice) {
	throw new Error("filesystem advise is unavailable in the vendored network-only host");
}
async function syncFileResourceData(id) {
	await fdatasyncAsync(fileResourceFd(id));
}
async function getFileResourceType(id) {
	return lookupType(await fstatAsync(fileResourceFd(id), { bigint: true }));
}
async function setFileResourceSize(id, size) {
	await ftruncateAsync(fileResourceFd(id), size);
}
async function setFileResourceTimes(id, atime, mtime) {
	await futimesAsync(fileResourceFd(id), atime, mtime);
}
async function readFileResource(id, length, offset) {
	const buffer = new Uint8Array(length);
	const result = await readAsync(fileResourceFd(id), buffer, 0, length, offset);
	const bytesRead = typeof result === "number" ? result : result.bytesRead;
	return {
		bytes: buffer.subarray(0, bytesRead),
		eof: bytesRead === 0
	};
}
async function writeFileResource(id, buffer, offset) {
	const result = await writeAsync(fileResourceFd(id), buffer, 0, buffer.byteLength, offset);
	return typeof result === "number" ? result : result.bytesWritten;
}
async function syncFileResource(id) {
	await fsyncAsync(fileResourceFd(id));
}
async function statFileResource(id) {
	const stats = await fstatAsync(fileResourceFd(id), { bigint: true });
	return {
		type: lookupType(stats),
		nlink: stats.nlink,
		size: stats.size,
		atimeNs: stats.atimeNs,
		mtimeNs: stats.mtimeNs,
		ctimeNs: stats.ctimeNs
	};
}
async function metadataHashFileResource(id) {
	const stats = await fstatAsync(fileResourceFd(id), { bigint: true });
	return {
		upper: stats.mtimeNs,
		lower: stats.ino
	};
}
//#endregion
//#region dist/io/worker-sockets.js
let stateCnt = 0;
const SOCKET_STATE_INIT = ++stateCnt;
const SOCKET_STATE_BIND = ++stateCnt;
const SOCKET_STATE_BOUND = ++stateCnt;
const SOCKET_STATE_LISTEN = ++stateCnt;
const SOCKET_STATE_LISTENER = ++stateCnt;
const SOCKET_STATE_CONNECT = ++stateCnt;
const SOCKET_STATE_CONNECTION = ++stateCnt;
const SOCKET_STATE_CLOSED = ++stateCnt;
const dnsLookupOptions = {
	verbatim: true,
	all: true,
	hints: V4MAPPED | ALL
};
function noLookup(ip, _opts, cb) {
	cb(null, ip);
}
function socketResolveAddress(name) {
	const isIpNum = isIP(name[0] === "[" && name[name.length - 1] === "]" ? name.slice(1, -1) : name);
	if (isIpNum > 0) return Promise.resolve([{
		tag: "ipv" + isIpNum,
		val: (isIpNum === 4 ? ipv4ToTuple : ipv6ToTuple)(name)
	}]);
	let parsedUrl = null;
	try {
		parsedUrl = new URL(`https://${name}`);
		if (parsedUrl.port.length || parsedUrl.username.length || parsedUrl.password.length || parsedUrl.pathname !== "/" || parsedUrl.search.length || parsedUrl.hash.length) parsedUrl = null;
	} catch {}
	if (!parsedUrl) throw "invalid-argument";
	return lookup(name, dnsLookupOptions).then((addresses) => {
		return (Array.isArray(addresses) ? addresses : [addresses]).map(({ address, family }) => {
			return [{
				tag: "ipv" + family,
				val: (family === 4 ? ipv4ToTuple : ipv6ToTuple)(address)
			}];
		});
	}, (err) => {
		switch (err.code) {
			case NODATA:
			case BADFAMILY:
			case NONAME:
			case NOTFOUND: throw "name-unresolvable";
			case TIMEOUT:
			case REFUSED:
			case CONNREFUSED:
			case SERVFAIL:
			case NOMEM:
			case CANCELLED: throw "temporary-resolver-failure";
			default: throw "permanent-resolver-failure";
		}
	});
}
function convertSocketError(err) {
	switch (err?.code) {
		case "EBADF":
		case "ENOTCONN":
		case "ERR_SOCKET_DGRAM_NOT_CONNECTED": return "invalid-state";
		case "EACCES":
		case "EPERM": return "access-denied";
		case "ENOTSUP": return "not-supported";
		case "EINVAL": return "invalid-argument";
		case "ENOMEM":
		case "ENOBUFS": return "out-of-memory";
		case "EALREADY": return "concurrency-conflict";
		case "EWOULDBLOCK": return "would-block";
		case "EMFILE":
		case "ENFILE": return "new-socket-limit";
		case "EADDRNOTAVAIL": return "address-not-bindable";
		case "EADDRINUSE": return "address-in-use";
		case "EHOSTUNREACH":
		case "EHOSTDOWN":
		case "ENETUNREACH":
		case "ENETDOWN": return "remote-unreachable";
		case "ECONNREFUSED": return "connection-refused";
		case "ECONNRESET": return "connection-reset";
		case "ECONNABORTED": return "connection-aborted";
		case "EMSGSIZE": return "datagram-too-large";
		default: return "unknown";
	}
}
function convertSocketErrorCode(code) {
	switch (code) {
		case 4053:
		case 4083:
		case ENOTCONN:
		case EBADF: return "invalid-state";
		case EACCES:
		case EPERM: return "access-denied";
		case ENOTSUP: return "not-supported";
		case EINVAL: return "invalid-argument";
		case ENOMEM:
		case ENOBUFS: return "out-of-memory";
		case EALREADY: return "concurrency-conflict";
		case EWOULDBLOCK: return "would-block";
		case EMFILE:
		case ENFILE: return "new-socket-limit";
		case 4090:
		case EADDRNOTAVAIL: return "address-not-bindable";
		case 4091:
		case EADDRINUSE: return "address-in-use";
		case EHOSTUNREACH:
		case ENETUNREACH:
		case ENETDOWN: return "remote-unreachable";
		case ECONNREFUSED: return "connection-refused";
		case ECONNRESET: return "connection-reset";
		case ECONNABORTED: return "connection-aborted";
		case EMSGSIZE: return "datagram-too-large";
		default: return "unknown";
	}
}
function tupleToIPv6(arr) {
	if (arr.length !== 8) return null;
	return arr.map((segment) => segment.toString(16)).join(":");
}
function tupleToIpv4(arr) {
	if (arr.length !== 4) return null;
	return arr.map((segment) => segment.toString(10)).join(".");
}
function isMulticastIpAddress(ipSocketAddress) {
	return ipSocketAddress.tag === "ipv4" && ipSocketAddress.val.address[0] === 224 || ipSocketAddress.tag === "ipv6" && ipSocketAddress.val.address[0] === 65280;
}
function isIPv4MappedAddress(ipSocketAddress) {
	return ipSocketAddress.tag === "ipv6" && ipSocketAddress.val.address[5] === 65535;
}
function isUnicastIpAddress(ipSocketAddress) {
	return !isMulticastIpAddress(ipSocketAddress) && !isBroadcastIpAddress(ipSocketAddress);
}
function isWildcardAddress(ipSocketAddress) {
	const { address } = ipSocketAddress.val;
	if (ipSocketAddress.tag === "ipv4") return address[0] === 0 && address[1] === 0 && address[2] === 0 && address[3] === 0;
	else return address[0] === 0 && address[1] === 0 && address[2] === 0 && address[3] === 0 && address[4] === 0 && address[5] === 0 && address[6] === 0 && address[7] === 0;
}
function isBroadcastIpAddress(ipSocketAddress) {
	const { address } = ipSocketAddress.val;
	return ipSocketAddress.tag === "ipv4" && address[0] === 255 && address[1] === 255 && address[2] === 255 && address[3] === 255;
}
function serializeIpAddress(addr) {
	if (addr.tag === "ipv4") return tupleToIpv4(addr.val.address);
	return tupleToIPv6(addr.val.address);
}
function ipv6ToTuple(ipv6) {
	const [lhs, rhs = ""] = ipv6.includes("::") ? ipv6.split("::") : [ipv6];
	const lhsParts = lhs === "" ? [] : lhs.split(":");
	const rhsParts = rhs === "" ? [] : rhs.split(":");
	return [
		...lhsParts,
		...Array(8 - lhsParts.length - rhsParts.length).fill(0),
		...rhsParts
	].map((segment) => parseInt(segment, 16));
}
function ipv4ToTuple(ipv4) {
	return ipv4.split(".").map((segment) => parseInt(segment, 10));
}
function ipSocketAddress(family, addr, port) {
	if (family === "ipv4" || family === "IPv4" || family === 4) return {
		tag: "ipv4",
		val: {
			port,
			address: ipv4ToTuple(addr)
		}
	};
	if (family !== "ipv6" && family !== "IPv6" && family !== 6) throw "invalid-argument";
	return {
		tag: "ipv6",
		val: {
			port,
			flowInfo: 0,
			address: ipv6ToTuple(addr),
			scopeId: 0
		}
	};
}
let _recvBufferSize;
let _sendBufferSize;
async function getDefaultBufferSizes() {
	var s = createSocket({ type: "udp4" });
	s.bind(0);
	await new Promise((resolve, reject) => {
		s.once("error", reject);
		s.once("listening", resolve);
	});
	_recvBufferSize = BigInt(s.getRecvBufferSize());
	_sendBufferSize = BigInt(s.getSendBufferSize());
	s.close();
}
async function getDefaultSendBufferSize() {
	if (!_sendBufferSize) await getDefaultBufferSizes();
	return _sendBufferSize;
}
async function getDefaultReceiveBufferSize() {
	if (!_recvBufferSize) await getDefaultBufferSizes();
	return _recvBufferSize;
}
//#endregion
//#region dist/io/worker-socket-tcp.js
const tcpSockets = /* @__PURE__ */ new Map();
let tcpSocketCnt = 0;
function createTcpSocket() {
	tcpSockets.set(++tcpSocketCnt, {
		state: SOCKET_STATE_INIT,
		future: null,
		tcpSocket: null,
		server: null,
		localAddress: null,
		listenBacklogSize: 128,
		keepAlive: false,
		keepAliveIdleTime: 0,
		pendingAccepts: [],
		pollState: {
			ready: true,
			listener: null,
			polls: [],
			parentStream: null
		}
	});
	return tcpSocketCnt;
}
function socketTcpFinish(id, fromState, toState) {
	const socket = tcpSockets.get(id);
	if (socket.state !== fromState) throw "not-in-progress";
	if (!socket.pollState?.ready) throw "would-block";
	const { tag, val } = futureTakeValue(socket.future)?.val ?? {};
	futureDispose(socket.future, false);
	socket.future = null;
	if (tag === "err") {
		if (fromState === SOCKET_STATE_BIND) {
			socket.server = null;
			socket.localAddress = null;
			socket.state = SOCKET_STATE_INIT;
		} else socket.state = SOCKET_STATE_CLOSED;
		throw val;
	} else {
		socket.state = toState;
		if (toState === SOCKET_STATE_LISTENER) socket.pollState.ready = socket.pendingAccepts.length > 0;
		return val;
	}
}
function socketTcpBindStart(id, localAddress, family) {
	const socket = tcpSockets.get(id);
	if (socket.state !== SOCKET_STATE_INIT) throw "invalid-state";
	if (family !== localAddress.tag || !isUnicastIpAddress(localAddress) || isIPv4MappedAddress(localAddress)) throw "invalid-argument";
	socket.state = SOCKET_STATE_BIND;
	socket.future = createFuture(new Promise((resolve, reject) => {
		const address = serializeIpAddress(localAddress);
		const port = localAddress.val.port;
		const server = socket.server = createTcpServer(socket);
		function handleErr(err) {
			server.off("listening", handleListen);
			reject(convertSocketError(err));
		}
		function handleListen() {
			server.off("error", handleErr);
			const boundAddress = server.address();
			if (!boundAddress || typeof boundAddress === "string") {
				reject("invalid-state");
				return;
			}
			socket.localAddress = ipSocketAddress(boundAddress.family, boundAddress.address, boundAddress.port);
			resolve();
		}
		server.once("listening", handleListen);
		server.once("error", handleErr);
		server.listen({
			host: address ?? void 0,
			port,
			backlog: socket.listenBacklogSize,
			ipv6Only: family === "ipv6"
		});
	}), socket.pollState);
}
function createTcpServer(socket) {
	const server = new Server({
		pauseOnConnect: true,
		allowHalfOpen: true
	});
	server.on("connection", (tcpSocket) => {
		if (socket.state !== SOCKET_STATE_LISTEN && socket.state !== SOCKET_STATE_LISTENER) {
			tcpSocket.destroy();
			return;
		}
		pollStateReady(socket.pollState);
		const pollState = createReadableStreamPollState(tcpSocket);
		socket.pendingAccepts.push({
			tcpSocket,
			err: null,
			pollState
		});
	});
	server.on("error", (err) => {
		if (socket.state === SOCKET_STATE_LISTEN || socket.state === SOCKET_STATE_LISTENER) {
			pollStateReady(socket.pollState);
			socket.pendingAccepts.push({
				tcpSocket: null,
				err,
				pollState: null
			});
		}
	});
	return server;
}
function closeTcpServer(socket) {
	const server = socket.server;
	socket.server = null;
	if (!server) return Promise.resolve();
	return new Promise((resolve, reject) => {
		server.close((err) => err ? reject(err) : resolve());
	});
}
function socketTcpConnectStart(id, remoteAddress, family) {
	const socket = tcpSockets.get(id);
	if (socket.state !== SOCKET_STATE_INIT && socket.state !== SOCKET_STATE_BOUND) throw "invalid-state";
	if (isWildcardAddress(remoteAddress) || family !== remoteAddress.tag || !isUnicastIpAddress(remoteAddress) || isMulticastIpAddress(remoteAddress) || remoteAddress.val.port === 0 || isIPv4MappedAddress(remoteAddress)) throw "invalid-argument";
	socket.state = SOCKET_STATE_CONNECT;
	socket.future = createFuture((async () => {
		const localAddress = socket.localAddress;
		await closeTcpServer(socket);
		const tcpSocket = socket.tcpSocket = new Socket({ allowHalfOpen: true });
		tcpSocket.setKeepAlive(socket.keepAlive, socket.keepAliveIdleTime);
		await new Promise((resolve, reject) => {
			function handleErr(err) {
				tcpSocket.off("connect", handleConnect);
				reject(convertSocketError(err));
			}
			function handleConnect() {
				tcpSocket.off("error", handleErr);
				resolve();
			}
			tcpSocket.once("connect", handleConnect);
			tcpSocket.once("error", handleErr);
			tcpSocket.connect({
				port: remoteAddress.val.port,
				host: serializeIpAddress(remoteAddress) ?? void 0,
				lookup: noLookup,
				localAddress: localAddress ? serializeIpAddress(localAddress) ?? void 0 : void 0,
				localPort: localAddress?.val.port
			});
		});
		return [createReadableStream(tcpSocket), createWritableStream(tcpSocket)];
	})(), socket.pollState);
}
function socketTcpListenStart(id) {
	const socket = tcpSockets.get(id);
	if (socket.state !== SOCKET_STATE_BOUND) throw "invalid-state";
	socket.state = SOCKET_STATE_LISTEN;
	socket.future = createFuture(Promise.resolve(), socket.pollState);
}
function socketTcpAccept(id) {
	const socket = tcpSockets.get(id);
	if (socket.state !== SOCKET_STATE_LISTENER) throw "invalid-state";
	if (socket.pendingAccepts.length === 0) throw "would-block";
	const accept = socket.pendingAccepts.shift();
	if (!accept || accept.err) {
		socket.state = SOCKET_STATE_CLOSED;
		throw convertSocketError(accept?.err);
	}
	if (socket.pollState && socket.pendingAccepts.length === 0) socket.pollState.ready = false;
	tcpSockets.set(++tcpSocketCnt, {
		state: SOCKET_STATE_CONNECTION,
		future: null,
		tcpSocket: accept.tcpSocket,
		server: null,
		localAddress: null,
		listenBacklogSize: 128,
		keepAlive: false,
		keepAliveIdleTime: 0,
		pendingAccepts: [],
		pollState: accept.pollState
	});
	return [
		tcpSocketCnt,
		createReadableStream(accept.tcpSocket, accept.pollState ?? void 0),
		createWritableStream(accept.tcpSocket)
	];
}
function socketTcpSetListenBacklogSize(id, backlogSize) {
	const socket = tcpSockets.get(id);
	if (socket.state === SOCKET_STATE_LISTEN || socket.state === SOCKET_STATE_LISTENER) throw "not-supported";
	if (socket.state !== SOCKET_STATE_INIT && socket.state !== SOCKET_STATE_BIND && socket.state !== SOCKET_STATE_BOUND) throw "invalid-state";
	socket.listenBacklogSize = Number(backlogSize);
}
function socketTcpGetLocalAddress(id) {
	const socket = tcpSockets.get(id);
	const address = socket.tcpSocket?.address();
	if (address && typeof address !== "string" && "family" in address) return ipSocketAddress(address.family, address.address, address.port);
	if (socket.localAddress) return socket.localAddress;
	throw "invalid-state";
}
function socketTcpGetRemoteAddress(id) {
	const { tcpSocket } = tcpSockets.get(id);
	if (!tcpSocket?.remoteFamily || !tcpSocket.remoteAddress || !tcpSocket.remotePort) throw "invalid-state";
	return ipSocketAddress(tcpSocket.remoteFamily, tcpSocket.remoteAddress, tcpSocket.remotePort);
}
function socketTcpShutdown(id, _shutdownType) {
	const socket = tcpSockets.get(id);
	if (socket.state !== SOCKET_STATE_CONNECTION) throw "invalid-state";
	socket.tcpSocket?.destroy();
}
function socketTcpSetKeepAlive(id, { keepAlive, keepAliveIdleTime }) {
	const socket = tcpSockets.get(id);
	socket.keepAlive = keepAlive;
	socket.keepAliveIdleTime = Number(keepAliveIdleTime / 1000000000n);
	socket.tcpSocket?.setKeepAlive(socket.keepAlive, socket.keepAliveIdleTime);
}
function socketTcpDispose(id) {
	const socket = tcpSockets.get(id);
	verifyPollsDroppedForDrop(socket.pollState, "tcp socket");
	socket.server?.close();
	socket.tcpSocket?.destroy();
	tcpSockets.delete(id);
}
//#endregion
//#region dist/io/worker-socket-udp.js
let udpSocketCnt = 0;
let datagramStreamCnt = 0;
const udpSockets = /* @__PURE__ */ new Map();
const datagramStreams = /* @__PURE__ */ new Map();
function createUdpSocket({ family, unicastHopLimit }) {
	const udpSocket = createSocket({
		type: family === "ipv6" ? "udp6" : "udp4",
		reuseAddr: false,
		ipv6Only: family === "ipv6",
		lookup: noLookup
	});
	udpSockets.set(++udpSocketCnt, {
		state: SOCKET_STATE_INIT,
		remoteAddress: null,
		remotePort: null,
		sendBufferSize: null,
		receiveBufferSize: null,
		unicastHopLimit,
		udpSocket,
		future: null,
		serializedLocalAddress: null,
		pollState: {
			ready: true,
			listener: () => null,
			polls: [],
			parentStream: null
		},
		incomingDatagramStream: null,
		outgoingDatagramStream: null
	});
	return udpSocketCnt;
}
function createIncomingDatagramStream(socket) {
	const id = ++datagramStreamCnt;
	const pollState = {
		ready: false,
		listener: null,
		polls: [],
		parentStream: null
	};
	const datagramStream = {
		id,
		active: true,
		error: null,
		socket,
		queue: [],
		cleanup,
		pollState
	};
	const { udpSocket } = socket;
	datagramStreams.set(id, datagramStream);
	function cleanup() {
		udpSocket.off("message", onMessage);
		udpSocket.off("error", onError);
	}
	function onMessage(data, rinfo) {
		datagramStream.queue?.push({
			data,
			remoteAddress: ipSocketAddress(rinfo.family, rinfo.address, rinfo.port)
		});
		if (!pollState.ready) pollStateReady(pollState);
	}
	function onError(err) {
		datagramStream.error = err;
		pollStateReady(datagramStream.pollState);
	}
	udpSocket.on("message", onMessage);
	udpSocket.once("error", onError);
	return datagramStream;
}
function createOutgoingDatagramStream(socket) {
	const id = ++datagramStreamCnt;
	const datagramStream = {
		id,
		active: true,
		error: null,
		socket,
		cleanup,
		pollState: {
			ready: true,
			listener: null,
			polls: [],
			parentStream: null
		}
	};
	const { udpSocket } = socket;
	datagramStreams.set(id, datagramStream);
	udpSocket.on("error", onError);
	function onError(err) {
		datagramStream.error = err;
		pollStateReady(datagramStream.pollState);
	}
	function cleanup() {
		udpSocket.off("error", onError);
	}
	return datagramStream;
}
function socketUdpBindStart(id, localAddress, family) {
	const socket = udpSockets.get(id);
	if (family !== localAddress.tag || isIPv4MappedAddress(localAddress)) throw "invalid-argument";
	const serializedLocalAddress = serializeIpAddress(localAddress);
	if (socket.state !== SOCKET_STATE_INIT) throw "invalid-state";
	socket.state = SOCKET_STATE_BIND;
	const { udpSocket } = socket;
	socket.future = createFuture(new Promise((resolve, reject) => {
		function bindOk() {
			resolve();
			udpSocket.off("error", bindErr);
		}
		function bindErr(err) {
			reject(convertSocketError(err));
			udpSocket.off("listening", bindOk);
		}
		udpSocket.once("listening", bindOk);
		udpSocket.once("error", bindErr);
		udpSocket.bind(localAddress.val.port, serializedLocalAddress ?? void 0);
	}), socket.pollState);
}
function socketUdpBindFinish(id) {
	const socket = udpSockets.get(id);
	if (socket.state !== SOCKET_STATE_BIND) throw "not-in-progress";
	if (!socket.pollState.ready) throw "would-block";
	const { tag, val } = futureTakeValue(socket.future)?.val ?? {};
	futureDispose(socket.future, false);
	socket.future = null;
	if (tag === "err") {
		socket.state = SOCKET_STATE_CLOSED;
		throw val;
	} else {
		socket.udpSocket.setTTL(socket.unicastHopLimit);
		if (socket.sendBufferSize) socket.udpSocket.setRecvBufferSize(socket.sendBufferSize);
		if (socket.receiveBufferSize) socket.udpSocket.setSendBufferSize(socket.receiveBufferSize);
		socket.state = SOCKET_STATE_BOUND;
		return val;
	}
}
function socketUdpGetLocalAddress(id) {
	const { udpSocket } = udpSockets.get(id);
	let address, family, port;
	try {
		({address, family, port} = udpSocket.address());
	} catch (err) {
		throw convertSocketError(err);
	}
	return ipSocketAddress(family, address, port);
}
/**
* @param {number} id
* @returns {IpSocketAddress}
*/
function socketUdpGetRemoteAddress(id) {
	const { udpSocket } = udpSockets.get(id);
	let address, family, port;
	try {
		({address, family, port} = udpSocket.remoteAddress());
	} catch (err) {
		throw convertSocketError(err);
	}
	return ipSocketAddress(family, address, port);
}
function socketUdpStream(id, remoteAddress) {
	const socket = udpSockets.get(id);
	const { udpSocket } = socket;
	if (socket.state !== SOCKET_STATE_BOUND && socket.state !== SOCKET_STATE_CONNECTION) throw "invalid-state";
	if (socket.state === SOCKET_STATE_INIT && !remoteAddress) throw "invalid-state";
	if (remoteAddress && (remoteAddress.val.port === 0 || isWildcardAddress(remoteAddress) || remoteAddress.tag === "ipv6" && isIPv4MappedAddress(remoteAddress))) throw "invalid-argument";
	if (socket.state === SOCKET_STATE_CONNECTION) {
		socketDatagramStreamClear(socket.incomingDatagramStream);
		socketDatagramStreamClear(socket.outgoingDatagramStream);
		try {
			udpSocket.disconnect();
		} catch (e) {
			throw convertSocketErrorCode(e);
		}
	}
	if (remoteAddress) {
		const serializedRemoteAddress = serializeIpAddress(remoteAddress);
		socket.remoteAddress = serializedRemoteAddress;
		socket.remotePort = remoteAddress.val.port;
		return new Promise((resolve, reject) => {
			function connectOk() {
				if (socket.state === SOCKET_STATE_INIT) {
					socket.udpSocket.setTTL(socket.unicastHopLimit);
					socket.udpSocket.setRecvBufferSize(socket.sendBufferSize);
					socket.udpSocket.setSendBufferSize(socket.receiveBufferSize);
				}
				udpSocket.off("error", connectErr);
				socket.state = SOCKET_STATE_CONNECTION;
				resolve([(socket.incomingDatagramStream = createIncomingDatagramStream(socket)).id, (socket.outgoingDatagramStream = createOutgoingDatagramStream(socket)).id]);
			}
			function connectErr(err) {
				udpSocket.off("connect", connectOk);
				reject(convertSocketError(err));
			}
			udpSocket.once("connect", connectOk);
			udpSocket.once("error", connectErr);
			udpSocket.connect(remoteAddress.val.port, serializedRemoteAddress ?? void 0);
		});
	} else {
		socket.state = SOCKET_STATE_BOUND;
		socket.remoteAddress = null;
		socket.remotePort = null;
		return [(socket.incomingDatagramStream = createIncomingDatagramStream(socket)).id, (socket.outgoingDatagramStream = createOutgoingDatagramStream(socket)).id];
	}
}
function socketUdpSetReceiveBufferSize(id, bufferSize) {
	const socket = udpSockets.get(id);
	const buf = Number(bufferSize);
	if (socket.state !== SOCKET_STATE_INIT && socket.state !== SOCKET_STATE_BIND) try {
		socket.udpSocket.setRecvBufferSize(buf);
	} catch (err) {
		throw convertSocketError(err);
	}
	socket.receiveBufferSize = buf;
}
function socketUdpSetSendBufferSize(id, bufferSize) {
	const socket = udpSockets.get(id);
	const buf = Number(bufferSize);
	if (socket.state !== SOCKET_STATE_INIT && socket.state !== SOCKET_STATE_BIND) try {
		socket.udpSocket.setSendBufferSize(buf);
	} catch (err) {
		throw convertSocketError(err);
	}
	socket.sendBufferSize = buf;
}
function socketUdpSetUnicastHopLimit(id, hopLimit) {
	const socket = udpSockets.get(id);
	if (socket.state !== SOCKET_STATE_INIT && socket.state !== SOCKET_STATE_BIND) try {
		socket.udpSocket.setTTL(hopLimit);
	} catch (err) {
		throw convertSocketError(err);
	}
	socket.unicastHopLimit = hopLimit;
}
async function socketUdpGetReceiveBufferSize(id) {
	const socket = udpSockets.get(id);
	if (socket.receiveBufferSize) return BigInt(socket.receiveBufferSize);
	if (socket.state !== SOCKET_STATE_INIT && socket.state !== SOCKET_STATE_BIND) try {
		return BigInt(socket.receiveBufferSize = socket.udpSocket.getRecvBufferSize());
	} catch (err) {
		throw convertSocketError(err);
	}
	else {
		const receiveBufferSize = await getDefaultReceiveBufferSize();
		socket.receiveBufferSize = Number(receiveBufferSize);
		return receiveBufferSize;
	}
}
async function socketUdpGetSendBufferSize(id) {
	const socket = udpSockets.get(id);
	if (socket.sendBufferSize) return BigInt(socket.sendBufferSize);
	if (socket.state !== SOCKET_STATE_INIT && socket.state !== SOCKET_STATE_BIND) try {
		return BigInt(socket.sendBufferSize = socket.udpSocket.getSendBufferSize());
	} catch (err) {
		throw convertSocketError(err);
	}
	else {
		const sendBufferSize = await getDefaultSendBufferSize();
		socket.sendBufferSize = Number(sendBufferSize);
		return sendBufferSize;
	}
}
function socketUdpGetUnicastHopLimit(id) {
	const { unicastHopLimit } = udpSockets.get(id);
	return unicastHopLimit;
}
function socketUdpDispose(id) {
	const { udpSocket } = udpSockets.get(id);
	return new Promise((resolve) => {
		udpSocket.close(() => {
			udpSockets.delete(id);
			resolve(0);
		});
	});
}
function socketIncomingDatagramStreamReceive(id, maxResults) {
	const datagramStream = datagramStreams.get(id);
	if (!datagramStream.active) throw new Error("wasi-io trap: attempt to receive on inactive incoming datagram stream");
	if (maxResults === 0n || datagramStream.queue?.length === 0) return [];
	if (datagramStream.error) throw convertSocketError(datagramStream.error);
	return datagramStream.queue?.splice(0, Number(maxResults));
}
async function socketOutgoingDatagramStreamSend(id, datagrams) {
	const { active, socket } = datagramStreams.get(id);
	if (!active) throw new Error("wasi-io trap: writing to inactive outgoing datagram stream");
	const { udpSocket } = socket;
	let sendQueue = [];
	let sendQueueAddress = null;
	let sendQueuePort = null;
	let datagramsSent = 0;
	for (const { data, remoteAddress } of datagrams) {
		const address = remoteAddress ? serializeIpAddress(remoteAddress) : socket.remoteAddress;
		const port = remoteAddress?.val.port ?? socket.remotePort;
		let sendLastBatch = true;
		if (sendQueue.length > 0) {
			if (sendQueueAddress === address && sendQueuePort === port) sendQueue.push(data);
			else sendLastBatch = true;
		} else {
			sendQueueAddress = address;
			sendQueuePort = port;
			sendQueue.push(data);
		}
		if (sendLastBatch) {
			if (await doSendBatch()) return BigInt(datagramsSent);
			sendQueue = [];
			sendQueuePort = port;
			sendQueueAddress = address;
		}
	}
	if (sendQueue.length) {
		if (await doSendBatch()) return BigInt(datagramsSent);
	}
	if (datagramsSent !== datagrams.length) throw new Error("wasi-io trap: expected to have sent all the datagrams");
	return BigInt(datagramsSent);
	function doSendBatch() {
		return new Promise((resolve, reject) => {
			if (socket.remoteAddress) {
				if (sendQueueAddress !== socket.remoteAddress || sendQueuePort !== socket.remotePort) {
					reject("invalid-argument");
					return;
				}
				udpSocket.send(sendQueue, handler);
			} else {
				if (!sendQueueAddress) {
					reject("invalid-argument");
					return;
				}
				udpSocket.send(sendQueue, sendQueuePort, sendQueueAddress, handler);
			}
			function handler(err, _sentBytes) {
				if (err) {
					if (datagramsSent > 0) resolve(datagramsSent);
					else reject(convertSocketError(err));
					return;
				}
				datagramsSent += sendQueue.length;
				resolve(false);
			}
		});
	}
}
function checkSend(socket) {
	try {
		return Math.floor((socket.udpSocket.getSendBufferSize() - socket.udpSocket.getSendQueueSize()) / 1500);
	} catch (err) {
		throw convertSocketError(err);
	}
}
function pollSend(socket) {
	socket.pollState.ready = false;
	setTimeout(() => {
		if (checkSend(socket) > 0) pollStateReady(socket.pollState);
		else pollSend(socket);
	});
}
function socketOutgoingDatagramStreamCheckSend(id) {
	const { active, socket } = datagramStreams.get(id);
	if (!active) throw new Error("wasi-io trap: check send on inactive outgoing datagram stream");
	const remaining = checkSend(socket);
	if (remaining <= 0) pollSend(socket);
	return BigInt(remaining);
}
function socketDatagramStreamClear(datagramStream) {
	datagramStream.active = false;
	if (datagramStream.cleanup) {
		datagramStream.cleanup();
		datagramStream.cleanup = null;
	}
}
function socketDatagramStreamDispose(id) {
	const datagramStream = datagramStreams.get(id);
	datagramStream.active = false;
	if (datagramStream.cleanup) {
		datagramStream.cleanup();
		datagramStream.cleanup = () => null;
	}
	verifyPollsDroppedForDrop(datagramStream.pollState, "datagram stream");
	datagramStreams.delete(id);
}
//#endregion
//#region dist/io/worker-thread.js
function log(msg) {
	if (debug) process._rawDebug(msg);
}
let pollCnt = 0;
let streamCnt = 0;
let futureCnt = 0;
/** @type {Map<number, PollState>} */
const polls = /* @__PURE__ */ new Map();
/** @type {Map<number, Stream>} */
const streams = /* @__PURE__ */ new Map();
/** @type {Map<number, Future>} */
const futures = /* @__PURE__ */ new Map();
function createReadableStreamPollState(nodeStream) {
	const pollState = {
		ready: true,
		listener: null,
		polls: [],
		parentStream: nodeStream
	};
	function pollDone() {
		pollStateReady(pollState);
		nodeStream.off("end", pollDone);
		nodeStream.off("close", pollDone);
		nodeStream.off("error", pollDone);
	}
	nodeStream.on("end", pollDone);
	nodeStream.on("close", pollDone);
	nodeStream.on("error", pollDone);
	return pollState;
}
/**
* @param {NodeJS.ReadableStream | NodeJS.WritableStream} stream
*/
function createReadableStream(nodeStream, pollState = createReadableStreamPollState(nodeStream)) {
	const stream = {
		stream: nodeStream,
		flushPromise: null,
		pollState
	};
	streams.set(++streamCnt, stream);
	return streamCnt;
}
function createWritableStream(nodeStream) {
	const pollState = {
		ready: true,
		listener: null,
		polls: [],
		parentStream: null
	};
	const stream = {
		stream: nodeStream,
		flushPromise: null,
		pollState
	};
	streams.set(++streamCnt, stream);
	function pollReady() {
		pollStateReady(pollState);
	}
	function pollDone() {
		pollStateReady(pollState);
		nodeStream.off("drain", pollReady);
		nodeStream.off("finish", pollDone);
		nodeStream.off("error", pollDone);
		nodeStream.off("close", pollDone);
	}
	nodeStream.on("drain", pollReady);
	nodeStream.on("finish", pollDone);
	nodeStream.on("error", pollDone);
	nodeStream.on("close", pollDone);
	return streamCnt;
}
createWritableStream(stdout);
createWritableStream(stderr);
/**
* @param {number} streamId
* @param {NodeJS.ReadableStream | NodeJS.WritableStream} stream
*/
function streamError(err) {
	return {
		tag: "last-operation-failed",
		val: {
			code: err.code,
			message: err.message,
			stack: err.stack
		}
	};
}
/**
* @param {number} streamId
* @returns {{ stream: NodeJS.ReadableStream | NodeJS.WritableStream, polls: number[] }}
*/
function getStreamOrThrow(streamId) {
	if (!streamId) throw new Error("wasi-io trap: no stream id provided");
	const stream = streams.get(streamId);
	if (!stream) throw { tag: "closed" };
	if (stream.stream.errored) throw streamError(stream.stream.errored);
	if (stream.stream.closed) throw { tag: "closed" };
	return stream;
}
/**
* @param {number} call
* @param {number | null} id
* @param {any} payload
* @returns {Promise<any>}
*/
function handle(call, id, payload) {
	if (uncaughtException) throw uncaughtException;
	switch (call) {
		case HTTP_CREATE_REQUEST: {
			const { method, scheme, authority, pathWithQuery, headers, body, connectTimeout, betweenBytesTimeout, firstByteTimeout } = payload;
			return createFuture(createHttpRequest(method, scheme, authority, pathWithQuery, headers, body, connectTimeout, betweenBytesTimeout, firstByteTimeout), void 0);
		}
		case OUTPUT_STREAM_CREATE | HTTP: {
			const stream = new PassThrough();
			stream["contentLength"] = payload;
			stream["bytesRemaining"] = payload;
			return createWritableStream(stream);
		}
		case OUTPUT_STREAM_SUBSCRIBE | HTTP:
		case OUTPUT_STREAM_FLUSH | HTTP:
		case OUTPUT_STREAM_BLOCKING_WRITE_AND_FLUSH | HTTP: {
			const { stream } = getStreamOrThrow(id);
			if (call === (OUTPUT_STREAM_BLOCKING_WRITE_AND_FLUSH | HTTP)) {
				stream.bytesRemaining -= payload.byteLength;
				if (stream.bytesRemaining < 0) throw {
					tag: "last-operation-failed",
					val: {
						tag: "HTTP-request-body-size",
						val: stream.contentLength - stream.bytesRemaining
					}
				};
			}
			return handle(call & ~HTTP, id, payload);
		}
		case OUTPUT_STREAM_WRITE | HTTP: {
			const { stream } = getStreamOrThrow(id);
			stream.bytesRemaining -= payload.byteLength;
			if (stream.bytesRemaining < 0) throw {
				tag: "last-operation-failed",
				val: {
					tag: "HTTP-request-body-size",
					val: stream.contentLength - stream.bytesRemaining
				}
			};
			return handle(OUTPUT_STREAM_WRITE, id, payload);
		}
		case OUTPUT_STREAM_DISPOSE | HTTP: throw new Error("wasi-io trap: Output stream dispose not implemented as an IO-call for HTTP");
		case HTTP_OUTPUT_STREAM_FINISH: {
			let stream;
			try {
				({stream} = getStreamOrThrow(id));
			} catch (e) {
				if (e.tag === "closed") throw {
					tag: "internal-error",
					val: "stream closed"
				};
				if (e.tag === "last-operation-failed") throw {
					tag: "internal-error",
					val: e.val.message
				};
			}
			if (stream.bytesRemaining > 0) throw {
				tag: "HTTP-request-body-size",
				val: stream.contentLength - stream.bytesRemaining
			};
			if (stream.bytesRemaining < 0) throw {
				tag: "HTTP-request-body-size",
				val: stream.contentLength - stream.bytesRemaining
			};
			stream.end();
			return;
		}
		case HTTP_OUTGOING_BODY_DISPOSE:
			if (debug && !streams.has(id)) console.warn(`wasi-io: stream ${id} not found to dispose`);
			streams.delete(id);
			return;
		case HTTP_SERVER_START: return startHttpServer(id, payload);
		case HTTP_SERVER_STOP: return stopHttpServer(id);
		case HTTP_SERVER_SET_OUTGOING_RESPONSE: return setOutgoingResponse(id, payload);
		case HTTP_SERVER_CLEAR_OUTGOING_RESPONSE: return clearOutgoingResponse(id);
		case HTTP_SERVER_GET_ADDRESS: return getHttpServerAddress(id);
		case SOCKET_RESOLVE_ADDRESS_CREATE_REQUEST: return createFuture(socketResolveAddress(payload), void 0);
		case SOCKET_RESOLVE_ADDRESS_SUBSCRIBE_REQUEST: return createPoll(futures.get(id).pollState);
		case SOCKET_RESOLVE_ADDRESS_DISPOSE_REQUEST:
			futureDispose(id, true);
			return;
		case SOCKET_RESOLVE_ADDRESS_TAKE_REQUEST: {
			const val = futureTakeValue(id);
			if (val === void 0) throw "would-block";
			return val.val;
		}
		case SOCKET_TCP_ACCEPT: return socketTcpAccept(id);
		case SOCKET_TCP_CREATE_HANDLE: return createTcpSocket();
		case SOCKET_TCP_BIND_START: return socketTcpBindStart(id, payload.localAddress, payload.family);
		case SOCKET_TCP_BIND_FINISH: return socketTcpFinish(id, SOCKET_STATE_BIND, SOCKET_STATE_BOUND);
		case SOCKET_TCP_CONNECT_START: return socketTcpConnectStart(id, payload.remoteAddress, payload.family);
		case SOCKET_TCP_CONNECT_FINISH: return socketTcpFinish(id, SOCKET_STATE_CONNECT, SOCKET_STATE_CONNECTION);
		case SOCKET_TCP_LISTEN_START: return socketTcpListenStart(id);
		case SOCKET_TCP_LISTEN_FINISH: return socketTcpFinish(id, SOCKET_STATE_LISTEN, SOCKET_STATE_LISTENER);
		case SOCKET_TCP_IS_LISTENING: return tcpSockets.get(id)?.state === SOCKET_STATE_LISTENER;
		case SOCKET_GET_DEFAULT_SEND_BUFFER_SIZE: return getDefaultSendBufferSize();
		case SOCKET_GET_DEFAULT_RECEIVE_BUFFER_SIZE: return getDefaultReceiveBufferSize();
		case SOCKET_TCP_SET_LISTEN_BACKLOG_SIZE: return socketTcpSetListenBacklogSize(id, void 0);
		case SOCKET_TCP_GET_LOCAL_ADDRESS: return socketTcpGetLocalAddress(id);
		case SOCKET_TCP_GET_REMOTE_ADDRESS: return socketTcpGetRemoteAddress(id);
		case SOCKET_TCP_SHUTDOWN: return socketTcpShutdown(id, payload);
		case SOCKET_TCP_SUBSCRIBE: return createPoll(tcpSockets.get(id)?.pollState);
		case SOCKET_TCP_SET_KEEP_ALIVE: return socketTcpSetKeepAlive(id, payload);
		case SOCKET_TCP_DISPOSE: return socketTcpDispose(id);
		case SOCKET_UDP_CREATE_HANDLE: return createUdpSocket(payload);
		case SOCKET_UDP_BIND_START: return socketUdpBindStart(id, payload.localAddress, payload.family);
		case SOCKET_UDP_BIND_FINISH: return socketUdpBindFinish(id);
		case SOCKET_UDP_STREAM: return socketUdpStream(id, payload);
		case SOCKET_UDP_SUBSCRIBE: return createPoll(udpSockets.get(id)?.pollState);
		case SOCKET_UDP_GET_LOCAL_ADDRESS: return socketUdpGetLocalAddress(id);
		case SOCKET_UDP_GET_REMOTE_ADDRESS: return socketUdpGetRemoteAddress(id);
		case SOCKET_UDP_SET_RECEIVE_BUFFER_SIZE: return socketUdpSetReceiveBufferSize(id, payload);
		case SOCKET_UDP_SET_SEND_BUFFER_SIZE: return socketUdpSetSendBufferSize(id, payload);
		case SOCKET_UDP_SET_UNICAST_HOP_LIMIT: return socketUdpSetUnicastHopLimit(id, payload);
		case SOCKET_UDP_GET_RECEIVE_BUFFER_SIZE: return socketUdpGetReceiveBufferSize(id);
		case SOCKET_UDP_GET_SEND_BUFFER_SIZE: return socketUdpGetSendBufferSize(id);
		case SOCKET_UDP_GET_UNICAST_HOP_LIMIT: return socketUdpGetUnicastHopLimit(id);
		case SOCKET_UDP_DISPOSE: return socketUdpDispose(id);
		case SOCKET_INCOMING_DATAGRAM_STREAM_RECEIVE: return socketIncomingDatagramStreamReceive(id, payload);
		case SOCKET_OUTGOING_DATAGRAM_STREAM_CHECK_SEND: return socketOutgoingDatagramStreamCheckSend(id);
		case SOCKET_OUTGOING_DATAGRAM_STREAM_SEND: return socketOutgoingDatagramStreamSend(id, payload);
		case SOCKET_DATAGRAM_STREAM_SUBSCRIBE: return createPoll(datagramStreams.get(id)?.pollState);
		case SOCKET_DATAGRAM_STREAM_DISPOSE: return socketDatagramStreamDispose(id);
		case OUTPUT_STREAM_BLOCKING_FLUSH | STDOUT:
		case OUTPUT_STREAM_BLOCKING_FLUSH | STDERR: return;
		case OUTPUT_STREAM_DISPOSE | STDOUT:
		case OUTPUT_STREAM_DISPOSE | STDERR: return;
		case INPUT_STREAM_CREATE | STDIN: return createReadableStream(new Readable({ read(n) {
			if (n <= 0) {
				this.push(null);
				return;
			}
			let buf = Buffer.allocUnsafeSlow(n);
			read(0, buf, 0, n, null, (err, bytesRead) => {
				if (err) {
					if (err.code === "EAGAIN") {
						nextTick(() => void this._read(n));
						return;
					}
					this.destroy(err);
				} else if (bytesRead > 0) {
					if (bytesRead !== buf.length) {
						const dst = Buffer.allocUnsafeSlow(bytesRead);
						buf.copy(dst, 0, 0, bytesRead);
						buf = dst;
					}
					this.push(buf);
				} else this.push(null);
			});
		} }));
		case CLOCKS_DURATION_SUBSCRIBE: payload = hrtime.bigint() + payload;
		case CLOCKS_INSTANT_SUBSCRIBE: {
			const pollState = {
				ready: false,
				listener: null,
				polls: [],
				parentStream: null
			};
			subscribeInstant(pollState, payload);
			return createPoll(pollState);
		}
		case FILESYSTEM_DESCRIPTOR_OPEN: return openFileResource(payload.path, payload.flags);
		case FILESYSTEM_DESCRIPTOR_CLOSE: return releaseFileResource(id);
		case FILESYSTEM_DESCRIPTOR_ADVISE: return adviseFileResource(id, payload.offset, payload.length, payload.advice);
		case FILESYSTEM_DESCRIPTOR_SYNC_DATA: return syncFileResourceData(id);
		case FILESYSTEM_DESCRIPTOR_GET_TYPE: return getFileResourceType(id);
		case FILESYSTEM_DESCRIPTOR_SET_SIZE: return setFileResourceSize(id, payload);
		case FILESYSTEM_DESCRIPTOR_SET_TIMES: return setFileResourceTimes(id, payload.atime, payload.mtime);
		case FILESYSTEM_DESCRIPTOR_READ: return readFileResource(id, payload.length, payload.offset);
		case FILESYSTEM_DESCRIPTOR_WRITE: return writeFileResource(id, payload.buffer, payload.offset);
		case FILESYSTEM_DESCRIPTOR_SYNC: return syncFileResource(id);
		case FILESYSTEM_DESCRIPTOR_STAT: return statFileResource(id);
		case FILESYSTEM_DESCRIPTOR_METADATA_HASH: return metadataHashFileResource(id);
		case INPUT_STREAM_CREATE | FILE: {
			const { descriptorId, offset } = payload;
			const fd = fileResourceFd(descriptorId);
			const streamId = createReadableStream(createReadStream(null, {
				fd,
				autoClose: false,
				highWaterMark: 65536,
				start: Number(offset)
			}));
			streams.get(streamId).fileResourceId = retainFileResource(descriptorId);
			return streamId;
		}
		case OUTPUT_STREAM_CREATE | FILE: {
			const { descriptorId, offset } = payload;
			const fd = fileResourceFd(descriptorId);
			const streamId = createWritableStream(createWriteStream(null, {
				fd,
				autoClose: false,
				emitClose: false,
				highWaterMark: 65536,
				start: Number(offset)
			}));
			streams.get(streamId).fileResourceId = retainFileResource(descriptorId);
			return streamId;
		}
	}
	switch (call & CALL_MASK) {
		case INPUT_STREAM_READ: {
			const stream = getStreamOrThrow(id);
			if (!stream.pollState.ready) return /* @__PURE__ */ new Uint8Array();
			const res = stream.stream.read(Math.min(stream.stream.readableLength, Number(payload)));
			if (res) return res;
			if (stream.stream.readableEnded) throw { tag: "closed" };
			return /* @__PURE__ */ new Uint8Array();
		}
		case INPUT_STREAM_BLOCKING_READ: {
			const { pollState } = streams.get(id);
			pollStateCheck(pollState);
			if (pollState.ready) return handle(INPUT_STREAM_READ | call & CALL_TYPE_MASK, id, payload);
			return new Promise((resolve) => void (pollState.listener = resolve)).then(() => handle(INPUT_STREAM_READ | call & CALL_TYPE_MASK, id, payload));
		}
		case INPUT_STREAM_SKIP: return handle(INPUT_STREAM_READ | call & CALL_TYPE_MASK, id, new Uint8Array(Number(payload)));
		case INPUT_STREAM_BLOCKING_SKIP: return handle(INPUT_STREAM_BLOCKING_READ | call & CALL_TYPE_MASK, id, new Uint8Array(Number(payload)));
		case INPUT_STREAM_SUBSCRIBE: return createPoll(streams.get(id).pollState);
		case INPUT_STREAM_DISPOSE: {
			const stream = streams.get(id);
			verifyPollsDroppedForDrop(stream.pollState, "input stream");
			streams.delete(id);
			if (stream.fileResourceId) return releaseFileResource(stream.fileResourceId);
			return;
		}
		case OUTPUT_STREAM_CHECK_WRITE: {
			const { stream, pollState } = getStreamOrThrow(id);
			const bytes = stream.writableHighWaterMark - stream.writableLength;
			if (bytes === 0) pollState.ready = false;
			return BigInt(bytes);
		}
		case OUTPUT_STREAM_WRITE: {
			const { stream } = getStreamOrThrow(id);
			if (payload.byteLength > stream.writableHighWaterMark - stream.writableLength) throw new Error("wasi-io trap: attempt to write too many bytes");
			stream.write(payload);
			return;
		}
		case OUTPUT_STREAM_BLOCKING_WRITE_AND_FLUSH: {
			const stream = getStreamOrThrow(id);
			if (stream.flushPromise) return stream.flushPromise.then(() => handle(call, id, payload));
			if (payload.byteLength > stream.stream.writableHighWaterMark - stream.stream.writableLength) throw new Error("wasi-io trap: Cannot write more than permitted writable length");
			stream.pollState.ready = false;
			return stream.flushPromise = new Promise((resolve, reject) => {
				if (stream.stream === stdout || stream.stream === stderr) {
					stream.stream.write(payload);
					stream.flushPromise = null;
					pollStateReady(stream.pollState);
					resolve(BigInt(payload.byteLength));
				} else stream.stream.write(payload, (err) => {
					stream.flushPromise = null;
					pollStateReady(stream.pollState);
					if (err) {
						reject(streamError(err));
						return;
					}
					resolve(BigInt(payload.byteLength));
				});
			});
		}
		case OUTPUT_STREAM_FLUSH: {
			const stream = getStreamOrThrow(id);
			if (stream.flushPromise) return;
			stream.pollState.ready = false;
			stream.flushPromise = new Promise((resolve, reject) => {
				if (stream.stream === stdout || stream.stream === stderr) {
					stream.flushPromise = null;
					pollStateReady(stream.pollState);
					resolve();
				} else stream.stream.write(new Uint8Array([]), (err) => {
					stream.flushPromise = null;
					pollStateReady(stream.pollState);
					if (err) {
						reject(streamError(err));
						return;
					}
					resolve();
				});
			});
			return stream.flushPromise;
		}
		case OUTPUT_STREAM_BLOCKING_FLUSH: {
			const stream = getStreamOrThrow(id);
			if (stream.flushPromise) return stream.flushPromise;
			return new Promise((resolve, reject) => {
				if (stream.stream === stdout || stream.stream === stderr) resolve();
				else stream.stream.write(new Uint8Array([]), (err) => err ? reject(streamError(err)) : resolve());
			});
		}
		case OUTPUT_STREAM_WRITE_ZEROES: return handle(OUTPUT_STREAM_WRITE | call & CALL_TYPE_MASK, id, new Uint8Array(Number(payload)));
		case OUTPUT_STREAM_BLOCKING_WRITE_ZEROES_AND_FLUSH: return handle(OUTPUT_STREAM_BLOCKING_WRITE_AND_FLUSH | call & CALL_TYPE_MASK, id, new Uint8Array(Number(payload)));
		case OUTPUT_STREAM_SPLICE: {
			const outputStream = getStreamOrThrow(id);
			const inputStream = getStreamOrThrow(payload.src);
			let bytesRemaining = Number(payload.len);
			let chunk;
			while (bytesRemaining > 0 && (chunk = inputStream.stream.read(Math.min(outputStream.writableHighWaterMark - outputStream.writableLength, bytesRemaining)))) {
				bytesRemaining -= chunk.byteLength;
				outputStream.stream.write(chunk);
			}
			if (inputStream.stream.errored) throw streamError(inputStream.stream.errored);
			if (outputStream.stream.errored) throw streamError(outputStream.stream.errored);
			return payload.len - BigInt(bytesRemaining);
		}
		case OUTPUT_STREAM_SUBSCRIBE: return createPoll(streams.get(id).pollState);
		case OUTPUT_STREAM_BLOCKING_SPLICE: {
			const outputStream = getStreamOrThrow(id);
			let promise = Promise.resolve();
			let resolve, reject;
			if (outputStream.stream.writableNeedDrain) promise = new Promise((_resolve, _reject) => {
				outputStream.stream.once("drain", resolve = _resolve).once("error", reject = _reject);
			}).then(() => {
				outputStream.stream.off("error", reject);
			}, (err) => {
				outputStream.stream.off("drain", resolve);
				throw streamError(err);
			});
			const inputStream = getStreamOrThrow(payload.src);
			if (!inputStream.stream.readable) promise = promise.then(() => new Promise((_resolve, _reject) => {
				inputStream.stream.once("readable", resolve = _resolve).once("error", reject = _reject);
			}).then(() => {
				inputStream.stream.off("error", reject);
			}, (err) => {
				inputStream.stream.off("readable", resolve);
				throw streamError(err);
			}));
			return promise.then(() => handle(OUTPUT_STREAM_SPLICE, id, payload));
		}
		case OUTPUT_STREAM_DISPOSE: {
			const stream = streams.get(id);
			verifyPollsDroppedForDrop(stream.pollState, "output stream");
			streams.delete(id);
			if (stream.fileResourceId) return new Promise((resolve) => stream.stream.end(resolve)).then(() => releaseFileResource(stream.fileResourceId));
			else {
				stream.stream.end();
				return;
			}
		}
		case POLL_POLLABLE_READY: return polls.get(id).ready;
		case POLL_POLLABLE_BLOCK: payload = [id];
		case POLL_POLL_LIST: {
			if (payload.length === 0) throw new Error("wasi-io trap: attempt to poll on empty list");
			const doneList = [];
			const pollList = payload.map((pollId) => polls.get(pollId));
			for (const [idx, pollState] of pollList.entries()) {
				pollStateCheck(pollState);
				if (pollState.ready) doneList.push(idx);
			}
			if (doneList.length > 0) return new Uint32Array(doneList);
			let readyPromiseResolve;
			const readyPromise = new Promise((resolve) => void (readyPromiseResolve = resolve));
			for (const poll of pollList) poll.listener = readyPromiseResolve;
			return readyPromise.then(() => {
				for (const [idx, pollState] of pollList.entries()) {
					pollState.listener = null;
					if (pollState.ready) doneList.push(idx);
				}
				return new Uint32Array(doneList);
			});
		}
		case POLL_POLLABLE_DISPOSE:
			if (!polls.delete(id)) throw new Error(`wasi-io trap: Disposed a poll ${id} that does not exist`);
			return;
		case FUTURE_TAKE_VALUE: return futureTakeValue(id);
		case FUTURE_SUBSCRIBE: {
			const { pollState } = futures.get(id);
			const pollId = ++pollCnt;
			polls.set(pollId, pollState);
			return pollId;
		}
		case FUTURE_DISPOSE:
			futureDispose(id, true);
			return;
		default: throw new Error(`wasi-io trap: Unknown call ${call} (${reverseMap[call]}) with type ${reverseMap[call & CALL_TYPE_MASK]}`);
	}
}
/**
* @param {PollState} pollState
*/
function createPoll(pollState) {
	const pollId = ++pollCnt;
	pollState.polls.push(pollId);
	polls.set(pollId, pollState);
	return pollId;
}
function subscribeInstant(pollState, instant) {
	const duration = instant - hrtime.bigint();
	if (duration <= 0) return pollStateReady(pollState);
	function cb() {
		if (hrtime.bigint() < instant) return subscribeInstant(pollState, instant);
		pollStateReady(pollState);
	}
	if (duration < 1e7) setImmediate(cb);
	else setTimeout(cb, Number(duration) / 1e6);
}
/**
* @param {PollState} pollState
* @param {string} polledResourceDebugName
*/
function verifyPollsDroppedForDrop(pollState, polledResourceDebugName) {
	for (const pollId of pollState.polls) if (polls.get(pollId)) throw new Error(`wasi-io trap: Cannot drop ${polledResourceDebugName} as it has a child poll resource which has not yet been dropped`);
}
/**
* @param {PollState} pollState
* @param {bool} finished
*/
function pollStateReady(pollState) {
	if (pollState.ready && pollState.listener) uncaughtException = /* @__PURE__ */ new Error("wasi-io trap: poll already ready with listener attached");
	pollState.ready = true;
	if (pollState.listener) {
		pollState.listener();
		pollState.listener = null;
	}
}
/**
* @param {PollState} pollState
*/
function pollStateCheck(pollState) {
	if (pollState.ready && pollState.parentStream) {
		const stream = pollState.parentStream;
		if (stream.read(0) !== null) throw new Error("wasi-io trap: got data for a null read");
		if (pollState.ready && stream.readableLength === 0 && !stream.readableEnded && !stream.errored) {
			pollState.ready = false;
			stream.once("readable", () => {
				pollStateReady(pollState);
			});
		}
	}
}
/**
*
* @param {Promise<any>} promise
* @param {PollState | undefined} pollState
* @returns {number}
*/
function createFuture(promise, pollState) {
	const futureId = ++futureCnt;
	if (pollState) pollState.ready = false;
	else pollState = {
		ready: false,
		listener: null,
		polls: [],
		parent: null
	};
	const future = {
		tag: "ok",
		val: null
	};
	futures.set(futureId, {
		future,
		pollState
	});
	promise.then((value) => {
		pollStateReady(pollState);
		future.val = value;
	}, (value) => {
		pollStateReady(pollState);
		future.tag = "err";
		future.val = value;
	});
	return futureId;
}
/**
* @param {number} id
* @returns {undefined | { tag: 'ok', val: any } | { tag: 'err', val: undefined }}
* @throws {undefined}
*/
function futureTakeValue(id) {
	const future = futures.get(id);
	if (!future.pollState.ready) return;
	if (!future.future) return {
		tag: "err",
		val: void 0
	};
	const out = {
		tag: "ok",
		val: future.future
	};
	future.future = null;
	return out;
}
function futureDispose(id, ownsState) {
	const { pollState } = futures.get(id);
	if (ownsState) verifyPollsDroppedForDrop(pollState, "future");
	futures.delete(id);
}
let uncaughtException;
process.on("uncaughtException", (err) => uncaughtException = err);
const debug = runAsWorker(handle);
//#endregion
export { createFuture, createReadableStream, createReadableStreamPollState, createWritableStream, futureDispose, futureTakeValue, futures, getStreamOrThrow, log, pollStateReady, polls, streams, verifyPollsDroppedForDrop };
