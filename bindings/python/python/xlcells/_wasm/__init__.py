"""xlcells as WebAssembly: one module (`xlcells.wasm`) for every processor and system.

The module holds what is otherwise built for each machine (reading and writing
workbooks, the formula engine, regions, stores, the MCP server). `wasmtime` runs it;
nothing is compiled on the machine that uses it. What the module cannot do itself,
this side does for it:

* files: the module sees the machine's files (a drive ``C:`` as ``/c``); paths go
  in and come out as this machine writes them, the module turns them around;
* Polars: the engine's plans run on the Polars installed here (`xlcells._polars_shim`),
  which also reads and writes the Parquet files of stores.

`call(name, args)` is one call of the module; `slim` and `engine` give it the shape of
the native modules it stands in for (`xlcells._xlcells`, `xlcells_engine._engine`).
Messages (see `bindings/wasm/src/rpc.rs`) are JSON with byte strings attached: bytes
stand as ``{"$b": i}``, dataframes as ``{"$f": i}`` (an Arrow IPC stream).
"""
import io
import json
import os
import struct
import threading
from pathlib import Path

FILE = Path(__file__).with_name("xlcells.wasm")


class EngineError(RuntimeError):
    """The module stopped (a fault in it); it is started anew for the next call, and
    objects held in it (workbooks read, writers) are gone."""


def _polars():
    try:
        import polars
    except ImportError:
        raise ImportError("xlcells needs the polars package for this (uv pip install polars)") from None
    return polars


def _to_ipc(frame):
    pl = _polars()
    from .._arrow_compat import prepare

    if not isinstance(frame, pl.DataFrame):
        frame = pl.DataFrame(frame)
    buf = io.BytesIO()
    prepare(frame).write_ipc_stream(buf)
    return buf.getvalue()


def _from_ipc(data):
    return _polars().read_ipc_stream(io.BytesIO(data))


def encode(value, blobs=None):
    """A message: `value` as JSON, bytes and dataframes in it attached."""
    blobs = [] if blobs is None else blobs

    def walk(v):
        if isinstance(v, (bytes, bytearray, memoryview)):
            blobs.append(bytes(v))
            return {"$b": len(blobs) - 1}
        if isinstance(v, (list, tuple)):
            return [walk(x) for x in v]
        if isinstance(v, dict):
            return {k: walk(x) for k, x in v.items()}
        if isinstance(v, os.PathLike):
            return os.fspath(v)
        if hasattr(v, "__arrow_c_stream__") or type(v).__module__.split(".")[0] == "polars":
            blobs.append(_to_ipc(v))
            return {"$f": len(blobs) - 1}
        return v

    text = json.dumps(walk(value), allow_nan=False).encode()
    parts = [struct.pack("<I", len(text)), text, struct.pack("<I", len(blobs))]
    for b in blobs:
        parts += [struct.pack("<I", len(b)), b]
    return b"".join(parts)


def decode(data):
    """A message as (JSON with attachments in place, the raw attachments)."""
    n = struct.unpack_from("<I", data, 0)[0]
    value = json.loads(bytes(data[4 : 4 + n]))
    at = 4 + n
    count = struct.unpack_from("<I", data, at)[0]
    at += 4
    blobs = []
    for _ in range(count):
        size = struct.unpack_from("<I", data, at)[0]
        blobs.append(bytes(data[at + 4 : at + 4 + size]))
        at += 4 + size

    def walk(v):
        if isinstance(v, list):
            return [walk(x) for x in v]
        if isinstance(v, dict):
            if len(v) == 1 and "$b" in v:
                return blobs[v["$b"]]
            if len(v) == 1 and "$f" in v:
                return _from_ipc(blobs[v["$f"]])
            return {k: walk(x) for k, x in v.items()}
        return v

    return walk(value), blobs


def _drives():
    """Where the module sees this machine's files: (directory here, name there)."""
    if os.name != "nt":
        return [("/", "/")]
    letters = [d[0] for d in os.listdrives()] if hasattr(os, "listdrives") else [c for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ" if os.path.exists(c + ":\\")]
    return [(c.upper() + ":\\", "/" + c.lower()) for c in letters]


def _host(name, args, blobs):
    """What the module asks of this side: Polars and files."""
    from .. import _polars_shim as shim

    if name == "run_plan":
        key, plan = args
        return {"ok": None}, [_to_ipc(shim.run(key, plan, _from_ipc(blobs[0])))]
    if name == "write_parquet":
        path, key, note = args
        shim.write_parquet(path, _from_ipc(blobs[0]), key, note)
        return {"ok": None}, []
    if name == "read_parquet":
        return {"ok": None}, [_to_ipc(shim.read_parquet(args[0]))]
    if name == "read_note":
        return {"ok": shim.read_note(args[0], args[1])}, []
    if name == "check_port":
        from .._port_check import differing

        return {"ok": differing(args[0])}, []
    if name == "polars_version":
        return {"ok": shim.version()}, []
    raise ValueError("the module asked for {0!r}, which this side does not do".format(name))


class Runtime:
    """The module, started: one instance, one call at a time."""

    #: Why the module runs in the interpreter instead of compiled, if it does.
    fallback = None

    def _compile(self, target=None):
        """The engine and the module, compiled for this machine or (`target`
        ``pulley64``) for wasmtime's interpreter."""
        wasmtime = self.wasmtime
        config = wasmtime.Config()
        # Compiled once per version of the module, then taken from the cache.
        try:
            config.cache = True
        except Exception:
            pass
        if target:
            config.target = target
        self.target = target or "native"
        self.engine = wasmtime.Engine(config)
        self.module = wasmtime.Module.from_file(self.engine, str(FILE))

    def __init__(self):
        try:
            import wasmtime
        except ImportError:
            raise ImportError("xlcells needs the wasmtime package to run its engine (uv pip install wasmtime)") from None
        self.wasmtime = wasmtime
        target = os.environ.get("XLCELLS_WASM_TARGET")
        try:
            self._compile(target)
        except Exception as e:
            if target:
                raise
            # A machine that allows no code made at run time (a policy against
            # executable memory): the module runs in wasmtime's interpreter, slower.
            self._compile("pulley64")
            self.fallback = "{0}: {1}".format(type(e).__name__, str(e).splitlines()[0] if str(e) else "")
        self.lock = threading.RLock()
        #: Objects of the module no longer used here, to be dropped before the next call
        #: (not at once: that may be in the middle of a call).
        self.dropped = []
        #: Counts the starts of the module: objects of an earlier start are gone.
        self.generation = 0
        self._start()

    def _start(self):
        wt = self.wasmtime
        linker = wt.Linker(self.engine)
        linker.define_wasi()
        i32 = wt.ValType.i32()
        linker.define_func("xl", "host_call", wt.FuncType([i32, i32], [i32]), self._host_call, access_caller=True)
        linker.define_func("xl", "host_take", wt.FuncType([i32], []), self._host_take, access_caller=True)
        store = wt.Store(self.engine)
        wasi = wt.WasiConfig()
        wasi.inherit_stderr()
        for here, there in _drives():
            try:
                wasi.preopen_dir(here, there)
            except Exception:  # a drive without a disc, a network drive not there
                pass
        store.set_wasi(wasi)
        instance = linker.instantiate(store, self.module)
        exports = instance.exports(store)
        self.store = store
        self.memory = exports["memory"]
        self._alloc, self._free, self._xl_call = exports["xl_alloc"], exports["xl_free"], exports["xl_call"]
        try:
            exports["_initialize"](store)
        except KeyError:
            pass
        self._pending = b""
        self.generation += 1
        self.call("init", [os.name == "nt", os.getcwd()])

    def _host_call(self, caller, ptr, length):
        data = self.memory.read(caller, ptr, ptr + length)
        try:
            request, blobs = decode(data)
            answer, out = _host(request["fn"], request["args"], [b for b in blobs])
        except Exception as e:  # the module is told; it fails the call it is in
            answer, out = {"err": "{0}: {1}".format(type(e).__name__, e)}, []
        self._pending = encode(answer, list(out))
        return len(self._pending)

    def _host_take(self, caller, ptr):
        self.memory.write(caller, self._pending, ptr)
        self._pending = b""

    def call(self, name, args=()):
        """One call of the module; its answer, or the error it gave as ValueError."""
        with self.lock:
            while self.dropped:
                number = self.dropped.pop()
                self.call("drop", [number])
            message = encode({"fn": name, "args": list(args)})
            try:
                ptr = self._alloc(self.store, len(message))
                self.memory.write(self.store, message, ptr)
                packed = self._xl_call(self.store, ptr, len(message))
            except (self.wasmtime.Trap, self.wasmtime.WasmtimeError) as e:
                self._start()
                raise EngineError("the engine stopped in {0} ({1}); it was started anew".format(name, str(e).splitlines()[0])) from None
            at, size = packed >> 32, packed & 0xFFFFFFFF
            data = self.memory.read(self.store, at, at + size)
            self._free(self.store, at, size)
        answer, _ = decode(data)
        if "err" in answer:
            raise ValueError(answer["err"])
        return answer["ok"]


_runtime = None
_made = threading.Lock()


def runtime():
    """The module, started once per process."""
    global _runtime
    if _runtime is None:
        with _made:
            if _runtime is None:
                _runtime = Runtime()
    return _runtime


def call(name, *args):
    return runtime().call(name, args)


def tuples(v):
    """Lists inside lists as tuples, as the native modules give them."""
    if isinstance(v, list):
        return tuple(tuples(x) for x in v)
    if isinstance(v, dict):
        return {k: tuples(x) for k, x in v.items()}
    return v


def outer(v):
    """A list whose items are tuples (see `tuples`)."""
    return [tuples(x) for x in v] if isinstance(v, list) else v


class Held:
    """An object held in the module under a number (a writer, a workbook read)."""

    kind = ""

    def __init__(self, *args):
        rt = runtime()
        self._generation = rt.generation
        self._number = rt.call(self.kind + "_new", args)

    def _call(self, *args):
        rt = runtime()
        if rt.generation != self._generation:
            raise EngineError("this object was held by the engine before it was started anew; make it again")
        return rt.call(self.kind + "_call", (self._number,) + args)

    def __del__(self):
        rt = _runtime
        if rt is not None and rt.generation == getattr(self, "_generation", None):
            rt.dropped.append(self._number)
