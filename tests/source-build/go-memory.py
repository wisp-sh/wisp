#!/usr/bin/env python3
"""Check a Go Component's startup arena and later canonical reallocations.

This optional engine check needs the Wasmtime Python API. Select its module
parent with --python-host or WISP_WASMTIME_PYTHON. Normal source acceptance
uses the Component through jco and does not require this extra Python API.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def core_modules(component):
    require(component[:8] == b"\0asm\x0d\0\x01\0", "expected a Component binary")
    offset = 8
    while offset < len(component):
        kind = component[offset]
        offset += 1
        size = shift = 0
        for _ in range(5):
            require(offset < len(component), "truncated Component section length")
            value = component[offset]
            offset += 1
            size |= (value & 127) << shift
            shift += 7
            if value < 128:
                break
        else:
            raise RuntimeError("invalid Component section length")
        require(offset + size <= len(component), "truncated Component section")
        payload = component[offset:offset + size]
        offset += size
        if kind == 1:
            yield payload


def verify(component_path, python_host=None, iterations=1000):
    if python_host:
        sys.path.insert(0, str(Path(python_host).resolve(strict=True)))
    import wasmtime

    component = Path(component_path).read_bytes()
    engine = wasmtime.Engine()
    candidates = []
    required = {"cabi_realloc", "wisp-cabi-realloc", "_initialize", "memory"}
    for binary in core_modules(component):
        module = wasmtime.Module(engine, binary)
        if required <= {entry.name for entry in module.exports}:
            candidates.append((module, binary))
    require(len(candidates) == 1, "expected one Go core with the reserved canonical allocator")
    module, binary = candidates[0]
    store = wasmtime.Store(engine)
    wasi = wasmtime.WasiConfig()
    wasi.argv = ["wisp-go-memory-regression"]
    store.set_wasi(wasi)
    linker = wasmtime.Linker(engine)
    linker.define_wasi()
    instance = linker.instantiate(store, module)
    exports = instance.exports(store)
    allocate, memory = exports["cabi_realloc"], exports["memory"]

    # The adapter makes these two requests before Go's runtime can allocate.
    pages = [allocate(store, 0, 0, 65536, 65536) for _ in range(2)]
    require(all(page % 65536 == 0 for page in pages), "adapter pages are not aligned")
    require(abs(pages[0] - pages[1]) == 65536, "adapter pages overlap or skip storage")
    sentinels = [(page + offset, bytes([index + 1]) * 64)
                 for index, page in enumerate(pages) for offset in (0, 65536 - 64)]
    for address, pattern in sentinels:
        memory.write(store, pattern, address)
    # Canonical hooks must also work before Go initializes. Calling Go here, or
    # during canonical post-return, can enter WASI while the Component forbids it.
    before_init = allocate(store, 0, 0, 16, 64)
    memory.write(store, b"before-runtime", before_init)
    resized = allocate(store, before_init, 64, 16, 128)
    require(memory.read(store, resized, resized + 14) == b"before-runtime", "pre-initialization realloc lost bytes")
    allocate(store, resized, 128, 16, 0)
    exports["_initialize"](store)

    # Later requests use the bounded static canonical pool. They preserve bytes
    # on grow and shrink and release storage without entering Go or WASI.
    for iteration in range(iterations):
        alignment = 1 << (iteration % 5)
        size = (iteration * 31) % 2048 + 1
        pointer = allocate(store, 0, 0, alignment, size)
        require(pointer % alignment == 0, "dynamic allocation is not aligned")
        require(all(pointer + size <= page or pointer >= page + 65536 for page in pages), "dynamic allocation overlaps adapter storage")
        data = bytes((index + iteration) % 256 for index in range(size))
        memory.write(store, data, pointer)
        new_size = size + 17 if iteration % 2 else max(1, size // 2)
        resized = allocate(store, pointer, size, alignment, new_size)
        require(resized % alignment == 0, "resized allocation is not aligned")
        length = min(size, new_size)
        require(memory.read(store, resized, resized + length) == data[:length], "realloc changed owned bytes")
        require(allocate(store, resized, new_size, alignment, 0) == alignment, "free returned an invalid empty pointer")
    try:
        allocate(store, 0, 0, 8, 4 * 1024 * 1024)
    except wasmtime.Trap:
        pass
    else:
        raise RuntimeError("canonical pool exhaustion must trap")
    after_exhaustion = allocate(store, 0, 0, 8, 16)
    allocate(store, after_exhaustion, 16, 8, 0)

    # Exercise Go-produced owned output and its redirected pure post-return.
    echoes = [name for name in exports if name.endswith("#echo") and not name.startswith("cabi_post_")]
    require(len(echoes) == 1, "expected one owned-string echo export")
    import struct
    text = "post-return 🦀".encode()
    for _ in range(iterations):
        parameter = allocate(store, 0, 0, 1, len(text))
        memory.write(store, text, parameter)
        result = exports[echoes[0]](store, parameter, len(text))
        pointer, length = struct.unpack("<II", memory.read(store, result, result + 8))
        require(memory.read(store, pointer, pointer + length) == text, "owned echo changed bytes")
        exports["cabi_post_" + echoes[0]](store, result)
        for released in (result, pointer):
            require(struct.unpack("<I", memory.read(store, released - 4, released))[0] != 0xffffffff,
                    "post-return left an owned buffer allocated")

    checked = [name for name in exports if name.endswith("#checked") and not name.startswith("cabi_post_")]
    require(len(checked) == 1, "expected the Go Packet/error fixture export")
    for iteration in range(iterations):
        success = iteration % 2
        result = exports[checked[0]](store, success)
        tag = memory.read(store, result, result + 1)[0]
        require(tag == (0 if success else 1), "Go result tag changed")
        # Packet's string and byte fields are at payload+0/+8. The error arm
        # contains one string at the same aligned payload offset.
        offsets = (8, 16) if success else (8,)
        released = [result]
        for offset in offsets:
            pointer, length = struct.unpack("<II", memory.read(store, result + offset, result + offset + 8))
            if length:
                released.append(pointer)
        exports["cabi_post_" + checked[0]](store, result)
        for pointer in released:
            require(struct.unpack("<I", memory.read(store, pointer - 4, pointer))[0] != 0xffffffff,
                    "variant post-return left its live arm allocated")
    for address, pattern in sentinels:
        require(memory.read(store, address, address + len(pattern)) == pattern,
                "Go initialization or dynamic allocations overwrote adapter storage")
    return {
        "passed": True,
        "componentSha256": hashlib.sha256(component).hexdigest(),
        "coreSha256": hashlib.sha256(binary).hexdigest(),
        "initialAdapterPages": pages,
        "dynamicAllocationResizeFreeCycles": iterations,
        "purePostReturnCycles": iterations,
        "variantPostReturnCycles": iterations,
        "preInitializationRealloc": True,
        "exhaustionRejected": True,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", required=True, type=Path)
    parser.add_argument("--python-host", default=os.environ.get("WISP_WASMTIME_PYTHON"))
    parser.add_argument("--iterations", type=int, default=1000)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    require(args.iterations > 0, "--iterations must be positive")
    result = verify(args.component, args.python_host, args.iterations)
    text = json.dumps(result, indent=2) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
