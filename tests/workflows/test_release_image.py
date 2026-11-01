"""Actual saved-graph and filesystem contract controls, without a daemon."""
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import time
import tarfile
import unittest
from unittest.mock import patch

import release_image as subject


# Independent literals from release-image design §3.3, not the verifier table.
FILESYSTEM = {
    "rustack": (0, 0, 0o755, "file"),
    "etc": (0, 0, 0o755, "directory"),
    "etc/ssl": (0, 0, 0o755, "directory"),
    "etc/ssl/certs": (0, 0, 0o755, "directory"),
    "etc/ssl/certs/ca-certificates.crt": (0, 0, 0o644, "file"),
    "tmp": (65532, 65532, 0o1777, "directory"),
    "data": (65532, 65532, 0o700, "directory"),
    "run": (0, 0, 0o555, "directory"),
    "run/rustack": (0, 0, 0o555, "directory"),
    "run/rustack/docker.sock": (0, 0, 0o444, "file"),
}
ENVIRONMENT = ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
               "GATEWAY_LISTEN=0.0.0.0:4566", "LOG_LEVEL=info", "SERVICES="]


def digest(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def encoded(value):
    return json.dumps(value, separators=(",", ":")).encode()


def archive(rows):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w", format=tarfile.USTAR_FORMAT) as saved:
        for name, body, uid, gid, mode, kind in rows:
            header = tarfile.TarInfo(name)
            header.uid, header.gid, header.mode = uid, gid, mode
            header.type = tarfile.DIRTYPE if kind == "directory" else tarfile.REGTYPE
            header.size = len(body)
            saved.addfile(header, io.BytesIO(body))
    return output.getvalue()


def physical(name, kind, body):
    header = tarfile.TarInfo(name)
    header.type, header.size, header.mode = kind, len(body), 0o644
    return header.tobuf(format=tarfile.USTAR_FORMAT) + body + b"\0" * ((-len(body)) % 512)


def pax(key, value):
    payload = key + b"=" + value + b"\n"
    size = len(payload) + 2
    for _ in range(5):
        line = str(size).encode() + b" " + payload
        if len(line) == size:
            return line
        size = len(line)
    raise AssertionError("fixture PAX length")


def image(architecture="arm64", compressed=True, change=None, graph_id=True):
    binary = bytearray(120)
    binary[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<HHI", binary, 16, 2, {"arm64":183, "amd64":62}[architecture], 1)
    struct.pack_into("<Q", binary, 32, 64)
    struct.pack_into("<HH", binary, 54, 56, 1)
    struct.pack_into("<I", binary, 64, 1)
    rows = [(name, bytes(binary) if name == "rustack" else b"ca-fixture" if name.endswith(".crt") else b"",
             uid, gid, mode, kind) for name,(uid,gid,mode,kind) in FILESYSTEM.items()]
    health = dict(Test=["CMD", "/rustack", "--health-check"], Interval=2_000_000_000,
                  Timeout=4_000_000_000, StartPeriod=1_000_000_000, Retries=3)
    config = dict(architecture=architecture, os="linux", config=dict(User="65532:65532", WorkingDir="/data",
        Entrypoint=["/rustack"], Env=list(ENVIRONMENT), Healthcheck=health),
        rootfs=dict(type="layers", diff_ids=[]))
    state = dict(rows=rows, config=config, layer_suffix=b"", outer_suffix=b"", diff=None)
    if change:
        change(state)
    layer = state.get("layer_prefix", b"") + archive(state["rows"]) + state["layer_suffix"]
    stored = gzip.compress(layer, mtime=0) if compressed else layer
    if "stored_change" in state:
        stored = state["stored_change"](stored)
    config["rootfs"]["diff_ids"] = [state["diff"] or digest(layer)]
    config_raw = encoded(config)
    config_digest, layer_digest = digest(config_raw), digest(stored)
    graph_raw = encoded(dict(schemaVersion=2, config=dict(digest=config_digest, size=len(config_raw)),
                            layers=[dict(digest=layer_digest, size=len(stored))]))
    graph_digest = digest(graph_raw)
    path = lambda digest: "blobs/sha256/" + digest[7:]
    manifest = encoded([dict(Config=path(config_digest), Layers=[path(layer_digest)], RepoTags=None)])
    rows = [("blobs", b"", 0, 0, 0o755, "directory"),
            ("blobs/sha256", b"", 0, 0, 0o755, "directory")]
    rows += [(name, data, 0, 0, 0o644, "file") for name,data in
             [("manifest.json",manifest),(path(config_digest),config_raw),
              (path(layer_digest),stored),(path(graph_digest),graph_raw)]]
    return archive(rows) + state["outer_suffix"], graph_digest if graph_id else config_digest


def replace_row(state, name, **changes):
    index = next(i for i,row in enumerate(state["rows"]) if row[0] == name)
    values = dict(zip(("name", "body", "uid", "gid", "mode", "kind"), state["rows"][index]))
    values.update(changes)
    state["rows"][index] = tuple(values[key] for key in ("name", "body", "uid", "gid", "mode", "kind"))


class ReleaseArtifactControls(unittest.TestCase):
    def test_should_bind_both_elf_machines_gzip_plain_and_manifest_or_config_identity(self):
        for architecture in ("arm64", "amd64"):
            for compressed in (True, False):
                for graph_id in (True, False):
                    raw, identity = image(architecture, compressed, graph_id=graph_id)
                    report = subject.verify(raw, architecture, identity)
                    self.assertEqual(report["image"], identity)
                    self.assertEqual(report["elfMachine"], {"arm64":183, "amd64":62}[architecture])
                    self.assertFalse(report["elfInterpreter"])

    def test_should_reject_actual_top_level_copy_mode_regression_and_wrong_ownership(self):
        for name,changes in [("data", dict(mode=0o755)), ("tmp", dict(mode=0o755)),
                             ("run", dict(mode=0o755)), ("data", dict(uid=0)),
                             ("tmp", dict(gid=0)), ("run/rustack/docker.sock", dict(mode=0o666))]:
            raw, identity = image(change=lambda state: replace_row(state, name, **changes))
            with self.subTest(name=name,changes=changes), self.assertRaises(subject.ImageError):
                subject.verify(raw, "arm64", identity)

    def test_should_reject_extra_duplicate_unsafe_or_nonempty_directory_paths(self):
        for name,body,kind in [("extra",b"unexpected","file"), ("tmp",b"","directory"),
                               ("../outside",b"x","file"), ("/absolute",b"x","file"),
                               ("empty",b"not-empty","directory")]:
            raw, identity = image(change=lambda state: state["rows"].append((name,body,0,0,0o755,kind)))
            with self.subTest(name=name), self.assertRaises(subject.ImageError):
                subject.verify(raw, "arm64", identity)

    def test_should_reject_changed_platform_program_headers_interpreter_and_executable(self):
        def changed(state, offset, value):
            binary = bytearray(state["rows"][0][1]);binary[offset:offset+len(value)] = value
            replace_row(state, "rustack", body=bytes(binary))
        for offset,value in [(18,struct.pack("<H",62)),(64,struct.pack("<I",3)),(0,b"nope"),
                             (54,struct.pack("<H",0)),(56,struct.pack("<H",129))]:
            raw, identity = image(change=lambda state: changed(state, offset, value))
            with self.subTest(offset=offset), self.assertRaises(subject.ImageError):
                subject.verify(raw, "arm64", identity)
        raw, identity = image()
        with self.assertRaises(subject.ImageError):
            subject.verify(raw, "amd64", identity)

    def test_should_reject_changed_image_user_command_health_and_credential_environment(self):
        for key,value in [("User","0:0"),("Cmd",["shell"]),("Volumes",{"/extra":{}}),
                          ("Env",ENVIRONMENT+["SECRET_KEY=must-not-appear"]),
                          ("OnBuild",["RUN untrusted"])]:
            raw, identity = image(change=lambda state: state["config"]["config"].update({key:value}))
            with self.subTest(key=key), self.assertRaises(subject.ImageError):
                subject.verify(raw, "arm64", identity)
        for key,value in [("Test",["NONE"]),("Timeout",1),("StartInterval",False),("Retries",True)]:
            raw, identity = image(change=lambda state: state["config"]["config"]["Healthcheck"].update({key:value}))
            with self.subTest(key=key), self.assertRaises(subject.ImageError):
                subject.verify(raw, "arm64", identity)

    def test_should_reject_graph_and_layer_digest_substitution(self):
        raw, identity = image()
        with self.assertRaises(subject.ImageError):
            subject.verify(raw, "arm64", digest(b"foreign"))
        raw, identity = image(change=lambda state: state.update(diff=digest(b"wrong layer")))
        with self.assertRaises(subject.ImageError):
            subject.verify(raw, "arm64", identity)
        altered = bytearray(raw)
        # Modify payload while retaining a canonical checksum-valid outer tar header.
        with tarfile.open(fileobj=io.BytesIO(raw),mode="r:") as saved:
            member = next(m for m in saved if m.name.startswith("blobs/sha256/") and m.isfile())
        altered[member.offset_data] ^= 1
        with self.assertRaises(subject.ImageError):
            subject.verify(bytes(altered), "arm64", identity)

    def test_should_reject_truncated_and_concatenated_archives(self):
        for change in (lambda state: state.update(layer_suffix=b"x"*512),
                       lambda state: state.update(outer_suffix=b"x"*512)):
            raw, identity = image(change=change)
            with self.assertRaises(subject.ImageError):
                subject.verify(raw, "arm64", identity)
        raw, identity = image()
        for truncated in (raw[:512],raw[:-1]):
            with self.assertRaises(subject.ImageError):
                subject.verify(truncated,"arm64",identity)

    def test_should_bound_saved_and_expanded_bytes_before_acceptance(self):
        raw, identity = image()
        with patch.object(subject,"ARCHIVE_LIMIT",len(raw)-1), self.assertRaises(subject.ImageError):
            subject.verify(raw,"arm64",identity)
        raw, identity = image(change=lambda state: replace_row(state,"etc/ssl/certs/ca-certificates.crt",body=b"x"*65536))
        with patch.object(subject,"EXPANDED_LIMIT",16384), self.assertRaises(subject.ImageError):
            subject.verify(raw,"arm64",identity)

    def test_should_normalize_truncated_gzip_and_crc_failures_without_tracebacks(self):
        for change in (lambda raw: raw[:2], lambda raw: raw[:-4],
                       lambda raw: raw[:-8] + bytes([raw[-8] ^ 1]) + raw[-7:],
                       lambda raw: raw[:10] + b"\x07" + raw[-8:]):
            raw, identity = image(change=lambda state: state.update(stored_change=change))
            with self.assertRaises(subject.ImageError) as caught:
                subject.verify(raw, "arm64", identity)
            self.assertEqual(str(caught.exception), "image-archive-invalid")

    def test_should_enforce_the_inherited_kernel_file_limit_during_child_output(self):
        runner = subject.Runner({"PATH": os.defpath})
        with tempfile.TemporaryDirectory() as name:
            target = str(Path(name) / "limited")
            writer = ("import os,errno; p=" + repr(target) + ";"
                      "fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);"
                      "assert os.write(fd,b'x'*8192)==4096;\n"
                      "try: os.write(fd,b'x')\n"
                      "except OSError as error: assert error.errno==errno.EFBIG\n"
                      "else: raise AssertionError('file bound missing')\n"
                      "finally: os.close(fd)\n")
            tools = str(Path(sys.modules[subject.document.__module__].__file__).parent)
            code = ("import sys,os,time; sys.path[:0]="
                    + repr([str(Path(subject.__file__).parent), tools])
                    + ";import release_image as s;s.ARCHIVE_LIMIT=4096;s._limit_file_output();"
                    "r=s.Runner({'PATH':os.defpath});reply=r.run("
                    + repr([sys.executable,"-B","-c",writer])
                    + ",time.monotonic()+3);assert reply.code==0;print('inherited bound enforced')")
            reply = runner.run([sys.executable,"-B","-c",code],time.monotonic()+5)
            self.assertEqual(reply.code,0)
            self.assertEqual(reply.stdout,b"inherited bound enforced\n")
            self.assertEqual(Path(target).stat().st_size,4096)

    def test_should_accept_one_bounded_timing_pax_without_changing_member_identity(self):
        prefix = physical("PaxHeaders.0/rustack",tarfile.XHDTYPE,pax(b"mtime",b"1.25"))
        raw, identity = image(change=lambda state: state.update(layer_prefix=prefix))
        report = subject.verify(raw,"arm64",identity)
        self.assertEqual(report["filesystem"]["data"]["mode"],0o700)

    def test_should_reject_hidden_gnu_global_or_chained_pax_before_normalized_iteration(self):
        timing = pax(b"mtime",b"1")
        prefixes = [physical("gnudata",tarfile.GNUTYPE_LONGNAME,b"rustack\0"),
                    physical("gnulink",tarfile.GNUTYPE_LONGLINK,b"\0"),
                    physical("global",tarfile.XGLTYPE,timing),
                    physical("local-one",tarfile.XHDTYPE,timing)
                    + physical("local-two",tarfile.XHDTYPE,pax(b"atime",b"1")),
                    physical("local",tarfile.XHDTYPE,pax(b"path",b"rustack")),
                    physical("large",tarfile.XHDTYPE,b"x"*2048)]
        for prefix in prefixes:
            raw, identity = image(change=lambda state: state.update(layer_prefix=prefix))
            with self.subTest(kind=prefix[156:157]), self.assertRaises(subject.ImageError):
                subject.verify(raw,"arm64",identity)
        header = tarfile.TarInfo("huge")
        header.size = 0x7fffffff
        raw = header.tobuf(format=tarfile.USTAR_FORMAT) + b"\0"*1024
        with self.assertRaises(subject.ImageError):
            subject.Contents(raw,128)

    def test_should_reject_links_devices_and_fifos_without_extraction(self):
        for kind in (tarfile.SYMTYPE,tarfile.LNKTYPE,tarfile.CHRTYPE,tarfile.FIFOTYPE):
            output=io.BytesIO()
            with tarfile.open(fileobj=output,mode="w") as saved:
                header=tarfile.TarInfo("target");header.type=kind;header.linkname="outside" if kind in (tarfile.SYMTYPE,tarfile.LNKTYPE) else ""
                saved.addfile(header)
            with self.subTest(kind=kind), self.assertRaises(subject.ImageError):
                subject.Contents(output.getvalue(),128)


if __name__ == "__main__":
    unittest.main()
