"use strict";

// Image-build-only: Go's DIY backend requires an actual POSIX user lookup.
const fs = require("node:fs");
const assert = require("node:assert/strict");
assert.equal(process.getuid(), 0);
for (const [file, entry, count] of [
    ["/etc/passwd", "rustack-iac:x:1000:1000:Rustack IaC:/tmp:/sbin/nologin", 7],
    ["/etc/group", "rustack-iac:x:1000:", 4],
]) {
    const source = fs.readFileSync(file);
    assert.ok(source.length <= 65536);
    const records = source.toString("ascii").split("\n").filter(Boolean).map(value => value.split(":"));
    assert.ok(records.every(fields => fields.length === count));
    assert.ok(records.every(fields => fields[0] !== "rustack-iac" && fields[2] !== "1000"));
    fs.appendFileSync(file, entry + "\n");
}
