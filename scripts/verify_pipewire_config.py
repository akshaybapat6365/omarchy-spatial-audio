#!/usr/bin/env python3
"""Offline PipeWire filter-chain validator for 91-spatial-fdtr.conf.

Uses PipeWire's own `pw-config` SPA-JSON parser/merger (read-only, zero daemon changes)
plus graph topology, port contract, LADSPA dlopen, SOFA load, and WAV header verification.
"""
from __future__ import annotations

import ctypes
import json
import os
import struct
import subprocess
import sys
from collections import defaultdict, deque
from pathlib import Path

SO_PLUGIN_SPA = [
    "/usr/lib/spa-0.2/filter-graph/libspa-filter-graph-plugin-builtin.so",
    "/usr/lib/spa-0.2/filter-graph/libspa-filter-graph-plugin-ladspa.so",
    "/usr/lib/spa-0.2/filter-graph/libspa-filter-graph-plugin-lv2.so",
    "/usr/lib/spa-0.2/filter-graph/libspa-filter-graph-plugin-sofa.so",
]

VALID_PORTS = {
    ("builtin", "copy"): ({"In"}, {"Out"}),
    ("builtin", "convolver"): ({"In"}, {"Out"}),
    ("builtin", "delay"): ({"In"}, {"Out"}),
    ("builtin", "mixer"): ({f"In {i}" for i in range(1, 9)}, {"Out"}),
    ("sofa", "spatializer"): ({"In"}, {"Out L", "Out R"}),
    ("ladspa", "omarchy_spatial_fdtr"): ({"In L", "In R"}, {"Out L", "Out R"}),
    ("lv2", "http://lsp-plug.in/plugins/lv2/filter_stereo"): ({"in_l", "in_r"}, {"out_l", "out_r"}),
    ("lv2", "http://lsp-plug.in/plugins/lv2/sc_mb_compressor_stereo"): ({"in_l", "in_r"}, {"out_l", "out_r"}),
    ("lv2", "http://lsp-plug.in/plugins/lv2/sc_limiter_stereo"): ({"in_l", "in_r"}, {"out_l", "out_r"}),
}


def check_wav(path: str) -> str:
    p = Path(path)
    if not p.is_file():
        raise AssertionError(f"Missing audio file: {path}")
    raw = p.read_bytes()
    if len(raw) < 44 or raw[:4] != b"RIFF" or raw[8:12] != b"WAVE":
        raise AssertionError(f"Invalid RIFF/WAVE header: {path}")
    pos = 12
    fmt_tag = nch = sr = bps = 0
    while pos + 8 <= len(raw):
        cid = raw[pos : pos + 4]
        sz = struct.unpack_from("<I", raw, pos + 4)[0]
        pos += 8
        if cid == b"fmt ":
            fmt_tag, nch, sr, _, _, bps = struct.unpack_from("<HHIIHH", raw, pos)
            break
        pos += sz + (sz & 1)
    assert sr == 48000, f"{path}: expected 48000 Hz, got {sr}"
    return f"{p.name} (fmt={fmt_tag}, ch={nch}, sr={sr}, {bps}-bit)"


def check_sofa(path: str) -> str:
    p = Path(path)
    if not p.is_file():
        raise AssertionError(f"Missing SOFA file: {path}")
    lib = ctypes.CDLL("libmysofa.so.1")
    lib.mysofa_open.argtypes = [
        ctypes.c_char_p,
        ctypes.c_float,
        ctypes.POINTER(ctypes.c_int),
        ctypes.POINTER(ctypes.c_int),
    ]
    lib.mysofa_open.restype = ctypes.c_void_p
    lib.mysofa_close.argtypes = [ctypes.c_void_p]
    flt_len = ctypes.c_int(0)
    err = ctypes.c_int(0)
    h = lib.mysofa_open(path.encode(), 48000.0, ctypes.byref(flt_len), ctypes.byref(err))
    if not h or err.value != 0:
        raise AssertionError(f"libmysofa failed to open {path}: err={err.value}")
    lib.mysofa_close(h)
    return f"{p.name} (filter_len={flt_len.value} @ 48 kHz)"


def check_ladspa(path: str, expected_label: str) -> str:
    p = Path(path)
    if not p.is_file():
        raise AssertionError(f"Missing LADSPA .so: {path}")
    lib = ctypes.CDLL(path)
    fn = lib.ladspa_descriptor
    fn.argtypes = [ctypes.c_ulong]
    fn.restype = ctypes.c_void_p
    ptr = fn(0)
    if not ptr:
        raise AssertionError(f"ladspa_descriptor(0) returned NULL in {path}")
    # Offset 8 (after unsigned long UniqueID) is const char *Label
    label_ptr = ctypes.c_char_p.from_address(ptr + ctypes.sizeof(ctypes.c_ulong)).value
    assert label_ptr is not None and label_ptr.decode() == expected_label, (
        f"Expected label {expected_label!r}, got {label_ptr!r}"
    )
    return f"{p.name} (label={expected_label})"


def validate_graph(sink_name: str, args: dict) -> dict:
    fg = args["filter.graph"]
    nodes = fg["nodes"]
    links = fg["links"]
    inputs = fg["inputs"]
    outputs = fg["outputs"]

    node_map: dict[str, dict] = {}
    checked_assets: list[str] = []

    for nd in nodes:
        name = nd["name"]
        assert name not in node_map, f"{sink_name}: duplicate node name {name}"
        node_map[name] = nd
        ntype = nd["type"]
        if ntype == "builtin":
            key = ("builtin", nd["label"])
            if nd["label"] == "convolver":
                fn = nd["config"]["filename"]
                checked_assets.append(check_wav(fn))
        elif ntype == "sofa":
            key = ("sofa", nd["label"])
            checked_assets.append(check_sofa(nd["config"]["filename"]))
        elif ntype == "ladspa":
            key = ("ladspa", nd["label"])
            checked_assets.append(check_ladspa(nd["plugin"], nd["label"]))
        elif ntype == "lv2":
            key = ("lv2", nd["plugin"])
        else:
            raise AssertionError(f"{sink_name}: unknown node type {ntype}")
        assert key in VALID_PORTS, f"{sink_name}: unverified plugin signature {key}"

    def port_sets(node_name: str) -> tuple[set[str], set[str]]:
        nd = node_map[node_name]
        ntype = nd["type"]
        key = (ntype, nd["plugin"] if ntype == "lv2" else nd["label"])
        return VALID_PORTS[key]

    adj: dict[str, list[str]] = defaultdict(list)
    indeg: dict[str, int] = {n: 0 for n in node_map}

    for link in links:
        src_node, src_port = link["output"].split(":", 1)
        dst_node, dst_port = link["input"].split(":", 1)
        assert src_node in node_map, f"{sink_name}: missing link src node {src_node}"
        assert dst_node in node_map, f"{sink_name}: missing link dst node {dst_node}"
        _, valid_out = port_sets(src_node)
        valid_in, _ = port_sets(dst_node)
        assert src_port in valid_out, f"{sink_name}: invalid output port {link['output']}"
        assert dst_port in valid_in, f"{sink_name}: invalid input port {link['input']}"
        adj[src_node].append(dst_node)
        indeg[dst_node] += 1

    for ep in inputs:
        n, p = ep.split(":", 1)
        assert n in node_map and p in port_sets(n)[0], f"{sink_name}: invalid graph input {ep}"
    for ep in outputs:
        n, p = ep.split(":", 1)
        assert n in node_map and p in port_sets(n)[1], f"{sink_name}: invalid graph output {ep}"

    # Kahn topological sort -> prove DAG (zero feedback cycles)
    q = deque(n for n, d in indeg.items() if d == 0)
    visited = 0
    while q:
        u = q.popleft()
        visited += 1
        for v in adj[u]:
            indeg[v] -= 1
            if indeg[v] == 0:
                q.append(v)
    assert visited == len(node_map), f"{sink_name}: cycle detected in filter graph!"

    pb = args["playback.props"]
    assert pb.get("node.passive") is True, f"{sink_name}: playback.props.node.passive must be true"
    target = pb.get("target.object", "")
    assert isinstance(target, str) and len(target) > 0 and not target.startswith("@"), (
        f"{sink_name}: invalid playback.props.target.object={target!r}"
    )

    return {
        "sink": sink_name,
        "nodes": len(node_map),
        "links": len(links),
        "inputs": len(inputs),
        "outputs": len(outputs),
        "dag_acyclic": True,
        "unique_assets": sorted(set(checked_assets)),
    }


def main() -> int:
    for spa in SO_PLUGIN_SPA:
        if not os.path.isfile(spa):
            print(f"FAIL: missing SPA filter-graph plugin {spa}", file=sys.stderr)
            return 1

    paths_raw = subprocess.check_output(
        ["pw-config", "-N", "-n", "omarchy-speaker-engine.conf", "paths"], text=True
    )
    paths = json.loads(paths_raw)
    print("pw-config discovered paths:")
    for k, v in paths.items():
        print(f"  {k}: {v}")
    assert any("91-spatial-fdtr.conf" in v for v in paths.values()), (
        "91-spatial-fdtr.conf not discovered by pw-config!"
    )

    merged_raw = subprocess.check_output(
        ["pw-config", "-N", "-r", "-n", "omarchy-speaker-engine.conf", "merge", "context.modules"],
        text=True,
    )
    modules = json.loads(merged_raw)

    capture_names: list[str] = []
    playback_names: list[str] = []
    spatial_modules: dict[str, dict] = {}

    for mod in modules:
        if mod.get("name") != "libpipewire-module-filter-chain":
            continue
        args = mod["args"]
        cn = args["capture.props"]["node.name"]
        pn = args["playback.props"]["node.name"]
        capture_names.append(cn)
        playback_names.append(pn)
        if cn.startswith("omarchy_speaker_spatial"):
            spatial_modules[cn] = args

    assert len(capture_names) == len(set(capture_names)), f"Duplicate capture names: {capture_names}"
    assert len(playback_names) == len(set(playback_names)), f"Duplicate playback names: {playback_names}"
    assert "omarchy_speaker_spatial" in spatial_modules
    assert "omarchy_speaker_spatial_71" in spatial_modules

    print(f"Merged filter-chain sinks ({len(capture_names)}, zero collisions): {capture_names}")
    for name in ("omarchy_speaker_spatial", "omarchy_speaker_spatial_71"):
        res = validate_graph(name, spatial_modules[name])
        print(
            f"  [PASS] {res['sink']}: {res['nodes']} nodes, {res['links']} links, "
            f"{res['inputs']}ch->2ch, DAG acyclic=True"
        )
        for asset in res["unique_assets"]:
            print(f"         asset OK: {asset}")

    print("PIPEWIRE CONFIG VALIDATION VERDICT: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
