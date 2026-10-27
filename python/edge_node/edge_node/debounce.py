"""Persistence filter: a detection must hold over several sampled frames
before a node raises an alert. Clouds and dust tend to flicker in and out of
a detector's output; a smoke column persists."""

from __future__ import annotations

from collections import deque


class Persistence:
    """Alert when at least `k` of the last `m` frames had a detection."""

    def __init__(self, k: int, m: int) -> None:
        if not 1 <= k <= m:
            raise ValueError("need 1 <= k <= m")
        self.k = k
        self.hits: deque[bool] = deque(maxlen=m)

    def update(self, hit: bool) -> bool:
        self.hits.append(hit)
        return sum(self.hits) >= self.k
