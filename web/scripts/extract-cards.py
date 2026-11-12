"""Cut each character card out of its photo: find the cream border, straighten the card, round
its corners into transparency, and save a clean RGBA WebP at a fixed 3:4 size.

    python scripts/extract-cards.py <photos-dir> assets/cards
"""

import sys
from pathlib import Path

import cv2
import numpy as np

src, dst = Path(sys.argv[1]), Path(sys.argv[2])
dst.mkdir(parents=True, exist_ok=True)
W, H = 900, 1200  # 3:4, what every card becomes

for n, path in enumerate(sorted(src.glob("*.png")), start=1):
    img = cv2.imread(str(path), cv2.IMREAD_COLOR)
    gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
    # The card is everything brighter than the near-black table it lies on.
    _, mask = cv2.threshold(gray, 60, 255, cv2.THRESH_BINARY)
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, np.ones((25, 25), np.uint8))
    contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    card = max(contours, key=cv2.contourArea)
    hull = cv2.convexHull(card)
    (cx, cy), (w, h), angle = cv2.minAreaRect(hull)
    if w > h:  # keep portrait
        w, h = h, w
        angle += 90
    box = cv2.boxPoints(((cx, cy), (w, h), angle))
    # Order corners: top-left, top-right, bottom-right, bottom-left.
    s = box.sum(axis=1)
    d = np.diff(box, axis=1).ravel()
    ordered = np.array([box[np.argmin(s)], box[np.argmin(d)], box[np.argmax(s)], box[np.argmax(d)]], dtype=np.float32)
    # Trim a hair inside the edge, so no table shows at the corners.
    inset = 0.006
    centre = ordered.mean(axis=0)
    ordered = centre + (ordered - centre) * (1 - inset)
    target = np.array([[0, 0], [W - 1, 0], [W - 1, H - 1], [0, H - 1]], dtype=np.float32)
    warp = cv2.warpPerspective(img, cv2.getPerspectiveTransform(ordered, target), (W, H), flags=cv2.INTER_CUBIC)

    # Rounded corners, as alpha.
    alpha = np.zeros((H, W), np.uint8)
    r = int(W * 0.055)
    cv2.rectangle(alpha, (r, 0), (W - r, H), 255, -1)
    cv2.rectangle(alpha, (0, r), (W, H - r), 255, -1)
    for x, y in [(r, r), (W - r, r), (r, H - r), (W - r, H - r)]:
        cv2.circle(alpha, (x, y), r, 255, -1, lineType=cv2.LINE_AA)
    alpha = cv2.GaussianBlur(alpha, (3, 3), 0)
    rgba = cv2.cvtColor(warp, cv2.COLOR_BGR2BGRA)
    rgba[:, :, 3] = alpha
    out = dst / f"card-{n}.webp"
    cv2.imwrite(str(out), rgba, [cv2.IMWRITE_WEBP_QUALITY, 88])
    print(path.name, "->", out.name, f"angle={angle:.1f}", f"size={w:.0f}x{h:.0f}", f"{out.stat().st_size // 1024} KB")
