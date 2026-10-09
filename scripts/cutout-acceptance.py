"""Release CLI cutout acceptance, independent OpenCV comparison and visual artifacts.

Uses NASA's public-domain Eileen Collins photograph, distributed by scikit-image.
OpenCV is a development oracle only; production uses native Rust implementations.
"""
from pathlib import Path
import hashlib
import json
import subprocess
import time
import urllib.request
import numpy as np
import cv2
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "target/release/tinge.exe"
ART = ROOT / "artifacts"
SOURCE = ROOT / "target/cutout-corpus/astronaut.png"
URL = "https://raw.githubusercontent.com/scikit-image/scikit-image/v0.25.2/skimage/data/astronaut.png"


def run(request):
    start = time.perf_counter()
    p = subprocess.run([str(CLI), "run", "-"], input=json.dumps(request), text=True,
                       encoding="utf-8", capture_output=True, cwd=ROOT, timeout=90)
    assert p.returncode == 0, p.stderr
    value = json.loads(p.stdout)
    assert value["ok"]
    return value["data"], time.perf_counter()-start


def stroke_mask(mask, strokes, label):
    h, w = mask.shape
    for s in strokes:
        pts = np.array([[round(p[0]*w-0.5), round(p[1]*h-0.5)] for p in s["points"]], np.int32)
        radius = max(1, round(s.get("radius", .01)*min(w, h)))
        if len(pts) > 1:
            cv2.polylines(mask, [pts], False, label, thickness=2*radius+1)
        for p in pts:
            cv2.circle(mask, tuple(p), radius, label, thickness=-1)


def main():
    SOURCE.parent.mkdir(parents=True, exist_ok=True)
    if not SOURCE.exists():
        SOURCE.write_bytes(urllib.request.urlopen(URL, timeout=30).read())
    source_hash = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    opts = json.loads((ROOT / "examples/cutout/astronaut.json").read_text(encoding="utf-8"))
    full, duration = run({"command": "cutout", "input": str(SOURCE),
                          "options": opts, "output": str(ART / "cutout-astronaut.png"),
                          "matte": str(ART / "cutout-astronaut-alpha.png"), "overwrite": True})
    assert full["cutout"]["solver"]["converged"]
    assert not full["cutout"]["model_weights_required"]
    rgba = np.asarray(Image.open(ART / "cutout-astronaut.png").convert("RGBA"))
    # OpenCV retains actual 16-bit PNG samples; Pillow RGBA conversion uses 8-bit.
    native16 = cv2.imread(str(ART / "cutout-astronaut.png"), cv2.IMREAD_UNCHANGED)
    alpha16 = cv2.imread(str(ART / "cutout-astronaut-alpha.png"), cv2.IMREAD_UNCHANGED)
    assert native16.dtype == alpha16.dtype == np.uint16
    assert np.array_equal(native16[..., 3], alpha16)
    assert np.any(alpha16 == 0) and np.any(alpha16 == 65535) and np.any((alpha16>0)&(alpha16<65535))
    # Compare segmentation to official OpenCV at the same working size, not a quality ground truth.
    segmentation, segment_seconds = run({"command": "cutout", "input": str(SOURCE),
                                        "options": {"selection": opts["selection"]},
                                        "output": str(ART / "cutout-grabcut-only.png"), "overwrite": True})
    bgr = cv2.resize(cv2.imread(str(SOURCE)), (256,256), interpolation=cv2.INTER_LINEAR)
    mask = np.full((256,256), cv2.GC_PR_FGD, np.uint8)
    stroke_mask(mask, opts["selection"]["foreground"], cv2.GC_FGD)
    stroke_mask(mask, opts["selection"]["background"], cv2.GC_BGD)
    cv2.setRNGSeed(81725)
    cv2.grabCut(bgr, mask, None, np.zeros((1,65)), np.zeros((1,65)), 5, cv2.GC_INIT_WITH_MASK)
    oracle = np.isin(mask, [cv2.GC_FGD,cv2.GC_PR_FGD])
    native = cv2.imread(str(ART / "cutout-grabcut-only.png"), cv2.IMREAD_UNCHANGED)[...,3]
    native = cv2.resize(native, (256,256), interpolation=cv2.INTER_NEAREST)>32767
    iou = float(np.sum(native & oracle)/np.sum(native | oracle))
    assert iou > .90, f"unexpectedly divergent segmentation: {iou}"
    Image.fromarray(oracle.astype(np.uint8)*255).save(ART / "cutout-opencv-reference.png")
    # Physical scene-linear mixture test with known alpha, decoded independent 16-bit samples.
    w, h = 96,64
    a=np.tile(np.clip((np.arange(w)-20)/55,0,1),(h,1))
    f=np.array([.7,.05,.02]);b=np.array([.02,.8,.04])
    linear=a[...,None]*f+(1-a[...,None])*b
    srgb=np.where(linear<=.0031308,linear*12.92,1.055*linear**(1/2.4)-.055)
    scene=SOURCE.parent/"linear-mixture.png";tri=SOURCE.parent/"linear-trimap.png"
    cv2.imwrite(str(scene),np.round(srgb[...,::-1]*65535).astype(np.uint16))
    trimap=np.where(a==0,0,np.where(a==1,65535,32768)).astype(np.uint16);cv2.imwrite(str(tri),trimap)
    numeric,numeric_seconds=run({"command":"cutout","input":str(scene),"output":str(ART/"cutout-linear-mixture.png"),
                                "matte":str(ART/"cutout-linear-mixture-alpha.png"),"overwrite":True,
                                "options":{"selection":{"type":"trimap","path":str(tri)},
                                           "refinement":{"iterations":1000,"tolerance":1e-8},"decontaminate":1}})
    actual=cv2.imread(str(ART/"cutout-linear-mixture-alpha.png"),cv2.IMREAD_UNCHANGED)/65535
    alpha_error=float(np.max(np.abs(actual-a)))
    assert alpha_error < .002, alpha_error
    # Standalone visual QA: original, matte data displayed directly, checkerboard composite.
    original=Image.open(SOURCE).convert("RGB")
    yy,xx=np.indices((512,512));checker=np.where(((xx//24+yy//24)%2)[...,None],np.array([175,183,192]),np.array([222,226,232])).astype(np.uint8)
    check=Image.fromarray(checker).convert("RGBA");check.alpha_composite(Image.fromarray(rgba))
    contact=Image.new("RGB",(1536,548),(24,26,30));draw=ImageDraw.Draw(contact)
    panels=[("NASA original",original),("16-bit matte / shown without gamma",Image.fromarray((alpha16/257).round().astype(np.uint8)).convert("RGB")),("Native cutout / checkerboard",check.convert("RGB"))]
    for i,(label,panel) in enumerate(panels):
        draw.text((i*512+12,12),label,fill=(235,235,235));contact.paste(panel,(i*512,36))
    contact.save(ART/"cutout-contact-sheet.png")
    result={"cli_sha256":hashlib.sha256(CLI.read_bytes()).hexdigest(),"source":{"url":URL,"sha256":source_hash,"bytes":SOURCE.stat().st_size,
            "license":"NASA public domain, as documented by scikit-image","license_source":"https://scikit-image.org/docs/0.25.x/api/skimage.data.html#skimage.data.astronaut"},
            "opencv_version":cv2.__version__,"full_cutout_seconds":duration,"segmentation_only_seconds":segment_seconds,
            "native_opencv_binary_iou":iou,"same_algorithm_family_not_pixel_parity_or_ground_truth":True,
            "alpha_png_exactly_matches_rgba16":True,"linear_mixture_max_alpha_error":alpha_error,
            "linear_mixture_seconds":numeric_seconds,"full_cutout":full,"linear_mixture":numeric,
            "semantic_model_or_weights_used":False}
    (ART/"cutout-reference.json").write_text(json.dumps(result,indent=2)+"\n",encoding="utf-8")
    print(json.dumps({"full_seconds":duration,"segment_seconds":segment_seconds,"opencv_iou":iou,"alpha_error":alpha_error}))


if __name__=="__main__":
    main()
