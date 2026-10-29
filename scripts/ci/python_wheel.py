"""Make an installable Python Component host package without build hooks.

The first Python profile supports synchronous, freestanding exports and no
host imports. Unsupported resources, async values and WASI fail during build.
"""

import base64
import csv
import hashlib
import io
import keyword
import re
import zipfile

from common import fail, json_bytes, registry_identity

RUNTIME = '''"""Python bindings for a Wisp WebAssembly Component."""
import threading
from importlib.resources import files
from wasmtime import Engine, Store
from wasmtime.component import Component, Linker

class Client:
    def __init__(self):
        self._engine = Engine()
        self._store = Store(self._engine)
        self._component = Component(self._engine, files(__package__).joinpath("component.wasm").read_bytes())
        self._instance = Linker(self._engine).instantiate(self._store, self._component)
        self._lock = threading.Lock()
        self._poisoned = False

    def call(self, interface, function, *args):
        with self._lock:
            if self._poisoned:
                raise RuntimeError("Component trapped; create a new Client")
            if interface is None:
                export = function
            else:
                parent = self._instance.get_export_index(self._store, interface)
                if parent is None:
                    raise KeyError(interface)
                export = self._instance.get_export_index(self._store, function, parent)
            if export is None:
                raise KeyError(function)
            func = self._instance.get_func(self._store, export)
            if func is None:
                raise KeyError(function)
            try:
                value = func(self._store, *args)
                func.post_return(self._store)
                return value
            except Exception:
                self._poisoned = True
                raise
'''


def identifier(value):
    result = re.sub(r"[^a-zA-Z0-9_]", "_", value)
    if not result.isidentifier() or keyword.iskeyword(result):
        fail(f"Python export name is unsupported: {value}")
    return result


def validate_type(ty):
    if ty is None:
        return
    if isinstance(ty, str):
        if ty in {"resource", "error-context"}:
            fail(f"Python profile does not support {ty}")
        return
    if isinstance(ty, list):
        for child in ty:
            validate_type(child)
        return
    if isinstance(ty, dict):
        if set(ty) & {"own", "borrow", "future", "stream", "fixed-length-list", "map"}:
            fail("Python profile does not support resources, async values, fixed lists, or maps")
        for value in ty.values():
            validate_type(value)


def annotation(ty):
    if ty is None:
        return "None"
    if isinstance(ty, str):
        if ty in {"bool"}:
            return "bool"
        if ty in {"string", "char"}:
            return "str"
        if re.fullmatch(r"[us](8|16|32|64)", ty):
            return "int"
        if ty in {"f32", "f64"}:
            return "float"
    return "object"


def generate_source(contract):
    if contract.get("schema") != 1 or contract.get("imports"):
        fail("PyPI currently requires an import-free Component; WASI host bindings are not supported yet")
    result = RUNTIME
    used = {"call", "__init__"}
    exports = contract.get("exports", [])
    if not exports:
        fail("Python package requires an exported function")
    for interface in exports:
        for ty in interface.get("types", []):
            validate_type(ty["type"])
        world = interface["name"] == contract["world"] + "-world" and interface["package"] == contract["package"]
        package, _, version = interface["package"].partition("@")
        export = None if world else package + "/" + interface["name"] + ("@" + version if version else "")
        for function in interface["functions"]:
            if function["kind"] != "freestanding" or function["async"] != "sync":
                fail("Python profile requires synchronous freestanding functions")
            for _, ty in function["params"]:
                validate_type(ty)
            validate_type(function["result"])
            name = identifier(function["name"] if len(exports) == 1 else interface["name"] + "_" + function["name"])
            if name in used:
                fail("Python export names collide")
            used.add(name)
            params = []
            names = {"self"}
            for parameter, ty in function["params"]:
                parameter = identifier(parameter)
                if parameter in names:
                    fail("Python parameter names collide")
                names.add(parameter)
                params.append((parameter, annotation(ty)))
            signature = "".join(f", {parameter}: {ty}" for parameter, ty in params)
            arguments = "".join(", " + parameter for parameter, _ in params)
            result += f"\n    def {name}(self{signature}) -> {annotation(function['result'])}:\n"
            result += f"        return self.call({export!r}, {function['itemName']!r}{arguments})\n"
    compile(result, "<wisp-python-bindings>", "exec")
    return result.encode()


def wheel_filename(name, version):
    # Require stable SemVer. A prerelease needs explicit PEP 440 translation
    # rather than silently changing an immutable registry version.
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        fail("PyPI currently requires a stable MAJOR.MINOR.PATCH package version")
    # The distribution and import names both come from the owner's PyPI name.
    module = registry_identity("pypi", name).replace("-", "_")
    if not module.isidentifier() or keyword.iskeyword(module):
        fail("the PyPI name must normalize to a Python import name")
    return f"{module}-{version}-py3-none-any.whl"


def make_wheel(name, version, release, component, destination, notices=None, url=None):
    filename = wheel_filename(name, version)
    module = filename.split("-", 1)[0]
    info = f"{module}-{version}.dist-info"
    metadata = release.get("metadata", {})
    description = metadata.get("description") or f"Wisp Component package {name}"
    description = str(description).replace("\r", " ").replace("\n", " ")
    project_urls = f"Project-URL: Verification, {url}\n" if url else ""
    files = {
        f"{module}/__init__.py": generate_source(release["contract"]),
        f"{module}/component.wasm": component,
        f"{module}/library-release.json": json_bytes(release),
        f"{module}/py.typed": b"",
        f"{info}/METADATA": (f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}\nSummary: {description}\n{project_urls}Requires-Python: >=3.9\nRequires-Dist: wasmtime==48.0.0\n\n").encode(),
        f"{info}/WHEEL": b"Wheel-Version: 1.0\nGenerator: wisp\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
    }
    for filename, data in (notices or {}).items():
        files[f"{info}/licenses/{filename}"] = data
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for path, data in sorted(files.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        writer.writerow([path, "sha256=" + digest, len(data)])
    writer.writerow([f"{info}/RECORD", "", ""])
    files[f"{info}/RECORD"] = record.getvalue().encode()
    target = destination / filename
    with zipfile.ZipFile(target, "x", zipfile.ZIP_DEFLATED) as archive:
        for path, data in sorted(files.items()):
            entry = zipfile.ZipInfo(path, (1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = 0o100644 << 16
            archive.writestr(entry, data)
    return target
