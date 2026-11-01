"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const { randomUUID } = require("node:crypto");
const { performance } = require("node:perf_hooks");
const { read } = require("./evidence_reader");

async function workspace(run) {
    const directory = "/tmp/iac-" + randomUUID();
    await fs.mkdir(directory, { mode: 0o700 });
    try { await fs.mkdir(directory + "/logs"); return await run(directory); }
    finally { await fs.rm(directory, { recursive: true }); }
}
test("test_should_read_actual_files_into_bounded_complete_ustar_without_mutation", async () => workspace(async directory => {
    const data = Buffer.from("actual independent file bytes");
    await fs.writeFile(directory + "/logs/4-create.stderr", data, { mode: 0o600 });
    const chunks = [];
    await read(directory, "logs", async value => { chunks.push(value); }, performance.now() + 3000);
    const archive = Buffer.concat(chunks);
    assert.equal(archive.subarray(0, 4).toString(), "logs");
    assert.equal(archive.subarray(257, 263).toString(), "ustar\0");
    assert.equal(archive.subarray(1024, 1024 + data.length).toString(), data.toString());
    assert.deepEqual(archive.subarray(-1024), Buffer.alloc(1024));
    assert.deepEqual(await fs.readFile(directory + "/logs/4-create.stderr"), data);
    assert.deepEqual(await fs.readdir(directory + "/logs"), ["4-create.stderr"]);
}));
test("test_should_reject_evidence_links_oversized_files_and_expired_reader", async () => workspace(async directory => {
    await fs.symlink("/etc/passwd", directory + "/logs/link");
    await assert.rejects(read(directory, "logs", async () => {}, performance.now() + 3000));
    await fs.unlink(directory + "/logs/link");
    const handle = await fs.open(directory + "/logs/large", "wx", 0o600);
    try { await handle.truncate(2 * 1024 * 1024 + 1); } finally { await handle.close(); }
    await assert.rejects(read(directory, "logs", async () => {}, performance.now() + 3000));
    await assert.rejects(read(directory, "logs", async () => {}, performance.now() - 1));
}));
test("test_should_refuse_excessive_entries_and_non_fixture_roots", async () => workspace(async directory => {
    for (let index = 0; index < 129; index += 1) await fs.writeFile(directory + "/logs/" + index, "");
    await assert.rejects(read(directory, "logs", async () => {}, performance.now() + 3000));
    await assert.rejects(read(directory, "project", async () => {}, performance.now() + 3000));
    await assert.rejects(read("/tmp/unrelated", "logs", async () => {}, performance.now() + 3000));
}));

test("test_should_encode_real_long_backend_paths_using_ustar_prefix", async () => workspace(async directory => {
    const component = "a".repeat(70), name = "b".repeat(50) + ".stdout";
    await fs.mkdir(directory + "/logs/" + component);
    await fs.writeFile(directory + "/logs/" + component + "/" + name, "long actual file");
    const chunks = [];
    await read(directory, "logs", async value => { chunks.push(value); }, performance.now() + 3000);
    const raw = Buffer.concat(chunks), file = raw.subarray(1024, 1536);
    assert.equal(file.subarray(0, 100).toString().replace(/\0.*$/s, ""), name);
    assert.equal(file.subarray(345, 500).toString().replace(/\0.*$/s, ""), "logs/" + component);
    assert.equal(raw.subarray(1536, 1552).toString(), "long actual file");
}));
