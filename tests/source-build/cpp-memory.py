#!/usr/bin/env python3
"""Exercise a freestanding C++ canonical allocator through an actual Wasm engine."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sys


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def verify(core_path, python_host=None):
    if python_host:
        sys.path.insert(0, str(Path(python_host).resolve(strict=True)))
    import wasmtime

    binary = Path(core_path).read_bytes()
    engine = wasmtime.Engine()
    module = wasmtime.Module(engine, binary)
    store = wasmtime.Store(engine)
    linker = wasmtime.Linker(engine)
    linker.define_wasi()
    instance = linker.instantiate(store, module)
    exports = instance.exports(store)
    allocate, memory = exports["cabi_realloc"], exports["memory"]
    initial_pages = memory.size(store)

    # Increasing temporary inputs must retain memory in proportion to peak live
    # bytes, rather than the sum of all historical allocation sizes.
    for iteration in range(1, 4097):
        size = iteration * 31
        pointer = allocate(store, 0, 0, 1, size)
        require(memory.read(store, pointer, pointer + min(size, 32)) == bytes(min(size, 32)), "allocation is not zeroed")
        allocate(store, pointer, size, 1, 0)
    require(memory.size(store) <= initial_pages + 3, "increasing temporary allocations retain unused heap chunks")

    # A retained tail allocation keeps the large freed chunk inside the heap.
    # Split it for small values, then merge arbitrarily freed pieces back into
    # a block large enough to satisfy the original request.
    large = allocate(store, 0, 0, 8, 65536)
    pinned = allocate(store, 0, 0, 8, 64)
    sentinel = bytes(range(64))
    memory.write(store, sentinel, pinned)
    allocate(store, large, 65536, 8, 0)
    pieces = [allocate(store, 0, 0, 8, 32) for _ in range(64)]
    require(all(large <= pointer < large + 65536 for pointer in pieces), "small requests did not split the freed large block")
    for pointer in pieces[::2] + pieces[1::2]:
        allocate(store, pointer, 32, 8, 0)
    merged = allocate(store, 0, 0, 8, 65536)
    require(merged == large, "adjacent free blocks did not coalesce")
    require(memory.read(store, pinned, pinned + 64) == sentinel, "coalescing overwrote a live allocation")
    allocate(store, merged, 65536, 8, 0)
    allocate(store, pinned, 64, 8, 0)

    # Releasing a trailing allocation also releases the alignment padding
    # preceding it. Otherwise page-aligned requests consume a page each time.
    aligned = []
    for _ in range(128):
        pointer = allocate(store, 0, 0, 65536, 32)
        require(pointer % 65536 == 0, "page-aligned request is misaligned")
        aligned.append(pointer)
        allocate(store, pointer, 32, 65536, 0)
    require(len(set(aligned)) == 1, "tail reclamation retains alignment padding")

    for iteration in range(1000):
        alignment = 1 << (iteration % 17)
        size = (iteration * 31) % 2048 + 1
        pointer = allocate(store, 0, 0, alignment, size)
        require(pointer % alignment == 0, "allocation is misaligned")
        data = bytes((index + iteration) % 256 for index in range(size))
        memory.write(store, data, pointer)
        resized_size = size + 17 if iteration % 2 else max(1, size // 2)
        resized = allocate(store, pointer, size, alignment, resized_size)
        require(resized % alignment == 0, "reallocation is misaligned")
        length = min(size, resized_size)
        require(memory.read(store, resized, resized + length) == data[:length], "reallocation changed owned bytes")
        require(allocate(store, resized, resized_size, alignment, 0) == alignment, "free returned an invalid empty pointer")
    return {
        "passed": True,
        "coreSha256": hashlib.sha256(binary).hexdigest(),
        "initialPages": initial_pages,
        "peakPages": memory.size(store),
        "increasingTemporaryAllocations": 4096,
        "splitAndCoalescedAllocations": len(pieces),
        "alignedTailAllocations": len(aligned),
        "allocationResizeFreeCycles": 1000,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", required=True, type=Path)
    parser.add_argument("--python-host", default=os.environ.get("WISP_WASMTIME_PYTHON"))
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    result = verify(args.core, args.python_host)
    text = json.dumps(result, indent=2) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
