"""Owned native ARM rootless bootstrap and independent, bounded attestation."""
import hashlib
import json
import os
from pathlib import Path
import platform
import pwd
import re
import stat
import sys
import time
import urllib.request
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import (MIB, QualificationError, Runner, bounded_read,
                               deadline_guard, document, profile, provision, require,
                               safe_path, status)
from settings import ActionError

KEY_FINGERPRINT = "9DC858229FC7DD38854AE2D88D81803C0EBFCD88"
SCRIPT = Path(__file__).resolve()
CONTROLLERS = {"cpu", "memory", "pids"}
ENTRY_KEYS = {"schemaVersion", "profile", "nonce", "scriptSha256", "targetDirectory"}
SYSTEM_FILE_PUBLISHER = r'''
import hashlib, os, pathlib, stat, sys, tempfile
source, target = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
allowed = {"/etc/apt/keyrings/rustack-d103-docker.asc",
           "/etc/apt/sources.list.d/rustack-d103-docker.sources"}
if str(target) not in allowed:
    sys.exit(2)
parent = os.lstat(target.parent)
if not stat.S_ISDIR(parent.st_mode) or parent.st_uid != 0 or stat.S_IMODE(parent.st_mode) != 0o755:
    sys.exit(2)
fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
with os.fdopen(fd, "rb") as stream:
    metadata = os.fstat(stream.fileno())
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != int(sys.argv[4]) or metadata.st_size > 65536:
        sys.exit(2)
    raw = stream.read(65537)
if not raw or len(raw) > 65536 or hashlib.sha256(raw).hexdigest() != sys.argv[3]:
    sys.exit(2)
fd, temporary = tempfile.mkstemp(prefix=".rustack-d103-", dir=target.parent)
try:
    with os.fdopen(fd, "wb") as stream:
        os.fchmod(stream.fileno(), 0o644)
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    try:
        os.link(temporary, target, follow_symlinks=False)
    except FileExistsError:
        pass
    fd = os.open(target, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0
                or stat.S_IMODE(metadata.st_mode) != 0o644 or metadata.st_size != len(raw)
                or stream.read(65537) != raw):
            sys.exit(2)
    descriptor = os.open(target.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
finally:
    os.unlink(temporary)
'''


def token(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{32}", value) is not None,
            "rootless-nonce")
    return value


def script_hash():
    return hashlib.sha256(bounded_read(SCRIPT, MIB)).hexdigest()


def write_new(path, raw):
    require(isinstance(raw, bytes) and len(raw) <= 65536, "rootless-output-bound")
    with open(path, "xb") as stream:
        os.chmod(path, 0o600)
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def publish(path, value):
    write_new(path, json.dumps(value, separators=(",", ":"), allow_nan=False).encode())


def entry(value, selected, current_source=True):
    require(isinstance(value, dict) and set(value) == ENTRY_KEYS
            and type(value["schemaVersion"]) is int and value["schemaVersion"] == 1
            and value["profile"] == profile(selected)
            and isinstance(value["scriptSha256"], str)
            and re.fullmatch(r"[0-9a-f]{64}", value["scriptSha256"])
            and (not current_source or value["scriptSha256"] == script_hash()), "rootless-entry")
    token(value["nonce"])
    safe_path(value["targetDirectory"])
    return value


class Commands:
    """Reuse the joined child owner; retain raw bounded setup results privately."""

    def __init__(self, owner, deadline, evidence=None):
        self.owner, self.deadline, self.evidence = owner, deadline, evidence
        self.sequence = 0
        self.job_nonce = uuid.uuid4().hex

    def run(self, arguments, seconds=15, clean=False):
        self.sequence += 1
        cutoff = min(self.deadline, time.monotonic() + seconds)
        privileged = arguments[:2] == ["sudo", "-n"]
        if privileged:
            # Root children cannot rely on an unprivileged owner's signalling
            # authority. The root manager independently caps and kills their
            # complete service cgroup, even if the SSH/controller disappears.
            lifetime = int(cutoff - time.monotonic()) - 6
            require(self.evidence is not None and lifetime >= 1 and len(arguments) >= 3
                    and arguments[2] in ("python3", "env", "install", "loginctl"),
                    "rootless-privileged-command-budget")
            unit = "rustack-d103-" + self.job_nonce + "-%03d" % self.sequence
            arguments = ["sudo", "-n", "systemd-run", "--quiet", "--wait", "--pipe", "--collect",
                         "--unit=" + unit, "--property=Type=exec", "--property=Restart=no",
                         "--property=RuntimeMaxSec=%ds" % lifetime, "--property=TimeoutStopSec=2s",
                         "--property=TimeoutStartSec=2s", "--property=KillMode=control-group",
                         "--property=SendSIGKILL=yes", "/usr/bin/" + arguments[2]] + arguments[3:]
        if self.evidence is not None:
            publish(self.evidence / ("command-%03d.intent.json" % self.sequence),
                    {"argv": arguments, "callerCutoffMonotonic": cutoff, "ownerPid": os.getpid()})
        if privileged:
            require(time.monotonic() + lifetime + 6 <= cutoff, "rootless-privileged-intent-late")
        result = self.owner.run(arguments, cutoff,
                                stdout_limit=65536)
        if self.evidence is not None:
            base = self.evidence / ("command-%03d" % self.sequence)
            publish(base.with_suffix(".json"), {"argv": arguments, "code": result.code,
                    "stdoutSha256": hashlib.sha256(result.stdout).hexdigest(),
                    "stderrSha256": hashlib.sha256(result.stderr).hexdigest()})
            write_new(base.with_suffix(".stdout"), result.stdout)
            write_new(base.with_suffix(".stderr"), result.stderr)
        require(result.code == 0 and (not clean or result.stderr == b""), "rootless-command")
        return result.stdout


def version(raw):
    value = raw.decode("ascii").strip()
    require(re.fullmatch(r"[0-9]{1,2}\.[0-9]{1,2}\.[0-9]{1,2}", value) is not None,
            "rootless-engine-version")
    return value


def key_fingerprint(raw):
    lines = raw.decode("ascii").splitlines()
    require(1 <= len(lines) <= 64, "rootless-key-bound")
    primary = [index for index, line in enumerate(lines) if line.startswith("pub:")]
    fingerprints = [line.split(":")[9] for line in lines if line.startswith("fpr:")
                    and len(line.split(":")) > 9]
    require(len(primary) == 1 and fingerprints and fingerprints[0] == KEY_FINGERPRINT,
            "rootless-key-fingerprint")


def mappings(raw, uid):
    require(isinstance(raw, bytes) and len(raw) <= 4096, "rootless-map-bound")
    rows = [line.split() for line in raw.decode("ascii").splitlines()]
    require(len(rows) == 2 and all(len(row) == 3 and all(item.isdecimal() for item in row)
                                 for row in rows), "rootless-map-shape")
    values = [[int(item) for item in row] for row in rows]
    require(values[0] == [0, uid, 1] and values[1][0] == 1
            and 65536 <= values[1][2] <= 1073741824 and 65536 <= values[1][1]
            and values[1][1] + values[1][2] <= 4294967295,
            "rootless-map-policy")
    return values


def process_uid(raw, uid):
    require(len(raw) <= 32768, "rootless-process-bound")
    lines = [line.split()[1:] for line in raw.decode("ascii").splitlines()
             if line.startswith("Uid:")]
    require(len(lines) == 1 and lines[0] == [str(uid)] * 4, "rootless-process-uid")


def info(value):
    require(isinstance(value, dict) and set(value) == {"id", "os", "arch", "version",
            "security", "cgroupVersion", "cgroupDriver", "warnings"}, "rootless-info-shape")
    require(isinstance(value["id"], str) and re.fullmatch(r"[a-zA-Z0-9:-]{1,128}", value["id"])
            is not None and value["os"] == "linux" and value["arch"] == "aarch64"
            and value["cgroupVersion"] == "2" and value["cgroupDriver"] == "systemd",
            "rootless-info-policy")
    require(isinstance(value["version"], str), "rootless-engine-version")
    version(value["version"].encode("ascii"))
    security = value["security"]
    require(isinstance(security, list) and 1 <= len(security) <= 16
            and all(isinstance(item, str) and len(item) <= 256 for item in security)
            and "name=rootless" in security, "rootless-security")
    require(value["warnings"] in (None, []), "rootless-daemon-warning")
    return value


def result_proof(value, nonce):
    keys = {"schemaVersion", "platformQualification", "uid", "gid", "socket", "namespaceSocketGid", "serviceMainPid", "dockerdPid",
            "uidMap", "gidMap", "controllers", "rootless", "rootfulEngineId", "kernel",
            "referenceQualification", "scriptSha256", "nonce", "rootlessExtrasVersion", "packages",
            "delegation"}
    require(isinstance(value, dict) and set(value) == keys
            and type(value["schemaVersion"]) is int and value["schemaVersion"] == 1
            and value["nonce"] == token(nonce) and value["scriptSha256"] == script_hash()
            and value["platformQualification"] == value["referenceQualification"] == "pending",
            "rootless-guest-receipt")
    require(type(value["uid"]) is int and 1 <= value["uid"] <= 65535
            and type(value["gid"]) is int and 1 <= value["gid"] <= 65535
            and all(type(value[key]) is int and 1 <= value[key] <= 999999999
                    for key in ("serviceMainPid", "dockerdPid")), "rootless-receipt-process")
    controllers = value["controllers"]
    require(isinstance(controllers, list) and 3 <= len(controllers) <= 16
            and all(isinstance(item, str) and re.fullmatch(r"[a-z]{1,16}", item) for item in controllers)
            and sorted(set(controllers)) == controllers and CONTROLLERS <= set(controllers),
            "rootless-delegation")
    proof = value["delegation"]
    require(isinstance(proof, dict) and set(proof) == {"manager", "service"}, "rootless-delegation-proof")
    for kind in ("manager", "service"):
        item = proof[kind]
        require(isinstance(item, dict) and set(item) == {"declared", "available", "enabled", "controlUid", "controlWritable"}
                and type(item["controlUid"]) is int and item["controlUid"] == value["uid"]
                and item["controlWritable"] is True, "rootless-delegation-owner")
        for key in ("declared", "available", "enabled"):
            names = item[key]
            require(isinstance(names, list) and len(names) <= 16
                    and all(isinstance(name, str) and re.fullmatch(r"[a-z]{1,16}", name) for name in names)
                    and sorted(set(names)) == names, "rootless-delegation-controller-shape")
        require(CONTROLLERS <= set(item["declared"]) and CONTROLLERS <= set(item["available"])
                and (kind != "manager" or CONTROLLERS <= set(item["enabled"])), "rootless-delegation")
    require(proof["manager"]["available"] == controllers, "rootless-controller-evidence")
    socket = value["socket"]
    require(isinstance(socket, dict) and set(socket) == {"path", "uid", "gid", "mode", "inode"}
            and socket["path"] == "/run/user/%d/docker.sock" % value["uid"]
            and type(socket["uid"]) is int and socket["uid"] == value["uid"]
            and type(socket["gid"]) is int and 0 <= socket["gid"] <= 4294967295
            and type(socket["mode"]) is int and socket["mode"] in (0o600, 0o660, 0o1600, 0o1660)
            and type(socket["inode"]) is int and 1 <= socket["inode"] < 2 ** 64,
            "rootless-receipt-socket")
    for key, outside in (("uidMap", value["uid"]), ("gidMap", value["gid"])):
        rows = value[key]
        require(isinstance(rows, list) and len(rows) == 2
                and all(isinstance(row, list) and len(row) == 3
                        and all(type(item) is int and 0 <= item <= 4294967295 for item in row)
                        for row in rows), "rootless-receipt-map")
        mappings(("\n".join(" ".join(str(item) for item in row) for row in rows)).encode(), outside)
    require(type(value["namespaceSocketGid"]) is int
            and value["namespaceSocketGid"] == namespace_gid(socket["gid"], value["gidMap"]),
            "rootless-socket-gid-map")
    observed = info(value["rootless"])
    require(isinstance(value["rootfulEngineId"], str)
            and re.fullmatch(r"[a-zA-Z0-9:-]{1,128}", value["rootfulEngineId"])
            and value["rootfulEngineId"] != observed["id"], "rootless-rootful-identity")
    require(value["rootlessExtrasVersion"] == "5:" + observed["version"] + "-1~ubuntu.24.04~noble"
            and isinstance(value["kernel"], str)
            and re.fullmatch(r"[A-Za-z0-9_.+-]{1,256}", value["kernel"]), "rootless-receipt-version")
    packages = value["packages"]
    require(isinstance(packages, dict) and set(packages) == {"uidmap", "dbus-user-session",
            "slirp4netns", "docker-ce-rootless-extras", "repositoryCandidate"}
            and all(isinstance(item, str) and re.fullmatch(r"[A-Za-z0-9.+:~_-]{1,128}", item)
                    for item in packages.values())
            and packages["docker-ce-rootless-extras"] == value["rootlessExtrasVersion"],
            "rootless-receipt-packages")
    return value


def socket_proof(path, uid):
    metadata = os.lstat(path)
    require(stat.S_ISSOCK(metadata.st_mode) and metadata.st_uid == uid
            and stat.S_IMODE(metadata.st_mode) & 0o007 == 0, "rootless-socket-policy")
    parent = os.lstat(path.parent)
    require(stat.S_ISDIR(parent.st_mode) and parent.st_uid == uid
            and stat.S_IMODE(parent.st_mode) == 0o700, "rootless-runtime-directory")
    return {"path": str(path), "uid": metadata.st_uid, "gid": metadata.st_gid,
            "mode": stat.S_IMODE(metadata.st_mode), "inode": metadata.st_ino}


def namespace_gid(host_gid, rows):
    found = [inside + host_gid - outside for inside, outside, count in rows
             if outside <= host_gid < outside + count]
    require(len(found) == 1, "rootless-socket-gid-map")
    return found[0]


def guest_base():
    require(sys.platform == "linux" and platform.machine() == "aarch64"
            and 1 <= os.getuid() <= 65535, "rootless-guest-platform")
    release = bounded_read(Path("/usr/lib/os-release"), 16384).splitlines()
    require(b"ID=ubuntu" in release and b'VERSION_ID="24.04"' in release, "rootless-guest-distribution")
    uid = os.getuid()
    runtime = Path("/run/user") / str(uid)
    require(os.environ.get("XDG_RUNTIME_DIR") == str(runtime), "rootless-runtime-path")
    account = pwd.getpwuid(uid)
    require(re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", account.pw_name) is not None,
            "rootless-account")
    return uid, runtime, account.pw_name


def user_command(runtime, arguments):
    return ["env", "DBUS_SESSION_BUS_ADDRESS=unix:path=" + str(runtime / "bus")] + arguments


def guest_fresh(nonce):
    token(nonce)
    uid, runtime, _user = guest_base()
    require(not os.path.lexists(Path.home() / ".local/state/rustack-rootless-d103")
            and not os.path.lexists(Path.home() / ".config/systemd/user/docker.service")
            and not os.path.lexists(runtime / "docker.sock")
            and not os.path.lexists(Path.home() / ".local/share/docker"), "rootless-guest-not-fresh")
    return {"nonce": nonce, "uid": uid, "scriptSha256": script_hash(), "rootlessStateAbsent": True}


def cgroup_control(base, uid, declared):
    available = bounded_read(base / "cgroup.controllers", 4096).decode("ascii").split()
    enabled = bounded_read(base / "cgroup.subtree_control", 4096).decode("ascii").split()
    control = os.lstat(base / "cgroup.subtree_control")
    require(1 <= len(available) <= 16 and CONTROLLERS <= set(available)
            and CONTROLLERS <= set(declared) and control.st_uid == uid
            and os.access(base / "cgroup.subtree_control", os.W_OK), "rootless-delegation")
    return {"declared": sorted(declared), "available": sorted(available), "enabled": sorted(enabled),
            "controlUid": control.st_uid, "controlWritable": True}


def delegation(uid, commands):
    base = Path("/sys/fs/cgroup/user.slice") / ("user-%d.slice" % uid) / ("user@%d.service" % uid)
    raw = commands.run(["systemctl", "show", "user@%d.service" % uid,
                        "--property=Delegate", "--property=DelegateControllers"], clean=True)
    properties = dict(line.split("=", 1) for line in raw.decode("ascii").splitlines())
    require(set(properties) == {"Delegate", "DelegateControllers"}
            and properties["Delegate"] == "yes", "rootless-manager-delegation")
    result = cgroup_control(base, uid, properties["DelegateControllers"].split())
    require(CONTROLLERS <= set(result["enabled"]), "rootless-manager-enabled-controllers")
    return result


def guest_attest(commands):
    uid, runtime, _user = guest_base()
    require(Path("/sys/fs/cgroup/cgroup.controllers").is_file(), "rootless-cgroup-v2")
    manager = delegation(uid, commands)
    controllers = manager["available"]
    socket = runtime / "docker.sock"
    before = socket_proof(socket, uid)
    service = commands.run(user_command(runtime, ["systemctl", "--user", "show", "docker.service",
        "--property=ActiveState", "--property=MainPID", "--property=ControlGroup", "--property=Delegate",
        "--property=DelegateControllers"]), clean=True)
    properties = dict(line.split("=", 1) for line in service.decode("ascii").splitlines())
    require(set(properties) == {"ActiveState", "MainPID", "ControlGroup", "Delegate", "DelegateControllers"}
            and properties["ActiveState"] == "active" and properties["Delegate"] == "yes"
            and re.fullmatch(r"[1-9][0-9]{0,8}", properties["MainPID"]) is not None,
            "rootless-service")
    main_pid = int(properties["MainPID"])
    process_uid(bounded_read(Path("/proc") / str(main_pid) / "status", 32768), uid)
    control_group = properties["ControlGroup"]
    require(re.fullmatch(r"/user.slice/user-%d.slice/user@%d.service/[a-zA-Z0-9_.@/-]{1,128}" % (uid, uid),
                         control_group) is not None and ".." not in control_group.split("/"),
            "rootless-service-cgroup")
    cgroup = Path("/sys/fs/cgroup") / control_group[1:]
    service_control = cgroup_control(cgroup, uid, properties["DelegateControllers"].split())
    # Before workloads, the dockerd process must belong to this exact user service.
    processes = []
    visited = 0
    for base, dirs, _files in os.walk(cgroup, followlinks=False):
        visited += 1
        require(visited <= 64 and len(dirs) <= 64, "rootless-cgroup-tree-bound")
        rows = bounded_read(Path(base) / "cgroup.procs", 32768).decode("ascii").split()
        require(len(rows) <= 1024, "rootless-process-count")
        for row in rows:
            require(re.fullmatch(r"[1-9][0-9]{0,8}", row) is not None, "rootless-process-id")
            proc = Path("/proc") / row
            if bounded_read(proc / "comm", 256).strip() == b"dockerd":
                processes.append(int(row))
    require(len(processes) == 1, "rootless-dockerd-process")
    daemon = Path("/proc") / str(processes[0])
    process_uid(bounded_read(daemon / "status", 32768), uid)
    uid_map = mappings(bounded_read(daemon / "uid_map", 4096), uid)
    gid_map = mappings(bounded_read(daemon / "gid_map", 4096), os.getgid())
    endpoint = "unix://" + str(socket)
    template = '{"id":{{json .ID}},"os":{{json .OSType}},"arch":{{json .Architecture}},"version":{{json .ServerVersion}},"security":{{json .SecurityOptions}},"cgroupVersion":{{json .CgroupVersion}},"cgroupDriver":{{json .CgroupDriver}},"warnings":{{json .Warnings}}}'
    docker = ["docker", "--host", endpoint]
    observed = info(document(commands.run(docker + ["info", "--format", template], clean=True)))
    rootful = commands.run(["docker", "--host", "unix:///var/run/docker.sock", "info", "--format", "{{.ID}}"], clean=True).decode("ascii").strip()
    require(re.fullmatch(r"[a-zA-Z0-9:-]{1,128}", rootful) is not None
            and rootful != observed["id"], "rootless-rootful-identity")
    observed_again = info(document(commands.run(docker + ["info", "--format", template], clean=True)))
    require(observed == observed_again and socket_proof(socket, uid) == before
            and delegation(uid, commands) == manager
            and cgroup_control(cgroup, uid, properties["DelegateControllers"].split()) == service_control,
            "rootless-attestation-changed")
    process_uid(bounded_read(daemon / "status", 32768), uid)
    packages = {}
    for package in ("uidmap", "dbus-user-session", "slirp4netns", "docker-ce-rootless-extras"):
        packages[package] = commands.run(["dpkg-query", "-W", "-f=${Version}", package], clean=True).decode("ascii")
    policy = commands.run(["apt-cache", "policy", "docker-ce-rootless-extras"], clean=True).decode("ascii")
    candidates = re.findall(r"^  Candidate: ([A-Za-z0-9.+:~_-]{1,128})$", policy, re.MULTILINE)
    require(len(candidates) == 1, "rootless-repository-candidate")
    packages["repositoryCandidate"] = candidates[0]
    return {"schemaVersion": 1, "platformQualification": "pending", "uid": uid, "gid": os.getgid(),
            "socket": before, "serviceMainPid": main_pid, "dockerdPid": processes[0],
            "namespaceSocketGid": namespace_gid(before["gid"], gid_map),
            "uidMap": uid_map, "gidMap": gid_map, "controllers": controllers,
            "rootless": observed, "rootfulEngineId": rootful, "kernel": platform.release(),
            "referenceQualification": "pending", "scriptSha256": script_hash(), "packages": packages,
            "delegation": {"manager": manager, "service": service_control}}


def install_file(commands, source, target, mode):
    require(mode == "0644", "rootless-system-file-mode")
    raw = bounded_read(source, 65536)
    if os.path.lexists(target):
        metadata = os.lstat(target)
        require(stat.S_ISREG(metadata.st_mode) and metadata.st_uid == 0
                and stat.S_IMODE(metadata.st_mode) == 0o644 and bounded_read(target, 65536) == raw,
                "rootless-system-file-conflict")
    commands.run(["sudo", "-n", "python3", "-I", "-B", "-c", SYSTEM_FILE_PUBLISHER,
                  str(source), str(target), hashlib.sha256(raw).hexdigest(), str(os.getuid())])


def existing_repository(commands, sources=Path("/etc/apt/sources.list.d"), main=Path("/etc/apt/sources.list"),
                        key=Path("/etc/apt/keyrings/docker.asc")):
    paths = list(sources.iterdir())
    require(len(paths) <= 64, "rootless-apt-source-count")
    paths = [path for path in paths if path.suffix in (".list", ".sources")]
    if os.path.lexists(main):
        paths.append(main)
    docker = []
    for path in paths:
        raw = bounded_read(path, 65536)
        if b"download.docker.com" in raw:
            metadata = os.lstat(path)
            require(stat.S_ISREG(metadata.st_mode) and metadata.st_uid == 0
                    and stat.S_IMODE(metadata.st_mode) == 0o644, "rootless-existing-source-metadata")
            docker.append((path, raw))
    if not docker:
        return False
    canonical = ("deb [arch=arm64 signed-by=" + str(key)
                 + "] https://download.docker.com/linux/ubuntu noble stable\n").encode()
    require(docker == [(sources / "docker.list", canonical)], "rootless-existing-source-shape")
    metadata = os.lstat(key)
    require(stat.S_ISREG(metadata.st_mode) and metadata.st_uid == 0
            and stat.S_IMODE(metadata.st_mode) == 0o644, "rootless-existing-key-metadata")
    bounded_read(key, 65536)
    key_fingerprint(commands.run(["gpg", "--batch", "--with-colons", "--show-keys", str(key)]))
    return True


def guest_recover(nonce, owner_source, commands):
    """Re-attest a completed install whose final observer rejected its report."""
    uid, runtime, _user = guest_base()
    token(nonce)
    require(re.fullmatch(r"[0-9a-f]{64}", owner_source) is not None, "rootless-recovery-source")
    state = Path.home() / ".local/state/rustack-rootless-d103"
    metadata = os.lstat(state)
    require(stat.S_ISDIR(metadata.st_mode) and metadata.st_uid == uid
            and stat.S_IMODE(metadata.st_mode) == 0o700, "rootless-recovery-directory")
    previous = document(bounded_read(state / "entry.json", 65536))
    require(previous == {"nonce": nonce, "scriptSha256": owner_source, "uid": uid}
            and hashlib.sha256(bounded_read(state / "source.py", 65536)).hexdigest() == owner_source,
            "rootless-recovery-owner")
    require(document(bounded_read(state / "failure.json", 65536))
            == {"nonce": nonce, "outcome": "failed-profile-retained"}
            and not os.path.lexists(state / "result.json"), "rootless-recovery-outcome")
    paths = list(state.iterdir())
    require(len(paths) <= 512, "rootless-recovery-file-count")
    intents = sorted(path for path in paths if re.fullmatch(r"command-[0-9]{3}\.intent\.json", path.name))
    require(1 <= len(intents) <= 64, "rootless-recovery-command-count")
    require([path.name for path in intents] == ["command-%03d.intent.json" % number
            for number in range(1, len(intents) + 1)], "rootless-recovery-command-sequence")
    numbered = {path.name for path in paths if path.name.startswith("command-")}
    expected = {"command-%03d%s" % (number, suffix) for number in range(1, len(intents) + 1)
                for suffix in (".intent.json", ".json", ".stdout", ".stderr")}
    require(numbered == expected, "rootless-recovery-command-files")
    installs, observed, rootful, units = 0, [], [], []
    for path in intents:
        intent = document(bounded_read(path, 65536))
        base = state / path.name.removesuffix(".intent.json")
        result = document(bounded_read(base.with_suffix(".json"), 65536))
        require(isinstance(intent, dict) and set(intent) == {"argv", "callerCutoffMonotonic", "ownerPid"}
                and isinstance(result, dict) and set(result) == {"argv", "code", "stdoutSha256", "stderrSha256"}
                and type(result["code"]) is int and result["code"] == 0
                and result["argv"] == intent["argv"], "rootless-recovery-incomplete-command")
        arguments = result["argv"]
        require(isinstance(arguments, list) and 1 <= len(arguments) <= 128
                and all(isinstance(item, str) for item in arguments)
                and sum(len(item.encode()) for item in arguments) <= 32768, "rootless-recovery-command-bound")
        stdout = bounded_read(base.with_suffix(".stdout"), 65536)
        stderr = bounded_read(base.with_suffix(".stderr"), 32768)
        require(hashlib.sha256(stdout).hexdigest() == result["stdoutSha256"]
                and hashlib.sha256(stderr).hexdigest() == result["stderrSha256"], "rootless-recovery-raw-command")
        if arguments[-3:] == ["dockerd-rootless-setuptool.sh", "install", "--force"]:
            installs += 1
        if arguments[:3] == ["docker", "--host", "unix://" + str(runtime / "docker.sock")]:
            observed.append(info(document(stdout)))
        if arguments[:5] == ["docker", "--host", "unix:///var/run/docker.sock", "info", "--format"]:
            rootful.append(stdout.decode("ascii").strip())
        for argument in arguments:
            if argument.startswith("--unit=rustack-d103-"):
                require(re.fullmatch(r"--unit=rustack-d103-[0-9a-f]{32}-[0-9]{3}", argument) is not None,
                        "rootless-recovery-unit")
                units.append(argument[7:] + ".service")
    require(installs == 1 and len(observed) == 2 and observed[0] == observed[1]
            and len(rootful) == 1 and 1 <= len(units) <= 32, "rootless-recovery-install-proof")
    for unit in units:
        require(commands.run(["systemctl", "list-units", "--all", "--plain", "--no-legend", "--full", unit],
                             clean=True) == b"", "rootless-recovery-job-not-retired")
    result = guest_attest(commands)
    require(result["rootless"] == observed[0] and result["rootfulEngineId"] == rootful[0],
            "rootless-recovery-engine-changed")
    result.update(nonce=nonce, rootlessExtrasVersion=result["packages"]["docker-ce-rootless-extras"])
    return result_proof(result, nonce)


def guest_setup(nonce, owner, deadline):
    token(nonce)
    uid, runtime, user = guest_base()
    state = Path.home() / ".local/state/rustack-rootless-d103"
    require(not os.path.lexists(state), "rootless-guest-already-owned")
    require(not os.path.lexists(Path.home() / ".config/systemd/user/docker.service")
            and not os.path.lexists(runtime / "docker.sock")
            and not os.path.lexists(Path.home() / ".local/share/docker"), "rootless-guest-not-fresh")
    state.mkdir(mode=0o700, parents=True)
    write_new(state / "source.py", bounded_read(SCRIPT, MIB))
    publish(state / "entry.json", {"nonce": nonce, "scriptSha256": script_hash(), "uid": uid})
    commands = Commands(owner, deadline, state)
    try:
        for name in ("subuid", "subgid"):
            rows = [line.split(":") for line in bounded_read(Path("/etc") / name, 65536).decode().splitlines()]
            require(any(len(row) == 3 and row[0] == user and row[1].isdigit() and row[2].isdigit()
                        and int(row[1]) >= 65536 and int(row[2]) >= 65536 for row in rows), "rootless-subordinate-ids")
        delegation(uid, commands)
        engine = version(commands.run(["docker", "--host", "unix:///var/run/docker.sock", "version",
                                      "--format", "{{.Server.Version}}"], clean=True))
        key_path = state / "docker.asc"
        with urllib.request.urlopen("https://download.docker.com/linux/ubuntu/gpg", timeout=30) as response:
            require(response.geturl() == "https://download.docker.com/linux/ubuntu/gpg", "rootless-key-origin")
            write_new(key_path, response.read(65537))
        key_fingerprint(commands.run(["gpg", "--batch", "--with-colons", "--show-keys", str(key_path)]))
        if not existing_repository(commands):
            key_target = Path("/etc/apt/keyrings/rustack-d103-docker.asc")
            commands.run(["sudo", "-n", "install", "-d", "-m", "0755", str(key_target.parent)])
            install_file(commands, key_path, key_target, "0644")
            source = state / "docker.sources"
            write_new(source, ("Types: deb\nURIs: https://download.docker.com/linux/ubuntu\nSuites: noble\n"
                               "Components: stable\nArchitectures: arm64\nSigned-By: " + str(key_target) + "\n").encode())
            install_file(commands, source, Path("/etc/apt/sources.list.d/rustack-d103-docker.sources"), "0644")
        apt = ["sudo", "-n", "env", "DEBIAN_FRONTEND=noninteractive", "apt-get",
               "-o", "Acquire::Retries=0", "-o", "Acquire::http::Timeout=30",
               "-o", "Acquire::https::Timeout=30", "-o", "APT::Update::Error-Mode=any"]
        commands.run(apt + ["update"], 300)
        package_version = "5:" + engine + "-1~ubuntu.24.04~noble"
        commands.run(apt + ["install", "-y", "--no-upgrade", "--no-install-recommends", "uidmap",
                           "dbus-user-session", "slirp4netns", "docker-ce-rootless-extras=" + package_version], 300)
        package = commands.run(["dpkg-query", "-W", "-f=${Version}", "docker-ce-rootless-extras"], clean=True).decode()
        require(package == package_version, "rootless-package-version")
        configuration = state / "docker-client"
        configuration.mkdir(mode=0o700)
        owner.environment["DOCKER_CONFIG"] = str(configuration)
        commands.run(["sudo", "-n", "loginctl", "enable-linger", user], 60)
        # Packages add user units after the existing login manager was started.
        # Its private systemd socket works before the session DBus socket exists.
        commands.run(["systemctl", "--user", "daemon-reload"], 60)
        commands.run(["systemctl", "--user", "start", "dbus.socket"], 60)
        commands.run(user_command(runtime, ["dockerd-rootless-setuptool.sh", "install", "--force"]), 60)
        result = guest_attest(commands)
        result.update(nonce=nonce, rootlessExtrasVersion=package)
        result_proof(result, nonce)
        publish(state / "result.json", result)
        return result
    except BaseException:
        publish(state / "failure.json", {"nonce": nonce, "outcome": "failed-profile-retained"})
        raise


def host(mode, selected, target_or_evidence, evidence_arg, owner, deadline):
    selected = profile(selected)
    require(selected.startswith("rustack-workflows-rootless-"), "rootless-profile-name")
    creating = mode in ("provision", "resume", "recover-attest")
    recovery_source = None
    evidence = safe_path(evidence_arg if creating else target_or_evidence)
    if creating:
        require(sys.platform == "darwin" and platform.machine() == "arm64", "provision-platform")
        require(not os.path.lexists(evidence), "rootless-existing-evidence")
        previous = None
        if mode == "provision":
            require(not os.path.lexists(Path.home() / ".colima" / selected), "existing-profile")
            target = safe_path(target_or_evidence)
        else:
            prior = safe_path(target_or_evidence)
            metadata = os.lstat(prior)
            require(stat.S_ISDIR(metadata.st_mode) and metadata.st_uid == os.getuid()
                    and stat.S_IMODE(metadata.st_mode) == 0o700, "rootless-evidence-directory")
            previous = bounded_read(prior / "entry.json", 65536)
            old = entry(document(previous), selected, current_source=False)
            target = safe_path(old["targetDirectory"])
        require(target.is_dir(), "target-directory")
        evidence.mkdir(mode=0o700)
        write_new(evidence / "source.py", bounded_read(SCRIPT, MIB))
        receipt = {"schemaVersion": 1, "profile": selected, "nonce": old["nonce"] if mode == "recover-attest" else uuid.uuid4().hex,
                   "scriptSha256": script_hash(), "targetDirectory": str(target)}
        publish(evidence / "entry.json", receipt)
        if mode == "provision":
            provision(selected, str(target), min(deadline, time.monotonic() + 330), owner)
        else:
            publish(evidence / ("recovery-parent.json" if mode == "recover-attest" else "resume-parent.json"), {"previousEvidence": str(prior),
                    "previousEntrySha256": hashlib.sha256(previous).hexdigest(), "previousEntry": old})
            if mode == "recover-attest":
                recovery_source = old["scriptSha256"]
            else:
                fresh = document(Commands(owner, deadline).run(["colima", "-p", selected, "ssh", "--",
                    "python3", "-I", "-B", str(SCRIPT), "guest-fresh", receipt["nonce"]], 30, clean=True))
                require(isinstance(fresh, dict) and set(fresh) == {"nonce", "uid", "scriptSha256", "rootlessStateAbsent"}
                        and fresh["nonce"] == receipt["nonce"] and fresh["scriptSha256"] == script_hash()
                        and type(fresh["uid"]) is int and 1 <= fresh["uid"] <= 65535
                        and fresh["rootlessStateAbsent"] is True, "rootless-fresh-proof")
                publish(evidence / "fresh-proof.json", fresh)
    else:
        metadata = os.lstat(evidence)
        require(stat.S_ISDIR(metadata.st_mode) and metadata.st_uid == os.getuid()
                and stat.S_IMODE(metadata.st_mode) == 0o700, "rootless-evidence-directory")
        receipt = entry(document(bounded_read(evidence / "entry.json", 65536)), selected)
        if (evidence / "recovery-parent.json").exists():
            parent = document(bounded_read(evidence / "recovery-parent.json", 65536))
            old = entry(parent["previousEntry"], selected, current_source=False)
            require(old["nonce"] == receipt["nonce"], "rootless-recovery-nonce")
            recovery_source = old["scriptSha256"]
    configuration = status(owner, selected, deadline)
    installing = mode in ("provision", "resume")
    commands = Commands(owner, deadline, evidence if creating else None)
    guest_mode = "guest-recover" if recovery_source else "guest-setup" if installing else "guest-attest"
    raw = commands.run(["colima", "-p", selected, "ssh", "--", "python3", "-I", "-B", str(SCRIPT),
                        guest_mode, receipt["nonce"]] + ([recovery_source] if recovery_source else []),
                       1100 if installing else 70, clean=True)
    result = result_proof(document(raw), receipt["nonce"])
    require(script_hash() == receipt["scriptSha256"] and status(owner, selected, deadline) == configuration,
            "rootless-host-source-or-profile-changed")
    result.update(profile=selected, profileConfiguration=configuration, profileRetained=True)
    publish(evidence / ("result.json" if creating else "attestation-" + uuid.uuid4().hex + ".json"), result)
    return result


def main():
    try:
        mode = sys.argv[1] if len(sys.argv) > 1 else ""
        require((mode in ("provision", "resume", "recover-attest") and len(sys.argv) == 5)
                or (mode == "attest" and len(sys.argv) == 4)
                or (mode == "guest-recover" and len(sys.argv) == 4)
                or (mode in ("guest-setup", "guest-attest", "guest-fresh") and len(sys.argv) == 3), "rootless-arguments")
        owner = Runner(os.environ)
        with deadline_guard({"provision": 1500, "resume": 1200, "recover-attest": 90, "attest": 90, "guest-setup": 1100,
                             "guest-attest": 65, "guest-recover": 65, "guest-fresh": 10}[mode], owner) as deadline:
            if mode == "guest-setup":
                result = guest_setup(sys.argv[2], owner, deadline)
            elif mode == "guest-attest":
                nonce = token(sys.argv[2])
                state = Path.home() / ".local/state/rustack-rootless-d103"
                previous = result_proof(document(bounded_read(state / "result.json", 65536)), nonce)
                result = guest_attest(Commands(owner, deadline))
                require(result["rootless"]["id"] == previous["rootless"]["id"]
                        and result["rootfulEngineId"] == previous["rootfulEngineId"], "rootless-engine-changed")
                result.update(nonce=nonce, rootlessExtrasVersion=previous["rootlessExtrasVersion"])
                result_proof(result, nonce)
            elif mode == "guest-fresh":
                result = guest_fresh(sys.argv[2])
            elif mode == "guest-recover":
                result = guest_recover(sys.argv[2], sys.argv[3], Commands(owner, deadline))
            else:
                result = host(mode, sys.argv[2], sys.argv[3], sys.argv[4] if mode in ("provision", "resume", "recover-attest") else None,
                              owner, deadline)
            require(time.monotonic() < deadline, "rootless-late-result")
            print(json.dumps(result, separators=(",", ":"), allow_nan=False), flush=True)
        return 0
    except (ActionError, QualificationError) as error:
        print("rootless-qualification-failed:" + str(error), file=sys.stderr)
        return 2
    except (OSError, ValueError, KeyError, TypeError, RecursionError):
        print("rootless-qualification-failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
