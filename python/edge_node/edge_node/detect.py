"""Smoke and fire detectors behind one interface.

Both backends return boxes with coordinates normalised to [0, 1], so the
caller can turn the box centre into a bearing without knowing the frame size.
Neither backend downloads anything at run time: the weights file must be on
disk.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol


@dataclass(frozen=True)
class Box:
    label: str
    confidence: float
    x1: float
    y1: float
    x2: float
    y2: float

    @property
    def x_center(self) -> float:
        return (self.x1 + self.x2) / 2


class Detector(Protocol):
    def __call__(self, frame_bgr) -> list[Box]: ...


class Yolov5Detector:
    """YOLOv5 weights, such as D-Fire's, through the `yolov5` pip package."""

    def __init__(self, weights: str, conf: float, device: str = "cpu") -> None:
        import yolov5  # noqa: PLC0415, optional dependency

        self.model = yolov5.load(weights, device=device)
        self.model.conf = conf
        self.names = self.model.names

    def __call__(self, frame_bgr) -> list[Box]:
        h, w = frame_bgr.shape[:2]
        rgb = frame_bgr[..., ::-1]
        pred = self.model(rgb, size=640).pred[0]
        boxes = []
        for x1, y1, x2, y2, score, cls in pred.tolist():
            boxes.append(Box(self.names[int(cls)], float(score), x1 / w, y1 / h, x2 / w, y2 / h))
        return boxes


class UltralyticsDetector:
    """YOLOv8 or YOLO11 weights (`.pt`, or an exported `.onnx`) through
    `ultralytics`."""

    def __init__(self, weights: str, conf: float, device: str = "cpu") -> None:
        from ultralytics import YOLO  # noqa: PLC0415, optional dependency

        self.model = YOLO(weights)
        self.conf = conf
        self.device = device

    def __call__(self, frame_bgr) -> list[Box]:
        r = self.model(frame_bgr, conf=self.conf, device=self.device, verbose=False)[0]
        boxes = []
        for (x1, y1, x2, y2), score, cls in zip(
            r.boxes.xyxyn.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist()
        ):
            boxes.append(Box(r.names[int(cls)], float(score), x1, y1, x2, y2))
        return boxes


def load_detector(backend: str, weights: str, conf: float, device: str = "cpu") -> Detector:
    if backend == "yolov5":
        return Yolov5Detector(weights, conf, device)
    if backend == "ultralytics":
        return UltralyticsDetector(weights, conf, device)
    raise ValueError(f"unknown backend {backend!r}")
