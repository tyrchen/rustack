"""Build original upstream handlers into individually owned immutable RIC images."""
import json
import os
from pathlib import Path
import sys
import uuid
import shutil

sys.path.insert(0, str(Path(__file__).resolve().parent))

from localstack_policy import VENDOR, digest, manifest, publish, read, require, upstream
from linux_performance import Runner, deadline_guard
from image_performance_images import catalog, fenced, image, retain_content

BASE = "public.ecr.aws/lambda/python:3.12@sha256:ab3f7e33c838a04d8750648f4cd1250ef8c20f6137904a3bbccd90da95e4157a"
HANDLERS = (
    "tests/aws/services/stepfunctions/templates/services/lambdafunctions/id_function.py",
    "tests/aws/services/stepfunctions/templates/services/lambdafunctions/return_bytes_str.py",
    "tests/aws/services/stepfunctions/templates/errorhandling/lambdafunctions/raise_exception.py",
    "tests/aws/services/stepfunctions/templates/errorhandling/lambdafunctions/raise_custom_exception.py",
    "tests/aws/services/stepfunctions/templates/timeouts/lambdafunctions/wait_60_seconds.py",
    "tests/aws/services/stepfunctions/lambda_functions/base/echo_function.py",
)


def recipe():
    return ("FROM " + BASE + "\nCOPY --chmod=0444 handler.py /var/task/handler.py\n"
            "USER 1000:1000\nCMD [\"handler.handler\"]\n").encode()


def build_arguments(builder, context, name):
    return [str(builder), "build", "--load", "--provenance=false", "--progress=quiet",
            "--metadata-file", str(context / "metadata.json"), "--platform", "linux/arm64",
            "--tag", name, str(context)]


def validate_builds(root, value):
    original = json.loads(read(root / "intent.json"))
    require(original["engine"] == value["engine"] and original["endpoint"] == value["endpoint"]
            and original["manifestSha256"] == value["manifestSha256"] and original["base"] == BASE,
            "compat-image-build-affiliation")
    builder = Path(value["builderExecutable"])
    require(builder.is_absolute(), "compat-image-builder")
    for index, source in enumerate(HANDLERS):
        context = root / ("handler-%d" % index)
        require(not context.is_symlink(), "compat-image-context-substitution")
        row = value["handlers"][source]
        saved = read(context / "result.json")
        require(digest(saved) == row["buildResultSha256"]
                and json.loads(saved) == {k:v for k,v in row.items() if k != "buildResultSha256"},
                "compat-image-result-affiliation")
        raw = read(VENDOR / source, 65536)
        require(read(context / "handler.py", 65536) == raw and read(context / "Dockerfile") == recipe()
                and digest(raw) == row["handlerSha256"], "compat-image-build-inputs")
        intent_raw = read(context / "build-intent.json")
        intent = json.loads(intent_raw)
        name = "rustack-localstack-compat:" + original["nonce"] + "-" + str(index)
        require(intent == {"argv":build_arguments(builder, context, name), "engine":value["engine"],
                "source":source, "handlerSha256":digest(raw), "dockerfileSha256":digest(recipe())}
                and digest(intent_raw) == row["buildIntentSha256"], "compat-image-build-intent")
        metadata_raw = read(context / "metadata.json", 65536)
        metadata = json.loads(metadata_raw)
        require(row["imageId"] in (metadata.get("containerimage.digest"), metadata.get("containerimage.config.digest"))
                and digest(metadata_raw) == row["metadataSha256"]
                and digest(read(context / "build.stdout", 65536)) == row["buildStdoutSha256"],
                "compat-image-build-output")


def validate_receipt(path, endpoint, owner, deadline):
    value = json.loads(read(path))
    require(value.get("schemaVersion") == 1 and value.get("allChildGroupsJoined") is True
            and value.get("endpoint") == endpoint and value.get("base") == BASE
            and value.get("manifestSha256") == manifest()[1]
            and set(value.get("handlers", {})) == set(HANDLERS), "compat-image-receipt")
    validate_builds(path.parent, value)
    engine = value["engine"]
    for source, row in value["handlers"].items():
        require(digest(read(VENDOR / source)) == row["handlerSha256"]
                and image(owner, endpoint, row["imageId"], deadline, engine) == row["image"]
                and row["image"]["id"] == row["imageId"], "compat-image-live-binding")
    return value


def run(root, socket, builder, owner, deadline):
    upstream()
    _, hashed = manifest()
    require(root.is_absolute() and socket.is_absolute() and builder.is_absolute()
            and not root.exists() and builder.is_file(), "compat-image-paths")
    root.mkdir(mode=0o700)
    endpoint = "unix://" + str(socket)
    result = owner.run(["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline)
    require(result.code == 0 and not result.stderr, "compat-image-engine")
    engine = result.stdout.decode().strip()
    original = catalog(owner, endpoint, deadline, engine)
    nonce = uuid.uuid4().hex
    publish(root / "intent.json", {"engine":engine, "endpoint":endpoint, "manifestSha256":hashed,
        "originalIds":sorted(original), "base":BASE, "nonce":nonce})
    handlers = {}
    owner.environment["DOCKER_HOST"] = endpoint
    for index, source in enumerate(HANDLERS):
        context = root / ("handler-%d" % index)
        context.mkdir(mode=0o700)
        raw = read(VENDOR / source, 65536)
        (context / "handler.py").write_bytes(raw)
        os.chmod(context / "handler.py", 0o444)
        recipe_raw = recipe()
        (context / "Dockerfile").write_bytes(recipe_raw)
        name = "rustack-localstack-compat:" + nonce + "-" + str(index)
        existing = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "ls", "-a",
            "--filter", "reference=" + name, "--format", "{{.ID}}"], deadline)
        require(not existing, "compat-image-tag-conflict")
        arguments = build_arguments(builder, context, name)
        publish(context / "build-intent.json", {"argv":arguments, "engine":engine,
            "source":source, "handlerSha256":digest(raw), "dockerfileSha256":digest(recipe_raw)})
        output = fenced(owner, endpoint, engine, arguments, deadline, 180)
        (context / "build.stdout").write_bytes(output)
        metadata = image(owner, endpoint, name, deadline, engine)
        build_metadata = json.loads(read(context / "metadata.json", 65536))
        require(metadata["id"] in (build_metadata.get("containerimage.digest"),
                build_metadata.get("containerimage.config.digest")), "compat-image-addressable-build-output")
        alias = retain_content(owner, endpoint, metadata["id"], deadline, engine)
        handlers[source] = {"imageId":metadata["id"], "image":metadata, "handlerSha256":digest(raw),
            "tag":name, "cacheAlias":alias, "metadataSha256":digest(read(context / "metadata.json", 65536)),
            "buildIntentSha256":digest(read(context / "build-intent.json")), "buildStdoutSha256":digest(output)}
        publish(context / "result.json", handlers[source])
        handlers[source]["buildResultSha256"] = digest(read(context / "result.json"))
    upstream()
    require(manifest()[1] == hashed and original <= catalog(owner, endpoint, deadline, engine),
            "compat-image-postconditions")
    publish(root / "images.json", {"schemaVersion":1, "engine":engine, "endpoint":endpoint,
        "manifestSha256":hashed, "base":BASE, "handlers":handlers,
        "dockerExecutable":shutil.which("docker"), "builderExecutable":str(builder),
        "originalIds":sorted(original), "resultIds":sorted(catalog(owner, endpoint, deadline, engine)),
        "allChildGroupsJoined":not owner.unsettled})


if __name__ == "__main__":
    try:
        require(len(sys.argv) == 4, "compat-image-arguments")
        owner = Runner(os.environ)
        with deadline_guard(1200, owner) as deadline:
            run(Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), owner, deadline)
    except (ValueError, OSError, KeyError, TypeError):
        print("localstack-handler-images-rejected", file=sys.stderr)
        sys.exit(2)
