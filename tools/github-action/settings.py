"""Validated composite-Action inputs; opaque authentication values stay private."""
from dataclasses import dataclass, field
import re


class ActionError(Exception):
    """A fixed diagnostic code, never an external value or subprocess output."""


SERVICES = frozenset(("s3", "dynamodb", "dynamodbstreams", "sqs", "ssm", "sns", "lambda",
                      "stepfunctions", "events", "logs", "kms", "kinesis", "secretsmanager",
                      "ses", "apigatewayv2", "cloudwatch", "iam", "sts", "cloudfront"))
FLAGS = ("s3", "dynamodb", "sqs", "ssm", "sns", "lambda", "stepfunctions")
DEFAULTS = {"image-tag": "latest", "port": "4566", "default-region": "us-east-1",
            "log-level": "info", "wait-timeout": "30", "access-key": "test", "secret-key": "test",
            "persistence": "false", "services": "",
            **{service + "-skip-auth": "true" for service in FLAGS}}


@dataclass(frozen=True)
class Authentication:
    access: str = field(repr=False)
    secret: str = field(repr=False)


@dataclass(frozen=True)
class Settings:
    tag: str
    port: int
    region: str
    log_level: str
    wait_seconds: int
    authentication: Authentication
    flags: tuple
    persistence: str
    services: str

    @classmethod
    def load(cls, environment):
        values = {}
        for key, default in DEFAULTS.items():
            value = environment.get("RUSTACK_ACTION_" + key.upper().replace("-", "_"), default)
            if not isinstance(value, str):
                raise ActionError("input-" + key)
            try:
                raw = value.encode("ascii")
            except UnicodeEncodeError:
                raise ActionError("input-" + key) from None
            if len(raw) > 512 or any(byte < 32 or byte > 126 for byte in raw):
                raise ActionError("input-" + key)
            values[key] = value
        def require(key, pattern):
            if re.fullmatch(pattern, values[key]) is None:
                raise ActionError("input-" + key)
        def decimal(key, maximum, digits):
            require(key, "[1-9][0-9]{0," + str(digits - 1) + "}")
            number = int(values[key])
            if number > maximum:
                raise ActionError("input-" + key)
            return number
        require("image-tag", r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}")
        port = decimal("port", 65535, 5)
        wait = decimal("wait-timeout", 300, 3)
        require("default-region", r"[a-z0-9][a-z0-9-]{0,63}")
        if values["log-level"] not in ("error", "warn", "info", "debug", "trace"):
            raise ActionError("input-log-level")
        for key in ["persistence", *[name + "-skip-auth" for name in FLAGS]]:
            if values[key] not in ("true", "false"):
                raise ActionError("input-" + key)
        selected = values["services"].split(",") if values["services"] else []
        if len(selected) > 32 or len(selected) != len(set(selected)) or any(s not in SERVICES for s in selected):
            raise ActionError("input-services")
        access, secret = values["access-key"], values["secret-key"]
        if bool(access) != bool(secret):
            raise ActionError("input-authentication-pair")
        enabled = set(selected) if selected else SERVICES
        if not access and any(service in enabled and values[service + "-skip-auth"] == "false" for service in FLAGS):
            raise ActionError("input-strict-authentication-pair")
        return cls(values["image-tag"], port, values["default-region"], values["log-level"], wait,
                   Authentication(access, secret), tuple((s, values[s + "-skip-auth"]) for s in FLAGS),
                   values["persistence"], values["services"])

    @property
    def reference(self):
        return "ghcr.io/tyrchen/rustack:" + self.tag

    @property
    def endpoint(self):
        return "http://localhost:" + str(self.port)

    def docker_environment(self):
        values = {"DEFAULT_REGION": self.region, "LOG_LEVEL": self.log_level,
                  "PERSISTENCE": self.persistence, "SERVICES": self.services}
        if self.authentication.access:
            values.update(ACCESS_KEY=self.authentication.access, SECRET_KEY=self.authentication.secret)
        values.update((name.upper() + "_SKIP_SIGNATURE_VALIDATION", value) for name, value in self.flags)
        return values

    def caller_environment(self):
        return {"AWS_ENDPOINT_URL": self.endpoint, "LOCALSTACK_ENDPOINT": self.endpoint,
                "AWS_DEFAULT_REGION": self.region, "AWS_ACCESS_KEY_ID": self.authentication.access,
                "AWS_SECRET_ACCESS_KEY": self.authentication.secret}
