"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const { randomUUID } = require("node:crypto");
const { performance } = require("node:perf_hooks");
const { setTimeout: delay } = require("node:timers/promises");
const { validate, resourceArgs, gateResource, stagePath, smallJSON, publishJSON } = require("./program_support");

function config(workspace) {
    return { nonce: "a".repeat(32), endpoint: "http://127.0.0.1:4566", target: "IacTarget",
        businessArn: "arn:aws:lambda:us-east-1:000000000000:function:Business",
        role: "arn:aws:iam::000000000000:role/ImageIac",
        kmsKey: "arn:aws:kms:us-east-1:000000000000:key/00000000-0000-4000-8000-000000000000",
        emptyConfig: workspace + "/empty-config" };
}
const arn = "arn:aws:states:us-east-1:000000000000:stateMachine:IacTarget";

test("test_should_validate_literal_program_contract_and_negative_shapes", () => {
    const source = config("/tmp/iac-" + randomUUID());
    assert.equal(validate(source), source);
    for (const value of [21, 23]) {
        const args = resourceArgs(source, value, "none");
        assert.equal(JSON.parse(args.definition).States.Call.Parameters.Payload.value, value);
        assert.deepEqual(args.tags, { Fixture: source.nonce, Value: String(value) });
    }
    assert.equal(resourceArgs(source, 23, "publish").publish, true);
    assert.equal(resourceArgs(source, 23, "logging").loggingConfiguration.level, "ALL");
    assert.equal(resourceArgs(source, 23, "encryption").encryptionConfiguration.type, "CUSTOMER_MANAGED_KMS_KEY");
    assert.throws(() => validate({ ...source, extra: 1 }));
    assert.throws(() => resourceArgs(source, 22, "none"));
    assert.throws(() => validate({ ...source, endpoint: "http://user@127.0.0.1:4566" }));
});

test("test_should_skip_known_arn_preview_and_recovery_gates_without_io", async () => {
    const workspace = "/tmp/iac-" + randomUUID(), source = config(workspace);
    assert.equal(await gateResource(arn, source, { dryRun: true, stage: "created" }), arn);
    assert.equal(await gateResource(arn, source, { dryRun: false, stage: "none" }), arn);
    await assert.rejects(fs.stat(workspace), { code: "ENOENT" });
});

test("test_should_bind_live_gate_to_nonce_ordinal_and_stage", async () => {
    const workspace = "/tmp/iac-" + randomUUID(), source = config(workspace);
    await fs.mkdir(workspace, { mode: 0o700 });
    const options = { dryRun: false, stage: "created", ordinal: 4, workspace, deadline: performance.now() + 3000 };
    let settled = false;
    const owned = gateResource(arn, source, options).finally(() => { settled = true; });
    try {
        while (!await fs.stat(stagePath(workspace, 4, "created", false)).catch(() => false)) await delay(10);
        await publishJSON(stagePath(workspace, 3, "created", true), { nonce: source.nonce, ordinal: 3, stage: "created" }, options.deadline);
        await delay(75);
        assert.equal(settled, false);
        await publishJSON(stagePath(workspace, 4, "created", true), { nonce: source.nonce, ordinal: 4, stage: "created" }, options.deadline);
        assert.equal(await owned, arn);
    } finally { await owned.catch(() => {}); await fs.rm(workspace, { recursive: true }); }
});

test("test_should_reject_wrong_nonce_invalid_utf8_and_symlink_control_files", async () => {
    const workspace = "/tmp/iac-" + randomUUID(), source = config(workspace);
    await fs.mkdir(workspace, { mode: 0o700 });
    try {
        await fs.writeFile(stagePath(workspace, 4, "created", true), JSON.stringify({ nonce: "b".repeat(32), ordinal: 4, stage: "created" }));
        await assert.rejects(gateResource(arn, source, { dryRun: false, stage: "created", ordinal: 4,
            workspace, deadline: performance.now() + 2000 }));
        await fs.writeFile(workspace + "/invalid", Buffer.from([34, 255, 34]));
        await assert.rejects(smallJSON(workspace + "/invalid", 512));
        await fs.symlink(workspace + "/invalid", workspace + "/link");
        await assert.rejects(smallJSON(workspace + "/link", 512));
        await fs.writeFile(workspace + "/oversize", " ".repeat(513));
        await assert.rejects(smallJSON(workspace + "/oversize", 512));
    } finally { await fs.rm(workspace, { recursive: true }); }
});

test("test_should_publish_complete_phase_and_grant_atomically_without_replacement", async () => {
    const { publishJSON } = require("./program_support");
    const workspace = "/tmp/iac-" + randomUUID();
    await fs.mkdir(workspace, { mode: 0o700 });
    try {
        for (const grant of [false, true]) {
            const file = stagePath(workspace, 4, "created", grant);
            const value = { nonce: "a".repeat(32), ordinal: 4, stage: "created" };
            let release, reached;
            const held = new Promise(resolve => { release = resolve; });
            const ready = new Promise(resolve => { reached = resolve; });
            const owned = publishJSON(file, value, performance.now() + 3000, async temporary => {
                assert.deepEqual(await smallJSON(temporary, 512), value);
                assert.equal((await fs.stat(temporary)).mode & 0o777, 0o600);
                reached();
                await held;
            });
            try {
                await Promise.race([ready, owned.then(() => { throw new Error("publication passed its held barrier"); })]);
                await assert.rejects(fs.stat(file), { code: "ENOENT" });
            } finally { release(); await owned; }
            assert.deepEqual(await smallJSON(file, 512), value);
            await assert.rejects(publishJSON(file, { ...value, nonce: "b".repeat(32) }, performance.now() + 3000), { code: "EEXIST" });
            assert.deepEqual(await smallJSON(file, 512), value);
            assert.equal((await fs.readdir(workspace)).filter(name => name.includes("pending")).length, 0);
        }
    } finally { await fs.rm(workspace, { recursive: true }); }
});
