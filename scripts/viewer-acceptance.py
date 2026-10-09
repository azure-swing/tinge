"""Exercise the release viewer over real loopback HTTP, without web/Node dependencies."""
from pathlib import Path
import hashlib
import http.client
import json
import struct
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zlib

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "target/release/tinge.exe"


def native(request):
    p = subprocess.run([str(CLI), "run", "-"], input=json.dumps(request), text=True,
                       encoding="utf-8", capture_output=True, cwd=ROOT, timeout=60)
    assert p.returncode == 0, p.stderr
    value = json.loads(p.stdout)
    assert value["ok"], value
    return value["data"]


def png(file):
    def chunk(name, data):
        return struct.pack(">I", len(data)) + name + data + struct.pack(">I", zlib.crc32(name+data))
    rows = b"".join(b"\0" + b"".join(bytes((20+x*5, 40+y*6, 100, 255)) for x in range(40)) for y in range(24))
    file.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 40, 24, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def chunks(bytes_):
    offset = 8
    while offset < len(bytes_):
        size = struct.unpack(">I", bytes_[offset:offset+4])[0]
        yield bytes_[offset+4:offset+8], bytes_[offset+8:offset+8+size]
        offset += size + 12


def main():
    results = {}
    with tempfile.TemporaryDirectory(prefix="tinge-web-acceptance-") as folder:
        folder = Path(folder)
        source = folder / "input.png"
        project = folder / "sample.tinge"
        png(source)
        native({"command": "init", "input": str(source), "project": str(project)})
        native({"command": "apply", "project": str(project), "expect_revision": 0,
                "edits": [{"type": "upsert_node", "node": {"id": "grade", "inputs": ["source"], "op": {"type": "exposure", "stops": 0.5}}}, {"type": "set_output", "id": "grade"}]})
        server = subprocess.Popen([str(CLI), "view", str(project), "--no-open"], stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True, encoding="utf-8", cwd=ROOT)
        try:
            ready = json.loads(server.stdout.readline())
            assert ready["event"] == "viewer_ready", ready
            base = ready["data"]["url"]
            parsed = urllib.parse.urlparse(base)
            token = parsed.path.split('/')[1]

            def request(path, body=None, headers=None, expected=200):
                data = None if body is None else json.dumps(body).encode()
                h = {} if body is None else {"Content-Type": "application/json", "X-Tinge": token}
                if headers:
                    h.update(headers)
                r = urllib.request.Request(base+path, data=data, headers=h)
                try:
                    response = urllib.request.urlopen(r, timeout=15)
                except urllib.error.HTTPError as e:
                    response = e
                with response:
                    assert response.status == expected, (path, response.status, response.read())
                    b = response.read()
                    assert response.headers["Cache-Control"] == "no-store"
                    return json.loads(b) if response.headers["Content-Type"].startswith("application/json") else b

            def preview(revision, reference=None):
                job = request("api/preview", {"revision": revision, "reference": reference, "max_edge": 1600})["job"]
                for _ in range(150):
                    status = request(f"api/job/{job}")
                    if status["status"] == "ready":
                        return job, status["result"]
                    assert status["status"] != "error", status
                    time.sleep(.05)
                raise AssertionError("render timed out")

            html = request("").decode("utf-8")
            assert "保存圈选" in html and "script" in html
            assert request("style.css") and request("app.js")
            assert request("api/state")["head"] == 1
            job, result = preview(1)
            graded, original = request(result["image"]), request(result["reference"])
            output = folder / "cli-preview.png"
            native({"command": "preview", "project": str(project), "output": str(output), "max_edge": 1600})
            assert graded == output.read_bytes(), "viewer differs from CLI color/quantization pipeline"
            reference = folder / "original.png"
            native({"command": "preview", "project": str(project), "revision": 0, "output": str(reference), "max_edge": 1600})
            assert original == reference.read_bytes()
            icc = [data for name, data in chunks(graded) if name == b"iCCP"]
            assert len(icc) == 1
            zero = icc[0].index(b"\0")
            profile = zlib.decompress(icc[0][zero+2:])
            assert profile[36:40] == b"acsp" and profile[16:20] == b"RGB "
            results["png_icc_profile_sha256"] = hashlib.sha256(profile).hexdigest()
            results["cli_preview_byte_identical"] = True
            assert graded != original
            _, pair = preview(1, 0)
            assert request(pair["reference"]) == original
            results["arbitrary_revision_compare"] = True

            mask = {"type": "combine", "mode": "subtract", "masks": [
                {"type": "ellipse", "center": [.5, .5], "radius": [.3, .3], "rotation": 0, "feather": 0},
                {"type": "polygon", "points": [[.4, .4], [.6, .4], [.5, .6]], "feather": 0}]}
            before = project.read_bytes()
            saved = request("api/selection", {"expect_revision": 1, "job": job, "mask": mask, "note": "只调整天空"})["selection"]
            assert project.read_bytes() == before
            assert (saved["width"], saved["height"], saved["revision"], saved["output_node"]) == (40, 24, 1, "grade")
            read = native({"command": "selections", "project": str(project)})
            assert read["items"][0] == saved
            request("api/selection", {"expect_revision": 0, "job": job, "mask": mask}, expected=409)
            request("api/selection", {"expect_revision": 1, "job": job, "mask": {"type": "bitmap", "path": "secret.png"}}, expected=400)
            results["selection_coordinate_basis_and_atomic_conflict"] = True

            request("api/save", {"expect_revision": 1, "revision": 0, "name": "原图"})
            named = request("api/state")
            assert named["head"] == 2 and named["tags"]["原图"] == 0
            assert native({"command": "show", "project": str(project)})["history"][-1]["recipe"]["output"] == "grade"
            request("api/save", {"expect_revision": 1, "revision": 0, "name": "冲突"}, expected=409)
            request("api/restore", {"expect_revision": 2, "revision": 0})
            restored = request("api/state")
            assert restored["head"] == 3 and len(restored["history"]) == 4
            assert len(native({"command": "selections", "project": str(project)})["items"]) == 1
            results["version_name_old_revision_and_restore_keep_history"] = True

            # Independent CLI edits are visible without restarting the viewer.
            native({"command": "apply", "project": str(project), "expect_revision": 3,
                    "edits": [{"type": "upsert_node", "node": {"id": "crop", "inputs": ["source"], "op": {"type": "crop", "x": 5, "y": 3, "width": 20, "height": 10}}}, {"type": "set_output", "id": "crop"}]})
            assert request("api/state")["head"] == 4
            crop_job, crop = preview(4)
            assert (crop["width"], crop["height"], crop["reference_width"], crop["reference_height"]) == (20, 10, 40, 24)
            saved_crop = request("api/selection", {"expect_revision": 4, "job": crop_job, "mask": mask})["selection"]
            assert (saved_crop["width"], saved_crop["height"], saved_crop["output_node"]) == (20, 10, "crop")
            results["external_agent_edit_and_crop_basis"] = True

            # A wide-gamut project still receives the defined SDR sRGB preview view.
            pipeline = json.loads((ROOT / "examples/aces2-display-p3.json").read_text())
            native({"command": "apply", "project": str(project), "expect_revision": 4,
                    "edits": [{"type": "set_color_pipeline", "pipeline": pipeline}]})
            _, p3 = preview(5)
            p3_output = folder / "p3-project-srgb-preview.png"
            native({"command": "preview", "project": str(project), "output": str(p3_output)})
            assert request(p3["image"]) == p3_output.read_bytes()
            assert p3["graded"]["export"]["color_space"] == {"primaries": "srgb", "transfer": "srgb"}
            results["p3_project_uses_native_srgb_preview_without_extra_gamma"] = True

            request("api/state", headers={"Host": "foreign.test"}, expected=400)
            request("api/save", {"expect_revision": 5, "revision": 0, "name": "bad"}, headers={"Origin": "https://foreign.test"}, expected=400)
            request("api/save", {"expect_revision": 5, "revision": 0, "name": "bad"}, headers={"X-Tinge": "bad"}, expected=400)
            request("image/../../sample.tinge", expected=400)
            request("api/run", {"command": "show", "project": "secret.tinge"}, expected=400)
            c = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=5)
            c.request("POST", parsed.path+"api/save", headers={"Content-Length": str(1024*1024+1), "Content-Type": "application/json", "X-Tinge": token})
            response = c.getresponse(); assert response.status == 400; response.read(); c.close()
            results["host_origin_token_path_and_body_limits"] = True

            # The existing MCP tool discovers/reads annotations through the same strict Request type.
            rpc = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
                   {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
                   {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "tinge_selections", "arguments": { "project": str(project)}}}]
            p = subprocess.run([str(CLI), "mcp"], input="".join(json.dumps(r)+"\n" for r in rpc), text=True, encoding="utf-8", capture_output=True, timeout=30)
            assert p.returncode == 0, p.stderr
            replies = [json.loads(line) for line in p.stdout.splitlines()]
            assert "selection_save" in json.dumps(replies[1])
            assert not replies[2]["result"].get("isError", False)
            assert "crop" in json.dumps(replies[2])
            results["mcp_annotation_discovery_and_read"] = True
        finally:
            server.terminate()
            server.communicate(timeout=15)
    results["full_resolve_lightroom_parity"] = False
    (ROOT / "artifacts/viewer-acceptance.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
