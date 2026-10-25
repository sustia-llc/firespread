"""Build, submit and poll a LANDFIRE Product Service (LFPS) job for the master grid window."""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
from urllib.parse import quote, urlencode

import requests

from .paths import EXTENT_JSON, LANDFIRE_DIR

SUBMIT_URL = "https://lfps.usgs.gov/api/job/submit"
STATUS_URL = "https://lfps.usgs.gov/api/job/status"
LAYER_LIST = (
    "LF2022_FBFM40;LF2022_CC;LF2022_CH;LF2022_CBH;LF2022_CBD;LF2022_FDist;"
    "LF2020_Elev;LF2020_SlpD;LF2020_Asp"
)
OUTPUT_PROJECTION = 5070
JOB_JSON = LANDFIRE_DIR / "lfps_job.json"
TERMINAL_STATUS = {"Succeeded", "Failed", "Canceled"}


def area_of_interest() -> str:
    extent = json.loads(EXTENT_JSON.read_text())
    w, s, e, n = extent["grid"]["bbox_4326"]
    return f"{w:.6f} {s:.6f} {e:.6f} {n:.6f}"


def submit_params(email: str, resample: int | None) -> dict[str, str]:
    params = {
        "Layer_List": LAYER_LIST,
        "Area_of_Interest": area_of_interest(),
        "Output_Projection": str(OUTPUT_PROJECTION),
        "Email": email,
    }
    if resample is not None:
        if not 31 <= resample <= 9999:
            raise SystemExit("Resample_Resolution must be an integer in 31..9999; omit it for native 30 m")
        params["Resample_Resolution"] = str(resample)
    return params


def submit_url(email: str, resample: int | None) -> str:
    query = urlencode(submit_params(email, resample), quote_via=quote, safe=";")
    return f"{SUBMIT_URL}?{query}"


def cmd_submit(args: argparse.Namespace) -> int:
    url = submit_url(args.email, args.resample)
    print(url)
    if args.dry_run:
        return 0
    response = requests.get(url, timeout=120)
    print(f"HTTP {response.status_code}")
    print(response.text)
    response.raise_for_status()
    LANDFIRE_DIR.mkdir(parents=True, exist_ok=True)
    JOB_JSON.write_text(response.text + "\n")
    print(f"wrote {JOB_JSON}")
    return 0


def find_zip_url(status: dict) -> str | None:
    if isinstance(status.get("outputFile"), str):
        return status["outputFile"]
    text = json.dumps(status)
    match = re.search(r"https?://[^\s\"']+\.zip", text)
    return match.group(0) if match else None


def cmd_poll(args: argparse.Namespace) -> int:
    job_id = args.job_id
    if job_id is None:
        job_id = json.loads(JOB_JSON.read_text())["jobId"]
    while True:
        response = requests.get(STATUS_URL, params={"JobId": job_id}, timeout=120)
        response.raise_for_status()
        status = response.json()
        print(f"{time.strftime('%H:%M:%S')} status={status.get('status')} queue={status.get('queuePosition')}")
        if status.get("status") in TERMINAL_STATUS:
            break
        time.sleep(args.interval)
    print(json.dumps(status, indent=2))
    if status.get("status") != "Succeeded":
        return 1
    zip_url = find_zip_url(status)
    if zip_url is None:
        print("no .zip URL in the status response; download it from the job status page")
        return 1
    LANDFIRE_DIR.mkdir(parents=True, exist_ok=True)
    out = LANDFIRE_DIR / f"{job_id}.zip"
    with requests.get(zip_url, stream=True, timeout=600) as dl:
        dl.raise_for_status()
        with out.open("wb") as fh:
            for chunk in dl.iter_content(chunk_size=1 << 20):
                fh.write(chunk)
    print(f"wrote {out} ({out.stat().st_size} bytes)")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.lfps_submit")
    sub = parser.add_subparsers(dest="command", required=True)
    s = sub.add_parser("submit", help="build (and unless --dry-run, send) the LFPS submit request")
    s.add_argument("--email", default="<required_user_email>")
    s.add_argument("--dry-run", action="store_true", help="print the URL and exit")
    s.add_argument("--resample", type=int, default=None, help="Resample_Resolution (31..9999); omitted = native 30 m")
    s.set_defaults(func=cmd_submit)
    p = sub.add_parser("poll", help="poll job status and download the zip to data/landfire/")
    p.add_argument("--job-id", default=None, help=f"defaults to jobId in {JOB_JSON}")
    p.add_argument("--interval", type=float, default=30.0)
    p.set_defaults(func=cmd_poll)
    return parser


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if not argv or argv[0].startswith("-"):
        argv = ["submit", *argv]
    args = build_parser().parse_args(argv)
    if args.command == "submit" and not args.dry_run and args.email == "<required_user_email>":
        raise SystemExit("--email is required to submit; use --dry-run to only print the URL")
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
