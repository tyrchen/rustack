"""Private controlled Docker/curl executable; never accesses a real daemon."""
import json
import os
from pathlib import Path
import sys
import time

root = Path(__file__).resolve().parent
model_path = root / "model.json"
model = json.loads(model_path.read_text())
args = sys.argv[1:]
with (root / "events.jsonl").open("a") as stream:
    stream.write(json.dumps({"tool": Path(sys.argv[0]).name, "args": args, "pid": os.getpid(),
                             "at": time.monotonic(), "environment": sorted(os.environ)}) + "\n")
def save():
    model_path.write_text(json.dumps(model))
def emit(value):
    print(json.dumps(value, separators=(",", ":")))
mode = model.get("mode", "success")
if Path(sys.argv[0]).name == "curl":
    if mode == "missing-http":
        raise SystemExit(22)
    if mode == "late-http":
        time.sleep(2)
    if mode == "unrelated-http":
        emit({"message": "unrelated200"})
    elif mode == "not-ready":
        emit({"ready": False, "services": {"s3": "running"}})
    else:
        emit({"ready": True, "services": {"s3": "running", "stepfunctions": "running"}})
    print("302" if mode == "redirect-http" else "200", end="", file=sys.stderr)
    raise SystemExit(0)
if args[:2] == ["--host", "unix:///tmp/rustack-action-control.sock"]:
    args = args[2:]
elif args[:2] == ["context", "inspect"]:
    emit(model.get("contextEndpoint", "unix:///tmp/rustack-action-control.sock"))
    raise SystemExit(0)
else:
    raise SystemExit(71)
if args[0] == "info":
    emit("86c8bc6d-e4d7-4f2d-8492-cba249018921" if model.get("changed") else model["engine"])
elif args[:2] == ["image", "ls"]:
    if mode == "lookup-failure":
        raise SystemExit(1)
    if not model.get("missing"):
        emit(model["source"])
elif args[:2] == ["pull", "--quiet"]:
    model["missing"] = False
    save()
    print(model["source"])
elif args[:2] == ["image", "inspect"]:
    emit({"id": model["source"], "os": "linux", "architecture": "arm64"})
elif args[0] == "run":
    if mode == "foreign-name":
        raise SystemExit(125)
    model["labels"] = dict(args[i + 1].split("=", 1) for i, arg in enumerate(args) if arg == "--label")
    model["serverEnvironment"] = dict(line.split("=", 1) for line in Path(args[args.index("--env-file") + 1]).read_text().splitlines())
    model["running"] = mode != "exited-cid"
    model["changed"] = mode == "changed-engine"
    save()
    print(model["cid"])
elif args[:2] == ["container", "inspect"]:
    if args[-1] != model["cid"]:
        raise SystemExit(72)
    port = "5555" if mode == "wrong-publication" else model.get("port", "4566")
    mapping = {"4566/tcp": [{"HostIp": "127.0.0.1", "HostPort": port}]}
    emit({"id": model["cid"], "name": "/rustack", "image": model["source"], "reference": model["source"],
          "labels": model["labels"], "running": model.get("running", False), "dead": False, "oom": False,
          "exit": 0, "startedAt": "2026-10-31T00:00:00.123Z", "health": "starting" if mode == "never-healthy" else "healthy",
          "ports": mapping, "bindings": mapping, "restart": {"Name": "no", "MaximumRetryCount": 0}})
elif args[:3] == ["kill", "--signal", "TERM"]:
    if args[-1] != model["cid"]:
        raise SystemExit(73)
    model["running"] = False
    save()
    print(model["cid"])
elif args[0] == "wait":
    print("0")
elif args[:2] == ["container", "rm"]:
    if args[-1] != model["cid"] or model.get("running"):
        raise SystemExit(74)
    model["removed"] = True
    save()
    print(model["cid"])
elif args[:2] == ["container", "ls"]:
    if not model.get("removed"):
        emit(model["cid"])
else:
    raise SystemExit(75)
