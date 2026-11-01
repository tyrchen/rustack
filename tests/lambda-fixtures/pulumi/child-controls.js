"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { performance } = require("node:perf_hooks");
const fs = require("node:fs/promises");
const path = require("node:path");
const os = require("node:os");
const { setTimeout: delay } = require("node:timers/promises");
const { execute } = require("./child_owner");

function options(extra = {}) { return { deadline: performance.now() + 3000, cwd: __dirname,
    env: { PATH: process.env.PATH }, grace: 100, ...extra }; }
function absent(pid) { assert.throws(() => process.kill(pid, 0), { code: "ESRCH" }); }

test("test_should_join_real_child_exit_and_both_pipes", async () => {
    let pid;
    const result = await execute(process.execPath, ["-e", 'process.stdout.write("out");process.stderr.write("err");process.exitCode=7'],
        options({ hold: async owner => { pid = owner.pid; } }));
    assert.equal(result.code, 7);
    assert.equal(result.stdout.toString(), "out");
    assert.equal(result.stderr.toString(), "err");
    absent(pid);
});

test("test_should_escalate_and_join_sigint_resistant_child_on_hold_failure", async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "iac-child-"));
    const marker = path.join(root, "ready");
    let pid;
    try {
        await assert.rejects(execute(process.execPath, ["-e",
            'process.on("SIGINT",()=>{});require("node:fs").writeFileSync(process.argv[1],"ready",{flag:"wx",mode:0o600});setInterval(()=>{},10)', marker],
        options({ hold: async owner => {
            pid = owner.pid;
            while (owner.alive()) {
                try { await fs.stat(marker); break; }
                catch (error) { if (error.code !== "ENOENT") throw error; }
                await delay(10);
            }
            assert.ok(owner.alive());
            throw new Error("hold rejected");
        } })), error => error.cause?.message === "hold rejected" && error.signal === "SIGKILL");
        absent(pid);
    } finally { await fs.rm(root, { recursive: true }); }
});

test("test_should_join_child_after_original_deadline_or_output_cap", async () => {
    for (const source of ['process.on("SIGINT",()=>{});setInterval(()=>{},10)', 'process.stdout.write("x".repeat(10000));setInterval(()=>{},10)']) {
        let pid;
        await assert.rejects(execute(process.execPath, ["-e", source], options({ deadline: performance.now() + 200, cap: 1024,
            hold: async owner => { pid = owner.pid; } })));
        absent(pid);
    }
});

test("test_should_join_spawn_failure_without_waiting_for_exit_event", async () => {
    await assert.rejects(execute("/nonexistent/rustack-iac-command", [], options()));
});

test("test_should_retain_bounded_diagnostics_after_root_exits_before_live_gate", async () => {
    let pid;
    await assert.rejects(execute(process.execPath, ["-e", 'process.stderr.write("owned private diagnostic");process.exitCode=9'],
        options({ hold: async owner => {
            pid = owner.pid;
            while (owner.alive()) await delay(10);
            throw new Error("live gate missing");
        } })), error => error.code === 9 && error.stderr.toString() === "owned private diagnostic" &&
            error.cause?.message === "live gate missing");
    absent(pid);
});
