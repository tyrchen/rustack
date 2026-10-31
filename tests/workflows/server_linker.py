#!/usr/bin/env python3
"""Translate the one obsolete GNU optimization argument for Zig0.16 only."""
import os
from pathlib import Path
import sys


def command(arguments, environment):
    """Keep every effective linker option and diagnostic; never invoke a shell."""
    if environment.get("CARGO_ZIGBUILD_ZIG_VERSION") != "0.16.0":
        raise ValueError("unsupported server linker compatibility version")
    target = environment.get("WORKFLOWS_SERVER_LINKER_TARGET")
    if target not in ("aarch64-linux-gnu", "x86_64-linux-gnu"):
        raise ValueError("unsupported server linker target")
    program = environment.get("WORKFLOWS_SERVER_LINKER_TOOL", "")
    if not (0 < len(os.fsencode(program)) <= 4096 and Path(program).is_absolute()
            and not any(ord(character) < 32 or ord(character) == 127 for character in program)):
        raise ValueError("invalid explicit server linker tool")
    if not (1 <= len(arguments) <= 4096
            and all(isinstance(arg, str) and len(os.fsencode(arg)) <= 16384 and "\x00" not in arg
                    for arg in arguments)
            and sum(len(os.fsencode(arg)) + 1 for arg in arguments) <= 4 * 1024 * 1024):
        raise ValueError("server linker argv bound")
    filtered = [arg for arg in arguments if arg != "-Wl,-O1"]
    return program, [program, "zig", "cc", "--", "-g", "-fno-sanitize=all", "-target", target, *filtered]


def main():
    try:
        program, arguments = command(sys.argv[1:], os.environ)
        os.execv(program, arguments)
    except (OSError, ValueError):
        print("server linker compatibility boundary failed", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
