"use strict";

const { spawn } = require("node:child_process");
const { performance } = require("node:perf_hooks");

class CommandFailure extends Error {
    constructor(result, cause) {
        super("bounded IaC command failed", { cause });
        Object.assign(this, result);
    }
}

// The function namespace owns plugins in independent process groups. This owner
// joins the direct CLI and both pipes, including a root that exits before EOF.
async function execute(command, args, options) {
    const { deadline, cwd, env, hold, grace = 5000, cap = 1024 * 1024 } = options;
    if (!Number.isFinite(deadline) || deadline <= performance.now() || grace < 1 || grace > 5000 ||
        !Number.isSafeInteger(cap) || cap < 1 || cap > 1024 * 1024) throw new Error("invalid CLI owner limits");
    const child = spawn(command, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
    const controller = new AbortController();
    const streams = [child.stdout, child.stderr], outputs = [[], []], sizes = [0, 0];
    let exited = false, forced = false, problem, escalation;
    function stop() {
        forced = true;
        controller.abort();
        for (const stream of streams) stream.destroy();
        if (!exited) {
            child.kill("SIGINT");
            if (!escalation) escalation = setTimeout(() => { if (!exited) child.kill("SIGKILL"); }, grace);
        }
    }
    const drained = streams.map((stream, index) => new Promise(resolve => {
        stream.on("data", chunk => {
            sizes[index] += chunk.length;
            if (sizes[index] > cap) { problem = new Error("IaC command output cap"); stop(); }
            else outputs[index].push(chunk);
        });
        stream.once("error", () => { problem = new Error("IaC command pipe failed"); stop(); });
        stream.once("close", resolve);
    }));
    const done = new Promise(resolve => {
        child.once("error", () => { problem = new Error("IaC command spawn failed"); });
        child.once("exit", (code, signal) => { exited = true; resolve({ code, signal }); });
        child.once("close", (code, signal) => { if (!exited) { exited = true; resolve({ code, signal }); } });
    });
    const timer = setTimeout(stop, Math.max(1, deadline - performance.now()));
    let status, failure;
    try {
        if (hold) await hold({ pid: child.pid, deadline, alive: () => !exited && !forced, signal: controller.signal });
        status = await done;
        await Promise.all(drained);
        if (forced || problem || performance.now() >= deadline || status.signal !== null || !Number.isInteger(status.code)) {
            throw new Error("bounded IaC command failed");
        }
    } catch (error) {
        failure = error;
    } finally {
        if (!exited || streams.some(stream => !stream.closed)) stop();
        status = await done;
        await Promise.all(drained);
        clearTimeout(timer);
        clearTimeout(escalation);
        controller.abort();
    }
    const result = { ...status, stdout: Buffer.concat(outputs[0]), stderr: Buffer.concat(outputs[1]) };
    if (failure) throw new CommandFailure(result, failure);
    return result;
}

module.exports = { execute, CommandFailure };
