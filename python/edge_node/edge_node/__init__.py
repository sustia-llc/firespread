"""Simulated smoke-detection nodes for the firespread detection demo.

A node runs a smoke and fire detector over a video, turns the position of a
detection in the frame into a compass bearing, and posts the alert to the
`serve` example's `/api/alerts`. `simulate` posts a scripted scenario from
every node without a model.
"""
